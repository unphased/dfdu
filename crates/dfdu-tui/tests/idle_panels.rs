//! With the details pane and the debug panel open, the run loop still draws nothing while
//! nothing changes: the panels refresh with the frames the loop draws anyway.
//!
//! Like `tests/idle.rs`, this drives the real run loop (`dfdu_tui::event_loop_with`, with the
//! debug panel open) and the real engine, drawing to a `TestBackend` wrapped in a backend that
//! records every flush.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use common::Recording;
use crossbeam_channel::{Sender, unbounded};
use dfdu_engine::{Command, Engine, EngineOptions};
use dfdu_scan::JobOptions;
use dfdu_tui::{ColorLevel, IDLE_BEFORE_SORT, MIN_SORT_INTERVAL, Theme, UiOptions};
use ratatui::Terminal;
use ratatui::buffer::{Buffer, Cell};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/// How long without a frame counts as the loop having caught up with the finished scan. Longer
/// than any of its timed wakes: the spinner while busy and the automatic re-sort, which waits
/// for the longer of [`IDLE_BEFORE_SORT`] and [`MIN_SORT_INTERVAL`].
const QUIET: Duration = {
    let resort = if IDLE_BEFORE_SORT.as_nanos() > MIN_SORT_INTERVAL.as_nanos() {
        IDLE_BEFORE_SORT
    } else {
        MIN_SORT_INTERVAL
    };
    resort.saturating_add(Duration::from_millis(500))
};

#[derive(Default)]
struct Frames {
    at: Vec<Instant>,
    /// The text of the last frame.
    screen: String,
}

/// Frames flushed so far. The lock is released before an assertion can fail, so a failure does
/// not poison it for the run loop's thread.
fn count(frames: &Mutex<Frames>) -> usize {
    frames.lock().expect("frames").at.len()
}

/// Adds the frame in `buf` to `frames`.
fn record(frames: &Mutex<Frames>, buf: &Buffer) {
    let mut screen = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            screen.push_str(buf.cell((x, y)).map_or(" ", Cell::symbol));
        }
        screen.push('\n');
    }
    let mut f = frames.lock().expect("frames");
    f.at.push(Instant::now());
    f.screen = screen;
}

fn press(tx: &Sender<Event>, code: KeyCode) {
    let key = KeyEvent::new(code, KeyModifiers::NONE);
    tx.send(Event::Key(key)).expect("send key");
}

/// Scans a small temporary tree with the run loop drawing it, the debug panel open from the
/// start and the details pane opened with `i`; waits until the scan is done and the loop has
/// gone quiet, then hands the key sender and the recorded frames to `check`. The loop ends
/// when `check` drops the sender, also when it panics.
fn settled_with_panels(
    check: impl FnOnce(&Engine<dfdu_scan::NativeBackend>, Sender<Event>, &Mutex<Frames>),
) {
    let tmp = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).expect("temp"))
        .expect("tempdir");
    for d in 0..10 {
        let dir = tmp.path().join(format!("d{d}/sub"));
        fs::create_dir_all(&dir).expect("mkdir");
        for f in 0..30 {
            fs::write(dir.join(format!("f{f}")), vec![0u8; f * 100]).expect("write");
        }
    }
    let frames = Arc::new(Mutex::new(Frames::default()));
    let recorded = frames.clone();
    let mut terminal = Terminal::new(Recording::new(120, 24, move |buf| record(&recorded, buf)))
        .expect("terminal");
    let engine = Engine::start(
        dfdu_scan::native_backend(dfdu_sys::storage(tmp.path()), false),
        EngineOptions {
            workers: 2,
            journal_dir: None,
            crash_dir: None,
            ..Default::default()
        },
    );
    engine.send(Command::Scan {
        path: tmp.path().to_path_buf().into(),
        opts: JobOptions::default(),
    });
    let ui = UiOptions {
        #[cfg(unix)]
        peer: None,
        theme: Theme::no_color(),
        color_level: ColorLevel::None,
        background: None,
        mouse: false,
        debug: true,
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
    };
    thread::scope(|scope| {
        let (tx, rx) = unbounded();
        let run = scope.spawn(|| dfdu_tui::event_loop_with(&mut terminal, &engine, rx, ui));
        press(&tx, KeyCode::Char('i'));
        // Not `Engine::wait_idle`: it takes wakes from the channel the run loop waits on, and
        // a wake it takes is a change the loop never draws.
        let scan = Instant::now();
        while engine.read(|m| m.status.busy() || m.status.commands_applied == 0) {
            assert!(scan.elapsed() < Duration::from_secs(10), "scan");
            thread::sleep(Duration::from_millis(10));
        }
        let start = Instant::now();
        loop {
            let last = frames.lock().expect("frames").at.last().copied();
            let now = Instant::now();
            if now.duration_since(last.unwrap_or(start).max(start)) >= QUIET {
                break;
            }
            assert!(
                now < start + Duration::from_secs(10),
                "the loop still draws 10 s after the scan finished: {} frames",
                count(&frames)
            );
            thread::sleep(Duration::from_millis(10));
        }
        let screen = frames.lock().expect("frames").screen.clone();
        assert!(screen.contains("done in"), "{screen}");
        assert!(screen.contains("╭ Debug"), "{screen}");
        assert!(
            screen.contains("╭ d") && screen.contains("largest by"),
            "{screen}"
        );
        // The pane shows the scan state of the finished scan, not of a snapshot before it.
        assert!(screen.contains("directory · complete"), "{screen}");
        check(&engine, tx, &frames);
        run.join().expect("run loop thread").expect("run loop");
    });
    engine.shutdown(Duration::from_millis(200));
}

