//! Copying what one frame needs out of the engine model. This runs under the read lock, so it
//! only reads and copies; drawing happens after the lock is released.

use std::collections::HashSet;

use dfdu_engine::{DeleteSummary, JobKind, JobPhase, Model, SizeMode, subvol_label};
use dfdu_tree::filetype::{self, Breakdown, TypeFilter};
use dfdu_tree::{
    DirId, DirState, HostPath, Ix, JobId, Kind, NodeRef, Target, Totals, Tree, find_path,
};

mod tree;

#[cfg(test)]
mod tests {
    use super::*;
    use dfdu_engine::test_support::{add_dir, add_file, meta};

    #[test]
    fn snapshots_follow_the_owning_root() {
        let mut m = Model::default();
        let first = m.tree.new_root(b"/first", &meta(Kind::Dir, 0, 1), true);
        let second = m.tree.new_root(b"/second", &meta(Kind::Dir, 0, 2), true);
        let child1 = add_dir(&mut m.tree, first, "child", 3);
        let child2 = add_dir(&mut m.tree, second, "child", 4);
        add_file(&mut m.tree, first, "one", 10, 5);
        add_file(&mut m.tree, second, "two", 20, 6);
        let roots = vec![m.tree.host_path(first), m.tree.host_path(second)];

        for (dir, root, at_root) in [
            (first, first, true),
            (child1, first, false),
            (second, second, true),
            (child2, second, false),
        ] {
            let mut target = Target::of(&m.tree, dir);
            let snap = build(&m, &mut target, SizeMode::Disk);
            assert_eq!(snap.roots, roots);
            assert_eq!(snap.dir_path, m.tree.host_path(dir));
            assert_eq!(snap.root_path, m.tree.host_path(root));
            assert_eq!(snap.root_total, m.tree.total(root));
            assert_eq!(snap.at_root, at_root);
        }
    }
}

/// How an entry is marked in the list.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Mark {
    None,
    /// A directory still being scanned.
    Scanning,
    /// A directory whose scan is not responding.
    Stalled,
    /// A directory being rescanned.
    Rescanning,
    /// The entry or directory has an error, or its scan ended early.
    Error,
    /// A directory on another filesystem that was not scanned.
    OtherFs,
    /// A second mount of something the scan counts elsewhere; not scanned.
    DupMount,
    /// A directory read in full, with errors somewhere below it.
    ErrorsInside,
    /// A network filesystem, not scanned without `--network`.
    Network,
    /// A kernel filesystem (`/proc`, `/sys`), never scanned.
    Kernel,
    /// In a delete plan that has not finished.
    Deleting,
    /// Its scan was stopped (cancelled, or cut for a rescan or delete that did not complete):
    /// the size shown is a lower bound. Rescan with `r`.
    Incomplete,
}

/// An entry name. Short names (most of them) are stored inline, so that copying a directory
/// of a million entries into a snapshot, and dropping the previous one, does not allocate and
/// free a million times.
#[derive(Clone)]
pub enum Name {
    Inline(u8, [u8; Name::INLINE]),
    Heap(Box<[u8]>),
}

impl Name {
    const INLINE: usize = 22;
}

impl From<&[u8]> for Name {
    fn from(b: &[u8]) -> Self {
        if b.len() <= Name::INLINE {
            let mut buf = [0; Name::INLINE];
            buf[..b.len()].copy_from_slice(b);
            Name::Inline(u8::try_from(b.len()).unwrap_or(u8::MAX), buf)
        } else {
            Name::Heap(b.into())
        }
    }
}

impl std::ops::Deref for Name {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Name::Inline(n, buf) => &buf[..*n as usize],
            Name::Heap(b) => b,
        }
    }
}

impl std::fmt::Debug for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(self).fmt(f)
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Name) -> bool {
        self[..] == other[..]
    }
}

impl Eq for Name {}

impl<const N: usize> PartialEq<&[u8; N]> for Name {
    fn eq(&self, other: &&[u8; N]) -> bool {
        self[..] == other[..]
    }
}

