//! Image work stays off the UI thread. Only the loop writes the completed stream.

use std::io::{self, IsTerminal, Write};

use crossbeam_channel::{Receiver, Sender, bounded, never};
use ratatui::layout::Rect;

use super::{PreviewState, Request, clip, is_image, supported_terminal};
use crate::app::App;
use crate::app::views::{self, View};

/// One id for this browser's image and all its placements.
const IMAGE_ID: u32 = 0x6466_6475;

pub(crate) fn enabled() -> bool {
    io::stdout().is_terminal()
        && supported_terminal(
            std::env::var("TERM").ok().as_deref(),
            std::env::var("TERM_PROGRAM").ok().as_deref(),
            std::env::var_os("KITTY_WINDOW_ID").as_ref().map(|_| ""),
            std::env::var("DFDU_IMAGE_PREVIEW").ok().as_deref(),
        )
}

/// Also used by terminal restoration and the panic hook, before leaving the screen.
pub(crate) fn delete_terminal_image() {
    if enabled() {
        let mut out = io::stdout().lock();
        let _ = out.write_all(&jpg_preview::delete_image(IMAGE_ID));
        let _ = out.flush();
    }
}

/// Beside the selected row's rendered name. No remote paths are opened.
pub(crate) fn request(app: &App, screen: Rect) -> Option<Request> {
    if !app.session.outside.local
        || app.session.from_file.is_some()
        || views::current(app) != View::Files
        || app.areas.overlay.is_some()
        || app.areas.debug.is_some()
        || app.quit
        || app.reopen.is_some()
    {
        return None;
    }
    let (x, y) = app.areas.preview_anchor?;
    let (path, is_dir) = dfdu_session::selected_path::selected(&app.session)?;
    let path = path.to_local();
    if is_dir || !is_image(&path) {
        return None;
    }
    let area = clip(Rect::new(x, y, 30, 12), screen)?;
    Some(Request::new(path, area))
}

struct Job {
    generation: u64,
    request: Request,
}

pub(crate) struct Decoded {
    job: Job,
    stream: Option<Vec<u8>>,
}

/// One in-flight decode, no queued obsolete work, and one stream awaiting a draw. Dropping never
/// waits for a decoder: it cannot write to the terminal, even after the browser has left.
pub(crate) struct Preview<W: Write> {
    out: W,
    state: PreviewState,
    work: Option<Sender<Job>>,
    results: Receiver<Decoded>,
    busy: bool,
    attempted: bool,
    cached: Option<Vec<u8>>,
    visible: bool,
}

impl<W: Write> Preview<W> {
    pub(crate) fn new(out: W, enabled: bool) -> io::Result<Self> {
        if enabled {
            Self::with_decoder(out, |request| {
                let area = request.area;
                jpg_preview::render(
                    &request.path,
                    usize::from(area.width),
                    usize::from(area.height),
                    usize::from(area.x),
                    usize::from(area.y),
                    IMAGE_ID,
                )
                .ok()
            })
        } else {
            Ok(Self {
                out,
                state: PreviewState::default(),
                work: None,
                results: never(),
                busy: false,
                attempted: false,
                cached: None,
                visible: false,
            })
        }
    }

    fn with_decoder(
        out: W,
        decode: impl Fn(&Request) -> Option<Vec<u8>> + Send + 'static,
    ) -> io::Result<Self> {
        let (work, jobs) = bounded::<Job>(1);
        let (done, results) = bounded(1);
        std::thread::Builder::new()
            .name("dfdu-image-preview".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let stream = decode(&job.request);
                    if done.send(Decoded { job, stream }).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            out,
            state: PreviewState::default(),
            work: Some(work),
            results,
            busy: false,
            attempted: false,
            cached: None,
            visible: false,
        })
    }

    pub(crate) fn results(&self) -> &Receiver<Decoded> {
        &self.results
    }

    /// Completion wakes the loop but never writes before the next terminal draw.
    pub(crate) fn complete(&mut self, decoded: Decoded) {
        self.busy = false;
        if self
            .state
            .is_current(decoded.job.generation, &decoded.job.request)
        {
            self.cached = decoded.stream;
        }
    }

    pub(crate) fn after_draw(&mut self, request: Option<Request>) -> io::Result<()> {
        let request = request.filter(|_| self.work.is_some());
        let generation = self.state.generation();
        self.state.select(request.clone());
        if self.state.generation() != generation {
            self.cached = None;
            self.attempted = false;
            self.delete_visible()?;
        }
        if let Some(stream) = self.cached.take() {
            // A partial write still needs cleanup if writing fails.
            self.visible = true;
            self.out.write_all(&stream)?;
            self.out.flush()?;
        } else if !self.busy
            && !self.attempted
            && let Some(request) = request
        {
            let job = Job {
                generation: self.state.generation(),
                request,
            };
            if self
                .work
                .as_ref()
                .is_some_and(|work| work.send(job).is_ok())
            {
                self.busy = true;
                self.attempted = true;
            } else {
                self.work = None;
                self.results = never();
            }
        }
        Ok(())
    }

    pub(crate) fn disable(&mut self) -> io::Result<()> {
        self.work = None;
        self.results = never();
        self.clear()
    }

    /// Invalidate unfinished work as well as deleting a visible placement.
    pub(crate) fn clear(&mut self) -> io::Result<()> {
        self.state.select(None);
        self.cached = None;
        self.attempted = false;
        self.delete_visible()
    }

    fn delete_visible(&mut self) -> io::Result<()> {
        if self.visible {
            self.out.write_all(&jpg_preview::delete_image(IMAGE_ID))?;
            self.out.flush()?;
            self.visible = false;
        }
        Ok(())
    }
}

impl<W: Write> Drop for Preview<W> {
    fn drop(&mut self) {
        // The panic hook clears graphics before restoring the alternate screen.
        if !std::thread::panicking() {
            let _ = self.clear();
        }
    }
}

#[cfg(test)]
mod tests;
