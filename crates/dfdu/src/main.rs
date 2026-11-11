use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use dfdu_rules::RuleSet;
use dfdu_scan::JobOptions;
use dfdu_tree::filetype::TypeFilter;
use dfdu_tui::Bars;

use crate::args::{ColorFlag, bars_flag, color_flag, type_filter};

/// What writing the output on stdout ends the command with: nothing when it was written, a
/// quiet success when stdout was closed (`dfdu ... | head`), and exit code 1 with the error
/// otherwise.
fn written<T>(r: std::io::Result<T>) -> Result<T, ExitCode> {
    written_to(&"writing the output", r)
}

/// Writes the output with `f` to stdout through a buffer and flushes it, ending as
/// [`written`] says.
fn print_out(
    f: impl FnOnce(&mut dyn std::io::Write) -> std::io::Result<()>,
) -> Result<(), ExitCode> {
    let mut w = std::io::BufWriter::new(std::io::stdout().lock());
    written(f(&mut w).and_then(|()| std::io::Write::flush(&mut w)))
}

/// [`written`] for output to `what`.
fn written_to<T>(what: &dyn std::fmt::Display, r: std::io::Result<T>) -> Result<T, ExitCode> {
    match r {
        Ok(v) => Ok(v),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Err(ExitCode::SUCCESS),
        Err(e) => {
            eprintln!("dfdu: {what}: {e}");
            Err(ExitCode::from(1))
        }
    }
}

mod args;
mod browse;
mod cache;
mod clean;
mod disks;
mod dupes;
mod history;
mod launch;
mod matches;
mod output;
mod peer_cli;
mod recover;
mod remote;
mod report;
mod scan;
mod transfer;
mod update;

/// The exit codes, for `--help` and the man page.
const EXIT_CODES: &str = "Exit codes:
  0    success
  1    fatal error (the directory could not be read, an output could not be written)
  2    usage error
  3    finished, but some entries could not be read or the scan is incomplete
  4    report --fail-above: the matches add up to more than the limit
  5    clean, recover: some deletions failed or were refused
  130  interrupted (Ctrl-C)";

