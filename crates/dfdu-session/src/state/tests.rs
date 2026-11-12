//! Helpers for the tests of the logic on a [`SessionState`]: a browser holding a directory
//! with its rows sorted, as a front end has it after a refresh, and one that reads a model as
//! a front end does for a frame.

use std::time::Instant;

use dfdu_engine::Model;
use dfdu_tree::HostPath;

use super::{SessionState, shown};
use crate::snapshot::{EntryData, Snapshot, test_entry};

/// A browser of `/r` holding `entries`, its rows sorted and the first one selected.
pub(crate) fn browser(entries: Vec<EntryData>) -> SessionState {
    let mut s = SessionState::new(Instant::now());
    s.snap = Snapshot {
        dir_path: HostPath::from("/r"),
        dir_node: Some(test_entry(99, "/r", 0).node),
        at_root: true,
        entries,
        ..Default::default()
    };
    sort(&mut s);
    s.select(0);
    s
}

#[test]
fn go_to_an_entry_already_shown_selects_it_without_another_snapshot() {
    let mut child = test_entry(2, "b", 1);
    child.depth = 1;
    let mut s = browser(vec![child, test_entry(0, "a", 10), test_entry(1, "b", 5)]);
    let root = s.snap.dir_path.clone();
    for name in [b"b", b"a"] {
        let path = root.join(name);
        s.go_to(&path, &root).unwrap();
        assert_eq!(crate::selected_path::selected(&s).unwrap().0, path);
        assert_eq!(s.selected_entry().unwrap().depth, 0);
    }
}

/// Sorts the rows again, as a front end's refresh does with an exact sort.
pub(crate) fn sort(s: &mut SessionState) {
    s.filter.prepare(&s.snap, true);
    let (hide, filter) = (s.hide_hidden, &s.filter);
    let visible = |i: usize, e: &EntryData| shown(hide, e) && filter.matches(i);
    s.list.order = crate::order::Order::default();
    s.list.order.sort_exact(&s.snap.entries, visible, s.sort);
    s.list.rows = s.list.order.rows().to_vec();
}

/// The names of the marked entries, in the order they were marked.
pub(crate) fn marked_names(s: &SessionState) -> Vec<String> {
    (s.marks.items().iter())
        .map(|it| {
            String::from_utf8_lossy(it.target.path.file_name().unwrap_or_default()).into_owned()
        })
        .collect()
}

/// Reads `m` as a front end does for a frame with a new snapshot: the snapshot with its rows
/// sorted (the first selected if nothing is), then the extras until none is stale.
pub(crate) fn sync(s: &mut SessionState, m: &Model) {
    let mut target = s.target.clone();
    s.snap = crate::snapshot::build(m, &mut target, s.size_mode);
    s.target = target;
    sort(s);
    if s.selected.is_none() {
        s.select(0);
    }
    for _ in 0..3 {
        let budget = dfdu_scan::Budget::new(1 << 20);
        let x = crate::extras::read(m, s, &budget, crate::debug::Counters::default());
        let _ = crate::extras::apply(s, x);
        if !crate::extras::stale(s) {
            return;
        }
    }
    panic!("the extras stay stale");
}

/// A browser of `m` read once, then switched to `view` as the user does, and read again.
pub(crate) fn opened(
    m: &Model,
    view: crate::view::View,
) -> (SessionState, Vec<crate::effect::Effect>) {
    let mut s = SessionState::new(Instant::now());
    sync(&mut s, m);
    s.view = view;
    let fx = match view {
        crate::view::View::Cleanup => crate::cleanup::on_open(&mut s, false),
        crate::view::View::Largest => {
            crate::largest::on_open(&mut s);
            Vec::new()
        }
        _ => Vec::new(),
    };
    sync(&mut s, m);
    (s, fx)
}