/// What a row counts as for cleanup: the class of the rule it matched, or of the match above
/// it that covers it (`dfdu_rules::RuleSet::class_in_effect`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    pub class: dfdu_tree::Class,
    /// The match is above the row, not the row itself.
    pub inherited: bool,
}

impl Tag {
    /// The short name of its category (`build` for `dev.build`) and the theme's colour slot.
    pub fn category(self, rules: &dfdu_rules::RuleSet) -> (&'static str, usize) {
        let c = rules.rule(self.class.rule).category();
        (c.id.rsplit('.').next().unwrap_or(c.id), c.color as usize)
    }

    /// A match offered for cleanup without review: safe or regenerable (a match version
    /// control tracks is protected).
    pub fn candidate(self) -> bool {
        self.class.safety <= dfdu_tree::Safety::Regenerable
    }
}

/// The class in effect at node `c` given `above`, the one at its parent
/// ([`dfdu_rules::RuleSet::class_below`]). Without the rules no match is known to stop the
/// descent.
pub fn class_below(
    m: &Model,
    above: Option<(Ix, dfdu_tree::Class)>,
    c: Ix,
) -> Option<(Ix, dfdu_tree::Class)> {
    match &m.rules {
        Some(r) => r.class_below(&m.tree, above, c),
        None => m.tree.class_below(above, c, |_| false),
    }
}

/// The tag of node `c` of a directory whose own class in effect is `dir`. Without classes
/// this is one check of an empty map.
pub fn tag_of(m: &Model, dir: Option<(Ix, dfdu_tree::Class)>, c: Ix) -> Option<Tag> {
    let (at, class) = class_below(m, dir, c)?;
    Some(Tag {
        class,
        inherited: at != c,
    })
}

#[derive(Clone, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about one row, copied out of the tree"
)]
pub struct EntryData {
    pub node: NodeRef,
    pub name: Name,
    pub kind: Kind,
    pub is_dir: bool,
    /// Size in the current [`SizeMode`].
    pub size: u64,
    pub items: u64,
    pub mtime: i64,
    pub dev_ino: (u64, u64),
    pub mark: Mark,
    /// What went wrong reading it ([`dfdu_engine::problem`]).
    pub problem: Option<String>,
    /// Errors below it, not counting its own.
    pub errors_inside: u64,
    /// btrfs subvolume label ([`subvol_label`]).
    pub label: Option<String>,
    pub tag: Option<Tag>,
    /// Bytes of its disk usage that data outside the scan also uses: deleting it frees less.
    pub shared_out: u64,
    /// A file with compressed extents whose on-disk size is not known (an upper bound).
    pub compressed: bool,
    /// The "shared between entries" row: data counted at the directory itself because entries
    /// share it. Not an entry; it cannot be opened or deleted.
    pub pseudo: bool,
    /// A deleted entry kept in its row until the next re-sort, drawn struck through. Always
    /// also `pseudo` and never `is_dir`, so it cannot be opened or deleted either.
    pub ghost: bool,
    /// Tree mode: how many directories down from the current one, zero for its own entries.
    pub depth: u8,
    /// Tree mode: the index of the parent's row in the entries, `u32::MAX` at the top.
    pub parent: u32,
    /// Tree mode: the number of small children this row stands for (a fold row, also
    /// `pseudo`), zero for any other row.
    pub fold: u32,
}

impl EntryData {
    pub fn hidden(&self) -> bool {
        self.name.first() == Some(&b'.')
    }
}

