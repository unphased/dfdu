use std::time::Duration;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};

use super::*;
use crate::app::Overlay;
use crate::model::snapshot::{EntryData, Snapshot, test_entry};
use crate::tests::{
    assert_cue, press, render, render_buffer, row_names, snap_with_totals, style_at, styled, text,
    typed,
};
use crate::theme::Theme;

fn dir(i: u32, name: &str, size: u64) -> EntryData {
    EntryData {
        is_dir: true,
        ..test_entry(i, name, size)
    }
}

fn entries() -> Vec<EntryData> {
    let gib = 1 << 30;
    vec![
        dir(0, "Library", 112 * gib),
        dir(1, "dev", 78 * gib),
        dir(2, ".android", 51 * gib),
        dir(3, "Downloads", 9 * gib),
        test_entry(4, "notes.sqlite", 625 << 20),
        dir(5, "lib64", 300 << 20),
        test_entry(6, "build.log", 12 << 10),
    ]
}

fn app_with(theme: Theme, es: Vec<EntryData>, t0: Instant) -> App {
    let mut app = App::new(theme, t0);
    app.refresh(snap_with_totals(es, 1), t0);
    app
}

/// Types `s` into the filter and takes in the rows it leaves.
fn filter_by(app: &mut App, s: &str, now: Instant) {
    typed(app, s, now);
    app.reorder(now);
}

fn line_text(app: &App) -> Option<String> {
    crate::view::filter::line(app).map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
}

#[test]
fn typing_filters_and_highlights() {
    let t0 = Instant::now();
    let mut app = app_with(Theme::no_color(), entries(), t0);
    filter_by(&mut app, "/li", t0);
    assert!(editing(&app));
    assert_eq!(row_names(&app), ["Library", "notes.sqlite", "lib64"]);
    assert_eq!(line_text(&app).as_deref(), Some(" / li▌   3 of 7"));
    assert_eq!(
        app.session.filter.indices("Library").as_deref(),
        Some(&[0, 1][..])
    );
    assert_eq!(app.session.filter.indices("dev").as_deref(), None);
    insta::assert_snapshot!("filter_editing_mono", render(&mut app, t0, 80, 9));
    let buf = render_buffer(&mut app, t0, 80, 9);
    let underlined = |s: Style| s.add_modifier.contains(Modifier::UNDERLINED);
    assert_cue(&buf, "Li", underlined);
    assert!(!underlined(style_at(&buf, "brary")));

    let mut app = app_with(Theme::ansi16(), entries(), t0);
    filter_by(&mut app, "/li", t0);
    let buf = render_buffer(&mut app, t0, 80, 9);
    insta::assert_snapshot!("filter_editing_ansi16", styled(&buf));
    let cyan_underlined = |s: Style| s.fg == Some(Color::Cyan) && underlined(s);
    assert_cue(&buf, "li", cyan_underlined);
    assert_cue(&buf, "3 of 7", |s| s.fg == Some(Color::DarkGray));
    // In a name, only the matched characters are highlighted.
    let row = text(&buf)
        .lines()
        .position(|l| l.contains("notes.sqlite"))
        .map(|r| u16::try_from(r).unwrap())
        .expect("row");
    let at = (0..68)
        .find(|&x| {
            (x..x + 12)
                .map(|x| buf[(x, row)].symbol())
                .collect::<String>()
                == "notes.sqlite"
        })
        .expect("name");
    let hits: Vec<bool> = (at..at + 12)
        .map(|x| underlined(buf[(x, row)].style()))
        .collect();
    let want: Vec<bool> = "notes.sqlite"
        .chars()
        .enumerate()
        .map(|(i, _)| i == 8 || i == 9)
        .collect();
    assert_eq!(hits, want);
}

#[test]
fn inactive_filter_counts_nothing_until_opened() {
    let t0 = Instant::now();
    let mut app = app_with(Theme::no_color(), entries(), t0);
    assert_eq!(app.session.filter.shown(false), (0, 0));
    dfdu_session::filter::open(&mut app.session);
    assert_eq!(app.session.filter.shown(false), (7, 7));
    assert_eq!(app.session.filter.shown(true), (6, 6), "one is hidden");
    assert_eq!(line_text(&app).as_deref(), Some(" / ▌   7 of 7"));
    filter_by(&mut app, "li", t0);
    assert_eq!(line_text(&app).as_deref(), Some(" / li▌   3 of 7"));
}

