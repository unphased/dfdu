//! The cursor moves into and out of directories with the list's scroll: the session saves the
//! selection of each directory left (`SessionState::enter`, `up`, `go_to`) and the app the
//! scroll with it, to restore both on the way back.

use dfdu_session::state::Moved;
use dfdu_tree::HostPath;

use super::App;

impl App {
    /// Jumps among literal substring hits in the visible Files rows, without changing the filter.
    pub(crate) fn jump_match(&mut self, backwards: bool) {
        let query = self.session.filter.query();
        if query.is_empty() {
            return;
        }
        let needle = query.to_lowercase();
        let hits: Vec<usize> = self
            .session
            .list
            .rows
            .iter()
            .enumerate()
            .filter_map(|(row, &i)| {
                let e = &self.session.snap.entries[i];
                (!e.pseudo
                    && String::from_utf8_lossy(&e.name)
                        .to_lowercase()
                        .contains(&needle))
                .then_some(row)
            })
            .collect();
        if hits.is_empty() {
            self.session.status = Some(format!("no exact match for \"{query}\""));
            return;
        }
        let current = self.session.selected_index();
        let at = if backwards {
            hits.iter()
                .rposition(|&row| current.is_some_and(|c| row < c))
                .unwrap_or(hits.len() - 1)
        } else {
            hits.iter()
                .position(|&row| current.is_none_or(|c| row > c))
                .unwrap_or(0)
        };
        self.session.move_to(hits[at]);
        self.session.status = Some(format!("exact {}/{}", at + 1, hits.len()));
    }

    /// Keeps the scroll of the directory left, and puts back that of the directory opened.
    pub(crate) fn moved(&mut self, m: Moved) {
        self.scrolls.insert(m.left, self.scroll);
        if let Some(dir) = m.opened {
            self.scroll = self.scrolls.get(&dir).copied().unwrap_or(0);
        }
    }

    /// Goes to an entry below the scan root `root`: opens its parent directory and selects it,
    /// with the list scrolled to the top.
    pub(crate) fn go_to(&mut self, path: &HostPath, root: &HostPath) {
        if let Some(m) = self.session.go_to(path, root) {
            self.went(m);
        }
    }

