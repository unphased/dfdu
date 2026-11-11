//! A resume's rescans, started a few at a time once its walk and its check have ended: the
//! rescan of each target lists again the changed directories below it and every directory
//! above them up to it, keeping the loaded subdirectories those listings find, which are
//! gathered from the loaded tree a step at a time. Until all have started, nothing that would
//! touch the tree is taken ([`Applier::resuming`]); none starts once the resume is cancelled.

use std::collections::{HashMap, HashSet};

use dfdu_scan::Backend;
use dfdu_tree::{DirState, Ix, NodeFlags, NodeRef, Tree};

use super::walk::ResumeWalk;
use super::{Homes, Kept, topmost};
use crate::applier::work::{STEP_UNITS, Work};
use crate::applier::{Applier, Model};

/// What starting one rescan costs, in the units of a step ([`STEP_UNITS`]): about as much as
/// looking at this many loaded children.
const RESCAN_UNITS: usize = 32;

/// The rescans of a resume still to start.
pub(super) struct Rescans {
    /// The devices whose directories the directories listed again keep: the root's and the
    /// entered mounts'.
    homes: Homes,
    /// Each target still to start (the next last) with the directories its rescan lists
    /// again: none for a mount, which is scanned in full, keeping nothing.
    targets: Vec<(NodeRef, Vec<NodeRef>)>,
    /// What the next target keeps, gathered so far.
    gather: Option<Gather>,
}

/// The loaded subdirectories a rescan keeps, gathered from the directories it lists again: of
/// those, the ones not listed again themselves, read to the end, and on the root's device and
/// no mount.
struct Gather {
    listed: HashSet<NodeRef>,
    /// The listed directories not looked through yet, the next last, and how many children of
    /// the next were.
    left: Vec<NodeRef>,
    at: usize,
    kept: Box<Kept>,
}

impl Gather {
    fn new(listed: Vec<NodeRef>) -> Gather {
        Gather {
            listed: listed.iter().copied().collect(),
            left: listed,
            at: 0,
            kept: Box::default(),
        }
    }

    /// Looks at up to `budget` children; how many it did, and whether it has seen all.
    fn step(&mut self, tree: &Tree, homes: &Homes, budget: usize) -> (usize, bool) {
        let mut n = 0;
        while n < budget
            && let Some(&l) = self.left.last()
        {
            let Some(lx) = tree
                .resolve_visible(l)
                .filter(|&l| homes.home(tree.dev_ino(l).0))
            else {
                self.left.pop();
                self.at = 0;
                continue;
            };
            let want = budget - n;
            let mut took = 0;
            for c in tree.children(lx).skip(self.at).take(want) {
                took += 1;
                let keep = tree.dir(c).is_some_and(|d| d.state == DirState::Complete)
                    && homes.home(tree.dev_ino(c).0)
                    && !tree.flags(c).has(NodeFlags::MOUNT_ROOT)
                    && !self.listed.contains(&tree.node_ref(c));
                if keep {
                    self.kept
                        .nodes
                        .insert(tree.dev_ino(c), (tree.node_ref(c), tree.mtime(c)));
                }
            }
            n += took;
            if took < want {
                self.left.pop();
                self.at = 0;
            } else {
                self.at += took;
            }
        }
        (n, self.left.is_empty())
    }
}

/// For each of `targets` (the topmost of `wanted`), the directories its rescan lists again:
/// those of `wanted` below it and every one above them up to it.
fn listed_by_target(tree: &Tree, wanted: &[Ix], targets: &[Ix]) -> Vec<Vec<NodeRef>> {
    let at: HashMap<Ix, usize> = targets.iter().enumerate().map(|(i, &t)| (t, i)).collect();
    // Each directory listed again, with the index of its target.
    let mut listed: HashMap<Ix, usize> = HashMap::new();
    for &d in wanted {
        let mut path = Vec::new();
        let mut cur = d;
        let target = loop {
            if let Some(&i) = listed.get(&cur).or_else(|| at.get(&cur)) {
                break Some(i);
            }
            path.push(cur);
            match tree.parent(cur) {
                Some(p) => cur = p,
                None => break None,
            }
        };
        if let Some(i) = target {
            listed.insert(cur, i);
            listed.extend(path.into_iter().map(|p| (p, i)));
        }
    }
    let mut out = vec![Vec::new(); targets.len()];
    for (l, i) in listed {
        out[i].push(tree.node_ref(l));
    }
    out
}