#[test]
fn query_typed_before_the_first_snapshot_stays() {
    let t0 = Instant::now();
    let mut app = App::new(Theme::no_color(), t0);
    filter_by(&mut app, "/li", t0);
    app.refresh(snap_with_totals(entries(), 1), t0);
    app.reorder(t0);
    assert!(editing(&app));
    assert_eq!(row_names(&app), ["Library", "notes.sqlite", "lib64"]);
    assert_eq!(line_text(&app).as_deref(), Some(" / li▌   3 of 7"));

    let other = Snapshot {
        dir_path: "/elsewhere".into(),
        ..snap_with_totals(entries(), 2)
    };
    app.refresh(other, t0);
    app.reorder(t0);
    assert!(!editing(&app));
    assert_eq!(line_text(&app), None);
    assert_eq!(app.session.list.rows.len(), 7);
}

#[test]
fn enter_keeps_filter_esc_clears() {
    let t0 = Instant::now();
    let mut app = app_with(Theme::no_color(), entries(), t0);
    filter_by(&mut app, "/dow", t0);
    app.key(press(KeyCode::Enter), t0);
    app.reorder(t0);
    assert!(!editing(&app));
    assert_eq!(row_names(&app), ["Downloads"]);
    assert_eq!(
        line_text(&app).as_deref(),
        Some(" filter: \"dow\" (1 of 7)   / edit  Esc clear")
    );
    // Keys go to the list again: `A` sorts by name instead of being typed.
    app.key(press(KeyCode::Char('A')), t0);
    app.reorder(t0);
    assert_eq!(app.session.sort.key, crate::state::order::SortKey::Name);
    assert_eq!(row_names(&app), ["Downloads"]);

    // `/` edits the query in force; Esc clears it and closes the input.
    filter_by(&mut app, "/n", t0);
    assert_eq!(line_text(&app).as_deref(), Some(" / down▌   1 of 7"));
    app.key(press(KeyCode::Esc), t0);
    app.reorder(t0);
    assert!(!editing(&app));
    assert_eq!(line_text(&app), None);
    assert_eq!(app.session.list.rows.len(), 7);

    // Ctrl-U empties the input; Enter on an empty query closes it.
    filter_by(&mut app, "/xyz", t0);
    assert_eq!(app.session.list.rows, [] as [usize; 0]);
    app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL), t0);
    app.reorder(t0);
    assert_eq!(app.session.list.rows.len(), 7);
    app.key(press(KeyCode::Enter), t0);
    assert_eq!(line_text(&app), None);
}

#[test]
fn esc_in_list_clears_before_closing() {
    let t0 = Instant::now();
    let mut app = app_with(Theme::no_color(), entries(), t0);
    filter_by(&mut app, "/li", t0);
    app.key(press(KeyCode::Enter), t0);
    // An open overlay takes Esc first.
    app.key(press(KeyCode::Char('?')), t0);
    assert!(matches!(app.overlay, Overlay::Help(_)));
    app.key(press(KeyCode::Esc), t0);
    app.reorder(t0);
    assert!(matches!(app.overlay, Overlay::None));
    assert_eq!(app.session.list.rows.len(), 3);
    // Then Esc clears the filter, and a second Esc has nothing left to do.
    assert!(app.key(press(KeyCode::Esc), t0).is_empty());
    app.reorder(t0);
    assert_eq!(app.session.list.rows.len(), 7);
    assert_eq!(line_text(&app), None);
    assert!(!dfdu_session::filter::clear(&mut app.session));
}

#[test]
fn hidden_toggle_and_filter_combine() {
    let t0 = Instant::now();
    let mut app = app_with(Theme::no_color(), entries(), t0);
    filter_by(&mut app, "/d", t0);
    assert_eq!(
        row_names(&app),
        ["dev", ".android", "Downloads", "build.log"]
    );
    app.key(press(KeyCode::Enter), t0);
    app.key(press(KeyCode::Char('e')), t0);
    app.reorder(t0);
    assert_eq!(row_names(&app), ["dev", "Downloads", "build.log"]);
    assert_eq!(
        line_text(&app).as_deref(),
        Some(" filter: \"d\" (3 of 6)   / edit  Esc clear")
    );
    app.key(press(KeyCode::Esc), t0);
    app.reorder(t0);
    assert_eq!(app.session.list.rows.len(), 6, "hidden files stay hidden");
}