#[derive(Clone, Debug, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about the engine and the directory, copied out of the model"
)]
pub struct Snapshot {
    /// The rules the scan classifies against, when it does: rows have a category column.
    pub rules: Option<std::sync::Arc<dfdu_rules::RuleSet>>,
    pub dir_path: HostPath,
    /// Paths of all scan roots, in tree order.
    pub roots: Vec<HostPath>,
    /// Path of the scan root owning `dir_path`.
    pub root_path: HostPath,
    pub dir_node: Option<NodeRef>,
    pub at_root: bool,
    pub dir_total: Totals,
    pub dir_mark: Option<Mark>,
    pub root_total: Totals,
    pub entries: Vec<EntryData>,
    /// The engine has work: a scan, a delete, or what follows them.
    pub busy: bool,
    /// A scan or rescan runs: what the scan's clock and its "done" go by.
    pub scanning: bool,
    pub stalled: usize,
    pub incomplete: bool,
    pub notice: Option<String>,
    /// `(done, total)` of the running delete plan.
    pub deleting: Option<(usize, usize)>,
    /// What [`Command::Undo`](dfdu_engine::Command::Undo) would put back.
    pub undo: Option<dfdu_engine::UndoInfo>,
    pub undoing: bool,
    /// The latest estimate of what a delete would free.
    pub estimate: Option<dfdu_engine::Estimate>,
    pub last_delete: Option<DeleteSummary>,
    /// `(done, requested)` of the shared-extent measurement in progress.
    pub measuring: Option<(u64, u64)>,
    /// Files whose extents could not be measured.
    pub unmeasured: u64,
    /// Compressed files are shown uncompressed (an upper bound); exact as root.
    pub compressed_upper_bound: bool,
    /// btrfs data profile and raw bytes per byte of data.
    pub profile: Option<(dfdu_sys::Profile, f64)>,
    /// `(job, dir_id)` of the current directory, for scan prioritisation.
    pub focus: Option<(JobId, DirId)>,
    /// Cheap fingerprint of the model, to skip redraws when nothing changed.
    pub version: u64,
    /// The file type filter the entries were read with (`overlay::type_filter`); empty: none. With
    /// one, entries and `dir_total` count the matching files only.
    pub types: TypeFilter,
    /// The directory's files by type and extension, read while the type picker is open.
    pub breakdown: Option<std::sync::Arc<Breakdown>>,
    /// Tree mode: what the read found of the expanded directories (`state::tree`). `None`
    /// when tree mode is off.
    pub tree: Option<crate::tree::Found>,
}

pub(crate) fn mark_of(tree: &Tree, ix: Ix, rescanning: &HashSet<NodeRef>) -> Mark {
    if tree.flags(ix).has(dfdu_tree::NodeFlags::DELETING) {
        return Mark::Deleting;
    }
    if tree.flags(ix).has(dfdu_tree::NodeFlags::DUP_MOUNT) {
        return Mark::DupMount;
    }
    if tree.flags(ix).has(dfdu_tree::NodeFlags::KERNEL) {
        return Mark::Kernel;
    }
    if tree.flags(ix).has(dfdu_tree::NodeFlags::NETWORK) {
        return Mark::Network;
    }
    if !rescanning.is_empty() && rescanning.contains(&tree.node_ref(ix)) {
        return Mark::Rescanning;
    }
    match tree.dir(ix) {
        Some(d) => match d.state {
            DirState::Pending if d.stalled => Mark::Stalled,
            DirState::Pending => Mark::Scanning,
            DirState::Skipped => Mark::OtherFs,
            DirState::Failed | DirState::Partial | DirState::Abandoned | DirState::Lost => {
                Mark::Error
            }
            DirState::Cancelled if tree.total(ix).items <= 1 => Mark::Error,
            _ if d.cut || matches!(d.state, DirState::Cancelled | DirState::Superseded) => {
                Mark::Incomplete
            }
            _ if tree.error(ix).is_some() => Mark::Error,
            _ if tree.total(ix).errors > 0 => Mark::ErrorsInside,
            _ => Mark::None,
        },
        None if tree.error(ix).is_some() => Mark::Error,
        None => Mark::None,
    }
}

/// Builds the snapshot for the directory `target` points at, updating `target` if the node had
/// to be found again (rescan swap) or no longer exists (falls back to the nearest ancestor).
#[cfg(any(test, feature = "test-support"))]
pub fn build(m: &Model, target: &mut Target, mode: SizeMode) -> Snapshot {
    build_view(m, target, mode, &TypeFilter::default(), false, None)
}

