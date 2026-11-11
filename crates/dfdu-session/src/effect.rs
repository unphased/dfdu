//! What a key or a click leaves to be carried out after the state changed ([`Effect`]):
//! commands for the engine, reads of the model, and what only the front end can do (the
//! clipboard, a file manager). [`carry_out`] does the engine's part and returns the rest
//! ([`Output`]).

use std::path::PathBuf;

use dfdu_engine::{Command, Engine, ErrorList, errors};
use dfdu_scan::Backend;

use crate::dupes;
use crate::state::SessionState;

/// Most entries the error list shows.
pub const ERROR_LIST_MAX: usize = 10_000;

#[derive(Debug)]
pub enum Effect {
    Send(Command),
    /// Read the error list from the engine and show it.
    ShowErrors,
    /// Read the duplicate candidates and start comparing them (`dupes::begin`).
    FindDupes,
    /// Put the text on the clipboard.
    Copy(String),
    /// Show the path in the file manager, without waiting for it.
    Open {
        path: PathBuf,
        is_dir: bool,
    },
}

/// What [`carry_out`] leaves to the front end.
#[derive(Debug)]
pub enum Output {
    /// Show what could not be read.
    Errors(ErrorList),
    /// Put the text on the clipboard: the terminal's (OSC 52) or the system's.
    Copy(String),
    /// Show the path in the file manager, without waiting for it.
    Open { path: PathBuf, is_dir: bool },
}

/// Carries out `fx` in order: sends the commands to `engine`, reads the error list and starts a
/// comparison of duplicates from the candidates read under its read lock. Returns, in order,
/// what only the front end can do.
pub fn carry_out<B: Backend>(
    s: &mut SessionState,
    engine: &Engine<B>,
    fx: Vec<Effect>,
) -> Vec<Output> {
    let mut out = Vec::new();
    for fx in fx {
        match fx {
            Effect::Send(cmd) => engine.send(cmd),
            Effect::FindDupes => {
                let (root, cands) = engine.read(|m| {
                    let root = m
                        .tree
                        .roots()
                        .next()
                        .map(|r| m.tree.path_buf(r))
                        .unwrap_or_default();
                    (root, duplicate_candidates(&m.tree))
                });
                dupes::begin(s, root, cands);
            }
            Effect::ShowErrors => {
                // The error indicator describes the currently viewed root, not the whole scan.
                let list = engine.read(|m| root_errors(&m.tree, &s.snap.root_path));
                out.push(Output::Errors(list));
            }
            Effect::Copy(text) => out.push(Output::Copy(text)),
            Effect::Open { path, is_dir } => out.push(Output::Open { path, is_dir }),
        }
    }
    out
}

fn root_errors(tree: &dfdu_tree::Tree, path: &dfdu_tree::HostPath) -> ErrorList {
    let root = tree
        .roots()
        .find(|&root| tree.host_path(root) == *path)
        .or_else(|| tree.roots().next());
    root.map_or_else(ErrorList::default, |root| {
        errors(tree, root, ERROR_LIST_MAX)
    })
}

