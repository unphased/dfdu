//! The lines below the list: the prompt about directories that do not respond, the status line
//! and the key hints.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{marks, spread};
use crate::app::keys::{Action, first_key};
use crate::app::views::{self, View};
use crate::app::{App, Overlay};
use crate::features::details;
use crate::model::snapshot::Mark;
use crate::state::filter;

/// Drawn only while directories are stalled (the row has no height otherwise).
pub fn stalled(f: &mut Frame, app: &App, area: Rect) {
    let n = app.session.snap.stalled;
    let text = format!(
        " {n} director{} not responding: wait, or press x to skip {}",
        if n == 1 { "y is" } else { "ies are" },
        if n == 1 { "it" } else { "them" }
    );
    f.render_widget(
        Paragraph::new(Line::styled(text, app.theme.status_warn)),
        area,
    );
}

/// Delete progress, else the last status message or engine notice; what is marked on the right.
/// Without any of those, the list view shows the selection line there, unless an overlay is
/// open over it.
pub fn status(f: &mut Frame, app: &App, area: Rect, list_view: bool) {
    let msg = match app.session.snap.deleting {
        Some((done, total)) => format!("deleting {done}/{total}…"),
        None if app.session.snap.undoing => "putting back…".into(),
        None => app
            .session
            .status
            .clone()
            .or_else(|| app.session.snap.notice.clone())
            .unwrap_or_default(),
    };
    #[cfg(unix)]
    let msg = app.peer.as_ref().map_or_else(
        || msg.clone(),
        |p| {
            let mut peer = app
                .peer_background
                .status_text(p.peer.id(), &p.peer.status());
            if !app.nav.footer.is_empty() {
                peer.push_str(&format!(" | {}", app.nav.footer));
            }
            if msg.is_empty() {
                peer
            } else {
                format!("{peer} | {msg}")
            }
        },
    );
    if msg.is_empty()
        && list_view
        && matches!(app.overlay, Overlay::None)
        && !crate::overlay::confirm::is_open(app)
        && let Some(line) = super::selection::line(app, area.width)
    {
        f.render_widget(Paragraph::new(line), area);
        return;
    }
    let line = match marks::summary(app) {
        Some(marked) => spread(
            vec![Span::raw(format!(" {msg}"))],
            vec![marked, Span::raw(" ")],
            area.width,
        )
        .style(app.theme.fg_dim),
        None => Line::styled(format!(" {msg}"), app.theme.fg_dim),
    };
    f.render_widget(Paragraph::new(line), area);
}

/// At most this many hints on the line.
const MAX_HINTS: usize = 12;

/// One key hint: the keys and a short label. When the line is too short, the hints with the
/// highest `drop` go first.
#[derive(Clone, Debug, PartialEq)]
struct Hint {
    keys: String,
    label: &'static str,
    drop: u8,
}

fn hint(keys: impl Into<String>, label: &'static str, drop: u8) -> Hint {
    Hint {
        keys: keys.into(),
        label,
        drop,
    }
}

/// The hints for what the keys do now, in the order they are shown; in a read-only browser
/// without the keys that delete.
fn context(app: &App) -> Vec<Hint> {
    let mut hints = offered_now(app);
    if app.session.read_only {
        let deleting = [Action::Trash, Action::DeletePermanent, Action::Undo].map(first_key);
        hints.retain(|h| !deleting.contains(&h.keys.as_str()));
    }
    hints
}

/// The hints for what the keys do now. Whatever takes the keys first decides, as in
/// [`App::key`]: the open overlay, the filter input, the marks pane, the focused details pane,
/// or else the list (with what marks, a running scan and errors add) or the tree.
fn offered_now(app: &App) -> Vec<Hint> {
    let arrows = format!("{}{}", first_key(Action::Up), first_key(Action::Down));
    let help = hint(first_key(Action::Help), "help", 0);
    let quit = hint(first_key(Action::Quit), "quit", 1);
    let views = hint(
        format!(
            "{} {}",
            first_key(Action::PrevView),
            first_key(Action::NextView)
        ),
        "views",
        3,
    );
    if crate::overlay::confirm::is_open(app) {
        return vec![
            hint("p", "permanent", 0),
            hint("s", "stage", 1),
            hint("Esc", "cancel", 0),
        ];
    }
    if let Some(hints) = overlay_hints(&app.overlay, &arrows) {
        return hints;
    }
    if filter::editing(app) {
        return editing(&arrows, "move");
    }
    if crate::features::marks_pane::is_open(app) {
        return vec![
            hint(first_key(Action::Close), "back", 2),
            hint(arrows, "move", 2),
            hint(first_key(Action::ToggleMark), "unmark", 1),
            hint(
                crate::features::marks_pane::INVERT_KEY.to_string(),
                "invert here",
                4,
            ),
            hint(first_key(Action::Trash), "trash", 0),
            hint(first_key(Action::DeletePermanent), "delete", 3),
            help,
            quit,
        ];
    }
    if details::focused(app) {
        return vec![
            hint(first_key(Action::Close), "back", 2),
            hint(first_key(Action::FocusDetails), "marks", 3),
            hint(arrows, "scroll", 2),
            hint(first_key(Action::Details), "close", 3),
            help,
            quit,
        ];
    }
    if let Some(actions) = view_actions(app, &arrows) {
        return in_view(actions, help, quit, views);
    }
    if let Some(tree) = app.tree_shown() {
        let fold = crate::state::tree::act::fold_label(tree.fold_ratio).unwrap_or("no fold");
        return vec![
            hint(arrows, "move", 4),
            hint(first_key(Action::Open), "expand", 1),
            hint(first_key(Action::Parent), "collapse", 2),
            hint(first_key(Action::ExpandChain), "expand largest", 3),
            hint(first_key(Action::Enter), "open", 2),
            hint(first_key(Action::CycleFold), fold, 5),
            hint(first_key(Action::ToggleTree), "list", 3),
            help,
            Hint { drop: 6, ..quit },
            Hint { drop: 7, ..views },
        ];
    }
    list_hints(app, arrows, help, quit, views)
}

