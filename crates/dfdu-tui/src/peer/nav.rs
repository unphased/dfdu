//! Navigation matching and bounded sampled-hash workers.
use dfdu_peer::{Nav, Verdict};

pub(crate) fn mark(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::HashMatch => "✓",
        Verdict::HashMismatch => "✗",
        Verdict::Pending => "…",
        Verdict::NameOnly => "name",
        Verdict::None => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_marks_are_distinct() {
        assert_eq!(mark(Verdict::HashMatch), "✓");
        assert_eq!(mark(Verdict::HashMismatch), "✗");
        assert_eq!(mark(Verdict::Pending), "…");
    }
    #[test]
    fn candidate_panel_is_docked_and_hidden_when_unbound() {
        let master = dfdu_peer::Peer::start(Vec::new()).unwrap();
        let slave = std::sync::Arc::new(dfdu_peer::Peer::start(Vec::new()).unwrap());
        let mut app = crate::app::App::new(
            crate::theme::Theme::builtin(
                crate::theme::BuiltinTheme::Dark,
                crate::theme::ColorLevel::TrueColor,
                None,
            ),
            std::time::Instant::now(),
        );
        app.peer = Some(crate::PeerOptions {
            peer: slave.clone(),
            remote_bin: String::new(),
        });
        app.nav.size = Some(21);
        app.nav.candidates.push(Candidate {
            path: "/backup/photo.bin".into(),
            size: 21,
            mtime: 1234,
        });
        assert_eq!(panel_height(&app, 24), 0);
        master
            .bind(slave.id(), dfdu_peer::DEFAULT_REMOTE_BIN)
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while slave.status().role != dfdu_peer::Role::Slave {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|f| crate::view::draw(f, &mut app, std::time::Instant::now()))
            .unwrap();
        let dock = app.areas.candidates.unwrap();
        assert!(app.areas.list.bottom() <= dock.y);
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        for word in ["Candidates", "/backup/photo.bin", "21", "1234", "…"] {
            assert!(text.contains(word), "missing {word}");
        }
        app.nav.seq = 99;
        app.nav.expected = Some("same".into());
        let mut runtime = Runtime::new();
        runtime.generation = 2;
        runtime.complete(
            &mut app,
            (1, "/backup/photo.bin".into(), Some("old".into())),
        );
        assert!(
            app.nav.hashes.is_empty(),
            "obsolete worker generation must be ignored"
        );
        runtime.complete(
            &mut app,
            (2, "/backup/photo.bin".into(), Some("same".into())),
        );
        assert_eq!(app.nav.verdict(&app.nav.candidates[0]), Verdict::HashMatch);
        app.nav.candidates.clear();
        assert_eq!(panel_height(&app, 24), 0);
    }
    #[test]
    fn stale_hash_cannot_change_candidates() {
        let mut state = State {
            seq: 2,
            ..State::default()
        };
        state.complete((1, "old".into(), Some("hash".into())));
        assert!(state.hashes.is_empty());
    }
}

#[derive(Default, Debug)]
pub(crate) struct State {
    pub seq: u64,
    pub candidates: Vec<Candidate>,
    pub hashes: std::collections::HashMap<String, Option<String>>,
    pub expected: Option<String>,
    pub size: Option<u64>,
    pub focus: bool,
    pub cursor: usize,
    pub footer: String,
}
#[derive(Debug)]
pub(crate) struct Candidate {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
}
type Result = (u64, String, Option<String>);
impl State {
    pub fn complete(&mut self, (seq, path, hash): Result) {
        if seq == self.seq {
            self.hashes.insert(path, hash);
        }
    }
    pub fn verdict(&self, c: &Candidate) -> Verdict {
        if self.size != Some(c.size) {
            return Verdict::NameOnly;
        }
        match (self.expected.as_ref(), self.hashes.get(&c.path)) {
            (Some(a), Some(Some(b))) if a == b => Verdict::HashMatch,
            (Some(_), Some(Some(_))) => Verdict::HashMismatch,
            (_, Some(None)) => Verdict::NameOnly,
            _ => Verdict::Pending,
        }
    }
}
/// Two fixed workers share a replaceable queue. Old in-flight reads cannot publish
/// into a newer generation; moving never creates another thread.
type Queue = std::sync::Arc<(
    std::sync::Mutex<Option<Vec<(u64, String)>>>,
    std::sync::Condvar,
)>;
pub(crate) struct Workers {
    queue: Queue,
    pub results: crossbeam_channel::Receiver<Result>,
}
impl Workers {
    pub fn new() -> Self {
        let queue = std::sync::Arc::new((
            std::sync::Mutex::new(Some(Vec::<(u64, String)>::new())),
            std::sync::Condvar::new(),
        ));
        let (tx, results) = crossbeam_channel::bounded(64);
        for _ in 0..2 {
            let q = queue.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                loop {
                    let job = {
                        let (lock, wake) = &*q;
                        let mut jobs = lock.lock().unwrap();
                        loop {
                            let Some(list) = jobs.as_mut() else {
                                return;
                            };
                            if let Some(job) = list.pop() {
                                break job;
                            }
                            jobs = wake.wait(jobs).unwrap();
                        }
                    };
                    let hash = dfdu_peer::sample_hash(std::path::Path::new(&job.1)).ok();
                    if tx.send((job.0, job.1, hash)).is_err() {
                        return;
                    }
                }
            });
        }
        Self { queue, results }
    }
    pub fn replace(&self, jobs: Vec<(u64, String)>) {
        *self.queue.0.lock().unwrap() = Some(jobs);
        self.queue.1.notify_all();
    }
}
impl Drop for Workers {
    fn drop(&mut self) {
        *self.queue.0.lock().unwrap() = None;
        self.queue.1.notify_all();
    }
}

