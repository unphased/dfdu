//! `dfdu [PATH]` on a terminal: the interactive browser.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use dfdu_engine::EngineOptions;
use dfdu_tree::HostPath;
use dfdu_tui::{Reopen, Reopened, Source, StartLocal, Theme, ThemeChoice, ThemeFile, UiSettings};

use crate::launch::Local;
use crate::transfer::read_export;
use crate::{Cli, MountArgs, RuleArgs, remote};

/// `dfdu [PATH]` on a terminal: the browser, on PATH, a disk from the picker, a scan read from a
/// file, or a directory on another host.
pub fn run(cli: Cli) -> Result<ExitCode, ExitCode> {
    let theme = match theme_choice(&cli.theme, dfdu_sys::dirs::theme_dir().as_deref()) {
        Ok(theme) => theme,
        Err(e) => {
            eprintln!("dfdu: theme {}: {e}", cli.theme);
            return Err(ExitCode::from(2));
        }
    };
    let engine = EngineOptions {
        read_only: cli.read_only,
        ..EngineOptions::from_env()
    };
    let mut paths = cli.paths;
    if paths.is_empty() {
        paths.push(PathBuf::from("."));
    }
    #[cfg(unix)]
    let peer = if cli.peer {
        Some(crate::peer_cli::start(
            &paths,
            cli.bind.as_deref(),
            cli.peer_remote_bin.clone(),
        )?)
    } else {
        None
    };
    #[cfg(unix)]
    let _peer_signals = peer
        .as_ref()
        .map(|options| crate::peer_cli::signals(&options.peer))
        .transpose()?;
    #[cfg(not(unix))]
    if cli.peer {
        eprintln!("dfdu: peer mode requires Unix");
        return Err(ExitCode::from(2));
    }
    let multi = paths.len() > 1;
    let path = paths[0].clone();
    let mut reopen = None;
    let source = if let Some(f) = &cli.import {
        Source::Imported(read_export(f)?.tree)
    } else if let Some(host) = &cli.remote {
        reopen = Some(reopen_remote(
            host,
            &engine,
            cli.mounts,
            cli.rules,
            cli.remote_agent,
        ));
        // Connected before the browser takes the terminal, so ssh can ask for a password.
        // Deletion runs on the agent, journalled on its host.
        let rules = cli.rules.remote();
        let job = cli.mounts.job();
        let (engine, job, root) =
            remote::engine(host, engine.clone(), job, &path, rules, cli.remote_agent)?;
        Source::Remote {
            engine,
            path: root,
            job,
            local: false,
            elevated: false,
        }
    } else if multi {
        let rules = cli.rules.load();
        reopen = Some(reopen_local(cli.mounts, cli.no_cache, rules.clone()));
        // A single backend serves all roots. Use directory listing rather than choosing an
        // MFT backend based on just one of potentially several volumes.
        if cli.mounts.mft {
            eprintln!("dfdu: --mft is not used for multiple roots; listing directories");
        }
        let local = Local {
            mft: false,
            threads: None,
            job: cli.mounts.job(),
            rules,
        };
        let job_paths = paths.clone();
        Source::Multiple {
            paths,
            start: Box::new(move |opts| {
                let jobs = job_paths.iter().map(|path| local.job(path)).collect();
                let (engine, _) = local.start(&job_paths[0], opts);
                (engine, jobs)
            }),
        }
    } else {
        let resumed = (cli.resume_from.as_deref().map(read_export))
            .transpose()?
            .map(|im| (im.tree, im.header));
        let root = resumed
            .as_ref()
            .and_then(|(t, _)| t.roots().next().map(|r| t.path_buf(r)));
        let header = resumed.as_ref().and_then(|(_, h)| h.as_ref());
        // Checked before the browser takes the terminal. With --disks the directory is not
        // known yet, and --mft is not used.
        let rules = cli.rules.load();
        reopen = Some(reopen_local(cli.mounts, cli.no_cache, rules.clone()));
        let local = Local {
            mft: !cli.disks && cli.mounts.use_mft(root.as_deref().unwrap_or(&path)),
            threads: None,
            job: cli.mounts.job_resuming(header),
            rules,
        };
        // The cache is for a directory named on the command line, not one read from a file or
        // chosen from the disks.
        let slot = (resumed.is_none() && !cli.disks && !cli.no_cache)
            .then(|| cache_slot(&path, &local))
            .flatten();
        let start: StartLocal = Box::new(move |path, opts| local.start(path, opts));
        match (resumed, slot) {
            (Some((tree, header)), _) => Source::Resumed {
                tree,
                journal: header.and_then(|h| h.journal),
                start,
            },
            (None, Some(slot)) => Source::Cached { path, slot, start },
            (None, None) => Source::Local {
                path: (!cli.disks).then_some(path),
                start,
            },
        }
    };
    let opts = dfdu_tui::RunOptions {
        #[cfg(unix)]
        peer,
        source,
        engine,
        history: !multi && !cli.no_history && cli.import.is_none(),
        reopen,
        ui: UiSettings {
            theme,
            color: cli.color.0,
            bars: cli.bars,
            mouse: !cli.no_mouse,
            debug_stats: cli.debug_stats,
            types: cli.types.unwrap_or_default(),
        },
    };
    match dfdu_tui::run(opts) {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(e) => {
            eprintln!("dfdu: {e}");
            Err(ExitCode::from(1))
        }
    }
}

