//! Carrying out the actions of the registry ([`run`]): the session does what they do to the
//! browser ([`dfdu_session::act::run`]); the terminal opens its overlays and panels, copies
//! through the terminal, quits and asks the run loop for a scan outside this one, and follows
//! what the session left with its scroll and the treemap's view state.

use dfdu_session::act::{self as session, Ran};

use super::Action;
use crate::app::views::{self, View};
use crate::app::{App, Effect, Overlay};
use crate::features::{debug, details, selected_path, treemap};
use crate::overlay::{goto, help, mark_pattern, palette, type_filter};

/// Carries out `a` on the list (no overlay open), at the time of the input that asked for it,
/// and returns the effects for the run loop. In a read-only browser an action that deletes
/// only says so.
pub fn run(app: &mut App, a: Action) -> Vec<Effect> {
    if a == Action::Parent && app.session.snap.at_root && app.session.snap.roots.len() > 1 {
        if !dfdu_session::tree::act::left(&mut app.session) {
            let selected = app
                .session
                .snap
                .roots
                .iter()
                .position(|root| *root == app.session.snap.root_path)
                .unwrap_or(0);
            app.overlay = Overlay::Roots(selected);
        }
        return Vec::new();
    }
    let now = app.session.sort_schedule.last_input();
    let mut then = match session::run(&mut app.session, a, now) {
        Ran::Done(then) => then,
        Ran::FrontEnd => return front_end(app, a),
    };
    match a {
        // The pane opened or closed shows from its top, and the list has the focus.
        Action::Details => app.details = details::Details::default(),
        // The map opened forgets the last click.
        Action::ViewMap | Action::PrevView | Action::NextView
            if views::current(app) == View::Map =>
        {
            treemap::on_open(app);
        }
        _ => {}
    }
    // Up past the scan root: the run loop opens the new scan.
    if let Some(path) = then.reopen.take() {
        app.reopen = Some(path);
    }
    views::carry(app, then)
}

/// The actions only the terminal carries out ([`Ran::FrontEnd`]).
fn front_end(app: &mut App, a: Action) -> Vec<Effect> {
    match a {
        Action::Quit => app.quit = true,
        Action::Columns => {
            app.columns = app.columns.next();
            app.session.status = Some(app.columns.describe().into());
        }
        Action::Help => help::open(app),
        Action::Palette => palette::open(app),
        Action::Debug => debug::toggle(app),
        Action::Types => type_filter::open(app),
        Action::MarkPattern => mark_pattern::open(app),
        Action::GoTo => goto::open(app),
        Action::FocusDetails => return details::focus(app),
        Action::CopyPath => return selected_path::copy(app).into_iter().collect(),
        _ => {}
    }
    Vec::new()
}
