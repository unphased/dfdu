use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use dfdu_tree::Meta;

use super::*;
use crate::cache::tests::{header, save, set_mtime, setup, tree_of};
use crate::cache::{MAX_AGE, now};
use crate::ncdu::tests::dump;

/// What is saved loads back as it was: the tree, and the header.
#[test]
fn a_saved_scan_loads_back() {
    let (_tmp, _cache, slot) = setup();
    let now = now();
    assert!(!slot.cached(now));
    assert!(slot.load(now).unwrap().is_none());
    let (t, r) = tree_of(slot.key.root(), 50);
    let h = header(now, now - 60);
    let Saved::Written { entries, bytes, .. } = save(&slot, &t, &h) else {
        panic!("not written")
    };
    assert_eq!(entries, 51);
    assert_eq!(bytes, fs::metadata(&slot.path).unwrap().len());
    assert!(slot.cached(now));
    let hit = slot.load(now).unwrap().expect("a hit");
    assert!(!hit.full_rescan);
    assert_eq!(hit.loaded.header, h);
    assert_eq!(dump(&t, r), dump(&hit.loaded.tree, hit.loaded.root));
}

/// A file written more than the maximum age ago is not loaded; a tree whose
/// last full scan is older than a week is loaded to be rescanned whole.
#[test]
fn old_scans_expire() {
    let (_tmp, _cache, slot) = setup();
    let now = now();
    let (t, _) = tree_of(slot.key.root(), 3);
    save(&slot, &t, &header(now, now - FULL_RESCAN_AFTER - 10));
    let hit = slot.load(now).unwrap().expect("a hit");
    assert!(hit.full_rescan);
    // Brought up to date from the journal since: still the old full scan.
    save(&slot, &t, &header(now, now - FULL_RESCAN_AFTER - 10));
    assert!(slot.load(now).unwrap().expect("a hit").full_rescan);
    save(&slot, &t, &header(now, now - FULL_RESCAN_AFTER + 10));
    assert!(!slot.load(now).unwrap().expect("a hit").full_rescan);
    // Formats before 4 do not say; when it was written stands in.
    save(&slot, &t, &header(now - 60, 0));
    assert!(!slot.load(now).unwrap().expect("a hit").full_rescan);

    save(&slot, &t, &header(now - MAX_AGE - 10, now - MAX_AGE - 10));
    assert!(slot.cached(now), "the file itself is new");
    assert!(slot.load(now).unwrap().is_none());
    assert!(slot.path.exists(), "an expired scan is left for the sweep");
    save(&slot, &t, &header(now, now));
    set_mtime(&slot.path, now - MAX_AGE - 10);
    assert!(!slot.cached(now));
}

#[test]
fn loading_obeys_max_age_without_shortening_retention() {
    let (_tmp, _cache, mut slot) = setup();
    let now = now();
    let (t, _) = tree_of(slot.key.root(), 3);
    for max_age in [3600, 2 * 86_400] {
        slot.cache.max_age = max_age;
        for age in [max_age - 1, max_age, max_age + 1] {
            save(&slot, &t, &header(now - age, now - age));
            set_mtime(&slot.path, now - age);
            assert_eq!(slot.cached(now), age <= max_age);
            assert_eq!(slot.load(now).unwrap().is_some(), age <= max_age);
            assert!(
                slot.path.exists(),
                "load limit must not remove retained scans"
            );
            assert_eq!(slot.sweep(now).files, 0);
        }
        // A recent mtime cannot make an old snapshot header eligible.
        save(&slot, &t, &header(now - max_age - 1, now));
        assert!(slot.cached(now));
        assert!(slot.load(now).unwrap().is_none());
    }
}

/// A damaged or cut-short file is an error and is removed; one of another directory is not
/// used, nor is a symbolic link.
#[test]
fn bad_files_are_not_loaded() {
    let (tmp, _cache, slot) = setup();
    let now = now();
    let (t, _) = tree_of(slot.key.root(), 20);
    save(&slot, &t, &header(now, now));
    let good = fs::read(&slot.path).unwrap();
    fs::write(&slot.path, &good[..good.len() / 2]).unwrap();
    assert!(slot.load(now).is_err());
    assert!(!slot.path.exists());
    fs::write(&slot.path, b"not a snapshot").unwrap();
    assert!(slot.load(now).is_err());
    assert!(!slot.path.exists());

    // Another directory's scan under this name.
    fs::create_dir(tmp.path().join("elsewhere")).unwrap();
    let (other, _) = tree_of(
        &dunce::canonicalize(tmp.path().join("elsewhere")).unwrap(),
        2,
    );
    let mut bytes = vec![];
    snapshot::write(
        &other,
        other.roots().next().unwrap(),
        &mut bytes,
        &header(now, now),
    )
    .unwrap();
    fs::write(&slot.path, &bytes).unwrap();
    assert!(slot.load(now).unwrap().is_none());
    // Other options.
    let mut cross = header(now, now);
    cross.cross_mounts = true;
    let mut bytes = vec![];
    snapshot::write(&t, t.roots().next().unwrap(), &mut bytes, &cross).unwrap();
    fs::write(&slot.path, &bytes).unwrap();
    assert!(slot.load(now).unwrap().is_none());

    #[cfg(unix)]
    {
        let real = tmp.path().join("real.snap");
        fs::write(&real, &good).unwrap();
        fs::remove_file(&slot.path).unwrap();
        std::os::unix::fs::symlink(&real, &slot.path).unwrap();
        assert!(!slot.cached(now));
        assert!(slot.load(now).unwrap().is_none());
        assert!(real.exists());
    }
}

