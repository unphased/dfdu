//! Resuming a tree loaded from a file with what the volume's change journal says changed: a
//! journal names the directories whose own entries changed, so each is listed again and the
//! subdirectories it still has are kept as loaded.
//!
//! The resume first walks the loaded tree a step at a time ([`Work::Resume`]): it classifies
//! it, takes it over for a job of this session and registers its hard links, and the scan
//! workers stat each directory it reaches that the journal does not name, to find what the
//! journal lost (`verify`). Until both end, nothing in the tree is deleted, rescanned or
//! cleaned. Then the changed directories are listed again by rescans that keep the loaded
//! subdirectories they find, started a few at a time (`rescans`), each putting what it kept in
//! the place of its placeholders as it swaps in (`graft`). What a cancelled resume did not list
//! again stays refused to deletes ([`Applier::unverified`]).

mod graft;
mod reclassify;
mod rescans;
mod verify;
mod walk;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use dfdu_scan::journal::Keep;
use dfdu_scan::{Backend, JobId, JobOptions};
use dfdu_tree::{Ix, Meta, NodeRef, Pages, Tree};

use super::links::VISIBLE;
use super::work::Work;
use super::{Applier, Command, Job, JobKind, JobPhase, Model};
pub(in crate::applier) use graft::Grafted;
use reclassify::ClassifyLoaded;
use verify::Verify;
pub(in crate::applier) use walk::ResumeWalk;

/// The devices on which a resume checks loaded directories by their stat and keeps them: the
/// root's, and those of the mounts its job enters ([`JobOptions::enter`]), whose roots loaded
/// below it are taken as part of the tree rather than scanned again in full.
#[derive(Clone, Debug, Default)]
pub(super) struct Homes {
    devs: Vec<u64>,
    entered: Vec<(u64, u64)>,
}

impl Homes {
    pub(super) fn new(root_dev: u64, enter: &[(u64, u64)]) -> Homes {
        let mut devs = vec![root_dev];
        devs.extend(enter.iter().map(|&(dev, _)| dev));
        Homes {
            devs,
            entered: enter.to_vec(),
        }
    }

    /// Whether loaded directories on `dev` are checked and kept.
    pub(super) fn home(&self, dev: u64) -> bool {
        self.devs.contains(&dev)
    }

    /// Whether mount root `ix` of `tree` is entered: one of the mounts entered, and the
    /// topmost on its device (a second mount of it below is a duplicate, the scan's to judge).
    pub(super) fn enters(&self, tree: &Tree, ix: Ix) -> bool {
        let at = tree.dev_ino(ix);
        if !self.entered.contains(&at) {
            return false;
        }
        let mut up = tree.parent(ix);
        while let Some(p) = up {
            if tree.dev_ino(p).0 == at.0 {
                return false;
            }
            up = tree.parent(p);
        }
        true
    }
}

/// What a resume's rescan keeps from the loaded tree: the loaded directories by device and
/// inode, with the mtime each was loaded with, which its scanner does not read while the mtime
/// holds ([`Keep`]), and the placeholders it added for those it met, which the loaded
/// directories replace when it swaps in, once each is found ([`graft::Finding`]).
#[derive(Default)]
pub(super) struct Kept {
    nodes: HashMap<(u64, u64), (NodeRef, i64)>,
    pub placeholders: Vec<NodeRef>,
    finding: graft::Finding,
}

impl Kept {
    /// What its scanner is told not to read.
    pub(super) fn keep(&self) -> Keep {
        Keep::new(self.nodes.iter().map(|(&k, &(_, mtime))| (k, mtime)))
    }
}

/// What the applier keeps of resumes beside their walks.
#[derive(Default)]
pub(super) struct Resumes {
    /// A walk that has taken the tree over and waits for the check of the directories it
    /// keeps to end.
    pub waiting: Option<Box<ResumeWalk>>,
    /// Directories a cancelled resume did not list again, or check below, as it was to: what
    /// its job still holds at or below them, as loaded, is not refreshed.
    unverified: Vec<NodeRef>,
    /// A rescan swapped in directories it kept from below one that held shared extents, whose
    /// homes may have gone with what it replaced: they are homed again once no resume's rescan
    /// is still to swap in.
    pub(super) rehome: bool,
}

