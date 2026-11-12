use std::sync::atomic::AtomicBool;
use std::time::Duration;

use dfdu_scan::JobOptions;
use dfdu_tree::{Ix, Kind, Meta, Tree};

use super::*;

#[test]
fn load_age_values() {
    assert_eq!(configured_max_age(None), 3600);
    for (value, seconds) in [("1s", 1), ("20m", 1200), ("1h", 3600), ("2d", 172800), ("0s", 0)] {
        assert_eq!(parse_max_age(value), Some(seconds));
        assert_eq!(configured_max_age(Some(value.as_ref())), seconds);
    }
    for value in ["", "20", "-1h", "1.5h", "h", "1fortnight", "18446744073709551615d"] {
        assert_eq!(parse_max_age(value), None);
        assert_eq!(configured_max_age(Some(value.as_ref())), 3600);
    }
}

mod files;
#[cfg(unix)]
mod private;

/// A tree of the directory `root` with `files` files in it.
pub(super) fn tree_of(root: &Path, files: usize) -> (Tree, Ix) {
    let mut t = Tree::new();
    let meta = Meta::from_metadata(&fs::metadata(root).unwrap());
    let r = t.new_root(root.as_os_str().as_encoded_bytes(), &meta, true);
    if let Some(d) = t.dir_mut(r) {
        d.state = dfdu_tree::DirState::Complete;
    }
    for i in 0..files {
        let m = Meta {
            kind: Kind::File,
            apparent: i as u64,
            alloc: 4096,
            ino: 1000 + i as u64,
            nlink: 1,
            ..meta
        };
        let f = t.add_file(r, format!("file-{i}").as_bytes(), &m, true, None);
        let own = t.own(f);
        t.propagate_add(r, &own);
    }
    (t, r)
}

pub(super) fn header(written: u64, last_full_scan: u64) -> Header {
    Header {
        tool_version: "test".into(),
        written,
        cross_mounts: false,
        network: false,
        journal: None,
        last_full_scan,
        scan_ms: 4_000,
    }
}

/// A cache in a temporary directory, a directory to scan, and its slot.
pub(super) fn setup() -> (tempfile::TempDir, Cache, Slot) {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join("scanned")).unwrap();
    let cache = Cache::at(tmp.path().join("cache/scans")).unwrap();
    let key = Key::new(&tmp.path().join("scanned"), &JobOptions::default(), false).unwrap();
    let slot = cache.slot(key);
    (tmp, cache, slot)
}

pub(super) fn save(slot: &Slot, t: &Tree, h: &Header) -> Saved {
    slot.save(h, |f| f(t), &AtomicBool::new(false)).unwrap()
}

pub(super) fn set_mtime(path: &Path, secs: u64) {
    let f = fs::File::options().write(true).open(path).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

#[test]
fn keys_follow_the_directory_and_the_options() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("d");
    fs::create_dir(&dir).unwrap();
    let k = Key::new(&dir, &JobOptions::default(), false).unwrap();
    // The same directory by another name is the same key.
    assert_eq!(
        Key::new(&dir.join("."), &JobOptions::default(), false).unwrap(),
        k
    );
    assert_eq!(k.root(), dunce::canonicalize(&dir).unwrap());
    let name = k.file_name();
    assert_eq!(name.len(), 16 + ".snap".len(), "{name}");
    assert_eq!(Path::new(&name).extension(), Some("snap".as_ref()));
    let names: std::collections::HashSet<String> = [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ]
    .iter()
    .map(|&(c, n, m)| {
        let job = JobOptions {
            cross_mounts: c,
            network: n,
            ..JobOptions::default()
        };
        Key::new(&dir, &job, m).unwrap().file_name()
    })
    .collect();
    assert_eq!(names.len(), 4);
    // A directory replaced by another of the same name is another key.
    fs::remove_dir(&dir).unwrap();
    fs::create_dir(tmp.path().join("other")).unwrap();
    fs::create_dir(&dir).unwrap();
    let again = Key::new(&dir, &JobOptions::default(), false).unwrap();
    assert_eq!(again.root(), k.root());
    assert_ne!(again.file_name(), name);
    fs::write(tmp.path().join("f"), b"").unwrap();
    assert!(Key::new(&tmp.path().join("f"), &JobOptions::default(), false).is_err());
    assert!(Key::new(&tmp.path().join("missing"), &JobOptions::default(), false).is_err());
}

#[test]
fn the_cache_turns_off_by_environment() {
    for v in ["0", "false", "OFF", "no", " 0 "] {
        assert!(is_off(std::ffi::OsStr::new(v)), "{v}");
    }
    for v in ["1", "", "yes", "on"] {
        assert!(!is_off(std::ffi::OsStr::new(v)), "{v}");
    }
}
