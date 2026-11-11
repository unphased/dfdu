#[cfg(test)]
mod tests {
    use std::path::Path;

    use ratatui::layout::Rect;

    use super::{Action, PreviewState, Request, clip, is_image, supported_terminal};

    #[test]
    fn clips_to_terminal_and_rejects_empty_or_offscreen() {
        let screen = Rect::new(0, 0, 80, 24);
        assert_eq!(clip(Rect::new(70, 20, 20, 10), screen), Some(Rect::new(70, 20, 10, 4)));
        assert_eq!(clip(Rect::new(80, 0, 1, 1), screen), None);
        assert_eq!(clip(Rect::new(0, 0, 0, 1), screen), None);
        assert_eq!(clip(Rect::new(u16::MAX, 0, 2, 1), screen), None);
    }

    #[test]
    fn capability_is_explicit_and_conservative() {
        assert!(supported_terminal(Some("xterm-kitty"), None, false));
        assert!(supported_terminal(Some("xterm-256color"), Some("1"), false));
        assert!(!supported_terminal(Some("xterm-256color"), None, false));
        assert!(!supported_terminal(Some("xterm-kitty"), None, true));
        assert!(!supported_terminal(None, Some(""), false));
    }

    #[test]
    fn detects_supported_extensions() {
        for name in ["a.JPG", "a.jpeg", "a.PnG", "a.ARW"] {
            assert!(is_image(Path::new(name)), "{name}");
        }
        assert!(!is_image(Path::new("a.gif")));
        assert!(!is_image(Path::new("jpg")));
    }

    #[test]
    fn state_redraws_and_deletes_and_rejects_stale_results() {
        let mut state = PreviewState::default();
        let area = Rect::new(1, 2, 10, 5);
        let first = Request::new("a.jpg".into(), area);
        let second = Request::new("b.png".into(), area);
        assert_eq!(state.select(Some(first.clone())), Action::Render);
        let token = state.generation();
        assert!(state.is_current(token, &first));
        assert_eq!(state.select(Some(first.clone())), Action::Render);
        assert_eq!(state.select(Some(second.clone())), Action::Render);
        assert!(!state.is_current(token, &first));
        let token = state.generation();
        assert_eq!(state.select(None), Action::Delete);
        assert!(!state.is_current(token, &second));
        assert_eq!(state.select(None), Action::None);
        state.select(Some(first.clone()));
        assert!(!state.is_current(1, &first));
    }
}
