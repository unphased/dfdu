//! Deletion against a real filesystem: verification against the scan, refusals, journal. Each
//! test runs with the portable backend, and on Windows with the Windows backend too, whose
//! file ids and volume serial numbers the deleter checks against.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dfdu_engine::{Command, Count, DeleteSummary, Engine, EngineOptions, Mode};
use dfdu_scan::{Backend, JobOptions, PortableBackend};
use dfdu_tree::NodeRef;
use tempfile::TempDir;

mod common;
use common::{node, wait_for};

/// A temporary directory under the canonical temp dir: the engine canonicalises scan roots
/// (on macOS the temp dir is behind the /var symlink), and paths compared with the tree's must
/// match.
fn tagged_tempdir(tag: &str) -> TempDir {
    let base = dunce::canonicalize(std::env::temp_dir()).unwrap();
    tempfile::Builder::new()
        .prefix(&format!("dfdu-del-{tag}-"))
        .tempdir_in(base)
        .unwrap()
}

fn write(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![7u8; bytes]).unwrap();
}

fn fixture() -> TempDir {
    let dir = tagged_tempdir("tree");
    let root = dir.path();
    write(&root.join("keep/a.bin"), 10_000);
    write(&root.join("junk/b.bin"), 50_000);
    write(&root.join("junk/sub/c.bin"), 20_000);
    write(&root.join("file.bin"), 3_000);
    dir
}

fn engine<B: Backend>(backend: B, journal: Option<PathBuf>, read_only: bool) -> Engine<B> {
    Engine::start(
        backend,
        EngineOptions {
            journal_dir: journal,
            read_only,
            check_invariants: true,
            workers: 2,
            ..Default::default()
        },
    )
}

fn scan<B: Backend>(e: &Engine<B>, root: &Path) {
    e.send(Command::Scan {
        path: root.to_path_buf().into(),
        opts: JobOptions::default(),
    });
    assert!(e.wait_idle(Some(Duration::from_secs(20))).is_idle());
}

fn delete<B: Backend>(e: &Engine<B>, items: Vec<NodeRef>, mode: Mode) -> DeleteSummary {
    e.send(Command::Delete { items, mode });
    assert!(e.wait_idle(Some(Duration::from_secs(20))).is_idle());
    e.read(|m| m.status.last_delete.clone())
        .expect("a plan finished")
}

fn root_alloc<B: Backend>(e: &Engine<B>) -> u64 {
    e.read(|m| m.tree.total(m.tree.roots().next().unwrap()).alloc)
}

fn journal_kinds(dir: &Path) -> Vec<String> {
    let mut out = vec![];
    for f in fs::read_dir(dir).unwrap() {
        for line in fs::read_to_string(f.unwrap().path()).unwrap().lines() {
            out.push(
                dfdu_clean::check_line(line)
                    .expect("valid journal line")
                    .to_string(),
            );
        }
    }
    out
}

#[test]
fn permanent_delete_updates_tree_and_journal() {
    permanent_delete(PortableBackend::default);
    #[cfg(windows)]
    permanent_delete(dfdu_scan::WindowsBackend::default);
}

fn permanent_delete<B: Backend>(backend: fn() -> B) {
    let tree = fixture();
    let root = tree.path();
    let jdir = tagged_tempdir("journal");
    let e = engine(backend(), Some(jdir.path().to_path_buf()), false);
    scan(&e, root);
    let before = root_alloc(&e);
    let junk_bytes = e.read(|m| {
        m.tree
            .total(m.tree.resolve(node(&e, "junk")).unwrap())
            .alloc
    });
    let s = delete(&e, vec![node(&e, "junk")], Mode::Permanent);
    assert_eq!((s.deleted, s.failed, s.stale, s.refused), (1, 0, 0, 0));
    assert!(!root.join("junk").exists());
    assert!(root.join("keep/a.bin").exists());
    assert_eq!(root_alloc(&e), before - junk_bytes);
    e.read(|m| {
        assert!(
            m.tree
                .children(m.tree.roots().next().unwrap())
                .all(|c| m.tree.name(c) != b"junk")
        );
    });
    // Read the journal after shutdown: the deleter closes it then (Windows locks are mandatory).
    e.shutdown(Duration::from_secs(5));
    assert_eq!(journal_kinds(jdir.path()), ["session", "intent", "done"]);
}

#[test]
fn changed_items_are_not_deleted() {
    changed_items(PortableBackend::default);
    #[cfg(windows)]
    changed_items(dfdu_scan::WindowsBackend::default);
}

