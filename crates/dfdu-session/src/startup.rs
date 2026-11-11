//! What a front end opens (`Source`) and the parts of starting it that need no terminal or
//! window: the engine for it with what its session needs (`open`), resuming a scan read from a
//! file (`resume`) and the link to the scan history its scan is recorded in (`history`). A
//! front end that offers a choice of disks asks for one before.

use std::io;
use std::path::{Path, PathBuf};

use dfdu_engine::{Command, Engine, EngineOptions, JournalPosition};
use dfdu_scan::{Backend, JobOptions, NativeBackend, PortableBackend};

use crate::cache_link::{CacheLink, Sent};
use crate::history_link::HistoryLink;
use crate::session::SessionOptions;

/// Starts an engine scanning a directory on this host with the options given, and returns it
/// with the options of the scan (which mounts it descends into, the rules it classifies with).
/// Nothing is sent to it yet.
pub type StartLocal =
    Box<dyn FnOnce(&Path, EngineOptions) -> (Engine<dfdu_scan::NativeBackend>, JobOptions)>;

/// Starts one engine and supplies independent options for its roots.
pub type StartMultiple = Box<dyn FnOnce(EngineOptions) -> (Engine<NativeBackend>, Vec<JobOptions>)>;

/// What the browser opens.
pub enum Source {
    /// A directory on this host, scanned by the engine `start` returns; `None` shows the disk
    /// picker first.
    Local {
        path: Option<PathBuf>,
        start: StartLocal,
    },
    /// Independent directories scanned in one engine, each with its own job options.
    /// Multi-root scans do not use the single-root cache or history.
    Multiple {
        paths: Vec<PathBuf>,
        start: StartMultiple,
    },
    /// A scan read from a file, shown at once while its root is rescanned by the engine `start`
    /// returns. When `journal`, where the change journal stood when the scan was taken, can
    /// tell what changed since, only that is rescanned.
    Resumed {
        tree: dfdu_tree::Tree,
        journal: Option<JournalPosition>,
        start: StartLocal,
    },
    /// A directory on this host, as [`Source::Local`], opened from its cached scan in `slot`
    /// when there is one (shown at once and brought up to date as [`Source::Resumed`] is), and
    /// whose complete scans are saved there.
    Cached {
        path: PathBuf,
        slot: dfdu_store::cache::Slot,
        start: StartLocal,
    },
    /// A scan read from a file, browsed read-only.
    Imported(dfdu_tree::Tree),
    /// An engine scanning `path` through an agent with `job`: the root the agent resolved
    /// ([`dfdu_engine::Connected::root`]). It deletes there too (unless
    /// [`EngineOptions::read_only`]); nothing is recorded in the history. The agent is on
    /// another host, or with `local` on this one, run as root ([`crate::elevate`]), so the
    /// tree is this machine's. `elevated`: the agent reported itself privileged, so it sees
    /// what this user cannot ([`Setup::elevated`]).
    Remote {
        engine: Engine<dfdu_scan::NativeBackend>,
        path: dfdu_tree::HostPath,
        job: JobOptions,
        local: bool,
        elevated: bool,
    },
}

/// Starts an engine on the root of `tree`, loads the tree into it and rescans what changed
/// since `journal`, or all of it. Returns the engine with the root's path and the options of
/// the scan; `None` when the tree has no root.
#[must_use]
pub fn resume(
    tree: dfdu_tree::Tree,
    journal: Option<JournalPosition>,
    start: StartLocal,
    engine: &EngineOptions,
) -> Option<(Engine<dfdu_scan::NativeBackend>, PathBuf, JobOptions)> {
    let root = tree.roots().next()?;
    let path = tree.path_buf(root);
    let (engine, job) = start(&path, engine.clone());
    engine.resume(tree, journal, job.clone());
    Some((engine, path, job))
}

/// An engine started for a [`Source`], with what the session around it needs.
pub struct Started<B: Backend> {
    pub engine: Engine<B>,
    pub setup: Setup,
}

/// What the session around an engine started for a [`Source`] needs.
#[derive(Clone, Debug)]
pub struct Setup {
    /// The history and cache jobs of its session.
    pub jobs: SessionOptions,
    /// The scan is of this machine: mounts it did not enter and links out of it are looked up
    /// here, and a comparison of duplicates reads its files.
    pub local: bool,
    /// Nothing is deleted: the scan was read from a file, or the engine is read-only.
    pub read_only: bool,
    /// The user's home directory, shown as `~`: `None` for another machine's scan.
    pub home: Option<PathBuf>,
    /// The scan runs as an administrator ([`crate::elevate`]): this machine's, read by an
    /// agent that can see more than this process, whether it was started through pkexec or
    /// the macOS helper. A scan by this process (`sudo dfdu`) is not: a folder looked up here
    /// is looked up with the rights the scan has.
    pub elevated: bool,
}