/// Opens a scan of another directory on this host as `dfdu PATH` does: the mounts and rules of
/// the command line (`rules` loaded once, so their errors are not written over the browser),
/// and the scan cache unless it is off. The browser holds the terminal, so what cannot be used
/// (--mft, the cache) is said on its status line.
fn reopen_local(
    mounts: MountArgs,
    no_cache: bool,
    rules: Option<std::sync::Arc<dfdu_rules::RuleSet>>,
) -> Reopen {
    let source = move |path: &HostPath| {
        let path = path.to_local();
        let mut notes = Vec::new();
        let local = Local {
            mft: mounts.mft_for(&path).unwrap_or_else(|why| {
                notes.push(why);
                false
            }),
            threads: None,
            job: mounts.job(),
            rules: rules.clone(),
        };
        let slot = if no_cache {
            None
        } else {
            open_slot(&path, &local).unwrap_or_else(|why| {
                notes.push(why);
                None
            })
        };
        let start: StartLocal = Box::new(move |path, opts| local.start(path, opts));
        let source = match slot {
            Some(slot) => Source::Cached { path, slot, start },
            None => Source::Local {
                path: Some(path),
                start,
            },
        };
        Ok(Reopened {
            source,
            notice: (!notes.is_empty()).then(|| notes.join("; ")),
        })
    };
    Reopen {
        source: Box::new(source),
        suspend: false,
    }
}

/// Opens a scan of another directory on `host` through a new connection, with the mounts and
/// rules of the command line. ssh may ask for a password, so the browser gives it the terminal.
fn reopen_remote(
    host: &str,
    engine: &EngineOptions,
    mounts: MountArgs,
    rule_args: RuleArgs,
    mode: Option<remote::AgentMode>,
) -> Reopen {
    let (host, engine) = (host.to_owned(), engine.clone());
    let source = move |path: &HostPath| {
        let typed = PathBuf::from(dfdu_tree::name_os(path.as_bytes()).into_owned());
        let rules = rule_args.remote();
        // The terminal is ssh's while this runs: what it writes is seen.
        match remote::engine(&host, engine.clone(), mounts.job(), &typed, rules, mode) {
            Ok((engine, job, root)) => Ok(Reopened {
                source: Source::Remote {
                    engine,
                    path: root,
                    job,
                    local: false,
                    elevated: false,
                },
                notice: None,
            }),
            Err(_) => Err(std::io::Error::other(format!(
                "cannot connect to {host} (the reason is on the terminal after dfdu quits)"
            ))),
        }
    };
    Reopen {
        source: Box::new(source),
        suspend: true,
    }
}

/// Where the scan of `path` with the options of `local` is cached; `None` when the cache is
/// off ([`dfdu_store::cache::OFF_VAR`]), refused (said on standard error, before the browser
/// takes the terminal: another user's directory, or one others can write to), or `path` is not
/// a directory (the scan then says why).
fn cache_slot(path: &Path, local: &Local) -> Option<dfdu_store::cache::Slot> {
    open_slot(path, local).unwrap_or_else(|why| {
        eprintln!("dfdu: {why}");
        None
    })
}

