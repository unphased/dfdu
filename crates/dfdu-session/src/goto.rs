//! Go to a folder: a path typed in a prompt, resolved against the directory shown
//! ([`resolve`]), and the completion of its last component from the directories on the scanned
//! host ([`complete`]; asked of the agent on another host's scan, [`tab`]), and the host's
//! mounts while nothing or only the root is typed ([`wants_mounts`]). A path inside the scan
//! is opened there once the tree has a folder at it ([`reach`]); one outside it is a new
//! scan, which the front end opens ([`go`]). Going up from the scan root is one too: the
//! first `Parent` there only says so, a second soon after opens the parent ([`up_at_root`]).
//!
//! Paths are resolved lexically: `.` and `..` are dropped and taken off, links are not
//! followed. `~` is the home directory on a scan of this machine. On a Windows host a path
//! that starts with one separator is on the drive of the directory shown, and names compare
//! without regard to ASCII case, as the file system does.

mod ask;
mod query;

pub use ask::{Asking, Mount, Offer, edited, tab, take_answer, wants_mounts};
pub use query::{Query, complete};

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use dfdu_tree::{DirState, Host, HostPath, Tree};

use crate::actions::{Action, first_key};
use crate::snapshot::{Mark, mark_of};
use crate::state::{Moved, SessionState};

/// How long a first `Parent` at the scan root waits for the second one.
pub const ARMED_FOR: Duration = Duration::from_millis(1500);

/// A first `Parent` at the scan root ([`up_at_root`]): when it was pressed, and the notice it
/// put on the status line, which goes when the arm does unless something else replaced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Armed {
    pub at: Instant,
    pub notice: String,
}

impl Armed {
    /// When a second `Parent` no longer opens the parent.
    #[must_use]
    pub fn expires_at(&self) -> Instant {
        self.at + ARMED_FOR
    }
}

/// Where a typed path goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dest {
    /// The scan root or a directory below it, spelled as the root is: the browser opens it,
    /// with no new scan.
    Within(HostPath),
    /// A path outside the scan: a new scan of it.
    Outside(HostPath),
}

/// What [`go`] did.
#[derive(Clone, Debug)]
pub enum Gone {
    /// The browser opened a directory of the scan; `None` when it was the one shown.
    Within(Option<Moved>),
    /// The path is outside the scan: the front end opens a new scan of it.
    Outside(HostPath),
    /// The path is inside the scan but the tree has no folder to show there: the browser
    /// stays where it is, and the front end says why on the status line.
    Stayed(Kept),
}

/// Why a path inside the scan was not opened ([`Gone::Stayed`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kept {
    /// It is the mount at this path, or below it, which the scan did not enter.
    NotEntered(HostPath),
    /// The scan has not got to it yet.
    NotYet(HostPath),
    /// The scan has nothing there.
    Missing(HostPath),
}

impl Kept {
    /// The status line's text; `enters` when the front end scans a mount with
    /// [`Action::EnterMount`].
    #[must_use]
    pub fn text(&self, enters: bool) -> String {
        match self {
            Kept::NotEntered(p) if enters => format!(
                "{p} was not entered by this scan; press {} on it to scan it",
                first_key(Action::EnterMount)
            ),
            Kept::NotEntered(p) => format!("{p} was not entered by this scan"),
            Kept::NotYet(p) => format!("{p} is not scanned yet"),
            Kept::Missing(p) => format!("{p} is not in this scan"),
        }
    }
}

/// What the tree has at a path inside the scan ([`reach`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// A folder, at this path as the tree spells it.
    Dir(HostPath),
    /// A file.
    NotDir,
    /// No folder it can show.
    Kept(Kept),
}

/// What a completion found: the input completed as far as the names agree, and the names it
/// can still become when there are several.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completion {
    /// The input with its last component completed; `None` when nothing could be added.
    pub text: Option<String>,
    /// The directory names that start with what was typed, sorted, when more than one does.
    pub candidates: Vec<String>,
}

/// What a terminal says of a scan read from a file, which has nothing outside it to open
/// ([`SessionState::from_file`]).
pub const FROM_FILE: &str =
    "this scan was read from a file: run dfdu with a folder to scan another one";

