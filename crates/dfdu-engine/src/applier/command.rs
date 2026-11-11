//! The commands the applier takes, why it refuses one, and their dispatch to the submodules
//! that carry them out.

use std::path::PathBuf;

use dfdu_clean::{DeleterReq, Mode};
use dfdu_scan::{Backend, JobId, JobOptions};
use dfdu_tree::{HostPath, Meta, NodeRef};

use super::jobs::JobStart;
use super::links::VISIBLE;
use super::{Applier, JobKind, Model, empty_dir_meta};
use crate::recent::Event;

#[derive(Clone, Debug)]
pub enum Command {
    /// Start a scan of `path` as a new visible root. A path on this machine (a
    /// `PathBuf` converts with `into()`), or for a remote engine the agent's
    /// ([`Connected::root`](crate::Connected::root)).
    Scan {
        path: HostPath,
        opts: JobOptions,
    },
    /// Rescan a visible directory.
    Rescan {
        node: NodeRef,
    },
    /// Scan a mount the scan listed but did not enter (another filesystem, or a network one),
    /// in place, with the options of the job that listed it: the mount is entered, mounts
    /// below it are treated as before. Rescans enter it again until the session ends.
    Enter {
        node: NodeRef,
    },
    /// Rescan the root of a tree loaded from a file ([`Engine::load`](crate::Engine::load))
    /// with these options, which no scan job of this session holds. With `changed` (paths
    /// relative to the root, as [`changed_since`](crate::changed_since) gives them), the tree
    /// is classified as loaded and only those directories, and those above them, are listed
    /// again, with every other loaded directory whose stat shows it changed since it was
    /// loaded: the subdirectories they still have are kept as loaded. `since` is when the
    /// journal position `changed` counts from was read
    /// ([`JournalPosition::time`](crate::JournalPosition::time)), 0 when not known: a directory
    /// changed in that second or later is listed again whatever its mtime says. `root` is the
    /// root's stat, taken after the journal was asked, off the applier, which does no I/O
    /// ([`Engine::resume`](crate::Engine::resume)); `None` when it could not be taken (the
    /// root is listed again, and its rescan says why). A remote scan's root is not judged on
    /// it.
    ///
    /// An engine is resumed with [`Engine::resume`](crate::Engine::resume), which loads the
    /// tree, asks the journal and stats the root before it sends this; only what drives an
    /// applier without an engine (the simulation) builds it.
    Resume {
        node: NodeRef,
        opts: JobOptions,
        changed: Option<Vec<PathBuf>>,
        since: u64,
        root: Option<Box<Meta>>,
    },
    Cancel {
        job: JobId,
    },
    CancelAll,
    /// Delete these visible nodes. Plans run one at a time.
    Delete {
        items: Vec<NodeRef>,
        mode: Mode,
    },
    /// Put back what this plan moved to the trash or staged
    /// ([`Status::undo`](super::Status::undo)).
    Undo {
        plan: u64,
    },
    /// Delete staged items now rather than when their undo window ends.
    CommitStaged,
    /// Run the native command of the rule that matched `node` (its `variant`, if given), or its
    /// dry run; progress goes to [`Status::native`](super::Status::native) with `id`. The
    /// node's directory is rescanned after a real run.
    RunNative {
        id: u64,
        node: NodeRef,
        variant: Option<usize>,
        dry_run: bool,
    },
    /// Estimate what deleting these nodes would free; the answer goes to
    /// [`Status::estimate`](super::Status::estimate) with `id`.
    Estimate {
        id: u64,
        items: Vec<NodeRef>,
    },
    /// Drop queued delete plans. The runtime also stops the running plan after its current item.
    AbortDelete,
    /// Panics inside the applier, to test crash handling.
    #[cfg(feature = "fault-injection")]
    InjectPanic,
    /// Give up on a stalled directory.
    Abandon {
        node: NodeRef,
    },
    AbandonAllStalled,
    /// Run the native `size_command` of every rule that matched something.
    NativeSizes,
}

