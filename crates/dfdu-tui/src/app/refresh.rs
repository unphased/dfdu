//! Folding a new [`Snapshot`] into [`App`]: the session brings the rows and the selection up to
//! date ([`SessionState::refresh`](dfdu_session::SessionState::refresh)); the deleted rows kept
//! as placeholders, the rows moving to their places and the scroll are the terminal's. The
//! session driver folds its reads in through the app as its front end (`FrontEnd`).

use std::time::Instant;

use dfdu_session::extras::Applied;
use dfdu_session::state::Reordered;
use dfdu_session::{FrontEnd, SessionState};

use super::App;
use crate::model::snapshot::Snapshot;

#[cfg(test)]
mod tests;

impl App {
    pub fn refresh(&mut self, mut snap: Snapshot, now: Instant) {
        self.motion.carry_ghosts(&self.session.snap, &mut snap, now);
        // Deleted rows kept as placeholders hold still until the next re-sort.
        let hold = self.motion.has_ghosts();
        let r = self.session.refresh(snap, now, hold);
        self.reordered(r);
        if !self.roots_initialized && self.session.snap.roots.len() > 1 {
            self.roots_initialized = true;
            if matches!(self.overlay, super::Overlay::None) {
                self.overlay = super::Overlay::Roots(0);
            }
        }
    }

    /// Applies sort requests and the automatic sort to the snapshot already held, for a frame
    /// drawn without reading the engine again.
    pub fn reorder(&mut self, now: Instant) {
        let hold = self.motion.has_ghosts();
        let r = self.session.reorder(now, hold);
        self.reordered(r);
    }

    /// Moves the rows of a re-sort to their new places, and the scroll where the session says.
    fn reordered(&mut self, r: Reordered) {
        if let Some(re) = &r.resorted {
            let s = &self.session;
            (self.motion).resorted(&re.before, &s.list.rows, &s.snap.entries, re.auto);
        }
        self.scroll = r.scroll.apply(self.scroll);
    }
}

/// The app around the session's state, as the session driver folds reads into it: with the
/// rows' motion and the scroll, the details pane's scroll, and the type picker's breakdown.
impl FrontEnd for App {
    fn state(&self) -> &SessionState {
        &self.session
    }

    fn state_mut(&mut self) -> &mut SessionState {
        &mut self.session
    }

    fn refresh(&mut self, snap: Snapshot, now: Instant) {
        App::refresh(self, snap, now);
    }

    fn reorder(&mut self, now: Instant) {
        App::reorder(self, now);
    }

    fn applied(&mut self, applied: Applied) {
        if applied.details_moved {
            self.details.scroll = 0;
        }
    }

    fn breakdown(&self) -> bool {
        matches!(self.overlay, super::Overlay::Types(_))
    }
}