/// The path `input` names, seen from `current` (the directory shown) in a scan of `root`:
/// absolute, or relative to `current`; `~` is `home` on a scan of this machine (`local`).
/// `.` and `..` are resolved lexically, and `..` stops at the root of the filesystem.
///
/// # Errors
///
/// When nothing is typed, or `~` is typed on a scan that is not of this machine.
pub fn resolve(
    input: &str,
    current: &HostPath,
    root: &HostPath,
    home: Option<&Path>,
    local: bool,
) -> Result<Dest, String> {
    let path = absolute(input, current, home, local)?;
    Ok(match below(&path, root) {
        Some(p) => Dest::Within(p),
        None => Dest::Outside(path),
    })
}

/// `path` spelled as `root` is, when it is `root` or below it. On a Windows host the names
/// compare without regard to ASCII case.
fn below(path: &HostPath, root: &HostPath) -> Option<HostPath> {
    if path.host() != Host::Windows {
        return path.starts_with(root).then(|| path.clone());
    }
    if !same(
        path.host(),
        prefix(path).as_bytes(),
        prefix(root).as_bytes(),
    ) {
        return None;
    }
    let mut names = path.components();
    for want in root.components() {
        if !names.next().is_some_and(|n| same(path.host(), n, want)) {
            return None;
        }
    }
    let mut out = root.clone();
    for n in names {
        out.push(n);
    }
    Some(out)
}

/// Whether two names are the same file's on `host`.
fn same(host: Host, a: &[u8], b: &[u8]) -> bool {
    match host {
        Host::Unix => a == b,
        Host::Windows => a.eq_ignore_ascii_case(b),
    }
}

/// The absolute, lexically normal path `input` names from `current`.
fn absolute(
    input: &str,
    current: &HostPath,
    home: Option<&Path>,
    local: bool,
) -> Result<HostPath, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("type the path of a folder".into());
    }
    let host = current.host();
    let sep = |c: char| u8::try_from(c).is_ok_and(|b| host.is_sep(b));
    let typed = HostPath::new(host, input.as_bytes().to_vec());
    let tilde = input == "~" || input.strip_prefix('~').is_some_and(|r| r.starts_with(sep));
    let full = if tilde {
        let Some(home) = home.filter(|_| local) else {
            return Err("~ is known only on a scan of this computer".into());
        };
        let mut p = HostPath::local(home);
        p.push(input[1..].trim_start_matches(sep).as_bytes());
        p
    } else if typed.is_absolute() {
        typed
    } else if host == Host::Windows && input.starts_with(sep) {
        // On the drive of the directory shown.
        let mut p = prefix(current);
        p.push(input.trim_start_matches(sep).as_bytes());
        p
    } else {
        current.join(input.as_bytes())
    };
    Ok(normal(&full))
}

/// The root prefix of `p` (`/`, `C:\`, `\\server\share\`). On a Windows host its separators
/// are `\` and its drive letter upper case, as a tree spells them.
fn prefix(p: &HostPath) -> HostPath {
    let mut root = p.clone();
    while let Some(up) = root.parent() {
        root = up;
    }
    if root.host() != Host::Windows {
        return root;
    }
    let mut b: Vec<u8> = (root.as_bytes().iter())
        .map(|&c| if c == b'/' { b'\\' } else { c })
        .collect();
    // `C:\`, or `\\?\C:\`.
    let drive = if b.starts_with(br"\\?\") || b.starts_with(br"\\.\") {
        4
    } else {
        0
    };
    if b.get(drive + 1) == Some(&b':') {
        b[drive].make_ascii_uppercase();
    }
    HostPath::new(Host::Windows, b)
}

/// `p` (absolute) with its `.` components dropped and each `..` taking off the one before it,
/// joined with the host's separator after its root prefix as [`prefix`] spells it.
fn normal(p: &HostPath) -> HostPath {
    let mut parts: Vec<&[u8]> = Vec::new();
    for c in p.components() {
        match c {
            b"." => {}
            b".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    let mut out = prefix(p);
    for c in parts {
        out.push(c);
    }
    out
}

/// The directories in `dir` on this machine, by name; names that are not UTF-8 cannot be typed
/// and are left out. Links to directories count as directories.
///
/// # Errors
///
/// When the directory cannot be read.
pub fn list_local(dir: &HostPath) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir.to_local())? {
        let e = e?;
        let is_dir = match e.file_type() {
            Ok(t) if t.is_symlink() => std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir()),
            Ok(t) => t.is_dir(),
            Err(_) => false,
        };
        if is_dir && let Ok(name) = e.file_name().into_string() {
            out.push(name);
        }
    }
    Ok(out)
}

