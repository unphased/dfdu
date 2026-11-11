//! Peer pairing controls. Transport work belongs to dfdu-peer's workers.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::theme::Theme;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::Instant;

    #[test]
    fn peer_key_is_registered_and_unused_elsewhere() {
        assert_eq!(crate::app::keys::first_key(crate::app::keys::Action::Peer), "B");
        assert_eq!(crate::app::keys::action_for(&KeyEvent::new(KeyCode::Char('B'), KeyModifiers::NONE)), Some(crate::app::keys::Action::Peer));
    }

    #[test]
    fn target_prompt_edits_cancels_and_keeps_spaces() {
        let mut app = App::new(Theme::default(), Instant::now());
        app.peer_prompt = Some(crate::input::line::LineInput::default());
        for c in "host id".chars() {
            assert!(key(&mut app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
        }
        assert_eq!(app.peer_prompt.as_ref().unwrap().text(), "host id");
        assert!(key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.peer_prompt.is_none());
    }

    #[test]
    fn roles_show_identity_and_partner() {
        for (role, name) in [(dfdu_peer::Role::Unbound, "unbound"), (dfdu_peer::Role::Master, "master"), (dfdu_peer::Role::Slave, "slave")] {
            let text = status_text("abc", &dfdu_peer::Status { role, peer: Some("xyz".into()) });
            assert!(text.contains(name));
            assert!(text.contains("abc"));
            assert!(text.contains("xyz"));
        }
    }
}
