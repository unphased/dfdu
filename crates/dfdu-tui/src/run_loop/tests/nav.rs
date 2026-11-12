//! Two real local sockets, engines and event loops. No mocked protocol events.
use super::*;

#[path = "../../../tests/common/mod.rs"]
mod common;

#[test]
fn selection_follows_and_sampled_match_returns_through_both_loops() {
    navigation_pair(false);
}

#[test]
fn swap_reverses_navigation_through_both_loops() {
    navigation_pair(true);
}

fn navigation_pair(swap: bool) {
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
        assert!(
            Instant::now() < deadline,
            "slave did not bind within 5 seconds"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let (master_tx, master_input) = unbounded();
    let (slave_tx, slave_input) = unbounded();
    let master_frame = Arc::new(Mutex::new((String::new(), String::new(), String::new())));
    let slave_frame = Arc::new(Mutex::new((String::new(), String::new(), String::new())));
    std::thread::scope(|scope| {
        let master_run =
            scope.spawn(|| exercise(master, &source, master_input, true, &master_frame));
        let slave_run = scope.spawn(|| exercise(slave, &backup, slave_input, false, &slave_frame));
        let ready = (|| {
            wait_for_match(&master_frame, &slave_frame, "photo.bin")?;
            for event in [
                key('C'),
                key('k'),
                Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            ] {
                slave_tx.send(event).unwrap();
            }
            if swap {
                master_tx.send(key('W')).unwrap();
                wait_for_match(&slave_frame, &master_frame, "photo.bin")?;
                let a = master_frame.lock().unwrap().1.clone();
                let b = slave_frame.lock().unwrap().1.clone();
                if !a.contains("slave ←") || !b.contains("master →") {
                    return Err(format!("roles did not reverse: {a:?} {b:?}"));
                }
                slave_tx.send(key('j')).unwrap();
                return wait_for_match(&slave_frame, &master_frame, "z-photo.bin");
            }
            master_tx.send(key('j')).unwrap();
            wait_for_match(&master_frame, &slave_frame, "z-photo.bin")
        })();
        master_tx.send(quit()).unwrap();
        slave_tx.send(quit()).unwrap();
        if let Err(e) = &ready {
            eprintln!("{e}");
        }
        let (master_footer, master_selected, _, seq) = master_run.join().unwrap();
        let (_, selected, panel, _) = slave_run.join().unwrap();
        ready.unwrap();
        assert!(
            seq >= if swap { 1 } else { 2 },
            "initial selection and local movement must send nav"
        );
        assert_eq!(
            master_selected,
            source.join("z-photo.bin").display().to_string()
        );
        if !swap {
            assert!(master_footer.contains('✓'), "{master_footer}");
        }
        assert_eq!(selected, backup.join("z-photo.bin").display().to_string());
        if !swap {
            assert!(panel.contains("photo.bin"));
            assert!(panel.contains("z-photo.bin"));
            assert!(panel.contains('✓'));
            assert!(panel.contains("21"));
        }
    });
}

// Observe complete frames, not elapsed scan or hash time. Return before panicking so the
// caller can stop and join both event loops even when a condition never arrives.
fn wait_for_match(
    master: &Mutex<(String, String, String)>,
    slave: &Mutex<(String, String, String)>,
    expected: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let master_frame = master.lock().unwrap().clone();
        let slave_frame = slave.lock().unwrap().clone();
        if master_frame.0 == expected
            && slave_frame.2.lines().any(|line| {
                line.contains("> ✓ ")
                    && line.split_whitespace().any(|word| {
                        std::path::Path::new(word)
                            .file_name()
                            .is_some_and(|name| name == expected)
                    })
            })
            && master_frame.1.contains('✓')
            && master_frame.1.contains(expected)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let tail = |text: &str| {
                let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
                lines[lines.len().saturating_sub(8)..].join("\n")
            };
            return Err(format!(
                "timed out after 20 seconds waiting for both selections to be {expected:?} and the master footer to show ✓\n\
                 master selected: {:?}\nmaster footer: {:?}\nmaster last frame (tail):\n{}\n\
                 slave selected: {:?}\nslave footer: {:?}\nslave last frame (tail):\n{}",
                master_frame.0,
                master_frame.1,
                tail(&master_frame.2),
                slave_frame.0,
                slave_frame.1,
                tail(&slave_frame.2),
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn exercise(
    peer: Arc<dfdu_peer::Peer>,
    root: &std::path::Path,
    input: crossbeam_channel::Receiver<Event>,
    master: bool,
    frame: &Mutex<(String, String, String)>,
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
    let mut app = App::with_options(&ui, Instant::now());
    // Give the selected list row a stable observable style regardless of the user's theme.
    app.theme.cursor =
        ratatui::style::Style::new().add_modifier(ratatui::style::Modifier::REVERSED);
    let mut session = Session::over(&engine, app, SessionOptions::default());
    let _ = session.tick(Instant::now()).unwrap();
    if master {
        let path = root.join("photo.bin").display().to_string();
        crate::peer::nav::jump(&mut session, &path);
    }
    let mut panel = String::new();
    let mut terminal = Terminal::with_options(
        common::Recording::new(160, 30, |buffer| {
            let mut selected = String::new();
            let mut text = String::new();
            for row in buffer.content.chunks(160) {
                let line: String = row.iter().map(|cell| cell.symbol()).collect();
                let highlighted: String = row
                    .iter()
                    .filter(|cell| cell.modifier.contains(ratatui::style::Modifier::REVERSED))
                    .map(|cell| cell.symbol())
                    .collect();
                if highlighted.contains("z-photo.bin") {
                    selected = "z-photo.bin".into();
                } else if highlighted.contains("photo.bin") {
                    selected = "photo.bin".into();
                }
                text.push_str(&line);
                text.push('\n');
            }
            panel.push_str(&text);
            // The peer status is on the penultimate row, above the key hints.
            let footer = text.lines().nth(28).unwrap_or_default().to_owned();
            *frame.lock().unwrap() = (selected, footer, text);
        }),
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
    assert_eq!(
        session.front().session.current_view(),
        dfdu_session::view::View::Files,
    );
    let selected = dfdu_session::selected_path::selected(&session.front().session)
        .unwrap_or_else(|| {
            panic!(
                "no selection: view={:?}, index={:?}, last frame={}",
                session.front().session.current_view(),
                session.front().session.selected_index(),
                frame.lock().unwrap().2,
            )
        })
        .0
        .to_string();
    let footer = session.front().nav.footer.clone();
    drop(terminal);
    let seq = session.front().nav.seq;
    session.shutdown(Duration::ZERO);
    (footer, selected, panel, seq)
}