/// Whether `ix` was loaded from a file ([`Engine::load`](crate::Engine::load)) and no resume has
/// taken it over yet: its directory (a file's parent) belongs to no job of this session. Until
/// one has, the tree may be out of date anywhere below `ix`, which the identity the deleter
/// checks again does not show. Deletes, native commands and rescans refuse such nodes (a
/// rescan would scan with options no job of this session has: the resume's are not known yet).
///
/// What a resume guarantees once it has: nothing is deleted while it walks the tree and checks
/// the directories it keeps, nor in a directory one of its rescans is listing again; a
/// directory listed again is as it is now; one it kept as loaded is trusted on its own stat,
/// taken after the journal was asked, being the one it was loaded with (device, inode and an
/// mtime older than the journal position), and on the journal's word that its entries did not
/// change within the same second as that mtime when the position's time is not known. A file
/// changed in place below a kept directory, which moves no directory's mtime, is seen only when
/// the journal names its directory, or by a rescan (the cache's periodic full scan is one). A
/// resume cancelled before it has listed again what it was to leaves that refused to deletes
/// and native commands ([`Applier::unverified`]), rescans allowed.
#[must_use]
pub fn unrefreshed(tree: &Tree, ix: Ix) -> bool {
    dir_of(tree, ix)
        .and_then(|d| tree.dir(d))
        .is_some_and(|d| d.job == JobId(0))
}

/// `ix` if it is a directory, else its parent.
fn dir_of(tree: &Tree, ix: Ix) -> Option<Ix> {
    if tree.dir(ix).is_some() {
        Some(ix)
    } else {
        tree.parent(ix)
    }
}

/// The directories of the tree at `root` that `paths` (relative to it) name, or the nearest
/// existing ancestor of one not in the tree. The subdirectories of a directory the paths go
/// through are looked up by name, once for all of them.
fn changed_dirs(tree: &Tree, root: Ix, paths: &[PathBuf]) -> Vec<Ix> {
    let mut subdirs: HashMap<Ix, HashMap<&[u8], Ix>> = HashMap::new();
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let mut at = root;
        for c in p.components() {
            let std::path::Component::Normal(n) = c else {
                continue;
            };
            let n = dfdu_tree::name_bytes(n);
            let names = subdirs.entry(at).or_insert_with(|| {
                tree.children(at)
                    .filter(|&k| tree.dir(k).is_some())
                    .map(|k| (tree.name(k), k))
                    .collect()
            });
            match names.get(&n[..]) {
                Some(&k) => at = k,
                None => break,
            }
        }
        out.push(at);
    }
    out
}

/// `dirs` without duplicates and without those below another of them.
fn topmost(tree: &Tree, mut dirs: Vec<Ix>) -> Vec<Ix> {
    dirs.sort_unstable_by_key(|d| d.0);
    dirs.dedup();
    let set: HashSet<Ix> = dirs.iter().copied().collect();
    dirs.retain(|&d| {
        let mut cur = tree.parent(d);
        while let Some(p) = cur {
            if set.contains(&p) {
                return false;
            }
            cur = tree.parent(p);
        }
        true
    });
    dirs
}

impl<B: Backend> Applier<B> {
    /// Takes over a tree loaded from a snapshot when the volume's change journal says which
    /// directories changed since (`changed`, relative to the root): the tree is classified as
    /// it stands and held by a finished job with `opts`, a step at a time ([`Work::Resume`]).
    /// Each changed directory, and every directory above it, is then listed again by a rescan
    /// of the topmost: the subdirectories those listings find that the loaded tree has (by
    /// device and inode, with the mtime it was loaded with) are kept as loaded unless listed
    /// again themselves, and new ones are scanned in full. The root is judged on `now`, its
    /// stat taken after the journal was asked, as the other loaded directories are on theirs
    /// ([`Verify`]): scanned again in full when it was replaced, listed again when its mtime
    /// changed or is not older than `since`, or it could not be stat'ed (its rescan says why).
    /// Any other loaded directory is listed again when its stat shows it changed, and its
    /// parent when it is gone. Volumes mounted below keep journals of their own (or none):
    /// their mount roots are scanned again in full.
    pub(super) fn resume_changed(
        &mut self,
        m: &mut Model,
        node: NodeRef,
        opts: JobOptions,
        changed: &[PathBuf],
        (since, now): (u64, Option<&Meta>),
    ) {
        let Some(root) = m
            .tree
            .resolve_visible(node)
            .filter(|&r| m.tree.dir(r).is_some() && m.tree.parent(r).is_none())
        else {
            return self.rescan(m, node, Some(opts), false);
        };
        let mut wanted = changed_dirs(&m.tree, root, changed);
        if self.settings.remote.is_none() {
            let before = i64::try_from(since).unwrap_or(i64::MAX);
            match now {
                Some(now) if (now.dev, now.ino) != m.tree.dev_ino(root) => {
                    return self.rescan(m, node, Some(opts), false);
                }
                Some(now)
                    if now.mtime == m.tree.mtime(root) && (since == 0 || now.mtime < before) => {}
                _ => wanted.push(root),
            }
        }
        // This resume takes the whole tree over, and checks it again. Its rescans enter the
        // mounts it enters.
        self.resume.unverified.clear();
        let entered = self.entered.entry(m.tree.node_ref(root)).or_default();
        for &at in &opts.enter {
            if !entered.contains(&at) {
                entered.push(at);
            }
        }
        let homes = Homes::new(m.tree.dev_ino(root).0, &opts.enter);
        let id = self.next_job;
        self.next_job += 1;
        if let Some(c) = &opts.class {
            m.rules = Some(c.rules.clone());
            m.tree.set_protection(root, c.protection.root_mark());
        }
        let class = opts
            .class
            .clone()
            .map(|c| (c.clone(), ClassifyLoaded::new(&m.tree, root, &c)));
        let stack = if class.is_some() { vec![] } else { vec![node] };
        self.jobs.insert(
            id,
            Job {
                id: JobId(id),
                kind: JobKind::Initial,
                target: node,
                root: node,
                scope: VISIBLE,
                opts,
                map: Pages::default(),
                held_close: HashMap::new(),
                stalled: HashMap::new(),
                submitted_seq: 0,
                phase: JobPhase::Done,
                measures: false,
                kept: None,
            },
        );
        let verify = self
            .settings
            .remote
            .is_none()
            .then(|| Verify::new(&m.tree, root, &wanted, since, homes.clone()));
        let wanted = wanted.into_iter().map(|w| m.tree.node_ref(w)).collect();
        self.work.push_back(Work::Resume(Box::new(ResumeWalk::new(
            id, node, wanted, class, stack, verify, homes,
        ))));
    }

