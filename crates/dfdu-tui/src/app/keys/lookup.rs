//! Looking keys up in [`ACTIONS`]: the action a key runs.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub use dfdu_session::actions::cursor_after;

use super::{ACTIONS, Action};

/// One key of [`ActionInfo::keys`]: the key code and whether it needs Ctrl.
pub(crate) fn parse(key: &str) -> Option<(KeyCode, bool)> {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(match c {
            '↑' => (KeyCode::Up, false),
            '↓' => (KeyCode::Down, false),
            '←' => (KeyCode::Left, false),
            '→' => (KeyCode::Right, false),
            c => (KeyCode::Char(c), false),
        });
    }
    let code = match key {
        "PgUp" => KeyCode::PageUp,
        "PgDn" => KeyCode::PageDown,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "Enter" => KeyCode::Enter,
        "Backspace" => KeyCode::Backspace,
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "Space" => KeyCode::Char(' '),
        _ => {
            if let Some(n) = key.strip_prefix('F').and_then(|n| n.parse().ok()) {
                return Some((KeyCode::F(n), false));
            }
            let mut c = key.strip_prefix("Ctrl-")?.chars();
            return match (c.next(), c.next()) {
                (Some(c), None) => Some((KeyCode::Char(c.to_ascii_lowercase()), true)),
                _ => None,
            };
        }
    };
    Some((code, false))
}

/// The action bound to `k`. A `Ctrl-` binding needs Ctrl; any other binding matches its key
/// whatever the modifiers.
pub fn action_for(k: &KeyEvent) -> Option<Action> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let bound = |want_ctrl: bool| {
        ACTIONS.iter().find_map(|info| {
            info.keys
                .split_whitespace()
                .filter_map(parse)
                .any(|(code, needs_ctrl)| code == k.code && needs_ctrl == want_ctrl)
                .then_some(info.action)
        })
    };
    if ctrl && let Some(a) = bound(true) {
        return Some(a);
    }
    bound(false)
}