/// The text a go-to prompt starts with: the directory shown, ending with a separator so what
/// is typed next goes below it.
pub fn prefill(s: &SessionState) -> String {
    let dir = &s.snap.dir_path;
    let mut text = dir.display().into_owned();
    if !dir.as_bytes().last().is_some_and(|&b| dir.host().is_sep(b)) {
        text.push(char::from(dir.host().sep()));
    }
    text
}

/// What `tree` has at `path`, inside its scan: the folder as the tree spells it, a file, a
/// mount the scan did not enter (at the path or above it), or nothing yet or at all.
pub fn reach(tree: &Tree, path: &HostPath) -> Reach {
    let missing = Reach::Kept(Kept::Missing(path.clone()));
    let mut roots = tree.roots().peekable();
    if roots.peek().is_none() {
        return Reach::Kept(Kept::NotYet(path.clone()));
    }
    let Some((root, root_path)) = roots
        .filter_map(|root| {
            let root_path = tree.host_path(root);
            below(path, &root_path).map(|_| (root, root_path))
        })
        .max_by_key(|(_, root_path)| root_path.components().count())
    else {
        return missing;
    };
    let Some(names) = below(path, &root_path).and_then(|p| {
        p.rel(&root_path)
            .map(|r| r.into_iter().map(<[u8]>::to_vec).collect::<Vec<_>>())
    }) else {
        return missing;
    };
    let host = path.host();
    let not_entered = |ix| {
        let mark = mark_of(tree, ix, &HashSet::new());
        (matches!(mark, Mark::OtherFs | Mark::Network))
            .then(|| Reach::Kept(Kept::NotEntered(tree.host_path(ix))))
    };
    let mut cur = root;
    for name in &names {
        if let Some(r) = not_entered(cur) {
            return r;
        }
        match tree.children(cur).find(|&c| same(host, tree.name(c), name)) {
            Some(c) => cur = c,
            None if tree.dir(cur).is_some_and(|d| d.state == DirState::Pending) => {
                return Reach::Kept(Kept::NotYet(path.clone()));
            }
            None => return missing,
        }
    }
    if tree.dir(cur).is_none() {
        return Reach::NotDir;
    }
    not_entered(cur).unwrap_or_else(|| Reach::Dir(tree.host_path(cur)))
}

/// Goes to the folder `input` names in the browser `s`, with what the scan's tree has at a
/// path inside it from `reach` ([`reach`] under the engine's read lock): opens it when the
/// tree has it as a folder, stays where it is when it does not, or returns a path outside the
/// scan for a new one. The status line is cleared. On a scan of this machine run as this user,
/// a path outside the scan must be a directory, and one that leads into the scan through links
/// is taken as the one inside.
///
/// # Errors
///
/// What is wrong with the path, to show in the prompt: nothing typed, no such folder, not a
/// folder, or a path outside a scan read from a file.
pub fn go(
    s: &mut SessionState,
    input: &str,
    reach: impl FnOnce(&HostPath) -> Reach,
) -> Result<Gone, String> {
    s.status = None;
    let root = s.snap.root_path.clone();
    let local = s.outside.local;
    let mut dest = resolve(input, &s.snap.dir_path, &root, s.home.as_deref(), local)?;
    let inside_roots = |path: &HostPath| {
        s.snap
            .roots
            .iter()
            .chain(std::iter::once(&root))
            .filter_map(|root| below(path, root).map(|p| (root.components().count(), p)))
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, p)| p)
    };
    if let Dest::Outside(p) = &dest
        && let Some(inside) = inside_roots(p)
    {
        dest = Dest::Within(inside);
    }
    // The tree is the truth inside the scan, and an administrator's scan sees what this user
    // cannot: only a path outside a scan of this user's is looked up here.
    if let Dest::Outside(p) = &dest
        && local
        && !s.elevated
    {
        check_local(p)?;
        if let Ok(real) = dunce::canonicalize(p.to_local())
            && let Some(inside) = inside_roots(&HostPath::local(&real))
        {
            dest = Dest::Within(inside);
        }
    }
    match dest {
        Dest::Within(p) => match reach(&p) {
            Reach::Dir(p) => Ok(Gone::Within(s.open_dir(p))),
            Reach::NotDir => Err(format!("not a folder: {p}")),
            Reach::Kept(why) => Ok(Gone::Stayed(why)),
        },
        Dest::Outside(p) => match s.from_file {
            Some(says) => Err(says.into()),
            None => Ok(Gone::Outside(p)),
        },
    }
}