    /// Records that a resume was cancelled before it listed again what it was to at or below
    /// `r`, which leaves the scan incomplete: what its job holds there is not refreshed.
    pub(super) fn unverified_below(&mut self, m: &mut Model, r: NodeRef) {
        m.status.incomplete = true;
        let tree = &m.tree;
        let left = &mut self.resume.unverified;
        left.retain(|&u| tree.resolve_visible(u).is_some());
        if !left.contains(&r) {
            left.push(r);
        }
    }

    /// Whether `ix` is at or below a directory a cancelled resume did not list again or
    /// check below, still held by that resume's job as loaded, or holds one: deletes and
    /// native commands refuse it, as they refuse [`unrefreshed`] nodes, until a rescan has
    /// listed it again.
    pub(super) fn unverified(&self, tree: &Tree, ix: Ix) -> bool {
        let left = &self.resume.unverified;
        if left.is_empty() {
            return false;
        }
        let Some(dir) = dir_of(tree, ix) else {
            return false;
        };
        let job = tree.dir(dir).map(|d| d.job);
        let mut cur = Some(dir);
        while let Some(c) = cur {
            if tree.dir(c).map(|d| d.job) == job && left.contains(&tree.node_ref(c)) {
                return true;
            }
            cur = tree.parent(c);
        }
        left.iter()
            .filter_map(|&u| tree.resolve_visible(u))
            .any(|u| Self::is_ancestor(tree, ix, u))
    }

    /// Why `cmd` is refused on a loaded tree, if it is: everything that would touch the tree
    /// while a resume walks it, checks what it keeps or starts its rescans, a native command
    /// or a rescan of what no resume has taken over yet, and a native command on what a
    /// cancelled resume did not refresh (deletes check each item, [`unrefreshed`] and
    /// [`Applier::unverified`]).
    pub(super) fn resume_refusal(&self, m: &Model, cmd: &Command) -> Option<&'static str> {
        let touches = matches!(
            cmd,
            Command::Rescan { .. }
                | Command::Enter { .. }
                | Command::Resume { .. }
                | Command::Delete { .. }
                | Command::RunNative { dry_run: false, .. }
        );
        if touches && self.resuming() {
            return Some(
                "not yet: the loaded tree is still being taken over; try again in a moment",
            );
        }
        if let Command::RunNative {
            node,
            dry_run: false,
            ..
        } = cmd
            && let Some(ix) = m.tree.resolve_visible(*node)
        {
            if unrefreshed(&m.tree, ix) {
                return Some(
                    "native command: not refreshed yet: loaded from a file, and no resume has taken it over",
                );
            }
            if self.unverified(&m.tree, ix) {
                return Some(UNVERIFIED_NATIVE);
            }
        }
        if let Command::Rescan { node } | Command::Enter { node } = cmd
            && m.tree
                .resolve_visible(*node)
                .is_some_and(|ix| unrefreshed(&m.tree, ix))
        {
            return Some(
                "rescan: not refreshed yet: loaded from a file, and no resume has taken it over",
            );
        }
        None
    }
}

/// Why a native command refuses what a cancelled resume did not refresh.
const UNVERIFIED_NATIVE: &str = "native command: not refreshed yet: its resume was cancelled before listing it again; rescan it first";

#[cfg(test)]
mod tests;