/// The actions of the view shown, as in [`App::key`]; `None` for the list, also where the
/// map is drawn as the list.
fn view_actions(app: &App, arrows: &str) -> Option<Vec<Hint>> {
    Some(match views::current(app) {
        View::Largest => vec![
            hint(arrows, "move", 1),
            hint("Enter", "show in Files", 2),
            hint(first_key(Action::ToggleMark), "tick", 1),
            hint(first_key(Action::ClearMarks), "untick all", 3),
            hint(first_key(Action::CopyPath), "copy path", 4),
            hint(first_key(Action::OpenPath), "open", 5),
            hint(first_key(Action::Trash), "delete selected", 0),
        ],
        View::Duplicates if !crate::features::dupes::compared(app) => {
            let mut actions = vec![hint(first_key(Action::Close), "back to Files", 1)];
            // Only this machine's files are compared.
            if app.session.outside.local {
                actions.insert(0, hint(first_key(Action::Enter), "compare now", 0));
            }
            actions
        }
        View::Duplicates => vec![
            hint(arrows, "move", 1),
            hint(first_key(Action::ToggleMark), "tick", 1),
            hint(first_key(Action::TickSuggested), "tick suggested", 2),
            hint(first_key(Action::ClearMarks), "untick all", 4),
            hint(first_key(Action::DupesKeep), "keep rule", 4),
            hint("Enter", "show in Files", 3),
            hint(first_key(Action::CompareAgain), "compare again", 3),
            hint(first_key(Action::Trash), "delete selected", 0),
        ],
        View::Cleanup => vec![
            hint(arrows, "move", 1),
            hint("Enter", crate::features::cleanup::enter_does(app), 2),
            hint(first_key(Action::ToggleMark), "tick", 1),
            hint(first_key(Action::TickSuggested), "tick suggested", 3),
            hint(first_key(Action::ClearMarks), "untick all", 4),
            hint(first_key(Action::CleanupFilter), "filter", 4),
            hint(first_key(Action::CleanupCommand), "command", 4),
            hint(first_key(Action::Trash), "delete selected", 0),
        ],
        View::Map if crate::features::treemap::active(app) => vec![
            hint("←↑↓→", "move", 1),
            hint("Enter", "zoom in", 2),
            hint("Backspace", "zoom out", 3),
            hint("+ -", "depth", 4),
            hint(first_key(Action::TreemapColor), "colour", 4),
            hint(first_key(Action::ToggleMark), "mark", 5),
            hint(first_key(Action::Trash), "trash", 5),
        ],
        View::Map | View::Files => return None,
    })
}

/// The hints of a view (Map, Cleanup, Duplicates, Largest): its own actions keep their place
/// first, then `[ ] views`, then `? help`, and `q quit` goes first when the line is short.
fn in_view(mut actions: Vec<Hint>, help: Hint, quit: Hint, views: Hint) -> Vec<Hint> {
    let last = actions.iter().map(|h| h.drop).max().unwrap_or(0);
    actions.extend([
        Hint {
            drop: last + 2,
            ..help
        },
        Hint {
            drop: last + 3,
            ..quit
        },
        Hint {
            drop: last + 1,
            ..views
        },
    ]);
    actions
}

/// The hints of a filter or search being typed, where the arrows do `arrows_do`.
fn editing(arrows: &str, arrows_do: &'static str) -> Vec<Hint> {
    vec![
        hint("Enter", "keep", 0),
        hint("Esc", "clear", 0),
        hint(arrows, arrows_do, 1),
    ]
}

