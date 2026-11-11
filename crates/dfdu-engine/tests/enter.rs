//! A mount the scan listed but did not enter is scanned in place on request (`Command::Enter`):
//! its contents are added to the tree and its parents' totals, and the mounts below it are
//! still left out, as the job's options say.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dfdu_engine::{Command, Engine, EngineOptions, Model};
use dfdu_scan::{Backend, Block, DirStream, JobOptions, PortableBackend, Raw, RawBlock};
use dfdu_sys::FsClass;
use dfdu_tree::{DirState, Ix, Meta, NodeFlags};

/// The portable backend, with `disk` (and `disk/inner`, another one again) and `nas` (a
/// network filesystem) below the root reported as mounts of their own devices.
struct Mounts {
    inner: PortableBackend,
    root: PathBuf,
}

/// Device of `path`: what the mount it is in is numbered.
fn dev_of(root: &Path, path: &Path) -> u64 {
    match path.strip_prefix(root) {
        Ok(rel) if rel.starts_with("disk/inner") => 4,
        Ok(rel) if rel.starts_with("disk") => 2,
        Ok(rel) if rel.starts_with("nas") => 3,
        _ => 1,
    }
}

fn fake(root: &Path, path: &Path, m: &mut Meta) {
    m.dev = dev_of(root, path);
    m.mount = 0;
}

struct Stream {
    inner: <PortableBackend as Backend>::Stream,
    dir: PathBuf,
    root: PathBuf,
}

struct Stat {
    inner: Raw<PathBuf>,
    dir: PathBuf,
    root: PathBuf,
}

impl RawBlock<PathBuf> for Stat {
    fn len(&self) -> usize {
        self.inner.len()
    }
    fn stat(&mut self, out: &mut Block<PathBuf>) -> io::Result<()> {
        let from = out.entries.len();
        self.inner.stat(out)?;
        for i in from..out.entries.len() {
            let name = out.name(out.entries[i].name).to_vec();
            let path = self.dir.join(dfdu_tree::name_os(&name));
            if let Ok(m) = &mut out.entries[i].meta {
                fake(&self.root, &path, m);
            }
        }
        Ok(())
    }
}

impl DirStream for Stream {
    type Loc = PathBuf;
    fn next_raw(&mut self, n: usize, bytes: usize) -> io::Result<(Raw<PathBuf>, bool)> {
        let (raw, more) = self.inner.next_raw(n, bytes)?;
        let raw: Raw<PathBuf> = Box::new(Stat {
            inner: raw,
            dir: self.dir.clone(),
            root: self.root.clone(),
        });
        Ok((raw, more))
    }
}

impl Backend for Mounts {
    type Loc = PathBuf;
    type Stream = Stream;

    fn loc_from_path(&self, path: &Path) -> PathBuf {
        self.inner.loc_from_path(path)
    }
    fn stat_root(&self, loc: &PathBuf) -> io::Result<Meta> {
        let mut m = self.inner.stat_root(loc)?;
        fake(&self.root, loc, &mut m);
        Ok(m)
    }
    fn open(&self, loc: &PathBuf, storage: dfdu_sys::Storage) -> io::Result<Stream> {
        Ok(Stream {
            inner: self.inner.open(loc, storage)?,
            dir: loc.clone(),
            root: self.root.clone(),
        })
    }
    fn duplicate_mounts(&self, _scope: &Path) -> Option<HashSet<u64>> {
        Some(HashSet::new())
    }
    fn mount_class(&self, _mount: u64, dev: u64) -> Option<FsClass> {
        Some(if dev == 3 {
            FsClass::Network
        } else {
            FsClass::Local
        })
    }
}

fn find(m: &Model, rel: &str) -> Ix {
    let t = &m.tree;
    let mut ix = t.roots().next().expect("root");
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        ix = t
            .children(ix)
            .find(|&c| t.name(c) == part.as_bytes())
            .unwrap_or_else(|| panic!("{rel}: no {part}"));
    }
    ix
}

fn state(m: &Model, rel: &str) -> DirState {
    m.tree.dir(find(m, rel)).expect("dir").state
}

/// `home/a`, `disk/b`, `disk/inner/c` and `nas/d` scanned without entering other mounts.
fn scanned() -> (tempfile::TempDir, Engine<Mounts>) {
    let tmp = common::tempdir();
    let root = tmp.path().to_path_buf();
    for d in ["home", "disk/inner", "nas"] {
        fs::create_dir_all(root.join(d)).unwrap();
    }
    fs::write(root.join("home/a"), vec![1u8; 1000]).unwrap();
    fs::write(root.join("disk/b"), vec![1u8; 20_000]).unwrap();
    fs::write(root.join("disk/inner/c"), vec![1u8; 300_000]).unwrap();
    fs::write(root.join("nas/d"), vec![1u8; 4000]).unwrap();
    let e = Engine::start(
        Mounts {
            inner: PortableBackend::default(),
            root: root.clone(),
        },
        EngineOptions {
            check_invariants: true,
            workers: 2,
            ..Default::default()
        },
    );
    e.send(Command::Scan {
        path: root.into(),
        opts: JobOptions::default(),
    });
    assert!(e.wait_idle(Some(Duration::from_secs(60))).is_idle());
    (tmp, e)
}

