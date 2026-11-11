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
                    if tree.name(p) != *expected {
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
