//! The command palette (`:`, `Ctrl-P`): every action and setting, searched by name.
//!
//! Each action of the registry is listed with its keys, so the palette also teaches them.
//! Settings without a key (the theme) follow the actions.

use std::cell::Cell;

use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::keys::{self, Action, Group};
use crate::app::{App, Effect, Overlay};
use crate::input::fuzzy::char_pos;
use crate::input::line::{Edit, LineInput};
use crate::input::line_keys;
use crate::theme::Theme;
use crate::view::input_spans;

/// State of the open palette, held in `Overlay::Palette`.
#[derive(Clone, Debug, Default)]
pub struct Palette {
    input: LineInput,
    /// Everything the palette offers, in registry order.
    entries: Vec<Entry>,
    /// The entries that match the input, best first, with the indices of their matched
    /// characters.
    shown: Vec<(usize, Vec<u32>)>,
    /// Index into `shown`.
    cursor: usize,
    /// First entry of `shown` on screen; kept by drawing, so the entries stay in place while
    /// the cursor moves among them.
    scroll: Cell<usize>,
}

#[derive(Clone, Debug)]
struct Entry {
    label: String,
    keys: &'static str,
    group: &'static str,
    run: Run,
}

#[derive(Clone, Debug)]
enum Run {
    Action(Action),
    Theme(String),
}

impl Palette {
    /// Lists the actions the browser offers (less those that delete when it is `read_only`) and
    /// the themes that exist now.
    fn new(read_only: bool) -> Self {
        let actions = keys::offered(read_only)
            .filter(|info| info.action != Action::Palette)
            .map(|info| Entry {
                label: info.label.to_owned(),
                keys: info.keys,
                group: group_name(info.group),
                run: Run::Action(info.action),
            });
        let themes = Theme::builtin_names().iter().map(|&name| Entry {
            label: format!("Theme: {name}"),
            keys: "",
            group: "setting",
            run: Run::Theme(name.to_owned()),
        });
        let mut p = Palette {
            entries: actions.chain(themes).collect(),
            ..Palette::default()
        };
        p.rank();
        p
    }

    /// Matches the entries against the input: by score, ties in registry order
    /// ([`dfdu_session::fuzzy::rank`]).
    fn rank(&mut self) {
        let labels = self.entries.iter().map(|e| e.label.as_str());
        self.shown = dfdu_session::fuzzy::rank(self.input.text(), labels);
        self.cursor = 0;
        self.scroll.set(0);
    }

    /// The first entry on screen when `rows` entries fit, scrolled as little as keeps the
    /// cursor in view.
    fn follow_cursor(&self, rows: usize) -> usize {
        let top = self
            .scroll
            .get()
            .min(self.cursor)
            .max((self.cursor + 1).saturating_sub(rows));
        self.scroll.set(top);
        top
    }
}

fn group_name(g: Group) -> &'static str {
    match g {
        Group::Navigate => "navigate",
        Group::Sort => "sort",
        Group::Display => "display",
        Group::Act => "act",
        Group::Panels => "panels",
        Group::Global => "global",
    }
}

/// Opens the palette with an empty query.
pub fn open(app: &mut App) {
    app.overlay = Overlay::Palette(Palette::new(app.session.read_only));
}

/// Typing searches; `↑`/`↓`/`PgUp`/`PgDn` move, `Enter` closes the palette and runs the entry,
/// `Esc` closes it.
pub fn key(app: &mut App, k: KeyEvent) -> Vec<Effect> {
    let page = super::list_page(app);
    let Overlay::Palette(p) = &mut app.overlay else {
        return Vec::new();
    };
    let last = p.shown.len().saturating_sub(1);
    if let Some(cursor) = super::list_cursor(k.code, p.cursor, last, page) {
        p.cursor = cursor;
        return Vec::new();
    }
    match line_keys::edit(&mut p.input, &k) {
        Edit::Changed => p.rank(),
        Edit::Submit => {
            let run = p
                .shown
                .get(p.cursor)
                .map(|&(i, _)| p.entries[i].run.clone());
            app.overlay = Overlay::None;
            return match run {
                Some(run) => run_entry(app, run),
                None => Vec::new(),
            };
        }
        Edit::Cancel => app.overlay = Overlay::None,
        Edit::Moved | Edit::Ignored => {}
    }
    Vec::new()
}

