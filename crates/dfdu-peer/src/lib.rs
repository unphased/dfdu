//! Local instance registration and authenticated SSH/stdin peer transport.
#![cfg(unix)]
#[cfg(test)]
mod tests;

use crossbeam_channel::{Receiver, Sender, unbounded};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::{self, Read, Write},
    net::Shutdown,
    os::fd::AsFd,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_REMOTE_BIN: &str = "~/.local/bin/dfdu";
const LIMIT: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(250);

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
#[serde(rename_all = "lowercase")]
enum Frame {
    Hello(Hello),
    Ping {},
    Bye {},
}
#[derive(Serialize, Deserialize, Debug)]
struct Hello {
    proto: u32,
    id: String,
    host: String,
    role: Role,
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
fn random_id() -> io::Result<String> {
    let mut bytes = [0u8; 4];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn stale_error(e: &io::Error) -> bool {
    // Darwin also reports ECONNREFUSED for a full live listener backlog.
    e.raw_os_error() == Some(libc::ENOENT)
        || (cfg!(target_os = "linux") && e.raw_os_error() == Some(libc::ECONNREFUSED))
}
fn nonblocking(fd: &impl AsFd) -> io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd)?;
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?;
    Ok(())
}
fn connect_local(path: &Path) -> io::Result<UnixStream> {
    use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
    let address = SocketAddrUnix::new(path)?;
    #[cfg(not(target_vendor = "apple"))]
    let flags = SocketFlags::CLOEXEC | SocketFlags::NONBLOCK;
    #[cfg(target_vendor = "apple")]
    let flags = SocketFlags::empty();
    let fd = rustix::net::socket_with(AddressFamily::UNIX, SocketType::STREAM, flags, None)?;
    // Darwin has no atomic SOCK_CLOEXEC/SOCK_NONBLOCK socket flags.
    rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC)?;
    let stream = UnixStream::from(fd);
    nonblocking(&stream)?;
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        let e = match rustix::net::connect(&stream, &address) {
            Ok(()) => return Ok(stream),
            Err(e) => io::Error::from(e),
        };
        match e.raw_os_error() {
            Some(libc::EISCONN) => return Ok(stream),
            Some(libc::EINPROGRESS | libc::EALREADY) => {
                use rustix::event::{PollFd, PollFlags, Timespec, poll};
                let mut fds = [PollFd::new(&stream, PollFlags::OUT)];
                let timeout = Timespec {
                    tv_sec: 0,
                    tv_nsec: 5_000_000,
                };
                match poll(&mut fds, Some(&timeout)) {
                    Ok(n) if n > 0 => {
                        if let Some(e) = stream.take_error()? {
                            return Err(e);
                        }
                        return Ok(stream);
                    }
                    Ok(_) | Err(rustix::io::Errno::INTR) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Some(libc::EAGAIN | libc::EINTR) => thread::sleep(Duration::from_millis(5)),
            _ => return Err(e),
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "peer connect timed out",
            ));
        }
    }
}
struct Transport {
    child: Child,
    stop: Arc<AtomicBool>,
    threads: Vec<thread::JoinHandle<()>>,
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(pid) = rustix::process::Pid::from_raw(self.child.id() as i32) {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
fn pump(mut input: impl Read, mut output: impl Write, stop: Arc<AtomicBool>) {
    let mut bytes = [0; 8192];
    while !stop.load(Ordering::Relaxed) {
        match input.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => {
                let mut pending = &bytes[..n];
                while !pending.is_empty() && !stop.load(Ordering::Relaxed) {
                    match output.write(pending) {
                        Ok(0) => return,
                        Ok(n) => pending = &pending[n..],
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => return,
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
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
    let temporary = dir.join(format!("{}.tmp", instance.id));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        serde_json::to_writer(&mut file, instance).map_err(io::Error::other)?;
        fs::rename(&temporary, dir.join(format!("{}.json", instance.id)))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
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
            id: random_id()?,
            host: rustix::system::uname()
                .nodename()
                .to_string_lossy()
                .into_owned(),
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
    /// Return a hook for the application's signal worker or panic hook.
    /// Calling it requests shutdown and registration cleanup without installing global handlers.
    /// Do not call from a raw signal handler; use a signal iterator or the application's event loop.
    pub fn cleanup_hook(&self) -> impl Fn() + Send + Sync + 'static {
        let control = self.control.clone();
        move || {
            let _ = control.send(Control::Stop);
        }
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

/// List registrations, removing entries with a dead process or definitively stale endpoint.
/// On Darwin, a live process's refused connection is retained because a full backlog also refuses.
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
        let dead = instance.pid > i32::MAX as u32
            || rustix::process::Pid::from_raw(instance.pid as i32).is_none_or(|pid| {
                rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH)
            });
        if dead || connect_local(&path).is_err_and(|e| stale_error(&e)) {
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
fn connect(dir: &Path, target: &str, bin: &str) -> io::Result<(UnixStream, Option<Transport>)> {
    let (host, id) = parse_target(target)?;
    let Some(host) = host else {
        return Ok((connect_local(&socket(dir, &id)?)?, None));
    };
    let binary = if let Some(rest) = bin.strip_prefix("~/") {
        format!("\"$HOME\"/{}", quote(rest))
    } else {
        quote(bin)
    };
    let child = Command::new(std::env::var_os("DFDU_SSH").unwrap_or_else(|| "ssh".into()))
        .args([
            "-T",
            "-o",
            "BatchMode=yes",
            &host,
            &format!("{binary} peer-attach {}", quote(&id)),
        ])
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut transport = Transport {
        child,
        stop: Arc::new(AtomicBool::new(false)),
        threads: vec![],
    };
    let (stream, bridge) = UnixStream::pair()?;
    let to_child = bridge.try_clone()?;
    let input = transport.child.stdin.take().unwrap();
    let output = transport.child.stdout.take().unwrap();
    nonblocking(&to_child)?;
    nonblocking(&bridge)?;
    nonblocking(&input)?;
    nonblocking(&output)?;
    let stop = transport.stop.clone();
    transport
        .threads
        .push(thread::spawn(move || pump(to_child, input, stop)));
    let stop = transport.stop.clone();
    transport.threads.push(thread::spawn(move || {
        pump(output, &bridge, stop);
        let _ = bridge.shutdown(Shutdown::Both);
    }));
    Ok((stream, Some(transport)))
}

struct Link {
    stream: UnixStream,
    _child: Option<Transport>,
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
    }
}
impl Link {
    fn new(
        stream: UnixStream,
        child: Option<Transport>,
        role: Role,
        id: &str,
        host: &str,
    ) -> io::Result<Self> {
        let mut link = Self {
            stream,
            _child: child,
            input: vec![],
            output: vec![],
            last: Instant::now(),
            ping: Instant::now(),
            role,
            ready: false,
        };
        link.stream.set_nonblocking(true)?;
        link.queue(Frame::Hello(Hello {
            proto: 1,
            id: id.into(),
            host: host.into(),
            role,
        }));
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
                            Frame::Hello(Hello {
                                proto: 1,
                                id,
                                host,
                                role,
                            }) if !self.ready
                                && role != self.role
                                && role != Role::Unbound
                                && !id.is_empty()
                                && !host.is_empty() =>
                            {
                                self.ready = true;
                                peer = Some(format!("{host}/{id}"));
                            }
                            Frame::Ping {} if self.ready => {}
                            Frame::Bye {} => return Err(error("peer disconnected")),
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
            self.queue(Frame::Ping {});
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
    instance.role = value.role;
    let _ = metadata(dir, instance);
    *status.lock().unwrap() = value.clone();
    let _ = tx.send(Event::Status(value));
}
struct Registration {
    dir: PathBuf,
    id: String,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(path) = socket(&self.dir, &self.id) {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_file(self.dir.join(format!("{}.json", self.id)));
    }
}
fn run(
    listener: UnixListener,
    rx: Receiver<Control>,
    tx: Sender<Event>,
    status: Arc<Mutex<Status>>,
    mut instance: Instance,
    dir: PathBuf,
) {
    let _registration = Registration {
        dir: dir.clone(),
        id: instance.id.clone(),
    };
    let mut link: Option<Link> = None;
    loop {
        while let Ok(control) = rx.try_recv() {
            match control {
                Control::Stop => {
                    if let Some(link) = &mut link {
                        link.queue(Frame::Bye {});
                        let _ = link.flush();
                    }
                    return;
                }
                Control::Unbind => {
                    if let Some(link) = &mut link {
                        link.queue(Frame::Bye {});
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
                    match connect(&dir, &target, &bin).and_then(|(s, c)| {
                        Link::new(s, c, Role::Master, &instance.id, &instance.host)
                    }) {
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
                    // Listing probes connect and immediately close without a handshake.
                    let mut byte = [0u8; 1];
                    if matches!(
                        rustix::net::recv(
                            &stream,
                            &mut byte,
                            rustix::net::RecvFlags::PEEK | rustix::net::RecvFlags::DONTWAIT
                        ),
                        Ok((0, _))
                    ) {
                        continue;
                    }
                    match Link::new(stream, None, Role::Slave, &instance.id, &instance.host) {
                        Ok(value) => link = Some(value),
                        Err(e) => {
                            let _ = tx.send(Event::Error(e.to_string()));
                        }
                    }
                }
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(true);
                    let _ = stream.write_all(b"{\"bye\":{}}\n");
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
    let mut stream = connect_local(&socket(&directory()?, id)?)?;
    stream.set_nonblocking(false)?;
    let mut input = stream.try_clone()?;
    thread::spawn(move || {
        let _ = io::copy(&mut io::stdin().lock(), &mut input);
        let _ = input.shutdown(Shutdown::Both);
    });
    let result = io::copy(&mut stream, &mut io::stdout().lock());
    let _ = stream.shutdown(Shutdown::Both);
    result.map(|_| ())
}
