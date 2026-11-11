//! The browser's loop: ticks the session driver ([`Session`]), which reads the engine model when
//! scan data or input calls for it, draws when it or input asks for a frame, and sleeps until
//! the next key, engine change or timed wake. The pacing rules are in the crate docs. It
//! returns when the user quits, or with a new scan opened when the user asked for a path outside
//! the scan ([`Exit::Reopen`]).

use std::io;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, select};
use dfdu_engine::{Engine, Scanned};
use dfdu_session::effect::Output;
use dfdu_session::{Session, SessionOptions};
use dfdu_tree::HostPath;
use ratatui::Terminal;
use ratatui::crossterm::event::Event;

use crate::app::{App, Effect};
use crate::features::{debug, selected_path};
use crate::input::mouse;
use crate::overlay::goto;
use crate::state::motion;
use crate::{UiOptions, view};

/// Longest sleep when nothing is going on.
const IDLE: Duration = Duration::from_secs(3600);

pub(crate) struct OnSelect {
    command: Option<std::ffi::OsString>,
    previous: Option<HostPath>,
}

impl OnSelect {
    pub(crate) fn new(command: Option<std::ffi::OsString>) -> Self {
        Self {
            command,
            previous: None,
        }
    }

    pub(crate) fn observe(
        &mut self,
        local: bool,
        selected: Option<HostPath>,
    ) -> Option<std::path::PathBuf> {
        let selected = selected.filter(|path| local && path.host() == dfdu_tree::Host::LOCAL)?;
        if self.previous.as_ref() == Some(&selected) {
            return None;
        }
        let path = selected.to_local();
        self.previous = Some(selected);
        Some(path)
    }

