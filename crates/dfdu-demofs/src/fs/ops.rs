//! The requests the kernel forwards, each answered from the tree under its lock.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use fuser::{
    BsdFileFlags, Errno, FileAttr, FileHandle, Filesystem, FopenFlags, Generation, INodeNo,
    LockOwner, OpenFlags, Owner, RenameFlags, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory,
    ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request, TimeOrNow, WriteFlags,
};

use super::{BLOCK, DemoFs, FILES, TTL, attr, entry, errno, listing, node, stat};
use crate::tree::{FsError, Kind, Tree};

/// What a `setattr` request changes; `None` leaves that attribute as it is.
#[derive(Clone, Copy, Default)]
struct Changes {
    mode: Option<u32>,
    uid: Option<u32>,
    gid: Option<u32>,
    size: Option<u64>,
    mtime: Option<TimeOrNow>,
}

/// Applies `c` to node `n` at `now` (the size first, which may fail), and gives its attributes
/// after.
fn set_attr(t: &mut Tree, n: usize, c: Changes, now: i64) -> Result<FileAttr, FsError> {
    if t.get(n).is_none() {
        return Err(FsError::NoEnt);
    }
    if let Some(s) = c.size {
        t.truncate(n, s, now)?;
    }
    let i = &mut t.nodes[n];
    if let Some(m) = c.mode {
        i.mode = m & 0o7777;
    }
    if let Some(u) = c.uid {
        i.uid = u;
    }
    if let Some(g) = c.gid {
        i.gid = g;
    }
    match c.mtime {
        Some(TimeOrNow::SpecificTime(st)) => {
            let secs = st.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
            i.mtime = i64::try_from(secs).unwrap_or(0);
        }
        Some(TimeOrNow::Now) => i.mtime = now,
        None => {}
    }
    attr(t, n)
}

/// Makes `name` in `parent`, of `kind`, permissions `mode` and the time `now`, owned by the
/// user and group `by` (the request's).
fn make(
    t: &mut Tree,
    parent: usize,
    name: &[u8],
    kind: Kind,
    now: i64,
    mode: u32,
    by: (u32, u32),
) -> Result<usize, FsError> {
    let n = t.insert(parent, name, kind, now, mode)?;
    (t.nodes[n].uid, t.nodes[n].gid) = by;
    Ok(n)
}

/// What symlink `n` points to.
fn link_target(t: &Tree, n: usize) -> Result<&[u8], FsError> {
    match t.get(n).map(|i| &i.kind) {
        Some(Kind::Symlink(target)) => Ok(target),
        Some(_) => Err(FsError::Inval),
        None => Err(FsError::NoEnt),
    }
}

/// How late directory `n` answers its opening (its fixture's `delay`), or its fault.
fn open_delay(t: &Tree, n: usize) -> Result<Duration, FsError> {
    let i = t.get(n).ok_or(FsError::NoEnt)?;
    if !i.is_dir() {
        return Err(FsError::NotDir);
    }
    if let Some(f) = i.fault {
        return Err(f.into());
    }
    Ok(i.delay)
}

