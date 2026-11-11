use std::time::Instant;

use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::tests::{library_snap, press, render};
use crate::theme::Theme;

fn app(busy: bool, errors: u64, t0: Instant) -> App {
    let mut app = App::new(Theme::no_color(), t0);
    let mut snap = library_snap();
    snap.busy = busy;
    snap.root_total.errors = errors;
    app.refresh(snap, t0);
    app
}

fn shown(app: &App, width: u16) -> String {
    line(app, width)
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

/// Cleanup's footer keeps its own keys before `[ ] matches`, `? help` and `q quit`.
#[test]
fn view_keys_come_before_help_and_quit() {
    let fx = crate::tests::classified();
    let app = crate::features::cleanup::tests::opened(&fx.m, Instant::now());
    let wide = shown(&app, 160);
    for want in ["f filter", "N command", "[ ] matches", "q quit"] {
        assert!(wide.contains(want), "{wide}");
    }
    let narrow = shown(&app, 80);
    assert!(narrow.contains("↑↓ move"), "{narrow}");
    assert!(!narrow.contains("q quit"), "quit goes first: {narrow}");
}

#[test]
fn hints_change_with_context() {
    let t0 = Instant::now();
    let mut list = app(false, 0, t0);
    assert_eq!(
        shown(&list, 120),
        " ↑↓ move  → open  ← up  s n C M sort  Space mark  / filter  i details  : commands  \
         ? help  q quit  [ ] matches"
    );
    let scanning = app(true, 0, t0);
    assert!(shown(&scanning, 200).contains("  S re-sort  "));
    assert!(!shown(&list, 200).contains("S re-sort"));
    let errors = app(false, 3, t0);
    assert!(shown(&errors, 200).contains("  ! errors  ? help"));
    assert!(!shown(&list, 200).contains("! errors"));
    // Never more than twelve, whatever the width.
    let busy_errors = app(true, 3, t0);
    let all = shown(&busy_errors, 400);
    assert_eq!(all.matches("  ").count() + 1, MAX_HINTS, "{all}");
    assert!(
        all.contains("S re-sort") && all.contains("! errors"),
        "{all}"
    );

    list.key(press(KeyCode::Char(' ')), t0);
    assert_eq!(
        shown(&list, 120),
        " ↑↓ move  → open  ← up  Space mark  d trash marked  D delete marked  s n C M sort  \
         / filter  : commands  ? help  q quit"
    );
    list.key(press(KeyCode::Char('D')), t0);
    assert!(list.session.confirm.is_some());
    assert_eq!(shown(&list, 120), " p permanent  s stage  Esc cancel");
    list.key(press(KeyCode::Esc), t0);

    list.key(press(KeyCode::Char('/')), t0);
    assert_eq!(shown(&list, 120), " Enter keep  Esc clear  ↑↓ move");
    list.key(press(KeyCode::Esc), t0);

    list.key(press(KeyCode::Char('i')), t0);
    render(&mut list, t0, 120, 20);
    list.key(press(KeyCode::Tab), t0);
    assert!(details::focused(&list));
    assert_eq!(
        shown(&list, 120),
        " Esc back  Tab marks  ↑↓ scroll  i close  ? help  q quit"
    );
    // The footer of a frame is the line for its width.
    let frame = render(&mut list, t0, 120, 20);
    assert_eq!(frame.lines().last(), Some(shown(&list, 120).trim_end()));
    // An overlay takes the keys before the pane, and so do the hints.
    list.key(press(KeyCode::Char('d')), t0);
    assert!(list.session.confirm.is_some());
    assert!(details::focused(&list));
    assert_eq!(shown(&list, 120), " p permanent  s stage  Esc cancel");
    list.key(press(KeyCode::Esc), t0);

    for (open, want) in overlays() {
        let mut app = app(true, 1, t0);
        open(&mut app);
        assert!(!matches!(app.overlay, Overlay::None));
        assert_eq!(shown(&app, 120), want);
    }
}

/// Opens an overlay.
type Open = fn(&mut App);

/// Ways to open each overlay but the delete dialog, and the hints while it is open.
fn overlays() -> [(Open, &'static str); 4] {
    [
        (
            |app| {
                app.key(press(KeyCode::Char('?')), Instant::now());
            },
            " ↑↓ scroll  PgUp PgDn page  / search  Esc close",
        ),
        (
            |app| {
                app.key(press(KeyCode::Char('?')), Instant::now());
                app.key(press(KeyCode::Char('/')), Instant::now());
            },
            " Enter keep  Esc clear  ↑↓ scroll",
        ),
        (
            |app| {
                app.key(press(KeyCode::Char(':')), Instant::now());
            },
            " ↑↓ move  Enter run  Esc close",
        ),
        (
            |app| {
                app.show_errors(dfdu_engine::ErrorList {
                    root: "/r".into(),
                    items: Vec::new(),
                    total: 0,
                    kinds: Vec::new(),
                });
            },
            " ↑↓ move  Enter go to it  Esc close",
        ),
    ]
}

#[test]
fn hints_never_cut_a_key() {
    let t0 = Instant::now();
    let mut marked = app(true, 2, t0);
    marked.key(press(KeyCode::Char(' ')), t0);
    let mut filtering = app(false, 0, t0);
    filtering.key(press(KeyCode::Char('/')), t0);
    let mut focused = app(false, 0, t0);
    focused.key(press(KeyCode::Char('i')), t0);
    render(&mut focused, t0, 120, 20);
    focused.key(press(KeyCode::Tab), t0);
    assert!(details::focused(&focused));
    let mut dialog = app(false, 0, t0);
    dialog.key(press(KeyCode::Char('d')), t0);
    assert!(dialog.session.confirm.is_some());
    let mut apps = vec![
        app(false, 0, t0),
        app(true, 1, t0),
        marked,
        filtering,
        focused,
        dialog,
    ];
    for (open, _) in overlays() {
        let mut app = app(true, 1, t0);
        open(&mut app);
        apps.push(app);
    }
    for app in apps {
        let whole: Vec<String> = context(&app)
            .iter()
            .map(|h| format!("{} {}", h.keys, h.label))
            .collect();
        for width in 40..=120 {
            let text = shown(&app, width);
            assert!(
                Span::raw(&text).width() <= usize::from(width),
                "{width}: {text}"
            );
            for part in text[1..].split("  ") {
                assert!(
                    whole.iter().any(|w| w == part),
                    "{width}: {part:?} in {text}"
                );
            }
            // The way to the rest stays on the line.
            assert!(text.contains("? help") || text.contains("Esc "), "{text}");
        }
    }
}
