#[cfg(unix)]
use dfdu_peer::{Peer, Role};
#[cfg(unix)]
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
fn main() {
    if let Some(id) = std::env::args().nth(1) {
        dfdu_peer::attach(&id).unwrap();
        return;
    }
    for close_input in [true, false] {
        let peer = Peer::start(vec![]).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg(peer.id())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        input
            .write_all(
                b"{\"hello\":{\"proto\":1,\"id\":\"attach-test\",\"host\":\"remote\",\"role\":\"master\"}}\n",
            )
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let hello: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(hello.as_object().unwrap().len(), 1);
        assert_eq!(hello["hello"]["role"], "slave");
        assert_eq!(hello["hello"]["id"], peer.id());
        assert!(
            hello["hello"]["host"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        );
        let until = Instant::now() + Duration::from_secs(4);
        while peer.status().role != Role::Slave {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
        if close_input {
            drop(input);
        } else {
            peer.unbind();
        }
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if Instant::now() >= until {
                let _ = child.kill();
                panic!("attach did not exit");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        while peer.status().role != Role::Unbound {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