#[derive(Parser)]
#[command(
    version,
    about,
    args_conflicts_with_subcommands = true,
    after_long_help = EXIT_CODES
)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "command-line flags, each an independent switch"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Directories to browse (opens the interactive browser when stdout is a terminal).
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    #[command(flatten)]
    mounts: MountArgs,
    #[command(flatten)]
    rules: RuleArgs,
    /// Enable peer mode with a private local socket (B binds or unbinds).
    #[arg(long, conflicts_with_all = ["remote", "import", "resume_from", "disks"])]
    peer: bool,
    /// Bind to ID, 'HOST ID' or HOST:ID at startup.
    #[arg(long, value_name = "TARGET", requires = "peer")]
    bind: Option<String>,
    /// Remote dfdu executable for peer binding (expanded by the remote shell).
    #[arg(long, default_value = "~/.local/bin/dfdu", requires = "peer")]
    peer_remote_bin: String,
    /// Choose a disk to browse from the mounted filesystems.
    #[arg(long, conflicts_with = "paths")]
    disks: bool,
    /// Open the scan saved in FILE (a snapshot or an ncdu export) at once, and rescan its
    /// directory in the background, replacing it when done. A snapshot is rescanned with the
    /// mount options it was taken with.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["paths", "disks", "import"])]
    resume_from: Option<PathBuf>,
    /// Browse a scan saved in FILE (a snapshot, an ncdu JSON export, plain or zstd, or an ncdu
    /// binary export from `ncdu -O`; `-` for stdin), read-only.
    #[arg(short = 'f', long = "file", value_name = "FILE", conflicts_with_all = ["paths", "disks"])]
    import: Option<PathBuf>,
    /// Never delete anything: the browser leaves out its delete keys and runs cleanup commands
    /// only as dry runs.
    #[arg(long)]
    read_only: bool,
    /// Browse PATH on HOST, scanned there by the dfdu agent through ssh (sent there if HOST
    /// has none, see --remote-agent) and classified with the rules of this host. The delete
    /// keys delete on HOST, unless --read-only; nothing is recorded in the history.
    #[arg(long, value_name = "HOST", conflicts_with_all = ["disks", "import", "resume_from"])]
    remote: Option<String>,
    #[arg(
        long,
        value_name = "HOW",
        value_enum,
        requires = "remote",
        help = crate::remote::AGENT_MODE_HELP
    )]
    remote_agent: Option<crate::remote::AgentMode>,
    /// Show only files of these types or extensions, comma-separated: `video,audio,.iso`
    /// (`t` in the browser changes it).
    #[arg(long = "type", value_name = "TYPES", value_parser = type_filter)]
    types: Option<TypeFilter>,
    /// Do not record the scan in the history, nor show what grew since the last one.
    #[arg(long)]
    no_history: bool,
    /// Scan PATH afresh: do not open its cached scan, nor save this one (`DFDU_CACHE=0` turns
    /// the cache off for good). Without it, the browser opens the last complete scan of PATH
    /// at once, if one is cached, and brings it up to date in the background; scans that take
    /// 3 s or more are cached for 14 days (`dfdu cache list`).
    #[arg(long)]
    no_cache: bool,
    /// Colour theme: auto, dark, light, ansi (the terminal's own 16 colours), mono, high-contrast,
    /// or NAME for the theme file NAME.toml in dfdu/themes in the configuration directory. auto
    /// asks the terminal for its background colour at start (up to 1 s if it does not answer)
    /// and picks dark or light, or ansi when the colour is unknown.
    #[arg(long, value_name = "THEME", default_value = "auto")]
    theme: String,
    /// Colours: auto (from the terminal and NO_COLOR), never, 16, 256 or truecolor. A level given
    /// here is used even when NO_COLOR is set.
    #[expect(
        clippy::doc_markdown,
        reason = "clap prints this as the help text, where backticks would show"
    )]
    #[arg(long, value_name = "WHEN", default_value = "auto", value_parser = color_flag)]
    color: ColorFlag,
    /// Bars: unicode (eighth blocks, the default) or ascii (# and =, for terminals and fonts
    /// without block characters). Overrides the theme file's `bars`.
    #[arg(long, value_name = "STYLE", value_parser = bars_flag)]
    bars: Option<Bars>,
    /// Do not capture the mouse (click to select, double-click to open, wheel to scroll). While
    /// it is captured, most terminals select text with Shift held down.
    #[arg(long)]
    no_mouse: bool,
    /// Start with the debug panel open, and print its numbers when the browser exits.
    #[arg(long, hide = true)]
    debug_stats: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// List live peer-mode instances on this host, removing stale entries.
    Peers {
        /// Print instance metadata as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Bridge stdin and stdout to a local peer instance (used over ssh).
    PeerAttach { id: String },
    /// Scan a directory and print a summary.
    Scan(scan::ScanArgs),
    /// List the mounted filesystems with their size and free space.
    Disks,
    /// Scan a directory and list what the cleanup rules found in it: caches, build output,
    /// dependencies, with how safe each is to delete.
    Report(report::ReportArgs),
    /// Delete what `report` recommends (dry run unless --yes).
    Clean(clean::CleanArgs),
    /// What grew: a directory now against its last recorded scan (or one from --since ago),
    /// or two saved scans.
    Diff(history::DiffArgs),
    /// List the recorded scans.
    History(history::HistoryArgs),
    /// Scan a directory and list the files with identical contents (size, then partial hash,
    /// then SHA-256), the most space to free first.
    Dupes(dupes::DupesArgs),
    /// Scan a directory and write it in ncdu's JSON format.
    Export(transfer::ExportArgs),
    /// Read a scan in ncdu's JSON format and print a summary (`dfdu -f FILE` browses it).
    Import(transfer::ImportArgs),
    /// List what earlier sessions left unfinished (staged items, interrupted deletes), and
    /// finish, restore or dismiss them.
    Recover(recover::RecoverArgs),
    /// List or clear the scans the browser cached, which it opens at once next time.
    #[command(subcommand)]
    Cache(cache::CacheCmd),
    /// Write man pages (dfdu.1 and one per command) into DIR, for packagers.
    #[command(hide = true)]
    Man { dir: PathBuf },
    /// Check for a newer release of dfdu and install it over this one (checked against its
    /// published SHA-256). A dfdu installed with Homebrew, Nix, Scoop, winget or a system package
    /// manager is left to it: the command to update it is printed instead.
    Update(update::UpdateArgs),
    /// Print a shell completion script (bash, zsh, fish, elvish, powershell).
    Completions { shell: clap_complete::Shell },
    /// Scan for a dfdu on another host (`--remote`): reads requests on stdin and writes what
    /// it finds to stdout in dfdu's wire format. Not for use by hand.
    Agent {
        /// Worker threads.
        #[arg(long)]
        threads: Option<usize>,
        /// Print the protocol versions this agent speaks (`MIN MAX`) and exit.
        #[arg(long)]
        protocols: bool,
    },
}

