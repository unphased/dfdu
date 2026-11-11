//! The five views (`1`-`5`, `dfdu_session::view`): switching between them, and what the header's
//! tab bar says of each. The view shown is one field of the session
//! ([`SessionState::view`](dfdu_session::SessionState::view)), and the session's
//! `view::switch` is the one place that changes it, so `1`-`5`, `Esc` and a click on
//! a tab switch the same way ([`switch`]).
//!
//! Files, Map and Largest show the current directory; Cleanup and Duplicates cover the whole
//! scan ([`View::whole_scan`]), which the header's scope line says.

use super::{App, Effect};
use crate::features::{cleanup, dupes, treemap};
use crate::format::human;

use dfdu_session::view as session;
use dfdu_session::view::Then;
pub use dfdu_session::view::View;

/// The view shown ([`SessionState::current_view`](dfdu_session::SessionState::current_view)).
pub fn current(app: &App) -> View {
    app.session.current_view()
}

/// Switches to `view` ([`session::switch`]); the map opened forgets the last click.
pub fn switch(app: &mut App, view: View) -> Vec<Effect> {
    let then = session::switch(&mut app.session, view);
    if view == View::Map {
        treemap::on_open(app);
    }
    carry(app, then)
}

/// Carries out what an action or an intent of a view left to the TUI: moves the list's scroll
/// with a move to another directory, shows a path in Files ([`session::show`]), which moves the
/// scroll too, and returns the effects for the run loop.
pub fn carry(app: &mut App, then: Then) -> Vec<Effect> {
    if let Some(m) = then.entered {
        app.moved(m);
    }
    if let Some(m) = then.moved {
        app.went(m);
    }
    if let Some(show) = &then.show
        && let Some(m) = session::show(&mut app.session, show)
    {
        app.went(m);
    }
    then.effects
}

/// What the scope line adds after `whole scan: <root>` for `view`: which files Duplicates
/// compares.
pub fn scope_note(view: View) -> Option<String> {
    (view == View::Duplicates).then(|| format!("files of {} or more", human(dupes::min_size())))
}

/// What a tab adds after its name: Cleanup's reclaimable total once a read of its matches came
/// in, and the number of groups of duplicates once a comparison finished, `…` while one runs.
pub fn badge(app: &App, view: View) -> Option<String> {
    match view {
        View::Cleanup => cleanup::reclaimable(app).filter(|&b| b > 0).map(short_size),
        View::Duplicates => dupes::groups(app).map(|n| match n {
            dupes::Groups::Found(n) => crate::format::count(n as u64),
            dupes::Groups::Running => "…".into(),
        }),
        View::Files | View::Map | View::Largest => None,
    }
}

/// A size in as few cells as a tab has room for: `3.1G` for `3.1 GiB`, `512B` for `512 B`.
fn short_size(bytes: u64) -> String {
    let long = human(bytes);
    match long.split_once(' ') {
        Some((n, unit)) => format!("{n}{}", unit.trim_end_matches("iB")),
        None => long,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ratatui::crossterm::event::KeyCode;

    use super::*;
    use crate::app::keys::{self, Action};
    use crate::tests::classified_app;
    use crate::tests::{library_snap, press};
    use crate::theme::Theme;

    /// The views actions lead to, one after each action, from Files.
    fn after(app: &mut App, actions: &[Action]) -> Vec<View> {
        actions
            .iter()
            .map(|&action| {
                keys::run(app, action);
                current(app)
            })
            .collect()
    }

    #[test]
    fn actions_step_through_the_views_round_the_ends() {
        let (_fx, mut app) = classified_app(Instant::now());
        let next = [Action::NextView; 5];
        assert_eq!(
            after(&mut app, &next),
            [
                View::Map,
                View::Cleanup,
                View::Duplicates,
                View::Largest,
                View::Files
            ]
        );
        let prev = [Action::PrevView; 2];
        assert_eq!(after(&mut app, &prev), [View::Largest, View::Duplicates]);
    }

    #[test]
    fn without_rules_view_actions_pass_over_cleanup() {
        let t0 = Instant::now();
        let mut app = App::new(Theme::no_color(), t0);
        app.refresh(library_snap(), t0);
        let actions = [Action::NextView, Action::NextView, Action::PrevView];
        assert_eq!(
            after(&mut app, &actions),
            [View::Map, View::Duplicates, View::Map]
        );
    }

    #[test]
    fn esc_goes_back_to_files_once_nothing_is_left_to_cancel() {
        let t0 = Instant::now();
        let mut app = App::new(Theme::no_color(), t0);
        app.refresh(library_snap(), t0);
        for key in ['2', '4', '5'] {
            app.key(press(KeyCode::Char(key)), t0);
            assert_ne!(current(&app), View::Files);
            app.key(press(KeyCode::Esc), t0);
            assert_eq!(current(&app), View::Files, "from {key}");
        }
        // A kept filter is cleared first; the next Esc goes back.
        app.key(press(KeyCode::Char('2')), t0);
        for c in "/lib".chars() {
            app.key(press(KeyCode::Char(c)), t0);
        }
        app.key(press(KeyCode::Enter), t0);
        app.reorder(t0);
        assert!(app.session.filter.active());
        app.key(press(KeyCode::Esc), t0);
        app.reorder(t0);
        assert!(!app.session.filter.active());
        assert_eq!(current(&app), View::Map);
        app.key(press(KeyCode::Esc), t0);
        assert_eq!(current(&app), View::Files);
    }

    #[test]
    fn cleanup_without_rules_is_neither_switched_to_nor_shown() {
        let t0 = Instant::now();
        let mut app = App::new(Theme::no_color(), t0);
        app.refresh(library_snap(), t0);
        assert!(switch(&mut app, View::Cleanup).is_empty());
        assert_eq!(app.session.view, View::Files);
        assert!(
            app.session
                .status
                .as_deref()
                .is_some_and(|s| s.starts_with("no rules"))
        );
        app.session.view = View::Cleanup;
        assert_eq!(current(&app), View::Files);
    }

    #[test]
    fn short_sizes_drop_the_space_and_the_binary_suffix() {
        assert_eq!(short_size(512), "512B");
        assert_eq!(short_size(3 * (1 << 30) + (1 << 30) / 10), "3.1G");
        assert_eq!(short_size(1 << 20), "1.0M");
    }
}
