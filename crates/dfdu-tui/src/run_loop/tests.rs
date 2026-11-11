//! A real decode through the loop, with terminal and graphics writes recorded in order.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Sender, bounded, unbounded};
use dfdu_engine::{Command, Engine, EngineOptions};
use dfdu_scan::{JobOptions, PortableBackend};
use dfdu_session::{Session, SessionOptions};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};

use super::run;
use crate::UiOptions;
use crate::app::App;
use crate::preview::runtime::Preview;

// One red pixel, RGB PNG. No external fixtures or image encoder dependency.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0,
    0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0, 0, 3, 1,
    1, 0, 201, 254, 146, 239, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[derive(Clone, Debug)]
enum Written {
    FrameFlushed,
    Graphics(Vec<u8>),
}

struct Recording {
    events: Arc<Mutex<Vec<Written>>>,
    /// Only the graphics writer ends the test once it sees a real image stream.
    input: Option<Sender<Event>>,
    on_image: Vec<Event>,
}

fn quit() -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
}

impl Write for Recording {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(input) = &self.input {
            self.events
                .lock()
                .unwrap()
                .push(Written::Graphics(bytes.to_vec()));
            if bytes.windows(5).any(|w| w == b"_Ga=T") {
                for event in std::mem::take(&mut self.on_image) {
                    let _ = input.send(event);
                }
            } else {
                // Leave even when the triggering action opened an overlay.
                let _ = input.send(Event::Key(KeyEvent::new(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL,
                )));
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.input.is_none() {
            self.events.lock().unwrap().push(Written::FrameFlushed);
        }
        Ok(())
    }
}

#[test]
fn real_image_completion_wakes_the_loop_and_cleanup_follows_ui_changes() {
    // Selection, view, overlay, quit, and the reopen/suspend boundary.
    for actions in [
        vec![quit()],
        vec![key('j')],
        vec![key('2')],
        vec![key('?')],
        vec![key('h'), key('h')],
    ] {
        run_actions(actions);
    }
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn run_actions(actions: Vec<Event>) {
    let reopening = actions == vec![key('h'), key('h')];
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("red.png"), PNG).unwrap();
    std::fs::write(tmp.path().join("text.txt"), b"x").unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap();
    let engine = Engine::start(
        PortableBackend::default(),
        EngineOptions {
            workers: 1,
            journal_dir: None,
            ..Default::default()
        },
    );
    engine.send(Command::Scan {
        path: root.into(),
        opts: JobOptions::default(),
    });
    let _ = engine.wait_idle(Some(Duration::from_secs(10)));
    let mut ui = UiOptions::from_env();
    ui.local = true;
    let app = App::with_options(&ui, Instant::now());
    let mut session = Session::over(&engine, app, SessionOptions::default());
    let (input_tx, input) = unbounded();
    let events = Arc::new(Mutex::new(Vec::new()));
    let backend = CrosstermBackend::new(Recording {
        events: events.clone(),
        input: None,
        on_image: Vec::new(),
    });
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 100, 30)),
        },
    )
    .unwrap();
    let mut preview = Preview::new(
        Recording {
            events: events.clone(),
            input: Some(input_tx.clone()),
            on_image: actions,
        },
        true,
    )
    .unwrap();
    let (done, finished) = bounded(1);
    std::thread::scope(|scope| {
        // Fail rather than hang if the completion fails to wake an idle loop.
        scope.spawn(|| {
            if finished.recv_timeout(Duration::from_secs(5)).is_err() {
                let _ = input_tx.send(quit());
            }
        });
        let result = run(
            &mut terminal,
            &mut session,
            &input,
            false,
            &mut |_, _, _| {
                assert!(
                    is_delete(events.lock().unwrap().last()),
                    "delete must precede handing off the terminal for reopen/suspend"
                );
                Ok(Ok(()))
            },
            &mut preview,
        );
        let _ = done.send(());
        assert_eq!(result.unwrap(), reopening.then_some(()));
    });
    drop(preview);
    session.shutdown(Duration::ZERO);
    engine.shutdown(Duration::from_millis(200));
    let events = events.lock().unwrap().clone();
    let images: Vec<_> = events
        .iter()
        .enumerate()
        .filter_map(|(i, event)| match event {
            Written::Graphics(bytes) if bytes.windows(5).any(|w| w == b"_Ga=T") => Some(i),
            _ => None,
        })
        .collect();
    assert!(
        !images.is_empty(),
        "real PNG must produce a kitty stream: {events:?}"
    );
    for i in images {
        assert!(
            matches!(events[i - 1], Written::FrameFlushed),
            "graphics follow draw"
        );
    }
    assert!(
        is_delete(
            events
                .iter()
                .rev()
                .find(|event| matches!(event, Written::Graphics(_)))
        ),
        "cleanup deletes: {events:?}"
    );
}

fn is_delete(event: Option<&Written>) -> bool {
    matches!(event, Some(Written::Graphics(bytes))
        if bytes.windows(10).any(|w| w == b"_Ga=d,d=I,"))
}