    pub(crate) fn update(&mut self, local: bool, selected: Option<HostPath>) {
        if let Some(path) = self.observe(local, selected)
            && let Some(command) = &self.command
            && let Ok(mut child) = std::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .arg("dfdu-on-select")
                .arg(path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

/// Why the browser's loop returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The user quit, or the input closed.
    Quit,
    /// The user asked for a scan of this path, outside the scan shown: up past the scan root,
    /// or a folder gone to.
    Reopen(HostPath),
}

/// The browser's loop with [`UiOptions::from_env`]. Public for `tests/latency.rs`, which passes
/// a recording backend and a channel it writes keys to.
///
/// # Errors
///
/// As [`event_loop_with`].
pub fn event_loop<T, B>(
    terminal: &mut Terminal<T>,
    engine: &Engine<B>,
    input: Receiver<Event>,
) -> io::Result<Exit>
where
    T: ratatui::backend::Backend<Error = io::Error>,
    B: dfdu_scan::Backend,
{
    event_loop_with(terminal, engine, input, UiOptions::from_env())
}

/// The browser's loop: terminal events from `input`, frames to `terminal`. Returns when the user
/// quits or `input` closes ([`Exit::Quit`]) or asks for a scan outside this one
/// ([`Exit::Reopen`]), once the session has stopped what runs beside the engine (a comparison
/// of duplicates, a save to the cache, a record in the history); the engine is the caller's to
/// shut down.
///
/// # Errors
///
/// When drawing to `terminal` fails, or the engine stopped with a fatal error (showing its
/// stale data as if it were live would mislead).
///
/// # Panics
///
/// On the first key press when `DFDU_TEST_UI_PANIC` is set: a hidden switch for
/// `crates/dfdu/tests/ui_panic.rs`, which tests that a panic on the UI thread restores the
/// terminal and writes a crash report.
pub fn event_loop_with<T, B>(
    terminal: &mut Terminal<T>,
    engine: &Engine<B>,
    input: Receiver<Event>,
    ui: UiOptions,
) -> io::Result<Exit>
where
    T: ratatui::backend::Backend<Error = io::Error>,
    B: dfdu_scan::Backend,
{
    let opened = event_loop_opening(terminal, engine, &input, ui, |_, path, _| {
        Ok(Ok(path.clone()))
    })?;
    Ok(opened.map_or(Exit::Quit, Exit::Reopen))
}

/// [`event_loop_with`], opening the scan of a path outside this one with `open` while this one
/// still runs, given what this scan read when the path is above its root
/// ([`Session::scanned`]): what it opened is returned, or what went wrong with the scan is shown on the
/// status line and the loop goes on. An error of `open`'s own (the terminal could not be taken
/// back) ends the loop with it.
pub(crate) fn event_loop_opening<T, B, N>(
    terminal: &mut Terminal<T>,
    engine: &Engine<B>,
    input: &Receiver<Event>,
    ui: UiOptions,
    mut open: impl FnMut(&mut Terminal<T>, &HostPath, Option<Scanned>) -> io::Result<Result<N, String>>,
) -> io::Result<Option<N>>
where
    T: ratatui::backend::Backend<Error = io::Error>,
    B: dfdu_scan::Backend,
{
    let mut app = App::with_options(&ui, Instant::now());
    motion::start(&mut app);
    if ui.debug {
        debug::toggle(&mut app);
    }
    let jobs = SessionOptions {
        history: ui.history.clone(),
        cache: ui.cache.clone(),
    };
    let mut session = Session::over(engine, app, jobs);
    let result = run(terminal, &mut session, input, ui.mouse, &mut open);
    session.shutdown(Duration::ZERO);
    result
}

/// The loop itself, until the user quits, `input` closes or `open` opened a scan outside this
/// one.
fn run<T, B, N>(
    terminal: &mut Terminal<T>,
    session: &mut Session<'_, B, App>,
    input: &Receiver<Event>,
    mouse_on: bool,
    open: &mut impl FnMut(&mut Terminal<T>, &HostPath, Option<Scanned>) -> io::Result<Result<N, String>>,
) -> io::Result<Option<N>>
where
    T: ratatui::backend::Backend<Error = io::Error>,
    B: dfdu_scan::Backend,
{
    let inject_panic = std::env::var_os("DFDU_TEST_UI_PANIC").is_some();
    let changes = session.changes().clone();
    // The first frame is drawn at once.
    let mut input_dirty = true;
    let mut on_select = OnSelect::new(std::env::var_os("DFDU_ON_SELECT"));
    let listener = if let Some(addr) = std::env::var_os("DFDU_LISTEN") {
        let key = std::env::var("DFDU_PEER_KEY").ok();
        let result = crate::peer::Listener::bind(&addr.to_string_lossy(), key);
        match result {
            Ok(listener) => Some(listener),
            Err(e) => {
                session.front_mut().session.status = Some(format!("peer listener: {e}"));
                None
            }
        }
    } else {
        None
    };
    let outgoing = std::env::var("DFDU_PEER")
        .ok()
        .map(|addr| crate::peer::Outgoing::start(addr, std::env::var("DFDU_PEER_KEY").ok()));
    let peer_replies = outgoing
        .as_ref()
        .map_or_else(crossbeam_channel::never, |p| p.replies.clone());
    let follow = std::env::var("DFDU_PEER_FOLLOW").as_deref() == Ok("1");
    let mut previous_peer_selection = None;
    let mut peer_move = false;
    let mut peer_target: Option<String> = None;
    let peer_requests = listener
        .as_ref()
        .map_or_else(crossbeam_channel::never, |listener| {
            listener.requests().clone()
        });
    loop {
        let now = Instant::now();
        let tick = session.tick(now)?;
        if crate::peer_commands::retry(session) {
            peer_move = true;
            peer_target = crate::peer_commands::target(session.front());
            input_dirty = true;
            // The retry navigated after this tick. Read the destination before sleeping
            // or observing the selection.
            continue;
        }
        let state = &session.front().session;
        let selected = dfdu_session::selected_path::selected(state).map(|(path, _)| path);
        let changed = selected != previous_peer_selection;
        let suppress = peer_move || peer_target.is_some();
        if let Some(target) = &peer_target
            && session
                .engine()
                .read(|m| peer_target_done(&m.tree, target, selected.as_ref()))
        {
            peer_target = None;
        }
        let yank = std::mem::take(&mut session.front_mut().peer.yank);
        if (yank || (follow && changed && !suppress))
            && let Some(outgoing) = &outgoing
        {
            let payload = selected.as_ref().and_then(|path| {
                session
                    .engine()
                    .read(|m| crate::peer_commands::payload(&m.tree, path))
            });
            if let Some(payload) = payload {
                if outgoing.queries.try_send(payload).is_err() {
                    session.front_mut().session.status = Some("peer: sender busy".into());
                }
            } else if yank {
                session.front_mut().session.status = Some("peer: select a file".into());
            }
        } else if yank && outgoing.is_none() {
            session.front_mut().session.status = Some("peer: DFDU_PEER not set".into());
        }
        previous_peer_selection = selected;
        peer_move = false;
        let state = &session.front().session;
        let selected = dfdu_session::selected_path::selected(state).map(|(path, _)| path);
        if suppress {
            on_select.observe(state.outside.local, selected);
        } else {
            on_select.update(state.outside.local, selected);
        }
        // What the scanned host answered the go-to prompt, which the tick took in.
        let answered = goto::poll(session.front_mut());
        let motion_due = (session.front().motion.next_frame_at()).is_some_and(|t| now >= t);
        if tick.changed || answered || input_dirty || motion_due {
            let drawing = Instant::now();
            let app = session.front_mut();
            terminal.draw(|f| view::draw(f, app, now))?;
            let read = session.read_cost();
            debug::frame_drawn(session.front_mut(), drawing.elapsed(), read);
            session.shown(now);
            input_dirty = false;
        }
        if session.front().quit {
            return Ok(None);
        }
        if let Some(path) = session.front_mut().reopen.take() {
            let scanned = session.scanned(&path);
            match open(terminal, &path, scanned)? {
                Ok(opened) => return Ok(Some(opened)),
                Err(e) => {
                    session.front_mut().session.status = Some(e);
                    input_dirty = true;
                    continue;
                }
            }
        }

        let wake = [session.wake_at(now), session.front().motion.next_frame_at()]
            .into_iter()
            .flatten()
            .min()
            .map_or(IDLE, |t| t.saturating_duration_since(now));
        select! {
            recv(input) -> ev => match ev {
                Ok(Event::Key(k)) => {
                    assert!(!inject_panic, "injected UI panic");
                    let before = cursor(session.front());
                    let fx = session.front_mut().key(k, Instant::now());
                    handle_effects(session, fx);
                    if let Some(text) = goto::submitted(session.front_mut()) {
                        let gone = session.go_to_path(&text);
                        let fx = goto::gone_to(session.front_mut(), gone);
                        handle_effects(session, fx);
                    }
                    if cursor_moved(&before, &cursor(session.front())) {
                        peer_target = None;
                        crate::peer_commands::cancel_retry(session.front_mut());
                    }
                    if crate::peer_commands::pending(session) { peer_move = true; }
                    input_dirty = true;
                }
                Ok(Event::Mouse(m)) if mouse_on => {
                    let before = cursor(session.front());
                    if let Some(fx) = mouse::event(session.front_mut(), m, Instant::now()) {
                        handle_effects(session, fx);
                        input_dirty = true;
                    }
                    if cursor_moved(&before, &cursor(session.front())) {
                        peer_target = None;
                        crate::peer_commands::cancel_retry(session.front_mut());
                    }
                }
                Ok(Event::Resize(..)) => input_dirty = true,
                Ok(_) => {}
                // The input thread ended (terminal closed): leave.
                Err(_) => return Ok(None),
            },
            recv(peer_requests) -> request => if let Ok(request) = request {
                let moves = request.command.get("match").is_some() || request.command.get("next").is_some() || request.command.get("prev").is_some() || request.command.get("goto").is_some();
                let reply = crate::peer_commands::command(session, request.command);
                if moves && let Some(path) = reply.get("path").and_then(|p| p.as_str()) {
                    peer_move = true;
                    peer_target = Some(path.to_owned());
                }
                let _ = request.reply.send(reply);
                input_dirty = true;
            },
            recv(peer_replies) -> reply => if let Ok(reply) = reply {
                session.front_mut().session.status = Some(reply);
                input_dirty = true;
            },
            recv(changes) -> _ => session.note_change(),
            default(wake) => {}
        }
    }
}

// Keep suppression through the tick that reaches or loses the destination.
fn peer_target_done(tree: &dfdu_tree::Tree, target: &str, selected: Option<&HostPath>) -> bool {
    selected.is_some_and(|path| path.display() == target) || {
        let path = HostPath::new(tree.host(), target.as_bytes().to_vec());
        dfdu_tree::find_path(tree, &path).is_none()
    }
}

type Cursor = (
    dfdu_tree::HostPath,
    Option<Vec<u8>>,
    dfdu_session::view::View,
    Option<dfdu_tree::HostPath>,
);

fn cursor(app: &App) -> Cursor {
    let state = &app.session;
    (
        state.target.path.clone(),
        state.selected.as_ref().map(|s| s.name.clone()),
        state.current_view(),
        dfdu_session::selected_path::selected(state).map(|(path, _)| path),
    )
}

fn cursor_moved(before: &Cursor, after: &Cursor) -> bool {
    before.0 != after.0 || before.1 != after.1 || (before.2 == after.2 && before.3 != after.3)
}

/// Carries out the effects of a key or mouse event: the session does the engine's part
/// ([`Session::carry_out`]), and the TUI shows the error list, copies through the terminal and
/// opens the file manager.
fn handle_effects<B: dfdu_scan::Backend>(session: &mut Session<'_, B, App>, fx: Vec<Effect>) {
    for out in session.carry_out(fx) {
        let app = session.front_mut();
        match out {
            Output::Errors(list) => app.show_errors(list),
            Output::Copy(text) => {
                // To the terminal the frames go to; a frame is never being drawn here.
                if let Err(e) = selected_path::write_osc52(&mut io::stdout(), &text) {
                    app.session.status = Some(format!("cannot copy: {e}"));
                }
            }
            Output::Open { path, is_dir } => selected_path::spawn_open(app, &path, is_dir),
        }
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use crate::model::snapshot::test_entry;
    use crate::tests::{press, snap_with_totals};
    use crate::theme::Theme;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseEvent, MouseEventKind};

    #[test]
    fn peer_target_guard_ends_when_reached_or_missing() {
        use dfdu_engine::test_support::meta;
        use dfdu_tree::{Kind, Tree};

        let mut tree = Tree::new();
        let root = tree.new_root(b"/r", &meta(Kind::Dir, 0, 1), true);
        let file = tree.add_file(root, b"file", &meta(Kind::File, 3, 2), true, None);
        let path = tree.host_path(file);
        let target = path.display();
        assert!(!peer_target_done(&tree, &target, None));
        assert!(peer_target_done(&tree, &target, Some(&path)));
        assert!(peer_target_done(&Tree::new(), &target, None));
    }

    #[test]
    fn only_actual_user_cursor_moves_are_detected() {
        let now = Instant::now();
        let mut app = App::new(Theme::no_color(), now);
        app.refresh(
            snap_with_totals(vec![test_entry(0, "a", 2), test_entry(1, "b", 1)], 1),
            now,
        );
        let before = cursor(&app);
        app.key(press(KeyCode::Char('?')), now);
        assert!(
            !cursor_moved(&before, &cursor(&app)),
            "help is not a cursor move"
        );
        app.key(press(KeyCode::Char('?')), now);
        let _ = mouse::event(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Moved,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            now,
        );
        assert!(
            !cursor_moved(&before, &cursor(&app)),
            "mouse motion is not a cursor move"
        );
        app.key(press(KeyCode::Down), now);
        assert!(
            cursor_moved(&before, &cursor(&app)),
            "down moves the selection"
        );
        let last = cursor(&app);
        app.key(press(KeyCode::Down), now);
        assert!(
            !cursor_moved(&last, &cursor(&app)),
            "down at the boundary does not move"
        );
    }
}
