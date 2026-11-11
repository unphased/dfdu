//! Untrusted UDP hints. Roots and connections always go through SSH.
use super::*;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

pub const DISCOVERY_PORT: u16 = 47839;
fn safe_hint(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn bounded_output(command: &mut Command) -> io::Result<std::process::Output> {
    let child = command
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(result) => result,
        Err(_) => {
            if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            }
            Err(error("discovery command timed out"))
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Beacon {
    pub host: String,
    pub user: String,
    pub ids: Vec<String>,
    #[serde(skip, default = "unknown_ip")]
    pub source: IpAddr,
}
fn unknown_ip() -> IpAddr {
    Ipv4Addr::UNSPECIFIED.into()
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    magic: String,
    version: u8,
    instance: String,
}
fn port() -> u16 {
    std::env::var("DFDU_DISCOVERY_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DISCOVERY_PORT)
}
fn probe(id: &str) -> Vec<u8> {
    serde_json::to_vec(&Probe {
        magic: "dfdu-discovery".into(),
        version: 1,
        instance: id.into(),
    })
    .unwrap()
}
fn answer(bytes: &[u8], own: &str, dir: &Path) -> Option<Vec<u8>> {
    if bytes.len() > 256 {
        return None;
    }
    let p: Probe = serde_json::from_slice(bytes).ok()?;
    if p.magic != "dfdu-discovery" || p.version != 1 || p.instance == own || p.instance.len() > 64 {
        return None;
    }
    let instances = list_in(dir).ok()?;
    let host = instances.first()?.host.clone();
    let beacon = Beacon {
        host,
        user: std::env::var("USER").unwrap_or_default(),
        ids: instances.into_iter().map(|i| i.id).collect(),
        source: unknown_ip(),
    };
    let bytes = serde_json::to_vec(&beacon).ok()?;
    (bytes.len() <= 8192).then_some(bytes)
}
pub(super) struct Responder(Arc<AtomicBool>);
impl Drop for Responder {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
pub(super) fn responder(dir: PathBuf, id: String) -> Responder {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            // One owner answers for all instances. Others retry when it exits.
            let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port())) else {
                thread::sleep(Duration::from_millis(250));
                continue;
            };
            let _ = socket.set_read_timeout(Some(Duration::from_millis(200)));
            let mut bytes = [0; 8193];
            while !flag.load(Ordering::Relaxed) {
                if let Ok((n, source)) = socket.recv_from(&mut bytes)
                    && let Some(reply) = answer(&bytes[..n], &id, &dir)
                {
                    let _ = socket.send_to(&reply, source);
                }
            }
        }
    });
    Responder(stop)
}
fn tailnet_targets(bytes: &[u8], port: u16) -> Vec<SocketAddr> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    value
        .get("Peer")
        .and_then(|p| p.as_object())
        .into_iter()
        .flat_map(|p| p.values())
        .filter(|p| p["Online"] == true)
        .flat_map(|p| p["TailscaleIPs"].as_array().into_iter().flatten())
        .filter_map(|ip| ip.as_str()?.parse::<IpAddr>().ok())
        .filter(|ip| ip.is_ipv4())
        .map(|ip| SocketAddr::new(ip, port))
        .collect()
}
/// Probe LAN and online tailnet peers, collecting untrusted hints for one second.
pub fn discover() -> io::Result<Vec<Beacon>> {
    discover_for(&random_id()?)
}
/// Probe on behalf of an instance, so its own responder ignores the probe.
pub fn discover_for(id: &str) -> io::Result<Vec<Beacon>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_broadcast(true)?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    let mut targets = vec![SocketAddr::new(Ipv4Addr::BROADCAST.into(), port())];
    let command = std::env::var_os("DFDU_TAILSCALE").unwrap_or_else(|| "tailscale".into());
    let output = bounded_output(Command::new(command).args(["status", "--json"])).or_else(|_| {
        bounded_output(
            Command::new("/Applications/Tailscale.app/Contents/MacOS/Tailscale")
                .args(["status", "--json"]),
        )
    });
    if let Ok(output) = output
        && output.status.success()
    {
        targets.extend(tailnet_targets(&output.stdout, port()));
    }
    let bytes = probe(id);
    for target in targets {
        let _ = socket.send_to(&bytes, target);
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut hosts = std::collections::BTreeMap::new();
    let mut bytes = [0; 8193];
    while Instant::now() < deadline {
        if let Ok((n, source)) = socket.recv_from(&mut bytes) {
            if n > 8192 {
                continue;
            }
            if let Ok(mut beacon) = serde_json::from_slice::<Beacon>(&bytes[..n]) {
                if !safe_hint(&beacon.host)
                    || (!beacon.user.is_empty() && !safe_hint(&beacon.user))
                    || beacon.ids.iter().any(|id| !safe_hint(id))
                    || beacon.host.is_empty()
                    || beacon.host.len() > 253
                    || beacon.user.len() > 256
                    || beacon.ids.len() > 256
                {
                    continue;
                }
                beacon.source = source.ip();
                hosts.insert(beacon.host.clone(), beacon);
            }
        }
    }
    Ok(hosts.into_values().collect())
}
/// Fetch trusted instance details through the same SSH executable as binding.
pub fn remote_peers(host: &str, remote_bin: &str) -> io::Result<Vec<Instance>> {
    if host.is_empty() || host.starts_with('-') || host.chars().any(char::is_whitespace) {
        return Err(error("invalid peer host"));
    }
    let output = bounded_output(
        Command::new(std::env::var_os("DFDU_SSH").unwrap_or_else(|| "ssh".into())).args([
            "-T",
            "-o",
            "BatchMode=yes",
            host,
            &format!(
                "{} peers --json",
                if let Some(rest) = remote_bin.strip_prefix("~/") {
                    format!("\"$HOME\"/{}", quote(rest))
                } else {
                    quote(remote_bin)
                }
            ),
        ]),
    )?;
    if !output.status.success() {
        return Err(error(String::from_utf8_lossy(&output.stderr)));
    }
    serde_json::from_slice(&output.stdout).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tailnet_online_ipv4_only() {
        let targets = tailnet_targets(br#"{"Peer":{"a":{"Online":true,"TailscaleIPs":["100.64.0.1","fd00::1"]},"b":{"Online":false,"TailscaleIPs":["100.64.0.2"]}}}"#, 1234);
        assert_eq!(targets, vec!["100.64.0.1:1234".parse().unwrap()]);
    }
    #[test]
    fn reject_invalid_and_own_probe() {
        assert!(answer(b"{}", "self", Path::new("/missing")).is_none());
        assert!(answer(&probe("self"), "self", Path::new("/missing")).is_none());
    }
}
