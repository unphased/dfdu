//! Frames of the list's columns at the widths of the approved mockups: one directory of 61
//! entries with changes since the last recorded scan, data also used outside the scan, the data
//! shared between entries and a long file name, drawn at 120, 100, 80, 64 and 48 columns.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use dfdu_store::history::{Growth, Sizes};
use dfdu_tree::{HostPath, Totals};
use ratatui::crossterm::event::KeyCode;
use ratatui::style::Modifier;

use super::{press, render, render_buffer};
use crate::app::App;
use crate::model::snapshot::{EntryData, Snapshot, test_entry};
use crate::theme::{ColorLevel, Theme};

const MIB: f64 = (1u64 << 20) as f64;
const GIB: f64 = (1u64 << 30) as f64;

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test sizes are small positive numbers of GiB"
)]
fn bytes(n: f64, unit: f64) -> u64 {
    (n * unit) as u64
}

/// 2026-10-18 12:00 UTC, when the last recorded scan finished.
const BASELINE: i64 = 1_792_324_800;

/// `~/dev` with the rows of the mockups, the third selected.
fn dev(t0: Instant) -> App {
    dev_in(Theme::no_color(), t0)
}

/// [`dev`] drawn with `theme`.
fn dev_in(theme: Theme, t0: Instant) -> App {
    // Name, size in GiB, items, change in MiB since the last scan, MiB also used outside.
    let rows: [(&str, f64, u64, f64, f64); 21] = [
        ("dfdu", 40.7, 1_500_000, 3481.6, 0.0),
        ("brewd", 16.6, 227_400, 0.0, 880.4),
        ("experiment", 16.3, 108_200, 2150.4, 196.5),
        ("foodchapters", 12.8, 507_100, 0.0, 171.2),
        ("MaterialKolor", 10.9, 238_000, -1228.8, 0.0),
        ("filmstrip", 7.3, 150_200, 0.0, 22.9),
        (
            "screen-recording-2026-10-18-cleanup-view-walkthrough-final.mov",
            7.1,
            1,
            0.0,
            0.0,
        ),
        ("whalesay", 7.0, 206_300, 0.0, 252.3),
        ("SIGO", 6.5, 126_300, 812.0, 179.0),
        ("toebeans", 6.4, 256_400, 0.0, 573.7),
        ("jolt", 4.7, 36_100, 0.0, 79.8),
        ("compass", 3.7, 67_100, 0.0, 0.0),
        ("bookdb", 3.5, 96_100, 0.0, 324.3),
        ("connectivity", 3.3, 41_200, 0.0, 0.0),
        ("kmpalette", 3.1, 39_200, 0.0, 0.0),
        ("(shared between entries)", 2.5, 0, 0.0, 0.0),
        ("1se-android", 2.2, 6_600, 0.0, 0.0),
        ("ReadingJourney", 1.9, 26_800, 0.0, 140.0),
        ("MaterialKolorBuilder", 1.5, 38_200, 640.0, 0.0),
        ("drag-select-compose", 1.4, 33_400, 0.0, 0.0),
        ("kodio", 1.1, 25_900, 0.0, 0.0),
    ];
    let mut entries = Vec::new();
    let mut then = std::collections::BTreeMap::new();
    for (i, &(name, gib, items, mib, out)) in rows.iter().enumerate() {
        let size = bytes(gib, GIB);
        let change = bytes(mib.abs(), MIB);
        let disk = if mib < 0.0 {
            size + change
        } else {
            size - change
        };
        let file = name.ends_with(".mov");
        let pseudo = name.starts_with('(');
        entries.push(EntryData {
            is_dir: !file && !pseudo,
            pseudo,
            items,
            mtime: BASELINE - 86_400 * i64::try_from(i).unwrap_or(0),
            shared_out: bytes(out, MIB),
            ..test_entry(u32::try_from(i).unwrap_or(0), name, size)
        });
        if !pseudo {
            then.insert(
                name.as_bytes().to_vec(),
                Sizes {
                    disk,
                    apparent: disk,
                    entries: items,
                },
            );
        }
    }
    // Small directories after them, 61 entries in all.
    for i in 21..61 {
        let size = bytes(f64::from(61 - i) * 10.0, MIB);
        entries.push(EntryData {
            is_dir: true,
            items: 100,
            ..test_entry(i, &format!("project-{i:02}"), size)
        });
    }
    let total = Totals {
        apparent: bytes(163.3, GIB),
        alloc: bytes(163.3, GIB),
        items: 4_500_000,
        ..Totals::default()
    };
    let root = test_entry(99, "dev", 0);
    let snap = Snapshot {
        dir_path: HostPath::from("/home/jo/dev"),
        root_path: HostPath::from("/home/jo/dev"),
        dir_node: Some(root.node),
        at_root: true,
        dir_total: total,
        root_total: total,
        entries,
        busy: false,
        version: 1,
        ..Default::default()
    };
    let mut app = App::new(theme, t0);
    app.session.home = Some(PathBuf::from("/home/jo"));
    app.session.growth = Some(Arc::new(Growth {
        root: PathBuf::from("/home/jo/dev"),
        at: BASELINE,
        sizes: then,
    }));
    app.refresh(snap, t0);
    app.key(press(KeyCode::Down), t0);
    app.key(press(KeyCode::Down), t0);
    app
}

