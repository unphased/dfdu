//! A panic on the UI thread restores the terminal (mouse capture and raw mode off, alternate
//! screen left, cursor shown) and then writes a crash report with the panic, the engine's
//! counters and its last messages. Quitting restores the terminal in the same order.
//!
//! Runs the real `dfdu` binary on a pseudo-terminal. The environment variable
//! `DFDU_TEST_UI_PANIC` (hidden; read by the event loop in `dfdu_tui`) makes the browser panic on
//! the first key press. The pseudo-terminal never answers the background-colour query, so the
//! browser is started with an explicit theme, which skips it.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustix::fs::{Mode, OFlags};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{LocalModes, Winsize, tcgetattr, tcsetwinsize};

const ENTER_ALT: &[u8] = b"\x1b[?1049h";
const LEAVE_ALT: &[u8] = b"\x1b[?1049l";
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
/// The first sequence crossterm writes to enable mouse capture, and the last it writes to
/// disable it.
const MOUSE_ON: &[u8] = b"\x1b[?1000h";
const MOUSE_OFF: &[u8] = b"\x1b[?1000l";

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

fn wait_until(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `dfdu` running on a pseudo-terminal, with everything it wrote to it so far.
struct Browser {
    child: Child,
    master: OwnedFd,
    slave: OwnedFd,
    out: Arc<Mutex<Vec<u8>>>,
}

impl Browser {
    /// Starts the browser on `tree` and waits for its first frame.
    fn start(tree: &Path, state: &Path, panic_on_key: bool) -> Browser {
        Self::start_options(tree, state, panic_on_key, false)
    }

    fn start_options(tree: &Path, state: &Path, panic_on_key: bool, peer: bool) -> Browser {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let name = ptsname(&master, Vec::new()).unwrap();
        let slave = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY,
            Mode::empty(),
        )
        .unwrap();
        let size = Winsize {
            ws_row: 30,
            ws_col: 100,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        tcsetwinsize(&master, size).unwrap();
        let cooked = tcgetattr(&slave).unwrap().local_modes;
        assert!(cooked.contains(LocalModes::ICANON | LocalModes::ECHO));

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_dfdu"));
        cmd.args(["--theme", "ansi"])
            .arg(tree)
            .env("DFDU_STATE_DIR", state)
            .env("DFDU_CACHE_DIR", state.join("cache"))
            .env("TERM", "xterm-256color")
            .env("RUST_BACKTRACE", "0")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave.try_clone().unwrap()));
        if panic_on_key {
            cmd.env("DFDU_TEST_UI_PANIC", "1");
        }
        if peer {
            cmd.arg("--peer").env("HOME", state);
        }
        let child = cmd.spawn().unwrap();

        // Everything the child writes to the terminal. The reader ends when the slave side
        // closes.
        let out = Arc::new(Mutex::new(Vec::new()));
        let mut reader = File::from(master.try_clone().unwrap());
        let o = out.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                o.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });

        let browser = Browser {
            child,
            master,
            slave,
            out,
        };
        // The browser is up: alternate screen entered (after raw mode was set) and a frame
        // drawn.
        wait_until("the first frame", Duration::from_secs(20), || {
            let o = browser.output();
            find(&o, ENTER_ALT).is_some_and(|i| find(&o[i..], b"d19").is_some())
        });
        let raw = browser.local_modes();
        assert!(
            !raw.intersects(LocalModes::ICANON | LocalModes::ECHO),
            "raw mode during the run: {raw:?}"
        );
        browser
    }

    fn output(&self) -> Vec<u8> {
        self.out.lock().unwrap().clone()
    }

    fn local_modes(&self) -> LocalModes {
        tcgetattr(&self.slave).unwrap().local_modes
    }

    fn press(&self, key: &[u8]) {
        File::from(self.master.try_clone().unwrap())
            .write_all(key)
            .unwrap();
    }

    /// Waits for the process to exit and checks that it left the terminal in cooked mode.
    fn wait(&mut self) -> std::process::ExitStatus {
        let mut status = None;
        wait_until("dfdu to exit", Duration::from_secs(20), || {
            status = self.child.try_wait().unwrap();
            status.is_some()
        });
        let restored = self.local_modes();
        assert!(
            restored.contains(LocalModes::ICANON | LocalModes::ECHO),
            "raw mode after exit: {restored:?}"
        );
        status.unwrap()
    }
}

fn tree(tmp: &Path) -> std::path::PathBuf {
    let tree = tmp.join("tree");
    for d in 0..20 {
        let dir = tree.join(format!("d{d}"));
        fs::create_dir_all(&dir).unwrap();
        for f in 0..20 {
            fs::write(dir.join(format!("f{f}")), vec![0u8; 100 * f]).unwrap();
        }
    }
    tree
}

