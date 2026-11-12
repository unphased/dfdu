//! UI state and its update function. [`App`] holds the browser's state shared with other front
//! ends ([`App::session`], a [`SessionState`]) and what only the terminal shows. Keys turn into
//! changes of it plus effects for the run loop to carry out: [`App::key`] passes a key to the
//! open overlay, the filter input or the focused details pane, and otherwise looks it up in the
//! action registry ([`keys`]). [`App::refresh`] folds a new snapshot into the state and applies
//! the row-stability rules. [`views`] switches between the five views.
//! Each feature keeps its state in its own field and module (`list`, `marks` and `filter` in
//! the session; `details`, `debug`, `motion`, `mouse`); what they read from the engine model
//! arrives in [`App::extras`].

mod cursor;
pub(crate) mod keys;
mod refresh;
pub(crate) mod views;

use std::collections::HashMap;
use std::time::Instant;

use dfdu_engine::ErrorList;
use dfdu_session::SessionState;
use dfdu_tree::HostPath;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::features::{debug, details, treemap};
use crate::input::{line_keys, mouse};

use crate::overlay::{confirm, error_list, goto, help, mark_pattern, palette, type_filter};
use crate::state::filter;
use crate::state::motion::Motion;
use crate::theme::{ColorLevel, Rgb, Theme};
use crate::view::Areas;
pub use dfdu_session::state::{IDLE_BEFORE_SORT, MIN_SORT_INTERVAL};

pub use dfdu_session::effect::Effect;

#[derive(Clone, Debug)]
pub enum Overlay {
    None,
    Help(help::Help),
    /// What could not be read, with a cursor.
    Errors {
        list: ErrorList,
        cursor: usize,
    },
    Palette(palette::Palette),
    /// The file type picker.
    Types(type_filter::TypePicker),
    /// The glob that marks the names it matches.
    MarkPattern(mark_pattern::Prompt),
    /// The path of a folder to go to.
    GoTo(goto::Prompt),
    /// The scanned roots, above their individual directory trees.
    Roots(usize),
}

pub struct App {
    #[cfg(unix)]
    pub peer: Option<crate::PeerOptions>,
    #[cfg(unix)]
    pub peer_prompt: Option<crate::input::line::LineInput>,
    #[cfg(unix)]
    pub(crate) peer_panel: Option<crate::peer::Panel>,
    #[cfg(unix)]
    pub(crate) peer_background: crate::peer::Background,
    #[cfg(unix)]
    pub(crate) nav: crate::peer::nav::State,
    pub theme: Theme,
    /// The terminal's colour level, which a theme chosen in the palette is resolved for. It
    /// can differ from the theme's own level: `mono` resolves without colours on any terminal.
    pub color_level: ColorLevel,
    /// The terminal's background colour, when it was queried.
    pub background: Option<Rgb>,
    /// What the browser holds besides how the terminal shows it: the directory, its rows and
    /// selection, sort, marks, filters and status line.
    pub session: SessionState,
    /// Where the Files list is scrolled to: its first row on screen.
    pub scroll: usize,
    pub(crate) roots_initialized: bool,
    /// The scroll of each directory left, restored on the way back as the session restores
    /// its selection.
    scrolls: HashMap<HostPath, usize>,
    /// The optional columns of the list (`c`).
    pub columns: crate::view::columns::Columns,
    pub overlay: Overlay,
    pub quit: bool,
    /// A new scan of this path is asked for, outside the scan shown: the run loop opens it.
    pub reopen: Option<HostPath>,
    pub details: details::Details,
    /// The marks pane, in the place of the details pane while it is open.
    pub marks_pane: crate::features::marks_pane::MarksPane,
    pub debug: debug::Debug,
    pub motion: Motion,
    pub mouse: mouse::Mouse,
    pub treemap: treemap::Treemap,
    pub cleanup: crate::features::cleanup::Cleanup,
    pub dupes: crate::features::dupes::Dupes,
    pub largest: crate::features::largest::Largest,
    /// Where the last frame drew each part of the screen.
    pub areas: Areas,
}

impl App {
    /// An empty browser drawn with `theme`, on a terminal with the theme's colour level and
    /// background.
    #[must_use]
    pub fn new(theme: Theme, now: Instant) -> Self {
        App {
            #[cfg(unix)]
            peer: None,
            #[cfg(unix)]
            peer_prompt: None,
            #[cfg(unix)]
            peer_panel: None,
            #[cfg(unix)]
            peer_background: crate::peer::Background::default(),
            #[cfg(unix)]
            nav: crate::peer::nav::State::default(),
            color_level: theme.level(),
            background: theme.background(),
            theme,
            session: SessionState::new(now),
            scroll: 0,
            roots_initialized: false,
            scrolls: HashMap::new(),
            columns: crate::view::columns::Columns::default(),
            overlay: Overlay::None,
            quit: false,
            reopen: None,
            details: details::Details::default(),
            marks_pane: crate::features::marks_pane::MarksPane::default(),
            debug: debug::Debug::default(),
            motion: Motion::default(),
            mouse: mouse::Mouse::default(),
            treemap: treemap::Treemap::default(),
            cleanup: crate::features::cleanup::Cleanup::default(),
            dupes: crate::features::dupes::Dupes::default(),
            largest: crate::features::largest::Largest::default(),
            areas: Areas::default(),
        }
    }

