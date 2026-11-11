//! The interactive browser. The run loop (`run_loop`) ticks the session driver
//! (`dfdu_session::Session`), which reads the engine model under the read lock only to copy a
//! `model::snapshot::Snapshot` and the features' `model::extras::Extras`; the loop draws from
//! the copy.
//!
//! State and key handling are in `app` (with the action registry in `app::keys`), drawing in
//! `view`, with the styles in `theme` and the text for sizes, counts and bars in `format`. Each
//! feature has its own module with its state, keys and drawing: the views and panels beside the
//! list in `features`, and the overlays drawn over it in `overlay`. The state of the list's rows
//! is in `state`, the text input, the mouse and fuzzy matching in `input`, and what is read from
//! outside the UI (the engine model, the scan history and cache, the mounts outside the scan) in
//! `model`; only the duplicates comparison and the disk picker read files and disks themselves.
//! Each of these lists its modules. A scan of another path, asked for from the browser, is
//! opened in `reopen`.
//!
//! Redraws: at once on input, at most 10 times a second when the engine reports changes (at
//! once until a frame with entries has been drawn), every 100 ms while a spinner is visible, at
//! the automatic re-sort deadline, while an animation runs, and never when idle. Input is drawn
//! from the snapshot held unless it changed the directory or size mode; the model is read again
//! when scan data is due, at most 10 times a second and less often when reading takes long
//! (large directories). When only the extras are out of date (marks changed, another entry
//! selected for the details pane), they are read without copying the directory again.

mod app;
mod features;
mod format;
mod input;
mod model;
#[cfg(test)]
mod on_select_tests;
mod overlay;
mod peer;
mod peer_commands;
#[cfg(test)]
mod peer_tests;
mod reopen;
mod run_loop;
mod setup;
mod state;
mod theme;
mod view;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use dfdu_engine::{Engine, EngineOptions};
use dfdu_session::startup::{self, Opened, Setup};
use dfdu_tree::HostPath;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;

use features::picker::Picked;
use input::reader::Reader;

pub use app::{IDLE_BEFORE_SORT, MIN_SORT_INTERVAL};
pub use dfdu_session::cache_link::CacheLink;
pub use dfdu_session::history_link::HistoryLink;
pub use dfdu_session::startup::{Source, StartLocal};
pub use features::debug::summary as debug_summary;
pub use run_loop::{Exit, event_loop, event_loop_with};
pub use setup::{ThemeChoice, UiSettings};
pub use state::motion::MAX_STEP as MAX_EASING_STEP;
pub use theme::{Bars, BuiltinTheme, ColorLevel, Theme, ThemeError, ThemeFile, parse_color_flag};

/// How long the terminal gets to answer the background-colour query at start.
const BACKGROUND_QUERY: Duration = Duration::from_secs(1);

pub struct RunOptions {
    pub source: Source,
    /// Options of engines the browser starts (all but [`Source::Remote`]'s). Its `read_only`
    /// also tells the browser to leave out the delete keys, for a remote engine as well.
    pub engine: EngineOptions,
    pub ui: UiSettings,
    /// Record a scan of this host's files in the history once it finishes, and show what grew
    /// since the last.
    pub history: bool,
    /// How a scan of another path on the same host is opened: up past the scan root, or a
    /// folder gone to outside the scan. `None` offers neither.
    pub reopen: Option<Reopen>,
}

/// Builds the source for a scan of a path, with the options of the first scan.
pub type SourceFor = Box<dyn FnMut(&HostPath) -> io::Result<Reopened>>;

/// The source for a scan of another path ([`SourceFor`]), and what the new browser's status
/// line says of how it is opened (an option that could not be used, the scan cache left out):
/// the terminal belongs to the browser, so nothing is written to it.
pub struct Reopened {
    pub source: Source,
    pub notice: Option<String>,
}

/// How the browser opens a scan of another path on the host it scans, with the options of the
/// first scan.
pub struct Reopen {
    /// The source for a scan of the path.
    pub source: SourceFor,
    /// Building it talks to the user on the terminal (ssh asking for a password): the browser
    /// gives the terminal back while it runs, and takes it again after.
    pub suspend: bool,
}

