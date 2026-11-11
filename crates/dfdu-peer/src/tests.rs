use super::*;

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
