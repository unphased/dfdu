//! Commands received from a local peer. Tree indices never leave the model read lock.
use dfdu_session::{
    Session,
    match_index::MatchIndex,
    view::{self, Show},
};
use dfdu_tree::{HostPath, Tree};
use serde_json::{Value, json};

use crate::app::App;

#[derive(Clone)]
struct Query {
    size: u64,
    name: String,
    parents: Vec<String>,
}

#[derive(Default)]
pub(crate) struct PeerState {
    index: MatchIndex,
    query: Option<Query>,
    current: Option<HostPath>,
    pending: Option<isize>,
}

pub(crate) fn cycle_key(app: &mut App, direction: isize) {
    app.peer.pending = Some(direction);
}

pub(crate) fn pending<B: dfdu_scan::Backend>(session: &mut Session<'_, B, App>) {
    if let Some(direction) = session.front_mut().peer.pending.take() {
        let _ = cycle(session, direction);
    }
}

fn candidates(tree: &Tree, index: &mut MatchIndex, q: &Query) -> Vec<(HostPath, HostPath)> {
    let parents: Vec<_> = q.parents.iter().map(|p| p.as_bytes()).collect();
    index
        .candidates(tree, q.size, q.name.as_bytes(), &parents)
        .into_iter()
        .map(|ix| {
            let mut root = ix;
            while let Some(parent) = tree.parent(root) {
                root = parent;
            }
            (tree.host_path(ix), tree.host_path(root))
        })
        .collect()
}

fn show(app: &mut App, path: HostPath, root: HostPath) {
    dfdu_session::filter::clear(&mut app.session);
    dfdu_session::type_filter::clear(&mut app.session);
    if app.session.hide_hidden {
        app.session.hide_hidden = false;
        app.session.request_refilter();
    }
    if let Some(moved) = view::show(&mut app.session, &Show { path, root }) {
        app.went(moved);
    }
}

fn cycle<B: dfdu_scan::Backend>(session: &mut Session<'_, B, App>, direction: isize) -> Value {
    let mut peer = std::mem::take(&mut session.front_mut().peer);
    let Some(query) = &peer.query else {
        session.front_mut().session.status = Some("no peer matches".into());
        session.front_mut().peer = peer;
        return json!({"error":"no matches"});
    };
    let matches = session
        .engine()
        .read(|m| candidates(&m.tree, &mut peer.index, query));
    if matches.is_empty() {
        peer.current = None;
        session.front_mut().session.status = Some("0 matches".into());
        session.front_mut().peer = peer;
        return json!({"count":0});
    }
    let at = peer
        .current
        .as_ref()
        .and_then(|p| matches.iter().position(|(path, _)| path == p));
    let at = match at {
        Some(i) => (i as isize + direction).rem_euclid(matches.len() as isize) as usize,
        None => 0,
    };
    let (path, root) = matches[at].clone();
    peer.current = Some(path.clone());
    let app = session.front_mut();
    show(app, path.clone(), root);
    app.session.status = Some(format!("{}/{} matches", at + 1, matches.len()));
    app.peer = peer;
    json!({"path":path.display().to_string(),"index":at + 1,"count":matches.len()})
}