/// Collect candidates across roots before filtering by size, so matches in separate roots count.
pub(crate) fn duplicate_candidates(tree: &dfdu_tree::Tree) -> Vec<dfdu_dupes::Candidate> {
    use dfdu_tree::Kind;
    use std::collections::{HashMap, HashSet};

    let mut roots = tree.roots();
    let Some(first) = roots.next() else {
        return Vec::new();
    };
    if roots.next().is_none() {
        return dfdu_dupes::candidates(tree, first, dupes::min_size());
    }
    let mut paths = HashSet::new();
    let mut inodes = HashMap::new();
    let mut out: Vec<dfdu_dupes::Candidate> = Vec::new();
    // Match find_path's preference for the most specific root, preserving root order on ties.
    let mut roots: Vec<_> = tree.roots().collect();
    roots.sort_by_cached_key(|&root| {
        let path = tree.host_path(root);
        std::cmp::Reverse(
            path.as_bytes()
                .split(|&b| path.host().is_sep(b))
                .filter(|part| !part.is_empty())
                .count(),
        )
    });
    for root in roots {
        tree.for_each_leaf(root, &mut |ix| {
            if tree.kind(ix) != Kind::File {
                return;
            }
            let path = tree.path_buf(ix);
            if !paths.insert(path.clone()) {
                return;
            }
            let own = tree.own(ix);
            if own.apparent < dupes::min_size().max(1) {
                return;
            }
            let dev_ino = tree.dev_ino(ix);
            if dev_ino.1 != 0
                && let Some(&i) = inodes.get(&dev_ino)
            {
                let c: &mut dfdu_dupes::Candidate = &mut out[i];
                c.alloc = c.alloc.max(own.alloc);
                c.links += 1;
                return;
            }
            inodes.insert(dev_ino, out.len());
            out.push(dfdu_dupes::Candidate {
                path,
                size: own.apparent,
                alloc: own.alloc,
                dev_ino,
                mtime: tree.mtime(ix),
                links: 0,
                private: None,
            });
        });
    }
    out.retain(|c| c.alloc > 0);
    let mut sizes = HashMap::new();
    for c in &out {
        *sizes.entry(c.size).or_insert(0usize) += 1;
    }
    out.retain(|c| sizes[&c.size] > 1);
    out
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use dfdu_engine::EngineOptions;
    use dfdu_scan::PortableBackend;

    use super::*;

    #[test]
    fn overlapping_roots_use_the_more_specific_metadata() {
        use dfdu_engine::test_support::meta;
        use dfdu_tree::Kind;

        let (mut fx, root) = crate::test_support::Fixture::new();
        let size = 2 * dupes::min_size();
        let sub = fx.dir(root, "sub");
        fx.file(sub, "a", size);
        fx.file(root, "other", size);
        let overlap = fx.m.tree.new_root(b"/r/sub", &meta(Kind::Dir, 0, 51), true);
        let mut current = meta(Kind::File, size, 99);
        current.alloc = size / 2;
        current.mtime = 123;
        let file = fx.m.tree.add_file(overlap, b"a", &current, false, None);
        let candidates = duplicate_candidates(&fx.m.tree);
        let candidate = candidates
            .iter()
            .find(|c| c.path == fx.m.tree.path_buf(file))
            .unwrap();
        assert_eq!(candidate.dev_ino, (1, 99));
        assert_eq!(candidate.alloc, size / 2);
        assert_eq!(candidate.mtime, 123);
        assert_eq!(candidate.links, 0);
    }

    #[test]
    fn errors_follow_the_current_root() {
        use dfdu_engine::test_support::meta;
        use dfdu_tree::{ErrKind, Kind};

        let (mut fx, first) = crate::test_support::Fixture::new();
        let second =
            fx.m.tree
                .new_root(b"/second", &meta(Kind::Dir, 0, 10), true);
        fx.m.tree.set_error(first, ErrKind::NotFound);
        fx.m.tree.set_error(second, ErrKind::PermissionDenied);
        let list = root_errors(&fx.m.tree, &fx.m.tree.host_path(second));
        assert_eq!(list.total, 1);
        assert_eq!(list.items[0].path, fx.m.tree.host_path(second));
        assert_eq!(list.kinds, vec![(ErrKind::PermissionDenied, 1)]);
        assert_eq!(
            root_errors(&fx.m.tree, &fx.m.tree.host_path(first)).total,
            1
        );
    }

    #[test]
    fn the_engine_part_is_carried_out_and_the_rest_returned_in_order() {
        let engine = Engine::start(
            PortableBackend::default(),
            EngineOptions {
                workers: 1,
                ..Default::default()
            },
        );
        let mut s = SessionState::new(Instant::now());
        let fx = vec![
            Effect::Copy("/r/a".into()),
            Effect::ShowErrors,
            Effect::Open {
                path: "/r".into(),
                is_dir: true,
            },
        ];
        let out = carry_out(&mut s, &engine, fx);
        assert!(
            matches!(&out[..], [Output::Copy(t), Output::Errors(_), Output::Open { is_dir: true, .. }] if t == "/r/a"),
            "{out:?}"
        );
        // Without a scan there are no candidates; the comparison of none starts all the same.
        assert!(carry_out(&mut s, &engine, vec![Effect::FindDupes]).is_empty());
        assert!(s.dupes.run.is_some());
        engine.shutdown(Duration::from_millis(200));
    }
}