/// `build` counting only the files `types` matches (none left out when it is empty), and with
/// the directory's [`Breakdown`] when `breakdown` is set. Both walk the directory's subtree.
/// With `tree_state` set (tree mode), the rows of the expanded directories follow the
/// directory's own entries (`tree::expand`).
pub fn build_view(
    m: &Model,
    target: &mut Target,
    mode: SizeMode,
    types: &TypeFilter,
    breakdown: bool,
    tree_state: Option<&crate::tree::TreeState>,
) -> Snapshot {
    let tree = &m.tree;
    let mut snap = status_of(m, types);
    // Read for this tree state even when there is nothing to show yet, so it is not stale.
    snap.tree = tree_state.map(|t| crate::tree::Found {
        generation: t.generation,
        ..Default::default()
    });
    snap.roots = tree.roots().map(|root| tree.host_path(root)).collect();
    let Some(fallback) = tree.roots().next() else {
        return snap;
    };

    let mut dir = target.resolve(tree);
    // The directory is gone: go up until something resolves.
    let mut path = target.path.parent();
    while dir.is_none()
        && let Some(p) = path
    {
        dir = find_path(tree, &p);
        path = p.parent();
    }
    let dir = dir.filter(|&d| tree.dir(d).is_some()).unwrap_or(fallback);
    let mut root = dir;
    while let Some(parent) = tree.parent(root) {
        root = parent;
    }
    snap.root_path = tree.host_path(root);
    snap.root_total = tree.total(root);
    *target = Target::of(tree, dir);
    let Some(rec) = tree.dir(dir) else {
        return snap;
    };

    let rescanning: HashSet<NodeRef> = m
        .status
        .jobs
        .iter()
        .filter(|j| {
            j.kind == JobKind::Rescan && matches!(j.phase, JobPhase::Running | JobPhase::Finishing)
        })
        .map(|j| j.target)
        .collect();

    snap.focus = Some((rec.job, rec.dir_id));
    snap.dir_path = tree.host_path(dir);
    snap.dir_node = Some(tree.node_ref(dir));
    snap.at_root = dir == root;
    snap.dir_total = tree.total(dir);
    snap.dir_mark = Some(mark_of(tree, dir, &rescanning));
    let dir_class = m.rules.as_ref().and_then(|r| r.class_in_effect(tree, dir));
    // This runs under the read lock for every entry of the directory, which can have a
    // million: each lookup is done once, and the error and subvolume lookups only when the
    // entry has an error or the scan has subvolumes.
    snap.entries = tree
        .children(dir)
        .map(|c| entry_of(m, c, dir_class, &rescanning, mode))
        .collect();
    if breakdown {
        let mut b = Breakdown::default();
        filetype::sum(tree, dir, &TypeFilter::default(), Some(&mut b));
        snap.breakdown = Some(std::sync::Arc::new(b));
    }
    if types.is_empty() {
        // Data the entries share is counted at the directory: show it as its own row.
        let home = tree.shared_here(dir);
        if mode == SizeMode::Disk && home > 0 {
            snap.entries.push(shared_row(tree, dir, home));
        }
    } else {
        only_types(tree, dir, types, mode, &mut snap);
    }
    if let Some(state) = tree_state {
        let read = tree::Read {
            m,
            mode,
            types,
            rescanning: &rescanning,
            dir_class,
        };
        tree::expand(&read, state, &mut snap);
    }
    snap
}

/// A snapshot with the engine's status and nothing of a directory yet.
fn status_of(m: &Model, types: &TypeFilter) -> Snapshot {
    Snapshot {
        busy: m.status.busy(),
        scanning: m.status.scanning(),
        stalled: m.status.stalled.len(),
        incomplete: m.status.incomplete,
        notice: m.status.notices.last().cloned(),
        deleting: m.status.deleting.as_ref().map(|d| (d.done, d.total)),
        undo: m.status.undo.clone(),
        undoing: m.status.undoing.is_some(),
        estimate: m.status.estimate.clone(),
        last_delete: m.status.last_delete.clone(),
        measuring: m.status.measuring.as_ref().map(|p| (p.done, p.requested)),
        unmeasured: m.status.unmeasured,
        compressed_upper_bound: m.status.compressed_upper_bound,
        profile: m.status.profile,
        version: m.fingerprint(),
        rules: m.rules.clone(),
        types: types.clone(),
        ..Default::default()
    }
}

