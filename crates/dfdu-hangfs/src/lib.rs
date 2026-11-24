//! A FUSE filesystem for tests: it serves an in-memory tree in which selected directories never
//! answer `readdir` and selected files never answer `getattr`, like a hung network mount, while
//! the rest answers normally, optionally after a delay so that concurrent requests can be
//! counted.
//!
//! The filesystem runs in the test process (one `fuser` session thread). A request that hangs
//! is kept, unanswered, until `Mounted::release` or drop, which answer every kept request
//! with `EIO` so the threads waiting in the kernel return; drop then unmounts, falling back to
//! aborting the connection and a lazy unmount if the mount is still busy.
//!
//! Mounting needs `/dev/fuse` and either root (a direct `mount(2)`) or `fusermount3`.
//! `Mounted::mount` returns an error otherwise; tests skip with a message.
#![cfg(target_os = "linux")]

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fuser::{
    BackgroundSession, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, Generation,
    INodeNo, InitFlags, KernelConfig, MountOption, ReplyAttr, ReplyDirectory, ReplyEntry, Request,
};

/// One entry of the tree. Directories have `children`; `hang` makes a directory's `readdir`
/// or a file's `getattr` never answer.
#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub hang: bool,
    pub children: Vec<usize>,
}

/// The tree served, node 0 being the root.
#[derive(Clone, Debug)]
pub struct Spec {
    pub nodes: Vec<Node>,
}

impl Default for Spec {
    fn default() -> Self {
        Self::new()
    }
}

impl Spec {
    #[must_use]
    pub fn new() -> Self {
        Spec {
            nodes: vec![Node {
                name: String::new(),
                dir: true,
                size: DIR_SIZE,
                hang: false,
                children: vec![],
            }],
        }
    }

    fn add(&mut self, parent: usize, name: &str, dir: bool, size: u64) -> usize {
        self.nodes.push(Node {
            name: name.into(),
            dir,
            size,
            hang: false,
            children: vec![],
        });
        let i = self.nodes.len() - 1;
        self.nodes[parent].children.push(i);
        i
    }

    pub fn dir(&mut self, parent: usize, name: &str) -> usize {
        self.add(parent, name, true, DIR_SIZE)
    }

    pub fn file(&mut self, parent: usize, name: &str, size: u64) -> usize {
        self.add(parent, name, false, size)
    }

    /// Makes `n` hang: a directory's listing, or a file's stat.
    pub fn hang(&mut self, n: usize) {
        self.nodes[n].hang = true;
    }

    /// Bytes allocated to node `n`, as `st_blocks * 512` reports it.
    #[must_use]
    pub fn alloc(&self, n: usize) -> u64 {
        blocks(self.nodes[n].size) * 512
    }

    /// A tree for scan tests: `dirs` directories `ok<i>` with eight files and a subdirectory of
    /// four, plus three directories a scan cannot finish: `hung` and `deep/hung`, whose
    /// listings never answer, and `slow`, whose file `stuck` never answers its stat. Returns
    /// the spec and the paths of those three, relative to the root.
    #[must_use]
    pub fn hanging(dirs: usize) -> (Spec, Vec<&'static str>) {
        let mut s = Spec::new();
        for i in 0..dirs {
            let d = s.dir(0, &format!("ok{i}"));
            for k in 0..8u64 {
                s.file(
                    d,
                    &format!("f{k}"),
                    1 + (i as u64 * 7919 + k * 104_729) % 300_000,
                );
            }
            let sub = s.dir(d, "sub");
            for k in 0..4u64 {
                s.file(sub, &format!("g{k}"), 100 * k);
            }
        }
        let h = s.dir(0, "hung");
        s.file(h, "never-listed", 10);
        let deep = s.dir(0, "deep");
        s.file(deep, "seen", 4097);
        let h2 = s.dir(deep, "hung");
        s.hang(h);
        s.hang(h2);
        let slow = s.dir(0, "slow");
        for k in 0..5 {
            s.file(slow, &format!("f{k}"), 50_000);
        }
        let stuck = s.file(slow, "stuck", 1);
        s.hang(stuck);
        (s, vec!["hung", "deep/hung", "slow"])
    }

    /// Paths below the root (relative, `/`-separated) with the node at each, in depth-first
    /// order, the root (`""`) first.
    #[must_use]
    pub fn paths(&self) -> Vec<(String, usize)> {
        let mut out = vec![];
        let mut stack = vec![(String::new(), 0)];
        while let Some((p, n)) = stack.pop() {
            for &c in self.nodes[n].children.iter().rev() {
                let name = &self.nodes[c].name;
                stack.push((
                    if p.is_empty() {
                        name.clone()
                    } else {
                        format!("{p}/{name}")
                    },
                    c,
                ));
            }
            out.push((p, n));
        }
        out
    }
}

/// The size directories report.
const DIR_SIZE: u64 = 4096;