#[test]
fn new_entries_during_scan_are_filtered() {
    let t0 = Instant::now();
    let mut app = app_with(Theme::no_color(), entries(), t0);
    filter_by(&mut app, "/li", t0);
    app.key(press(KeyCode::Enter), t0);
    assert_eq!(app.session.list.rows.len(), 3);
    let mut es = entries();
    es.push(test_entry(7, "linux", 1 << 40));
    es.push(test_entry(8, "zzz", 1 << 40));
    let at = t0 + Duration::from_millis(100);
    app.refresh(snap_with_totals(es.clone(), 2), at);
    assert_eq!(
        (app.session.list.rows.len(), app.session.list.new_count),
        (3, 1)
    );
    assert_eq!(app.session.list.new_largest, 1 << 40);
    es.push(test_entry(9, "lilo.conf", 5));
    es.push(test_entry(10, "other", 5));
    app.refresh(snap_with_totals(es, 3), at);
    assert_eq!(
        (app.session.list.rows.len(), app.session.list.new_count),
        (3, 2)
    );
    assert_eq!(
        line_text(&app).as_deref(),
        Some(" filter: \"li\" (5 of 11)   / edit  Esc clear")
    );
    let frame = render(&mut app, at, 80, 9);
    assert!(frame.contains("+ 2 new entries since last sort"), "{frame}");
    app.key(press(KeyCode::Char('S')), at);
    app.reorder(at);
    assert_eq!(
        row_names(&app),
        ["linux", "Library", "notes.sqlite", "lib64", "lilo.conf"]
    );
}

#[test]
fn filter_keeps_sort_order() {
    let t0 = Instant::now();
    let mut app = app_with(Theme::no_color(), entries(), t0);
    let with_o = |names: Vec<String>| -> Vec<String> {
        names.into_iter().filter(|n| n.contains('o')).collect()
    };
    let by_size = with_o(row_names(&app));
    assert_eq!(
        by_size,
        [".android", "Downloads", "notes.sqlite", "build.log"]
    );
    filter_by(&mut app, "/o", t0);
    assert_eq!(row_names(&app), by_size);
    app.key(press(KeyCode::Enter), t0);
    app.key(press(KeyCode::Char('A')), t0);
    app.reorder(t0);
    let filtered_by_name = row_names(&app);
    // Clearing the filter keeps the selected entry selected when the cursor was moved.
    app.key(press(KeyCode::Down), t0);
    app.key(press(KeyCode::Down), t0);
    let picked = app.session.selected_entry().map(|e| e.name.to_vec());
    app.key(press(KeyCode::Esc), t0);
    app.reorder(t0);
    assert_eq!(app.session.list.rows.len(), 7);
    assert_eq!(with_o(row_names(&app)), filtered_by_name);
    assert_eq!(
        app.session.selected_entry().map(|e| e.name.to_vec()),
        picked
    );
    assert_eq!(picked.as_deref(), Some(filtered_by_name[2].as_bytes()));
}

#[test]
fn clearing_the_filter_selects_the_hidden_entry_again() {
    let t0 = Instant::now();
    let mut app = app_with(Theme::no_color(), entries(), t0);
    app.key(press(KeyCode::Down), t0);
    assert_eq!(
        app.session.selected_entry().map(|e| &e.name[..]),
        Some(&b"dev"[..])
    );
    // "dow" hides `dev`; the selection moves to a row that is left.
    filter_by(&mut app, "/dow", t0);
    assert_eq!(row_names(&app), ["Downloads"]);
    let picked = |app: &App| app.session.selected_entry().map(|e| e.name.to_vec());
    assert_eq!(picked(&app).as_deref(), Some(&b"Downloads"[..]));
    // While no other entry was picked, `dev` is selected again once it shows.
    app.key(press(KeyCode::Backspace), t0);
    assert_eq!(picked(&app).as_deref(), Some(&b"Downloads"[..]));
    app.key(press(KeyCode::Backspace), t0);
    assert_eq!(picked(&app).as_deref(), Some(&b"dev"[..]));
    filter_by(&mut app, "ow", t0);
    app.key(press(KeyCode::Esc), t0);
    app.reorder(t0);
    assert_eq!(picked(&app).as_deref(), Some(&b"dev"[..]));
    // An entry picked under the filter stays selected when the filter is cleared.
    filter_by(&mut app, "/o", t0);
    app.key(press(KeyCode::Enter), t0);
    app.key(press(KeyCode::Char('G')), t0);
    let last = picked(&app);
    assert_eq!(last.as_deref(), Some(&b"build.log"[..]));
    assert!(app.key(press(KeyCode::Esc), t0).is_empty());
    app.reorder(t0);
    assert_eq!(app.session.list.rows.len(), 7);
    assert_eq!(picked(&app), last);
}

