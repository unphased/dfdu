//! Local instance registration and authenticated SSH/stdin peer transport.
#![cfg(unix)]

use crossbeam_channel::{Receiver, Sender, unbounded};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_REMOTE_BIN: &str = "~/.local/bin/dfdu";
const LIMIT: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Unbound,
    Master,
    Slave,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub role: Role,
    pub peer: Option<String>,
}
#[derive(Clone, Debug)]
pub enum Event {
    Status(Status),
    Error(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub host: String,
    pub pid: u32,
    pub roots: Vec<PathBuf>,
    /// Unix seconds at registration.
    pub started: u64,
    pub role: Role,
}
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Frame {
    Hello { proto: u32, id: String, role: Role },
    Ping,
    Bye,
}
enum Control {
    Bind(String, String),
    Unbind,
    Stop,
}

pub struct Peer {
    instance: Instance,
    status: Arc<Mutex<Status>>,
    events: Receiver<Event>,
    control: Sender<Control>,
    worker: Option<thread::JoinHandle<()>>,
    dir: PathBuf,
}
fn error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
fn directory() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| error("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".local/state/dfdu/peers"))
}
fn socket(dir: &Path, id: &str) -> io::Result<PathBuf> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(error("invalid peer ID"));
    }
    let path = dir.join(format!("{id}.sock"));
    if path.as_os_str().as_bytes().len() >= 104 {
        return Err(error("peer socket path exceeds 103 bytes"));
    }
    Ok(path)
}
fn metadata(dir: &Path, instance: &Instance) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(dir.join(format!("{}.json", instance.id)))?;
    serde_json::to_writer(&mut file, instance).map_err(io::Error::other)
}
impl Peer {
    pub fn start(roots: Vec<PathBuf>) -> io::Result<Self> {
        Self::start_in(directory()?, roots)
    }
    fn start_in(dir: PathBuf, roots: Vec<PathBuf>) -> io::Result<Self> {
        DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
        let info = fs::symlink_metadata(&dir)?;
        if !info.is_dir() || info.uid() != rustix::process::geteuid().as_raw() {
            return Err(error("peer directory must be owned by the current user"));
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        let instance = Instance {
            id: format!(
                "{:x}-{:x}-{:x}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ),
            host: std::env::var("HOSTNAME")
                .or_else(|_| std::env::var("HOST"))
                .unwrap_or_else(|_| "localhost".into()),
            pid: std::process::id(),
            roots,
            started: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            role: Role::Unbound,
        };
        let path = socket(&dir, &instance.id)?;
        let listener = UnixListener::bind(&path)?;
        if let Err(e) = (|| {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            listener.set_nonblocking(true)?;
            metadata(&dir, &instance)
        })() {
            let _ = fs::remove_file(path);
            return Err(e);
        }
        let (control, rx) = unbounded();
        let (tx, events) = unbounded();
        let status = Arc::new(Mutex::new(Status::default()));
        let shared = status.clone();
        let registration = instance.clone();
        let worker_dir = dir.clone();
        let worker = thread::spawn(move || run(listener, rx, tx, shared, registration, worker_dir));
        Ok(Self {
            instance,
            status,
            events,
            control,
            worker: Some(worker),
            dir,
        })
    }
    pub fn id(&self) -> &str {
        &self.instance.id
    }
    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }
    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }
    /// Queue a bind. Connection failures are reported through `events`.
    pub fn bind(&self, target: &str, remote_bin: &str) -> io::Result<()> {
        parse_target(target)?;
        self.control
            .send(Control::Bind(target.into(), remote_bin.into()))
            .map_err(|_| error("peer stopped"))
    }
    pub fn unbind(&self) {
        let _ = self.control.send(Control::Unbind);
    }
}
use std::os::unix::fs::MetadataExt;
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Ok(path) = socket(&self.dir, self.id()) {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_file(self.dir.join(format!("{}.json", self.id())));
    }
}

