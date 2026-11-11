//! Holding on to a node across changes to the tree: its reference, and its path with an
//! identity to find it again when the reference no longer resolves (after a rescan swap).

use super::{Ix, NodeRef, Tree};
use crate::{HostPath, Kind};

/// Identity of a node, compared when a reference is re-resolved by path after a rescan swap.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Ident {
    pub dev_ino: (u64, u64),
    pub kind: Kind,
}

/// A node referred to across tree changes: the reference, plus the path and identity to find
/// it again.
#[derive(Clone, Debug)]
pub struct Target {
    pub node: Option<NodeRef>,
    pub path: HostPath,
    pub ident: Option<Ident>,
}

impl Target {
    #[must_use]
    pub fn root() -> Self {
        Target {
            node: None,
            path: HostPath::default(),
            ident: None,
        }
    }

    #[must_use]
    pub fn of(tree: &Tree, ix: Ix) -> Self {
        Target {
            node: Some(tree.node_ref(ix)),
            path: tree.host_path(ix),
            ident: Some(ident(tree, ix)),
        }
    }

    /// By reference if it still resolves, otherwise by path with an identity check. `None` if
    /// neither works.
    #[must_use]
    pub fn resolve(&self, tree: &Tree) -> Option<Ix> {
        if let Some(ix) = self.node.and_then(|r| tree.resolve_visible(r)) {
            return Some(ix);
        }
        let ix = find_path(tree, &self.path)?;
        self.matches(tree, ix).then_some(ix)
    }

    /// The identity check for a node found by path: `ix` has the target's kind and device and
    /// inode numbers, so a new file or directory under the old name is not taken for it. A
    /// target without an identity, or without device and inode numbers, passes.
    #[must_use]
    pub fn matches(&self, tree: &Tree, ix: Ix) -> bool {
        self.ident
            .is_none_or(|id| id.dev_ino == (0, 0) || ident(tree, ix) == id)
    }
}

fn ident(tree: &Tree, ix: Ix) -> Ident {
    Ident {
        dev_ino: tree.dev_ino(ix),
        kind: tree.kind(ix),
    }
}

/// Finds a visible node by path, trying matching roots from most specific to least specific.
#[must_use]
pub fn find_path(tree: &Tree, path: &HostPath) -> Option<Ix> {
    let mut roots: Vec<_> = tree
        .roots()
        .filter_map(|root| path.rel(&tree.host_path(root)).map(|rest| (root, rest)))
        .collect();
    roots.sort_by_key(|(_, rest)| rest.len());
    roots.into_iter().find_map(|(root, rest)| {
        rest.into_iter().try_fold(root, |cur, name| {
            tree.children(cur).find(|&c| tree.name(c) == name)
        })
    })
}

#[cfg(test)]
mod tests;