/// `dfdu man DIR`: `dfdu.1` and `dfdu-<command>.1` for each visible command.
fn man(dir: &Path) -> ExitCode {
    use clap::CommandFactory;
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let cmd = Cli::command();
        let mut out = std::fs::File::create(dir.join("dfdu.1"))?;
        clap_mangen::Man::new(cmd.clone()).render(&mut out)?;
        for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
            let name: &'static str = format!("dfdu-{}", sub.get_name()).leak();
            let page = sub.clone().name(name).bin_name(name.replacen('-', " ", 1));
            let mut out = std::fs::File::create(dir.join(format!("{name}.1")))?;
            clap_mangen::Man::new(page).render(&mut out)?;
        }
        Ok(())
    };
    match write() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dfdu: {}: {e}", dir.display());
            ExitCode::from(1)
        }
    }
}

/// Which mounts below the scanned directory are descended into. Kernel filesystems (`/proc`,
/// `/sys`, cgroups) never are.
#[derive(Args, Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "command-line flags, each an independent switch"
)]
struct MountArgs {
    /// Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts
    /// and --network given before it).
    #[arg(short = 'x', long, overrides_with = "cross_mounts")]
    one_file_system: bool,
    /// Descend into other local filesystems (disks, tmpfs).
    #[arg(long, overrides_with = "one_file_system")]
    cross_mounts: bool,
    /// Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with
    /// or without --cross-mounts.
    #[arg(long, overrides_with = "one_file_system")]
    network: bool,
    /// Windows, as administrator: read each NTFS volume's master file table instead of listing
    /// its directories, which is much faster on large volumes. Volumes whose table cannot be
    /// read are listed as usual.
    #[arg(long)]
    mft: bool,
}

impl MountArgs {
    /// Whether scans of `path` list NTFS volumes from their master file tables: with --mft,
    /// when the table can be read. Says why it cannot be used.
    fn use_mft(self, path: &Path) -> bool {
        self.mft_for(path).unwrap_or_else(|why| {
            eprintln!("dfdu: {why}");
            false
        })
    }

    /// [`MountArgs::use_mft`] without writing anything: why --mft cannot be used is returned,
    /// for a browser that holds the terminal to show.
    fn mft_for(self, path: &Path) -> Result<bool, String> {
        if !self.mft {
            return Ok(false);
        }
        #[cfg(windows)]
        match dfdu_scan::check_mft(path) {
            Ok(()) => Ok(true),
            Err(e) => Err(format!(
                "--mft: the master file table cannot be read ({e}); listing directories"
            )),
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err("--mft: only on Windows; listing directories".into())
        }
    }

    fn job(self) -> JobOptions {
        JobOptions {
            cross_mounts: self.cross_mounts && !self.one_file_system,
            network: self.network && !self.one_file_system,
            ..Default::default()
        }
    }

    /// The options of a scan: [`MountArgs::job`], or for the rescan of a snapshot those it was
    /// taken with.
    fn job_resuming(self, snapshot: Option<&dfdu_store::snapshot::Header>) -> JobOptions {
        let mut job = self.job();
        if let Some(h) = snapshot {
            job.cross_mounts = h.cross_mounts;
            job.network = h.network;
        }
        job
    }
}

/// Classification of what the scan finds (categories and cleanup rules).
#[derive(Args, Clone, Copy, Default)]
struct RuleArgs {
    /// Do not classify entries: no categories, no cleanup candidates.
    #[arg(long)]
    no_rules: bool,
}

impl RuleArgs {
    /// The built-in rules with the user's rule files (dfdu/rules/*.toml in the configuration
    /// directory). A rule that does not load is reported and left out.
    fn load(self) -> Option<Arc<RuleSet>> {
        if self.no_rules {
            return None;
        }
        let dir = dfdu_sys::dirs::rules_dir();
        let (set, errors) = dfdu_rules::load(&dfdu_rules::Env::current(), dir.as_deref());
        for e in errors {
            eprintln!("dfdu: {e}");
        }
        Some(Arc::new(set))
    }