/// What the rescan of each of `targets` (the topmost of `wanted`) keeps, gathered at once;
/// nothing for a target in `mounts`, which is scanned in full.
#[cfg(test)]
pub(super) fn kept_by_target(
    tree: &Tree,
    root: Ix,
    wanted: &[Ix],
    targets: &[Ix],
    mounts: &[Ix],
) -> Vec<Kept> {
    let homes = Homes::new(tree.dev_ino(root).0, &[]);
    let listed = listed_by_target(tree, wanted, targets);
    targets
        .iter()
        .zip(listed)
        .map(|(t, l)| {
            let mut g = Gather::new(if mounts.contains(t) { Vec::new() } else { l });
            while !g.step(tree, &homes, usize::MAX).1 {}
            *g.kept
        })
        .collect()
}

impl<B: Backend> Applier<B> {
    /// Ends a resume's walk: the loaded files are measured where files can share extents, and
    /// the rescans of the changed directories and of the mounts below are set up, to start a
    /// few at a time ([`Applier::rescans_step`]). None when the resume was cancelled, which
    /// leaves the loaded tree unverified ([`Applier::unverified`]).
    pub(super) fn resume_rescans(&mut self, m: &mut Model, root: Ix, walk: &ResumeWalk) -> Rescans {
        let tree = &m.tree;
        let all: Vec<Ix> = walk
            .mounts
            .iter()
            .filter_map(|&r| tree.resolve_visible(r))
            .collect();
        // Those the job enters are part of the tree: checked and kept as the root's device is.
        let (entered, mounts): (Vec<Ix>, Vec<Ix>) =
            all.into_iter().partition(|&ix| walk.homes.enters(tree, ix));
        let mut wanted: Vec<Ix> = walk
            .wanted
            .iter()
            .filter_map(|&r| tree.resolve_visible(r))
            .collect();
        // Snapshots keep no measured extents: the loaded files are measured again where files
        // can share them (rescanned parts are measured when their rescans end).
        for ix in std::iter::once(root)
            .chain(entered)
            .chain(mounts.iter().copied())
        {
            let dev = m.tree.dev_ino(ix).0;
            if !self.extent_devs.contains(&dev) {
                let loc = self.scan.backend.loc_from_path(&m.tree.path_buf(ix));
                if self.scan.backend.shares_extents(&loc) {
                    self.extent_devs.insert(dev);
                }
            }
        }
        self.start_measure(m, walk.root);
        // The loaded tree entered them: so do the rescans whose listings meet them.
        let mut top = root;
        while let Some(parent) = m.tree.parent(top) {
            top = parent;
        }
        let entered = self.entered.entry(m.tree.node_ref(top)).or_default();
        for &ix in &mounts {
            let at = m.tree.dev_ino(ix);
            if !entered.contains(&at) {
                entered.push(at);
            }
        }
        m.tree.bump_version();
        self.work.push_back(Work::FinalPassVisible);
        self.queue_checks(m);
        let mut out = Rescans {
            homes: walk.homes.clone(),
            targets: Vec::new(),
            gather: None,
        };
        if walk.cancelled {
            self.unverified_below(m, walk.root);
            return out;
        }
        wanted.extend(mounts.iter().copied());
        let targets = topmost(&m.tree, wanted.clone());
        let listed = listed_by_target(&m.tree, &wanted, &targets);
        out.targets = targets
            .into_iter()
            .zip(listed)
            .map(|(t, l)| {
                let l = if mounts.contains(&t) { Vec::new() } else { l };
                (m.tree.node_ref(t), l)
            })
            .collect();
        out
    }

    /// Starts a step's worth of a resume's rescans, gathering what each keeps first; whether
    /// all have started. A cancel stops it: what is left is not refreshed.
    pub(super) fn rescans_step(&mut self, m: &mut Model, walk: &mut ResumeWalk) -> bool {
        let Some(r) = walk.rescans.as_mut() else {
            return true;
        };
        if walk.cancelled {
            if !r.targets.is_empty() {
                r.targets.clear();
                r.gather = None;
                self.unverified_below(m, walk.root);
            }
            return true;
        }
        let mut n = 0;
        while n < STEP_UNITS
            && let Some((t, listed)) = r.targets.last_mut()
        {
            let t = *t;
            let g = r
                .gather
                .get_or_insert_with(|| Gather::new(std::mem::take(listed)));
            let (looked, all) = g.step(&m.tree, &r.homes, STEP_UNITS - n);
            n += looked + RESCAN_UNITS;
            if !all {
                break;
            }
            let kept = r.gather.take().map(|g| g.kept);
            r.targets.pop();
            self.rescan_keeping(m, t, None, false, kept);
        }
        r.targets.is_empty()
    }
}
