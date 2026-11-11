//! The browser's loop: ticks the session driver ([`Session`]), which reads the engine model when
//! scan data or input calls for it, draws when it or input asks for a frame, and sleeps until
//! the next key, engine change or timed wake. The pacing rules are in the crate docs. It
//! returns when the user quits, or with a new scan opened when the user asked for a path outside
//! the scan ([`Exit::Reopen`]).

#[cfg(test)]
mod tests;

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
use crate::preview::runtime::{self as preview, Preview};
use crate::state::motion;
use crate::{UiOptions, view};

/// Longest sleep when nothing is going on.
const IDLE: Duration = Duration::from_secs(3600);

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
    let mut previews = Preview::new(
        io::stdout(),
        preview::enabled() && ui.local && !ui.from_file,
    )?;
    let result = run(
        terminal,
        &mut session,
        input,
        ui.mouse,
        &mut open,
        &mut previews,
    );
    drop(previews);
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
    previews: &mut Preview<impl io::Write>,
) -> io::Result<Option<N>>
where
    T: ratatui::backend::Backend<Error = io::Error>,
    B: dfdu_scan::Backend,
{
    let inject_panic = std::env::var_os("DFDU_TEST_UI_PANIC").is_some();
    let changes = session.changes().clone();
    // The first frame is drawn at once.
    let mut input_dirty = true;
    loop {
        let now = Instant::now();
        let tick = session.tick(now)?;
        // What the scanned host answered the go-to prompt, which the tick took in.
        let answered = goto::poll(session.front_mut());
        let motion_due = (session.front().motion.next_frame_at()).is_some_and(|t| now >= t);
        if tick.changed || answered || input_dirty || motion_due {
            let drawing = Instant::now();
            let app = session.front_mut();
            let area = terminal.draw(|f| view::draw(f, app, now))?.area;
            // Graphics are outside ratatui's cell buffer and must follow its flush.
            previews.after_draw(preview::request(session.front(), area))?;
            let read = session.read_cost();
            debug::frame_drawn(session.front_mut(), drawing.elapsed(), read);
            session.shown(now);
            input_dirty = false;
        }
        if session.front().quit {
            return Ok(None);
        }
        if let Some(path) = session.front_mut().reopen.take() {
            previews.clear()?;
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
                    let fx = session.front_mut().key(k, Instant::now());
                    handle_effects(session, fx);
                    if let Some(text) = goto::submitted(session.front_mut()) {
                        let gone = session.go_to_path(&text);
                        let fx = goto::gone_to(session.front_mut(), gone);
                        handle_effects(session, fx);
                    }
                    input_dirty = true;
                }
                Ok(Event::Mouse(m)) if mouse_on => {
                    if let Some(fx) = mouse::event(session.front_mut(), m, Instant::now()) {
                        handle_effects(session, fx);
                        input_dirty = true;
                    }
                }
                Ok(Event::Resize(..)) => input_dirty = true,
                Ok(_) => {}
                // The input thread ended (terminal closed): leave.
                Err(_) => return Ok(None),
            },
            recv(previews.results()) -> decoded => {
                match decoded {
                    Ok(decoded) => previews.complete(decoded),
                    Err(_) => previews.disable()?,
                }
                input_dirty = true;
            },
            recv(changes) -> _ => session.note_change(),
            default(wake) => {}
        }
    }
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