    /// The rule files [`RuleArgs::load`] loads, for an agent to classify a scan on its host
    /// with.
    fn remote(self) -> Option<dfdu_engine::RemoteRules> {
        if self.no_rules {
            return None;
        }
        let dir = dfdu_sys::dirs::rules_dir();
        let (files, errors) = dfdu_rules::sources(dir.as_deref());
        for e in errors {
            eprintln!("dfdu: {e}");
        }
        Some(dfdu_engine::RemoteRules { files })
    }
}

/// Exit code when an entry could not be read or the scan is incomplete.
const EXIT_PARTIAL: u8 = 3;
/// Exit code when some deletions failed or were refused.
const EXIT_DELETE_FAILED: u8 = 5;

/// Writes a crash report for a panic on any thread, with the engine's counters and last
/// messages, then runs the previous hook. In the browser, `dfdu_tui::run` wraps this hook in one
/// that restores the terminal first. Panics the engine catches (a scan worker's, which becomes a
/// `Panicked` directory and an error in the scan's result, and the applier's, for which the
/// engine writes its own report) are left to the engine: no file, no message.
fn install_crash_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if dfdu_engine::panic_is_caught_by_engine(std::thread::current().name()) {
            return;
        }
        if let Some(dir) = dfdu_sys::dirs::crash_dir() {
            let thread = std::thread::current();
            let report = format!(
                "dfdu {} crash report\n\nthread: {}\npanic: {info}\n\nbacktrace:\n{}\n\n{}",
                env!("CARGO_PKG_VERSION"),
                thread.name().unwrap_or("?"),
                std::backtrace::Backtrace::force_capture(),
                dfdu_engine::crash_state().unwrap_or_default(),
            );
            if let Some(p) = dfdu_engine::write_crash_report(&dir, &report) {
                eprintln!("dfdu: crash report written to {}", p.display());
            }
        }
        previous(info);
    }));
}

fn main() -> ExitCode {
    // First: may execute dfdu again with the invoking user's HOME (see `dfdu_sys::acting`).
    if let Err(e) = dfdu_sys::acting::adopt() {
        eprintln!("dfdu: cannot act for the user who ran sudo: {e}");
        return ExitCode::from(1);
    }
    dfdu_sys::dirs::give_back();
    install_crash_hook();
    let code = run(Cli::parse()).unwrap_or_else(|code| code);
    dfdu_sys::dirs::give_back();
    code
}

/// Runs the subcommand. Those that can stop early return the exit code they stop with as
/// `Err`, so their steps chain with `?`.
fn run(cli: Cli) -> Result<ExitCode, ExitCode> {
    if let Some(message) = multiple_paths_error(&cli, std::io::stdout().is_terminal()) {
        eprintln!("dfdu: {message}");
        return Err(ExitCode::from(2));
    }
    if cli.peer && (cli.cmd.is_some() || !std::io::stdout().is_terminal()) {
        eprintln!("dfdu: --peer requires the interactive browser on a terminal");
        return Err(ExitCode::from(2));
    }
    let rules = || RuleArgs::default().load().ok_or(ExitCode::from(1));
    match cli.cmd {
        Some(Cmd::Peers { json }) => peer_cli::list(json),
        Some(Cmd::PeerAttach { id }) => peer_cli::attach(&id),
        Some(Cmd::Scan(args)) => scan::run(args),
        Some(Cmd::Disks) => disks::run(),
        Some(Cmd::Update(a)) => Ok(update::run(&a)),
        Some(Cmd::Report(args)) => report::run(&args, &rules()?),
        Some(Cmd::Clean(args)) => clean::run(&args, &rules()?),
        Some(Cmd::Diff(args)) => history::diff(&args),
        Some(Cmd::History(args)) => history::list(args),
        Some(Cmd::Dupes(args)) => dupes::run(&args),
        Some(Cmd::Export(args)) => transfer::export(&args),
        Some(Cmd::Import(args)) => transfer::import(&args),
        Some(Cmd::Recover(args)) => recover::run(args),
        Some(Cmd::Cache(cmd)) => cache::run(&cmd),
        Some(Cmd::Man { dir }) => Ok(man(&dir)),
        Some(Cmd::Agent { threads, protocols }) => Ok(remote::agent(threads, protocols)),
        Some(Cmd::Completions { shell }) => {
            use clap::CommandFactory;
            clap_complete::generate(shell, &mut Cli::command(), "dfdu", &mut std::io::stdout());
            Ok(ExitCode::SUCCESS)
        }
        // Piped: what `dfdu disks` or `dfdu scan PATH` prints.
        None if !std::io::stdout().is_terminal() => {
            if cli.disks {
                return disks::run();
            }
            scan::run(scan::ScanArgs {
                path: cli
                    .paths
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| PathBuf::from(".")),
                remote: cli.remote,
                remote_agent: cli.remote_agent,
                mounts: cli.mounts,
                rules: cli.rules,
                types: cli.types,
                no_history: cli.no_history,
                ..scan::ScanArgs::defaults()
            })
        }
        None => browse::run(cli),
    }
}