#[test]
fn entering_a_mount_scans_it_in_place() {
    let (_tmp, e) = scanned();
    let idle = || assert!(e.wait_idle(Some(Duration::from_secs(60))).is_idle());
    let apparent = |m: &Model, rel: &str| m.tree.total(find(m, rel)).apparent;
    let before = e.read(|m| {
        assert_eq!(state(m, "disk"), DirState::Skipped);
        assert!(m.tree.flags(find(m, "disk")).has(NodeFlags::MOUNT_ROOT));
        assert!(m.tree.flags(find(m, "nas")).has(NodeFlags::NETWORK));
        assert_eq!(state(m, "nas"), DirState::Skipped);
        (apparent(m, ""), apparent(m, "disk"))
    });

    let node = |rel: &str| e.read(|m| m.tree.node_ref(find(m, rel)));
    e.send(Command::Enter { node: node("disk") });
    idle();
    e.read(|m| {
        assert_eq!(state(m, "disk"), DirState::Complete);
        assert!(m.tree.flags(find(m, "disk")).has(NodeFlags::MOUNT_ROOT));
        // The mount below it is another filesystem again: left out.
        assert_eq!(state(m, "disk/inner"), DirState::Skipped);
        assert!(
            m.tree
                .flags(find(m, "disk/inner"))
                .has(NodeFlags::MOUNT_ROOT)
        );
        let added = apparent(m, "disk");
        assert!((20_000..300_000).contains(&added), "{added}");
        // The parents count it now.
        assert_eq!(apparent(m, ""), before.0 - before.1 + added);
    });

    // A network filesystem is entered the same way, when asked.
    e.send(Command::Enter { node: node("nas") });
    idle();
    e.read(|m| {
        assert_eq!(state(m, "nas"), DirState::Complete);
        assert!(!m.tree.flags(find(m, "nas")).has(NodeFlags::NETWORK));
        assert!(apparent(m, "nas") >= 4000);
    });

    // Anything else is refused.
    e.send(Command::Enter { node: node("home") });
    idle();
    e.read(|m| {
        let last = m.status.notices.last().map_or("", String::as_str);
        assert!(last.contains("not a mount"), "{last}");
        assert_eq!(state(m, "home"), DirState::Complete);
    });
    e.shutdown(Duration::from_secs(1));
}

/// A rescan of a parent keeps entering the mounts entered this session: the original options
/// alone would leave them out again.
#[test]
fn rescans_of_a_parent_enter_the_mounts_entered_again() {
    let (tmp, e) = scanned();
    let idle = || assert!(e.wait_idle(Some(Duration::from_secs(60))).is_idle());
    let node = |rel: &str| e.read(|m| m.tree.node_ref(find(m, rel)));
    e.send(Command::Enter { node: node("disk") });
    idle();
    e.send(Command::Enter { node: node("nas") });
    idle();
    let entered = |m: &Model| {
        assert_eq!(state(m, "disk"), DirState::Complete);
        assert_eq!(state(m, "nas"), DirState::Complete);
        assert_eq!(state(m, "disk/inner"), DirState::Skipped);
        (
            m.tree.total(find(m, "")).apparent,
            m.tree.total(find(m, "disk")).items,
        )
    };
    let before = e.read(entered);
    fs::write(tmp.path().join("disk/b2"), vec![1u8; 500]).unwrap();
    // The root, the parent of both.
    e.send(Command::Rescan { node: node("") });
    idle();
    let after = e.read(entered);
    assert_eq!(
        after.1,
        before.1 + 1,
        "the new file in the entered mount is counted"
    );
    assert!(after.0 >= before.0 + 500, "{after:?} {before:?}");
    // A rescan of the mount itself, too.
    e.send(Command::Rescan { node: node("disk") });
    idle();
    e.read(entered);
    e.shutdown(Duration::from_secs(1));
}

#[test]
fn entered_mounts_do_not_leak_between_overlapping_roots() {
    let (tmp, e) = scanned();
    let idle = || assert!(e.wait_idle(Some(Duration::from_secs(60))).is_idle());
    let outer_disk = e.read(|m| m.tree.node_ref(find(m, "disk")));
    e.send(Command::Enter { node: outer_disk });
    idle();
    e.send(Command::Scan {
        path: tmp.path().join("disk").into(),
        opts: JobOptions::default(),
    });
    idle();
    let (outer, inner, mount) = e.read(|m| {
        let roots: Vec<_> = m.tree.roots().collect();
        assert_eq!(roots.len(), 2);
        let inner = *roots
            .iter()
            .find(|&&r| m.tree.path_buf(r) == tmp.path().join("disk"))
            .unwrap();
        let outer = *roots.iter().find(|&&r| r != inner).unwrap();
        let mount = m
            .tree
            .children(inner)
            .find(|&c| m.tree.name(c) == b"inner")
            .unwrap();
        (
            m.tree.node_ref(outer),
            m.tree.node_ref(inner),
            m.tree.node_ref(mount),
        )
    });
    e.send(Command::Enter { node: mount });
    idle();
    e.send(Command::Rescan { node: outer });
    idle();
    e.read(|m| {
        let inner = m.tree.resolve_visible(inner).unwrap();
        let entered = m
            .tree
            .children(inner)
            .find(|&c| m.tree.name(c) == b"inner")
            .unwrap();
        assert_eq!(m.tree.dir(entered).unwrap().state, DirState::Complete);
        let disk = find(m, "disk");
        let left_out = m
            .tree
            .children(disk)
            .find(|&c| m.tree.name(c) == b"inner")
            .unwrap();
        assert_eq!(m.tree.dir(left_out).unwrap().state, DirState::Skipped);
    });
    e.shutdown(Duration::from_secs(1));
}
