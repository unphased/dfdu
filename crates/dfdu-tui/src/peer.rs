//! Peer pairing controls. Transport work belongs to dfdu-peer's workers.
pub(crate) mod nav;

use crate::app::App;
use crate::input::{
    line::{Edit, LineInput},
    line_keys,
};
use ratatui::crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Clear, Paragraph},
};
use std::sync::Arc;

/// Peer transport shared across browser reopen operations.
#[derive(Clone)]
pub struct PeerOptions {
    pub peer: Arc<dfdu_peer::Peer>,
    pub remote_bin: String,
}

impl std::fmt::Debug for PeerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerOptions")
            .field("id", &self.peer.id())
            .field("remote_bin", &self.remote_bin)
            .finish()
    }
}

pub(crate) fn status_text(id: &str, status: &dfdu_peer::Status) -> String {
    let role = match status.role {
        dfdu_peer::Role::Unbound => "unbound",
        dfdu_peer::Role::Master => "master",
        dfdu_peer::Role::Slave => "slave",
    };
    format!(
        "peer {id} {role}{}",
        status.peer.as_ref().map_or_else(String::new, |p| {
            let arrow = if status.role == dfdu_peer::Role::Slave {
                "←"
            } else {
                "→"
            };
            format!(" {arrow} {p}")
        })
    )
}

#[derive(Debug)]
pub(crate) struct Panel {
    rows: Vec<Row>,
    selected: usize,
    pending: crossbeam_channel::Receiver<Result<Vec<Row>, String>>,
    loading: bool,
    error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct Row {
    label: String,
    target: String,
    source_target: Option<String>,
}

fn instance_row(instance: dfdu_peer::Instance, host: Option<&str>) -> Row {
    Row {
        label: format!(
            "{} {:?} {}",
            instance.id,
            instance.role,
            instance
                .roots
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        target: host.map_or_else(|| instance.id.clone(), |h| format!("{h} {}", instance.id)),
        source_target: None,
    }
}

fn host_row(beacon: dfdu_peer::Beacon) -> Row {
    let target = |host: &str| {
        if beacon.user.is_empty() {
            host.to_owned()
        } else {
            format!("{}@{host}", beacon.user)
        }
    };
    Row {
        label: format!("{} ({})", target(&beacon.host), beacon.ids.join(", ")),
        target: target(&beacon.host),
        source_target: Some(target(&beacon.source.to_string())),
    }
}

fn resolve_target(target: String, fallback: String) -> String {
    resolve_target_with(target, fallback, |host| {
        use std::net::ToSocketAddrs;
        (host, 22)
            .to_socket_addrs()
            .is_ok_and(|mut addresses| addresses.next().is_some())
    })
}

fn resolve_target_with(
    target: String,
    fallback: String,
    resolve: impl FnOnce(&str) -> bool + Send + 'static,
) -> String {
    let host = target.rsplit('@').next().unwrap_or(&target).to_owned();
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send(resolve(&host));
    });
    if rx.recv_timeout(std::time::Duration::from_millis(500)) == Ok(true) {
        target
    } else {
        fallback
    }
}