/// The rows an exact sort gives of the entries the filter and the hidden toggle show.
fn exact_rows(app: &App) -> Vec<usize> {
    let mut o = crate::state::order::Order::default();
    let visible = |i: usize, e: &EntryData| {
        !e.ghost && (!app.session.hide_hidden || !e.hidden()) && app.session.filter.matches(i)
    };
    o.sort_exact(&app.session.snap.entries, visible, app.session.sort);
    o.rows().to_vec()
}

#[test]
fn filter_rows_are_those_of_an_exact_sort() {
    let t0 = Instant::now();
    let mut es = entries();
    // Equal sizes, so the order of these comes from the tie-breaks.
    es.push(test_entry(7, "dove", 9 << 30));
    es.push(test_entry(8, "Dot", 9 << 30));
    let mut app = app_with(Theme::no_color(), es.clone(), t0);
    let steps: [&[KeyCode]; 7] = [
        &[KeyCode::Char('/'), KeyCode::Char('o')],
        &[KeyCode::Char('w')],
        &[KeyCode::Backspace],
        &[KeyCode::Backspace],
        &[KeyCode::Char('d')],
        &[KeyCode::Char('z')],
        &[KeyCode::Esc],
    ];
    let mut seen = Vec::new();
    for keys in steps {
        for &k in keys {
            app.key(press(k), t0);
        }
        app.reorder(t0);
        assert_eq!(app.session.list.rows, exact_rows(&app), "after {keys:?}");
        seen.push(app.session.list.rows.len());
    }
    assert_eq!(seen, [6, 1, 6, 9, 6, 0, 9]);

    // During a scan, an entry that arrived since the last sort is sorted in with the rest.
    filter_by(&mut app, "/do", t0);
    es.push(test_entry(9, "docs", 10 << 30));
    app.refresh(snap_with_totals(es, 2), t0);
    assert_eq!(app.session.list.new_count, 1);
    filter_by(&mut app, "s", t0);
    assert_eq!(row_names(&app), ["docs", "Downloads"]);
    assert_eq!(app.session.list.rows, exact_rows(&app));
    assert_eq!(app.session.list.new_count, 0);
}

#[test]
fn a_snapshot_before_the_cleared_filter_is_applied_is_sorted_in() {
    let t0 = Instant::now();
    let mut app = app_with(Theme::no_color(), entries(), t0);
    filter_by(&mut app, "/d", t0);
    app.key(press(KeyCode::Esc), t0);
    // The scan's next snapshot arrives before the loop applies the cleared filter.
    let mut es = entries();
    es[6].size = 200 << 30;
    es.push(test_entry(7, "zeta", 1 << 30));
    app.refresh(snap_with_totals(es, 2), t0);
    app.reorder(t0);
    assert_eq!(app.session.list.rows, exact_rows(&app));
    assert_eq!(row_names(&app)[0], "build.log");
    assert_eq!(app.session.list.rows.len(), 8);
}

#[test]
fn showing_hidden_files_before_the_cleared_filter_is_applied_shows_them() {
    let t0 = Instant::now();
    let mut app = app_with(Theme::no_color(), entries(), t0);
    app.key(press(KeyCode::Char('e')), t0);
    app.reorder(t0);
    assert_eq!(app.session.list.rows.len(), 6);
    filter_by(&mut app, "/d", t0);
    app.key(press(KeyCode::Esc), t0);
    app.key(press(KeyCode::Char('e')), t0);
    app.reorder(t0);
    assert_eq!(app.session.list.rows, exact_rows(&app));
    assert_eq!(app.session.list.rows.len(), 7);
}