#[test]
fn list_columns_at_120() {
    let t0 = Instant::now();
    insta::assert_snapshot!("list_120", render(&mut dev(t0), t0, 120, 32));
}

#[test]
fn list_columns_at_100() {
    let t0 = Instant::now();
    insta::assert_snapshot!("list_100", render(&mut dev(t0), t0, 100, 24));
}

#[test]
fn list_columns_at_80() {
    let t0 = Instant::now();
    insta::assert_snapshot!("list_80", render(&mut dev(t0), t0, 80, 24));
}

#[test]
fn list_columns_at_64() {
    let t0 = Instant::now();
    insta::assert_snapshot!("list_64", render(&mut dev(t0), t0, 64, 20));
}

#[test]
fn list_columns_at_48() {
    let t0 = Instant::now();
    insta::assert_snapshot!("list_48", render(&mut dev(t0), t0, 48, 16));
}

/// `c` adds the modification date, then takes the optional columns away.
#[test]
fn c_cycles_the_columns() {
    let t0 = Instant::now();
    let mut app = dev(t0);
    app.key(press(KeyCode::Char('c')), t0);
    let out = render(&mut app, t0, 140, 12);
    assert!(out.contains("items  modified       Δ 10-18"), "{out}");
    assert!(out.contains("2026-10-16"), "{out}");
    app.key(press(KeyCode::Char('c')), t0);
    let out = render(&mut app, t0, 140, 12);
    assert!(out.contains("columns: size and share only"), "{out}");
    let names = out.lines().nth(2).expect("column names");
    assert!(!names.contains("items") && !names.contains("Δ"), "{out}");
}

/// Two entries modified a second apart, plus rows without a filesystem timestamp.
fn dated(t0: Instant) -> App {
    let mut app = App::new(Theme::no_color(), t0);
    let entries = vec![
        EntryData {
            mtime: BASELINE + 1,
            ..test_entry(0, "new.txt", 100)
        },
        EntryData {
            is_dir: true,
            items: 42,
            mtime: BASELINE,
            ..test_entry(1, "old-dir", 200)
        },
        test_entry(2, "unknown.txt", 300),
        EntryData {
            pseudo: true,
            mtime: BASELINE,
            ..test_entry(3, "(shared)", 400)
        },
    ];
    app.refresh(super::snap_with_totals(entries, 1), t0);
    app
}

#[test]
fn sorting_by_modified_shows_seconds_for_files_and_directories() {
    let t0 = Instant::now();
    let mut app = dated(t0);
    app.key(press(KeyCode::Char('M')), t0);
    app.reorder(t0);
    let out = render(&mut app, t0, 120, 12);
    assert!(
        out.lines().nth(2).unwrap().contains("modified (UTC)"),
        "{out}"
    );
    let file = out.lines().find(|l| l.contains("new.txt")).unwrap();
    let dir = out.lines().find(|l| l.contains("old-dir/")).unwrap();
    assert!(file.contains("2026-10-18 12:00:01"), "{out}");
    assert!(dir.contains("2026-10-18 12:00:00"), "{out}");
    assert!(
        out.find("new.txt").unwrap() < out.find("old-dir/").unwrap(),
        "{out}"
    );
    for name in ["unknown.txt", "(shared)"] {
        let row = out.lines().find(|l| l.contains(name)).unwrap();
        assert!(!row.contains("2026-") && !row.contains("1970-"), "{row}");
    }
    app.key(press(KeyCode::Char('M')), t0);
    app.reorder(t0);
    let out = render(&mut app, t0, 120, 12);
    assert!(
        out.find("old-dir/").unwrap() < out.find("new.txt").unwrap(),
        "{out}"
    );
    assert!(out.contains("2026-10-18 12:00:01"), "{out}");
}