fn changed_items<B: Backend>(backend: fn() -> B) {
    let tree = fixture();
    let root = tree.path();
    let jdir = tagged_tempdir("journal");
    let e = engine(backend(), Some(jdir.path().to_path_buf()), false);
    scan(&e, root);
    let (file, keep, junk) = (node(&e, "file.bin"), node(&e, "keep"), node(&e, "junk"));
    // A file whose size changed, a directory replaced by a new one, a directory swapped for a
    // symlink to another directory.
    write(&root.join("file.bin"), 4_000);
    fs::rename(root.join("keep"), root.join("keep.old")).unwrap();
    fs::create_dir(root.join("keep")).unwrap();
    let elsewhere_dir = tagged_tempdir("elsewhere");
    let elsewhere = elsewhere_dir.path();
    write(&elsewhere.join("precious"), 100);
    fs::remove_dir_all(root.join("junk")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(elsewhere, root.join("junk")).unwrap();
    // A junction, which needs no privilege.
    #[cfg(windows)]
    assert!(
        std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(root.join("junk"))
            .arg(elsewhere)
            .output()
            .unwrap()
            .status
            .success()
    );
    let s = delete(&e, vec![file, keep, junk], Mode::Permanent);
    assert_eq!(s.deleted, 0, "{s:?}");
    assert_eq!(s.stale, 3, "{s:?}");
    assert!(root.join("file.bin").exists() && root.join("keep").exists());
    assert!(
        elsewhere.join("precious").exists(),
        "a symlink target was touched"
    );
    e.shutdown(Duration::from_secs(1));
}

#[test]
fn refusals() {
    refusals_with(PortableBackend::default);
    #[cfg(windows)]
    refusals_with(dfdu_scan::WindowsBackend::default);
}

fn refusals_with<B: Backend>(backend: fn() -> B) {
    let tree = fixture();
    let root = tree.path();
    // The scan root itself.
    let jdir = tagged_tempdir("journal");
    let e = engine(backend(), Some(jdir.path().to_path_buf()), false);
    scan(&e, root);
    let r = e.read(|m| m.tree.node_ref(m.tree.roots().next().unwrap()));
    let s = delete(&e, vec![r], Mode::Permanent);
    assert_eq!((s.deleted, s.refused), (0, 1));
    assert!(root.exists());
    e.shutdown(Duration::from_secs(1));

    // A protected name, and a file below a protected directory: the deleter refuses them
    // without the classifier's marks (this scan has no rules).
    write(&root.join(".env"), 10);
    write(&root.join("keep/.git/config"), 10);
    let e = engine(backend(), Some(jdir.path().to_path_buf()), false);
    scan(&e, root);
    let s = delete(
        &e,
        vec![node(&e, ".env"), node(&e, "keep/.git/config")],
        Mode::Permanent,
    );
    assert_eq!((s.deleted, s.refused), (0, 2));
    assert!(root.join(".env").exists() && root.join("keep/.git/config").exists());
    e.shutdown(Duration::from_secs(1));

    // No journal: nothing is deleted.
    let e = engine(backend(), None, false);
    scan(&e, root);
    let s = delete(&e, vec![node(&e, "junk")], Mode::Permanent);
    assert_eq!((s.deleted, s.refused), (0, 1));
    assert!(root.join("junk").exists());
    e.shutdown(Duration::from_secs(1));

    // Read-only: the request is dropped before a plan exists.
    let jdir = tagged_tempdir("journal");
    let e = engine(backend(), Some(jdir.path().to_path_buf()), true);
    scan(&e, root);
    e.send(Command::Delete {
        items: vec![node(&e, "junk")],
        mode: Mode::Permanent,
    });
    assert!(e.wait_idle(Some(Duration::from_secs(5))).is_idle());
    assert!(e.read(|m| m.status.last_delete.is_none()));
    assert!(root.join("junk").exists());
    e.shutdown(Duration::from_secs(1));
}

#[test]
fn multi_root_deletes_are_refused_without_mutation() {
    let first = fixture();
    let second = fixture();
    let journal = tagged_tempdir("multi-journal");
    let e = engine(
        PortableBackend::default(),
        Some(journal.path().to_path_buf()),
        false,
    );
    scan(&e, first.path());
    scan(&e, second.path());
    let items = e.read(|m| {
        m.tree
            .roots()
            .map(|root| {
                let ix = common::child(&m.tree, root, "junk");
                m.tree.node_ref(ix)
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(items.len(), 2);
    for mode in [Mode::Permanent, Mode::Stage, Mode::Trash] {
        for selection in [items.clone(), vec![items[0]], vec![items[1]]] {
            e.send(Command::Delete {
                items: selection,
                mode,
            });
            assert!(e.wait_idle(Some(Duration::from_secs(20))).is_idle());
            e.read(|m| {
                assert!(m.status.last_delete.is_none());
                assert_eq!(
                    m.status.notices.last().map(String::as_str),
                    Some("delete: unavailable for scans with multiple roots")
                );
            });
            assert!(first.path().join("junk/b.bin").exists());
            assert!(second.path().join("junk/b.bin").exists());
            e.read(|m| {
                for item in &items {
                    assert!(m.tree.resolve_visible(*item).is_some());
                }
            });
        }
    }
    e.shutdown(Duration::from_secs(5));
    assert!(
        !journal_kinds(journal.path())
            .iter()
            .any(|kind| kind == "intent")
    );
}

fn engine_staging<B: Backend>(backend: B, state: &Path, window: Duration) -> Engine<B> {
    Engine::start(
        backend,
        EngineOptions {
            journal_dir: Some(state.join("ops")),
            stage_window: window,
            check_invariants: true,
            workers: 2,
            ..Default::default()
        },
    )
}

fn staged_files(state: &Path) -> Vec<String> {
    let mut out = vec![];
    for s in fs::read_dir(state.join("stage"))
        .into_iter()
        .flatten()
        .flatten()
    {
        for f in fs::read_dir(s.path()).unwrap().flatten() {
            out.push(f.file_name().to_string_lossy().into_owned());
        }
    }
    out.sort();
    out
}

#[test]
fn staged_items_are_deleted_on_commit() {
    let tree = fixture();
    let root = tree.path();
    let state = tagged_tempdir("state");
    let e = engine_staging(
        PortableBackend::default(),
        state.path(),
        Duration::from_secs(3600),
    );
    scan(&e, root);
    let before = root_alloc(&e);
    let junk = e.read(|m| {
        m.tree
            .total(m.tree.resolve(node(&e, "junk")).unwrap())
            .alloc
    });
    let s = delete(&e, vec![node(&e, "junk")], Mode::Stage);
    assert_eq!((s.deleted, s.mode), (1, Some(Mode::Stage)));
    assert!(!root.join("junk").exists());
    assert_eq!(root_alloc(&e), before - junk);
    // Renamed aside, whole, in the staging directory beside the journals.
    assert_eq!(staged_files(state.path()), ["1-junk"]);
    assert_eq!(
        e.read(|m| m.status.staged),
        Count {
            items: 1,
            bytes: junk
        }
    );
    assert!(e.read(|m| m.status.undo.as_ref().is_some_and(|u| u.items == 1)));
    e.send(Command::CommitStaged);
    wait_for(&e, |m| m.status.staged.items == 0);
    assert_eq!(staged_files(state.path()), [] as [std::string::String; 0]);
    assert!(
        e.read(|m| m.status.undo.is_none()),
        "nothing left to put back"
    );
    e.shutdown(Duration::from_secs(5));
    assert_eq!(
        journal_kinds(&state.path().join("ops")),
        ["session", "intent", "done", "commit-intent", "commit-done"]
    );
}

/// Items an exited session left staged are reported when the next one starts, and left alone.
#[test]
fn a_new_session_reports_what_an_earlier_one_left() {
    let tree = fixture();
    let root = tree.path();
    let state = tagged_tempdir("state");
    let e = engine_staging(
        PortableBackend::default(),
        state.path(),
        Duration::from_secs(3600),
    );
    scan(&e, root);
    delete(
        &e,
        vec![node(&e, "junk"), node(&e, "file.bin")],
        Mode::Stage,
    );
    e.shutdown(Duration::from_secs(5));
    assert_eq!(staged_files(state.path()), ["1-junk", "2-file.bin"]);

    let e = engine_staging(
        PortableBackend::default(),
        state.path(),
        Duration::from_secs(3600),
    );
    wait_for(&e, |m| {
        m.status
            .notices
            .iter()
            .any(|n| n.starts_with("an earlier session left 2 deletes unfinished"))
    });
    e.shutdown(Duration::from_secs(5));
    assert_eq!(staged_files(state.path()), ["1-junk", "2-file.bin"]);
}

#[test]
fn staged_items_are_deleted_when_the_window_ends() {
    let tree = fixture();
    let state = tagged_tempdir("state");
    let e = engine_staging(
        PortableBackend::default(),
        state.path(),
        Duration::from_millis(200),
    );
    scan(&e, tree.path());
    delete(
        &e,
        vec![node(&e, "junk"), node(&e, "file.bin")],
        Mode::Stage,
    );
    wait_for(&e, |m| m.status.staged.items == 0);
    assert_eq!(staged_files(state.path()), [] as [std::string::String; 0]);
    e.shutdown(Duration::from_secs(5));
}

/// Undo puts staged items back, next to what took their place if something did, and the tree
/// shows them again after a rescan of their directory.
#[test]
fn undo_puts_staged_items_back() {
    let tree = fixture();
    let root = tree.path();
    let state = tagged_tempdir("state");
    let e = engine_staging(
        PortableBackend::default(),
        state.path(),
        Duration::from_secs(3600),
    );
    scan(&e, root);
    let before = root_alloc(&e);
    let s = delete(
        &e,
        vec![node(&e, "junk/sub"), node(&e, "file.bin")],
        Mode::Stage,
    );
    assert_eq!(s.deleted, 2);
    write(&root.join("file.bin"), 10);
    let plan = e.read(|m| m.status.undo.as_ref().map(|u| u.plan)).unwrap();
    e.send(Command::Undo { plan });
    wait_for(&e, |m| {
        m.status.undoing.is_none() && m.status.undo.is_none()
    });
    assert!(e.wait_idle(Some(Duration::from_secs(20))).is_idle());
    assert!(root.join("junk/sub/c.bin").exists());
    assert_eq!(
        fs::read(root.join("file.bin.restored-1")).unwrap().len(),
        3_000
    );
    assert_eq!(fs::read(root.join("file.bin")).unwrap().len(), 10);
    assert_eq!(staged_files(state.path()), [] as [std::string::String; 0]);
    assert_eq!(e.read(|m| m.status.staged), Count::default());
    // The rescans found everything: what was put back, and the new file.bin.
    assert!(root_alloc(&e) > before);
    node(&e, "junk/sub/c.bin");
    node(&e, "file.bin.restored-1");
    e.shutdown(Duration::from_secs(5));
    let kinds = journal_kinds(&state.path().join("ops"));
    assert_eq!(
        kinds.iter().filter(|k| *k == "restored").count(),
        2,
        "{kinds:?}"
    );
}

/// The scan-to-delete race inside a directory item: a file added, a file replaced
/// by another of the same name, a directory replaced, and a directory swapped for a symlink,
/// each deep inside, right after the scan (in the same second, so modification times do not
/// tell). None of the items is deleted, permanently or staged, and each is reported.
#[test]
fn changes_inside_items_stop_their_delete() {
    for mode in [Mode::Permanent, Mode::Stage] {
        changes_inside(PortableBackend::default, mode);
        #[cfg(windows)]
        changes_inside(dfdu_scan::WindowsBackend::default, mode);
    }
}

fn changes_inside<B: Backend>(backend: fn() -> B, mode: Mode) {
    let tree = tagged_tempdir("inside");
    let root = tree.path();
    for d in ["added", "replaced", "dir", "swapped"] {
        write(&root.join(d).join("x/y/f.bin"), 1_000);
        write(&root.join(d).join("g.bin"), 1_000);
    }
    let state = tagged_tempdir("state");
    let e = engine_staging(backend(), state.path(), Duration::from_secs(3600));
    scan(&e, root);
    let items: Vec<NodeRef> = ["added", "replaced", "dir", "swapped"]
        .iter()
        .map(|d| node(&e, d))
        .collect();
    write(&root.join("added/x/y/new.bin"), 10);
    write(&root.join("replaced/x/y/f.tmp"), 1_000);
    fs::rename(
        root.join("replaced/x/y/f.tmp"),
        root.join("replaced/x/y/f.bin"),
    )
    .unwrap();
    fs::rename(root.join("dir/x/y"), root.join("dir/y.old")).unwrap();
    write(&root.join("dir/x/y/f.bin"), 1_000);
    let elsewhere_dir = tagged_tempdir("elsewhere");
    write(&elsewhere_dir.path().join("f.bin"), 1_000);
    fs::remove_dir_all(root.join("swapped/x/y")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(elsewhere_dir.path(), root.join("swapped/x/y")).unwrap();
    // cmd reads a `/` anywhere in an argument as a switch: the path has `\` only.
    #[cfg(windows)]
    assert!(
        std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(root.join("swapped").join("x").join("y"))
            .arg(elsewhere_dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );
    let s = delete(&e, items, mode);
    // Windows compares names only, so the file replaced under the same name is not seen.
    let expect = if cfg!(windows) { 3 } else { 4 };
    let notices = e.read(|m| m.status.notices.join("\n"));
    assert_eq!(
        (s.deleted, s.stale),
        (4 - expect, expect),
        "{mode:?} {s:?} {notices}"
    );
    assert!(notices.contains("changed since the scan"), "{notices}");
    for d in ["added", "dir", "swapped"] {
        assert!(
            root.join(d).join("g.bin").exists(),
            "{mode:?} {d} was touched"
        );
    }
    assert!(elsewhere_dir.path().join("f.bin").exists());
    e.shutdown(Duration::from_secs(1));
}