/// How the browser looks and behaves, decided before the loop starts.
#[derive(Clone, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent options of the browser, not one state spread over several bools"
)]
pub struct UiOptions {
    pub theme: Theme,
    /// The terminal's colour level, which a theme chosen in the palette is resolved for.
    pub color_level: ColorLevel,
    /// The terminal's background colour, when it was queried.
    pub background: Option<theme::Rgb>,
    /// Mouse capture is on, so mouse events are handled.
    pub mouse: bool,
    /// Start with the debug panel open.
    pub debug: bool,
    /// The user's home directory, shown as `~` in paths of a scan of this machine.
    pub home: Option<PathBuf>,
    /// The scan history this browser reads and adds to.
    pub history: Option<HistoryLink>,
    /// The scan cache: what the browser opens (it sends the first scan or resume itself) and
    /// saves complete scans to.
    pub cache: Option<CacheLink>,
    /// The file type filter to start with.
    pub types: dfdu_tree::filetype::TypeFilter,
    /// The scan is of this machine's disks: mounts it did not enter and links out of it are
    /// looked up here (not for imported or remote scans).
    pub local: bool,
    /// Nothing is deleted (`--read-only`, imported scans): the delete keys are left out and
    /// cleanup commands run only as dry runs.
    pub read_only: bool,
    /// The scan was read from a file: nothing outside it can be opened.
    pub from_file: bool,
    /// The scan runs as an administrator: a folder gone to is not looked up as the user.
    pub elevated: bool,
    /// What the status line says when the browser opens.
    pub notice: Option<String>,
    /// The root of the scan this browser was opened from: going up from it selects the folder
    /// come from once the new scan lists it.
    pub came_from: Option<HostPath>,
}

impl UiOptions {
    /// The theme [`run`] would choose from the environment with the default settings, without
    /// asking the terminal for its background; no mouse, no debug panel, no `~`.
    #[must_use]
    pub fn from_env() -> Self {
        let colors = setup::resolve_theme(&UiSettings::default(), |k| std::env::var_os(k), || None);
        UiOptions {
            theme: colors.theme,
            color_level: colors.level,
            background: colors.background,
            mouse: false,
            debug: false,
            home: None,
            history: None,
            cache: None,
            types: dfdu_tree::filetype::TypeFilter::default(),
            local: false,
            read_only: false,
            from_file: false,
            elevated: false,
            notice: None,
            came_from: None,
        }
    }
}

/// Opens the browser on `opts.source` (or a disk chosen in the picker) and returns when the user
/// quits. Before the alternate screen is entered, the colour level is detected and, when the
/// theme depends on it, the terminal is asked for its background colour (waiting at most
/// `BACKGROUND_QUERY` for terminals that do not answer).
///
/// A scan of a path outside the one shown is opened with `opts.reopen` before the one shown is
/// shut down, so a failure leaves the user where they were with the error shown.
///
/// # Errors
///
/// When the terminal cannot be set up, drawn to or taken back after a new connection asked
/// for a password, the disk picker fails, a resumed scan has no root, or the engine stops with
/// a fatal error.
pub fn run(opts: RunOptions) -> io::Result<()> {
    let ui = opts.ui;
    let colors = setup::resolve_theme(
        &ui,
        |k| std::env::var_os(k),
        || theme::query_background(BACKGROUND_QUERY),
    );
    let theme = colors.theme;
    if colors.force_color {
        // crossterm leaves out colours itself while `NO_COLOR` is set.
        ratatui::crossterm::style::force_color_output(true);
    }
    let mut terminal = ratatui::try_init()?;
    let mouse = ui.mouse;
    let mut restore = Some(setup::Restore { mouse });
    restore_on_panic(mouse);
    if mouse {
        execute!(io::stdout(), EnableMouseCapture)?;
    }
    let source = match opts.source {
        Source::Local { path: None, start } => match features::picker::pick(&mut terminal, &theme)?
        {
            Picked::Scan(p) => Source::Local {
                path: Some(p),
                start,
            },
            Picked::Quit => return Ok(()),
        },
        source => source,
    };
    let ui_opts = |setup: Setup, from_file: bool, notice: Option<String>, came_from| UiOptions {
        theme: theme.clone(),
        color_level: colors.level,
        background: colors.background,
        mouse,
        debug: ui.debug_stats,
        home: setup.home,
        history: setup.jobs.history,
        cache: setup.jobs.cache,
        types: ui.types.clone(),
        local: setup.local,
        read_only: setup.read_only,
        from_file,
        elevated: setup.elevated,
        notice,
        came_from,
    };
    let reader = Reader::spawn()?;
    let (engine_opts, history, mut reopen) = (opts.engine, opts.history, opts.reopen);
    let mut opened = (startup::open(source, &engine_opts, history)?, None);
    // The root of the scan the next one is opened from.
    let mut left: Option<HostPath> = None;
    loop {
        let mut open = |terminal: &mut ratatui::DefaultTerminal,
                        path: &HostPath,
                        scanned: Option<dfdu_engine::Scanned>| {
            let Some(reopen) = reopen.as_mut() else {
                return Ok(Err(NO_REOPEN.to_owned()));
            };
            let mut hand = reopen::Hand::new(terminal, &reader, mouse);
            let build = || {
                let r = (reopen.source)(path)?;
                let o = startup::open_reusing(r.source, &engine_opts, history, scanned)?;
                Ok((o, r.notice))
            };
            reopen::open_other(&mut hand, reopen.suspend, path, build, |(o, _)| {
                reopen::shut_down(o);
            })
        };
        let (next, notice) = opened;
        let next = match next {
            Opened::Native(s) => {
                let ui = ui_opts(s.setup, false, notice, left.take());
                browse(
                    &mut terminal,
                    &reader,
                    &mut restore,
                    s.engine,
                    ui,
                    (&mut open, &mut left),
                )
            }
            Opened::Imported(s) => {
                let ui = ui_opts(s.setup, true, notice, left.take());
                browse(
                    &mut terminal,
                    &reader,
                    &mut restore,
                    s.engine,
                    ui,
                    (&mut open, &mut left),
                )
            }
        }?;
        match next {
            Some(next) => {
                // What was typed while the new scan opened was meant for the old browser.
                reader.drain();
                opened = next;
            }
            None => return Ok(()),
        }
    }
}