/// A tree that changes between the parts of a save is not written, nor is anything once
/// cancelled; the file cached before stays, and no temporary file is left.
#[test]
fn a_changing_tree_or_a_cancel_writes_nothing() {
    let (_tmp, cache, slot) = setup();
    let now = now();
    let (t, _) = tree_of(slot.key.root(), STEP_ENTRIES + 10);
    let h = header(now, now);
    assert!(matches!(save(&slot, &t, &h), Saved::Written { .. }));
    let before = fs::read(&slot.path).unwrap();
    let tree = Mutex::new(t);
    let mut calls = 0;
    let changed = slot
        .save(
            &header(now + 1, now + 1),
            |f| {
                let mut t = tree.lock().unwrap();
                if calls == 1 {
                    t.bump_version();
                }
                calls += 1;
                f(&t);
            },
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(changed, Saved::Changed);
    assert_eq!(calls, 2);
    let t = tree.into_inner().unwrap();
    let cancelled = slot.save(&h, |f| f(&t), &AtomicBool::new(true)).unwrap();
    assert_eq!(cancelled, Saved::Cancelled);
    // A tree of another directory.
    let (other, _) = tree_of(cache.dir(), 1);
    assert_eq!(save(&slot, &other, &h), Saved::Changed);
    assert_eq!(fs::read(&slot.path).unwrap(), before);
    assert_eq!(fs::read_dir(cache.dir()).unwrap().count(), 1);
}

/// Removing the cached scan removes its file only, and a slot with none is fine.
#[test]
fn remove_takes_the_file_away() {
    let (_tmp, cache, slot) = setup();
    slot.remove();
    let (t, _) = tree_of(slot.key.root(), 2);
    save(&slot, &t, &header(now(), now()));
    fs::write(cache.dir().join("notes.txt"), b"mine").unwrap();
    slot.remove();
    assert!(!slot.path.exists());
    assert!(!slot.cached(now()));
    assert!(cache.dir().join("notes.txt").exists());
}

/// A slot's sweep is its cache's, keeping the slot's own file even when it is over the cap.
#[test]
fn the_sweep_of_a_slot_keeps_its_file() {
    let (_tmp, cache, slot) = setup();
    let now = now();
    let (t, _) = tree_of(slot.key.root(), 2);
    save(&slot, &t, &header(now, now));
    let expired = cache.dir().join("000000000000000a.snap");
    fs::write(&expired, b"old").unwrap();
    set_mtime(&expired, now - MAX_AGE - 1);
    let removed = slot.sweep(now);
    assert_eq!(removed, Removed { files: 1, bytes: 3 });
    assert!(slot.path.exists() && !expired.exists());
    set_mtime(&slot.path, now - MAX_AGE - 1);
    assert_eq!(
        slot.sweep(now).files,
        0,
        "the slot's file is kept even when expired"
    );
    assert!(slot.path.exists());
}

/// Nothing to load is `None`, not an error: no file, or no cache directory at all.
#[test]
fn no_file_is_no_hit() {
    let (_tmp, cache, slot) = setup();
    assert!(!cache.dir().exists());
    assert!(!slot.cached(now()));
    assert!(slot.load(now()).unwrap().is_none());
}

/// A save creates the cache directory when there is none.
#[test]
fn a_save_creates_the_directory() {
    let (_tmp, cache, slot) = setup();
    let (t, _) = tree_of(slot.key.root(), 1);
    assert!(matches!(
        save(&slot, &t, &header(now(), now())),
        Saved::Written { entries: 2, .. }
    ));
    assert!(cache.dir().is_dir());
}

#[test]
fn a_step_is_done_or_more() {
    assert!(matches!(Step::of(true), Step::Done));
    assert!(matches!(Step::of(false), Step::More));
}

/// Measures a save and a load of about a million entries: how long the tree is held still at
/// once, and how long each takes. `cargo test --release -p dfdu-store -- --ignored --nocapture
/// measure_a_million`.
#[test]
#[ignore = "a measurement, not a check"]
fn measure_a_million() {
    let (_tmp, _cache, slot) = setup();
    let root_path = slot.key.root();
    let meta = Meta::from_metadata(&fs::metadata(root_path).unwrap());
    let t = dfdu_engine::test_support::wide_tree(root_path, &meta, 10_000, 100);
    let lock = std::sync::RwLock::new(t);
    let mut holds = vec![];
    let t0 = Instant::now();
    let saved = slot
        .save(
            &header(now(), now()),
            |f| {
                let g = lock.read().unwrap();
                let h = Instant::now();
                f(&g);
                holds.push(h.elapsed());
            },
            &AtomicBool::new(false),
        )
        .unwrap();
    let save = t0.elapsed();
    holds.sort();
    let p = |q: usize| holds[(holds.len() - 1) * q / 100];
    let t0 = Instant::now();
    let hit = slot.load(now()).unwrap().unwrap();
    let load = t0.elapsed();
    eprintln!(
        "{saved:?}\nsave {save:?} in {} steps: hold p50 {:?} p99 {:?} max {:?}\nload {load:?}, {} entries",
        holds.len(),
        p(50),
        p(99),
        p(100),
        hit.loaded.entries
    );
}
