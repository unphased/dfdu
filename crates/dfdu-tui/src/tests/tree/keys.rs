//! The keys and clicks of tree mode: `T`, `→`, `←`, `Enter`, `*`, `z`, fold rows, and a click
//! on `▸`/`▾`.

use std::time::Instant;

use dfdu_tree::{ErrKind, HostPath};
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Modifier;

use super::{Fx, sync};
use crate::app::App;
use crate::input::mouse;
use crate::tests::{press, render, render_buffer, row_names, select, style_at};
use crate::theme::Theme;

/// `/r` with `a/` (`a1/` holding `x` of 500 bytes, `a2` of 300, and `s` of 1 byte that could
/// not be read, under 1% of `a/`), `b/` holding `b1` of 200, and `top` of 100.
pub(crate) fn small() -> Fx {
    let mut fx = Fx::new();
    let a = fx.dir(fx.root, "a");
    let a1 = fx.dir(a, "a1");
    fx.file(a1, "x", 500);
    fx.file(a, "a2", 300);
    let s = fx.file(a, "s", 1);
    fx.m.tree.set_error(s, ErrKind::PermissionDenied);
    let b = fx.dir(fx.root, "b");
    fx.file(b, "b1", 200);
    fx.file(fx.root, "top", 100);
    fx
}

/// A browser of [`small`] in tree mode, read once.
pub(crate) fn tree_app(fx: &Fx, t0: Instant) -> App {
    let mut app = App::new(Theme::no_color(), t0);
    sync(&mut app, &fx.m, t0);
    key(&mut app, fx, KeyCode::Char('T'), t0);
    app
}

/// Presses `code`, then reads as the run loop would.
pub(crate) fn key(app: &mut App, fx: &Fx, code: KeyCode, t0: Instant) {
    app.key(press(code), t0);
    sync(app, &fx.m, t0);
}

pub(crate) fn selected(app: &App) -> String {
    let e = app.session.selected_entry().expect("a selection");
    String::from_utf8_lossy(&e.name).into_owned()
}

#[test]
fn exact_filter_jumps_follow_visible_tree_rows() {
    let fx = small();
    let t0 = Instant::now();
    let mut app = tree_app(&fx, t0);
    key(&mut app, &fx, KeyCode::Right, t0);
    for code in [KeyCode::Char('/'), KeyCode::Char('a'), KeyCode::Enter] {
        key(&mut app, &fx, code, t0);
    }
    assert_eq!(selected(&app), "a");
    for (code, name, status) in [
        ('n', "a1", "exact 2/3"),
        ('n', "a2", "exact 3/3"),
        ('n', "a", "exact 1/3"),
        ('N', "a2", "exact 3/3"),
    ] {
        key(&mut app, &fx, KeyCode::Char(code), t0);
        assert_eq!(selected(&app), name);
        assert_eq!(app.session.status.as_deref(), Some(status));
    }
}

#[test]
fn t_turns_tree_mode_on_and_off() {
    let fx = small();
    let t0 = Instant::now();
    let mut app = tree_app(&fx, t0);
    assert!(app.session.tree.is_some());
    let frame = render(&mut app, t0, 120, 12);
    let flags = frame.lines().nth(1).expect("second header line");
    assert!(
        flags.contains("tree  ·  sort size ↓  ·  share of parent  ·  fold < 1%  ·  disk usage"),
        "{frame}"
    );
    assert!(frame.contains("▸ a/"), "{frame}");
    assert!(frame.contains("→ expand  ← collapse"), "{frame}");
    key(&mut app, &fx, KeyCode::Char('T'), t0);
    assert!(app.session.tree.is_none());
    let frame = render(&mut app, t0, 120, 12);
    // The selection line starts with `▸`, a row does not.
    assert!(
        !frame.contains("tree") && !frame.contains("▸ b/"),
        "{frame}"
    );
}