/// The row of child `c` of the directory shown.
fn entry_of(
    m: &Model,
    c: Ix,
    dir_class: Option<(Ix, dfdu_tree::Class)>,
    rescanning: &HashSet<NodeRef>,
    mode: SizeMode,
) -> EntryData {
    let tree = &m.tree;
    let t = tree.total(c);
    let error = tree.error(c);
    EntryData {
        node: tree.node_ref(c),
        name: tree.name(c).into(),
        kind: tree.kind(c),
        is_dir: tree.dir(c).is_some(),
        size: mode.of(&t),
        items: t.items,
        mtime: tree.mtime(c),
        dev_ino: tree.dev_ino(c),
        mark: mark_of(tree, c, rescanning),
        problem: error.and_then(|_| dfdu_engine::problem(tree, c)),
        errors_inside: t.errors - u64::from(error.is_some()),
        label: if m.subvols.is_empty() {
            None
        } else {
            subvol_label(m, c)
        },
        tag: tag_of(m, dir_class, c),
        shared_out: t.shared_out,
        compressed: tree.flags(c).has(dfdu_tree::NodeFlags::COMPRESSED),
        pseudo: false,
        ghost: false,
        depth: 0,
        parent: u32::MAX,
        fold: 0,
    }
}

/// The row for the `home` bytes the entries of `dir` share, counted at the directory.
fn shared_row(tree: &Tree, dir: Ix, home: u64) -> EntryData {
    EntryData {
        node: tree.node_ref(dir),
        name: b"(shared between entries)"[..].into(),
        kind: Kind::Other,
        is_dir: false,
        size: home,
        items: 0,
        mtime: 0,
        dev_ino: (0, 0),
        mark: Mark::None,
        problem: None,
        errors_inside: 0,
        label: None,
        tag: None,
        shared_out: 0,
        compressed: false,
        pseudo: true,
        ghost: false,
        depth: 0,
        parent: u32::MAX,
        fold: 0,
    }
}

/// Keeps the entries with files `types` matches, sized by those files, and sets `dir_total` to
/// their sum (errors and the rest of the directory's own totals kept).
fn only_types(tree: &Tree, dir: Ix, types: &TypeFilter, mode: SizeMode, snap: &mut Snapshot) {
    let mut sum = Totals::default();
    snap.entries.retain_mut(|e| {
        let Some(c) = tree.resolve_visible(e.node) else {
            return false;
        };
        let t = filetype::sum(tree, c, types, None);
        if t.items == 0 {
            return false;
        }
        sum.add(&t);
        e.size = mode.of(&t);
        e.items = t.items;
        e.shared_out = t.shared_out;
        true
    });
    let d = tree.total(dir);
    snap.dir_total = Totals {
        errors: d.errors,
        mounts: d.mounts,
        ro_subvols: d.ro_subvols,
        ..sum
    };
}

/// An entry with a unique node reference, for tests that do not need a real tree.
#[cfg(any(test, feature = "test-support"))]
pub fn test_entry(i: u32, name: &str, size: u64) -> EntryData {
    use dfdu_engine::test_support::meta;
    let mut t = Tree::new();
    let root = t.new_root(b"/", &meta(Kind::Dir, 0, 1), true);
    let file = meta(Kind::File, 0, 1);
    let mut last = root;
    for _ in 0..=i {
        last = t.add_file(root, b"x", &file, false, None);
    }
    EntryData {
        node: t.node_ref(last),
        name: name.as_bytes().into(),
        kind: Kind::File,
        is_dir: false,
        size,
        items: 1,
        mtime: 0,
        dev_ino: (1, u64::from(i) + 10),
        mark: Mark::None,
        problem: None,
        errors_inside: 0,
        label: None,
        tag: None,
        shared_out: 0,
        compressed: false,
        pseudo: false,
        ghost: false,
        depth: 0,
        parent: u32::MAX,
        fold: 0,
    }
}