impl Filesystem for DemoFs {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let t = self.tree();
        match t.lookup(node(parent), name.as_bytes()) {
            Ok(n) => entry(&t, n, reply),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        let t = self.tree();
        match stat(&t, node(ino)) {
            Ok(a) => reply.attr(&TTL, &a),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<std::time::SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<std::time::SystemTime>,
        _chgtime: Option<std::time::SystemTime>,
        _bkuptime: Option<std::time::SystemTime>,
        _flags: Option<BsdFileFlags>,
        _kill_suid_gid: bool,
        reply: ReplyAttr,
    ) {
        let now = self.time();
        let changes = Changes {
            mode,
            uid,
            gid,
            size,
            mtime,
        };
        match set_attr(&mut self.tree(), node(ino), changes, now) {
            Ok(a) => reply.attr(&TTL, &a),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn readlink(&self, _req: &Request, ino: INodeNo, reply: ReplyData) {
        match link_target(&self.tree(), node(ino)) {
            Ok(target) => reply.data(target),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        owner: Owner,
        reply: ReplyEntry,
    ) {
        let now = self.time();
        let mut t = self.tree();
        let (name, mode) = (name.as_bytes(), mode & !umask & 0o7777);
        match make(
            &mut t,
            node(parent),
            name,
            Kind::dir(),
            now,
            mode,
            (owner.uid, owner.gid),
        ) {
            Ok(n) => entry(&t, n, reply),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let now = self.time();
        match self.tree().unlink(node(parent), name.as_bytes(), now) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let now = self.time();
        match self.tree().rmdir(node(parent), name.as_bytes(), now) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn symlink(
        &self,
        _req: &Request,
        parent: INodeNo,
        link_name: &OsStr,
        target: &Path,
        owner: Owner,
        reply: ReplyEntry,
    ) {
        let now = self.time();
        let mut t = self.tree();
        let kind = Kind::Symlink(target.as_os_str().as_bytes().to_vec());
        match make(
            &mut t,
            node(parent),
            link_name.as_bytes(),
            kind,
            now,
            0o777,
            (owner.uid, owner.gid),
        ) {
            Ok(n) => entry(&t, n, reply),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
        _owner: Option<Owner>,
        reply: ReplyEmpty,
    ) {
        let known = RenameFlags::RENAME_NOREPLACE | RenameFlags::RENAME_EXCHANGE;
        if !known.contains(flags) {
            reply.error(Errno::EINVAL);
            return;
        }
        let now = self.time();
        let r = self.tree().rename(
            node(parent),
            name.as_bytes(),
            node(newparent),
            newname.as_bytes(),
            flags.contains(RenameFlags::RENAME_NOREPLACE),
            flags.contains(RenameFlags::RENAME_EXCHANGE),
            now,
        );
        match r {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn link(
        &self,
        _req: &Request,
        ino: INodeNo,
        newparent: INodeNo,
        newname: &OsStr,
        reply: ReplyEntry,
    ) {
        let mut t = self.tree();
        match t.link(node(ino), node(newparent), newname.as_bytes()) {
            Ok(()) => entry(&t, node(ino), reply),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn open(
        &self,
        _req: &Request,
        ino: INodeNo,
        _flags: OpenFlags,
        _kill_suid_gid: bool,
        reply: ReplyOpen,
    ) {
        let t = self.tree();
        match stat(&t, node(ino)) {
            Ok(_) => reply.opened(FileHandle(0), FopenFlags::empty()),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        match self.tree().read(node(ino), offset, size) {
            Ok(b) => reply.data(&b),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let now = self.time();
        match self.tree().write(node(ino), offset, data, now) {
            Ok(()) => reply.written(u32::try_from(data.len()).unwrap_or(u32::MAX)),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn fsync(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn opendir(&self, _req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        let delay = match open_delay(&self.tree(), node(ino)) {
            Ok(d) => d,
            Err(e) => return reply.error(errno(e)),
        };
        if delay.is_zero() {
            return reply.opened(FileHandle(0), FopenFlags::empty());
        }
        thread::spawn(move || {
            thread::sleep(delay);
            reply.opened(FileHandle(0), FopenFlags::empty());
        });
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let t = self.tree();
        let listed = match listing(&t, node(ino), offset) {
            Ok(l) => l,
            Err(e) => return reply.error(errno(e)),
        };
        for l in listed {
            if reply.add(l.ino, l.cookie, l.kind, OsStr::from_bytes(l.name)) {
                break;
            }
        }
        reply.ok();
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        let t = self.tree();
        let free = self.free(&t);
        reply.statfs(
            self.total / BLOCK,
            free / BLOCK,
            free / BLOCK,
            FILES,
            FILES.saturating_sub(t.live() as u64),
            4096,
            255,
            4096,
        );
    }

    fn access(&self, _req: &Request, ino: INodeNo, _mask: fuser::AccessFlags, reply: ReplyEmpty) {
        match self.tree().get(node(ino)) {
            Some(_) => reply.ok(),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        _flags: i32,
        _kill_suid_gid: bool,
        owner: Owner,
        reply: ReplyCreate,
    ) {
        let now = self.time();
        let mut t = self.tree();
        let (name, mode) = (name.as_bytes(), mode & !umask & 0o7777);
        let kind = Kind::stored(vec![]);
        let created = make(
            &mut t,
            node(parent),
            name,
            kind,
            now,
            mode,
            (owner.uid, owner.gid),
        )
        .and_then(|n| attr(&t, n));
        match created {
            Ok(a) => reply.created(&TTL, &a, Generation(0), FileHandle(0), FopenFlags::empty()),
            Err(e) => reply.error(errno(e)),
        }
    }
}

#[cfg(test)]
mod tests;
