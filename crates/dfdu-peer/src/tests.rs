use super::*;
use std::sync::atomic::AtomicU64;
static NEXT: AtomicU64 = AtomicU64::new(0);

fn dir() -> PathBuf {
    let path = PathBuf::from(format!(
        "/tmp/dp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
fn wire(bytes: &[u8], ready: bool) -> io::Result<Option<String>> {
    let (a, mut b) = UnixStream::pair().unwrap();
    let mut link = Link::new(a, None, Role::Slave, "local", "host").unwrap();
    link.ready = ready;
    b.write_all(bytes).unwrap();
    link.poll()
}
#[test]
fn protocol_requires_version_role_hello_and_bounded_lines() {
    assert_eq!(
        wire(
            b"{\"hello\":{\"proto\":1,\"id\":\"other\",\"host\":\"remote\",\"role\":\"master\"}}\n",
            false
        )
        .unwrap(),
        Some("remote/other".into())
    );
    for bytes in [
        b"{\"hello\":{\"proto\":2,\"id\":\"x\",\"host\":\"remote\",\"role\":\"master\"}}\n"
            .as_slice(),
        b"{\"hello\":{\"proto\":1,\"id\":\"x\",\"host\":\"remote\",\"role\":\"slave\"}}\n",
        b"{\"ping\":{}}\n",
        b"garbage\n",
        b"{\"bye\":{}}\n",
    ] {
        assert!(wire(bytes, false).is_err());
    }
    assert!(wire(b"{\"ping\":{}}\n", true).is_ok());
    let (a, mut b) = UnixStream::pair().unwrap();
    let mut link = Link::new(a, None, Role::Slave, "local", "host").unwrap();
    link.input.resize(LIMIT, b'x');
    b.write_all(b"x").unwrap();
    assert!(link.poll().unwrap_err().to_string().contains("64 KiB"));
}
#[test]
fn exact_wire_envelopes() {
    let (a, _b) = UnixStream::pair().unwrap();
    let link = Link::new(a, None, Role::Master, "abcdef12", "actual-host").unwrap();
    assert_eq!(
        std::str::from_utf8(&link.output).unwrap(),
        "{\"hello\":{\"proto\":1,\"id\":\"abcdef12\",\"host\":\"actual-host\",\"role\":\"master\"}}\n"
    );
    assert_eq!(
        serde_json::to_string(&Frame::Bye { reason: None }).unwrap(),
        "{\"bye\":{}}"
    );
    assert_eq!(
        serde_json::to_string(&Frame::Ping {}).unwrap(),
        "{\"ping\":{}}"
    );
    assert!(
        wire(
            b"{\"hello\":{\"proto\":1,\"id\":\"x\",\"role\":\"master\"}}\n",
            false
        )
        .is_err()
    );
    assert!(
        wire(
            b"{\"type\":\"hello\",\"proto\":1,\"id\":\"x\",\"host\":\"h\",\"role\":\"master\"}\n",
            false
        )
        .is_err()
    );
}
#[test]
fn panic_cleanup_does_not_wait_for_worker_or_bridge_acknowledgment() {
    if std::env::var_os("DFDU_PEER_PANIC_CHILD").is_some() {
        let real = Peer::start_in(dir(), Vec::new()).unwrap();
        let instance = real.instance.clone();
        let dir = real.dir.clone();
        drop(real);
        // An unserviced control channel models both the panicking worker itself
        // and a bridge that the worker must join before it can acknowledge Stop.
        let (control, commands) = unbounded();
        let (_, events) = unbounded();
        let peer = Peer {
            instance,
            dir,
            status: Arc::new(Mutex::new(Status::default())),
            pending: Arc::new(Mutex::new(Pending::default())),
            events,
            control,
            worker: None,
        };
        let cleanup = peer.cleanup_hook();
        std::panic::set_hook(Box::new(move |_| cleanup()));
        for name in ["peer-worker", "peer-bridge"] {
            assert!(
                thread::Builder::new()
                    .name(name.into())
                    .spawn(|| panic!("injected transport panic"))
                    .unwrap()
                    .join()
                    .is_err()
            );
            assert!(matches!(commands.try_recv(), Ok(Control::Stop(_))));
        }
        return;
    }
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::panic_cleanup_does_not_wait_for_worker_or_bridge_acknowledgment",
        ])
        .env("DFDU_PEER_PANIC_CHILD", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("panic cleanup waited for a worker or bridge acknowledgment");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn cleanup_hook_and_panic_unlink_registrations() {
    let dir = dir();
    let peer = Peer::start_in(dir.clone(), vec![]).unwrap();
    let path = socket(&dir, peer.id()).unwrap();
    let json = dir.join(format!("{}.json", peer.id()));
    peer.cleanup_hook()();
    let until = Instant::now() + Duration::from_secs(2);
    while path.exists() || json.exists() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(10));
    }
    drop(peer);
    let paths = Arc::new(Mutex::new(None));
    let capture = paths.clone();
    assert!(
        std::panic::catch_unwind(move || {
            let peer = Peer::start_in(dir.clone(), vec![]).unwrap();
            *capture.lock().unwrap() = Some((dir.clone(), peer.id().to_owned()));
            panic!("exercise peer unwinding");
        })
        .is_err()
    );
    let (dir, id) = paths.lock().unwrap().clone().unwrap();
    assert!(!socket(&dir, &id).unwrap().exists());
    assert!(!dir.join(format!("{id}.json")).exists());
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn pending_output_is_bounded_for_nonreading_peers() {
    let (a, _b) = UnixStream::pair().unwrap();
    let mut link = Link::new(a, None, Role::Master, "local", "host").unwrap();
    link.ready = true;
    link.output.resize(LIMIT + 1, b'x');
    assert!(
        link.poll()
            .unwrap_err()
            .to_string()
            .contains("pending output")
    );
}

#[test]
fn timeout_applies_to_handshake_and_bound_connections() {
    for (ready, idle) in [(false, HANDSHAKE_TIMEOUT), (true, TIMEOUT)] {
        let (a, _b) = UnixStream::pair().unwrap();
        let mut link = Link::new(a, None, Role::Master, "local", "host").unwrap();
        link.ready = ready;
        link.last = Instant::now() - idle;
        assert!(link.poll().unwrap_err().to_string().contains("timed out"));
    }
}
#[test]
fn handshake_waits_past_bound_timeout_for_slow_ssh_auth() {
    let (a, _b) = UnixStream::pair().unwrap();
    let mut link = Link::new(a, None, Role::Master, "local", "host").unwrap();
    link.last = Instant::now() - TIMEOUT;
    assert!(link.poll().is_ok());
}
#[test]
fn registration_modes_stale_cleanup_and_path_guard() {
    let dir = dir();
    let peer = Peer::start_in(dir.clone(), vec![PathBuf::from("root")]).unwrap();
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(socket(&dir, peer.id()).unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let mut stale = peer.instance.clone();
    stale.id = "stale".into();
    stale.pid = i32::MAX as u32;
    metadata(&dir, &stale).unwrap();
    let stale_socket = socket(&dir, &stale.id).unwrap();
    let listener = UnixListener::bind(&stale_socket).unwrap();
    // A dead PID is removed even while its endpoint is connectable.
    assert_eq!(list_in(&dir).unwrap().len(), 1);
    assert!(!stale_socket.exists());
    drop(listener);
    // A dead endpoint must also be removed on Darwin, where refusal alone is ambiguous.
    metadata(&dir, &stale).unwrap();
    drop(UnixListener::bind(&stale_socket).unwrap());
    assert_eq!(list_in(&dir).unwrap().len(), 1);
    assert!(!stale_socket.exists());
    // Missing endpoints are stale even when the PID is alive.
    stale.pid = std::process::id();
    metadata(&dir, &stale).unwrap();
    assert_eq!(list_in(&dir).unwrap().len(), 1);
    assert!(socket(Path::new(&"x".repeat(104)), "a").is_err());
    assert!(socket(&dir, "../bad").is_err());
    drop(peer);
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn transport_drop_cancels_blocked_bridges_and_reaps_child() {
    let mut child = Command::new("sh")
        .args(["-c", "sleep 60 & wait"])
        .process_group(0)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    nonblocking(&output).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    let worker = thread::spawn(move || pump(output, io::sink(), signal));
    let pid = rustix::process::Pid::from_raw(child.id() as i32).unwrap();
    let transport = Transport {
        child,
        stop,
        threads: vec![worker],
    };
    let start = Instant::now();
    drop(transport);
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(
        rustix::process::test_kill_process(pid),
        Err(rustix::io::Errno::SRCH)
    );
}
#[test]
fn random_ids_and_conservative_stale_errors() {
    let a = random_id().unwrap();
    assert_eq!(a.len(), 8);
    assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_ne!(a, random_id().unwrap());
    for code in [
        libc::EACCES,
        libc::EMFILE,
        libc::ENFILE,
        libc::EAGAIN,
        libc::ETIMEDOUT,
    ] {
        assert!(!stale_error(&io::Error::from_raw_os_error(code)));
    }
    assert!(stale_error(&io::Error::from_raw_os_error(libc::ENOENT)));
    assert_eq!(
        stale_error(&io::Error::from_raw_os_error(libc::ECONNREFUSED)),
        cfg!(target_os = "linux")
    );
}
#[test]
fn full_backlog_is_bounded_and_retained() {
    let dir = dir();
    let path = socket(&dir, "full").unwrap();
    let listener = UnixListener::bind(&path).unwrap();
    // Reduce the backlog so this exercises saturation on both macOS and Linux.
    rustix::net::listen(&listener, 1).unwrap();
    let mut connections = vec![];
    let start = Instant::now();
    loop {
        match connect_local(&path) {
            Ok(stream) => connections.push(stream),
            Err(e) => {
                assert!(matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::ConnectionRefused
                ));
                break;
            }
        }
        assert!(connections.len() < 128);
    }
    assert!(start.elapsed() < Duration::from_secs(3));
    let instance = Instance {
        id: "full".into(),
        host: "test".into(),
        pid: std::process::id(),
        roots: vec![],
        started: 0,
        role: Role::Unbound,
    };
    metadata(&dir, &instance).unwrap();
    assert_eq!(list_in(&dir).unwrap().len(), 1);
    assert!(path.exists());
    let peer = Peer::start_in(dir.clone(), vec![]).unwrap();
    peer.bind("full", DEFAULT_REMOTE_BIN).unwrap();
    let start = Instant::now();
    drop(peer);
    assert!(start.elapsed() < Duration::from_secs(2));
    drop(listener);
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn heartbeat_keeps_real_pair_alive_beyond_timeout() {
    let (a, b) = UnixStream::pair().unwrap();
    let mut master = Link::new(a, None, Role::Master, "a", "host").unwrap();
    let mut slave = Link::new(b, None, Role::Slave, "b", "host").unwrap();
    let start = Instant::now();
    while start.elapsed() < TIMEOUT + Duration::from_secs(1) {
        master.poll().unwrap();
        slave.poll().unwrap();
        thread::sleep(Duration::from_millis(10));
    }
    assert!(master.ready && slave.ready);
}
#[test]
fn target_parsing_rejects_shell_and_option_injection() {
    assert_eq!(
        parse_target("host abc").unwrap(),
        (Some("host".into()), "abc".into())
    );
    assert_eq!(
        parse_target("host:abc").unwrap(),
        (Some("host".into()), "abc".into())
    );
    for target in [
        "-ohost abc",
        "host;evil abc",
        "host:a/b",
        "host abc extra",
        "",
    ] {
        assert!(parse_target(target).is_err());
    }
    assert_eq!(quote("a'b"), "'a'\\''b'");
}

fn nav(seq: u64) -> Nav {
    Nav {
        seq,
        path: "/source/a".into(),
        name: "a".into(),
        kind: NavKind::File,
        size: 42,
        mtime: -1,
        parents: vec!["source".into()],
    }
}
#[test]
fn navigation_wire_and_direction() {
    let encoded = serde_json::to_string(&Frame::Nav(nav(7))).unwrap();
    assert!(encoded.starts_with("{\"nav\":{\"seq\":7,"));
    assert!(encoded.contains("\"kind\":\"file\""));
    assert!(wire(format!("{encoded}\n").as_bytes(), true).is_ok());
    assert!(wire(format!("{encoded}\n").as_bytes(), false).is_err());
    let status = Frame::NavStatus(NavStatus {
        seq: 7,
        verdict: Verdict::Match,
    });
    assert!(
        wire(
            format!("{}\n", serde_json::to_string(&status).unwrap()).as_bytes(),
            true
        )
        .is_err()
    );
}
#[test]
fn local_navigation_latest_sequence_and_status() {
    let dir = dir();
    let master = Peer::start_in(dir.clone(), vec![]).unwrap();
    let slave = Peer::start_in(dir.clone(), vec![]).unwrap();
    assert!(master.send_nav(nav(1)).is_err());
    master.bind(slave.id(), DEFAULT_REMOTE_BIN).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while master.status().role != Role::Master || slave.status().role != Role::Slave {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(slave.send_nav(nav(1)).is_err());
    assert!(
        master
            .send_nav_status(NavStatus {
                seq: 1,
                verdict: Verdict::Match
            })
            .is_err()
    );
    master.send_nav(nav(2)).unwrap();
    master.send_nav(nav(1)).unwrap();
    master
        .send_nav_hash(NavHash {
            seq: 2,
            hash: "abcd".into(),
        })
        .unwrap();
    loop {
        if let Event::Nav(value) = slave.events().recv_timeout(Duration::from_secs(3)).unwrap() {
            assert_eq!(value.seq, 2);
            break;
        }
    }
    loop {
        if let Event::NavHash(value) = slave.events().recv_timeout(Duration::from_secs(3)).unwrap()
        {
            assert_eq!(value.seq, 2);
            break;
        }
    }
    slave
        .send_nav_status(NavStatus {
            seq: 2,
            verdict: Verdict::Match,
        })
        .unwrap();
    loop {
        if let Event::NavStatus(value) = master
            .events()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
        {
            assert_eq!(value.verdict, Verdict::Match);
            break;
        }
    }
    drop(master);
    drop(slave);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn navigation_receive_coalesces_and_discards_stale_work() {
    let (a, mut b) = UnixStream::pair().unwrap();
    let mut link = Link::new(a, None, Role::Slave, "local", "host").unwrap();
    link.ready = true;
    for frame in [
        Frame::Nav(nav(1)),
        Frame::NavHash(NavHash {
            seq: 1,
            hash: "old".into(),
        }),
        Frame::Nav(nav(3)),
        Frame::Nav(nav(2)),
        Frame::NavHash(NavHash {
            seq: 1,
            hash: "stale".into(),
        }),
        Frame::NavHash(NavHash {
            seq: 3,
            hash: "current".into(),
        }),
    ] {
        writeln!(b, "{}", serde_json::to_string(&frame).unwrap()).unwrap();
    }
    link.poll().unwrap();
    assert_eq!(link.incoming.nav.take().unwrap().seq, 3);
    assert_eq!(link.incoming.hash.take().unwrap().hash, "current");
    let (a, mut b) = UnixStream::pair().unwrap();
    let mut link = Link::new(a, None, Role::Master, "local", "host").unwrap();
    link.ready = true;
    link.sent_seq = Some(3);
    for seq in [1, 3, 2] {
        writeln!(
            b,
            "{}",
            serde_json::to_string(&Frame::NavStatus(NavStatus {
                seq,
                verdict: Verdict::Match
            }))
            .unwrap()
        )
        .unwrap();
    }
    link.poll().unwrap();
    assert_eq!(link.incoming.status.take().unwrap().seq, 3);
}
#[test]
fn navigation_mailbox_is_latest_only_and_rejects_oversized_frames() {
    let mut pending = Pending::default();
    for seq in 0..1000 {
        pending.nav(nav(seq));
    }
    pending.nav(nav(3));
    assert_eq!(pending.nav.unwrap().seq, 999);
    let mut huge = nav(1);
    huge.path = "x".repeat(LIMIT);
    assert!(check_size(&Frame::Nav(huge)).is_err());
    assert_eq!(
        serde_json::to_string(&Frame::NavHash(NavHash {
            seq: 1,
            hash: "abc".into()
        }))
        .unwrap(),
        "{\"nav_hash\":{\"seq\":1,\"hash\":\"abc\"}}"
    );
    assert_eq!(
        serde_json::to_string(&Frame::NavStatus(NavStatus {
            seq: 1,
            verdict: Verdict::Missing
        }))
        .unwrap(),
        "{\"nav_status\":{\"seq\":1,\"verdict\":\"missing\"}}"
    );
}