fn blocks(size: u64) -> u64 {
    size.div_ceil(4096) * 8
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Reported as the filesystem's subtype (`fuse.<subtype>` in the mount table): `sshfs`
    /// makes scanners treat the mount as a network filesystem.
    pub subtype: Option<String>,
    /// Requests that answer are answered after this long, from another thread.
    pub delay: Duration,
}

/// Counters shared by the filesystem and the test.
#[derive(Default)]
struct State {
    /// Unanswered replies to requests of hanging nodes; dropping one answers it with `EIO`.
    held: Mutex<Vec<Box<dyn Send>>>,
    released: AtomicBool,
    /// Requests being answered after the delay, and the most at once.
    in_flight: AtomicUsize,
    peak: AtomicUsize,
}

impl State {
    fn begin(&self) {
        let n = self.in_flight.fetch_add(1, SeqCst) + 1;
        self.peak.fetch_max(n, SeqCst);
    }
}

type Job = Box<dyn FnOnce() + Send>;

struct HangFs {
    spec: Spec,
    state: Arc<State>,
    delay: Duration,
    /// Answers sent after the delay, in order (every delay is the same).
    later: Mutex<Option<Sender<(Instant, Job)>>>,
}

const TTL: Duration = Duration::from_secs(60);

impl HangFs {
    fn node(&self, ino: INodeNo) -> Option<usize> {
        let i = usize::try_from(u64::from(ino)).ok()?.checked_sub(1)?;
        (i < self.spec.nodes.len()).then_some(i)
    }

