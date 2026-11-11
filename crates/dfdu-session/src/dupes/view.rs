//! The Duplicates view: switching it on compares nothing. It says how many files a comparison
//! would read ([`count`]), and asking ([`Intent::Compare`]) starts one in a thread of its own
//! from the candidates read under the read lock ([`Effect::FindDupes`], then [`begin`]); closing
//! the view or shutting the session down stops it ([`cancel`]). Ticks are the marks: a tick on
//! a group ticks every copy but the one kept, and [`Intent::TickSuggested`] does that for every
//! group that frees something.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use dfdu_dupes::{Candidate, Found, Group, Options};
use dfdu_engine::Model;
use dfdu_tree::{HostPath, NodeRef};

use super::rows::{Row, mark_all, mark_of, open, rows};
use super::{Keep, Nodes, Run, copies, kept, live};
use crate::actions::{Action, first_key};
use crate::effect::Effect;
use crate::marks;
use crate::state::{READ_ONLY, SessionState};
use crate::view::{Then, View};

/// Files smaller than this are not compared: the size `dfdu dupes` starts from.
pub fn min_size() -> u64 {
    Options::default().min_size
}

/// What the view keeps between frames.
#[derive(Clone, Debug, Default)]
pub struct Dupes {
    /// The selected row.
    pub cursor: usize,
    /// The comparison asked for, running or done.
    pub run: Option<Run>,
    pub keep: Keep,
    /// Groups opened or closed against their default (open, but clones closed), by index in
    /// the result; cleared by a new comparison.
    pub toggled: HashSet<usize>,
    /// How many files a comparison would read, counted while the view waits to be asked.
    pub candidates: Option<usize>,
    /// The tree's version the candidates were counted at.
    counted_at: Option<u64>,
    /// The groups with two files left, counted when the files' nodes were last read; `None`
    /// while the view is closed.
    live_groups: Option<usize>,
    /// The suggested copies were asked for before the result's files had nodes: they are
    /// marked once a read brings them ([`apply`]).
    tick_waits: bool,
}

/// What a front end's keys ask of the view.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Intent {
    /// Starts the first comparison.
    Compare,
    /// Puts the cursor on row `n` of [`rows`].
    Select(usize),
    /// Ticks or unticks the selected file, or every copy of the selected group but the one kept.
    Toggle,
    /// Opens or closes the selected group, or shows the selected file in Files.
    Activate,
    /// Closes the group at or above the cursor, and selects it.
    CloseUp,
    /// Stops the comparison that runs, if one does, and starts another.
    CompareAgain,
    /// The next keep rule.
    CycleKeep,
    /// Marks every copy but the one kept in each group that frees something.
    TickSuggested,
}

/// What the comparison found, once it is done.
pub fn found(s: &SessionState) -> Option<&Found> {
    s.dupes.run.as_ref()?.result.get()
}

/// True when the view is shown, and so takes the intents it knows. Nothing is compared until
/// the user asks: before that the view says how many files a comparison would read.
pub fn active(s: &SessionState) -> bool {
    s.current_view() == View::Duplicates
}

/// A comparison is running: the view's progress line needs frames.
pub fn running(s: &SessionState) -> bool {
    active(s) && (s.dupes.run.as_ref()).is_some_and(|r| r.result.get().is_none())
}

/// Where a comparison is, for the view's tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Groups {
    Running,
    /// It ended with this many groups.
    Found(usize),
}

/// For the view's tab: `None` before a comparison was started. While the view is open only
/// groups with two files left count; closed, the files are not looked up and every group found
/// counts.
pub fn groups(s: &SessionState) -> Option<Groups> {
    let run = s.dupes.run.as_ref()?;
    let Some(f) = run.result.get() else {
        return Some(Groups::Running);
    };
    Some(Groups::Found(s.dupes.live_groups.unwrap_or(f.groups.len())))
}

/// A comparison was asked for: it runs or it is done.
pub fn compared(s: &SessionState) -> bool {
    s.dupes.run.is_some()
}

