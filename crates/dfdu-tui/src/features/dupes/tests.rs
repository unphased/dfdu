use std::fs;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use dfdu_engine::{Command as EngineCommand, Engine, EngineOptions, Model};
use dfdu_scan::{JobOptions, PortableBackend};
use dfdu_session::dupes::Run;
use dfdu_tree::Target;
use ratatui::crossterm::event::KeyCode;
use ratatui::style::Modifier;

use super::*;
use crate::app::views::View;
use crate::model::snapshot;
use crate::tests::{press, render};
use crate::theme::Theme;

/// The view needs a read ([`session::stale`]).
fn stale(app: &App) -> bool {
    session::stale(&app.session)
}

/// How many files a comparison would read ([`session::count`]).
fn count(m: &Model, app: &App) -> Option<(u64, usize)> {
    session::count(m, &app.session)
}

#[test]
fn duplicates_are_found_marked_and_shown() {
    let tmp = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let dir = tmp.path().to_path_buf();
    fs::create_dir_all(dir.join("photos/old")).unwrap();
    let data: Vec<u8> = (0..(u32::try_from(min_size()).unwrap() + 5000))
        .map(|i| (i % 249) as u8)
        .collect();
    fs::write(dir.join("photos/a.jpg"), &data).unwrap();
    fs::write(dir.join("photos/old/a copy.jpg"), &data).unwrap();
    fs::write(dir.join("b.jpg"), &data).unwrap();
    fs::write(dir.join("other.jpg"), vec![1u8; data.len()]).unwrap();
    let engine = Engine::start(
        PortableBackend::default(),
        EngineOptions {
            workers: 2,
            ..Default::default()
        },
    );
    engine.send(EngineCommand::Scan {
        path: dir.clone().into(),
        opts: JobOptions::default(),
    });
    assert!(engine.wait_idle(Some(Duration::from_secs(20))).is_idle());
    let t0 = Instant::now();
    let mut app = App::new(Theme::no_color(), t0);
    let mut target = Target::root();
    let snap = engine.read(|m| snapshot::build(m, &mut target, app.session.size_mode));
    app.session.target = target;
    app.refresh(snap, t0);

    // Nothing is compared until asked, and another machine's tree (remote or from a file) is
    // not compared with this one's files.
    assert!(app.key(press(KeyCode::Char('4')), t0).is_empty());
    assert!(app.key(press(KeyCode::Enter), t0).is_empty());
    assert!(
        app.session
            .status
            .as_deref()
            .unwrap()
            .contains("this machine's files")
    );
    app.session.outside.local = true;
    let fx = app.key(press(KeyCode::Enter), t0);
    assert!(matches!(fx[..], [Effect::FindDupes]));
    let out = dfdu_session::effect::carry_out(&mut app.session, &engine, fx);
    assert!(
        out.is_empty(),
        "the session reads the candidates and starts: {out:?}"
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while dfdu_session::dupes::running(&app.session) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(stale(&app), "the files need their nodes");
    app.session.extras.dupes = engine.read(|m| dfdu_session::dupes::read(m, &app.session));
    assert!(!stale(&app));
    engine.shutdown(Duration::from_millis(200));

    let out = render(&mut app, t0, 100, 12);
    assert!(out.contains("3 copies of 1.0 MiB"), "{out}");
    assert!(out.contains("photos/old/a copy.jpg"), "{out}");
    assert!(!out.contains("other.jpg"), "{out}");
    // `A` marks every copy but the oldest.
    app.key(press(KeyCode::Char('A')), t0);
    assert_eq!(app.session.marks.items().len(), 2);
    app.key(press(KeyCode::Char('A')), t0);
    assert_eq!(app.session.marks.items().len(), 2);
    // Space on the group unmarks its copies again.
    app.session.dupes.cursor = 0;
    app.key(press(KeyCode::Char(' ')), t0);
    assert!(app.session.marks.is_empty());
    // A deleted file leaves the group; two left still make one.
    let gone = found(&app).unwrap().groups[0].files[0].path.clone();
    app.session.extras.dupes.insert(gone, None);
    let out = render(&mut app, t0, 100, 12);
    assert!(out.contains("2 copies of 1.0 MiB"), "{out}");
}

/// A comparison done with `groups` found: files below `/r` with their dates, each still in the
/// tree.
pub(super) fn found_in(app: &mut App, groups: &[&[(&str, i64)]]) {
    let mut bits = 0;
    let mut file = |path: &str, mtime: i64| {
        bits += 1;
        app.session
            .extras
            .dupes
            .insert(path.into(), Some(NodeRef::from_bits(bits)));
        Candidate {
            path: path.into(),
            size: 1 << 20,
            alloc: 1 << 20,
            dev_ino: (1, bits),
            mtime,
            links: 0,
            private: None,
        }
    };
    let groups = (groups.iter())
        .map(|files| Group {
            size: 1 << 20,
            hash: [0; 32],
            files: files.iter().map(|&(p, mtime)| file(p, mtime)).collect(),
        })
        .collect();
    let result = Arc::new(OnceLock::new());
    result
        .set(Found {
            groups,
            compared: 4,
            ..Found::default()
        })
        .unwrap();
    app.session.view = View::Duplicates;
    app.session.dupes = dfdu_session::dupes::Dupes::default();
    app.session.dupes.run = Some(Run {
        root: "/r".into(),
        progress: Arc::default(),
        cancel: Arc::default(),
        result,
    });
}

/// The directory a group's files share goes in its header; each row shows the rest of its path,
/// shortened in the middle to leave the dates a column at the right edge.
#[test]
fn rows_show_the_path_below_the_shared_directory_and_align_the_dates() {
    let t0 = Instant::now();
    let mut app = App::new(Theme::no_color(), t0);
    let trip = "/r/photos/2026/trip-to-the-coast";
    let long = format!("{trip}/edited/exports for the family album/IMG_0001 (final, small).jpg");
    found_in(
        &mut app,
        &[
            &[
                (&format!("{trip}/IMG_0001.jpg"), 1_760_000_000),
                (&long, 1_761_000_000),
            ],
            &[
                ("/r/a.bin", 1_700_000_000),
                ("/r/backup/a.bin", 1_700_000_000),
            ],
        ],
    );
    // `render` shows `\` as `/`, so the frame is the same on every platform.
    let out = render(&mut app, t0, 80, 14);
    insta::assert_snapshot!("dupes_80x14", out);
    let lines: Vec<&str> = out.lines().collect();
    let groups: Vec<&str> = (lines.iter().copied())
        .filter(|l| l.contains(" copies of "))
        .collect();
    assert!(
        groups[0].contains("· in photos/2026/trip-to-the-coast/"),
        "{out}"
    );
    // Files in the scan root share no directory: the header names none.
    assert!(!groups[1].contains(" in "), "{out}");
    let files: Vec<&str> = (lines.iter().copied())
        .filter(|l| ["   [", "   KEEP"].iter().any(|s| l.starts_with(s)))
        .collect();
    assert_eq!(files.len(), 4, "{out}");
    assert!(files.iter().all(|l| l.chars().count() <= 80), "{out}");
    assert!(files.iter().all(|l| !l.contains("photos")), "{out}");
    // The paths start in one column, after the size and the age.
    let starts: Vec<_> = files
        .iter()
        .map(|l| {
            l.char_indices()
                .skip(28)
                .find(|&(_, c)| c != ' ')
                .map(|(i, _)| i)
        })
        .collect();
    assert!(starts.iter().all(|&s| s == starts[0]), "{out}");
    assert!(files[0].ends_with(" IMG_0001.jpg"), "{out}");
    assert!(
        files[1].contains('…') && files[1].ends_with("small).jpg"),
        "{out}"
    );
    assert!(files[3].ends_with(" backup/a.bin"), "{out}");
}

/// Marking every copy keeps the oldest, labelled `keep` in its row; `K` cycles the rule through
/// newest, shortest path and first in path order, and the summary line names it.
#[test]
fn the_keep_rule_picks_the_copy_marking_leaves() {
    let t0 = Instant::now();
    let mut app = App::new(Theme::no_color(), t0);
    found_in(
        &mut app,
        &[&[
            ("/r/b/new.bin", 1_760_000_000),
            ("/r/a/archive/old.bin", 1_700_000_000),
            ("/r/c.bin", 1_730_000_000),
        ]],
    );
    let kept_after_a = |app: &mut App| {
        dfdu_session::marks::clear(&mut app.session);
        app.key(press(KeyCode::Char('A')), t0);
        let marked: Vec<String> = (app.session.marks.items().iter())
            .map(|i| i.target.path.display().to_string())
            .collect();
        let files = &found(app).unwrap().groups[0].files;
        let left: Vec<String> = (files.iter())
            .map(|c| c.path.display().to_string())
            .filter(|p| !marked.contains(p))
            .collect();
        assert_eq!(left.len(), 1, "{marked:?}");
        left[0].clone()
    };
    assert_eq!(kept_after_a(&mut app), "/r/a/archive/old.bin");
    let out = render(&mut app, t0, 80, 10);
    // Every rule is listed, the one in force highlighted.
    let in_force = |app: &mut App, rule: &str| {
        let buf = crate::tests::render_buffer(app, t0, 80, 10);
        let style = crate::tests::style_at(&buf, rule);
        style.add_modifier.contains(Modifier::REVERSED)
    };
    assert!(
        out.contains("keep oldest · newest · shortest path · first in path order"),
        "{out}"
    );
    assert!(in_force(&mut app, "oldest") && !in_force(&mut app, "newest"));
    let row = |out: &str, name: &str| {
        (out.lines())
            .find(|l| l.contains(name))
            .unwrap_or_else(|| panic!("{name}: {out}"))
            .to_owned()
    };
    // Unticked, the kept copy says so.
    app.key(press(KeyCode::Char('X')), t0);
    let out = render(&mut app, t0, 80, 10);
    assert!(row(&out, "old.bin").contains(" KEEP "), "{out}");
    assert!(row(&out, "new.bin").contains(" [ ] "), "{out}");
    insta::assert_snapshot!("dupes_keep_80x10", out);

    for want in ["newest", "shortest path", "first in path order", "oldest"] {
        app.key(press(KeyCode::Char('K')), t0);
        assert_eq!(app.session.dupes.keep.label(), want);
        let status = app.session.status.clone().unwrap_or_default();
        assert!(status.contains(want), "{status}");
        let kept = kept_after_a(&mut app);
        assert!(in_force(&mut app, want), "{want}");
        let out = render(&mut app, t0, 80, 12);
        let expect = match want {
            "newest" => "/r/b/new.bin",
            "shortest path" => "/r/c.bin",
            "first in path order" | "oldest" => "/r/a/archive/old.bin",
            _ => unreachable!(),
        };
        assert_eq!(kept, expect, "{want}");
        let name = expect.rsplit('/').next().unwrap();
        assert!(row(&out, name).contains(" KEEP "), "{out}");
    }
}

/// Before comparing, the view says what it does and how many files it would read, and
/// compares on `Enter`; `Esc` goes back to Files instead.
#[test]
fn nothing_is_compared_until_asked() {
    use crate::tests::{Fixture, sync};
    let (mut fx, root) = Fixture::new();
    let big = 2 * min_size();
    for name in ["a.iso", "b.iso", "c.iso"] {
        fx.file(root, name, big);
    }
    fx.file(root, "other.bin", big + 1);
    fx.m.tree.dir_mut(root).expect("root").state = dfdu_tree::DirState::Complete;
    let t0 = Instant::now();
    let mut app = App::new(Theme::no_color(), t0);
    sync(&mut app, &fx.m, t0);
    app.session.snap.busy = false;
    // A tree from another machine (or a file) counts nothing and offers no comparison.
    assert!(app.key(press(KeyCode::Char('4')), t0).is_empty());
    assert!(!stale(&app));
    let frame = render(&mut app, t0, 100, 20);
    assert!(frame.contains("this scan is remote or from"), "{frame}");
    assert!(!frame.contains("compare now"), "{frame}");
    app.key(press(KeyCode::Esc), t0);
    app.session.outside.local = true;
    assert!(app.key(press(KeyCode::Char('4')), t0).is_empty());
    assert!(stale(&app), "the files to compare are counted");
    sync(&mut app, &fx.m, t0);
    app.session.snap.busy = false;
    assert_eq!(app.session.dupes.candidates, Some(3));
    let frame = render(&mut app, t0, 100, 20);
    assert!(frame.contains("╭ Duplicates ─"), "{frame}");
    assert!(frame.contains("3 files share their size"), "{frame}");
    assert!(
        frame.contains("Enter  compare now      Esc  back to Files"),
        "{frame}"
    );
    insta::assert_snapshot!("dupes_not_compared_100x20", frame);
    app.key(press(KeyCode::Esc), t0);
    assert!(!dfdu_session::dupes::active(&app.session));
    app.key(press(KeyCode::Char('4')), t0);
    // Counted once for this tree: coming back, or passing through, counts nothing again.
    assert!(!stale(&app));
    assert_eq!(count(&fx.m, &app), None);
    let fx_out = app.key(press(KeyCode::Enter), t0);
    assert!(matches!(fx_out[..], [Effect::FindDupes]), "{fx_out:?}");
}

#[test]
fn half_pages_move_the_cursor_and_y_copies_the_file() {
    let t0 = Instant::now();
    let mut app = App::new(Theme::no_color(), t0);
    found_in(
        &mut app,
        &[&[("/r/a", 1), ("/r/b", 2), ("/r/c", 3), ("/r/d", 4)]],
    );
    app.session
        .set_viewport(dfdu_session::view::View::Duplicates, 4);
    let ctrl = |c| {
        KeyEvent::new(
            KeyCode::Char(c),
            ratatui::crossterm::event::KeyModifiers::CONTROL,
        )
    };
    app.key(ctrl('d'), t0);
    assert_eq!(app.session.dupes.cursor, 2);
    let fx = app.key(press(KeyCode::Char('y')), t0);
    assert!(
        matches!(&fx[..], [Effect::Copy(p)] if p.as_str() == "/r/b"),
        "{fx:?}"
    );
    app.key(ctrl('u'), t0);
    assert_eq!(app.session.dupes.cursor, 0);
    let fx = app.key(press(KeyCode::Char('y')), t0);
    assert!(fx.is_empty(), "a group's row has no path: {fx:?}");
}

/// The copies `d` would delete in a group: the ticked ones still in the tree.
fn ticked(app: &App, group: usize) -> usize {
    let g = &found(app).unwrap().groups[group];
    (g.files.iter())
        .filter_map(|c| live(app, c))
        .filter(|&n| app.session.marks.contains(n))
        .count()
}

/// Every copy of a group ticked by hand: `d` refuses rather than delete them all.
#[test]
fn deleting_every_copy_of_a_group_is_refused() {
    let t0 = Instant::now();
    let mut app = App::new(Theme::no_color(), t0);
    found_in(
        &mut app,
        &[&[("/r/b.bin", 1_760_000_000), ("/r/a.bin", 1_700_000_000)]],
    );
    app.session.dupes.cursor = 1;
    app.key(press(KeyCode::Char(' ')), t0);
    app.key(press(KeyCode::Char(' ')), t0);
    assert_eq!(ticked(&app, 0), 2);
    for key in ['d', 'D'] {
        assert!(app.key(press(KeyCode::Char(key)), t0).is_empty());
        assert!(matches!(app.overlay, crate::app::Overlay::None));
        let status = app.session.status.clone().unwrap_or_default();
        assert!(status.contains("every copy of"), "{status}");
    }
    // One unticked, the other goes.
    app.session.dupes.cursor = 1;
    app.key(press(KeyCode::Char(' ')), t0);
    app.key(press(KeyCode::Char('d')), t0);
    assert!(app.session.confirm.is_some());
}

/// The footer keeps every key of the view at 120 columns, and at 80 lets `q quit` go before
/// `[ ] matches` and the arrows.
#[test]
fn the_footer_keeps_the_views_keys_first() {
    let t0 = Instant::now();
    let mut app = App::new(Theme::no_color(), t0);
    found_in(&mut app, &[&[("/r/a", 1), ("/r/b", 2)]]);
    let footer = |app: &mut App, w| {
        let frame = render(app, t0, w, 12);
        frame.lines().last().unwrap_or_default().to_owned()
    };
    let wide = footer(&mut app, 180);
    for want in ["R compare again", "K keep rule", "[ ] matches", "q quit"] {
        assert!(wide.contains(want), "{wide}");
    }
    let narrow = footer(&mut app, 80);
    assert!(narrow.contains("↑↓ move"), "{narrow}");
    if narrow.contains("q quit") {
        assert!(narrow.contains("[ ] matches"), "{narrow}");
    }
}
