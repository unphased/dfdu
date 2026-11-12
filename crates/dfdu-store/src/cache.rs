//! The scan cache: the browser's last complete scan of a directory, kept as a snapshot so the
//! next `dfdu PATH` opens it at once and brings it up to date in the background, as
//! `--resume-from` does with a file. One file per directory and set of scan options, named by
//! a hash of them ([`Key`]), in [`dfdu_sys::dirs::cache_dir`].
//!
//! What is read is the user's own: the directory is created `0700` and the files `0600`, and a
//! file or directory of another user (a `sudo dfdu` that kept `HOME`) is never read nor
//! written. A directory that exists already (one named by `DFDU_CACHE_DIR`) is used as it is,
//! and only when it is the user's and other users cannot write to it; its other files are left
//! alone: only those named as the cache names them are read or removed. A damaged or cut-short
//! file is removed, and the directory scanned instead.
//!
//! Expiry: a file is loaded within one hour of being written, configurable with
//! `DFDU_CACHE_MAX_AGE`. The sweep removes files older than [`MAX_AGE`],
//! and keeps the files together under [`SIZE_CAP`] by removing the least recently
//! written first. A tree whose last full scan started more than [`FULL_RESCAN_AFTER`] ago is
//! rescanned whole when it is loaded: a change journal can miss changes, and every save would
//! otherwise keep the cache young.

use std::fs;
use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use dfdu_scan::{Backend, JobOptions};

use crate::Limits;
use crate::snapshot::{self, Header};

mod slot;

pub use slot::{Hit, Saved, Slot};

/// Seconds after it was written that a cached scan is kept by the sweep.
pub const MAX_AGE: u64 = 14 * 86_400;
/// Default maximum age in seconds for loading a cached scan.
const DEFAULT_LOAD_AGE: u64 = 3600;
/// Seconds after the last full scan started that a loaded tree is rescanned whole.
pub const FULL_RESCAN_AFTER: u64 = 7 * 86_400;
/// The most bytes the cached scans take together.
pub const SIZE_CAP: u64 = 1 << 30;
/// Scans that took less than this many milliseconds are not cached: scanning again is about
/// as quick as loading.
pub const MIN_SCAN_MS: u64 = 3_000;
/// The environment variable that turns the cache off: `DFDU_CACHE=0`.
pub const OFF_VAR: &str = "DFDU_CACHE";
/// Entries a save adds per step, while it holds the tree still.
const STEP_ENTRIES: usize = 32_768;
/// The extension of cached scans.
const EXT: &str = "snap";

/// Whether the environment turns the cache off: [`OFF_VAR`] is `0`, `false`, `no` or `off`.
#[must_use]
pub fn off_by_env() -> bool {
    std::env::var_os(OFF_VAR).is_some_and(|v| is_off(&v))
}

/// Whether a value of [`OFF_VAR`] turns the cache off.
fn is_off(v: &std::ffi::OsStr) -> bool {
    let v = v.to_string_lossy().to_ascii_lowercase();
    matches!(v.trim(), "0" | "false" | "no" | "off")
}

/// A whole number of seconds, minutes, hours or days, with its unit.
fn parse_max_age(value: &str) -> Option<u64> {
    let value = value.trim();
    let split = value.find(|c: char| !c.is_ascii_digit())?;
    let count: u64 = value[..split].parse().ok()?;
    let unit = match &value[split..] {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return None,
    };
    count.checked_mul(unit)
}

fn configured_max_age(value: Option<&std::ffi::OsStr>) -> u64 {
    let Some(value) = value else {
        return DEFAULT_LOAD_AGE;
    };
    value.to_str().and_then(parse_max_age).unwrap_or_else(|| {
        eprintln!(
            "dfdu: invalid DFDU_CACHE_MAX_AGE; expected a duration such as 20m, 1h or 2d; using 1h"
        );
        DEFAULT_LOAD_AGE
    })
}

