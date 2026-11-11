//! Exercise attach with the pipes an SSH server supplies, not a terminal.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
fn attach_delivers_stdout_while_stdin_and_socket_remain_open() {
    let home = tempfile::Builder::new()
        .prefix("dp-")
        .tempdir_in("/tmp")
        .unwrap();
    let dir = home.path().join(".local/state/dfdu/peers");
    fs::create_dir_all(&dir).unwrap();
    let listener = UnixListener::bind(dir.join("test.sock")).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_dfdu"))
        .args(["peer-attach", "test"])
        .env("HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let flags = rustix::fs::fcntl_getfl(&stdout).unwrap();
    rustix::fs::fcntl_setfl(&stdout, flags | rustix::fs::OFlags::NONBLOCK).unwrap();
    let (tx, rx) = mpsc::channel();
    let (start, wait) = mpsc::channel();
    let reader = thread::spawn(move || {
        if wait.recv().is_err() {
            return;
        }
        let mut bytes = [0; 6];
        let mut received = 0;
        while received < bytes.len() {
            match stdout.read(&mut bytes[received..]) {
                Ok(0) => break,
                Ok(n) => received += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
        let _ = tx.send((bytes, received));
    });
    let result = (|| {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return Err("attach did not connect".to_owned());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e.to_string()),
            }
        };
        socket.write_all(b"hello\n").unwrap();
        thread::sleep(Duration::from_millis(100));
        start.send(()).unwrap();
        // Linux's socket-to-pipe splice can leave the pipe locked while waiting
        // for more socket input. Even an SSH server's nonblocking read then hangs.
        let output = rx.recv_timeout(Duration::from_secs(2));
        // Keep both input sources open until stdout has delivered the entire line.
        drop(socket);
        output.map_err(|e| e.to_string())
    })();
    drop(start);
    let _ = child.kill();
    child.wait().unwrap();
    reader.join().unwrap();
    assert_eq!(result.unwrap(), (*b"hello\n", 6));
}
