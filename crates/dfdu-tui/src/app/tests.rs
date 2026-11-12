use std::path::PathBuf;

use super::*;
use crate::model::snapshot::test_entry;
use crate::tests::{press, snap_without_totals};

#[test]
fn sort_keys_apply_without_a_new_snapshot() {
    let t0 = Instant::now();
    let mut app = App::new(Theme::no_color(), t0);
    let es = vec![
        test_entry(0, "b", 30),
        test_entry(1, "c", 20),
        test_entry(2, "a", 10),
    ];
    app.refresh(snap_without_totals(es, 1), t0);
    app.key(press(KeyCode::Down), t0);
    let picked = app.session.selected.as_ref().and_then(|s| s.node);
    app.key(press(KeyCode::Char('A')), t0);
    app.reorder(t0);
    let names: Vec<&[u8]> = app
        .session
        .list
        .rows
        .iter()
        .map(|&i| &app.session.snap.entries[i].name[..])
        .collect();
    assert_eq!(names, [b"a", b"b", b"c"]);
    assert_eq!(app.session.selected.as_ref().and_then(|s| s.node), picked);
    assert_eq!(app.session.selected_index(), Some(2));
}

#[test]
fn the_ui_options_it_owns_are_set_by_the_constructor() {
    let ui = crate::UiOptions {
        color_level: ColorLevel::Ansi16,
        background: Some((1, 2, 3)),
        home: Some(PathBuf::from("home")),
        local: true,
        read_only: true,
        from_file: true,
        elevated: true,
        notice: Some("the scan cache is not used".into()),
        came_from: Some(dfdu_tree::HostPath::from("/mnt/cache")),
        ..crate::UiOptions::from_env()
    };
    let app = App::with_options(&ui, Instant::now());
    assert_eq!(app.color_level, ColorLevel::Ansi16);
    assert_eq!(app.background, Some((1, 2, 3)));
    assert_eq!(app.session.home, ui.home);
    assert!(app.session.outside.local && app.session.read_only);
    assert_eq!(app.session.from_file, Some(dfdu_session::goto::FROM_FILE));
    assert!(app.session.elevated);
    assert_eq!(app.session.status, ui.notice);
    assert_eq!(app.session.came_from, ui.came_from);
}