/// The effective user, who owns what the cache reads and writes; 0 where users are not told
/// apart this way.
fn euid() -> u32 {
    #[cfg(unix)]
    {
        rustix::process::geteuid().as_raw()
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// Whether `m` is of a file or directory the effective user owns (always where users are not
/// told apart by owner).
fn owned(m: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        m.uid() == euid()
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        true
    }
}

/// Seconds since the Unix epoch of `t`, 0 before it.
fn secs(t: std::time::SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Seconds since the Unix epoch now.
#[must_use]
pub fn now() -> u64 {
    secs(std::time::SystemTime::now())
}

/// Whether `name` is of a cached scan: the hash of its key in 16 hex digits
/// ([`Key::file_name`]), then the extension.
fn is_scan_name(name: &str) -> bool {
    name.strip_suffix(EXT)
        .and_then(|n| n.strip_suffix('.'))
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// Whether `name` is of the temporary file of a save ([`crate::write_replacing`]) of a cached
/// scan.
fn is_temp_name(name: &[u8]) -> bool {
    crate::temp_target(name)
        .and_then(|t| std::str::from_utf8(t).ok())
        .is_some_and(is_scan_name)
}

/// `PermissionDenied`, saying why `dir` is not used.
fn refused(dir: &Path, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{} {why}", dir.display()),
    )
}

/// Whether the directory `dir`, of metadata `m` (not followed through a symbolic link), may hold
/// the cache as it is: a directory of the effective user that other users cannot write to.
fn usable(dir: &Path, m: &fs::Metadata) -> io::Result<()> {
    if m.is_symlink() {
        return Err(refused(dir, "is a symbolic link"));
    }
    if !m.is_dir() {
        return Err(refused(dir, "is not a directory"));
    }
    if !owned(m) {
        return Err(refused(dir, "belongs to another user"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if m.permissions().mode() & 0o022 != 0 {
            return Err(refused(dir, "is writable by other users"));
        }
    }
    Ok(())
}

/// FNV-1a, 64 bits: stable across builds and platforms, as file names must be.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// What a cached scan is of: the directory, made canonical, the user and the options that
/// change what the scan holds. A directory replaced by another of the same name (another
/// device or inode) has another key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    root: PathBuf,
    pub cross_mounts: bool,
    pub network: bool,
    pub mft: bool,
    euid: u32,
    dev: u64,
    ino: u64,
}

impl Key {
    /// The key of a scan of the directory `path` made with `job`, read from the MFT when `mft`.
    ///
    /// # Errors
    ///
    /// When `path` cannot be made canonical or read, or is not a directory.
    pub fn new(path: &Path, job: &JobOptions, mft: bool) -> io::Result<Key> {
        let root = dunce::canonicalize(path)?;
        let m = fs::metadata(&root)?;
        if !m.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "not a directory",
            ));
        }
        // As the scanners stat it: on Windows std has no volume serial or file id, and the
        // creation time standing in for them is carried over to a directory made again under
        // the same name.
        let meta = dfdu_scan::PortableBackend::default().stat_dir(&root)?;
        Ok(Key {
            root,
            cross_mounts: job.cross_mounts,
            network: job.network,
            mft,
            euid: euid(),
            dev: meta.dev,
            ino: meta.ino,
        })
    }

    /// The directory, as the scan's root is named.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The name of the key's file: a hash of all of it.
    #[must_use]
    pub fn file_name(&self) -> String {
        let mut b = b"dfdu scan cache\0".to_vec();
        b.extend_from_slice(&dfdu_tree::name_bytes(self.root.as_os_str()));
        b.push(0);
        b.push(u8::from(self.cross_mounts) | u8::from(self.network) << 1 | u8::from(self.mft) << 2);
        b.extend_from_slice(&self.euid.to_le_bytes());
        b.extend_from_slice(&self.dev.to_le_bytes());
        b.extend_from_slice(&self.ino.to_le_bytes());
        format!("{:016x}.{EXT}", fnv(&b))
    }
}

/// The cache directory.
#[derive(Clone, Debug)]
pub struct Cache {
    dir: PathBuf,
    max_age: u64,
}

/// A cached scan as `dfdu cache list` shows it.
#[derive(Debug)]
pub struct Listed {
    pub path: PathBuf,
    pub bytes: u64,
    /// Seconds since the Unix epoch when the file was last written.
    pub modified: u64,
    /// The scan's root and header, `None` when the file's start cannot be read.
    pub scan: Option<(PathBuf, Header)>,
    /// The entries its footer counts.
    pub entries: Option<u64>,
}

/// What a sweep or a clear removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Removed {
    pub files: usize,
    pub bytes: u64,
}

/// A cached scan file of the effective user: where, how large, when last written.
struct File {
    path: PathBuf,
    bytes: u64,
    modified: u64,
}

impl Cache {
    /// The cache in [`dfdu_sys::dirs::cache_dir`]; `None` when [`off_by_env`] or without a
    /// cache directory, and why it is not used when [`Cache::at`] refuses it.
    #[must_use]
    pub fn open_default() -> Option<io::Result<Cache>> {
        if off_by_env() {
            return None;
        }
        Some(Cache::at(dfdu_sys::dirs::cache_dir()?))
    }

