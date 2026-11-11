use std::time::Instant;

use ratatui::crossterm::event::KeyModifiers;
use ratatui::style::Modifier;

use super::*;
use crate::app::keys::ACTIONS;
use crate::tests::{library_snap, press, render, render_buffer, style_at, text, typed};

fn app_with_help(t0: Instant) -> App {
    let mut app = App::new(Theme::no_color(), t0);
    app.refresh(library_snap(), t0);
    assert!(app.key(press(KeyCode::Char('?')), t0).is_empty());
    app
}

fn help(app: &App) -> &Help {
    match &app.overlay {
        Overlay::Help(h) => h,
        o => panic!("the help is not open: {o:?}"),
    }
}

/// The rows of the frame inside the overlay's border.
fn inside(frame: &str) -> Vec<&str> {
    frame
        .lines()
        .filter_map(|l| l.split_once('┃')?.1.rsplit_once('┃'))
        .map(|(inside, _)| inside)
        .collect()
}

#[test]
fn help_lists_every_action() {
    let t0 = Instant::now();
    let mut app = app_with_help(t0);
    // One column on a tall screen: everything fits, and each action shows its keys before
    // its label.
    let frame = render(&mut app, t0, 76, 110);
    assert!(!frame.contains("↓ more"), "{frame}");
    let rows = inside(&frame);
    for info in ACTIONS {
        let first = info.label.split(' ').next().expect("a word");
        let shown = rows.iter().any(|r| {
            r.match_indices(info.keys)
                .any(|(at, _)| r[at + info.keys.len()..].trim_start().starts_with(first))
        });
        assert!(shown, "{:?} is not shown:\n{frame}", info.action);
    }
    let words: Vec<&str> = rows.iter().flat_map(|r| r.split_whitespace()).collect();
    let squeezed = words.join(" ");
    for info in ACTIONS {
        assert!(squeezed.contains(info.label), "{}\n{frame}", info.label);
    }
    for (_, heading) in GROUPS {
        assert!(words.contains(&heading), "{heading}\n{frame}");
    }
    for note in NOTES {
        assert!(squeezed.contains(note), "{note}\n{frame}");
    }
}

#[test]
fn yank_is_listed_and_runs_from_the_registry() {
    let t0 = Instant::now();
    let mut app = app_with_help(t0);
    let frame = render(&mut app, t0, 76, 100);
    assert!(
        frame.contains("find the selected file on the peer"),
        "{frame}"
    );
    app.key(press(KeyCode::Esc), t0);
    app.key(press(KeyCode::Char('Y')), t0);
    assert!(app.peer.yank);
}

#[test]
fn help_groups_in_two_columns_when_wide() {
    let t0 = Instant::now();
    let mut app = app_with_help(t0);
    let buf = render_buffer(&mut app, t0, 100, 30);
    let frame = text(&buf);
    insta::assert_snapshot!("help_100x30", frame);
    let rows = inside(&frame);
    let headings = |r: &str| GROUPS.iter().filter(|(_, h)| r.contains(h)).count();
    assert!(rows[0].starts_with(" Navigate"), "{frame}");
    assert_eq!(headings(rows[0]), 2, "a group heads each column: {frame}");
    assert_eq!(app.areas.overlay.expect("help area").width, 96);
    assert!(frame.contains("┏ Keys ━"), "a thick border: {frame}");
    assert!(frame.contains(" / search ┓"), "{frame}");
    assert_eq!(style_at(&buf, "┏").add_modifier, Modifier::BOLD);
}

#[test]
fn help_is_one_column_when_narrow() {
    let t0 = Instant::now();
    let mut app = app_with_help(t0);
    let frame = render(&mut app, t0, 60, 30);
    insta::assert_snapshot!("help_60x30", frame);
    // Taller, so the wrapped row checked below is not the last one shown.
    let frame = render(&mut app, t0, 60, 50);
    let rows = inside(&frame);
    for r in &rows {
        let headings = GROUPS.iter().filter(|(_, h)| r.contains(h)).count();
        assert!(headings <= 1, "{frame}");
    }
    assert_eq!(rows[0].trim_end(), " Navigate", "{frame}");
    // A label too long for the column goes on under itself, after the keys' column.
    let at = rows
        .iter()
        .position(|r| r.contains("toggle disk usage like du"))
        .unwrap_or_else(|| panic!("{frame}"));
    let indent = rows[at].find("toggle").expect("the label");
    assert_eq!(&rows[at][..indent], " u  ");
    let next: Vec<char> = rows[at + 1].chars().collect();
    assert!(
        next[..indent].iter().all(|&c| c == ' ') && next[indent] != ' ',
        "{frame}"
    );
    // An action without a key shows the palette's key in the keys column.
    let frame = render(&mut app, t0, 60, 80);
    let rows = inside(&frame);
    let at = (rows.iter())
        .position(|r| r.contains("mark by pattern"))
        .unwrap_or_else(|| panic!("{frame}"));
    assert!(rows[at].starts_with(" :  "), "{frame}");
    assert!(rows[at + 1].contains("(in the palette)"), "{frame}");
}

