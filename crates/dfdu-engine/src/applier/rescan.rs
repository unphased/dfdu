//! Rescans of visible directories.

use std::fmt;
use std::sync::Arc;

use dfdu_scan::classify::ClassOpts;
use dfdu_scan::{Backend, JobOptions};
use dfdu_tree::{DirState, Ix, NodeFlags, NodeRef, Tree};

use super::jobs::JobStart;
use super::{Applier, JobKind, JobPhase, Kept, Model, meta_of};

/// What a rescan of directory `ix` classifies against: the rules decided above it, whether it
/// is protected, and its own hits, from its parent's listing as the tree has it. `job` is what
/// the job that scanned `ix` classified against, and `job_root` that job's root.
pub(super) fn rescan_class(
    tree: &Tree,
    ix: Ix,
    job_root: Option<Ix>,
    job: &ClassOpts,
) -> ClassOpts {
    let rules = job.rules.clone();
    // The rule path: the job root's, then the names below it.
    let mut names = Vec::new();
    let mut cur = Some(ix);
    while let Some(i) = cur
        && Some(i) != job_root
    {
        names.push(tree.name(i));
        cur = tree.parent(i);
    }
    let path_of = |names: &[&[u8]]| {
        let mut p = job.path.clone();
        for n in names.iter().rev() {
            if !p.ends_with(b"/") {
                p.push(b'/');
            }
            p.extend_from_slice(n);
        }
        p
    };
    let raw = if cur.is_some() {
        path_of(&names)
    } else {
        dfdu_rules::rule_path_on(&tree.host_path(ix))
    };
    // Read through the firmlinks, as the scan reads it: a firmlinked directory of the Data
    // volume is judged where the system volume shows it, not by its parent there.
    let links = rules.protected().firmlinks();
    let linked = links.shown_at(&raw).is_some();
    let path = links.shown(raw);
    let mut inside = Vec::new();
    let mut cur = tree.parent(ix);
    while let Some(p) = cur {
        inside.extend(tree.class(p).map(|c| c.rule));
        cur = tree.parent(p);
    }
    inside.reverse();
    let protection = |i: Ix| {
        if tree.is_protected(i) {
            dfdu_rules::protected::State::On
        } else {
            dfdu_rules::protected::State::Off
        }
    };
    let mut chain = Vec::new();
    if let Some(p) = tree.parent(ix) {
        let cut = path.iter().rposition(|&b| b == b'/').unwrap_or(0);
        let parent = if cut == 0 {
            b"/".to_vec()
        } else {
            path[..cut].to_vec()
        };
        let sibling = |n: &[u8]| tree.children(p).any(|c| tree.name(c) == n);
        let v = rules.classify(&dfdu_rules::Candidate {
            parent: &parent,
            name: tree.name(ix),
            is_dir: true,
            sibling: &sibling,
            inside: &inside,
            protection: if linked {
                rules.protected().state_of(&parent)
            } else {
                protection(p)
            },
            partial: false,
            stat: None,
            link: None,
        });
        if let dfdu_rules::Verdict::Hits(h) = v {
            chain = h;
        }
    }
    ClassOpts {
        rules,
        path,
        inside,
        protection: protection(ix),
        chain,
    }
}

/// Whether `ix` is a mount the scan listed but did not enter: not a kernel filesystem, which is
/// never scanned, nor a second mount of something counted elsewhere.
fn left_out(tree: &Tree, ix: Ix) -> bool {
    let f = tree.flags(ix);
    tree.dir(ix).is_some_and(|d| d.state == DirState::Skipped)
        && f.has(NodeFlags::MOUNT_ROOT)
        && !f.has(NodeFlags::DUP_MOUNT)
        && !f.has(NodeFlags::KERNEL)
}

/// Why a directory is not rescanned ([`rescan_owner`]); said in a notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NotRescanned {
    /// An enter of what is not a mount the scan left out.
    NotLeftOut,
    NotADirectory,
    /// Counted at another mount of it.
    CountedElsewhere,
    Kernel,
    Network,
    /// On a filesystem the scan did not enter.
    OtherFilesystem,
}