/// The engine of a source: the scans of this machine and of others run on the native backend,
/// and a tree read from a file is browsed on the portable one.
pub enum Opened {
    Native(Started<NativeBackend>),
    Imported(Started<PortableBackend>),
}

/// Starts the engine of `source` with `opts` and sends it its first command: the scan of a
/// directory, the resume of a scan read from a file. A cached directory's session sends its
/// own ([`crate::cache_link`]). With `record`, a scan of this machine is recorded in the
/// history. A [`Source::Local`] needs its path: a front end asks for one first.
///
/// # Errors
///
/// When a [`Source::Local`] has no path, or a resumed scan has no root.
pub fn open(source: Source, opts: &EngineOptions, record: bool) -> io::Result<Opened> {
    open_reusing(source, opts, record, None)
}

/// [`open`], keeping what `scanned` read ([`crate::session::Session::scanned`]: another scan
/// of this machine, of a folder below the one opened) where it is unchanged
/// ([`Engine::scan_above`]), for a directory of this machine scanned afresh: a
/// [`Source::Local`], or a [`Source::Cached`] with no cached scan, which is preferred when
/// there is one. Other sources leave it.
///
/// # Errors
///
/// As [`open`].
pub fn open_reusing(
    source: Source,
    opts: &EngineOptions,
    record: bool,
    scanned: Option<dfdu_engine::Scanned>,
) -> io::Result<Opened> {
    let home = dfdu_engine::home_dir;
    let read_only = opts.read_only;
    let link = |root: &Path, job: &JobOptions| history(record, root, job, opts);
    let local = |engine, history, cache| {
        Opened::Native(Started {
            engine,
            setup: Setup {
                jobs: SessionOptions { history, cache },
                local: true,
                read_only,
                home: home(),
                elevated: false,
            },
        })
    };
    Ok(match source {
        Source::Imported(tree) => Opened::Imported(Started {
            engine: Engine::start_imported(tree, opts.clone()),
            setup: Setup {
                jobs: SessionOptions::default(),
                local: false,
                read_only: true,
                // Most files are of this machine's scans; another system's paths are left
                // alone by their host.
                home: home(),
                elevated: false,
            },
        }),
        Source::Remote {
            engine,
            path,
            job,
            local,
            elevated,
        } => {
            engine.send(Command::Scan { path, opts: job });
            Opened::Native(Started {
                engine,
                setup: Setup {
                    jobs: SessionOptions::default(),
                    local,
                    read_only,
                    // Another host's paths are not below this one's home directory.
                    home: if local { home() } else { None },
                    // Only an agent of this machine sees more of it than this user.
                    elevated: local && elevated,
                },
            })
        }
        Source::Multiple { paths, start } => {
            if paths.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "no directories to scan",
                ));
            }
            let (engine, jobs) = start(opts.clone());
            if paths.len() != jobs.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "each directory needs scan options",
                ));
            }
            for (path, job) in paths.into_iter().zip(jobs) {
                engine.send(Command::Scan {
                    path: path.into(),
                    opts: job,
                });
            }
            local(engine, None, None)
        }
        Source::Resumed {
            tree,
            journal,
            start,
        } => {
            let Some((engine, path, job)) = resume(tree, journal, start, opts) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the scan read from the file has no root",
                ));
            };
            local(engine, link(&path, &job), None)
        }
        Source::Cached { path, slot, start } => {
            let root = slot.key.root().to_path_buf();
            let (engine, job) = start(&path, opts.clone());
            let history = link(&root, &job);
            let sent = scanned
                .filter(|_| !slot.cached(dfdu_store::cache::now()))
                .map(|s| {
                    // Taken before the scan, as the cache's own scan takes it. What it keeps
                    // was read earlier: the journal is asked from before that next time.
                    let now = dfdu_engine::journal_position(&root);
                    let (full_scan, before, kept_ms) = (s.full_scan, s.journal, s.scan_ms);
                    if engine.scan_above(&path.as_path().into(), Some(s), job.clone()) {
                        Sent {
                            journal: before,
                            full_scan: Some(full_scan),
                            kept_ms,
                        }
                    } else {
                        Sent {
                            journal: now,
                            full_scan: None,
                            kept_ms: 0,
                        }
                    }
                });
            let cache = CacheLink {
                slot,
                path,
                job,
                save: !read_only,
                sent,
            };
            local(engine, history, Some(cache))
        }
        Source::Local { path: None, .. } => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no directory to scan was chosen",
            ));
        }
        Source::Local {
            path: Some(path),
            start,
        } => {
            let root = dunce::canonicalize(&path).unwrap_or_else(|_| path.clone());
            let (engine, job) = start(&path, opts.clone());
            let history = link(&root, &job);
            engine.scan_above(&path.into(), scanned, job);
            local(engine, history, None)
        }
    })
}

