//! The action registry: every user action once, with its keys, label and group. Key handling,
//! the help and the command palette all read [`ACTIONS`], so a binding shown anywhere is the
//! binding that works. The keys are data here; a front end reads them as its input needs.

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Quit,
    Up,
    Down,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    First,
    Last,
    Open,
    Enter,
    Parent,
    GoTo,
    Peer,
    PeerSwap,
    SortSize,
    SortName,
    SortItems,
    SortMtime,
    Resort,
    ToggleApparent,
    ToggleReferenced,
    ToggleHidden,
    Columns,
    ToggleTree,
    ExpandChain,
    CycleFold,
    Rescan,
    EnterMount,
    SkipStalled,
    Trash,
    DeletePermanent,
    Undo,
    Help,
    Errors,
    Close,
    ToggleMark,
    ClearMarks,
    RangeMark,
    MarkPattern,
    InvertMarks,
    CopyPath,
    OpenPath,
    Filter,
    Types,
    Details,
    FocusDetails,
    Palette,
    Debug,
    ViewFiles,
    ViewMap,
    ViewCleanup,
    ViewDupes,
    ViewLargest,
    PrevView,
    NextView,
    TickSuggested,
    DupesKeep,
    CompareAgain,
    CleanupFilter,
    CleanupCommand,
    TreemapDeeper,
    TreemapShallower,
    TreemapColor,
}

impl Action {
    /// Deletes, or puts back what was deleted: left out of a read-only browser.
    pub fn deletes(self) -> bool {
        matches!(self, Action::Trash | Action::DeletePermanent | Action::Undo)
    }

