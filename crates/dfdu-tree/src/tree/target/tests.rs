use super::super::test_util::{add_dir, add_file, dir_meta};
use super::*;

#[test]
fn overlapping_roots_prefer_the_most_specific_and_fall_back() {
    let mut tree = Tree::new();
    let outer = tree.new_root(b"/r", &dir_meta(), true);
    let nested_path = tree.host_path(outer).join(b"nested");
    let outer_nested = add_dir(&mut tree, outer, "nested");
    let outer_shared = add_file(&mut tree, outer_nested, "shared", 1);
    let outer_only = add_file(&mut tree, outer_nested, "outer-only", 2);
    let nested = tree.new_root(nested_path.as_bytes(), &dir_meta(), true);
    let inner_shared = add_file(&mut tree, nested, "shared", 3);

    assert_eq!(find_path(&tree, &nested_path), Some(nested));
    assert_eq!(
        find_path(&tree, &tree.host_path(inner_shared)),
        Some(inner_shared)
    );
    assert_ne!(outer_shared, inner_shared);
    assert_eq!(
        find_path(&tree, &tree.host_path(outer_only)),
        Some(outer_only)
    );
    assert_eq!(find_path(&tree, &nested_path.join(b"missing")), None);
}

#[test]
fn a_target_is_found_again_by_path_only_with_its_identity() {
    let mut tree = Tree::new();
    let root = tree.new_root(b"/r", &dir_meta(), true);
    let d = add_dir(&mut tree, root, "d");
    let f = add_file(&mut tree, d, "f", 100);
    let g = add_file(&mut tree, d, "g", 200);
    let (tf, tg) = (Target::of(&tree, f), Target::of(&tree, g));
    assert_eq!(tf.resolve(&tree), Some(f));
    assert_eq!(find_path(&tree, &tree.host_path(g)), Some(g));
    // Both replaced under the same names: `f` by another file, `g` by the same one.
    tree.detach(f);
    tree.detach(g);
    let f2 = add_file(&mut tree, d, "f", 300);
    let g2 = add_file(&mut tree, d, "g", 200);
    assert_eq!(tf.resolve(&tree), None);
    assert_eq!(tg.resolve(&tree), Some(g2));
    // Without an identity the path is enough.
    let bare = Target {
        ident: None,
        ..tf.clone()
    };
    assert_eq!(bare.resolve(&tree), Some(f2));
    assert_eq!(Target::root().resolve(&tree), None);
}
