#![cfg(unix)]
use dfdu_peer::{Peer, Role};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::Command,
    time::{Duration, Instant},
};

#[test]
fn ssh_child() {
    if std::env::var_os("DFDU_PEER_SSH_TEST").is_none() {
        return;
    }
    let slave = Peer::start(vec![]).unwrap();
    let master = Peer::start(vec![]).unwrap();
    master
        .bind(
            &format!("testhost:{}", slave.id()),
            dfdu_peer::DEFAULT_REMOTE_BIN,
        )
        .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while master.status().role != Role::Master || slave.status().role != Role::Slave {
        assert!(
            Instant::now() < until,
            "master {:?}; events {:?}",
            master.status(),
            master.events().try_iter().collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    master.unbind();
    while slave.status().role != Role::Unbound {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn ssh_command_and_bidirectional_stdio() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipped: python3 unavailable");
        return;
    }
    let dir = std::path::PathBuf::from(format!("/tmp/dpssh-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let script = dir.join("ssh");
    fs::write(
        &script,
        r#"#!/usr/bin/env python3
import os,sys,socket,select,shlex,json
assert sys.argv[1:5] == ['-T','-o','BatchMode=yes','testhost'],sys.argv
command = shlex.split(sys.argv[5])
assert command[0] == '$HOME/.local/bin/dfdu',command
assert command[1] == 'peer-attach',command
s=socket.socket(socket.AF_UNIX)
s.connect(os.path.join(os.environ['HOME'],'.local/state/dfdu/peers',command[2]+'.sock'))
while True:
    ready,_,_=select.select([s,sys.stdin.buffer],[],[])
    for source in ready:
        data=s.recv(4096) if source is s else os.read(0,4096)
        if not data: sys.exit(0)
        if source is s: os.write(1,data)
        else: s.sendall(data)
"#,
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "ssh_child", "--nocapture"])
        .env("HOME", &dir)
        .env("DFDU_SSH", &script)
        .env("DFDU_PEER_SSH_TEST", "1")
        .output()
        .unwrap();
    fs::remove_dir_all(dir).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