/// The view waits to be asked to compare this machine's files, and the scan is done.
fn waits(s: &SessionState) -> bool {
    active(s) && s.outside.local && s.dupes.run.is_none() && !s.snap.busy
}

/// The view waits, and the files to compare were never counted.
fn wants_count(s: &SessionState) -> bool {
    waits(s) && s.dupes.candidates.is_none()
}

/// Under the read lock, while the view waits to be asked: how many files a comparison would
/// read, and the tree's version. A walk of the scan without building paths, once per change
/// of the tree: passing through the view again counts nothing.
pub fn count(m: &Model, s: &SessionState) -> Option<(u64, usize)> {
    let version = m.tree.version();
    if !waits(s) || s.dupes.counted_at == Some(version) {
        return None;
    }
    Some((
        version,
        match (m.tree.roots().next(), m.tree.roots().nth(1)) {
            (Some(r), None) => dfdu_dupes::count_candidates(&m.tree, r, min_size()),
            (None, _) => 0,
            _ => crate::effect::duplicate_candidates(&m.tree).len(),
        },
    ))
}

/// Takes in what [`count`] found, and counts the groups left for the view's tab.
pub fn apply(s: &mut SessionState) {
    if let Some((version, n)) = s.extras.dupe_candidates.take() {
        s.dupes.candidates = Some(n);
        s.dupes.counted_at = Some(version);
    }
    let nodes = &s.extras.dupes;
    let live_groups = found(s).filter(|_| !nodes.is_empty()).map(|f| {
        let left = |g: &&Group| g.files.iter().filter(|c| live(nodes, c).is_some()).count() >= 2;
        f.groups.iter().filter(left).count()
    });
    s.dupes.live_groups = live_groups;
    if s.dupes.tick_waits && (!active(s) || nodes_read(s)) {
        s.dupes.tick_waits = false;
        if active(s) {
            tick_suggested(s);
        }
    }
}

/// Why a comparison cannot start now, as the status line says it; `None` when it can.
fn cannot_start(s: &SessionState) -> Option<String> {
    // The comparison reads the files here: for another machine's tree it would read this
    // one's files at the same paths, and a delete would then remove the other's.
    if !s.outside.local {
        return Some(
            "duplicates reads this machine's files; this scan is remote or from a file".into(),
        );
    }
    if s.snap.busy {
        let again = first_key(Action::CompareAgain);
        return Some(format!(
            "duplicates: wait until the scan is done, then press {again}"
        ));
    }
    None
}

/// Starts a comparison: the front end reads the candidates ([`Effect::FindDupes`]) and passes
/// them to [`begin`]. The cursor goes back to the top.
fn start(s: &mut SessionState) -> Vec<Effect> {
    if let Some(why) = cannot_start(s) {
        s.status = Some(why);
        return Vec::new();
    }
    s.dupes.cursor = 0;
    s.dupes.toggled.clear();
    s.dupes.tick_waits = false;
    s.extras.dupes.clear();
    vec![Effect::FindDupes]
}

/// Starts comparing `cands` (read under the lock) in a thread, with as many threads reading as
/// the storage under `root` serves well ([`Run::start`]).
pub fn begin(s: &mut SessionState, root: PathBuf, cands: Vec<Candidate>) {
    match Run::start(root, cands) {
        Ok(run) => s.dupes.run = Some(run),
        Err(e) => s.status = Some(format!("duplicates: cannot start a thread: {e}")),
    }
}

/// The view needs a read: the files to compare are to be counted, or a result's files have no
/// nodes yet.
pub fn stale(s: &SessionState) -> bool {
    wants_count(s) || active(s) && found(s).is_some() && !nodes_read(s)
}

/// Every file of the result has been looked up in the tree ([`read`]); true without a result.
fn nodes_read(s: &SessionState) -> bool {
    found(s).is_none_or(|f| {
        (f.groups.iter())
            .flat_map(|g| &g.files)
            .all(|c| s.extras.dupes.contains_key(&c.path))
    })
}