#[test]
fn the_sort_column_overrides_c_without_changing_its_preference() {
    let t0 = Instant::now();
    let mut app = dated(t0);
    app.key(press(KeyCode::Char('c')), t0);
    app.key(press(KeyCode::Char('c')), t0);
    let preference = app.columns;
    for (key, title) in [('M', "modified (UTC)"), ('C', "items")] {
        app.key(press(KeyCode::Char(key)), t0);
        let out = render(&mut app, t0, 120, 12);
        assert!(out.lines().nth(2).unwrap().contains(title), "{out}");
        assert_eq!(app.columns, preference);
        app.key(press(KeyCode::Char('s')), t0);
        let out = render(&mut app, t0, 120, 12);
        let header = out.lines().nth(2).unwrap();
        assert!(
            !header.contains("modified") && !header.contains("items"),
            "{out}"
        );
    }
    app.key(press(KeyCode::Char('M')), t0);
    app.key(press(KeyCode::Char('c')), t0);
    let out = render(&mut app, t0, 120, 12);
    assert!(out.contains("2026-10-18 12:00:01"), "{out}");
}

#[test]
fn the_active_sort_column_survives_narrow_windows() {
    let t0 = Instant::now();
    for width in [48, 64, 80, 120] {
        for (key, title) in [('M', "modified (UTC)"), ('C', "items")] {
            let mut app = dated(t0);
            app.key(press(KeyCode::Char(key)), t0);
            let out = render(&mut app, t0, width, 12);
            assert!(
                out.lines().nth(2).unwrap().contains(title),
                "width {width}:\n{out}"
            );
            if key == 'M' {
                assert!(out.contains("2026-10-18 12:00:01"), "width {width}:\n{out}");
            } else {
                let dir = out.lines().find(|l| l.contains("old-dir/")).unwrap();
                assert!(dir.contains("42"), "width {width}:\n{out}");
            }
        }
    }
}

#[test]
fn sort_values_are_also_visible_in_tree_mode() {
    let t0 = Instant::now();
    let mut app = dated(t0);
    app.key(press(KeyCode::Char('T')), t0);
    app.key(press(KeyCode::Char('M')), t0);
    app.reorder(t0);
    let out = render(&mut app, t0, 80, 12);
    assert!(out.contains("2026-10-18 12:00:01"), "{out}");
    assert!(out.contains("2026-10-18 12:00:00"), "{out}");
}

#[test]
fn leaving_modified_sort_restores_the_requested_date_column() {
    let t0 = Instant::now();
    let mut app = dated(t0);
    app.key(press(KeyCode::Char('c')), t0);
    app.key(press(KeyCode::Char('M')), t0);
    app.key(press(KeyCode::Char('s')), t0);
    let out = render(&mut app, t0, 120, 12);
    assert!(out.contains("2026-10-18"), "{out}");
    assert!(!out.contains("12:00:01"), "{out}");
}

/// The cursor fills its row up to the scrollbar, and dim text on it takes the normal text
/// colour, in every built-in theme at every colour level.
#[test]
fn the_cursor_row_is_full_and_readable_in_every_theme() {
    let t0 = Instant::now();
    for &name in Theme::builtin_names() {
        for level in [ColorLevel::TrueColor, ColorLevel::Ansi16, ColorLevel::None] {
            let theme = Theme::named(name, level, None).expect("theme");
            let (cursor, dim) = (theme.cursor, theme.fg_dim);
            let mut app = dev_in(theme, t0);
            let buf = render_buffer(&mut app, t0, 80, 24);
            // `experiment/`, below the header lines, the column names and two rows.
            let y = 5;
            let row: String = (0..80).map(|x| buf[(x, y)].symbol()).collect();
            assert!(row.contains("experiment/"), "{name}: {row}");
            for x in 0..79 {
                let cell = &buf[(x, y)];
                let what = format!("{name} {level:?} column {x}: {cell:?}");
                if let Some(bg) = cursor.bg {
                    assert_eq!(cell.bg, bg, "{what}");
                }
                if cursor.add_modifier.contains(Modifier::REVERSED) {
                    assert!(cell.modifier.contains(Modifier::REVERSED), "{what}");
                }
                if dim.fg.is_some() {
                    assert_ne!(Some(cell.fg), dim.fg, "{what}");
                }
                if !dim.add_modifier.is_empty() {
                    assert!(!cell.modifier.contains(dim.add_modifier), "{what}");
                }
            }
        }
    }
}