    /// The browser `ui` describes: its theme, colour level and background, `~`, starting file
    /// type filter, whether this machine is looked up and whether anything can be deleted. The
    /// run loop keeps the rest (mouse, debug panel, history, cache).
    #[must_use]
    pub fn with_options(ui: &crate::UiOptions, now: Instant) -> Self {
        let mut app = App {
            color_level: ui.color_level,
            background: ui.background,
            ..App::new(ui.theme.clone(), now)
        };
        #[cfg(unix)]
        {
            app.peer.clone_from(&ui.peer);
        }
        let s = &mut app.session;
        s.home.clone_from(&ui.home);
        s.types = ui.types.clone();
        s.read_only = ui.read_only;
        s.from_file = ui.from_file.then_some(dfdu_session::goto::FROM_FILE);
        s.elevated = ui.elevated;
        s.status.clone_from(&ui.notice);
        s.outside.local = ui.local;
        s.came_from.clone_from(&ui.came_from);
        app
    }

    /// True when the features need a read of the engine model for the next frame although no
    /// new snapshot is due. A read must make it false: while it is true, the session driver reads
    /// again at once.
    #[cfg(test)]
    pub fn extras_stale(&self) -> bool {
        dfdu_session::extras::stale(&self.session)
    }

    /// The tree mode state while the rows are a tree ([`SessionState::tree_shown`]).
    pub fn tree_shown(&self) -> Option<&crate::state::tree::TreeState> {
        self.session.tree_shown()
    }

    /// Opens the error list read by the run loop for [`Effect::ShowErrors`].
    pub fn show_errors(&mut self, list: ErrorList) {
        self.overlay = Overlay::Errors { list, cursor: 0 };
    }

    /// Handles a key: the delete confirmation or the open overlay takes it first, then the
    /// filter input while it is edited, then the marks pane while it is open, then the details
    /// pane while it has the focus, then the view shown (the treemap's moves between tiles once
    /// it is drawn), and otherwise the action bound to it in `keys::ACTIONS`.
    pub fn key(&mut self, k: KeyEvent, now: Instant) -> Vec<Effect> {
        if k.kind == KeyEventKind::Release {
            return Vec::new();
        }
        self.session.note_input(now);
        // Only an action bound to the key can be the second `Parent` at the scan root: a key
        // an overlay, the filter, a pane or a view takes disarms the first, and its notice goes.
        let armed = self.session.up_armed.take();
        let notice = armed.clone();
        let fx = self.key_armed(k, now, armed);
        if self.session.up_armed.is_none()
            && let Some(a) = &notice
        {
            self.session.drop_notice(a);
        }
        fx
    }

    /// [`App::key`] with the first `Parent` at the scan root taken out: `armed` is put back
    /// only for the action bound to the key.
    fn key_armed(
        &mut self,
        k: KeyEvent,
        now: Instant,
        armed: Option<dfdu_session::goto::Armed>,
    ) -> Vec<Effect> {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return Vec::new();
        }
        #[cfg(unix)]
        if crate::peer::key(self, k) {
            return Vec::new();
        }
        if confirm::is_open(self) {
            return confirm::key(self, k);
        }
        match self.overlay {
            Overlay::None => {}
            Overlay::Help(_) => return help::key(self, k),
            Overlay::Errors { .. } => return error_list::key(self, k),
            Overlay::Palette { .. } => return palette::key(self, k),
            Overlay::Types(_) => return type_filter::key(self, k),
            Overlay::MarkPattern(_) => return mark_pattern::key(self, k),
            Overlay::GoTo(_) => return goto::key(self, k),
            Overlay::Roots(_) => return crate::view::roots::key(self, k),
        }
        if filter::editing(self) {
            // `↑`/`↓` move the cursor in the list; the other keys edit the query.
            match k.code {
                KeyCode::Up => self.session.move_by(-1),
                KeyCode::Down => self.session.move_by(1),
                _ => {
                    if let Some(op) = line_keys::op(&k) {
                        return filter::edit(self, op, now);
                    }
                }
            }
            return Vec::new();
        }
        if crate::features::marks_pane::is_open(self)
            && let Some(fx) = crate::features::marks_pane::key(self, &k)
        {
            return fx;
        }
        if details::focused(self)
            && let Some(fx) = details::key(self, &k)
        {
            return fx;
        }
        let fx = match views::current(self) {
            views::View::Largest => crate::features::largest::key(self, &k),
            views::View::Duplicates => crate::features::dupes::key(self, &k),
            views::View::Cleanup => crate::features::cleanup::key(self, &k),
            views::View::Map if treemap::active(self) => treemap::key(self, &k),
            views::View::Map | views::View::Files => None,
        };
        if let Some(fx) = fx {
            return fx;
        }
        self.session.status = None;
        self.session.up_armed = armed;
        match keys::action_for(&k) {
            // These view-specific keys predate the Files bindings.
            Some(keys::Action::SortName)
                if matches!(
                    views::current(self),
                    views::View::Cleanup | views::View::Duplicates
                ) =>
            {
                keys::run(self, keys::Action::TickSuggested)
            }
            Some(keys::Action::PrevMatch) if views::current(self) == views::View::Cleanup => {
                keys::run(self, keys::Action::CleanupCommand)
            }
            Some(a) => keys::run(self, a),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