/// Under the read lock: the node of every file of the result, found once by path and then
/// checked to still be in the tree. `None` once a file is gone from the tree (deleted).
pub fn read(m: &Model, s: &SessionState) -> Nodes {
    let Some(f) = found(s).filter(|_| active(s)) else {
        return HashMap::new();
    };
    f.groups
        .iter()
        .flat_map(|g| &g.files)
        .map(|c| {
            let node = match s.extras.dupes.get(&c.path) {
                Some(Some(n)) => m.tree.resolve_visible(*n).map(|_| *n),
                Some(None) => None,
                None => dfdu_tree::find_path(&m.tree, &HostPath::local(&c.path))
                    .filter(|&ix| m.tree.dev_ino(ix) == c.dev_ino)
                    .map(|ix| m.tree.node_ref(ix)),
            };
            (c.path.clone(), node)
        })
        .collect()
}

/// The path of the selected file; `None` on a group's row.
pub fn selected(s: &SessionState) -> Option<PathBuf> {
    match rows(s).get(s.dupes.cursor)? {
        &Row::File(g, i) => Some(found(s)?.groups[g].files[i].path.clone()),
        Row::Group(_) => None,
    }
}

/// The nodes of the files the view lists (still in the tree), for a delete to act on its ticks
/// only; none before a comparison finished.
pub fn listed(s: &SessionState) -> HashSet<NodeRef> {
    found(s).map_or_else(HashSet::new, |f| {
        (f.groups.iter())
            .flat_map(|g| &g.files)
            .filter_map(|c| live(&s.extras.dupes, c))
            .collect()
    })
}

/// A group whose every copy still in the tree is ticked, by its file's name: deleting what is
/// ticked would leave no copy of it.
pub fn whole_group_ticked(s: &SessionState) -> Option<String> {
    let f = found(s)?;
    let nodes = &s.extras.dupes;
    f.groups.iter().find_map(|g| {
        let left: Vec<NodeRef> = g.files.iter().filter_map(|c| live(nodes, c)).collect();
        let all = left.len() >= 2 && left.iter().all(|&n| s.marks.contains(n));
        all.then(|| {
            let first = g.files.iter().find(|c| live(nodes, c).is_some())?;
            Some(first.path.file_name()?.to_string_lossy().into_owned())
        })
        .flatten()
    })
}

/// Carries out `intent`.
pub fn act(s: &mut SessionState, intent: Intent) -> Then {
    match intent {
        Intent::Compare => return Then::effects(start(s)),
        Intent::CompareAgain => return Then::effects(compare_again(s)),
        Intent::CycleKeep => cycle_keep(s),
        Intent::TickSuggested => tick_suggested(s),
        Intent::Select(row) => s.dupes.cursor = row,
        Intent::Toggle => toggle(s),
        Intent::Activate => return activate(s),
        Intent::CloseUp => close_up(s),
    }
    Then::nothing()
}

/// Ticks or unticks the selected file and moves down a row, or every copy of the selected group
/// but the one kept.
fn toggle(s: &mut SessionState) {
    let rows = rows(s);
    let last = rows.len().saturating_sub(1);
    let Some(f) = found(s) else {
        return;
    };
    match rows.get(s.dupes.cursor) {
        Some(&Row::File(group, file)) => {
            let c = f.groups[group].files[file].clone();
            if let Some(n) = live(&s.extras.dupes, &c) {
                let on = !s.marks.contains(n);
                marks::set(s, mark_of(&c, n), on);
            }
            s.dupes.cursor = (s.dupes.cursor + 1).min(last);
        }
        Some(&Row::Group(group)) => {
            let picks = copies(&f.groups[group], s.dupes.keep, &s.extras.dupes);
            let on = !picks.iter().all(|(_, n)| s.marks.contains(*n));
            for (c, n) in &picks {
                marks::set(s, mark_of(c, *n), on);
            }
        }
        None => {}
    }
}