/// Checks the order of the start and exit sequences in `o`: mouse capture is turned on after
/// the alternate screen is entered, and off again before it is left.
#[track_caller]
fn assert_mouse_capture_ends_first(o: &[u8]) -> (usize, usize) {
    let text = String::from_utf8_lossy(o);
    let enter = find(o, ENTER_ALT).unwrap_or_else(|| panic!("alternate screen: {text:?}"));
    let on = rfind(o, MOUSE_ON).unwrap_or_else(|| panic!("mouse capture on: {text:?}"));
    let off = rfind(o, MOUSE_OFF).unwrap_or_else(|| panic!("mouse capture off: {text:?}"));
    let leave = rfind(o, LEAVE_ALT).unwrap_or_else(|| panic!("alternate screen left: {text:?}"));
    assert!(enter < on && on < off && off < leave, "{text:?}");
    (off, leave)
}

#[test]
fn ui_panic_restores_terminal_and_writes_crash_file() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tree(tmp.path());
    let state = tmp.path().join("state");
    let mut browser = Browser::start(&tree, &state, true);
    let drawn = browser.output().len();

    browser.press(b"x");
    assert!(!browser.wait().success());

    wait_until("the crash report notice", Duration::from_secs(5), || {
        find(&browser.output(), b"crash report written").is_some()
    });
    let all = browser.output();
    assert_mouse_capture_ends_first(&all);
    let o = all[drawn..].to_vec();
    let text = String::from_utf8_lossy(&o);
    let leave = find(&o, LEAVE_ALT).unwrap_or_else(|| panic!("alternate screen left: {text:?}"));
    let show = find(&o, SHOW_CURSOR).unwrap_or_else(|| panic!("cursor shown: {text:?}"));
    let mouse_off = find(&o, MOUSE_OFF).unwrap_or_else(|| panic!("mouse capture off: {text:?}"));
    let message = find(&o, b"injected UI panic").unwrap_or_else(|| panic!("message: {text:?}"));
    let notice = find(&o, b"crash report written").unwrap();
    // The terminal is restored before anything is printed.
    assert!(
        leave < message && show < message && mouse_off < message && leave < notice,
        "{text:?}"
    );

    let reports: Vec<_> = fs::read_dir(state.join("crash"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(reports.len(), 1, "{reports:?}");
    let report = fs::read_to_string(&reports[0]).unwrap();
    for want in [
        "thread: main",
        "injected UI panic",
        "backtrace:",
        "stats: Stats {",
        "messages and commands, oldest first:",
        "Command(\"Scan\")",
        "Chunk {",
        "Close {",
    ] {
        assert!(report.contains(want), "{want:?} missing from:\n{report}");
    }
}

#[test]
fn peer_registration_is_removed_on_quit_panic_and_termination_signals() {
    for mode in ["quit", "panic", "term", "hup", "int"] {
        let tmp = tempfile::Builder::new()
            .prefix("dp-")
            .tempdir_in("/tmp")
            .unwrap();
        let tree = tree(tmp.path());
        let state = tmp.path().join("state");
        let mut browser = Browser::start_options(&tree, &state, mode == "panic", true);
        let peers = state.join(".local/state/dfdu/peers");
        assert_eq!(fs::read_dir(&peers).unwrap().count(), 2, "{mode}");
        match mode {
            "quit" => {
                browser.press(b"q");
                assert!(browser.wait().success());
            }
            "panic" => {
                browser.press(b"x");
                assert!(!browser.wait().success());
            }
            _ => {
                let signal = match mode {
                    "term" => rustix::process::Signal::TERM,
                    "hup" => rustix::process::Signal::HUP,
                    _ => rustix::process::Signal::INT,
                };
                rustix::process::kill_process(
                    rustix::process::Pid::from_raw(browser.child.id() as i32).unwrap(),
                    signal,
                )
                .unwrap();
                wait_until("signal termination", Duration::from_secs(5), || {
                    browser.child.try_wait().unwrap().is_some()
                });
            }
        }
        assert_eq!(fs::read_dir(&peers).unwrap().count(), 0, "{mode}");
    }
}

#[test]
fn quit_restores_terminal() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tree(tmp.path());
    let mut browser = Browser::start(&tree, &tmp.path().join("state"), false);
    browser.press(b"q");
    assert!(browser.wait().success());
    // The process can exit before the reader has read all it wrote.
    let shown_after_leave = |o: &[u8]| {
        rfind(o, LEAVE_ALT).is_some_and(|leave| rfind(o, SHOW_CURSOR).is_some_and(|s| s > leave))
    };
    let mut o = Vec::new();
    let waited = Instant::now();
    while !shown_after_leave(&o) && waited.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
        o = browser.output();
    }
    assert!(
        shown_after_leave(&o),
        "cursor shown after the alternate screen is left: {:?}",
        String::from_utf8_lossy(&o)
    );
    assert_mouse_capture_ends_first(&o);
}