/// List registrations. Delete only entries whose process is gone AND socket refuses connection.
pub fn list() -> io::Result<Vec<Instance>> {
    list_in(&directory()?)
}
fn list_in(dir: &Path) -> io::Result<Vec<Instance>> {
    let entries = match fs::read_dir(dir) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    let mut result = vec![];
    for entry in entries {
        let entry = entry?;
        if entry.path().extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        let Ok(instance) = serde_json::from_slice::<Instance>(&bytes) else {
            continue;
        };
        if entry.file_name() != format!("{}.json", instance.id).as_str() {
            continue;
        }
        let Ok(path) = socket(dir, &instance.id) else {
            continue;
        };
        let dead = rustix::process::Pid::from_raw(instance.pid as i32).is_none_or(|pid| {
            rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH)
        });
        if dead && UnixStream::connect(&path).is_err() {
            let _ = fs::remove_file(path);
            let _ = fs::remove_file(entry.path());
        } else {
            result.push(instance);
        }
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}

fn parse_target(target: &str) -> io::Result<(Option<String>, String)> {
    let parts: Vec<_> = target.split_whitespace().collect();
    let (host, id) = match parts.as_slice() {
        [host, id] => (Some((*host).to_string()), (*id).to_string()),
        [one] => match one.rsplit_once(':') {
            Some((host, id)) => (Some(host.to_string()), id.to_string()),
            None => (None, (*one).to_string()),
        },
        _ => return Err(error("use ID, HOST ID, or HOST:ID")),
    };
    if let Some(host) = &host
        && (host.is_empty()
            || host.starts_with('-')
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-@".contains(&b)))
    {
        return Err(error("invalid SSH host"));
    }
    socket(Path::new(""), &id)?;
    Ok((host, id))
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn connect(dir: &Path, target: &str, bin: &str) -> io::Result<(UnixStream, Option<Child>)> {
    let (host, id) = parse_target(target)?;
    let Some(host) = host else {
        return Ok((UnixStream::connect(socket(dir, &id)?)?, None));
    };
    let binary = if let Some(rest) = bin.strip_prefix("~/") {
        format!("\"$HOME\"/{}", quote(rest))
    } else {
        quote(bin)
    };
    let mut child = Command::new(std::env::var_os("DFDU_SSH").unwrap_or_else(|| "ssh".into()))
        .args([
            "-T",
            "-o",
            "BatchMode=yes",
            &host,
            &format!("{binary} peer-attach {}", quote(&id)),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let (stream, bridge) = UnixStream::pair()?;
    let mut to_child = bridge.try_clone()?;
    let mut input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    thread::spawn(move || {
        let _ = io::copy(&mut to_child, &mut input);
    });
    thread::spawn(move || {
        let mut bridge = bridge;
        let _ = io::copy(&mut output, &mut bridge);
        let _ = bridge.shutdown(Shutdown::Both);
    });
    Ok((stream, Some(child)))
}

struct Link {
    stream: UnixStream,
    child: Option<Child>,
    input: Vec<u8>,
    output: Vec<u8>,
    last: Instant,
    ping: Instant,
    role: Role,
    ready: bool,
}
impl Drop for Link {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Link {
    fn new(stream: UnixStream, child: Option<Child>, role: Role, id: &str) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        let mut link = Self {
            stream,
            child,
            input: vec![],
            output: vec![],
            last: Instant::now(),
            ping: Instant::now(),
            role,
            ready: false,
        };
        link.queue(Frame::Hello {
            proto: 1,
            id: id.into(),
            role,
        });
        Ok(link)
    }
    fn queue(&mut self, frame: Frame) {
        self.output.extend(serde_json::to_vec(&frame).unwrap());
        self.output.push(b'\n');
    }
    fn flush(&mut self) -> io::Result<()> {
        while !self.output.is_empty() {
            match self.stream.write(&self.output) {
                Ok(0) => return Err(error("peer closed")),
                Ok(n) => {
                    self.output.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    fn poll(&mut self) -> io::Result<Option<String>> {
        self.flush()?;
        let mut peer = None;
        let mut bytes = [0; 4096];
        // Bound work per tick so an abusive writer cannot starve control messages.
        for _ in 0..32 {
            match self.stream.read(&mut bytes) {
                Ok(0) => return Err(error("peer closed")),
                Ok(n) => {
                    for byte in &bytes[..n] {
                        if self.input.len() >= LIMIT {
                            return Err(error("peer frame exceeds 64 KiB"));
                        }
                        self.input.push(*byte);
                        if *byte != b'\n' {
                            continue;
                        }
                        let frame: Frame =
                            serde_json::from_slice(&self.input).map_err(io::Error::other)?;
                        self.input.clear();
                        match frame {
                            Frame::Hello { proto: 1, id, role }
                                if !self.ready
                                    && role != self.role
                                    && role != Role::Unbound
                                    && !id.is_empty() =>
                            {
                                self.ready = true;
                                peer = Some(id);
                            }
                            Frame::Ping if self.ready => {}
                            Frame::Bye => return Err(error("peer disconnected")),
                            _ => return Err(error("invalid peer handshake or protocol")),
                        }
                        self.last = Instant::now();
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        if self.last.elapsed() >= TIMEOUT {
            return Err(error("peer timed out"));
        }
        if self.ready && self.ping.elapsed() >= Duration::from_secs(2) {
            self.queue(Frame::Ping);
            self.ping = Instant::now();
        }
        Ok(peer)
    }
}
fn publish(
    status: &Arc<Mutex<Status>>,
    tx: &Sender<Event>,
    instance: &mut Instance,
    dir: &Path,
    value: Status,
) {
    *status.lock().unwrap() = value.clone();
    instance.role = value.role;
    let _ = metadata(dir, instance);
    let _ = tx.send(Event::Status(value));
}
fn run(
    listener: UnixListener,
    rx: Receiver<Control>,
    tx: Sender<Event>,
    status: Arc<Mutex<Status>>,
    mut instance: Instance,
    dir: PathBuf,
) {
    let mut link: Option<Link> = None;
    loop {
        while let Ok(control) = rx.try_recv() {
            match control {
                Control::Stop => {
                    if let Some(link) = &mut link {
                        link.queue(Frame::Bye);
                        let _ = link.flush();
                    }
                    return;
                }
                Control::Unbind => {
                    if let Some(link) = &mut link {
                        link.queue(Frame::Bye);
                        let _ = link.flush();
                    }
                    link = None;
                    publish(&status, &tx, &mut instance, &dir, Status::default());
                }
                Control::Bind(target, bin) => {
                    if link.is_some() {
                        let _ = tx.send(Event::Error("already bound or connecting".into()));
                        continue;
                    }
                    match connect(&dir, &target, &bin)
                        .and_then(|(s, c)| Link::new(s, c, Role::Master, &instance.id))
                    {
                        Ok(value) => link = Some(value),
                        Err(e) => {
                            let _ = tx.send(Event::Error(e.to_string()));
                        }
                    }
                }
            }
        }
        for _ in 0..16 {
            match listener.accept() {
                Ok((stream, _)) if link.is_none() => {
                    match Link::new(stream, None, Role::Slave, &instance.id) {
                        Ok(value) => link = Some(value),
                        Err(e) => {
                            let _ = tx.send(Event::Error(e.to_string()));
                        }
                    }
                }
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(true);
                    let _ = stream.write_all(b"{\"type\":\"bye\"}\n");
                }
                Err(_) => break,
            }
        }
        if let Some(active) = &mut link {
            match active.poll() {
                Ok(Some(id)) => publish(
                    &status,
                    &tx,
                    &mut instance,
                    &dir,
                    Status {
                        role: active.role,
                        peer: Some(id),
                    },
                ),
                Ok(None) => {}
                Err(e) => {
                    link = None;
                    let _ = tx.send(Event::Error(e.to_string()));
                    publish(&status, &tx, &mut instance, &dir, Status::default());
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Bridge standard input and output to a registered instance until either side closes.
/// The stdin reader is detached if the socket closes first; the command should exit afterwards.
pub fn attach(id: &str) -> io::Result<()> {
    let mut stream = UnixStream::connect(socket(&directory()?, id)?)?;
    let mut input = stream.try_clone()?;
    thread::spawn(move || {
        let _ = io::copy(&mut io::stdin().lock(), &mut input);
        let _ = input.shutdown(Shutdown::Both);
    });
    let result = io::copy(&mut stream, &mut io::stdout().lock());
    let _ = stream.shutdown(Shutdown::Both);
    result.map(|_| ())
}
