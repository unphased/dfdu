//! Matching regular files by apparent size.

use std::collections::HashMap;

use dfdu_tree::{Ix, Kind, Tree};

/// A lazy index for one tree. Clear it before using it with a different tree, even if the
/// trees have the same version. Indices returned by this type are valid only under the tree
/// lock at which they were obtained.
#[derive(Default)]
pub struct MatchIndex {
    version: Option<u64>,
    sizes: HashMap<u64, Vec<Ix>>,
}

impl MatchIndex {
    /// Discards the index, for example when a scan is replaced by another tree.
    pub fn clear(&mut self) {
        self.version = None;
        self.sizes.clear();
    }

    /// Files of `size`, ordered by exact `name`, then the length of the matching prefix of
    /// `parents` (parent first, root last), then path bytes. The returned order can be cycled
    /// by the caller. All sizes are apparent bytes, including uncharged hard links.
    pub fn candidates(
        &mut self,
        tree: &Tree,
        size: u64,
        name: &[u8],
        parents: &[&[u8]],
    ) -> Vec<Ix> {
        if self.version != Some(tree.version()) {
            self.sizes.clear();
            for root in tree.roots() {
                tree.for_each_leaf(root, |ix| {
                    if tree.kind(ix) == Kind::File {
                        self.sizes.entry(tree.raw_size(ix).0).or_default().push(ix);
                    }
                });
            }
            self.version = Some(tree.version());
        }
        let mut ranked: Vec<_> = self
            .sizes
            .get(&size)
            .into_iter()
            .flatten()
            .map(|&ix| {
                let mut parent = tree.parent(ix);
                let mut matching = 0;
                for expected in parents {
                    let Some(p) = parent else { break };
                    let matches = if tree.parent(p).is_none() {
                        tree.host_path(p).file_name() == Some(*expected)
                    } else {
                        tree.name(p) == *expected
                    };
                    if !matches {
                        break;
                    }
                    matching += 1;
                    parent = tree.parent(p);
                }
                (
                    std::cmp::Reverse(tree.name(ix) == name),
                    std::cmp::Reverse(matching),
                    tree.path(ix),
                    ix,
                )
            })
            .collect();
        ranked.sort_unstable();
        ranked.into_iter().map(|(_, _, _, ix)| ix).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;

    #[test]
    fn ranks_name_then_nearest_parents_then_path() {
        let (mut f, root) = Fixture::new();
        let a = f.dir(root, "a");
        let b = f.dir(root, "b");
        let ax = f.dir(a, "x");
        let bx = f.dir(b, "x");
        let same_b = f.file(bx, "file", 10);
        let same_a = f.file(ax, "file", 10);
        let other = f.file(ax, "other", 10);
        f.file(ax, "file2", 11);
        let mut index = MatchIndex::default();
        assert_eq!(
            index.candidates(&f.m.tree, 10, b"file", &[b"x", b"a"]),
            vec![same_a, same_b, other]
        );
        assert!(index.candidates(&f.m.tree, 99, b"file", &[]).is_empty());
    }

    #[test]
    fn ranks_scan_root_basename_as_parent() {
        use dfdu_engine::test_support::meta;

        let (mut f, a) = Fixture::new();
        let z =
            f.m.tree
                .new_root(b"/z/videos", &meta(Kind::Dir, 0, 100), true);
        let other = f.file(a, "file", 10);
        let matching = f.file(z, "file", 10);
        let mut index = MatchIndex::default();
        assert_eq!(
            index.candidates(&f.m.tree, 10, b"file", &[b"videos"]),
            vec![matching, other]
        );
    }

    #[test]
    fn indexes_only_regular_files_by_raw_apparent_size() {
        use dfdu_engine::test_support::meta;

        let (mut f, root) = Fixture::new();
        let mut stat = meta(Kind::File, 7, 42);
        stat.alloc = 4096;
        stat.nlink = 2;
        let file = f.m.tree.add_file(root, b"file", &stat, false, None);
        let link = meta(Kind::Symlink, 7, 43);
        f.m.tree.add_file(root, b"link", &link, true, None);
        let mut index = MatchIndex::default();
        assert_eq!(index.candidates(&f.m.tree, 7, b"", &[]), vec![file]);
        assert!(index.candidates(&f.m.tree, 4096, b"", &[]).is_empty());
        index.clear();
        assert_eq!(index.candidates(&f.m.tree, 7, b"", &[]), vec![file]);
    }

    #[test]
    fn ranked_files_union_name_and_size_with_parent_and_mtime_order() {
        use dfdu_engine::test_support::meta;

        let (mut f, root) = Fixture::new();
        let a = f.dir(root, "a");
        let b = f.dir(root, "b");
        let exact =
            f.m.tree
                .add_file(b, b"file", &meta(Kind::File, 10, i64::MIN), true, None);
        let near =
            f.m.tree
                .add_file(a, b"file", &meta(Kind::File, 20, 99), true, None);
        let far =
            f.m.tree
                .add_file(a, b"other", &meta(Kind::File, 10, i64::MAX), true, None);
        let tied =
            f.m.tree
                .add_file(a, b"z", &meta(Kind::File, 10, 99), true, None);
        f.file(a, "excluded", 30);
        let mut index = MatchIndex::default();
        assert_eq!(
            index.ranked_candidates(&f.m.tree, 10, b"file", &[b"a"], 100, false),
            vec![exact, near, tied, far]
        );
        assert_eq!(
            index.ranked_candidates(&f.m.tree, 99, b"file", &[], i64::MIN, false),
            vec![exact, near]
        );
    }

    #[test]
    fn ranked_directories_include_roots_and_match_parent_prefix() {
        use dfdu_engine::test_support::meta;

        let (mut f, root) = Fixture::new();
        let a = f.dir(root, "a");
        let b = f.dir(root, "b");
        let other = f.dir(b, "photos");
        let best = f.dir(a, "photos");
        let scan_root =
            f.m.tree
                .new_root(b"/backup/photos", &meta(Kind::Dir, 0, 100), true);
        f.file(a, "photos", 0);
        let mut index = MatchIndex::default();
        let found = index.ranked_candidates(&f.m.tree, 0, b"photos", &[b"a"], 100, true);
        assert_eq!(found[0], best);
        assert_eq!(found.len(), 3);
        assert!(found.contains(&other));
        assert!(found.contains(&scan_root));
    }

    #[test]
    fn ranked_indexes_rebuild_and_clear() {
        let (mut f, root) = Fixture::new();
        let first = f.file(root, "file", 7);
        let old_dir = f.dir(root, "folder");
        let mut index = MatchIndex::default();
        assert_eq!(
            index.ranked_candidates(&f.m.tree, 99, b"file", &[], 0, false),
            vec![first]
        );
        assert_eq!(
            index.ranked_candidates(&f.m.tree, 0, b"folder", &[], 0, true),
            vec![old_dir]
        );
        f.m.tree.detach(first);
        f.m.tree.detach(old_dir);
        let second = f.file(root, "file", 8);
        let new_dir = f.dir(root, "folder");
        assert_eq!(
            index.ranked_candidates(&f.m.tree, 99, b"file", &[], 0, false),
            vec![second]
        );
        assert_eq!(
            index.ranked_candidates(&f.m.tree, 0, b"folder", &[], 0, true),
            vec![new_dir]
        );
        index.clear();
        assert_eq!(
            index.ranked_candidates(&f.m.tree, 99, b"file", &[], 0, false),
            vec![second]
        );
    }

    #[test]
    fn rebuilds_when_tree_changes() {
        let (mut f, root) = Fixture::new();
        let first = f.file(root, "first", 7);
        let mut index = MatchIndex::default();
        assert_eq!(index.candidates(&f.m.tree, 7, b"", &[]), vec![first]);
        f.m.tree.detach(first);
        let second = f.file(root, "second", 7);
        assert_eq!(index.candidates(&f.m.tree, 7, b"", &[]), vec![second]);
    }
}