    /// Moves the cursor of the list: a range mark marks the rows it passes.
    pub fn moves(self) -> bool {
        matches!(
            self,
            Action::Up
                | Action::Down
                | Action::PageUp
                | Action::PageDown
                | Action::HalfPageUp
                | Action::HalfPageDown
                | Action::First
                | Action::Last
        )
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Group {
    Navigate,
    Sort,
    Display,
    Act,
    Panels,
    Global,
}

#[derive(Copy, Clone, Debug)]
pub struct ActionInfo {
    pub action: Action,
    /// The keys that run it, separated by spaces: a single character, a name (`↑`, `PgUp`,
    /// `Enter`, `Space`, `F1`) or `Ctrl-` and a letter. Empty for actions without a key.
    pub keys: &'static str,
    pub label: &'static str,
    pub group: Group,
}

const fn row(action: Action, keys: &'static str, label: &'static str, group: Group) -> ActionInfo {
    ActionInfo {
        action,
        keys,
        label,
        group,
    }
}

pub const ACTIONS: &[ActionInfo] = &[
    row(Action::Up, "↑ k", "move up", Group::Navigate),
    row(Action::Down, "↓ j", "move down", Group::Navigate),
    row(Action::PageUp, "PgUp", "page up", Group::Navigate),
    row(Action::PageDown, "PgDn", "page down", Group::Navigate),
    row(
        Action::HalfPageUp,
        "Ctrl-U",
        "half a page up",
        Group::Navigate,
    ),
    row(
        Action::HalfPageDown,
        "Ctrl-D",
        "half a page down",
        Group::Navigate,
    ),
    row(Action::First, "g Home", "first entry", Group::Navigate),
    row(Action::Last, "G End", "last entry", Group::Navigate),
    row(
        Action::Open,
        "→ l",
        "open directory (tree mode: expand it)",
        Group::Navigate,
    ),
    row(
        Action::Enter,
        "Enter",
        "open directory, even in tree mode",
        Group::Navigate,
    ),
    row(
        Action::Parent,
        "← h Backspace <",
        "parent directory (tree mode: collapse)",
        Group::Navigate,
    ),
    row(Action::GoTo, "Ctrl-G", "go to folder…", Group::Navigate),
    row(
        Action::Peer,
        "B",
        "bind / unbind peer (TARGET)",
        Group::Global,
    ),
    row(
        Action::PeerSwap,
        "W",
        "swap peer roles (master only)",
        Group::Global,
    ),
    row(
        Action::SortSize,
        "s",
        "sort by size (again: reverse)",
        Group::Sort,
    ),
    row(
        Action::SortName,
        "n",
        "sort by name (again: reverse)",
        Group::Sort,
    ),
    row(
        Action::SortItems,
        "C",
        "sort by items (again: reverse)",
        Group::Sort,
    ),
    row(
        Action::SortMtime,
        "M",
        "sort by modification time (again: reverse)",
        Group::Sort,
    ),
    row(Action::Resort, "S", "re-sort now", Group::Sort),
    row(
        Action::ViewFiles,
        "1",
        "Files view: the entries of this directory",
        Group::Display,
    ),
    row(
        Action::ViewMap,
        "2",
        "Map view: this directory as a treemap",
        Group::Display,
    ),
    row(
        Action::ViewCleanup,
        "3",
        "Cleanup view: what the rules found in the whole scan",
        Group::Display,
    ),
    row(
        Action::ViewDupes,
        "4",
        "Duplicates view: identical files in the whole scan",
        Group::Display,
    ),
    row(
        Action::ViewLargest,
        "5",
        "Largest view: the largest files below this directory",
        Group::Display,
    ),
    row(Action::PrevView, "[", "previous view", Group::Display),
    row(Action::NextView, "]", "next view", Group::Display),
    row(
        Action::DupesKeep,
        "K",
        "duplicates: change which copy is kept",
        Group::Display,
    ),
    row(
        Action::CleanupFilter,
        "f",
        "cleanup: only the safer levels, one fewer each time",
        Group::Display,
    ),
    row(
        Action::TreemapDeeper,
        "+",
        "map: one level deeper",
        Group::Display,
    ),
    row(
        Action::TreemapShallower,
        "-",
        "map: one level less",
        Group::Display,
    ),
    row(
        Action::TreemapColor,
        "p",
        "map: colour by category, age or top folder",
        Group::Display,
    ),
    row(
        Action::ToggleApparent,
        "a",
        "toggle apparent size / disk usage",
        Group::Display,
    ),
    row(
        Action::ToggleReferenced,
        "u",
        "toggle disk usage like du (shared data once per file)",
        Group::Display,
    ),
    row(
        Action::ToggleHidden,
        "e",
        "hide or show hidden files",
        Group::Display,
    ),
    row(
        Action::Columns,
        "c",
        "list columns: items, then items and modified, then neither",
        Group::Display,
    ),
    row(
        Action::ToggleTree,
        "T",
        "tree mode: expand directories in place",
        Group::Display,
    ),
    row(
        Action::ExpandChain,
        "*",
        "tree mode: expand the largest, level by level",
        Group::Display,
    ),
    row(
        Action::CycleFold,
        "z",
        "tree mode: fold under 1%, then 0.1%, then none",
        Group::Display,
    ),
    row(
        Action::Rescan,
        "r",
        "rescan the selected directory",
        Group::Act,
    ),
    row(
        Action::EnterMount,
        "m",
        "scan into the selected mount (another filesystem)",
        Group::Act,
    ),
    row(
        Action::SkipStalled,
        "x",
        "skip directories that are not responding",
        Group::Act,
    ),
    row(
        Action::ToggleMark,
        "Space",
        "mark or unmark, then move down",
        Group::Act,
    ),
    row(
        Action::ClearMarks,
        "X",
        "untick all: clear every mark",
        Group::Act,
    ),
    row(
        Action::TickSuggested,
        "A",
        "tick what Cleanup or Duplicates suggests deleting",
        Group::Act,
    ),
    row(
        Action::CompareAgain,
        "R",
        "duplicates: compare the files again",
        Group::Act,
    ),
    row(
        Action::CleanupCommand,
        "N",
        "cleanup: the item's own cleanup command (dry run first)",
        Group::Act,
    ),
    row(
        Action::RangeMark,
        "v",
        "mark a range: moving marks every row passed (v or Esc ends)",
        Group::Act,
    ),
    row(
        Action::MarkPattern,
        "",
        "mark by pattern: names in this directory matching a glob",
        Group::Act,
    ),
    row(
        Action::InvertMarks,
        "",
        "invert the marks in this directory, as * does in the marks pane",
        Group::Act,
    ),
    row(
        Action::CopyPath,
        "y",
        "copy the selected path (through the terminal, OSC 52)",
        Group::Act,
    ),
    row(
        Action::OpenPath,
        "o",
        "open in the file manager (a file: its folder)",
        Group::Act,
    ),
    row(Action::Trash, "d", "move to Trash", Group::Act),
    row(
        Action::DeletePermanent,
        "D",
        "delete permanently (type delete)",
        Group::Act,
    ),
    row(
        Action::Undo,
        "U",
        "put back what was last moved to the Trash or staged",
        Group::Act,
    ),
    row(Action::Filter, "/", "filter this directory", Group::Panels),
    row(
        Action::Types,
        "t",
        "show only files of some types (video, audio, .iso...)",
        Group::Panels,
    ),
    row(
        Action::Details,
        "i",
        "details of the selected entry",
        Group::Panels,
    ),
    row(
        Action::FocusDetails,
        "Tab",
        "move focus: the list, the details, the marks pane",
        Group::Panels,
    ),
    row(
        Action::Errors,
        "!",
        "list what could not be read, and go to it",
        Group::Panels,
    ),
    row(Action::Help, "? F1", "keys", Group::Panels),
    row(
        Action::Palette,
        ": Ctrl-P",
        "command palette",
        Group::Panels,
    ),
    row(Action::Debug, "F12", "debug overlay", Group::Panels),
    row(
        Action::Close,
        "Esc",
        "close the overlay, clear the filter, or go back to Files",
        Group::Global,
    ),
    row(Action::Quit, "q Ctrl-C", "quit", Group::Global),
];

/// The actions of [`ACTIONS`] the browser offers: all of them, less those that delete when it is
/// `read_only`.
pub fn offered(read_only: bool) -> impl Iterator<Item = &'static ActionInfo> {
    ACTIONS
        .iter()
        .filter(move |info| !(read_only && info.action.deletes()))
}

/// An action offered that matches a search ([`search`]), with the matched characters of its
/// keys and of its label.
#[derive(Clone, Debug)]
pub struct Found {
    pub info: &'static ActionInfo,
    pub key_hits: Vec<u32>,
    pub label_hits: Vec<u32>,
}

/// The groups, in `groups` order, with the actions offered (as [`offered`]) that match `query`,
/// all of them for an empty one, in registry order. A query matches the keys and the label of
/// an action together; groups without a match are left out.
pub fn search(query: &str, read_only: bool, groups: &[Group]) -> Vec<(Group, Vec<Found>)> {
    let mut fuzzy = crate::fuzzy::Fuzzy::new(query);
    let mut hits = Vec::new();
    groups
        .iter()
        .filter_map(|&group| {
            let found: Vec<Found> = offered(read_only)
                .filter(|info| info.group == group)
                .filter_map(|info| {
                    let keys = crate::fuzzy::char_pos(info.keys.chars().count());
                    fuzzy.indices(&format!("{}  {}", info.keys, info.label), &mut hits)?;
                    let (key_hits, label_hits): (Vec<u32>, Vec<u32>) =
                        hits.iter().partition(|&&i| i < keys);
                    Some(Found {
                        info,
                        key_hits,
                        label_hits: label_hits.iter().map(|i| i - keys - 2).collect(),
                    })
                })
                .collect();
            (!found.is_empty()).then_some((group, found))
        })
        .collect()
}

/// The first key of `a` in [`ACTIONS`]; empty for an action without a key.
pub fn first_key(a: Action) -> &'static str {
    ACTIONS
        .iter()
        .find(|info| info.action == a)
        .and_then(|info| info.keys.split_whitespace().next())
        .unwrap_or("")
}