impl fmt::Display for NotRescanned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NotRescanned::NotLeftOut => "enter: that is not a mount the scan left out",
            NotRescanned::NotADirectory => "rescan: not a directory",
            NotRescanned::CountedElsewhere => {
                "rescan: that directory is counted at another mount of it"
            }
            NotRescanned::Kernel => "rescan: that is a kernel filesystem, which is never scanned",
            NotRescanned::Network => {
                "rescan: that is a network filesystem; start dfdu with --network to scan it"
            }
            NotRescanned::OtherFilesystem => "rescan: that directory is on another filesystem",
        })
    }
}

/// The job that scanned directory `ix`, whose options its rescan takes, or why it is not
/// rescanned: with `enter`, unless it is a mount the scan left out ([`left_out`]); otherwise
/// when it is not a directory, is counted at another mount of it, or is on a filesystem the
/// scan does not enter (a kernel or network one, or another filesystem).
fn rescan_owner(tree: &Tree, ix: Ix, enter: bool) -> Result<u32, NotRescanned> {
    if enter && !left_out(tree, ix) {
        return Err(NotRescanned::NotLeftOut);
    }
    let f = tree.flags(ix);
    match tree.dir(ix) {
        Some(d) if enter => Ok(d.job.0),
        None => Err(NotRescanned::NotADirectory),
        Some(_) if f.has(NodeFlags::DUP_MOUNT) => Err(NotRescanned::CountedElsewhere),
        Some(_) if f.has(NodeFlags::KERNEL) => Err(NotRescanned::Kernel),
        Some(_) if f.has(NodeFlags::NETWORK) => Err(NotRescanned::Network),
        Some(d) if d.state == DirState::Skipped => Err(NotRescanned::OtherFilesystem),
        Some(d) => Ok(d.job.0),
    }
}

impl<B: Backend> Applier<B> {
    /// Keeps mount overrides with their visible root and applies them to its rescans.
    fn enter_again(&mut self, root: NodeRef, opts: &mut JobOptions, entered: Option<(u64, u64)>) {
        let mounts = self.entered.entry(root).or_default();
        if let Some(at) = entered.filter(|at| !mounts.contains(at)) {
            mounts.push(at);
        }
        for &at in mounts.iter() {
            if !opts.enter.contains(&at) {
                opts.enter.push(at);
            }
        }
    }

    /// Rescans `node` with the options of the job that scanned it, or `given`. With `enter`,
    /// `node` is a mount that was not entered, and is scanned in place: a job's root is read
    /// whatever filesystem it is on. Rescans enter mounts entered under the same root. A rescan of
    /// a directory whose parent is still being listed starts when that listing closes.
    pub(super) fn rescan(
        &mut self,
        m: &mut Model,
        node: NodeRef,
        given: Option<JobOptions>,
        enter: bool,
    ) {
        self.rescan_keeping(m, node, given, enter, None);
    }