pub(crate) fn panel_height(app: &crate::app::App, height: u16) -> u16 {
    let s = &app.nav;
    if s.candidates.is_empty()
        || app
            .peer
            .as_ref()
            .is_none_or(|p| p.peer.status().role != dfdu_peer::Role::Slave)
    {
        return 0;
    }
    u16::try_from(s.candidates.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .min(8)
        .min(height / 2)
}

pub(crate) fn draw(f: &mut ratatui::Frame, app: &crate::app::App, rect: ratatui::layout::Rect) {
    if rect.height == 0 {
        return;
    }
    let s = &app.nav;
    let lines: Vec<_> = s
        .candidates
        .iter()
        .enumerate()
        .skip(s.cursor.saturating_sub(4))
        .take(rect.height.saturating_sub(2) as usize)
        .map(|(i, c)| {
            format!(
                "{} {} {} {} {}",
                if i == s.cursor { ">" } else { " " },
                mark(s.verdict(c)),
                c.path,
                c.size,
                c.mtime
            )
        })
        .collect();
    f.render_widget(ratatui::widgets::Clear, rect);
    f.render_widget(
        ratatui::widgets::Paragraph::new(lines.join("\n")).block(
            ratatui::widgets::Block::bordered().title(if s.focus {
                "Candidates: j/k Enter Esc"
            } else {
                "Candidates: C focus"
            }),
        ),
        rect,
    );
}

pub(crate) struct Runtime {
    pub workers: Workers,
    index: dfdu_session::match_index::MatchIndex,
    last_path: Option<String>,
    last_nav: Option<Nav>,
    version: u64,
    followed: Option<String>,
    local_moved: bool,
    generation: u64,
}
impl Runtime {
    pub fn new() -> Self {
        Self {
            workers: Workers::new(),
            index: Default::default(),
            last_path: None,
            last_nav: None,
            version: 0,
            followed: None,
            local_moved: false,
            generation: 0,
        }
    }
    pub fn tick<B: dfdu_scan::Backend>(
        &mut self,
        session: &mut dfdu_session::Session<'_, B, crate::app::App>,
    ) {
        let Some(peer) = session.front().peer.as_ref().map(|p| p.peer.clone()) else {
            return;
        };
        let selected = dfdu_session::selected_path::selected(&session.front().session)
            .map(|(p, d)| (p.to_string(), d));
        match peer.status().role {
            dfdu_peer::Role::Master => {
                if crate::app::views::current(session.front()) != crate::app::views::View::Files {
                    return;
                }
                let Some((path, dir)) = selected else {
                    return;
                };
                if self.last_path.as_ref() == Some(&path) {
                    return;
                }
                self.last_path = Some(path.clone());
                session.front_mut().nav.seq += 1;
                let seq = session.front().nav.seq;
                let nav = session.engine().read(|m| {
                    let ix =
                        dfdu_tree::find_path(&m.tree, &dfdu_tree::HostPath::from(path.as_str()))?;
                    let mut parents = Vec::new();
                    let mut p = m.tree.parent(ix);
                    while let Some(i) = p {
                        parents.push(node_name(&m.tree, i));
                        p = m.tree.parent(i);
                    }
                    Some(Nav {
                        seq,
                        path: path.clone(),
                        name: node_name(&m.tree, ix),
                        kind: if dir {
                            dfdu_peer::NavKind::Dir
                        } else {
                            dfdu_peer::NavKind::File
                        },
                        size: m.tree.raw_size(ix).0,
                        mtime: m.tree.mtime(ix),
                        parents,
                    })
                });
                if let Some(nav) = nav {
                    let _ = peer.send_nav(nav);
                    self.generation += 1;
                    self.workers
                        .replace(if dir || !local_files(session.front()) {
                            vec![]
                        } else {
                            vec![(self.generation, path)]
                        });
                }
            }
            dfdu_peer::Role::Slave => {
                let (version, busy) = session.engine().read(|m| {
                    (
                        m.tree.version(),
                        m.status.jobs.iter().any(|j| {
                            matches!(
                                j.phase,
                                dfdu_engine::JobPhase::Running | dfdu_engine::JobPhase::Finishing
                            )
                        }),
                    )
                });
                if self.followed != selected.map(|s| s.0) {
                    self.local_moved = true;
                }
                if !busy
                    && !self.local_moved
                    && version != self.version
                    && let Some(nav) = self.last_nav.clone()
                {
                    self.receive(session, nav);
                }
                if !busy {
                    self.version = version;
                }
            }
            dfdu_peer::Role::Unbound => {
                self.last_path = None;
                self.last_nav = None;
                self.followed = None;
                let seq = session.front().nav.seq;
                session.front_mut().nav = State {
                    seq,
                    ..State::default()
                };
                self.generation += 1;
                self.workers.replace(vec![]);
            }
        }
    }
    pub fn receive<B: dfdu_scan::Backend>(
        &mut self,
        session: &mut dfdu_session::Session<'_, B, crate::app::App>,
        nav: Nav,
    ) {
        if self.last_nav.as_ref().is_some_and(|old| nav.seq < old.seq) {
            return;
        }
        let candidates = session.engine().read(|m| {
            let parents: Vec<_> = nav.parents.iter().map(|s| s.as_bytes()).collect();
            self.index
                .ranked_candidates(
                    &m.tree,
                    nav.size,
                    nav.name.as_bytes(),
                    &parents,
                    nav.mtime,
                    nav.kind == dfdu_peer::NavKind::Dir,
                )
                .into_iter()
                .map(|ix| Candidate {
                    path: m.tree.host_path(ix).to_string(),
                    size: m.tree.raw_size(ix).0,
                    mtime: m.tree.mtime(ix),
                })
                .collect::<Vec<_>>()
        });
        let expected = if session.front().nav.seq == nav.seq {
            session.front().nav.expected.clone()
        } else {
            None
        };
        self.generation += 1;
        let local = local_files(session.front());
        let jobs = candidates
            .iter()
            .filter(|c| local && c.size == nav.size && nav.kind == dfdu_peer::NavKind::File)
            .rev()
            .map(|c| (self.generation, c.path.clone()))
            .collect();
        let best = candidates.first().map(|c| c.path.clone());
        session.front_mut().nav = State {
            seq: nav.seq,
            candidates,
            expected,
            size: (local && nav.kind == dfdu_peer::NavKind::File).then_some(nav.size),
            ..State::default()
        };
        self.last_nav = Some(nav);
        self.local_moved = false;
        self.workers.replace(jobs);
        if let Some(path) = best {
            jump(session, &path);
        }
        self.followed = dfdu_session::selected_path::selected(&session.front().session)
            .map(|(p, _)| p.to_string());
        publish(session.front());
    }
    pub fn complete(&self, app: &mut crate::app::App, result: Result) {
        let Some(peer) = app.peer.as_ref().map(|p| p.peer.clone()) else {
            return;
        };
        if result.0 != self.generation {
            return;
        }
        if peer.status().role == dfdu_peer::Role::Master {
            if let Some(hash) = result.2 {
                let _ = peer.send_nav_hash(dfdu_peer::NavHash {
                    seq: app.nav.seq,
                    hash,
                });
            }
        } else {
            app.nav.complete((app.nav.seq, result.1, result.2));
            publish(app);
        }
    }
}
pub(crate) fn publish(app: &crate::app::App) {
    let Some(p) = &app.peer else {
        return;
    };
    let s = &app.nav;
    let best = s
        .candidates
        .iter()
        .find(|c| s.verdict(c) == Verdict::HashMatch)
        .or_else(|| s.candidates.first());
    let _ = p.peer.send_nav_status(dfdu_peer::NavStatus {
        seq: s.seq,
        best: best.map(|c| c.path.clone()),
        candidates: s.candidates.len(),
        verdict: best.map_or(Verdict::None, |c| s.verdict(c)),
    });
}
pub(crate) fn jump<B: dfdu_scan::Backend>(
    session: &mut dfdu_session::Session<'_, B, crate::app::App>,
    path: &str,
) {
    let gone = session.go_to_path(path);
    let _ = crate::overlay::goto::gone_to(session.front_mut(), gone);
}

fn local_files(app: &crate::app::App) -> bool {
    app.session.outside.local && app.session.from_file.is_none()
}
fn node_name(tree: &dfdu_tree::Tree, ix: dfdu_tree::Ix) -> String {
    if tree.parent(ix).is_none() {
        let path = tree.host_path(ix);
        String::from_utf8_lossy(path.file_name().unwrap_or(tree.name(ix))).into_owned()
    } else {
        String::from_utf8_lossy(tree.name(ix)).into_owned()
    }
}
