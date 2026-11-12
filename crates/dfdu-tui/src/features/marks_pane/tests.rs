use std::time::Instant;

use dfdu_engine::{Command, Estimate};
use dfdu_tree::HostPath;
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseEvent, MouseEventKind};

use super::*;
use crate::app::keys::READ_ONLY;
use crate::features::details;
use crate::model::snapshot::{EntryData, Snapshot, test_entry};
use crate::tests::{marked_names, press, refused, render, snap_with_totals};
use crate::theme::Theme;

const GIB: u64 = 1 << 30;

fn dir(i: u32, name: &str, size: u64) -> EntryData {
    EntryData {
        is_dir: true,
        items: 1000,
        ..test_entry(i, name, size)
    }
}

/// The browser in `~/dev/projects/website-redesign-2026`, a finished scan of `~`, with two
/// directories, a video with a long name and a small file.
fn browser() -> (App, Instant) {
    let t0 = Instant::now();
    let mut app = App::new(Theme::no_color(), t0);
    let entries = vec![
        dir(0, "target", 7 * GIB / 2),
        test_entry(
            1,
            "screen-recording-of-the-whole-launch-meeting.mkv",
            2 * GIB,
        ),
        dir(2, "node_modules", 6 * GIB / 5),
        test_entry(3, "README.md", 4 << 10),
    ];
    let dir_path = HostPath::from("/home/jo/dev/projects/website-redesign-2026");
    let snap = Snapshot {
        dir_path,
        root_path: HostPath::from("/home/jo"),
        at_root: false,
        busy: false,
        scanning: false,
        ..snap_with_totals(entries, 1)
    };
    app.refresh(snap, t0);
    (app, t0)
}

/// Marks the selected row and the next `n - 1`, from the top of the list.
fn mark(app: &mut App, n: usize, t0: Instant) {
    for _ in 0..n {
        app.key(press(KeyCode::Char(' ')), t0);
    }
}

/// The estimate the pane asked for, from the effects of a key.
fn asked(fx: &[Effect]) -> Option<(u64, usize)> {
    fx.iter().find_map(|e| match e {
        Effect::Send(Command::Estimate { id, items }) => Some((*id, items.len())),
        _ => None,
    })
}

/// The engine's answer to the estimate `id`.
fn answer(app: &mut App, id: u64, bytes: u64) {
    app.session.snap.estimate = Some(Estimate {
        id,
        bytes,
        items: 0,
        nodes: 0,
        linked_out: 0,
        shared_out: 0,
        scanning: false,
        may_free_less: false,
    });
}

#[test]
fn tab_cycles_the_list_the_details_and_the_marks() {
    let (mut app, t0) = browser();
    mark(&mut app, 2, t0);
    app.key(press(KeyCode::Char('i')), t0);
    render(&mut app, t0, 120, 24);
    app.key(press(KeyCode::Tab), t0);
    assert!(details::focused(&app));
    assert!(!is_open(&app));
    let fx = app.key(press(KeyCode::Tab), t0);
    assert!(is_open(&app), "Tab from the details opens the marks");
    assert!(!details::focused(&app));
    assert_eq!(asked(&fx).map(|a| a.1), Some(2), "{fx:?}");
    let out = render(&mut app, t0, 120, 24);
    assert!(
        out.contains(" Marks "),
        "the marks take the details' place: {out}"
    );
    app.key(press(KeyCode::Tab), t0);
    assert!(
        !is_open(&app) && !details::focused(&app),
        "back to the list"
    );
    let out = render(&mut app, t0, 120, 24);
    assert!(!out.contains(" Marks "), "the details are back: {out}");

    // Without the details, Tab goes from the list to the marks, and Esc comes back.
    app.key(press(KeyCode::Char('i')), t0);
    app.key(press(KeyCode::Tab), t0);
    assert!(is_open(&app));
    app.key(press(KeyCode::Esc), t0);
    assert!(!is_open(&app));
    assert_eq!(app.session.marks.items().len(), 2, "Esc keeps the marks");
}

#[test]
fn lists_the_marks_with_their_sizes_and_what_they_would_free() {
    let (mut app, t0) = browser();
    mark(&mut app, 3, t0);
    let fx = app.key(press(KeyCode::Tab), t0);
    let (id, n) = asked(&fx).expect("an estimate is asked for");
    assert_eq!(n, 3);
    let out = render(&mut app, t0, 100, 24);
    assert!(out.contains("3 marked, 6.7 GiB"), "{out}");
    assert!(out.contains("would free: estimating…"), "{out}");
    answer(&mut app, id, 6 * GIB);
    let out = render(&mut app, t0, 100, 24);
    assert!(out.contains("would free about 6.0 GiB"), "{out}");
    // Paths relative to the scan root, shortened in the middle to fit.
    assert!(out.contains("3.5 GiB  dev/projects/"), "{out}");
    assert!(out.contains("…26/target/┃"), "{out}");
    insta::assert_snapshot!("marks_pane_100x24", out);
    let out = render(&mut app, t0, 70, 20);
    assert!(out.contains("would free about 6.0 GiB"), "{out}");
    insta::assert_snapshot!("marks_pane_70x20", out);
    // An estimate for other marks is not shown.
    answer(&mut app, id - 1, GIB);
    let out = render(&mut app, t0, 100, 24);
    assert!(out.contains("would free: estimating…"), "{out}");
}