fn run_entry(app: &mut App, run: Run) -> Vec<Effect> {
    app.session.status = None;
    match run {
        Run::Action(a) => keys::run(app, a),
        Run::Theme(name) => {
            if let Some(theme) = Theme::named(&name, app.color_level, app.background) {
                let bars = app.theme.bars;
                app.theme = theme;
                app.theme.bars = bars;
            }
            Vec::new()
        }
    }
}

/// Draws the palette in `area` and returns the rectangle it covers.
pub fn draw(f: &mut Frame, app: &App, area: Rect) -> Rect {
    let Overlay::Palette(p) = &app.overlay else {
        return Rect::default();
    };
    let t = &app.theme;
    let width = super::width(area, 76);
    let height = super::list_height(area, p.shown.len());
    let rect = area.centered(Constraint::Length(width), Constraint::Length(height));
    let inner = usize::from(width.saturating_sub(2));
    let rows = usize::from(height.saturating_sub(3)).max(1);

    let mut prompt = vec![Span::styled(" :", t.key), Span::raw(" ")];
    prompt.extend(input_spans(&p.input, Style::new(), t.key));
    let mut lines = vec![Line::from(prompt)];
    if p.shown.is_empty() {
        lines.push(Line::styled(" nothing matches", t.fg_dim));
    }
    let top = p.follow_cursor(rows);
    for (row, (i, indices)) in p.shown.iter().enumerate().skip(top).take(rows) {
        let e = &p.entries[*i];
        let mut line = entry_line(app, e, indices, inner);
        if row == p.cursor {
            line = line.style(t.cursor);
        }
        lines.push(line);
    }
    let title = format!(" Commands {}/{} ", p.shown.len(), p.entries.len());
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(title)),
        rect,
    );
    rect
}