impl<B: Backend> Applier<B> {
    /// Starts a scan of `path` into a new visible root.
    fn start_scan(&mut self, m: &mut Model, path: HostPath, opts: JobOptions) {
        let id = self.next_job;
        self.next_job += 1;
        let root = m.tree.new_root(path.as_bytes(), &empty_dir_meta(), true);
        if let Some(c) = &opts.class {
            m.rules = Some(c.rules.clone());
            m.tree.set_protection(root, c.protection.root_mark());
        }
        let start = JobStart {
            kind: JobKind::Initial,
            target: root,
            root,
            scope: VISIBLE,
            path,
            opts,
            kept: None,
        };
        self.start(m, id, start);
    }

    /// Applies one command.
    ///
    /// # Panics
    ///
    /// On `Command::InjectPanic` (with the `fault-injection` feature), to test what follows
    /// an applier panic.
    pub fn apply_command(&mut self, m: &mut Model, cmd: Command) {
        m.recent.push(Event::of_command(&cmd));
        if let Some(why) = self.refusal(m, &cmd) {
            m.status.push_notice(why.into());
            m.status.commands_applied += 1;
            return;
        }
        match cmd {
            Command::Scan { path, opts } => self.start_scan(m, path, opts),
            Command::Rescan { node } => self.rescan(m, node, None, false),
            Command::Enter { node } => self.rescan(m, node, None, true),
            Command::Resume {
                node,
                opts,
                changed: None,
                ..
            } => self.rescan(m, node, Some(opts), false),
            Command::Resume {
                node,
                opts,
                changed: Some(dirs),
                since,
                root,
            } => self.resume_changed(m, node, opts, &dirs, (since, root.as_deref())),
            Command::Cancel { job } => self.cancel_jobs(Some(job.0)),
            Command::CancelAll => self.cancel_jobs(None),
            Command::Abandon { node } => self.abandon(m, node),
            Command::AbandonAllStalled => {
                for s in m.status.stalled.clone() {
                    self.abandon(m, s.node);
                }
            }
            Command::Delete { items, mode } => self.submit_plan(m, items, mode),
            Command::Undo { plan } => {
                if m.status.undoing.is_none() && self.delete.undoable.remove(&plan).is_some() {
                    m.status.undoing = Some(plan);
                    self.delete.controls.push(DeleterReq::Undo { plan });
                }
            }
            Command::CommitStaged => self.delete.controls.push(DeleterReq::Commit),
            Command::RunNative {
                id,
                node,
                variant,
                dry_run,
            } => self.run_native(m, id, node, variant, dry_run),
            Command::Estimate { id, items } => {
                m.status.estimate = Some(self.estimate(m, &items, id));
            }
            Command::NativeSizes => self.native_sizes(m),
            #[cfg(feature = "fault-injection")]
            #[expect(clippy::panic, reason = "the command exists to test the crash path")]
            Command::InjectPanic => panic!("injected applier panic"),
            Command::AbortDelete => {
                for p in std::mem::take(&mut self.delete.queued) {
                    for it in &p.items {
                        if let Some(ix) = m.tree.resolve(it.node) {
                            m.tree.set_deleting(ix, false);
                        }
                    }
                }
            }
        }
        m.status.commands_applied += 1;
        self.refresh_status(m);
    }

    /// Why `cmd` is refused, if it is. The one place that decides what read-only mode, an
    /// imported tree and a lost agent allow: the commands themselves do not check again.
    fn refusal(&self, m: &Model, cmd: &Command) -> Option<&'static str> {
        let scans = matches!(
            cmd,
            Command::Scan { .. }
                | Command::Rescan { .. }
                | Command::Enter { .. }
                | Command::Resume { .. }
        );
        if self.settings.imported
            && (scans
                || matches!(
                    cmd,
                    Command::Delete { .. } | Command::RunNative { .. } | Command::NativeSizes
                ))
        {
            // A native command's dry run is refused too: it would run on this host.
            return Some("an imported scan is read-only: nothing is scanned or deleted");
        }
        if self.settings.read_only {
            match cmd {
                Command::Delete { .. } => return Some("delete: read-only mode"),
                Command::RunNative { dry_run: false, .. } => {
                    return Some("native command: read-only mode");
                }
                _ => {}
            }
        }
        if matches!(cmd, Command::Delete { .. }) && m.tree.roots().nth(1).is_some() {
            return Some("delete: unavailable for scans with multiple roots");
        }
        if scans && m.status.disconnected.is_some() {
            return Some("the link to the agent is gone: nothing more is scanned");
        }
        self.resume_refusal(m, cmd)
    }
}

#[cfg(test)]
mod tests;