#[test]
fn empty_says_how_to_mark() {
    let (mut app, t0) = browser();
    let fx = app.key(press(KeyCode::Tab), t0);
    assert!(asked(&fx).is_none(), "nothing to estimate");
    let out = render(&mut app, t0, 100, 24);
    assert!(out.contains("Nothing is marked."), "{out}");
    assert!(out.contains("Space marks the selected"), "{out}");
}

#[test]
fn space_unmarks_the_selected_item_and_asks_again() {
    let (mut app, t0) = browser();
    mark(&mut app, 3, t0);
    let fx = app.key(press(KeyCode::Tab), t0);
    let (first, _) = asked(&fx).expect("asked");
    render(&mut app, t0, 100, 24);
    app.key(press(KeyCode::Down), t0);
    let fx = app.key(press(KeyCode::Char(' ')), t0);
    let names: Vec<_> = (app.session.marks.items().iter())
        .map(|it| String::from_utf8_lossy(it.target.path.file_name().expect("name")).into_owned())
        .collect();
    assert_eq!(names, ["target", "node_modules"]);
    assert_eq!(
        app.session.marks_pane.cursor, 1,
        "the cursor stays on the row"
    );
    let (again, n) = asked(&fx).expect("asked again");
    assert!(again > first);
    assert_eq!(n, 2);
    // The list's selection did not move: Space acted on the pane.
    assert_eq!(app.session.selected_index(), Some(3));
    // The last one unmarked leaves the cursor on the new last row.
    app.key(press(KeyCode::Char(' ')), t0);
    assert_eq!(app.session.marks_pane.cursor, 0);
    app.key(press(KeyCode::Char(' ')), t0);
    assert!(app.session.marks.is_empty());
    assert!(render(&mut app, t0, 100, 24).contains("Nothing is marked."));
}

#[test]
fn star_inverts_the_marks_of_the_directory() {
    let (mut app, t0) = browser();
    mark(&mut app, 1, t0);
    app.key(press(KeyCode::Tab), t0);
    app.key(press(KeyCode::Char('*')), t0);
    let mut names: Vec<_> = (app.session.marks.items().iter())
        .map(|it| String::from_utf8_lossy(it.target.path.file_name().expect("name")).into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "README.md",
            "node_modules",
            "screen-recording-of-the-whole-launch-meeting.mkv"
        ]
    );
    assert_eq!(
        app.session.status.as_deref(),
        Some("marks inverted: 3 marked, 1 unmarked")
    );
    // Again, back to the first mark. In the list `*` is tree mode's, not the marks'.
    app.key(press(KeyCode::Char('*')), t0);
    assert_eq!(marked_names(&app), ["target"]);
}

#[test]
fn star_marks_nothing_in_a_read_only_browser() {
    let (mut app, t0) = browser();
    app.session.read_only = true;
    app.key(press(KeyCode::Tab), t0);
    app.key(press(KeyCode::Char('*')), t0);
    assert!(app.session.marks.is_empty());
    assert_eq!(app.session.status.as_deref(), Some(READ_ONLY));
    // `d` there deletes nothing either.
    refused(&mut app, KeyCode::Char('d'), READ_ONLY, t0);
}

#[test]
fn d_asks_to_delete_the_marks_as_the_list_does() {
    let (mut app, t0) = browser();
    // With nothing marked, `d` in the pane does not fall back to the list's selection.
    app.key(press(KeyCode::Tab), t0);
    refused(&mut app, KeyCode::Char('d'), "nothing is marked", t0);
    app.key(press(KeyCode::Esc), t0);
    mark(&mut app, 2, t0);
    app.key(press(KeyCode::Tab), t0);
    let fx = app.key(press(KeyCode::Char('D')), t0);
    assert!(app.session.confirm.is_some());
    assert_eq!(asked(&fx).map(|a| a.1), Some(2), "{fx:?}");
    let out = render(&mut app, t0, 100, 24);
    assert!(out.contains("Permanently delete 2 entries?"), "{out}");
    // The dialog's estimate is for the same marks, and the pane shows it.
    let (id, _) = asked(&fx).expect("asked");
    answer(&mut app, id, 4 * GIB);
    app.key(press(KeyCode::Esc), t0);
    assert!(is_open(&app), "cancelling keeps the pane open");
    let out = render(&mut app, t0, 100, 24);
    assert!(out.contains("would free about 4.0 GiB"), "{out}");
}

#[test]
fn the_wheel_moves_the_cursor_wherever_the_pointer_is() {
    let (mut app, t0) = browser();
    mark(&mut app, 4, t0);
    app.key(press(KeyCode::Tab), t0);
    render(&mut app, t0, 100, 24);
    let list = app.areas.list;
    let down = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: list.x + 1,
        row: list.y + 1,
        modifiers: KeyModifiers::NONE,
    };
    let selected = app.session.selected_index();
    crate::input::mouse::event(&mut app, down, t0).expect("redraw");
    assert_eq!(app.session.marks_pane.cursor, 1);
    assert_eq!(app.session.selected_index(), selected, "the list stays");
}

#[test]
fn no_room_closes_the_pane_and_says_so() {
    let (mut app, t0) = browser();
    mark(&mut app, 1, t0);
    app.key(press(KeyCode::Tab), t0);
    let out = render(&mut app, t0, 70, 12);
    assert!(!is_open(&app));
    assert!(out.contains("no room for the marks pane"), "{out}");
}