#[test]
fn right_expands_then_goes_to_the_first_entry() {
    let fx = small();
    let t0 = Instant::now();
    let mut app = tree_app(&fx, t0);
    assert_eq!(row_names(&app), ["a", "b", "top"]);
    key(&mut app, &fx, KeyCode::Right, t0);
    assert_eq!(row_names(&app), ["a", "a1", "a2", "… 1 more", "b", "top"]);
    assert_eq!(selected(&app), "a", "expanding stays on the row");
    key(&mut app, &fx, KeyCode::Right, t0);
    assert_eq!(selected(&app), "a1");
    key(&mut app, &fx, KeyCode::Char('l'), t0);
    assert_eq!(
        row_names(&app),
        ["a", "a1", "x", "a2", "… 1 more", "b", "top"]
    );
    // A file has nothing to expand.
    select(&mut app, "top");
    key(&mut app, &fx, KeyCode::Right, t0);
    assert_eq!(selected(&app), "top");
    assert_eq!(app.session.target.path, HostPath::from("/r"));
}

#[test]
fn left_collapses_then_goes_to_the_parent_row_then_up() {
    let fx = small();
    let t0 = Instant::now();
    let mut app = tree_app(&fx, t0);
    for code in [KeyCode::Right, KeyCode::Right, KeyCode::Right] {
        key(&mut app, &fx, code, t0);
    }
    select(&mut app, "x");
    key(&mut app, &fx, KeyCode::Left, t0);
    assert_eq!(selected(&app), "a1");
    key(&mut app, &fx, KeyCode::Char('h'), t0);
    assert_eq!(row_names(&app), ["a", "a1", "a2", "… 1 more", "b", "top"]);
    key(&mut app, &fx, KeyCode::Left, t0);
    assert_eq!(selected(&app), "a");
    // Collapsing forgets what was expanded inside.
    key(&mut app, &fx, KeyCode::Left, t0);
    assert_eq!(row_names(&app), ["a", "b", "top"]);
    key(&mut app, &fx, KeyCode::Right, t0);
    assert_eq!(row_names(&app), ["a", "a1", "a2", "… 1 more", "b", "top"]);

    // Enter opens the selected directory; at the top of it, ← goes back up.
    select(&mut app, "a1");
    key(&mut app, &fx, KeyCode::Enter, t0);
    assert_eq!(
        app.session.target.path,
        HostPath::from("/r").join(b"a").join(b"a1")
    );
    assert_eq!(row_names(&app), ["x"]);
    key(&mut app, &fx, KeyCode::Left, t0);
    assert_eq!(app.session.target.path, HostPath::from("/r").join(b"a"));
    assert_eq!(selected(&app), "a1");
}

#[test]
fn enter_opens_a_row_below_the_top_keeping_what_is_expanded() {
    let fx = small();
    let t0 = Instant::now();
    let mut app = tree_app(&fx, t0);
    key(&mut app, &fx, KeyCode::Right, t0);
    key(&mut app, &fx, KeyCode::Right, t0);
    key(&mut app, &fx, KeyCode::Right, t0);
    select(&mut app, "a");
    key(&mut app, &fx, KeyCode::Enter, t0);
    assert_eq!(app.session.target.path, HostPath::from("/r").join(b"a"));
    assert_eq!(row_names(&app), ["a1", "x", "a2", "s"]);
    assert!(app.session.tree.is_some(), "still in tree mode");
    // In the list, → opens as it always did.
    key(&mut app, &fx, KeyCode::Char('T'), t0);
    select(&mut app, "a1");
    key(&mut app, &fx, KeyCode::Right, t0);
    assert_eq!(
        app.session.target.path,
        HostPath::from("/r").join(b"a").join(b"a1")
    );
}

#[test]
fn star_expands_the_largest_entries_and_turns_tree_mode_on() {
    let fx = small();
    let t0 = Instant::now();
    let mut app = App::new(Theme::no_color(), t0);
    sync(&mut app, &fx.m, t0);
    key(&mut app, &fx, KeyCode::Char('*'), t0);
    assert!(app.session.tree.is_some());
    assert_eq!(
        row_names(&app),
        ["a", "a1", "x", "a2", "… 1 more", "b", "top"]
    );
    // A file has no entries to expand.
    select(&mut app, "top");
    key(&mut app, &fx, KeyCode::Char('*'), t0);
    assert!(
        app.session
            .status
            .as_deref()
            .is_some_and(|s| s.contains("select a directory"))
    );
}