/// [`cache_slot`] without writing anything: why the cache is refused is returned.
fn open_slot(path: &Path, local: &Local) -> Result<Option<dfdu_store::cache::Slot>, String> {
    let cache = match dfdu_store::cache::Cache::open_default() {
        None => return Ok(None),
        Some(Ok(cache)) => cache,
        Some(Err(e)) => return Err(format!("{e}; the scan cache is not used")),
    };
    Ok((dfdu_store::cache::Key::new(path, &local.job, local.mft).ok()).map(|key| cache.slot(key)))
}

/// The theme `--theme` names: `auto`, a built-in theme, or the theme file `NAME.toml` in
/// `themes` (`None` when the platform has no configuration directory).
fn theme_choice(name: &str, themes: Option<&Path>) -> Result<ThemeChoice, String> {
    if let Some(choice) = ThemeChoice::builtin(name) {
        return Ok(choice);
    }
    let builtin = format!("auto, {}", Theme::builtin_names().join(", "));
    if Path::new(name).file_name() != Some(OsStr::new(name)) {
        return Err(format!(
            "not a built-in theme ({builtin}) or the name of a theme file"
        ));
    }
    let Some(themes) = themes else {
        return Err(format!(
            "not a built-in theme ({builtin}), and there is no configuration directory for theme \
             files"
        ));
    };
    let path = themes.join(format!("{name}.toml"));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "not a built-in theme ({builtin}), and there is no file {}",
                path.display()
            ));
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut file = ThemeFile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    file.name.get_or_insert_with(|| name.to_owned());
    Ok(ThemeChoice::File(file))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// The browser holds the terminal when it opens another scan: what cannot be used is for
    /// its status line, not written over it.
    #[cfg(not(windows))]
    #[test]
    fn a_scan_opened_from_the_browser_says_what_it_left_out() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let mounts = MountArgs {
            mft: true,
            ..MountArgs::default()
        };
        let mut reopen = reopen_local(mounts, true, None);
        let r = (reopen.source)(&HostPath::local(tmp.path())).expect("source");
        assert!(!reopen.suspend);
        let notice = r.notice.unwrap_or_default();
        assert!(notice.contains("--mft: only on Windows"), "{notice}");
        assert!(matches!(r.source, Source::Local { path: Some(_), .. }));

        let mut reopen = reopen_local(MountArgs::default(), true, None);
        let r = (reopen.source)(&HostPath::local(tmp.path())).expect("source");
        assert_eq!(r.notice, None);
    }

    #[test]
    fn bad_theme_file_is_reported() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let themes = tmp.path();
        let write = |name: &str, text: &str| fs::write(themes.join(name), text).expect("write");
        write("broken.toml", "base = \"sepia\"\n");
        write("typo.toml", "[tokens]\nfg = \"#12345\"\n");
        write("syntax.toml", "base = \n");
        write(
            "nord.toml",
            "base = \"light\"\n[tokens]\nfg = \"#2e3440\"\n",
        );
        fs::create_dir(themes.join("dir.toml")).expect("mkdir");

        let err = |name: &str| theme_choice(name, Some(themes)).expect_err(name);
        let missing = err("missing");
        assert!(missing.contains("not a built-in theme"), "{missing}");
        assert!(missing.contains("missing.toml"), "{missing}");
        let broken = err("broken");
        assert!(
            broken.contains("broken.toml") && broken.contains("sepia"),
            "{broken}"
        );
        let typo = err("typo");
        assert!(typo.contains("`fg`") && typo.contains("#12345"), "{typo}");
        assert!(err("syntax").contains("syntax.toml"));
        assert!(err("dir").contains("dir.toml"));
        assert!(err("../nord").contains("not a built-in theme"));
        assert!(theme_choice("nord", None).is_err());

        match theme_choice("nord", Some(themes)) {
            Ok(ThemeChoice::File(file)) => {
                assert_eq!(file.name.as_deref(), Some("nord"));
                assert_eq!(file.base(), dfdu_tui::BuiltinTheme::Light);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            theme_choice("high-contrast", None),
            Ok(ThemeChoice::Builtin(dfdu_tui::BuiltinTheme::HighContrast))
        ));
        assert!(matches!(theme_choice("auto", None), Ok(ThemeChoice::Auto)));
    }
}