impl Panel {
    fn load(&mut self, id: String, remote: Option<(String, String, String)>) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.pending = rx;
        self.loading = true;
        self.error = None;
        std::thread::spawn(move || {
            let result = (|| -> std::io::Result<Vec<Row>> {
                if let Some((host, bin, fallback)) = remote {
                    let host = resolve_target(host, fallback);
                    return Ok(dfdu_peer::remote_peers(&host, &bin)?
                        .into_iter()
                        .map(|p| instance_row(p, Some(&host)))
                        .collect());
                }
                let mut rows: Vec<_> = dfdu_peer::list()?
                    .into_iter()
                    .filter(|p| p.id != id)
                    .map(|p| instance_row(p, None))
                    .collect();
                rows.extend(dfdu_peer::discover_for(&id)?.into_iter().map(host_row));
                Ok(rows)
            })()
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }
}

pub(crate) fn results(app: &App) -> crossbeam_channel::Receiver<Result<Vec<Row>, String>> {
    app.peer_panel
        .as_ref()
        .filter(|p| p.loading)
        .map_or_else(crossbeam_channel::never, |p| p.pending.clone())
}

pub(crate) fn complete(app: &mut App, result: Result<Vec<Row>, String>) {
    if let Some(panel) = &mut app.peer_panel {
        panel.loading = false;
        match result {
            Ok(rows) => {
                panel.rows = rows;
                panel.selected = 0;
            }
            Err(error) => panel.error = Some(error),
        }
    }
}

pub(crate) fn open(app: &mut App) {
    let Some(options) = &app.peer else {
        app.session.status = Some("peer mode is not enabled".into());
        return;
    };
    if options.peer.status().role == dfdu_peer::Role::Unbound {
        let mut panel = Panel {
            rows: Vec::new(),
            selected: 0,
            pending: crossbeam_channel::never(),
            loading: false,
            error: None,
        };
        panel.load(options.peer.id().to_owned(), None);
        app.peer_panel = Some(panel);
    } else {
        options.peer.unbind();
    }
}

pub(crate) fn key(app: &mut App, k: KeyEvent) -> bool {
    if app.peer_prompt.is_none() {
        use ratatui::crossterm::event::KeyCode;
        let Some(panel) = &mut app.peer_panel else {
            return false;
        };
        match k.code {
            KeyCode::Esc => app.peer_panel = None,
            KeyCode::Char('m') => app.peer_prompt = Some(LineInput::default()),
            KeyCode::Char('r') => {
                if let Some(options) = &app.peer {
                    panel.load(options.peer.id().to_owned(), None);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                panel.selected = (panel.selected + 1).min(panel.rows.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => panel.selected = panel.selected.saturating_sub(1),
            KeyCode::Enter if !panel.loading => {
                if let Some(row) = panel.rows.get(panel.selected) {
                    let target = row.target.clone();
                    if let Some(options) = &app.peer {
                        if let Some(fallback) = row.source_target.clone() {
                            panel.load(
                                options.peer.id().to_owned(),
                                Some((target, options.remote_bin.clone(), fallback)),
                            );
                        } else {
                            if let Err(e) = options.peer.bind(&target, &options.remote_bin) {
                                app.session.status = Some(format!("peer: {e}"));
                            }
                            app.peer_panel = None;
                        }
                    }
                }
            }
            _ => {}
        }
        return true;
    }
    let Some(prompt) = &mut app.peer_prompt else {
        return false;
    };
    match line_keys::edit(prompt, &k) {
        Edit::Cancel => app.peer_prompt = None,
        Edit::Submit => {
            let target = prompt.text().trim().to_owned();
            if target.is_empty() {
                return true;
            }
            app.peer_prompt = None;
            app.peer_panel = None;
            if let Some(options) = &app.peer
                && let Err(e) = options.peer.bind(&target, &options.remote_bin)
            {
                app.session.status = Some(format!("peer: {e}"));
            }
        }
        _ => {}
    }
    true
}

pub(crate) fn draw(f: &mut Frame, app: &App) -> Option<Rect> {
    if app.peer_prompt.is_none() {
        let panel = app.peer_panel.as_ref()?;
        let area = f.area();
        let width = area.width.min(90);
        let height = area.height.min(18);
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        let available = usize::from(height.saturating_sub(5));
        let start = panel.selected.saturating_sub(available.saturating_sub(1));
        let mut lines = vec!["Enter select/bind · r refresh · m manual · Esc close".to_owned()];
        for (i, row) in panel.rows.iter().enumerate().skip(start).take(available) {
            lines.push(format!(
                "{} {}",
                if i == panel.selected { ">" } else { " " },
                row.label
            ));
        }
        if panel.loading {
            lines.push("Loading…".into());
        }
        if let Some(error) = &panel.error {
            lines.push(format!("peer: {error}"));
        }
        f.render_widget(Clear, rect);
        f.render_widget(
            Paragraph::new(lines.join("\n"))
                .block(Block::bordered().title(" PEERS "))
                .style(app.theme.fg_dim),
            rect,
        );
        return Some(rect);
    }
    let prompt = app.peer_prompt.as_ref()?;
    let area = f.area();
    let width = area.width.min(64);
    let height = area.height.min(5);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(format!(
            "{}\nID / HOST ID / HOST:ID\nEnter bind · Esc cancel",
            prompt.text()
        ))
        .block(Block::bordered().title(" TARGET "))
        .style(app.theme.fg_dim),
        rect,
    );
    if width > 2 && height > 2 {
        let cursor = unicode_width::UnicodeWidthStr::width(
            prompt
                .text()
                .chars()
                .take(prompt.cursor())
                .collect::<String>()
                .as_str(),
        );
        let cursor = u16::try_from(cursor).unwrap_or(u16::MAX).min(width - 3);
        f.set_cursor_position((rect.x + 1 + cursor, rect.y + 1));
    }
    Some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::theme::Theme;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::Instant;

    #[test]
    fn peer_key_is_registered_and_unused_elsewhere() {
        assert_eq!(
            crate::app::keys::first_key(crate::app::keys::Action::Peer),
            "B"
        );
        assert_eq!(
            crate::app::keys::action_for(&KeyEvent::new(KeyCode::Char('B'), KeyModifiers::NONE)),
            Some(crate::app::keys::Action::Peer)
        );
    }

    #[test]
    fn target_prompt_edits_cancels_and_keeps_spaces() {
        let mut app = App::new(
            Theme::builtin(
                crate::theme::BuiltinTheme::Dark,
                crate::theme::ColorLevel::TrueColor,
                None,
            ),
            Instant::now(),
        );
        app.peer_prompt = Some(crate::input::line::LineInput::default());
        for c in "host id".chars() {
            assert!(key(
                &mut app,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
            ));
        }
        assert_eq!(app.peer_prompt.as_ref().unwrap().text(), "host id");
        assert!(key(
            &mut app,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert!(app.peer_prompt.is_none());
    }

    fn app() -> App {
        App::new(
            Theme::builtin(
                crate::theme::BuiltinTheme::Dark,
                crate::theme::ColorLevel::TrueColor,
                None,
            ),
            Instant::now(),
        )
    }

    fn wait_role(peer: &dfdu_peer::Peer, role: dfdu_peer::Role) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if peer.status().role == role {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "peer did not become {role:?}");
            peer.events().recv_timeout(remaining).unwrap();
        }
    }

    #[test]
    fn keyboard_target_binds_two_real_peers_and_same_key_sends_bye() {
        let master = Arc::new(dfdu_peer::Peer::start(Vec::new()).unwrap());
        let slave = dfdu_peer::Peer::start(Vec::new()).unwrap();
        let mut app = app();
        app.peer = Some(PeerOptions {
            peer: master.clone(),
            remote_bin: dfdu_peer::DEFAULT_REMOTE_BIN.into(),
        });
        let b = KeyEvent::new(KeyCode::Char('B'), KeyModifiers::NONE);
        app.key(b, Instant::now());
        assert!(app.peer_panel.is_some());
        app.key(
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
            Instant::now(),
        );
        app.peer_prompt.as_mut().unwrap().set(slave.id());
        app.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            Instant::now(),
        );
        assert!(app.peer_prompt.is_none());
        wait_role(&master, dfdu_peer::Role::Master);
        wait_role(&slave, dfdu_peer::Role::Slave);
        app.key(b, Instant::now());
        wait_role(&master, dfdu_peer::Role::Unbound);
        wait_role(&slave, dfdu_peer::Role::Unbound);
        assert!(app.peer_prompt.is_none());
    }

    #[test]
    fn prompt_and_persistent_status_render_even_with_notice() {
        let peer = Arc::new(dfdu_peer::Peer::start(Vec::new()).unwrap());
        let mut app = app();
        app.peer = Some(PeerOptions {
            peer: peer.clone(),
            remote_bin: String::new(),
        });
        app.session.status = Some("other notice".into());
        open(&mut app);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|f| crate::view::draw(f, &mut app, Instant::now()))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("PEERS"));
        assert!(text.contains("manual"));
        assert!(text.contains(&format!("peer {} unbound", peer.id())));
        assert!(text.contains("other notice"));
        assert!(app.areas.overlay.is_some());
    }

