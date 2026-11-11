//! Local peer commands and browser registration.

use std::process::ExitCode;

fn failure(error: impl std::fmt::Display) -> ExitCode {
    eprintln!("dfdu: peer: {error}");
    ExitCode::from(1)
}

#[cfg(unix)]
pub fn list(json: bool) -> Result<ExitCode, ExitCode> {
    let peers = dfdu_peer::list().map_err(failure)?;
    if json {
        println!("{}", serde_json::to_string(&peers).map_err(failure)?);
    } else {
        for peer in peers {
            println!(
                "{}\t{}\t{:?}\tpid {}\t{}",
                peer.id,
                peer.host,
                peer.role,
                peer.pid,
                peer.roots
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(unix)]
pub fn attach(id: &str) -> Result<ExitCode, ExitCode> {
    dfdu_peer::attach(id).map_err(failure)?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(unix))]
pub fn list(_: bool) -> Result<ExitCode, ExitCode> {
    Err(failure("peer mode requires Unix"))
}

#[cfg(not(unix))]
pub fn attach(_: &str) -> Result<ExitCode, ExitCode> {
    Err(failure("peer mode requires Unix"))
}

#[cfg(unix)]
pub struct SignalGuard {
    handle: signal_hook::iterator::Handle,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl Drop for SignalGuard {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Clean registrations before normal termination signals or an uncaught panic.
#[cfg(unix)]
pub fn signals(peer: &dfdu_peer::Peer) -> Result<SignalGuard, ExitCode> {
    use signal_hook::{
        consts::signal::{SIGHUP, SIGINT, SIGTERM},
        iterator::Signals,
    };
    let mut signals = Signals::new([SIGHUP, SIGINT, SIGTERM]).map_err(failure)?;
    let handle = signals.handle();
    let cleanup = peer.cleanup_hook();
    let worker = std::thread::spawn(move || {
        if let Some(signal) = signals.forever().next() {
            cleanup();
            let _ = signal_hook::low_level::emulate_default_handler(signal);
        }
    });
    let cleanup = peer.cleanup_hook();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !dfdu_engine::panic_is_caught_by_engine(std::thread::current().name()) {
            cleanup();
        }
        previous(info);
    }));
    Ok(SignalGuard {
        handle,
        worker: Some(worker),
    })
}

#[cfg(unix)]
pub fn start(
    paths: &[std::path::PathBuf],
    target: Option<&str>,
    remote_bin: String,
) -> Result<dfdu_tui::PeerOptions, ExitCode> {
    let roots = paths
        .iter()
        .map(std::fs::canonicalize)
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(failure)?;
    let peer = std::sync::Arc::new(dfdu_peer::Peer::start(roots).map_err(failure)?);
    if let Some(target) = target {
        peer.bind(target, &remote_bin).map_err(failure)?;
    }
    Ok(dfdu_tui::PeerOptions { peer, remote_bin })
}
