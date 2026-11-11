//! Decisions for terminal image previews. The private runtime decodes off-thread and writes
//! graphics after the UI frame.

use std::path::{Path, PathBuf};

use ratatui::layout::Rect;

pub(crate) mod runtime;

/// Intersects a requested placement with the available terminal area.
pub fn clip(requested: Rect, screen: Rect) -> Option<Rect> {
    let left = u32::from(requested.x).max(u32::from(screen.x));
    let top = u32::from(requested.y).max(u32::from(screen.y));
    let right = (u32::from(requested.x) + u32::from(requested.width))
        .min(u32::from(screen.x) + u32::from(screen.width));
    let bottom = (u32::from(requested.y) + u32::from(requested.height))
        .min(u32::from(screen.y) + u32::from(screen.height));
    (right > left && bottom > top).then(|| {
        Rect::new(
            left as u16,
            top as u16,
            (right - left) as u16,
            (bottom - top) as u16,
        )
    })
}

/// Recognizes a kitty terminal, excluding multiplexers without verified passthrough.
pub fn supported_terminal(
    term: Option<&str>,
    kitty_window_id: Option<&str>,
    multiplexed: bool,
) -> bool {
    !multiplexed
        && (term == Some("xterm-kitty") || kitty_window_id.is_some_and(|id| !id.is_empty()))
}

/// Checks the filename extension, not the contents.
pub fn is_image(path: &Path) -> bool {
    jpg_preview::is_supported_image(path)
}

/// A selected image and its placement in terminal cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    path: PathBuf,
    area: Rect,
}

impl Request {
    pub fn new(path: PathBuf, area: Rect) -> Self {
        Self { path, area }
    }
}

/// What the caller should do after a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Render,
    Delete,
}

/// Tracks the current selection and invalidates work from previous selections.
#[derive(Default)]
pub struct PreviewState {
    current: Option<Request>,
    generation: u64,
}

impl PreviewState {
    pub fn select(&mut self, request: Option<Request>) -> Action {
        if self.current != request {
            self.generation = self
                .generation
                .checked_add(1)
                .expect("preview generation exhausted");
            let had_preview = self.current.is_some();
            self.current = request;
            if self.current.is_some() {
                Action::Render
            } else if had_preview {
                Action::Delete
            } else {
                Action::None
            }
        } else if self.current.is_some() {
            Action::Render
        } else {
            Action::None
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_current(&self, generation: u64, request: &Request) -> bool {
        self.generation == generation && self.current.as_ref() == Some(request)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ratatui::layout::Rect;

    use super::{Action, PreviewState, Request, clip, is_image, supported_terminal};

    #[test]
    fn clips_to_terminal_and_rejects_empty_or_offscreen() {
        let screen = Rect::new(0, 0, 80, 24);
        assert_eq!(
            clip(Rect::new(70, 20, 20, 10), screen),
            Some(Rect::new(70, 20, 10, 4))
        );
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
