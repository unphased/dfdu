//! Drawing. Works only from [`App`] and its snapshot, never from the engine model, so it runs
//! after the read lock is released. [`draw`] lays out the screen, draws the header, the list
//! or the treemap, and the footer (modules here, the treemap in its own), the details pane or the marks pane in its place, the debug panel and the open overlay (each in its
//! feature's module), and last lets motion effects work on the finished buffer. `empty` draws
//! the box a view shows when it has nothing to show; `filter` and `marks` draw what those keep
//! in `state`, and [`input_spans`] draws a text input.

pub(crate) mod columns;
pub(crate) mod empty;
pub(crate) mod filter;
mod footer;
mod header;
mod list;
mod marks;
pub(crate) mod roots;
mod selection;

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::views::{self, View};
use crate::app::{App, Overlay};
use crate::features::{debug, details, treemap};
use crate::input::line::LineInput;
use crate::overlay::{confirm, error_list, goto, help, mark_pattern, palette, type_filter};
use crate::state::motion;

/// Where the last frame drew each part of the screen, for mapping mouse positions.
#[derive(Copy, Clone, Debug, Default)]
pub struct Areas {
    pub list: Rect,
    /// First cell after the selected row's rendered name and notes, with a one-cell gap.
    pub preview_anchor: Option<(u16, u16)>,
    pub details: Option<Rect>,
    /// The open overlay.
    pub overlay: Option<Rect>,
    /// The debug panel, over the list.
    pub debug: Option<Rect>,
}

/// A number of rows or columns as a terminal length, at most `u16::MAX`.
pub fn cells(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

fn mmss(d: std::time::Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// Draws the whole screen and updates the list height, scroll offset and areas in `app`.
pub fn draw(f: &mut Frame, app: &mut App, now: Instant) {
    let area = f.area();
    let stalled_line = u16::from(app.session.snap.stalled > 0);
    let filter_line = filter::line(app);
    let [header, body, prompt, filter_row, status, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(stalled_line),
        Constraint::Length(u16::from(filter_line.is_some())),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    // The list view has its selection line, and a row naming its columns above the rows if a
    // row is left below it.
    let view = views::current(app);
    let list_view = match view {
        View::Files => true,
        View::Map => !treemap::fits(area),
        View::Cleanup | View::Duplicates | View::Largest => false,
    };
    let (list, pane) = details::split(app, body, list_view);
    let names = u16::from(list_view && list.height >= 2);
    let [column_names, list] =
        Layout::vertical([Constraint::Length(names), Constraint::Min(0)]).areas(list);
    app.areas = Areas {
        list,
        details: pane,
        preview_anchor: None,
        overlay: None,
        debug: None,
    };
    let frame = u64::try_from(now.duration_since(app.session.started).as_millis() / 100)
        .unwrap_or(u64::MAX);

    // The header names the treemap's colouring and depth while it is shown.
    let map = view == View::Map && treemap::fits(area);
    app.session.set_map_drawn(map);
    header::draw(f, app, header, now, frame);
    match view {
        View::Largest => crate::features::largest::draw(f, app, list),
        View::Duplicates => crate::features::dupes::draw(f, app, list),
        View::Cleanup => crate::features::cleanup::draw(f, app, list),
        View::Map if map => treemap::draw(f, app, list, now),
        View::Map | View::Files => list::draw(f, app, column_names, list, now, frame),
    }
    if stalled_line > 0 {
        footer::stalled(f, app, prompt);
    }
    if let Some(line) = filter_line {
        f.render_widget(Paragraph::new(line), filter_row);
    }
    footer::status(f, app, status, list_view);
    footer::hints(f, app, footer);

    if let Some(pane) = pane {
        if crate::features::marks_pane::is_open(app) {
            crate::features::marks_pane::draw(f, app, pane);
        } else {
            details::draw(f, app, pane);
        }
    }
    app.areas.debug = debug::draw(f, app, body);
    app.areas.overlay = match &app.overlay {
        _ if confirm::is_open(app) => Some(confirm::draw(f, app, area)),
        Overlay::None => None,
        Overlay::Help(_) => Some(help::draw(f, app, area)),
        Overlay::Errors { .. } => Some(error_list::draw(f, app, area)),
        Overlay::Palette { .. } => Some(palette::draw(f, app, area)),
        Overlay::Types(_) => Some(type_filter::draw(f, app, area)),
        Overlay::MarkPattern(_) => Some(mark_pattern::draw(f, app, area)),
        Overlay::GoTo(_) => Some(goto::draw(f, app, area)),
        Overlay::Roots(_) => Some(roots::draw(
            f,
            app,
            Rect {
                height: area.height.saturating_sub(1),
                ..area
            },
        )),
    };
    #[cfg(unix)]
    if let Some(rect) = crate::peer::draw(f, app) {
        app.areas.overlay = Some(rect);
    }
    motion::post_process(app, f.buffer_mut(), now);
}

/// A line with `left` at the start and `right` at the end of `width` cells.
pub(crate) fn spread<'a>(left: Vec<Span<'a>>, right: Vec<Span<'a>>, width: u16) -> Line<'a> {
    let lw: usize = left.iter().map(Span::width).sum();
    let rw: usize = right.iter().map(Span::width).sum();
    let pad = (width as usize).saturating_sub(lw + rw);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(pad)));
    spans.extend(right);
    Line::from(spans)
}

/// The text of `input` in `style` with a `▌` in `cursor_style` at the cursor.
pub(crate) fn input_spans(
    input: &LineInput,
    style: Style,
    cursor_style: Style,
) -> Vec<Span<'static>> {
    let (before, after) = input.split();
    let mut spans = Vec::with_capacity(3);
    if !before.is_empty() {
        spans.push(Span::styled(before.to_string(), style));
    }
    spans.push(Span::styled("▌", cursor_style));
    if !after.is_empty() {
        spans.push(Span::styled(after.to_string(), style));
    }
    spans
}
