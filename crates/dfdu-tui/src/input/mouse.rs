//! Mouse input: clicks, double clicks and the wheel, mapped through `App::areas`.
//!
//! A click selects a list row (or treemap tile) and a second click on it within [`DOUBLE_CLICK`]
//! opens it; in tree mode a click on a row's `▸`/`▾` expands or collapses it. A click
//! on the "new entries" row sorts, one on a tab switches to its view, one on a breadcrumb goes
//! to its directory, and one outside an open overlay closes it. The delete dialog ignores the mouse, so a stray click never confirms
//! or cancels a delete. The wheel moves the selection, or scrolls the details pane. Over the
//! debug panel, which hides the rows below it, clicks and the wheel do nothing. Pointer movement
//! changes nothing and is not input: it draws no frame and does not hold off the automatic
//! re-sort.

use std::time::{Duration, Instant};

use dfdu_tree::NodeRef;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::app::keys::{self, Action};
use crate::app::{App, Effect, Overlay};
use crate::features::details;

/// A second click on the same row within this long opens it.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Rows one wheel step moves.
const WHEEL_ROWS: i8 = 1;

#[derive(Clone, Debug, Default)]
pub struct Mouse {
    /// When the last click on a list row was, and the entry on that row.
    last_click: Option<(Instant, NodeRef)>,
    /// The breadcrumbs the last frame drew, except the current directory's.
    pub crumbs: Vec<Crumb>,
    /// The tabs of the views the last frame drew.
    pub tabs: Vec<Tab>,
    /// The column the list's name cells start at in the last frame: in tree mode a click on
    /// the `▸`/`▾` after a row's guide expands or collapses it.
    pub name_x: u16,
}

/// A breadcrumb on screen. A click goes to its directory and selects `child`, the directory
/// below it on the way to the current one.
#[derive(Clone, Debug)]
pub struct Crumb {
    pub area: Rect,
    pub child: dfdu_tree::HostPath,
}

/// A view's tab on screen. A click switches to the view.
#[derive(Clone, Debug)]
pub struct Tab {
    pub area: Rect,
    pub view: crate::app::views::View,
}

/// Handles a mouse event; `None` when it changed nothing, so no frame is drawn.
pub fn event(app: &mut App, m: MouseEvent, now: Instant) -> Option<Vec<Effect>> {
    let at = Position::new(m.column, m.row);
    let up = match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            app.session.note_input(now);
            app.session.disarm();
            return click(app, at, now);
        }
        MouseEventKind::ScrollUp => true,
        MouseEventKind::ScrollDown => false,
        _ => return None,
    };
    app.session.note_input(now);
    app.session.disarm();
    if !matches!(app.overlay, Overlay::None)
        || crate::overlay::confirm::is_open(app)
        || over_debug(app, at)
    {
        return None;
    }
    let step = if up {
        -i32::from(WHEEL_ROWS)
    } else {
        i32::from(WHEEL_ROWS)
    };
    if (app.areas.details.is_some_and(|r| r.contains(at))
        || details::focused(app)
        || crate::features::marks_pane::is_open(app))
        && details::scroll(app, step)
    {
        return Some(Vec::new());
    }
    if app.session.list.rows.is_empty() {
        return None;
    }
    app.session.status = None;
    let rows = isize::from(WHEEL_ROWS);
    app.session.move_by(if up { -rows } else { rows });
    Some(Vec::new())
}

fn over_debug(app: &App, at: Position) -> bool {
    app.areas.debug.is_some_and(|r| r.contains(at))
}

fn click(app: &mut App, at: Position, now: Instant) -> Option<Vec<Effect>> {
    if crate::overlay::confirm::is_open(app) {
        return None;
    }
    match app.overlay {
        Overlay::None => {}
        _ if app.areas.overlay.is_some_and(|r| r.contains(at)) => return None,
        _ => {
            app.overlay = Overlay::None;
            return Some(Vec::new());
        }
    }
    if over_debug(app, at) {
        return None;
    }
    if let Some(tab) = app.mouse.tabs.iter().find(|t| t.area.contains(at)) {
        let view = tab.view;
        app.session.status = None;
        app.mouse.last_click = None;
        return Some(crate::app::views::switch(app, view));
    }
    if let Some(c) = app.mouse.crumbs.iter().find(|c| c.area.contains(at)) {
        let (child, root) = (c.child.clone(), app.session.snap.root_path.clone());
        app.session.status = None;
        app.mouse.last_click = None;
        app.go_to(&child, &root);
        return Some(Vec::new());
    }
    if !app.areas.list.contains(at) {
        return None;
    }
    if crate::features::treemap::active(app) {
        app.mouse.last_click = None;
        return crate::features::treemap::click(app, at, now);
    }
    click_list(app, at, now)
}

/// A click at `at` in the list: on an entry's row, or on the "new entries" row after them.
fn click_list(app: &mut App, at: Position, now: Instant) -> Option<Vec<Effect>> {
    // The rows as the list drew them: entries from `scroll`, then the "new entries" row.
    let list = app.areas.list;
    let line = (at.y - list.y) as usize;
    let new_row = app.session.list.new_count > 0;
    let entries = app
        .session
        .list
        .rows
        .len()
        .saturating_sub(app.scroll)
        .min((list.height as usize).saturating_sub(usize::from(new_row)));
    app.session.status = None;
    if line < entries {
        return Some(click_row(app, app.scroll + line, at.x, now));
    }
    if line == entries && new_row {
        app.mouse.last_click = None;
        return Some(keys::run(app, Action::Resort));
    }
    None
}

/// A click at column `x` of row `row`: on its `▸`/`▾` it expands or collapses, a second one on
/// the same entry within [`DOUBLE_CLICK`] opens it, and any other selects it.
fn click_row(app: &mut App, row: usize, x: u16, now: Instant) -> Vec<Effect> {
    if on_glyph(app, row, x) {
        app.mouse.last_click = None;
        crate::state::tree::act::toggle_row(&mut app.session, row);
        return Vec::new();
    }
    let node = app.session.snap.entries[app.session.list.rows[row]].node;
    let again = app
        .mouse
        .last_click
        .is_some_and(|(t, n)| n == node && now.duration_since(t) <= DOUBLE_CLICK);
    if again {
        app.mouse.last_click = None;
        // The selection may have moved since the first click; open the row clicked.
        app.session.move_to(row);
        return keys::run(app, Action::Enter);
    }
    app.mouse.last_click = Some((now, node));
    app.session.move_to(row);
    Vec::new()
}

/// Whether column `x` is on the `▸`/`▾` of row `row` (or the blank after it), in tree mode:
/// after the row's guide, two cells per level below the top.
fn on_glyph(app: &App, row: usize, x: u16) -> bool {
    if app.tree_shown().is_none() {
        return false;
    }
    let e = &app.session.snap.entries[app.session.list.rows[row]];
    if !e.is_dir || e.pseudo {
        return false;
    }
    let at = app.mouse.name_x + 2 * u16::from(e.depth);
    (at..at + 2).contains(&x)
}

#[cfg(test)]
mod tests;