/// Multiple roots are only supported by the local interactive browser.
fn multiple_paths_error(cli: &Cli, terminal: bool) -> Option<&'static str> {
    if cli.paths.len() <= 1 {
        None
    } else if cli.remote.is_some() {
        Some("multiple paths are not supported with --remote")
    } else if !terminal {
        Some("multiple paths require the interactive browser")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use dfdu_tui::ColorLevel;

    use super::*;

    #[test]
    fn multiple_paths_are_only_allowed_in_the_local_browser() {
        let cli = Cli::try_parse_from(["dfdu", "a", "b"]).unwrap();
        assert_eq!(cli.paths, [PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(multiple_paths_error(&cli, true), None);
        assert_eq!(
            multiple_paths_error(&cli, false),
            Some("multiple paths require the interactive browser")
        );

        let remote = Cli::try_parse_from(["dfdu", "--remote", "host", "a", "b"]).unwrap();
        assert_eq!(
            multiple_paths_error(&remote, true),
            Some("multiple paths are not supported with --remote")
        );
        assert!(multiple_paths_error(&Cli::try_parse_from(["dfdu"]).unwrap(), false).is_none());
        assert!(
            multiple_paths_error(
                &Cli::try_parse_from(["dfdu", "--remote", "host", "a"]).unwrap(),
                false
            )
            .is_none()
        );

        for args in [
            vec!["dfdu", "--disks", "a", "b"],
            vec!["dfdu", "--file", "scan", "a", "b"],
            vec!["dfdu", "--resume-from", "scan", "a", "b"],
            vec!["dfdu", "scan", "a", "b"],
            vec!["dfdu", "disks", "a", "b"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn a_failed_write_exits_1_and_a_closed_pipe_quietly() {
        let fail = |kind: std::io::ErrorKind| written::<()>(Err(kind.into()));
        assert_eq!(
            fail(std::io::ErrorKind::StorageFull),
            Err(ExitCode::from(1))
        );
        assert_eq!(fail(std::io::ErrorKind::BrokenPipe), Err(ExitCode::SUCCESS));
        assert_eq!(written(Ok(7)), Ok(7));
    }

    #[test]
    fn theme_and_color_flags_parse() {
        let cli = Cli::try_parse_from(["dfdu", "/x"]).expect("defaults");
        assert_eq!(cli.theme, "auto");
        assert_eq!(cli.color, ColorFlag(None));
        assert!(!cli.no_mouse && !cli.debug_stats);

        let cli = Cli::try_parse_from([
            "dfdu",
            "--theme",
            "light",
            "--color",
            "256",
            "--no-mouse",
            "--debug-stats",
            "/x",
        ])
        .expect("flags");
        assert_eq!(cli.theme, "light");
        assert_eq!(cli.color, ColorFlag(Some(ColorLevel::Ansi256)));
        assert!(cli.no_mouse && cli.debug_stats);
        assert_eq!(cli.paths, vec![PathBuf::from("/x")]);

        for (value, level) in [
            ("never", ColorLevel::None),
            ("16", ColorLevel::Ansi16),
            ("truecolor", ColorLevel::TrueColor),
        ] {
            let cli = Cli::try_parse_from(["dfdu", "--color", value]).expect(value);
            assert_eq!(cli.color, ColorFlag(Some(level)));
        }
        assert!(Cli::try_parse_from(["dfdu", "--color", "always"]).is_err());
        assert_eq!(cli.bars, None);
        let cli = Cli::try_parse_from(["dfdu", "--bars", "ascii"]).expect("bars");
        assert_eq!(cli.bars, Some(Bars::Ascii));
        assert!(Cli::try_parse_from(["dfdu", "--bars", "blocks"]).is_err());
        // After browser options, names that also match subcommands are paths.
        let cli = Cli::try_parse_from(["dfdu", "--theme", "dark", "scan", "/x"]).unwrap();
        assert_eq!(cli.paths, [PathBuf::from("scan"), PathBuf::from("/x")]);
    }
}
