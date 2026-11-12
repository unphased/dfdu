use super::*;

// Environment overrides belong to a child process, never the parallel test runner.
pub(super) fn isolated(name: &str) -> bool {
    if std::env::var("DFDU_BG_TEST").as_deref() == Ok(name) {
        return false;
    }
    use std::net::UdpSocket;
    use std::os::unix::fs::PermissionsExt;
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let port = socket.local_addr().unwrap().port();
    let tmp = tempfile::tempdir().unwrap();
    let tailscale = tmp.path().join("tailscale");
    std::fs::write(&tailscale, "#!/bin/sh\nprintf '%s' '{\"Peer\":{\"fake\":{\"Online\":true,\"TailscaleIPs\":[\"127.0.0.1\"]}}}'\n").unwrap();
    std::fs::set_permissions(&tailscale, std::fs::Permissions::from_mode(0o755)).unwrap();
    let responder = std::thread::spawn(move || {
        let mut bytes = [0; 1024];
        if let Ok((_, source)) = socket.recv_from(&mut bytes) {
            socket.send_to(br#"{"host":"fake-nas","user":"slu","ids":["remote"]}"#, source).unwrap();
        }
    });
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("DFDU_BG_TEST", name)
        .env("DFDU_DISCOVERY_PORT", port.to_string())
        .env("DFDU_TAILSCALE", tailscale)
        .output().unwrap();
    responder.join().unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    true
}

#[test]
fn background_discovery_redraws_idle_footer_without_keys() {
    if isolated("run_loop::tests::discovery::background_discovery_redraws_idle_footer_without_keys") { return; }
    struct FooterWriter {
        bytes: Vec<u8>,
        found: Arc<Mutex<bool>>,
        input: Sender<Event>,
    }
    impl Write for FooterWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            if String::from_utf8_lossy(&self.bytes).contains("fake-nas found") {
                *self.found.lock().unwrap() = true;
                self.input.send(quit()).unwrap();
            }
            self.bytes.clear();
            Ok(())
        }
    }
    let peer = Arc::new(dfdu_peer::Peer::start(Vec::new()).unwrap());
    let mut ui = UiOptions::from_env();
    ui.peer = Some(crate::PeerOptions { peer, remote_bin: dfdu_peer::DEFAULT_REMOTE_BIN.into() });
    let engine = Engine::start(PortableBackend::default(), EngineOptions::default());
    let mut session = Session::over(&engine, App::with_options(&ui, Instant::now()), SessionOptions::default());
    let (input_tx, input) = unbounded();
    let found = Arc::new(Mutex::new(false));
    let backend = CrosstermBackend::new(FooterWriter { bytes: Vec::new(), found: found.clone(), input: input_tx.clone() });
    let mut terminal = Terminal::with_options(backend, TerminalOptions { viewport: Viewport::Fixed(Rect::new(0, 0, 160, 24)) }).unwrap();
    let mut previews = Preview::new(io::sink(), false).unwrap();
    let (done, finished) = bounded(1);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            if finished.recv_timeout(Duration::from_secs(5)).is_err() { let _ = input_tx.send(quit()); }
        });
        run(&mut terminal, &mut session, &input, false, &mut |_, _, _| Ok(Ok(())), &mut previews).unwrap();
        let _ = done.send(());
    });
    session.shutdown(Duration::ZERO);
    engine.shutdown(Duration::from_millis(200));
    assert!(*found.lock().unwrap(), "discovery completion must redraw the idle footer");
}