/// One entry on `width` cells: the label with its matched characters highlighted on the left,
/// its group and keys on the right. A label that does not fit is cut with `…`.
fn entry_line(app: &App, e: &Entry, indices: &[u32], width: usize) -> Line<'static> {
    let t = &app.theme;
    let mut right = vec![Span::styled(e.group, t.fg_dim)];
    if !e.keys.is_empty() {
        right.push(Span::raw("  "));
        right.push(Span::styled(e.keys, t.key));
    }
    right.push(Span::raw(" "));
    let right_width: usize = right.iter().map(Span::width).sum();
    let room = width.saturating_sub(right_width + 3);
    let len = e.label.chars().count();
    let keep = if len > room {
        room.saturating_sub(1)
    } else {
        len
    };

    let mut spans = vec![Span::raw(" ")];
    let mut next = indices.iter().copied().peekable();
    for (i, ch) in e.label.chars().take(keep).enumerate() {
        let hit = next.next_if_eq(&char_pos(i)).is_some();
        let style = if hit { t.r#match } else { Style::new() };
        match spans.last_mut() {
            Some(last) if last.style == style => {
                last.content.to_mut().push(ch);
            }
            _ => spans.push(Span::styled(ch.to_string(), style)),
        }
    }
    if keep < len {
        spans.push(Span::raw("…"));
    }
    let used: usize = spans.iter().map(Span::width).sum();
    if used + right_width <= width {
        spans.push(Span::raw(" ".repeat(width - used - right_width)));
        spans.extend(right);
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::style::{Color, Modifier};

    use super::*;
    use crate::app::keys::ACTIONS;
    use crate::tests::{library_snap, press, render, render_buffer, style_at, text, typed};
    use crate::theme::ColorLevel;
    use dfdu_engine::SizeMode;

    fn app_with(theme: Theme, t0: Instant) -> App {
        let mut app = App::new(theme, t0);
        app.refresh(library_snap(), t0);
        app
    }

    fn palette(app: &App) -> &Palette {
        match &app.overlay {
            Overlay::Palette(p) => p,
            o => panic!("the palette is not open: {o:?}"),
        }
    }

    fn labels(app: &App) -> Vec<&str> {
        let p = palette(app);
        p.shown
            .iter()
            .map(|&(i, _)| p.entries[i].label.as_str())
            .collect()
    }

    #[test]
    fn palette_lists_every_action_with_its_key() {
        let t0 = Instant::now();
        for open in [
            press(KeyCode::Char(':')),
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
        ] {
            let mut app = app_with(Theme::no_color(), t0);
            app.key(open, t0);
            let p = palette(&app);
            let actions: Vec<Action> = p
                .shown
                .iter()
                .filter_map(|&(i, _)| match p.entries[i].run {
                    Run::Action(a) => Some(a),
                    Run::Theme(_) => None,
                })
                .collect();
            let want: Vec<Action> = ACTIONS
                .iter()
                .map(|info| info.action)
                .filter(|&a| a != Action::Palette)
                .collect();
            assert_eq!(actions, want, "registry order");
            let themes: Vec<&str> = labels(&app)
                .into_iter()
                .filter_map(|l| l.strip_prefix("Theme: "))
                .collect();
            assert_eq!(themes, Theme::builtin_names());
            assert_eq!(p.shown.len(), p.entries.len());
        }
        // Each row shows the keys of its action.
        let mut app = app_with(Theme::no_color(), t0);
        app.key(press(KeyCode::Char(':')), t0);
        let frame = render(&mut app, t0, 100, 68);
        for info in ACTIONS.iter().filter(|i| i.action != Action::Palette) {
            let row = frame
                .lines()
                .find(|l| l.contains(info.label))
                .unwrap_or_else(|| panic!("{} is not shown:\n{frame}", info.label));
            assert!(row.contains(info.keys), "{row}");
        }
        insta::assert_snapshot!("palette_open", frame);
    }

    #[test]
    fn fuzzy_query_ranks_matches() {
        let t0 = Instant::now();
        let mut app = app_with(Theme::no_color(), t0);
        app.key(press(KeyCode::Char(':')), t0);
        typed(&mut app, "srt sz", t0);
        assert_eq!(labels(&app)[0], "sort by size (again: reverse)");
        // Equal scores keep registry order.
        app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL), t0);
        typed(&mut app, "theme", t0);
        let want: Vec<String> = Theme::builtin_names()
            .iter()
            .map(|n| format!("Theme: {n}"))
            .collect();
        assert_eq!(labels(&app)[..want.len()], want);
        typed(&mut app, "zzzz", t0);
        assert_eq!(labels(&app), [] as [&str; 0]);
        let frame = render(&mut app, t0, 80, 20);
        assert!(frame.contains("nothing matches"), "{frame}");
        assert!(app.key(press(KeyCode::Enter), t0).is_empty());
        assert!(matches!(app.overlay, Overlay::None));
    }

    #[test]
    fn enter_runs_the_action() {
        let t0 = Instant::now();
        let mut app = app_with(Theme::no_color(), t0);
        app.key(press(KeyCode::Char(':')), t0);
        typed(&mut app, "apparent", t0);
        assert_eq!(labels(&app)[0], "toggle apparent size / disk usage");
        assert!(app.key(press(KeyCode::Enter), t0).is_empty());
        assert!(matches!(app.overlay, Overlay::None));
        assert_eq!(app.session.size_mode, SizeMode::Apparent);

        // Moving the cursor picks another entry; one that opens an overlay works too.
        app.key(press(KeyCode::Char(':')), t0);
        typed(&mut app, "keys", t0);
        assert_eq!(labels(&app)[0], "keys");
        app.key(press(KeyCode::Enter), t0);
        assert!(matches!(app.overlay, Overlay::Help(_)));
        app.key(press(KeyCode::Esc), t0);

        app.key(press(KeyCode::Char(':')), t0);
        typed(&mut app, "sort by", t0);
        app.key(press(KeyCode::Down), t0);
        assert_eq!(labels(&app)[1], "sort by name (again: reverse)");
        app.key(press(KeyCode::Enter), t0);
        assert_eq!(app.session.sort.key, crate::state::order::SortKey::Name);

        // Rescan returns its effect to the run loop.
        app.key(press(KeyCode::Char(':')), t0);
        typed(&mut app, "rescan", t0);
        let fx = app.key(press(KeyCode::Enter), t0);
        assert!(matches!(fx[..], [Effect::Send(_)]), "{fx:?}");

        // Esc closes without running anything.
        app.key(press(KeyCode::Char(':')), t0);
        typed(&mut app, "quit", t0);
        app.key(press(KeyCode::Esc), t0);
        assert!(matches!(app.overlay, Overlay::None));
        assert!(!app.quit);
    }

    #[test]
    fn theme_setting_switches_theme() {
        let t0 = Instant::now();
        let dark = Theme::named("dark", ColorLevel::TrueColor, None).expect("dark");
        let light = Theme::named("light", ColorLevel::TrueColor, None).expect("light");
        assert_ne!(dark.dir, light.dir);
        let mut app = app_with(dark.clone(), t0);
        let buf = render_buffer(&mut app, t0, 80, 8);
        assert_eq!(style_at(&buf, "Library/").fg, dark.dir.fg);
        app.key(press(KeyCode::Char(':')), t0);
        typed(&mut app, "light", t0);
        assert_eq!(labels(&app)[0], "Theme: light");
        app.key(press(KeyCode::Enter), t0);
        assert_eq!(app.theme, light);
        let buf = render_buffer(&mut app, t0, 80, 8);
        let style = style_at(&buf, "Library/");
        assert_eq!(style.fg, light.dir.fg);
        assert_eq!(style.bg, light.cursor.bg);
        assert_eq!(style_at(&buf, "· disk").fg, light.fg_dim.fg);
        assert_ne!(light.fg_dim.fg, dark.fg_dim.fg);
    }

    #[test]
    fn theme_after_mono_has_colors_again() {
        let t0 = Instant::now();
        let background = Some((0, 0, 0x20));
        let dark = Theme::named("dark", ColorLevel::TrueColor, background).expect("dark");
        let mut app = app_with(dark.clone(), t0);
        for name in ["mono", "dark"] {
            app.key(press(KeyCode::Char(':')), t0);
            typed(&mut app, name, t0);
            assert_eq!(labels(&app)[0], format!("Theme: {name}"));
            app.key(press(KeyCode::Enter), t0);
            assert_eq!(app.theme.name(), name);
        }
        assert_eq!(app.theme, dark);
        let buf = render_buffer(&mut app, t0, 80, 8);
        assert_eq!(style_at(&buf, "Library/").fg, dark.dir.fg);
        assert!(matches!(dark.dir.fg, Some(Color::Rgb(..))), "{dark:?}");
    }

    #[test]
    fn palette_fits_a_small_terminal() {
        let t0 = Instant::now();
        for (w, h) in [(40, 8), (100, 30)] {
            let mut app = app_with(Theme::no_color(), t0);
            app.key(press(KeyCode::Char(':')), t0);
            // The cursor on the last entry is still drawn.
            for _ in 0..palette(&app).entries.len() {
                app.key(press(KeyCode::Down), t0);
            }
            let buf = render_buffer(&mut app, t0, w, h);
            let covered = app.areas.overlay.expect("palette area");
            assert!(covered.width <= w && covered.height <= h, "{covered:?}");
            assert!(covered.height >= 4, "{covered:?}");
            let frame = text(&buf);
            assert!(frame.contains(" : ▌"), "{frame}");
            let cursor = style_at(&buf, "Theme: high-contrast");
            assert!(cursor.add_modifier.contains(Modifier::REVERSED), "{frame}");
            insta::assert_snapshot!(format!("palette_{w}x{h}"), frame);
            // Moving up keeps the entries in place until the cursor reaches the top row.
            let shown = covered.height as usize - 3;
            for _ in 1..shown {
                app.key(press(KeyCode::Up), t0);
            }
            let buf = render_buffer(&mut app, t0, w, h);
            assert_eq!(text(&buf), frame);
            app.key(press(KeyCode::Up), t0);
            let buf = render_buffer(&mut app, t0, w, h);
            assert_ne!(text(&buf), frame, "scrolls up by one");
            // PgUp and PgDn move by the entries on screen.
            let (before, last) = (palette(&app).cursor, palette(&app).shown.len() - 1);
            app.key(press(KeyCode::PageDown), t0);
            assert_eq!(palette(&app).cursor, (before + shown).min(last));
            app.key(press(KeyCode::PageUp), t0);
            assert_eq!(palette(&app).cursor, before);
        }
        // A label too long for the width is cut, and its keys stay visible.
        let mut app = app_with(Theme::no_color(), t0);
        app.key(press(KeyCode::Char(':')), t0);
        typed(&mut app, "shared", t0);
        let frame = render(&mut app, t0, 40, 8);
        let row = frame
            .lines()
            .find(|l| l.contains("toggle disk usage"))
            .unwrap_or_else(|| panic!("{frame}"));
        assert!(row.contains("…") && row.contains("display  u │"), "{frame}");
    }
}