/// Opens or closes the selected group, or shows the selected file in Files.
fn activate(s: &mut SessionState) -> Then {
    match rows(s).get(s.dupes.cursor) {
        Some(&Row::Group(g)) => {
            let d = &mut s.dupes;
            if !d.toggled.remove(&g) {
                d.toggled.insert(g);
            }
        }
        Some(&Row::File(g, i)) => {
            if let (Some(f), Some(run)) = (found(s), &s.dupes.run) {
                let path = HostPath::local(&f.groups[g].files[i].path);
                return Then::show(path, HostPath::local(&run.root));
            }
        }
        None => {}
    }
    Then::nothing()
}

/// Closes the group at or above the cursor, and selects it.
fn close_up(s: &mut SessionState) {
    let rows = rows(s);
    let last = rows.len().saturating_sub(1);
    let at = (0..=s.dupes.cursor.min(last))
        .rev()
        .find(|&i| matches!(rows.get(i), Some(Row::Group(_))));
    if let Some(i) = at
        && let Row::Group(g) = rows[i]
        && let Some(group) = found(s).map(|f| &f.groups[g])
    {
        if open(s, g, group) {
            s.dupes.toggled.insert(g);
        }
        s.dupes.cursor = i;
    }
}

/// The next keep rule, with the status line saying so. A copy the new rule keeps is unticked:
/// the copy kept is never one that a delete takes.
fn cycle_keep(s: &mut SessionState) {
    s.dupes.keep = s.dupes.keep.next();
    let nodes = &s.extras.dupes;
    let kept_now: Vec<(Candidate, NodeRef)> = found(s)
        .map(|f| {
            (f.groups.iter())
                .filter_map(|g| {
                    let c = &g.files[kept(g, s.dupes.keep, nodes)?];
                    live(nodes, c).map(|n| (c.clone(), n))
                })
                .filter(|(_, n)| s.marks.contains(*n))
                .collect()
        })
        .unwrap_or_default();
    for (c, n) in &kept_now {
        marks::set(s, mark_of(c, *n), false);
    }
    s.status = Some(format!(
        "duplicates: keep the {} copy ({} ticks the others)",
        s.dupes.keep.label(),
        first_key(Action::TickSuggested)
    ));
}

/// Stops the comparison that runs, if one does, and starts another.
fn compare_again(s: &mut SessionState) -> Vec<Effect> {
    // What there is stays shown when a new comparison cannot start.
    if let Some(why) = cannot_start(s) {
        s.status = Some(why);
        return Vec::new();
    }
    if let Some(r) = s.dupes.run.take() {
        r.cancel.store(true, Ordering::Relaxed);
    }
    start(s)
}

/// Stops the comparison that runs, if one does, and forgets it: the view waits to be asked
/// again. A comparison that ended stays. True when one was stopped. Called when the view
/// closes and when the session shuts down: nothing else reads what a comparison finds.
pub fn cancel(s: &mut SessionState) -> bool {
    let Some(r) = s.dupes.run.take_if(|r| r.result.get().is_none()) else {
        return false;
    };
    r.cancel.store(true, Ordering::Relaxed);
    s.dupes.cursor = 0;
    s.dupes.toggled.clear();
    true
}

/// Marks every copy but the one kept in each group that frees something. Asked for in the
/// moment between the result arriving and the read that finds its files in the tree, it waits
/// for that read ([`apply`]) rather than mark nothing.
fn tick_suggested(s: &mut SessionState) {
    if s.read_only {
        s.status = Some(READ_ONLY.into());
        return;
    }
    if !nodes_read(s) {
        s.dupes.tick_waits = true;
        s.status = Some("marking the copies once their files are found in the tree".into());
        return;
    }
    let n = mark_all(s);
    let keep = s.dupes.keep.label();
    s.status = Some(match n {
        0 => "every copy is marked already, or there are none to free".into(),
        1 => format!("marked 1 copy (the {keep} file of each group is kept)"),
        n => format!("marked {n} copies (the {keep} file of each group is kept)"),
    });
}

#[cfg(test)]
mod tests;
