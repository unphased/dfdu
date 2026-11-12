//! The selection and the cursor moves: the selected row, found again after a rescan by name
//! and identity, the moves through the rows, and the way into and out of directories, which
//! remembers the selection of each directory left to restore it on the way back.

use dfdu_tree::{HostPath, NodeRef, Target};

use super::SessionState;
use crate::snapshot::EntryData;

/// The selected entry: its reference, plus name and identity to find it again after a rescan.
#[derive(Clone, Debug)]
pub struct Selection {
    /// `None` when only the name is known (going up to the parent directory).
    pub node: Option<NodeRef>,
    pub name: Vec<u8>,
    pub dev_ino: (u64, u64),
}

impl Selection {
    pub fn of(e: &EntryData) -> Self {
        Selection {
            node: Some(e.node),
            name: e.name.to_vec(),
            dev_ino: e.dev_ino,
        }
    }

    /// The entry named `name` below the directory opened, to find again by name once its
    /// rows are read.
    fn named(name: Vec<u8>) -> Self {
        Selection {
            node: None,
            name,
            dev_ino: (0, 0),
        }
    }
}

/// A move to another directory, which saved the selection of the directory left. A front end
/// keeps its own position in the list (the scroll) of each directory the same way.
#[derive(Clone, Debug)]
pub struct Moved {
    /// The directory left, whose selection was saved.
    pub left: HostPath,
    /// The directory opened, whose saved selection came back when it had one; `None` when
    /// the browser stays where it is.
    pub opened: Option<HostPath>,
}

impl SessionState {
    pub fn selected_index(&self) -> Option<usize> {
        let node = self.selected.as_ref()?.node?;
        let is_sel = |&i: &usize| self.snap.entries[i].node == node;
        // Usually the row found last time; a directory can have a million rows.
        let hint = self.list.sel_hint.get();
        if self.list.rows.get(hint).is_some_and(is_sel) {
            return Some(hint);
        }
        let found = self.list.rows.iter().position(is_sel)?;
        self.list.sel_hint.set(found);
        Some(found)
    }

    pub fn selected_entry(&self) -> Option<&EntryData> {
        self.selected_index()
            .map(|i| &self.snap.entries[self.list.rows[i]])
    }

    pub fn select(&mut self, index: usize) {
        if let Some(&i) = self.list.rows.get(index) {
            self.selected = Some(Selection::of(&self.snap.entries[i]));
            self.list.sel_hint.set(index);
        }
    }

    /// Selects row `index` as a cursor move of the user's, so re-sorts follow the selected row.
    pub fn move_to(&mut self, index: usize) {
        self.list.moved = true;
        self.select(index);
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.list.rows.is_empty() {
            return;
        }
        self.list.moved = true;
        let cur = self.selected_index().unwrap_or(0);
        let next = cur
            .saturating_add_signed(delta)
            .min(self.list.rows.len() - 1);
        self.select(next);
    }

    /// Saves the selection of the current directory, and says so.
    fn save_cursor(&mut self) -> HostPath {
        let dir = self.snap.dir_path.clone();
        self.list.cursors.insert(dir.clone(), self.selected.clone());
        dir
    }

    /// Opens the directory of entry `index` of the snapshot: in tree mode a row at any depth,
    /// and the expanded directories stay expanded.
    pub fn enter(&mut self, index: usize) -> Moved {
        let left = self.save_cursor();
        let target = crate::tree::target_of(&self.snap, index);
        let saved = self.list.cursors.get(&target.path).cloned();
        let opened = Some(target.path.clone());
        self.target = target;
        self.list.moved = saved.is_some();
        self.selected = saved.flatten();
        Moved { left, opened }
    }

    /// Goes up to the parent directory and selects the directory come from. `None` at the
    /// root of the scan.
    pub fn up(&mut self) -> Option<Moved> {
        if self.snap.at_root {
            return None;
        }
        let left = self.save_cursor();
        let path = &self.snap.dir_path;
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Some(Moved { left, opened: None });
        };
        let name = name.to_vec();
        self.target = Target {
            node: None,
            path: parent.clone(),
            ident: None,
        };
        self.list.moved = true;
        // Select the directory we came from, found again by name after the refresh.
        self.selected = Some(Selection::named(name));
        Some(Moved {
            left,
            opened: Some(parent),
        })
    }

    /// Opens the directory at `path`, found by path once its rows are read, with the selection
    /// it had when it was left. `None` when it is the directory shown or being opened.
    pub fn open_dir(&mut self, path: HostPath) -> Option<Moved> {
        if path == self.target.path {
            return None;
        }
        let left = self.save_cursor();
        let saved = self.list.cursors.get(&path).cloned();
        self.list.moved = saved.is_some();
        self.selected = saved.flatten();
        self.target = Target {
            node: None,
            path: path.clone(),
            ident: None,
        };
        Some(Moved {
            left,
            opened: Some(path),
        })
    }

    /// Goes to an entry below the scan root `root`: opens its parent directory and selects it.
    /// `None` when `path` is the root or has no parent.
    pub fn go_to(&mut self, path: &HostPath, root: &HostPath) -> Option<Moved> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return None;
        };
        if path == root {
            return None;
        }
        let left = self.save_cursor();
        self.target = Target {
            node: None,
            path: parent.clone(),
            ident: None,
        };
        self.list.moved = true;
        // An idle scan may not produce another snapshot when the directory is unchanged.
        // Resolve an entry already shown now; otherwise refresh finds it by name later.
        self.selected = Some(
            self.snap
                .entries
                .iter()
                .find(|entry| {
                    self.snap.dir_path == parent && entry.depth == 0 && &*entry.name == name
                })
                .map(Selection::of)
                .unwrap_or_else(|| Selection::named(name.to_vec())),
        );
        Some(Moved {
            left,
            opened: Some(parent),
        })
    }
}