/// The hints of the open overlay; `None` when none is open.
fn overlay_hints(overlay: &Overlay, arrows: &str) -> Option<Vec<Hint>> {
    Some(match overlay {
        Overlay::None => return None,
        Overlay::Help(h) if h.searching() => editing(arrows, "scroll"),
        Overlay::Help(_) => {
            vec![
                hint(arrows, "scroll", 1),
                hint("PgUp PgDn", "page", 2),
                hint("/", "search", 1),
                hint("Esc", "close", 0),
            ]
        }
        Overlay::Palette(_) => {
            vec![
                hint(arrows, "move", 1),
                hint("Enter", "run", 0),
                hint("Esc", "close", 0),
            ]
        }
        Overlay::Errors { .. } => {
            vec![
                hint(arrows, "move", 1),
                hint("Enter", "go to it", 0),
                hint("Esc", "close", 0),
            ]
        }
        Overlay::Types(_) => {
            vec![
                hint(arrows, "move", 1),
                hint("Space", "toggle", 0),
                hint("Enter", "done", 0),
                hint("Esc", "close", 0),
            ]
        }
        Overlay::MarkPattern(_) => {
            vec![hint("Enter", "mark", 0), hint("Esc", "cancel", 0)]
        }
        Overlay::Roots(_) => vec![
            hint(arrows, "move", 1),
            hint("Enter", "open", 0),
            hint("q", "quit", 0),
        ],
        Overlay::GoTo(_) => {
            vec![
                hint("Tab", "complete", 0),
                hint("Enter", "go", 0),
                hint("Esc", "cancel", 0),
            ]
        }
    })
}

/// The hints of the list, with what marks, a running scan and errors add.
fn list_hints(app: &App, arrows: String, help: Hint, quit: Hint, views: Hint) -> Vec<Hint> {
    let sort = [
        Action::SortSize,
        Action::SortName,
        Action::SortItems,
        Action::SortMtime,
    ]
    .map(first_key)
    .join(" ");
    let mut hints = vec![
        hint(arrows, "move", 2),
        hint(first_key(Action::Open), "open", 3),
    ];
    hints.push(hint(first_key(Action::Parent), "up", 4));
    if app
        .session
        .selected_entry()
        .is_some_and(|e| matches!(e.mark, Mark::OtherFs | Mark::Network))
    {
        hints.push(hint(first_key(Action::EnterMount), "scan it", 1));
    }
    if !app.session.marks.is_empty() {
        hints.push(hint(first_key(Action::ToggleMark), "mark", 2));
        hints.push(hint(first_key(Action::Trash), "trash marked", 2));
        hints.push(hint(first_key(Action::DeletePermanent), "delete marked", 3));
    }
    hints.push(hint(sort, "sort", 7));
    if app.session.snap.busy {
        hints.push(hint(first_key(Action::Resort), "re-sort", 8));
    }
    if app.session.marks.is_empty() {
        hints.push(hint(first_key(Action::ToggleMark), "mark", 6));
    }
    hints.push(hint(first_key(Action::Filter), "filter", 5));
    hints.push(hint(first_key(Action::Details), "details", 10));
    hints.push(hint(first_key(Action::Palette), "commands", 9));
    if app.session.snap.root_total.errors > 0 {
        hints.push(hint(first_key(Action::Errors), "errors", 4));
    }
    hints.push(help);
    hints.push(quit);
    hints.push(Hint { drop: 9, ..views });
    hints
}

/// The hints of `hints` that fit on `width` cells after the leading space, at most
/// [`MAX_HINTS`], whole and in their order; the ones to drop first make room for the others.
fn fit(hints: &[Hint], width: usize) -> Vec<&Hint> {
    let mut order: Vec<usize> = (0..hints.len()).collect();
    order.sort_by_key(|&i| hints[i].drop);
    let mut keep = vec![false; hints.len()];
    let (mut used, mut kept) = (1, 0);
    for i in order {
        let h = &hints[i];
        let w = Span::raw(&h.keys).width() + 1 + Span::raw(h.label).width();
        let gap = if kept == 0 { 0 } else { 2 };
        if kept < MAX_HINTS && used + gap + w <= width {
            keep[i] = true;
            used += gap + w;
            kept += 1;
        }
    }
    hints
        .iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(h, _)| h)
        .collect()
}

/// The hints line for `width` cells.
fn line(app: &App, width: u16) -> Line<'static> {
    let t = &app.theme;
    let hints = context(app);
    let mut spans = vec![Span::raw(" ")];
    for (i, h) in fit(&hints, usize::from(width)).into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(h.keys.clone(), t.key));
        spans.push(Span::styled(format!(" {}", h.label), t.fg_dim));
    }
    Line::from(spans)
}

/// The most used keys for the current context, as many as fit.
pub fn hints(f: &mut Frame, app: &App, area: Rect) {
    f.render_widget(Paragraph::new(line(app, area.width)), area);
}

#[cfg(test)]
mod tests;