    #[test]
    fn panel_selection_binds_local_instance_and_refresh_discards_old_work() {
        let master = Arc::new(dfdu_peer::Peer::start(Vec::new()).unwrap());
        let slave = dfdu_peer::Peer::start(Vec::new()).unwrap();
        let mut app = app();
        app.peer = Some(PeerOptions {
            peer: master.clone(),
            remote_bin: String::new(),
        });
        open(&mut app);
        let old = results(&app);
        key(
            &mut app,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
        );
        assert!(!old.same_channel(&results(&app)));
        complete(
            &mut app,
            Ok(vec![Row {
                label: "local roots unbound".into(),
                target: slave.id().into(),
                source_target: None,
            }]),
        );
        key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.peer_panel.is_none());
        wait_role(&master, dfdu_peer::Role::Master);
        wait_role(&slave, dfdu_peer::Role::Slave);
    }

    #[test]
    fn beacon_rows_use_hostname_and_keep_source_fallback() {
        for user in ["", "alice"] {
            let row = host_row(dfdu_peer::Beacon {
                host: "nas.invalid".into(),
                user: user.into(),
                ids: vec!["abcd1234".into()],
                source: "192.0.2.1".parse().unwrap(),
            });
            let prefix = if user.is_empty() { "" } else { "alice@" };
            assert_eq!(row.target, format!("{prefix}nas.invalid"));
            assert_eq!(row.label, format!("{prefix}nas.invalid (abcd1234)"));
            assert_eq!(row.source_target, Some(format!("{prefix}192.0.2.1")));
        }
    }

    #[test]
    fn selected_host_resolution_preserves_user_and_falls_back() {
        for success in [true, false] {
            let chosen =
                resolve_target_with("alice@nas".into(), "alice@192.0.2.1".into(), move |host| {
                    assert_eq!(host, "nas");
                    success
                });
            assert_eq!(
                chosen,
                if success {
                    "alice@nas"
                } else {
                    "alice@192.0.2.1"
                }
            );
            let row = instance_row(
                dfdu_peer::Instance {
                    id: "abcd1234".into(),
                    host: "nas".into(),
                    pid: 1,
                    roots: Vec::new(),
                    started: 0,
                    role: dfdu_peer::Role::Unbound,
                },
                Some(&chosen),
            );
            assert_eq!(row.target, format!("{chosen} abcd1234"));
        }
    }

    #[test]
    fn selected_host_resolution_has_bounded_wait() {
        let (release, blocked) = crossbeam_channel::bounded::<()>(1);
        let start = Instant::now();
        let chosen = resolve_target_with("nas".into(), "192.0.2.1".into(), move |_| {
            let _ = blocked.recv();
            true
        });
        let elapsed = start.elapsed();
        release.send(()).unwrap();
        assert_eq!(chosen, "192.0.2.1");
        assert!(elapsed >= std::time::Duration::from_millis(500));
        assert!(elapsed < std::time::Duration::from_secs(3));
    }

    #[test]
    fn remote_rows_keep_ssh_target_roots_and_role() {
        let row = instance_row(
            dfdu_peer::Instance {
                id: "abcd1234".into(),
                host: "nas".into(),
                pid: 1,
                roots: vec![std::path::PathBuf::from("backup")],
                started: 0,
                role: dfdu_peer::Role::Unbound,
            },
            Some("user@nas"),
        );
        assert_eq!(row.target, "user@nas abcd1234");
        assert!(row.label.contains("backup"));
        assert!(row.label.contains("Unbound"));
        assert!(row.source_target.is_none());
    }

    #[test]
    fn panel_navigation_errors_and_manual_cancel() {
        let mut app = app();
        app.peer_panel = Some(Panel {
            rows: vec![],
            selected: 0,
            pending: crossbeam_channel::never(),
            loading: true,
            error: None,
        });
        complete(&mut app, Err("ssh refused".into()));
        assert_eq!(
            app.peer_panel.as_ref().unwrap().error.as_deref(),
            Some("ssh refused")
        );
        key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.peer_panel.as_ref().unwrap().selected, 0);
        key(
            &mut app,
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
        );
        assert!(app.peer_prompt.is_some());
        key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.peer_panel.is_some());
        key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.peer_panel.is_none());
    }

    #[test]
    fn disabled_peer_key_reports_notice_without_prompt() {
        let mut app = app();
        open(&mut app);
        assert!(app.peer_prompt.is_none());
        assert_eq!(
            app.session.status.as_deref(),
            Some("peer mode is not enabled")
        );
    }

    #[test]
    fn discovered_hosts_footer_and_expiry() {
        let mut background = Background::default();
        let unbound = dfdu_peer::Status { role: dfdu_peer::Role::Unbound, peer: None };
        let beacon = |host: &str, ids: &[&str]| dfdu_peer::Beacon {
            host: host.into(), user: "slu".into(),
            ids: ids.iter().map(|s| (*s).into()).collect(),
            source: "127.0.0.1".parse().unwrap(),
        };
        assert_eq!(background.status_text("self", &unbound), "peer self unbound");
        background.complete("self", &unbound, Ok(vec![beacon("own", &["self"]), beacon("nas", &["remote"])]));
        assert_eq!(background.status_text("self", &unbound), "peer self unbound · ◆ nas found · B to connect");
        background.complete("self", &unbound, Ok(vec![beacon("local", &["self", "other"])]));
        assert_eq!(background.status_text("self", &unbound), "peer self unbound · ◆ 2 peers found · B to connect");
        for role in [dfdu_peer::Role::Master, dfdu_peer::Role::Slave] {
            let bound = dfdu_peer::Status { role, peer: Some("remote".into()) };
            assert_eq!(background.status_text("self", &bound), status_text("self", &bound));
        }
        background.complete("self", &unbound, Ok(vec![]));
        assert!(background.status_text("self", &unbound).contains("2 peers"));
        background.complete("self", &unbound, Ok(vec![]));
        assert!(background.status_text("self", &unbound).contains("local found"));
        background.complete("self", &unbound, Ok(vec![]));
        assert_eq!(background.status_text("self", &unbound), "peer self unbound");
    }

    #[test]
    fn roles_show_identity_and_partner() {
        for (role, name) in [
            (dfdu_peer::Role::Unbound, "unbound"),
            (dfdu_peer::Role::Master, "master"),
            (dfdu_peer::Role::Slave, "slave"),
        ] {
            let text = status_text(
                "abc",
                &dfdu_peer::Status {
                    role,
                    peer: Some("xyz".into()),
                },
            );
            assert!(text.contains(name));
            assert!(text.contains("abc"));
            assert!(text.contains("xyz"));
        }
    }
}
