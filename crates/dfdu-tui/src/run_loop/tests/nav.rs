//! Two real local sockets, engines and event loops. No mocked protocol events.
use super::*;

#[test]
fn selection_follows_and_sampled_match_returns_through_both_loops() {
    let source = tempfile::tempdir().unwrap();
    let backup = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("photo.bin"), b"identical photo bytes").unwrap();
    std::fs::write(backup.path().join("photo.bin"), b"identical photo bytes").unwrap();
    std::fs::write(source.path().join("z-photo.bin"), b"identical photo bytes").unwrap();
    std::fs::write(backup.path().join("z-photo.bin"), b"identical photo bytes").unwrap();
    let source = dunce::canonicalize(source.path()).unwrap();
    let backup = dunce::canonicalize(backup.path()).unwrap();
    let master = Arc::new(dfdu_peer::Peer::start(Vec::new()).unwrap());
    let slave = Arc::new(dfdu_peer::Peer::start(Vec::new()).unwrap());
    master
        .bind(slave.id(), dfdu_peer::DEFAULT_REMOTE_BIN)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while slave.status().role != dfdu_peer::Role::Slave {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let (master_tx, master_input) = unbounded();
    let (slave_tx, slave_input) = unbounded();
    std::thread::scope(|scope| {
        let master_run = scope.spawn(|| exercise(master, &source, master_input, true));
        let slave_run = scope.spawn(|| exercise(slave, &backup, slave_input, false));
        std::thread::sleep(Duration::from_millis(500));
        for event in [
            key('C'),
            key('j'),
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        ] {
            slave_tx.send(event).unwrap();
        }
        master_tx.send(key('j')).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        master_tx.send(key('k')).unwrap();
        std::thread::sleep(Duration::from_secs(1));
        master_tx.send(quit()).unwrap();
        slave_tx.send(quit()).unwrap();
        let (master_footer, master_selected, _, seq) = master_run.join().unwrap();
        let (_, selected, panel, _) = slave_run.join().unwrap();
        assert!(
            seq >= 3,
            "initial selection and two local moves must send nav"
        );
        assert_eq!(
            master_selected,
            source.join("photo.bin").display().to_string()
        );
        assert!(master_footer.contains('✓'), "{master_footer}");
        assert_eq!(selected, backup.join("photo.bin").display().to_string());
        assert!(panel.contains("photo.bin"));
        assert!(panel.contains("z-photo.bin"));
        assert!(panel.contains('✓'));
        assert!(panel.contains("21"));
    });
}

fn exercise(
    peer: Arc<dfdu_peer::Peer>,
    root: &std::path::Path,
    input: crossbeam_channel::Receiver<Event>,
    master: bool,
) -> (String, String, String, u64) {
    let engine = Engine::start(PortableBackend::default(), EngineOptions::default());
    engine.send(Command::Scan {
        path: root.into(),
        opts: JobOptions::default(),
    });
    engine.wait_idle(Some(Duration::from_secs(5)));
    let mut ui = UiOptions::from_env();
    ui.local = true;
    ui.peer = Some(crate::PeerOptions {
        peer,
        remote_bin: dfdu_peer::DEFAULT_REMOTE_BIN.into(),
    });
    let app = App::with_options(&ui, Instant::now());
    let mut session = Session::over(&engine, app, SessionOptions::default());
    let _ = session.tick(Instant::now()).unwrap();
    if master {
        let path = root.join("photo.bin").display().to_string();
        crate::peer::nav::jump(&mut session, &path);
    }
    let output = Arc::new(Mutex::new(Vec::new()));
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(Capture(output.clone())),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 160, 30)),
        },
    )
    .unwrap();
    let mut preview = Preview::new(io::sink(), false).unwrap();
    run(
        &mut terminal,
        &mut session,
        &input,
        false,
        &mut |_, _, _| -> io::Result<Result<(), String>> { panic!("no reopen") },
        &mut preview,
    )
    .unwrap();
    let selected = dfdu_session::selected_path::selected(&session.front().session)
        .unwrap()
        .0
        .to_string();
    let footer = session.front().nav.footer.clone();
    let panel = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
    let seq = session.front().nav.seq;
    session.shutdown(Duration::ZERO);
    (footer, selected, panel, seq)
}

struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
