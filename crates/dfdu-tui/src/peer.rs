//! Peer pairing controls. Transport work belongs to dfdu-peer's workers.

use crate::app::App;
use crate::input::{
    line::{Edit, LineInput},
    line_keys,
};
use ratatui::crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Clear, Paragraph},
};
use std::sync::Arc;

/// Peer transport shared across browser reopen operations.
#[derive(Clone)]
pub struct PeerOptions {
    pub peer: Arc<dfdu_peer::Peer>,
    pub remote_bin: String,
}

impl std::fmt::Debug for PeerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerOptions")
            .field("id", &self.peer.id())
            .field("remote_bin", &self.remote_bin)
            .finish()
    }
}

pub(crate) fn status_text(id: &str, status: &dfdu_peer::Status) -> String {
    let role = match status.role {
        dfdu_peer::Role::Unbound => "unbound",
        dfdu_peer::Role::Master => "master",
        dfdu_peer::Role::Slave => "slave",
    };
    format!(
        "peer {id} {role}{}",
        status
            .peer
            .as_ref()
            .map_or_else(String::new, |p| format!(" → {p}"))
    )
}

pub(crate) fn open(app: &mut App) {
    let Some(options) = &app.peer else {
        app.session.status = Some("peer mode is not enabled".into());
        return;
    };
    if options.peer.status().role == dfdu_peer::Role::Unbound {
        app.peer_prompt = Some(LineInput::default());
    } else {
        options.peer.unbind();
    }
}

pub(crate) fn key(app: &mut App, k: KeyEvent) -> bool {
    let Some(prompt) = &mut app.peer_prompt else {
        return false;
    };
    match line_keys::edit(prompt, &k) {
        Edit::Cancel => app.peer_prompt = None,
        Edit::Submit => {
            let target = prompt.text().trim().to_owned();
            if target.is_empty() {
                return true;
            }
            app.peer_prompt = None;
            if let Some(options) = &app.peer
                && let Err(e) = options.peer.bind(&target, &options.remote_bin)
            {
                app.session.status = Some(format!("peer: {e}"));
            }
        }
        _ => {}
    }
    true
}

pub(crate) fn draw(f: &mut Frame, app: &App) -> Option<Rect> {
    let prompt = app.peer_prompt.as_ref()?;
    let area = f.area();
    let width = area.width.min(64);
    let height = area.height.min(5);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(format!(
            "{}\nID / HOST ID / HOST:ID\nEnter bind · Esc cancel",
            prompt.text()
        ))
        .block(Block::bordered().title(" TARGET "))
        .style(app.theme.fg_dim),
        rect,
    );
    if width > 2 && height > 2 {
        let cursor = unicode_width::UnicodeWidthStr::width(
            prompt
                .text()
                .chars()
                .take(prompt.cursor())
                .collect::<String>()
                .as_str(),
        );
        let cursor = u16::try_from(cursor).unwrap_or(u16::MAX).min(width - 3);
        f.set_cursor_position((rect.x + 1 + cursor, rect.y + 1));
    }
    Some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::theme::Theme;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::Instant;

    #[test]
    fn peer_key_is_registered_and_unused_elsewhere() {
        assert_eq!(
            crate::app::keys::first_key(crate::app::keys::Action::Peer),
            "B"
        );
        assert_eq!(
            crate::app::keys::action_for(&KeyEvent::new(KeyCode::Char('B'), KeyModifiers::NONE)),
            Some(crate::app::keys::Action::Peer)
        );
    }

    #[test]
    fn target_prompt_edits_cancels_and_keeps_spaces() {
        let mut app = App::new(
            Theme::builtin(
                crate::theme::BuiltinTheme::Dark,
                crate::theme::ColorLevel::TrueColor,
                None,
            ),
            Instant::now(),
        );
        app.peer_prompt = Some(crate::input::line::LineInput::default());
        for c in "host id".chars() {
            assert!(key(
                &mut app,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
            ));
        }
        assert_eq!(app.peer_prompt.as_ref().unwrap().text(), "host id");
        assert!(key(
            &mut app,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert!(app.peer_prompt.is_none());
    }

    fn app() -> App {
        App::new(
            Theme::builtin(
                crate::theme::BuiltinTheme::Dark,
                crate::theme::ColorLevel::TrueColor,
                None,
            ),
            Instant::now(),
        )
    }

    fn wait_role(peer: &dfdu_peer::Peer, role: dfdu_peer::Role) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if peer.status().role == role {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "peer did not become {role:?}");
            peer.events().recv_timeout(remaining).unwrap();
        }
    }

    #[test]
    fn keyboard_target_binds_two_real_peers_and_same_key_sends_bye() {
        let master = Arc::new(dfdu_peer::Peer::start(Vec::new()).unwrap());
        let slave = dfdu_peer::Peer::start(Vec::new()).unwrap();
        let mut app = app();
        app.peer = Some(PeerOptions {
            peer: master.clone(),
            remote_bin: dfdu_peer::DEFAULT_REMOTE_BIN.into(),
        });
        let b = KeyEvent::new(KeyCode::Char('B'), KeyModifiers::NONE);
        app.key(b, Instant::now());
        assert!(app.peer_prompt.is_some());
        app.peer_prompt.as_mut().unwrap().set(slave.id());
        app.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            Instant::now(),
        );
        assert!(app.peer_prompt.is_none());
        wait_role(&master, dfdu_peer::Role::Master);
        wait_role(&slave, dfdu_peer::Role::Slave);
        app.key(b, Instant::now());
        wait_role(&master, dfdu_peer::Role::Unbound);
        wait_role(&slave, dfdu_peer::Role::Unbound);
        assert!(app.peer_prompt.is_none());
    }

    #[test]
    fn prompt_and_persistent_status_render_even_with_notice() {
        let peer = Arc::new(dfdu_peer::Peer::start(Vec::new()).unwrap());
        let mut app = app();
        app.peer = Some(PeerOptions {
            peer: peer.clone(),
            remote_bin: String::new(),
        });
        app.session.status = Some("other notice".into());
        open(&mut app);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|f| crate::view::draw(f, &mut app, Instant::now()))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("TARGET"));
        assert!(text.contains("ID / HOST ID / HOST:ID"));
        assert!(text.contains(&format!("peer {} unbound", peer.id())));
        assert!(text.contains("other notice"));
        assert!(app.areas.overlay.is_some());
    }

    #[test]
    fn disabled_peer_key_reports_notice_without_prompt() {
        let mut app = app();
        open(&mut app);
        assert!(app.peer_prompt.is_none());
        assert_eq!(
            app.session.status.as_deref(),
            Some("peer mode is not enabled")
        );
    }

    #[test]
    fn roles_show_identity_and_partner() {
        for (role, name) in [
            (dfdu_peer::Role::Unbound, "unbound"),
            (dfdu_peer::Role::Master, "master"),
            (dfdu_peer::Role::Slave, "slave"),
        ] {
            let text = status_text(
                "abc",
                &dfdu_peer::Status {
                    role,
                    peer: Some("xyz".into()),
                },
            );
            assert!(text.contains(name));
            assert!(text.contains("abc"));
            assert!(text.contains("xyz"));
        }
    }
}
