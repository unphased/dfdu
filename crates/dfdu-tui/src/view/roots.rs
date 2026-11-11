//! The level above the scanned directory trees.

use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{
    App, Effect, Overlay,
    keys::{self, Action},
};

pub fn key(app: &mut App, k: KeyEvent) -> Vec<Effect> {
    let Overlay::Roots(selected) = app.overlay else {
        return Vec::new();
    };
    let len = app.session.snap.roots.len();
    match keys::action_for(&k) {
        Some(Action::Up) => app.overlay = Overlay::Roots(selected.saturating_sub(1)),
        Some(Action::Down) => {
            app.overlay = Overlay::Roots((selected + 1).min(len.saturating_sub(1)))
        }
        Some(Action::Open | Action::Enter) => {
            if let Some(path) = app.session.snap.roots.get(selected).cloned() {
                let moved = app.session.open_dir(path);
                app.overlay = Overlay::None;
                if let Some(moved) = moved {
                    app.moved(moved);
                }
            }
        }
        Some(Action::Quit) => app.quit = true,
        _ => {}
    }
    Vec::new()
}

pub fn draw(f: &mut Frame, app: &App, area: Rect) -> Rect {
    let Overlay::Roots(selected) = app.overlay else {
        return area;
    };
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(" Scanned roots ")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let height = usize::from(inner.height);
    let start = selected.saturating_add(1).saturating_sub(height);
    let lines: Vec<Line<'static>> = app
        .session
        .snap
        .roots
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, path)| {
            let text: String = path
                .display()
                .chars()
                .flat_map(|c| {
                    if c.is_control() {
                        c.escape_default().collect::<String>()
                    } else {
                        c.to_string()
                    }
                    .chars()
                    .collect::<Vec<_>>()
                })
                .collect();
            let line = Line::from(format!("{} {text}", if i == selected { ">" } else { " " }));
            if i == selected {
                line.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                line
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
    area
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        tests::{press, snap_without_totals},
        theme::Theme,
    };
    use ratatui::crossterm::event::KeyCode;
    use std::time::Instant;

    #[test]
    fn multi_root_navigation_and_single_root_unchanged() {
        let now = Instant::now();
        let mut app = App::new(Theme::no_color(), now);
        let mut snap = snap_without_totals(vec![], 1);
        snap.roots = vec!["/one".into(), "/two".into()];
        snap.root_path = "/one".into();
        snap.dir_path = "/one".into();
        snap.at_root = true;
        app.refresh(snap.clone(), now);
        assert!(matches!(app.overlay, Overlay::Roots(0)));
        app.key(press(KeyCode::Down), now);
        app.key(press(KeyCode::Enter), now);
        assert!(matches!(app.overlay, Overlay::None));
        assert_eq!(app.session.target.path, snap.roots[1]);
        snap.dir_path = snap.roots[1].clone();
        snap.root_path = snap.roots[1].clone();
        app.refresh(snap, now);
        app.key(press(KeyCode::Left), now);
        assert!(matches!(app.overlay, Overlay::Roots(1)));

        app.overlay = Overlay::None;
        crate::overlay::palette::open(&mut app);
        for c in "parent directory".chars() {
            app.key(press(KeyCode::Char(c)), now);
        }
        app.key(press(KeyCode::Enter), now);
        assert!(matches!(app.overlay, Overlay::Roots(1)));

        let mut single = App::new(Theme::no_color(), now);
        let mut snap = snap_without_totals(vec![], 1);
        snap.roots = vec!["/one".into()];
        single.refresh(snap, now);
        assert!(matches!(single.overlay, Overlay::None));
    }
}