    /// After the session went to an entry in another directory: keeps the scroll of the
    /// directory left, and shows the one opened from the top.
    pub(crate) fn went(&mut self, m: Moved) {
        self.scrolls.insert(m.left, self.scroll);
        self.scroll = 0;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::model::snapshot::{EntryData, Snapshot, test_entry};
    use crate::tests::{snap_with_totals, snap_without_totals};
    use crate::theme::Theme;

    fn dir(i: u32, name: &str, size: u64) -> EntryData {
        EntryData {
            is_dir: true,
            ..test_entry(i, name, size)
        }
    }

    /// A browser of `/r` with the directories `d0` (the largest) to `d4`.
    fn browser() -> (App, Instant) {
        let t0 = Instant::now();
        let mut app = App::new(Theme::no_color(), t0);
        let entries = (0..5)
            .map(|i| dir(i, &format!("d{i}"), 100 - u64::from(i)))
            .collect();
        app.refresh(snap_with_totals(entries, 1), t0);
        (app, t0)
    }

    fn selected(app: &App) -> Option<String> {
        app.session
            .selected_entry()
            .map(|e| String::from_utf8_lossy(&e.name).into_owned())
    }

    fn filtered_browser(query: &str) -> (App, Instant) {
        use dfdu_session::{filter, line::TextOp};
        let t0 = Instant::now();
        let mut app = App::new(Theme::no_color(), t0);
        app.refresh(
            snap_with_totals(
                vec![
                    test_entry(0, "PYthon", 40),
                    test_entry(1, "partly", 30),
                    test_entry(2, "copy", 20),
                    test_entry(3, "hidden", 10),
                ],
                1,
            ),
            t0,
        );
        filter::open(&mut app.session);
        for c in query.chars() {
            filter::edit(&mut app.session, TextOp::Insert(c));
        }
        app.session.request_refilter();
        app.reorder(t0);
        filter::edit(&mut app.session, TextOp::Submit);
        (app, t0)
    }

    fn hit(app: &mut App, t0: Instant, c: char) {
        app.key(
            crate::tests::press(ratatui::crossterm::event::KeyCode::Char(c)),
            t0,
        );
    }

    #[test]
    fn exact_jumps_skip_fuzzy_hits_and_wrap_in_both_directions() {
        let (mut app, t0) = filtered_browser("py");
        assert_eq!(app.session.list.rows.len(), 3);
        app.session.move_to(0);
        for (key, name, status) in [
            ('n', "copy", "exact 2/2"),
            ('n', "PYthon", "exact 1/2"),
            ('N', "copy", "exact 2/2"),
            ('N', "PYthon", "exact 1/2"),
        ] {
            hit(&mut app, t0, key);
            assert_eq!(selected(&app).as_deref(), Some(name));
            assert_eq!(app.session.status.as_deref(), Some(status));
        }
        app.session.move_to(1);
        hit(&mut app, t0, 'N');
        assert_eq!(selected(&app).as_deref(), Some("PYthon"));
    }

    #[test]
    fn exact_jumps_with_no_hits_or_no_filter() {
        let (mut app, t0) = filtered_browser("pl");
        let before = selected(&app);
        for key in ['n', 'N'] {
            hit(&mut app, t0, key);
            assert_eq!(selected(&app), before);
            assert_eq!(
                app.session.status.as_deref(),
                Some("no exact match for \"pl\"")
            );
        }
        let (mut app, t0) = filtered_browser("");
        for key in ['n', 'N'] {
            hit(&mut app, t0, key);
            assert_eq!(selected(&app).as_deref(), Some("PYthon"));
            assert!(app.session.status.is_none());
        }
    }

    #[test]
    fn a_sorts_by_name_while_n_jumps() {
        let (mut app, t0) = filtered_browser("py");
        hit(&mut app, t0, 'A');
        assert_eq!(app.session.sort.key, crate::state::order::SortKey::Name);
        hit(&mut app, t0, 'n');
        assert_eq!(app.session.sort.key, crate::state::order::SortKey::Name);
        assert!(
            app.session
                .status
                .as_deref()
                .unwrap_or("")
                .starts_with("exact ")
        );
    }

    #[test]
    fn moves_stay_within_the_rows() {
        let (mut app, _) = browser();
        assert_eq!(app.session.selected_index(), Some(0));
        app.session.move_by(3);
        assert_eq!(selected(&app).as_deref(), Some("d3"));
        app.session.move_by(10);
        assert_eq!(
            app.session.selected_index(),
            Some(4),
            "the last row at most"
        );
        app.session.move_by(-10);
        assert_eq!(
            app.session.selected_index(),
            Some(0),
            "the first row at least"
        );
        app.session.move_to(2);
        assert_eq!(selected(&app).as_deref(), Some("d2"));
        // A row that is not there selects nothing new.
        app.session.select(99);
        assert_eq!(selected(&app).as_deref(), Some("d2"));
    }

    #[test]
    fn the_selection_is_found_where_it_moved_to() {
        let (mut app, t0) = browser();
        app.session.move_to(3);
        // The same entries, the selected one now the largest.
        let mut entries = app.session.snap.entries.clone();
        entries[3].size = 1000;
        app.refresh(snap_with_totals(entries, 2), t0);
        app.reorder(t0);
        let at = app.session.selected_index().expect("still selected");
        assert_eq!(
            app.session.snap.entries[app.session.list.rows[at]].name,
            b"d3"
        );
    }

    #[test]
    fn the_cursor_of_a_directory_left_comes_back() {
        let (mut app, t0) = browser();
        app.session.move_to(2);
        crate::app::keys::run(&mut app, crate::app::keys::Action::Enter);
        assert_eq!(app.session.target.path, HostPath::from("/r").join(b"d2"));
        assert!(
            app.session.selected.is_none(),
            "a directory not seen before starts at the top"
        );
        let inside = Snapshot {
            dir_path: HostPath::from("/r").join(b"d2"),
            at_root: false,
            ..snap_without_totals(vec![dir(10, "inner", 5)], 2)
        };
        app.refresh(inside, t0);
        // Up selects the directory come from, by name until the rows are read.
        crate::app::keys::run(&mut app, crate::app::keys::Action::Parent);
        assert_eq!(app.session.target.path, HostPath::from("/r"));
        let sel = app.session.selected.as_ref().expect("a selection");
        assert_eq!((sel.node, &sel.name[..]), (None, &b"d2"[..]));
    }

    #[test]
    fn go_to_opens_the_parent_and_selects_by_name() {
        let (mut app, _) = browser();
        let root = HostPath::from("/r");
        let path = root.join(b"d1").join(b"deep.bin");
        app.go_to(&path, &root);
        assert_eq!(app.session.target.path, root.join(b"d1"));
        assert_eq!(
            app.session.selected.as_ref().map(|s| &s.name[..]),
            Some(&b"deep.bin"[..])
        );
        // The root itself has no parent to open below the scan.
        app.go_to(&root, &root);
        assert_eq!(app.session.target.path, root.join(b"d1"));
    }
}