    /// [`Applier::rescan`], keeping the loaded directories of `kept` where the scan meets them:
    /// one of a resume's rescans, which has a `kept` even when it keeps nothing.
    pub(super) fn rescan_keeping(
        &mut self,
        m: &mut Model,
        node: NodeRef,
        given: Option<JobOptions>,
        enter: bool,
        kept: Option<Box<Kept>>,
    ) {
        let Some(ix) = m.tree.resolve_visible(node) else {
            m.status
                .push_notice("rescan: that directory is gone".into());
            return;
        };
        let owner_id = match rescan_owner(&m.tree, ix, enter) {
            Ok(id) => id,
            Err(why) => {
                m.status.push_notice(why.to_string());
                return;
            }
        };
        // Overlap with deletes and other rescans.
        if self.plan_overlaps(&m.tree, ix) {
            m.status.push_notice(
                "rescan: a delete in this directory is in progress; rescan after it completes"
                    .into(),
            );
            return;
        }
        // Its class is decided against its siblings, all of them once its parent's listing
        // closes: until then it waits, and the scan that is reading it carries on.
        if given.is_none() && self.parent_listing_open(&m.tree, ix) {
            if !self.rescans_after_listing.contains(&(node, enter)) {
                self.rescans_after_listing.push((node, enter));
            }
            return;
        }
        // A resume's rescans (those that keep) overlap neither one another nor any other:
        // nothing else rescans the loaded tree until all of them have started.
        if kept.is_none() && !self.supersede_rescans(m, ix) {
            return;
        }
        self.cut(m, ix, DirState::Superseded);
        let id = self.next_job;
        self.next_job += 1;
        let path = m.tree.host_path(ix);
        let shadow = m.tree.new_root(
            m.tree.name(ix).to_vec().as_slice(),
            &meta_of(&m.tree, ix),
            false,
        );
        // Whether the directory is a mount root depends on its parent, which the rescan keeps.
        if m.tree.flags(ix).has(NodeFlags::MOUNT_ROOT) {
            m.tree.set_flag(shadow, NodeFlags::MOUNT_ROOT, true);
        }
        let owner = self.jobs.get(&owner_id);
        let owner_root = owner.and_then(|j| m.tree.resolve(j.root));
        let resumed = given.is_some();
        let mut opts = given
            .or_else(|| owner.map(|j| j.opts.clone()))
            .or_else(|| self.retired.get(&owner_id).cloned())
            .unwrap_or_default();
        // Duplicate mounts are judged against the whole visible tree, not just this part.
        let mut top = ix;
        while let Some(p) = m.tree.parent(top) {
            top = p;
        }
        // Not sent to an agent, which judges against its job's root.
        opts.scope = self.settings.remote.is_none().then(|| m.tree.path_buf(top));
        opts.keep = kept.as_ref().map(|k| Arc::new(k.keep()));
        self.enter_again(
            m.tree.node_ref(top),
            &mut opts,
            enter.then(|| m.tree.dev_ino(ix)),
        );
        if let Some(c) = opts.class.as_ref().filter(|_| resumed) {
            m.rules = Some(c.rules.clone());
            m.tree.set_protection(shadow, c.protection.root_mark());
        } else if let Some(c) = &mut opts.class {
            *c = rescan_class(&m.tree, ix, owner_root, c);
            m.tree.set_protection(shadow, m.tree.protection(ix));
        }
        let start = JobStart {
            kind: JobKind::Rescan,
            target: ix,
            root: shadow,
            scope: id,
            path,
            opts,
            kept,
        };
        self.start(m, id, start);
    }

    /// Whether `ix`'s parent is still being listed by a running job, so that its listing
    /// does not have all of `ix`'s siblings yet. Not under a cut, whose listings never close.
    fn parent_listing_open(&self, tree: &Tree, ix: Ix) -> bool {
        tree.parent(ix).is_some_and(|p| {
            tree.dir(p).is_some_and(|d| {
                !d.listing_closed
                    && self
                        .jobs
                        .get(&d.job.0)
                        .is_some_and(|j| j.phase == JobPhase::Running)
            }) && !Self::under_cut(tree, p)
        })
    }

    /// Starts the waiting rescans whose parent's listing has closed since, or whose parent's
    /// job ended without closing it.
    pub(super) fn retry_rescans_after_listing(&mut self, m: &mut Model) {
        if self.rescans_after_listing.is_empty() {
            return;
        }
        for (r, enter) in std::mem::take(&mut self.rescans_after_listing) {
            match m.tree.resolve_visible(r) {
                None => {}
                Some(ix) if self.parent_listing_open(&m.tree, ix) => {
                    self.rescans_after_listing.push((r, enter));
                }
                Some(_) => self.rescan(m, r, None, enter),
            }
        }
    }

    /// Whether a rescan of `ix` may start: not while a rescan of it or of a parent waits to
    /// swap in. Rescans of its subdirectories are cancelled, the new one replacing them.
    fn supersede_rescans(&mut self, m: &mut Model, ix: Ix) -> bool {
        let mut cancel = Vec::new();
        for j in self
            .jobs
            .values()
            .filter(|j| j.kind == JobKind::Rescan && j.swap_pending())
        {
            let Some(t) = m.tree.resolve_visible(j.target) else {
                continue;
            };
            if t == ix || Self::is_ancestor(&m.tree, t, ix) {
                m.status
                    .push_notice("rescan: already rescanning this directory or a parent".into());
                return false;
            }
            if Self::is_ancestor(&m.tree, ix, t) {
                cancel.push(j.id.0);
            }
        }
        for id in cancel {
            self.cancel(id);
        }
        true
    }
}

#[cfg(test)]
mod tests;