/// Whether `p` on this machine is a directory, and if not, why not.
fn check_local(p: &HostPath) -> Result<(), String> {
    match std::fs::metadata(p.to_local()) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(format!("not a folder: {p}")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(format!("no such folder: {p}")),
        Err(e) => Err(format!("{p}: {e}")),
    }
}

/// `Parent` at the scan root at `now`: the first press says what a second one does, and a
/// second within [`ARMED_FOR`] (`armed`, when the first was) returns the parent to scan.
/// Nothing above the root of the filesystem, and nothing outside a scan read from a file.
pub fn up_at_root(s: &mut SessionState, armed: Option<Instant>, now: Instant) -> Option<HostPath> {
    if let Some(says) = s.from_file {
        s.status = Some(says.into());
        return None;
    }
    let Some(parent) = s.snap.root_path.parent() else {
        s.status = Some("already at the top of the filesystem".into());
        return None;
    };
    if armed.is_some_and(|at| now.saturating_duration_since(at) < ARMED_FOR) {
        return Some(parent);
    }
    let notice = format!(
        "top of the scan: press {} again to scan {parent}",
        first_key(Action::Parent)
    );
    s.status = Some(notice.clone());
    s.up_armed = Some(Armed { at: now, notice });
    None
}

/// A click at `now` that asks for a scan of `path`, a folder above the scan root (a
/// breadcrumb): the folder to scan now, or `None` when the click only said what a second one
/// does. A scan that is still running, which the scan of `path` would not keep (`keeps`,
/// [`dfdu_engine::Engine::keeps`]), is stopped only by a second click on the same folder
/// within [`ARMED_FOR`]: one click would throw away what it read.
pub fn reopen_above(
    s: &mut SessionState,
    path: &HostPath,
    keeps: bool,
    now: Instant,
) -> Option<HostPath> {
    let armed = s.reopen_armed.take();
    if let Some((_, a)) = &armed {
        s.drop_notice(a);
    }
    let again = armed.is_some_and(|(to, a)| to == *path && now < a.expires_at());
    if keeps || !s.snap.busy || again {
        return Some(path.clone());
    }
    let notice = format!("this scan has not finished: click again to stop it and scan {path}");
    s.status = Some(notice.clone());
    s.reopen_armed = Some((path.clone(), Armed { at: now, notice }));
    None
}

/// The scan root is shown, its scan is over and it holds nothing: the list says so, and how
/// to go up or elsewhere.
#[must_use]
pub fn empty_root(s: &SessionState) -> bool {
    let snap = &s.snap;
    snap.dir_node.is_some()
        && snap.dir_path == snap.root_path
        && snap.entries.is_empty()
        && !snap.busy
        && !matches!(
            snap.dir_mark,
            Some(
                Mark::Scanning | Mark::Rescanning | Mark::Stalled | Mark::Error | Mark::Incomplete
            )
        )
}

/// The name in the scan root of the entry on the way to `from`, the root of the scan this one
/// was opened from going up ([`SessionState::came_from`]): `None` when `from` is not below
/// `root`.
#[must_use]
pub fn came_from_name(from: &HostPath, root: &HostPath) -> Option<Vec<u8>> {
    let inside = below(from, root)?;
    let rel = inside.rel(root)?;
    rel.into_iter().next().map(<[u8]>::to_vec)
}

#[cfg(test)]
mod tests;