#[test]
fn z_folds_less_then_nothing() {
    let fx = small();
    let t0 = Instant::now();
    let mut app = tree_app(&fx, t0);
    key(&mut app, &fx, KeyCode::Right, t0);
    assert!(row_names(&app).contains(&"… 1 more".to_owned()));
    // 1 byte of 801 is more than 0.1%.
    key(&mut app, &fx, KeyCode::Char('z'), t0);
    assert_eq!(app.session.status.as_deref(), Some("fold < 0.1%"));
    assert_eq!(row_names(&app), ["a", "a1", "a2", "s", "b", "top"]);
    app.session.status = None;
    let frame = render(&mut app, t0, 120, 12);
    assert!(frame.contains("fold < 0.1%"), "{frame}");
    key(&mut app, &fx, KeyCode::Char('z'), t0);
    assert_eq!(app.session.status.as_deref(), Some("nothing folds"));
    app.session.status = None;
    let frame = render(&mut app, t0, 120, 12);
    assert!(!frame.contains("fold <"), "{frame}");
    assert!(frame.contains("z no fold"), "{frame}");
    key(&mut app, &fx, KeyCode::Char('z'), t0);
    assert!(row_names(&app).contains(&"… 1 more".to_owned()));
}

#[test]
fn fold_rows_open_in_place_and_cannot_be_marked_or_entered() {
    let fx = small();
    let t0 = Instant::now();
    let mut app = tree_app(&fx, t0);
    key(&mut app, &fx, KeyCode::Right, t0);
    select(&mut app, "… 1 more");
    let buf = render_buffer(&mut app, t0, 80, 12);
    let frame = crate::tests::text(&buf);
    let row = frame
        .lines()
        .find(|l| l.contains("… 1 more"))
        .expect("the fold row");
    // Its entry could not be read, but the fold row has no mark of its own.
    assert!(!row.contains('!'), "{frame}");
    assert!(
        style_at(&buf, "… 1 more")
            .add_modifier
            .contains(Modifier::ITALIC)
    );
    key(&mut app, &fx, KeyCode::Char(' '), t0);
    assert!(app.session.marks.is_empty());
    assert!(
        app.session
            .status
            .as_deref()
            .is_some_and(|s| s.contains("fold row"))
    );
    key(&mut app, &fx, KeyCode::Enter, t0);
    assert_eq!(app.session.target.path, HostPath::from("/r"));
    // → shows what it folds, for this directory only, and the selection goes to it.
    key(&mut app, &fx, KeyCode::Right, t0);
    assert_eq!(row_names(&app), ["a", "a1", "a2", "s", "b", "top"]);
    assert_eq!(selected(&app), "s");
}

fn click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn a_click_on_the_triangle_expands_and_one_on_the_row_selects() {
    let fx = small();
    let t0 = Instant::now();
    let mut app = tree_app(&fx, t0);
    render(&mut app, t0, 80, 12);
    // The rows start below the two header lines and the column names.
    let (x, top) = (app.mouse.name_x, 3);
    mouse::event(&mut app, click(x + 4, top + 1), t0).expect("redraw");
    assert_eq!(selected(&app), "b");
    assert!(
        app.session
            .tree
            .as_ref()
            .is_some_and(|t| t.expanded.is_empty())
    );
    mouse::event(&mut app, click(x, top), t0).expect("redraw");
    assert_eq!(selected(&app), "a");
    sync(&mut app, &fx.m, t0);
    render(&mut app, t0, 80, 12);
    assert_eq!(row_names(&app), ["a", "a1", "a2", "… 1 more", "b", "top"]);
    // One level down the triangle is after the guide.
    mouse::event(&mut app, click(x + 2, top + 1), t0).expect("redraw");
    sync(&mut app, &fx.m, t0);
    assert_eq!(
        row_names(&app),
        ["a", "a1", "x", "a2", "… 1 more", "b", "top"]
    );
    mouse::event(&mut app, click(x + 1, top), t0).expect("redraw");
    sync(&mut app, &fx.m, t0);
    assert_eq!(row_names(&app), ["a", "b", "top"]);
}