/// What the browser says when it has no way to open another scan.
const NO_REOPEN: &str = "another folder cannot be scanned from here: quit and run dfdu with it";

/// Restores the terminal on a panic of the UI thread. `ratatui::try_init` installed a hook that
/// restores it on any panic; panic hooks also run for panics that are caught, and worker and
/// applier panics are caught and recorded by the engine, so for those threads the terminal
/// must be left alone.
fn restore_on_panic(mouse: bool) {
    let restore = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !dfdu_engine::panic_is_caught_by_engine(std::thread::current().name()) {
            // ratatui's hook leaves raw mode and the alternate screen but does not show the
            // cursor or end mouse capture; the terminal's Drop would show the cursor, but only
            // if unwinding gets that far.
            let _ = execute!(io::stdout(), ratatui::crossterm::cursor::Show);
            if mouse {
                let _ = execute!(io::stdout(), DisableMouseCapture);
            }
            restore(info);
        }
    }));
}

/// Runs the loop on `engine` until the user quits or `open` opened a scan outside this one,
/// then stops the engine; for a scan opened, `left` is the root of this one. When the user
/// quits or the loop fails, the terminal is restored first (`restore`).
fn browse<B: dfdu_scan::Backend, N>(
    terminal: &mut ratatui::DefaultTerminal,
    reader: &Reader,
    restore: &mut Option<setup::Restore>,
    engine: Engine<B>,
    ui: UiOptions,
    (open, left): (
        &mut impl FnMut(
            &mut ratatui::DefaultTerminal,
            &HostPath,
            Option<dfdu_engine::Scanned>,
        ) -> io::Result<Result<N, String>>,
        &mut Option<HostPath>,
    ),
) -> io::Result<Option<N>> {
    let debug_stats = ui.debug;
    let result = run_loop::event_loop_opening(terminal, &engine, reader.events(), ui, open);
    if matches!(result, Ok(Some(_))) {
        *left = engine.read(|m| m.tree.roots().next().map(|r| m.tree.host_path(r)));
        engine.shutdown(Duration::from_millis(200));
        return result;
    }
    drop(restore.take());
    if debug_stats {
        let summary = debug_summary(&engine);
        if !summary.is_empty() {
            eprintln!("{}", summary.trim_end());
        }
    }
    engine.shutdown(Duration::from_millis(200));
    result
}

#[cfg(test)]
mod tests;
