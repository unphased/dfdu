//! Where one scan is cached: loading it, and saving it a part at a time.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};

use dfdu_tree::Tree;

use super::{
    Cache, FULL_RESCAN_AFTER, Key, Removed, SIZE_CAP, STEP_ENTRIES, open_own, owned, secs,
};
use crate::ImportError;
use crate::Limits;
use crate::snapshot::{self, Header, Loaded, Writer};

/// Where one scan is cached.
#[derive(Clone, Debug)]
pub struct Slot {
    pub path: PathBuf,
    pub key: Key,
    /// The cache it is in.
    pub(super) cache: Cache,
}

/// A cached scan loaded.
pub struct Hit {
    pub loaded: Loaded,
    /// The last full scan is older than [`FULL_RESCAN_AFTER`]: rescan it all rather than ask
    /// the change journal.
    pub full_rescan: bool,
}

/// How a save ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saved {
    /// Written: the entries, the file's size, and the longest the tree was held still at once.
    Written {
        entries: u64,
        bytes: u64,
        longest_hold: Duration,
    },
    /// The tree changed while it was written, or has no root of the key's directory: nothing
    /// was written.
    Changed,
    /// Cancelled: nothing was written.
    Cancelled,
}

impl Slot {
    /// Whether a scan is cached here that [`Slot::load`] may load at `now`: a file of the
    /// effective user written within the configured load age (default one hour).
    /// Only looks at the file's metadata.
    #[must_use]
    pub fn cached(&self, now: u64) -> bool {
        fs::symlink_metadata(&self.path).is_ok_and(|m| {
            m.is_file()
                && owned(&m)
                && now.saturating_sub(m.modified().map_or(0, secs)) <= self.cache.max_age
        })
    }

    /// Sweeps the cache this slot is in ([`Cache::sweep`] under [`SIZE_CAP`]), keeping this
    /// slot's file.
    pub fn sweep(&self, now: u64) -> Removed {
        self.cache.sweep(now, SIZE_CAP, Some(&self.path))
    }

    /// Removes the cached scan.
    pub fn remove(&self) {
        let _ = fs::remove_file(&self.path);
    }

    /// Reads the cached scan. `Ok(None)` when there is none to use: no file, another user's,
    /// older than the configured load age, or of another directory or other options.
    /// Scans beyond the load age are left for the cache sweep.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, or is damaged or cut short; it is removed then.
    pub fn load(&self, now: u64) -> Result<Option<Hit>, ImportError> {
        let file = match open_own(&self.path) {
            Ok(f) => f,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                ) =>
            {
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        if now.saturating_sub(file.metadata()?.modified().map_or(0, secs)) > self.cache.max_age {
            return Ok(None);
        }
        let loaded = match snapshot::read(
            io::BufReader::with_capacity(1 << 16, file),
            &Limits::default(),
        ) {
            Ok(l) => l,
            Err(e) => {
                self.remove();
                return Err(e);
            }
        };
        let h = &loaded.header;
        if now.saturating_sub(h.written) > self.cache.max_age {
            return Ok(None);
        }
        let same = loaded.tree.path_buf(loaded.root) == self.key.root
            && h.cross_mounts == self.key.cross_mounts
            && h.network == self.key.network;
        if !same {
            return Ok(None);
        }
        let full_rescan = now.saturating_sub(h.full_scan_at()) > FULL_RESCAN_AFTER;
        Ok(Some(Hit {
            loaded,
            full_rescan,
        }))
    }

    /// Writes the tree's scan of the key's directory with `header`, a part at a time:
    /// `locked` runs the function it is given with the tree held still (under a read lock),
    /// and the parts are compressed and written after it returns. A tree that changes between
    /// parts is not written ([`Saved::Changed`]); nor is anything once `cancel` is set. The
    /// file replaces the one cached before only once complete.
    ///
    /// # Errors
    ///
    /// When the directory cannot be created or is another user's, or writing fails.
    pub fn save(
        &self,
        header: &Header,
        mut locked: impl FnMut(&mut dyn FnMut(&Tree)),
        cancel: &AtomicBool,
    ) -> io::Result<Saved> {
        self.cache.ensure_dir()?;
        let mut ended = None;
        let written = crate::write_replacing(&self.path, |f| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                f.set_permissions(fs::Permissions::from_mode(0o600))?;
            }
            let mut out = io::BufWriter::with_capacity(1 << 16, f);
            let mut writer: Option<Writer> = None;
            let mut version = 0;
            let mut longest = Duration::ZERO;
            loop {
                if cancel.load(Relaxed) {
                    ended = Some(Saved::Cancelled);
                    return Err(io::ErrorKind::Interrupted.into());
                }
                let mut step = Step::More;
                locked(&mut |tree| {
                    let t = Instant::now();
                    step = match writer.as_mut() {
                        Some(_) if tree.version() != version => Step::Changed,
                        Some(w) => Step::of(w.step(tree, STEP_ENTRIES)),
                        None => match tree.roots().next() {
                            Some(r) if tree.path_buf(r) == self.key.root => {
                                version = tree.version();
                                let w = writer.insert(Writer::new(tree, r, header));
                                Step::of(w.step(tree, STEP_ENTRIES))
                            }
                            _ => Step::Changed,
                        },
                    };
                    longest = longest.max(t.elapsed());
                });
                match (step, writer.as_mut()) {
                    (Step::Done, _) => break,
                    (Step::More, Some(w)) => w.flush(&mut out)?,
                    _ => {
                        ended = Some(Saved::Changed);
                        return Err(io::ErrorKind::Interrupted.into());
                    }
                }
            }
            let entries = match writer {
                Some(w) => w.finish(&mut out)?,
                None => 0,
            };
            Ok((entries, longest))
        });
        match (written, ended) {
            (Ok((entries, longest_hold)), _) => Ok(Saved::Written {
                entries,
                bytes: fs::metadata(&self.path).map_or(0, |m| m.len()),
                longest_hold,
            }),
            (Err(_), Some(ended)) => Ok(ended),
            (Err(e), None) => Err(e),
        }
    }
}

/// Where a save stands after a step.
#[derive(Clone, Copy)]
enum Step {
    More,
    Done,
    Changed,
}

impl Step {
    fn of(done: bool) -> Step {
        if done { Step::Done } else { Step::More }
    }
}

#[cfg(test)]
mod tests;