#[test]
fn no_frames_with_panels_open_after_scan_settles() {
    settled_with_panels(|engine, tx, frames| {
        let before = count(frames);
        thread::sleep(Duration::from_secs(2));
        assert_eq!(count(frames), before);
        // The panel shows the settled pipeline: everything sent has been received.
        let screen = frames.lock().expect("frames").screen.clone();
        let summary = dfdu_tui::debug_summary(engine);
        let first = summary.lines().next().expect("summary");
        assert!(first.ends_with("pending 0"), "{summary}");
        assert!(screen.contains(first), "{first:?} is not in\n{screen}");
        drop(tx);
    });
}

#[test]
fn moving_the_selection_with_panels_open_draws_one_frame() {
    settled_with_panels(|_, tx, frames| {
        let before = count(frames);
        let old = frames.lock().expect("frames").screen.clone();
        let sent = Instant::now();
        press(&tx, KeyCode::Down);
        while count(frames) == before {
            assert!(
                sent.elapsed() < Duration::from_millis(500),
                "no frame for the key"
            );
            thread::sleep(Duration::from_millis(1));
        }
        thread::sleep(Duration::from_secs(2));
        assert_eq!(count(frames), before + 1);
        // That one frame already shows the details of the newly selected entry.
        let new = frames.lock().expect("frames").screen.clone();
        let title = |s: &str| {
            s.lines()
                .find_map(|l| l.split_once("╭ d").map(|(_, t)| t[..1].to_string()))
        };
        assert_ne!(title(&old), title(&new), "{new}");
        drop(tx);
    });
}

#[test]
fn opening_a_directory_with_panels_open_draws_one_frame() {
    settled_with_panels(|_, tx, frames| {
        let before = count(frames);
        let sent = Instant::now();
        press(&tx, KeyCode::Enter);
        while count(frames) == before {
            assert!(
                sent.elapsed() < Duration::from_millis(500),
                "no frame for the key"
            );
            thread::sleep(Duration::from_millis(1));
        }
        thread::sleep(Duration::from_secs(2));
        // That one frame shows the details of the entry selected in the opened directory.
        let screen = frames.lock().expect("frames").screen.clone();
        assert!(screen.contains("╭ sub/ "), "{screen}");
        assert_eq!(count(frames), before + 1, "{screen}");
        drop(tx);
    });
}

#[test]
fn switching_to_the_treemap_draws_one_frame_then_nothing() {
    settled_with_panels(|_, tx, frames| {
        for code in [KeyCode::Char('2'), KeyCode::Right] {
            let before = count(frames);
            let sent = Instant::now();
            press(&tx, code);
            while count(frames) == before {
                assert!(
                    sent.elapsed() < Duration::from_millis(500),
                    "no frame for {code:?}"
                );
                thread::sleep(Duration::from_millis(1));
            }
            thread::sleep(Duration::from_secs(2));
            // That one frame already has the levels below the top: the read for them comes
            // before it, and nothing is read or drawn after it.
            let screen = frames.lock().expect("frames").screen.clone();
            assert!(screen.contains("zoom in"), "{screen}");
            assert!(screen.contains("sub/"), "{screen}");
            assert_eq!(count(frames), before + 1, "{screen}");
        }
        drop(tx);
    });
}