    fn attr(&self, i: usize) -> FileAttr {
        let n = &self.spec.nodes[i];
        let t = UNIX_EPOCH + Duration::from_secs(1_000_000);
        FileAttr {
            ino: INodeNo(i as u64 + 1),
            size: n.size,
            blocks: blocks(n.size),
            atime: t,
            mtime: t,
            ctime: t,
            crtime: SystemTime::UNIX_EPOCH,
            kind: if n.dir {
                FileType::Directory
            } else {
                FileType::RegularFile
            },
            perm: if n.dir { 0o755 } else { 0o644 },
            nlink: if n.dir { 2 } else { 1 },
            uid: 0,
            gid: 0,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// Answers now, or after the delay on the delay thread.
    fn answer(&self, f: impl FnOnce() + Send + 'static) {
        self.state.begin();
        let state = self.state.clone();
        let job: Job = Box::new(move || {
            f();
            state.in_flight.fetch_sub(1, SeqCst);
        });
        if self.delay.is_zero() {
            job();
            return;
        }
        let tx = self
            .later
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let job = match tx {
            Some(tx) => match tx.send((Instant::now() + self.delay, job)) {
                Ok(()) => return,
                Err(e) => e.0.1,
            },
            None => job,
        };
        job();
    }

    /// Keeps the request unanswered, unless the filesystem has been released: then the reply
    /// is dropped here, which answers it with `EIO`.
    fn hold(&self, reply: Box<dyn Send>) {
        let mut held = self
            .state
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !self.state.released.load(SeqCst) {
            held.push(reply);
        }
    }
}

impl Filesystem for HangFs {
    fn init(&mut self, _req: &Request, config: &mut KernelConfig) -> io::Result<()> {
        // Lookups and listings in one directory may run in parallel, as on a network
        // filesystem; without this the kernel serialises them per directory.
        let _ = config.add_capabilities(InitFlags::FUSE_PARALLEL_DIROPS);
        let (tx, rx) = channel::<(Instant, Job)>();
        thread::Builder::new()
            .name("hangfs-delay".into())
            .spawn(move || {
                for (at, job) in rx {
                    let now = Instant::now();
                    if at > now {
                        thread::sleep(at - now);
                    }
                    job();
                }
            })?;
        *self.later.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
        Ok(())
    }

    fn destroy(&mut self) {
        self.later
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let Some(p) = self.node(parent) else {
            return reply.error(Errno::ENOENT);
        };
        let child = self.spec.nodes[p]
            .children
            .iter()
            .copied()
            .find(|&c| OsStr::new(&self.spec.nodes[c].name) == name);
        let Some(c) = child else {
            return reply.error(Errno::ENOENT);
        };
        // Attributes of a hanging file are never cached, so every stat asks `getattr`.
        let ttl = if self.spec.nodes[c].hang && !self.spec.nodes[c].dir {
            Duration::ZERO
        } else {
            TTL
        };
        let attr = self.attr(c);
        self.answer(move || reply.entry(&ttl, &attr, Generation(0)));
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        let Some(i) = self.node(ino) else {
            return reply.error(Errno::ENOENT);
        };
        let n = &self.spec.nodes[i];
        if n.hang && !n.dir {
            return self.hold(Box::new(reply));
        }
        let attr = self.attr(i);
        self.answer(move || reply.attr(&TTL, &attr));
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let Some(i) = self.node(ino) else {
            return reply.error(Errno::ENOENT);
        };
        let n = &self.spec.nodes[i];
        if !n.dir {
            return reply.error(Errno::ENOTDIR);
        }
        if n.hang {
            return self.hold(Box::new(reply));
        }
        let mut entries = vec![
            (ino, FileType::Directory, ".".to_string()),
            (ino, FileType::Directory, "..".to_string()),
        ];
        for &c in &n.children {
            let a = self.attr(c);
            entries.push((a.ino, a.kind, self.spec.nodes[c].name.clone()));
        }
        for (k, (ino, kind, name)) in entries
            .into_iter()
            .enumerate()
            .skip(usize::try_from(offset).unwrap_or(usize::MAX))
        {
            if reply.add(ino, k as u64 + 1, kind, &name) {
                break;
            }
        }
        self.answer(move || reply.ok());
    }
}

/// A mounted `HangFs`; unmounted on drop.
pub struct Mounted {
    at: PathBuf,
    state: Arc<State>,
    session: Option<BackgroundSession>,
}

impl Mounted {
    /// Mounts `spec` at the directory `at`.
    ///
    /// # Errors
    ///
    /// There is no `/dev/fuse`, the mount fails (no root and no `fusermount3`), or the mounted
    /// root does not answer.
    pub fn mount(spec: Spec, at: &Path, opts: Options) -> io::Result<Mounted> {
        if !Path::new("/dev/fuse").exists() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no /dev/fuse"));
        }
        let state = Arc::new(State::default());
        let fs = HangFs {
            spec,
            state: state.clone(),
            delay: opts.delay,
            later: Mutex::new(None),
        };
        let mut config = Config::default();
        config.mount_options = vec![MountOption::RO, MountOption::FSName("dfdu-hangfs".into())];
        if let Some(s) = opts.subtype {
            // A kernel option: fuser passes `Subtype` only to fusermount.
            config
                .mount_options
                .push(MountOption::CUSTOM(format!("subtype={s}")));
        }
        let session = fuser::spawn_mount(fs, at, &config)?;
        let m = Mounted {
            at: at.to_path_buf(),
            state,
            session: Some(session),
        };
        // The mount is usable once the root answers.
        std::fs::metadata(at)?;
        Ok(m)
    }

    /// Requests kept unanswered now.
    #[must_use]
    pub fn held(&self) -> usize {
        self.state
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// The most requests that were being answered (after the delay) at once.
    #[must_use]
    pub fn peak_in_flight(&self) -> usize {
        self.state.peak.load(SeqCst)
    }

    /// Answers every kept request with `EIO`, and every later request of a hanging node at
    /// once, so threads waiting in the kernel return.
    pub fn release(&self) {
        // Under the lock, so a request being held now is either taken here or not kept.
        let held = {
            let mut h = self
                .state
                .held
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            self.state.released.store(true, SeqCst);
            std::mem::take(&mut *h)
        };
        drop(held);
    }

    /// Aborts the FUSE connection through `fusectl`, if it is mounted: every request fails
    /// at once.
    fn abort(&self) {
        use std::os::linux::fs::MetadataExt;
        let Ok(m) = std::fs::symlink_metadata(&self.at) else {
            return;
        };
        let minor = rustix::fs::minor(m.st_dev());
        let _ = std::fs::write(format!("/sys/fs/fuse/connections/{minor}/abort"), "1");
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        use rustix::mount::{UnmountFlags, unmount};
        self.release();
        // Threads that were waiting may still hold descriptors for a moment.
        let mut clean = false;
        for _ in 0..50 {
            match unmount(&self.at, UnmountFlags::empty()) {
                Ok(()) => {
                    clean = true;
                    break;
                }
                Err(rustix::io::Errno::BUSY) => thread::sleep(Duration::from_millis(20)),
                Err(_) => break,
            }
        }
        if !clean {
            self.abort();
            if unmount(&self.at, UnmountFlags::DETACH).is_err() {
                // Not root. fuser leaves an aborted mount in place, so detach it through
                // fusermount.
                for bin in ["fusermount3", "fusermount"] {
                    let done = std::process::Command::new(bin)
                        .arg("-uz")
                        .arg(&self.at)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status();
                    if done.is_ok_and(|s| s.success()) {
                        break;
                    }
                }
            }
        }
        if let Some(s) = self.session.take() {
            // The session thread ends when the connection does; a lazily detached mount that is
            // still in use keeps it open. Joining (or dropping) the session waits for that, so it
            // happens on its own thread, which is left behind if the session does not end.
            let (done, ended) = channel();
            thread::spawn(move || {
                let _ = s.umount_and_join();
                let _ = done.send(());
            });
            let _ = ended.recv_timeout(Duration::from_secs(2));
        }
    }
}

#[cfg(test)]
mod tests;