pub(crate) fn command<B: dfdu_scan::Backend>(
    session: &mut Session<'_, B, App>,
    command: Value,
) -> Value {
    let Some(object) = command.as_object() else {
        return json!({"error":"invalid command"});
    };
    if object.len() != 1 {
        return json!({"error":"invalid command"});
    }
    let Some((name, value)) = object.iter().next() else {
        return json!({"error":"invalid command"});
    };
    match name.as_str() {
        "match" => {
            let Some(size) = value.get("size").and_then(Value::as_u64) else {
                return json!({"error":"invalid match size"});
            };
            let Some(name) = value.get("name").and_then(Value::as_str) else {
                return json!({"error":"invalid match name"});
            };
            let Some(parents) = value.get("parents").and_then(Value::as_array) else {
                return json!({"error":"invalid match parents"});
            };
            let Some(parents) = parents
                .iter()
                .map(|p| p.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
            else {
                return json!({"error":"invalid match parents"});
            };
            let peer = &mut session.front_mut().peer;
            peer.query = Some(Query {
                size,
                name: name.to_owned(),
                parents,
            });
            peer.current = None;
            cycle(session, 0)
        }
        "next" => cycle(session, 1),
        "prev" => cycle(session, -1),
        "goto" => {
            let Some(text) = value
                .as_str()
                .or_else(|| value.get("path").and_then(Value::as_str))
            else {
                return json!({"error":"invalid path"});
            };
            let found = session.engine().read(|m| {
                let path = HostPath::new(m.tree.host(), text.as_bytes().to_vec());
                let ix = dfdu_tree::find_path(&m.tree, &path)?;
                let mut root = ix;
                while let Some(parent) = m.tree.parent(root) {
                    root = parent;
                }
                Some((path, m.tree.host_path(root)))
            });
            let Some((path, root)) = found else {
                return json!({"error":"path not found"});
            };
            if path == root {
                return json!({"error":"cannot select root"});
            }
            let app = session.front_mut();
            app.peer.query = None;
            app.peer.current = None;
            show(app, path.clone(), root);
            json!({"path":path.display().to_string()})
        }
        "state" => {
            let state = &session.front().session;
            let path =
                dfdu_session::selected_path::selected(state).map(|(p, _)| p.display().to_string());
            json!({"path":path})
        }
        _ => json!({"error":"unsupported command"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use dfdu_engine::{Command, Engine, EngineOptions};
    use dfdu_scan::{JobOptions, PortableBackend};
    use dfdu_session::SessionOptions;
    use std::time::{Duration, Instant};

    #[test]
    fn goto_selects_in_current_directory_through_filters() {
        use dfdu_session::type_filter::{self, Row};
        use dfdu_tree::filetype::FileType;

        let dir = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        std::fs::write(dir.path().join("other"), b"1234").unwrap();
        std::fs::write(dir.path().join(".target"), b"123").unwrap();
        let engine = Engine::start(PortableBackend::default(), EngineOptions::default());
        engine.send(Command::Scan {
            path: dir.path().to_path_buf().into(),
            opts: JobOptions::default(),
        });
        assert!(engine.wait_idle(Some(Duration::from_secs(20))).is_idle());
        let now = Instant::now();
        let mut session = Session::over(
            &engine,
            App::new(Theme::no_color(), now),
            SessionOptions::default(),
        );
        let _ = session.tick(now).unwrap();
        let target = HostPath::from(dir.path())
            .join(b".target")
            .display()
            .to_string();
        command(&mut session, json!({"goto":target}));
        let _ = session.tick(now).unwrap();
        assert_eq!(command(&mut session, json!({"state":null}))["path"], target);

        dfdu_session::filter::open(&mut session.front_mut().session);
        for c in "other".chars() {
            dfdu_session::filter::edit(
                &mut session.front_mut().session,
                dfdu_session::line::TextOp::Insert(c),
            );
        }
        session.front_mut().session.hide_hidden = true;
        type_filter::toggle(&mut session.front_mut().session, Row::Type(FileType::Video));
        session.front_mut().session.request_refilter();
        let _ = session.tick(now).unwrap();
        command(&mut session, json!({"goto":target}));
        let _ = session.tick(now).unwrap();
        assert_eq!(command(&mut session, json!({"state":null}))["path"], target);
        drop(session);
        engine.shutdown(Duration::from_millis(200));
    }

    #[test]
    fn matches_cycle_and_goto_a_scanned_tree() {
        let dir = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        std::fs::create_dir(dir.path().join("a")).unwrap();
        std::fs::create_dir(dir.path().join("b")).unwrap();
        std::fs::write(dir.path().join("a/file"), b"123").unwrap();
        std::fs::write(dir.path().join("b/file"), b"123").unwrap();
        let engine = Engine::start(PortableBackend::default(), EngineOptions::default());
        engine.send(Command::Scan {
            path: dir.path().to_path_buf().into(),
            opts: JobOptions::default(),
        });
        assert!(engine.wait_idle(Some(Duration::from_secs(20))).is_idle());
        let now = Instant::now();
        let mut session = Session::over(
            &engine,
            App::new(Theme::no_color(), now),
            SessionOptions::default(),
        );
        let _ = session.tick(now).unwrap();
        let reply = command(
            &mut session,
            json!({"match":{"size":3,"name":"file","parents":["a"]}}),
        );
        assert_eq!(reply["count"], 2);
        assert_eq!(reply["index"], 1);
        let _ = session.tick(now).unwrap();
        assert_eq!(
            command(&mut session, json!({"state":null}))["path"],
            reply["path"]
        );
        assert_eq!(
            reply["path"],
            HostPath::from(dir.path())
                .join(b"a")
                .join(b"file")
                .display()
                .to_string()
        );
        let next = command(&mut session, json!({"next":null}));
        let _ = session.tick(now).unwrap();
        assert_eq!(
            command(&mut session, json!({"state":null}))["path"],
            next["path"]
        );
        assert_eq!(
            next["path"],
            HostPath::from(dir.path())
                .join(b"b")
                .join(b"file")
                .display()
                .to_string()
        );
        assert_eq!(
            command(&mut session, json!({"prev":null}))["path"],
            reply["path"]
        );
        assert_eq!(
            command(&mut session, json!({"goto":"missing"}))["error"],
            "path not found"
        );
        assert_eq!(
            command(&mut session, json!({"match":{"size":0}}))["error"],
            "invalid match name"
        );
        drop(session);
        engine.shutdown(Duration::from_millis(200));
    }
}
