use std::collections::HashSet;
use std::time::Instant;

use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

use super::*;
use crate::app::App;

const ALL: [Action; 66] = [
    Action::Quit,
    Action::Up,
    Action::Down,
    Action::PageUp,
    Action::PageDown,
    Action::HalfPageUp,
    Action::HalfPageDown,
    Action::First,
    Action::Last,
    Action::Open,
    Action::Enter,
    Action::Parent,
    Action::GoTo,
    Action::Peer,
    Action::PeerSwap,
    Action::SortSize,
    Action::SortName,
    Action::NextMatch,
    Action::PrevMatch,
    Action::SortItems,
    Action::SortMtime,
    Action::Resort,
    Action::ToggleApparent,
    Action::ToggleReferenced,
    Action::ToggleHidden,
    Action::Columns,
    Action::ToggleTree,
    Action::ExpandChain,
    Action::CycleFold,
    Action::Rescan,
    Action::EnterMount,
    Action::SkipStalled,
    Action::Trash,
    Action::DeletePermanent,
    Action::Undo,
    Action::Help,
    Action::Errors,
    Action::Close,
    Action::ToggleMark,
    Action::ClearMarks,
    Action::RangeMark,
    Action::MarkPattern,
    Action::InvertMarks,
    Action::CopyPath,
    Action::OpenPath,
    Action::Filter,
    Action::Types,
    Action::Details,
    Action::FocusDetails,
    Action::Palette,
    Action::Debug,
    Action::ViewFiles,
    Action::ViewMap,
    Action::ViewCleanup,
    Action::ViewDupes,
    Action::ViewLargest,
    Action::PrevView,
    Action::NextView,
    Action::TickSuggested,
    Action::DupesKeep,
    Action::CompareAgain,
    Action::CleanupFilter,
    Action::CleanupCommand,
    Action::TreemapDeeper,
    Action::TreemapShallower,
    Action::TreemapColor,
];

#[test]
fn every_action_has_one_registry_row() {
    for a in ALL {
        let rows = ACTIONS.iter().filter(|r| r.action == a).count();
        assert_eq!(rows, 1, "{a:?}");
    }
    assert_eq!(ACTIONS.len(), ALL.len());
    let mut claimed = HashSet::new();
    for r in ACTIONS {
        assert!(!r.label.is_empty(), "{:?} has no label", r.action);
        for k in r.keys.split_whitespace() {
            let parsed = parse(k).unwrap_or_else(|| panic!("{:?}: bad key {k}", r.action));
            if !claimed.insert(parsed) {
                let shared: Vec<_> = ACTIONS
                    .iter()
                    .filter(|info| info.keys.split_whitespace().any(|key| key == k))
                    .map(|info| info.action)
                    .collect();
                assert!(
                    shared == [Action::SortName, Action::TickSuggested]
                        || shared == [Action::PrevMatch, Action::CleanupCommand],
                    "{k} is bound twice without a view-specific override"
                );
            }
        }
    }
    let groups: HashSet<_> = ACTIONS.iter().map(|r| format!("{:?}", r.group)).collect();
    assert_eq!(groups.len(), 6, "every group has an action");
    let keyless: Vec<_> = ACTIONS.iter().filter(|r| r.keys.is_empty()).collect();
    let keyless: Vec<_> = keyless.iter().map(|r| r.action).collect();
    assert_eq!(keyless, [Action::MarkPattern, Action::InvertMarks]);
}

/// What a view may do with a registry key before the registry sees it: move its own cursor
/// (the navigation keys), mark the row its cursor is on (`Space`, as in the list), and, with
/// nothing marked, say what to mark before the delete keys delete anything. Every other key
/// means the same in every view.
fn view_may_take(a: Action) -> bool {
    let nav = ACTIONS
        .iter()
        .any(|r| r.action == a && r.group == Group::Navigate);
    nav || matches!(
        a,
        Action::ToggleMark | Action::Trash | Action::DeletePermanent
    )
}

#[test]
fn no_view_rebinds_a_global_key() {
    use crate::app::views::{self, View};
    use crate::features::{cleanup, dupes, largest, treemap};
    use crate::tests::{classified, sync};
    let fx = classified();
    let t0 = Instant::now();
    let opened = |view: View| {
        let mut app = App::new(crate::theme::Theme::no_color(), t0);
        sync(&mut app, &fx.m, t0);
        views::switch(&mut app, view);
        sync(&mut app, &fx.m, t0);
        app
    };
    for view in [View::Map, View::Cleanup, View::Duplicates, View::Largest] {
        for info in ACTIONS {
            for key in info.keys.split_whitespace() {
                let (code, ctrl) = parse(key).expect("a key");
                let mods = if ctrl {
                    KeyModifiers::CONTROL
                } else {
                    KeyModifiers::NONE
                };
                let k = KeyEvent::new(code, mods);
                let mut app = opened(view);
                let taken = match view {
                    View::Map => treemap::key(&mut app, &k).is_some(),
                    View::Cleanup => cleanup::key(&mut app, &k).is_some(),
                    View::Duplicates => dupes::key(&mut app, &k).is_some(),
                    View::Largest => largest::key(&mut app, &k).is_some(),
                    View::Files => false,
                };
                assert!(
                    !taken || view_may_take(info.action),
                    "{view:?} takes {key}, bound to {:?}",
                    info.action
                );
            }
        }
    }
}
