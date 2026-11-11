//! Matching regular files by apparent size.

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
        assert_eq!(index.candidates(&f.m.tree, 10, b"file", &[b"x", b"a"]), vec![same_a, same_b, other]);
        assert!(index.candidates(&f.m.tree, 99, b"file", &[]).is_empty());
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