/// The link to the scan history a scan of `root` with `job`, by an engine with `engine`, is
/// recorded in, its last scan's sizes loading; `None` unless `record` and the history has a
/// place on this machine. Only scans that count the same data are compared.
#[must_use]
pub fn history(
    record: bool,
    root: &Path,
    job: &JobOptions,
    engine: &EngineOptions,
) -> Option<HistoryLink> {
    let db = dfdu_sys::dirs::history_path().filter(|_| record)?;
    let key = dfdu_store::history::options_key(job, engine.measure_workers > 0);
    Some(HistoryLink::start(db, root.to_path_buf(), key))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use dfdu_tree::HostPath;

    use super::*;

    #[test]
    fn an_imported_tree_is_browsed_read_only_and_a_local_scan_needs_its_path() {
        let opts = EngineOptions::default();
        let Ok(Opened::Imported(s)) =
            open(Source::Imported(dfdu_tree::Tree::default()), &opts, true)
        else {
            panic!("an imported tree");
        };
        assert!(s.setup.read_only && !s.setup.local);
        assert!(s.setup.jobs.history.is_none() && s.setup.jobs.cache.is_none());
        s.engine.shutdown(Duration::from_millis(200));
        let start: StartLocal = Box::new(|_, _| panic!("no engine is started without a path"));
        let local = Source::Local { path: None, start };
        let e = open(local, &opts, false).err().expect("no path");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
    }

    /// An engine on this machine's backend, without rules.
    fn native() -> StartLocal {
        Box::new(|_, opts| {
            let backend = dfdu_scan::native_backend(dfdu_sys::Storage::Unknown, false);
            (Engine::start(backend, opts), JobOptions::default())
        })
    }

    #[test]
    fn multiple_roots_scan_independently_without_cache_or_history() {
        let tmp = tempfile::tempdir().unwrap();
        let paths: Vec<_> = ["a", "b"]
            .iter()
            .map(|name| {
                let path = tmp.path().join(name);
                std::fs::create_dir(&path).unwrap();
                std::fs::write(path.join("file"), name).unwrap();
                path
            })
            .collect();
        let source = Source::Multiple {
            paths: paths.clone(),
            start: Box::new(|opts| {
                let backend = dfdu_scan::native_backend(dfdu_sys::Storage::Unknown, false);
                (Engine::start(backend, opts), vec![JobOptions::default(); 2])
            }),
        };
        let Ok(Opened::Native(s)) = open(source, &EngineOptions::default(), true) else {
            panic!("multiple local roots");
        };
        assert!(s.setup.jobs.history.is_none() && s.setup.jobs.cache.is_none());
        assert!(s.engine.wait_idle(Some(Duration::from_secs(10))).is_idle());
        s.engine.read(|m| {
            let roots: Vec<_> = m.tree.roots().collect();
            assert_eq!(roots.len(), 2);
            for (root, path) in roots.into_iter().zip(&paths) {
                assert_eq!(m.tree.path_buf(root), dunce::canonicalize(path).unwrap());
                assert!(m.tree.total(root).items > 0);
            }
        });
        s.engine.shutdown(Duration::from_millis(200));
    }

    fn wait_totals(e: &Engine<NativeBackend>) -> (u64, u64) {
        assert!(e.wait_idle(Some(Duration::from_secs(10))).is_idle());
        e.read(|m| {
            let r = m.tree.roots().next().expect("root");
            (m.tree.total(r).items, m.tree.total(r).apparent)
        })
    }

    /// A folder above another scan's root, opened with what it read, ends as a fresh scan of
    /// the folder does.
    #[test]
    fn a_folder_opened_with_a_scan_below_it_ends_as_a_fresh_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let top = dunce::canonicalize(tmp.path()).unwrap();
        let c = top.join("a").join("c");
        std::fs::create_dir_all(c.join("d")).unwrap();
        std::fs::write(c.join("d").join("f"), b"0123456789").unwrap();
        std::fs::write(top.join("h"), b"012").unwrap();
        let opts = EngineOptions::default();
        let open_local = |p: &Path, scanned| {
            let source = Source::Local {
                path: Some(p.to_path_buf()),
                start: native(),
            };
            match open_reusing(source, &opts, false, scanned) {
                Ok(Opened::Native(s)) => s.engine,
                _ => panic!("a local scan"),
            }
        };
        let old = open_local(&c, None);
        wait_totals(&old);
        let scanned = old.scanned(&HostPath::local(&top));
        assert!(scanned.is_some(), "the old scan can be kept");
        old.shutdown(Duration::from_millis(200));
        let fresh = open_local(&top, None);
        let want = wait_totals(&fresh);
        fresh.shutdown(Duration::from_millis(200));
        let new = open_local(&top, scanned);
        assert_eq!(wait_totals(&new), want);
        new.shutdown(Duration::from_millis(200));
    }
}