#[test]
fn filter_rows_after_an_automatic_sort_are_those_of_an_exact_sort() {
    let t0 = Instant::now();
    let es = |x: u64| vec![test_entry(0, "x", x), test_entry(1, "y", 1_001_000)];
    let mut app = app_with(Theme::no_color(), es(1_000_000), t0);
    assert_eq!(row_names(&app), ["y", "x"]);
    // `x` overtakes `y` by less than 2%: the automatic sort keeps them where they are.
    let later = t0 + Duration::from_secs(5);
    app.refresh(snap_with_totals(es(1_003_000), 2), later);
    app.reorder(later);
    assert_eq!(row_names(&app), ["y", "x"]);
    app.key(press(KeyCode::Char('/')), later);
    app.reorder(later);
    assert_eq!(
        row_names(&app),
        ["y", "x"],
        "opening the filter moves nothing"
    );
    // Any change of the query gives the rows of an exact sort, like `S`.
    filter_by(&mut app, " ", later);
    assert_eq!(row_names(&app), ["x", "y"]);
    filter_by(&mut app, "x", later);
    assert_eq!(row_names(&app), ["x"]);
    app.key(press(KeyCode::Backspace), later);
    app.reorder(later);
    assert_eq!(row_names(&app), ["x", "y"]);
    assert_eq!(app.session.list.rows, exact_rows(&app));
}

#[test]
fn non_utf8_names_match_as_displayed() {
    let t0 = Instant::now();
    let raw = EntryData {
        name: b"caf\xe9.txt"[..].into(),
        ..test_entry(0, "", 30)
    };
    let es = vec![raw, test_entry(1, "Café", 20), test_entry(2, "cfg", 10)];
    let mut app = app_with(Theme::no_color(), es, t0);
    filter_by(&mut app, "/caf", t0);
    assert_eq!(row_names(&app), ["caf\u{fffd}.txt", "Café"]);
    // The replacement character is matched like the others, and highlighted in place.
    app.key(press(KeyCode::Esc), t0);
    filter_by(&mut app, "/\u{fffd}t", t0);
    assert_eq!(row_names(&app), ["caf\u{fffd}.txt"]);
    let buf = render_buffer(&mut app, t0, 60, 7);
    assert_cue(&buf, "\u{fffd}", |s| {
        s.add_modifier.contains(Modifier::UNDERLINED)
    });
    assert!(
        !style_at(&buf, ".txt")
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
}

/// `n` entries with names like `data-0000042-report.json`.
fn synthetic(n: usize) -> Snapshot {
    const WORDS: [&str; 8] = [
        "cache",
        "build",
        "report",
        "data",
        "node_modules",
        "image",
        "Video",
        "tmp",
    ];
    const EXTS: [&str; 5] = ["json", "log", "png", "rs", "tar.gz"];
    let base = test_entry(0, "x", 1);
    let entries = (0..n)
        .map(|i| EntryData {
            name: format!(
                "{}-{i:07}-{}.{}",
                WORDS[i % 8],
                WORDS[(i / 8) % 8],
                EXTS[i % 5]
            )
            .as_bytes()
            .into(),
            ..base.clone()
        })
        .collect();
    Snapshot {
        entries,
        ..Default::default()
    }
}

/// Times the first keystrokes of a filter in a directory of `n` entries of different sizes
/// whose last sort was automatic, as when its scan finished, as the run loop handles them,
/// each with its frame: `/`, which keeps an exact sort of the entries, then `e`, which
/// prepares the filter and picks its rows from that sort.
fn first_keystroke_times(n: usize) {
    let t0 = Instant::now();
    let mut es = synthetic(n).entries;
    for (i, e) in es.iter_mut().enumerate() {
        e.size = (i as u64 * 7919) % 1_000_003;
    }
    let mut app = app_with(Theme::no_color(), es.clone(), t0);
    let later = t0 + Duration::from_secs(5);
    app.refresh(snap_with_totals(es, 2), later);
    render_buffer(&mut app, later, 100, 30);
    let mut times = Vec::new();
    for c in ['/', 'e'] {
        let t = Instant::now();
        app.key(press(KeyCode::Char(c)), later);
        app.reorder(later);
        render_buffer(&mut app, later, 100, 30);
        times.push(t.elapsed());
    }
    let shown = app.session.list.rows.len();
    println!(
        "first filter keystrokes on {n} entries, each with its frame: {times:?}, {shown} rows"
    );
    assert!(shown > 0 && shown < n);
    for t in times {
        assert!(t < Duration::from_millis(50), "{t:?}");
    }
}

#[test]
#[ignore = "builds a million entries; run in release. On an M4 Max, release, with the frame: \
            `/` 32-34 ms and `e` 29-33 ms. Debug: `/` 0.59 s, `e` 0.48 s"]
fn filtering_a_large_directory_is_fast_at_a_million_entries() {
    first_keystroke_times(1_000_000);
}