/// Where `a` moves a cursor at `cursor` in a list of rows up to `last` that shows `page` rows,
/// as the list moves its own; `None` for an action that does not move it. The views with a
/// cursor of their own move theirs by it.
#[must_use]
pub fn cursor_after(a: Action, cursor: usize, last: usize, page: usize) -> Option<usize> {
    let half = (page / 2).max(1);
    Some(match a {
        Action::Up => cursor.saturating_sub(1),
        Action::Down => (cursor + 1).min(last),
        Action::PageUp => cursor.saturating_sub(page),
        Action::PageDown => (cursor + page).min(last),
        Action::HalfPageUp => cursor.saturating_sub(half),
        Action::HalfPageDown => (cursor + half).min(last),
        Action::First => 0,
        Action::Last => last,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_key_is_the_first_binding() {
        assert_eq!(first_key(Action::Up), "↑");
        assert_eq!(first_key(Action::Palette), ":");
        assert_eq!(first_key(Action::ClearMarks), "X");
        assert_eq!(first_key(Action::MarkPattern), "");
    }

    #[test]
    fn a_read_only_browser_offers_no_action_that_deletes() {
        let all: Vec<Action> = offered(false).map(|info| info.action).collect();
        let read_only: Vec<Action> = offered(true).map(|info| info.action).collect();
        assert_eq!(all.len(), ACTIONS.len());
        assert_eq!(read_only.len(), all.len() - 3);
        assert!(read_only.iter().all(|a| !a.deletes()));
        assert!(all.contains(&Action::Undo) && !read_only.contains(&Action::Undo));
    }
}
