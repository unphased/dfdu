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
    let mut link = Link::new(a, None, Role::Slave, "local").unwrap();
    link.ready = ready;
    b.write_all(bytes).unwrap();
    link.poll()
}
#[test]
fn protocol_requires_version_role_hello_and_bounded_lines() {
    assert_eq!(
        wire(
            b"{\"type\":\"hello\",\"proto\":1,\"id\":\"other\",\"role\":\"master\"}\n",
            false
        )
        .unwrap(),
        Some("other".into())
    );
    for bytes in [
        b"{\"type\":\"hello\",\"proto\":2,\"id\":\"x\",\"role\":\"master\"}\n".as_slice(),
        b"{\"type\":\"hello\",\"proto\":1,\"id\":\"x\",\"role\":\"slave\"}\n",
        b"{\"type\":\"ping\"}\n",
        b"garbage\n",
        b"{\"type\":\"bye\"}\n",
    ] {
        assert!(wire(bytes, false).is_err());
    }
    assert!(wire(b"{\"type\":\"ping\"}\n", true).is_ok());
    let (a, mut b) = UnixStream::pair().unwrap();
    let mut link = Link::new(a, None, Role::Slave, "local").unwrap();
    link.input.resize(LIMIT, b'x');
    b.write_all(b"x").unwrap();
    assert!(link.poll().unwrap_err().to_string().contains("64 KiB"));
}
#[test]
fn timeout_applies_to_handshake_and_bound_connections() {
    for ready in [false, true] {
        let (a, _b) = UnixStream::pair().unwrap();
        let mut link = Link::new(a, None, Role::Master, "local").unwrap();
        link.ready = ready;
        link.last = Instant::now() - TIMEOUT;
        assert!(link.poll().unwrap_err().to_string().contains("timed out"));
    }
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
    // A connectable socket is retained even if the PID is gone.
    assert_eq!(list_in(&dir).unwrap().len(), 2);
    drop(listener);
    // Missing endpoints are definitively stale on all Unix platforms.
    fs::remove_file(&stale_socket).unwrap();
    assert_eq!(list_in(&dir).unwrap().len(), 1);
    assert!(!stale_socket.exists());
    // A living PID is retained even if its socket is missing.
    stale.pid = std::process::id();
    metadata(&dir, &stale).unwrap();
    assert_eq!(list_in(&dir).unwrap().len(), 2);
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
    assert_eq!(a.len(), 32);
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
        pid: i32::MAX as u32,
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
    let mut master = Link::new(a, None, Role::Master, "a").unwrap();
    let mut slave = Link::new(b, None, Role::Slave, "b").unwrap();
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