/// The rows `k` moves by when it is bound to half a page up or down, in a list that shows
/// `page` rows; `None` for any other key. The views and overlays with a cursor of their own
/// move theirs by it, as the list does.
pub fn half_page(k: &KeyEvent, page: usize) -> Option<isize> {
    let half = isize::try_from(page / 2).unwrap_or(isize::MAX).max(1);
    match action_for(k)? {
        Action::HalfPageUp => Some(-half),
        Action::HalfPageDown => Some(half),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::tests::press;

    #[test]
    fn keys_parse_by_name_and_with_ctrl() {
        assert_eq!(parse("q"), Some((KeyCode::Char('q'), false)));
        assert_eq!(parse("↑"), Some((KeyCode::Up, false)));
        assert_eq!(parse("PgDn"), Some((KeyCode::PageDown, false)));
        assert_eq!(parse("Space"), Some((KeyCode::Char(' '), false)));
        assert_eq!(parse("F12"), Some((KeyCode::F(12), false)));
        assert_eq!(parse("Ctrl-P"), Some((KeyCode::Char('p'), true)));
        for bad in ["", "Ctrl-", "Ctrl-PP", "Fx", "Shift-a"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn existing_keys_map_to_the_same_actions() {
        let c = KeyCode::Char;
        let table = [
            (c('q'), Action::Quit),
            (KeyCode::Up, Action::Up),
            (c('k'), Action::Up),
            (KeyCode::Down, Action::Down),
            (c('j'), Action::Down),
            (KeyCode::PageUp, Action::PageUp),
            (KeyCode::PageDown, Action::PageDown),
            (KeyCode::Home, Action::First),
            (c('g'), Action::First),
            (KeyCode::End, Action::Last),
            (c('G'), Action::Last),
            (KeyCode::Right, Action::Open),
            (c('l'), Action::Open),
            (KeyCode::Enter, Action::Enter),
            (KeyCode::Left, Action::Parent),
            (c('h'), Action::Parent),
            (KeyCode::Backspace, Action::Parent),
            (c('s'), Action::SortSize),
            (c('A'), Action::SortName),
            (c('n'), Action::NextMatch),
            (c('N'), Action::PrevMatch),
            (c('C'), Action::SortItems),
            (c('M'), Action::SortMtime),
            (c('S'), Action::Resort),
            (c('a'), Action::ToggleApparent),
            (c('u'), Action::ToggleReferenced),
            (c('e'), Action::ToggleHidden),
            (c('c'), Action::Columns),
            (c('r'), Action::Rescan),
            (c('m'), Action::EnterMount),
            (c('x'), Action::SkipStalled),
            (c('d'), Action::Trash),
            (c('D'), Action::DeletePermanent),
            (c('U'), Action::Undo),
            (c('?'), Action::Help),
            (c('!'), Action::Errors),
            (KeyCode::Esc, Action::Close),
        ];
        for (code, want) in table {
            assert_eq!(action_for(&press(code)), Some(want), "{code:?}");
        }
        // Keys match whatever the modifiers: Shift for capitals, and Ctrl on a key without a
        // Ctrl binding of its own.
        let shifted = KeyEvent::new(c('G'), KeyModifiers::SHIFT);
        assert_eq!(action_for(&shifted), Some(Action::Last));
        let ctrl_j = KeyEvent::new(c('j'), KeyModifiers::CONTROL);
        assert_eq!(action_for(&ctrl_j), Some(Action::Down));
        let ctrl_p = KeyEvent::new(c('p'), KeyModifiers::CONTROL);
        assert_eq!(action_for(&ctrl_p), Some(Action::Palette));
        assert_eq!(action_for(&press(c('p'))), Some(Action::TreemapColor));
        for (code, want) in [
            (c(' '), Action::ToggleMark),
            (c('/'), Action::Filter),
            (c('i'), Action::Details),
            (KeyCode::Tab, Action::FocusDetails),
            (c(':'), Action::Palette),
            (KeyCode::F(12), Action::Debug),
            (KeyCode::F(1), Action::Help),
            (c('<'), Action::Parent),
            (c('1'), Action::ViewFiles),
            (c('2'), Action::ViewMap),
            (c('3'), Action::ViewCleanup),
            (c('4'), Action::ViewDupes),
            (c('t'), Action::Types),
            (c('+'), Action::TreemapDeeper),
            (c('-'), Action::TreemapShallower),
            (c('T'), Action::ToggleTree),
            (c('*'), Action::ExpandChain),
            (c('z'), Action::CycleFold),
        ] {
            assert_eq!(action_for(&press(code)), Some(want), "{code:?}");
        }
        for code in [c('Z'), c('6'), KeyCode::F(2), KeyCode::Insert] {
            assert_eq!(action_for(&press(code)), None, "{code:?}");
        }
    }

    #[test]
    fn navigation_and_mark_keys() {
        let c = KeyCode::Char;
        for (code, want) in [
            (c('y'), Action::CopyPath),
            (c('o'), Action::OpenPath),
            (c('v'), Action::RangeMark),
            (c('5'), Action::ViewLargest),
        ] {
            assert_eq!(action_for(&press(code)), Some(want), "{code:?}");
        }
        // Ctrl-D and Ctrl-U move half a page; without Ctrl the keys stay what they were.
        let ctrl = |ch| KeyEvent::new(c(ch), KeyModifiers::CONTROL);
        assert_eq!(action_for(&ctrl('d')), Some(Action::HalfPageDown));
        assert_eq!(action_for(&ctrl('u')), Some(Action::HalfPageUp));
        assert_eq!(action_for(&press(c('d'))), Some(Action::Trash));
        assert_eq!(action_for(&press(c('u'))), Some(Action::ToggleReferenced));
        assert_eq!(half_page(&ctrl('d'), 20), Some(10));
        assert_eq!(half_page(&ctrl('u'), 20), Some(-10));
        assert_eq!(half_page(&ctrl('d'), 1), Some(1), "at least a row");
        assert_eq!(half_page(&press(c('d')), 20), None);
    }

    #[test]
    fn cursors_move_within_the_rows() {
        assert_eq!(cursor_after(Action::Up, 0, 9, 4), Some(0));
        assert_eq!(cursor_after(Action::Down, 9, 9, 4), Some(9));
        assert_eq!(cursor_after(Action::PageDown, 3, 9, 4), Some(7));
        assert_eq!(cursor_after(Action::PageUp, 3, 9, 4), Some(0));
        assert_eq!(cursor_after(Action::HalfPageDown, 3, 9, 4), Some(5));
        assert_eq!(
            cursor_after(Action::HalfPageUp, 3, 9, 1),
            Some(2),
            "at least a row"
        );
        assert_eq!(cursor_after(Action::First, 3, 9, 4), Some(0));
        assert_eq!(cursor_after(Action::Last, 3, 9, 4), Some(9));
        assert_eq!(cursor_after(Action::Down, 0, 0, 4), Some(0), "no rows");
        assert_eq!(cursor_after(Action::ToggleMark, 3, 9, 4), None);
    }
}