    /// The cache in `dir`, which need not exist yet: it is then created when a scan is saved.
    ///
    /// # Errors
    ///
    /// `PermissionDenied` when `dir` exists and may not hold the cache as it is (another user's,
    /// writable by other users, not a directory), or when the nearest directory above it that
    /// exists is another user's; `NotFound` when none above it exists.
    pub fn at(dir: PathBuf) -> io::Result<Cache> {
        match fs::symlink_metadata(&dir) {
            Ok(m) => usable(&dir, &m)?,
            Err(_) => {
                let mut d = dir.as_path();
                let m = loop {
                    match fs::metadata(d) {
                        Ok(m) => break m,
                        Err(e) => d = d.parent().ok_or(e)?,
                    }
                };
                if !owned(&m) {
                    return Err(refused(d, "belongs to another user"));
                }
            }
        }
        Ok(Cache {
            dir,
            max_age: configured_max_age(std::env::var_os("DFDU_CACHE_MAX_AGE").as_deref()),
        })
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where the scan `key` is cached.
    #[must_use]
    pub fn slot(&self, key: Key) -> Slot {
        Slot {
            path: self.dir.join(key.file_name()),
            key,
            cache: self.clone(),
        }
    }

    /// Creates the directory, the user's alone, when there is none; one that exists is used as
    /// it is, and only when it may hold the cache ([`Cache::at`]).
    fn ensure_dir(&self) -> io::Result<()> {
        match fs::symlink_metadata(&self.dir) {
            Ok(m) => return usable(&self.dir, &m),
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            Err(_) => {}
        }
        let mut b = fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(&self.dir)?;
        usable(&self.dir, &fs::symlink_metadata(&self.dir)?)
    }

    /// The cached scans of the effective user, newest first; none when there is no directory.
    fn files(&self) -> io::Result<Vec<File>> {
        let rd = match fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!("{}: {e}", self.dir.display()),
                ));
            }
        };
        let mut out: Vec<File> = rd
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if !is_scan_name(&e.file_name().to_string_lossy()) {
                    return None;
                }
                let m = fs::symlink_metadata(&path).ok()?;
                (m.is_file() && owned(&m)).then(|| File {
                    bytes: m.len(),
                    modified: m.modified().map_or(0, secs),
                    path,
                })
            })
            .collect();
        out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
        Ok(out)
    }

    /// Removes the cached scans written more than [`MAX_AGE`] before `now`, then the least
    /// recently written until the rest take at most `cap` bytes, and what dead saves left.
    /// `keep`, the scan just written or in use, stays and counts first.
    pub fn sweep(&self, now: u64, cap: u64, keep: Option<&Path>) -> Removed {
        crate::remove_stale_temps(&self.dir, is_temp_name);
        let mut removed = Removed::default();
        // Best effort: a directory that cannot be read has nothing to sweep.
        let files = self.files().unwrap_or_default();
        let is_kept = |f: &File| keep == Some(f.path.as_path());
        // The file kept counts first, then the others newest first.
        let mut total: u64 = files.iter().filter(|f| is_kept(f)).map(|f| f.bytes).sum();
        for f in files.iter().filter(|f| !is_kept(f)) {
            let old = now.saturating_sub(f.modified) > MAX_AGE;
            if old || total.saturating_add(f.bytes) > cap {
                if fs::remove_file(&f.path).is_ok() {
                    removed.files += 1;
                    removed.bytes += f.bytes;
                }
                continue;
            }
            total = total.saturating_add(f.bytes);
        }
        removed
    }

    /// Removes every cached scan of the effective user.
    ///
    /// # Errors
    ///
    /// The directory could not be read, or the first file that could not be removed.
    pub fn clear(&self) -> io::Result<Removed> {
        crate::remove_stale_temps(&self.dir, is_temp_name);
        let mut removed = Removed::default();
        for f in self.files()? {
            match fs::remove_file(&f.path) {
                Ok(()) => {
                    removed.files += 1;
                    removed.bytes += f.bytes;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("{}: {e}", f.path.display()),
                    ));
                }
            }
        }
        Ok(removed)
    }

    /// The cached scans of the effective user, newest first, with what their headers say.
    ///
    /// # Errors
    ///
    /// The directory could not be read.
    pub fn list(&self) -> io::Result<Vec<Listed>> {
        Ok(self
            .files()?
            .into_iter()
            .map(|f| {
                let file = open_own(&f.path).ok();
                let scan = file.as_ref().and_then(|file| {
                    let (root, h) =
                        snapshot::read_header(io::BufReader::new(file), &Limits::default()).ok()?;
                    Some((PathBuf::from(dfdu_tree::name_os(&root).into_owned()), h))
                });
                let entries = file.as_ref().and_then(|file| footer_entries(file).ok());
                Listed {
                    path: f.path,
                    bytes: f.bytes,
                    modified: f.modified,
                    scan,
                    entries,
                }
            })
            .collect())
    }
}

/// Opens `path` for reading when it is a regular file of the effective user, not through a
/// symbolic link; `NotFound` when there is none, `PermissionDenied` when it is not such a file.
fn open_own(path: &Path) -> io::Result<fs::File> {
    let not_own = || io::Error::new(io::ErrorKind::PermissionDenied, "not this user's file");
    if fs::symlink_metadata(path)?.is_symlink() {
        return Err(not_own());
    }
    let mut o = fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits().cast_signed());
    }
    let f = o.open(path)?;
    let m = f.metadata()?;
    if !m.is_file() || !owned(&m) {
        return Err(not_own());
    }
    Ok(f)
}

/// The entries the footer of the snapshot in `f` counts (not checked).
fn footer_entries(mut f: &fs::File) -> io::Result<u64> {
    f.seek(io::SeekFrom::End(-24))?;
    let mut b = [0u8; 8];
    f.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

#[cfg(test)]
mod tests;