#[test]
fn help_scrolls() {
    let t0 = Instant::now();
    let mut app = app_with_help(t0);
    let first = render(&mut app, t0, 40, 12);
    assert!(first.contains(" Navigate"), "{first}");
    assert!(first.contains("↓ more"), "{first}");
    app.key(press(KeyCode::Down), t0);
    let down = render(&mut app, t0, 40, 12);
    assert!(!down.contains(" Navigate"), "one line down:\n{down}");
    app.key(press(KeyCode::Up), t0);
    assert_eq!(render(&mut app, t0, 40, 12), first);
    // A page is the lines on screen, and the end stops the scroll.
    let page = app.areas.overlay.expect("help area").height as usize - 2;
    app.key(press(KeyCode::PageDown), t0);
    render(&mut app, t0, 40, 12);
    assert_eq!(help(&app).scroll.get(), page);
    for _ in 0..100 {
        app.key(press(KeyCode::PageDown), t0);
        render(&mut app, t0, 40, 12);
    }
    let end = render(&mut app, t0, 40, 12);
    assert!(!end.contains("↓ more"), "{end}");
    assert!(end.contains("--no-mouse"), "{end}");
    app.key(press(KeyCode::Up), t0);
    assert_ne!(render(&mut app, t0, 40, 12), end, "scrolls back at once");
    for _ in 0..100 {
        app.key(press(KeyCode::PageUp), t0);
    }
    assert_eq!(render(&mut app, t0, 40, 12), first);
    // Keys between two frames stop at the end the last frame had, so one key back moves
    // back at once.
    let max = help(&app).max.get().expect("drawn");
    for _ in 0..100 {
        app.key(press(KeyCode::PageDown), t0);
        app.key(press(KeyCode::Down), t0);
    }
    assert_eq!(help(&app).scroll.get(), max);
    app.key(press(KeyCode::Up), t0);
    assert_eq!(help(&app).scroll.get(), max - 1);
    let back = render(&mut app, t0, 40, 12);
    assert_ne!(back, end);
    assert!(back.contains("↓ more"), "{back}");
}

#[test]
fn help_search_filters_rows() {
    let t0 = Instant::now();
    let mut app = app_with_help(t0);
    typed(&mut app, "/trash", t0);
    assert!(help(&app).searching);
    let buf = render_buffer(&mut app, t0, 100, 30);
    let frame = text(&buf);
    assert!(frame.contains(" / trash▌"), "{frame}");
    assert!(frame.contains("move to Trash"), "{frame}");
    assert!(!frame.contains("move up"), "{frame}");
    assert!(!frame.contains("Navigate"), "{frame}");
    assert!(!frame.contains("Rows hold"), "{frame}");
    assert!(
        style_at(&buf, "Trash")
            .add_modifier
            .contains(Modifier::UNDERLINED),
        "matches are highlighted"
    );
    // A query matches the keys too, and every word must match.
    app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL), t0);
    typed(&mut app, "F12", t0);
    let frame = render(&mut app, t0, 100, 30);
    assert!(frame.contains("debug overlay"), "{frame}");
    let one_of_all = format!("1 of {}", ACTIONS.len());
    assert!(frame.contains(&one_of_all), "{frame}");
    typed(&mut app, " zzz", t0);
    let frame = render(&mut app, t0, 100, 30);
    assert!(frame.contains("nothing matches"), "{frame}");
    // Enter keeps the query and leaves the search; then `q` closes instead of typing.
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL), t0);
    app.key(press(KeyCode::Backspace), t0);
    app.key(press(KeyCode::Enter), t0);
    assert!(!help(&app).searching);
    let frame = render(&mut app, t0, 100, 30);
    assert!(frame.contains(&format!(" / F12   {one_of_all}")), "{frame}");
    // Esc while searching clears the query and keeps the help open.
    app.key(press(KeyCode::Char('/')), t0);
    app.key(press(KeyCode::Esc), t0);
    assert!(help(&app).search.is_empty());
    let frame = render(&mut app, t0, 100, 30);
    assert!(frame.contains("move up"), "{frame}");
    app.key(press(KeyCode::Char('q')), t0);
    assert!(matches!(app.overlay, Overlay::None));
    assert!(!app.quit);
}

#[test]
fn help_closes_on_esc_q_question_f1() {
    let t0 = Instant::now();
    for close in [
        KeyCode::Esc,
        KeyCode::Char('q'),
        KeyCode::Char('?'),
        KeyCode::F(1),
        KeyCode::Enter,
    ] {
        let mut app = app_with_help(t0);
        // Other keys leave it open and do nothing to the list.
        for other in [KeyCode::Char('d'), KeyCode::Char('s'), KeyCode::Right] {
            assert!(app.key(press(other), t0).is_empty());
            assert!(matches!(app.overlay, Overlay::Help(_)), "{other:?}");
        }
        assert_eq!(app.session.sort, crate::state::order::SortSpec::default());
        app.key(press(close), t0);
        assert!(matches!(app.overlay, Overlay::None), "{close:?}");
        assert!(!app.quit, "{close:?}");
    }
    let mut app = app_with_help(t0);
    app.key(press(KeyCode::F(1)), t0);
    app.key(press(KeyCode::F(1)), t0);
    assert!(matches!(app.overlay, Overlay::Help(_)), "F1 opens it too");
}

#[test]
fn labels_wrap_at_spaces() {
    assert_eq!(wrap("one two three", 7), ["one two", "three"]);
    assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    assert_eq!(wrap_at("ab cd", 2), [(0, "ab"), (3, "cd")]);
    assert_eq!(wrap("", 5), Vec::<&str>::new());
}

#[test]
fn help_scrolls_half_a_page() {
    let t0 = Instant::now();
    let mut app = app_with_help(t0);
    render(&mut app, t0, 40, 12);
    let page = app.areas.overlay.expect("help area").height as usize - 2;
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    app.key(ctrl('d'), t0);
    assert_eq!(help(&app).scroll.get(), page / 2);
    app.key(ctrl('u'), t0);
    assert_eq!(help(&app).scroll.get(), 0);
    // While a search is typed, Ctrl-U clears it instead.
    app.key(press(KeyCode::Char('/')), t0);
    typed(&mut app, "zzz", t0);
    app.key(ctrl('u'), t0);
    assert_eq!(help(&app).search.text(), "");
}
