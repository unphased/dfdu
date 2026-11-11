//! Going to a folder in a browser, the tree's say on paths inside the scan, and `Parent` at
//! the scan root.

use dfdu_engine::test_support::meta;
use dfdu_tree::{Kind, NodeFlags};

use super::*;
use crate::act::{self, Ran};
use crate::snapshot::{EntryData, test_entry};
use crate::state::tests::browser;

/// `p` with `/` for each separator: a Windows host joins with `\`, so tests compare paths in
/// one form on every platform.
fn slashed(p: &HostPath) -> String {
    String::from_utf8_lossy(p.as_bytes()).replace('\\', "/")
}

/// A browser of the scan root `/r` with the directory `lib`, a scan of another host.
fn at_root() -> SessionState {
    let lib = EntryData {
        is_dir: true,
        ..test_entry(0, "lib", 100)
    };
    let mut s = browser(vec![lib]);
    s.snap.root_path = s.snap.dir_path.clone();
    s
}

fn reopen(ran: Ran) -> Option<HostPath> {
    match ran {
        Ran::Done(then) => then.reopen,
        Ran::FrontEnd => panic!("left to the front end"),
    }
}

/// The tree's answer for a scan that has every path as a folder.
fn dir(p: &HostPath) -> Reach {
    Reach::Dir(p.clone())
}

#[test]
fn parent_at_the_root_asks_first_and_scans_the_parent_on_a_second_press() {
    let t = Instant::now();
    let mut s = at_root();
    assert_eq!(reopen(act::run(&mut s, Action::Parent, t)), None);
    let status = s.status.clone().unwrap_or_default();
    assert!(status.contains("again to scan /"), "{status}");
    let soon = t + ARMED_FOR / 2;
    assert_eq!(
        reopen(act::run(&mut s, Action::Parent, soon)),
        Some(HostPath::from("/"))
    );

    // Any other action in between disarms it.
    let mut s = at_root();
    let _ = reopen(act::run(&mut s, Action::Parent, t));
    let _ = reopen(act::run(&mut s, Action::Down, t));
    assert_eq!(reopen(act::run(&mut s, Action::Parent, t)), None);
    assert_eq!(s.up_armed.map(|a| a.at), Some(t), "asked again");
}

#[test]
fn a_first_parent_at_the_root_expires_or_is_cleared_by_other_input() {
    let t = Instant::now();
    let mut s = at_root();
    let _ = reopen(act::run(&mut s, Action::Parent, t));
    let late = t + ARMED_FOR + Duration::from_millis(1);
    assert_eq!(reopen(act::run(&mut s, Action::Parent, late)), None);
    assert_eq!(
        s.up_armed.map(|a| a.at),
        Some(late),
        "the late press asks again"
    );

    // A click, or a key an overlay or the filter took.
    let mut s = at_root();
    let _ = reopen(act::run(&mut s, Action::Parent, t));
    s.disarm();
    assert_eq!(s.up_armed, None);
    assert_eq!(s.status, None, "its notice went with it");
    assert_eq!(reopen(act::run(&mut s, Action::Parent, t)), None);

    // A new browser, as after a reopen, starts disarmed.
    assert_eq!(SessionState::new(t).up_armed, None);
}

#[test]
fn the_notice_of_a_first_parent_goes_when_it_expires_or_is_disarmed() {
    let t = Instant::now();
    let mut s = at_root();
    let _ = reopen(act::run(&mut s, Action::Parent, t));
    let notice = s.status.clone();
    assert!(notice.is_some());
    let expiry = s.up_armed.as_ref().map(Armed::expires_at);
    assert_eq!(expiry, Some(t + ARMED_FOR));
    assert!(!s.expire_arm(t + ARMED_FOR - Duration::from_millis(1)));
    assert_eq!(s.status, notice, "still armed");
    assert!(s.expire_arm(t + ARMED_FOR), "the status line changed");
    assert_eq!((s.up_armed.is_none(), s.status.as_deref()), (true, None));
    assert!(!s.expire_arm(t + ARMED_FOR * 2), "nothing left to expire");

    // A second press at the expiry asks again, as after a tick at that time.
    let mut s = at_root();
    let _ = reopen(act::run(&mut s, Action::Parent, t));
    assert_eq!(
        reopen(act::run(&mut s, Action::Parent, t + ARMED_FOR)),
        None
    );

    // Another action takes the notice with the arm.
    let mut s = at_root();
    let _ = reopen(act::run(&mut s, Action::Parent, t));
    let _ = reopen(act::run(&mut s, Action::Down, t));
    assert_eq!(s.status, None);
}

#[test]
fn a_status_that_replaced_the_notice_of_a_first_parent_stays() {
    let t = Instant::now();
    let mut s = at_root();
    let _ = reopen(act::run(&mut s, Action::Parent, t));
    s.status = Some("copied".into());
    assert!(!s.expire_arm(t + ARMED_FOR), "nothing new to show");
    assert_eq!(s.up_armed, None);
    assert_eq!(s.status.as_deref(), Some("copied"));

    let mut s = at_root();
    let _ = reopen(act::run(&mut s, Action::Parent, t));
    s.status = Some("copied".into());
    s.disarm();
    assert_eq!(s.status.as_deref(), Some("copied"));
}

#[test]
fn parent_at_the_top_of_the_filesystem_or_of_a_file_opens_nothing() {
    let t = Instant::now();
    let mut s = at_root();
    s.snap.root_path = HostPath::from("/");
    s.snap.dir_path = HostPath::from("/");
    for _ in 0..2 {
        assert_eq!(reopen(act::run(&mut s, Action::Parent, t)), None);
    }
    assert_eq!(
        s.status.as_deref(),
        Some("already at the top of the filesystem")
    );

    let mut s = at_root();
    s.from_file = Some("from a file, in the front end's words");
    for _ in 0..2 {
        assert_eq!(reopen(act::run(&mut s, Action::Parent, t)), None);
        assert_eq!(s.status.as_deref(), s.from_file);
    }
}

#[test]
fn go_opens_a_directory_of_the_scan_or_returns_one_outside_it() {
    let mut s = at_root();
    s.status = Some("left from before".into());
    match go(&mut s, "lib/x/..", dir) {
        Ok(Gone::Within(Some(m))) => {
            assert_eq!(m.opened.as_ref().map(slashed).as_deref(), Some("/r/lib"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(s.status, None, "the status line is cleared");
    assert_eq!(slashed(&s.target.path), "/r/lib");
    assert!(matches!(go(&mut s, "/r", dir), Ok(Gone::Within(Some(_)))));
    match go(&mut s, "/elsewhere", |_| unreachable!("outside the scan")) {
        Ok(Gone::Outside(p)) => assert_eq!(slashed(&p), "/elsewhere"),
        other => panic!("{other:?}"),
    }
    s.from_file = Some(FROM_FILE);
    assert_eq!(
        go(&mut s, "/elsewhere", dir).err().as_deref(),
        Some(FROM_FILE)
    );
    assert!(go(&mut s, "", dir).is_err());
}

#[test]
fn go_opens_another_scanned_root_even_from_a_file() {
    let mut s = at_root();
    s.snap.roots = vec![unix("/r"), unix("/other")];
    s.snap.root_path = unix("/r");
    s.snap.dir_path = unix("/r");
    s.from_file = Some(FROM_FILE);
    assert!(matches!(
        go(&mut s, "/other/child", dir),
        Ok(Gone::Within(Some(_)))
    ));
    assert_eq!(s.target.path, unix("/other/child"));
    assert_eq!(
        go(&mut s, "/outside", dir).err().as_deref(),
        Some(FROM_FILE)
    );
}

#[test]
fn go_stays_where_it_is_when_the_tree_has_no_folder_there() {
    let mut s = at_root();
    let before = (s.target.path.clone(), s.selected.clone().map(|x| x.name));
    let kept = Kept::NotEntered(HostPath::from("/r/lib"));
    match go(&mut s, "lib/x", |_| Reach::Kept(kept.clone())) {
        Ok(Gone::Stayed(why)) => assert_eq!(why, kept),
        other => panic!("{other:?}"),
    }
    let err = go(&mut s, "lib", |_| Reach::NotDir).err();
    assert_eq!(
        err.map(|e| e.replace('\\', "/")).as_deref(),
        Some("not a folder: /r/lib")
    );
    let after = (s.target.path.clone(), s.selected.clone().map(|x| x.name));
    assert_eq!(after, before, "the user's place is kept");
    assert!(
        kept.text(true).contains("press m on it"),
        "{}",
        kept.text(true)
    );
    assert_eq!(kept.text(false), "/r/lib was not entered by this scan");
}

#[test]
fn go_on_this_machine_looks_up_only_what_is_outside_the_scan() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = dunce::canonicalize(tmp.path()).expect("canonical");
    std::fs::write(root.join("f"), b"").expect("write");
    let mut s = at_root();
    s.outside.local = true;
    let scan = HostPath::local(&root.join("scan"));
    s.snap.root_path = scan.clone();
    s.snap.dir_path = scan;
    // Inside: the tree is asked, not the disk, where `scan` does not even exist.
    assert!(matches!(go(&mut s, "gone", dir), Ok(Gone::Within(Some(_)))));
    // Outside: looked up here.
    let err = go(&mut s, "../f", dir).err().unwrap_or_default();
    assert!(err.starts_with("not a folder"), "{err}");
    let err = go(&mut s, "../nope", dir).err().unwrap_or_default();
    assert!(err.starts_with("no such folder"), "{err}");
    assert!(matches!(go(&mut s, "..", dir), Ok(Gone::Outside(_))));
    // An administrator's scan sees what this user cannot: nothing is looked up as the user.
    s.elevated = true;
    assert!(matches!(go(&mut s, "../nope", dir), Ok(Gone::Outside(_))));
}

#[cfg(unix)]
#[test]
fn go_through_a_link_into_the_scan_opens_the_folder_inside() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = dunce::canonicalize(tmp.path()).expect("canonical");
    let scan = root.join("scan");
    std::fs::create_dir_all(scan.join("d")).expect("mkdir");
    std::os::unix::fs::symlink(&scan, root.join("link")).expect("symlink");
    let mut s = at_root();
    s.outside.local = true;
    s.snap.root_path = HostPath::local(&scan);
    s.snap.dir_path = HostPath::local(&scan);
    let via = root.join("link/d");
    match go(&mut s, &via.to_string_lossy(), dir) {
        Ok(Gone::Within(Some(m))) => assert_eq!(m.opened, Some(HostPath::local(&scan.join("d")))),
        other => panic!("{other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn go_through_a_link_into_another_root() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = dunce::canonicalize(tmp.path()).expect("canonical");
    let other = root.join("other");
    std::fs::create_dir_all(other.join("d")).expect("mkdir");
    std::os::unix::fs::symlink(&other, root.join("link")).expect("symlink");
    let mut s = at_root();
    s.outside.local = true;
    s.snap.roots = vec![s.snap.root_path.clone(), HostPath::local(&other)];
    match go(&mut s, &root.join("link/d").to_string_lossy(), dir) {
        Ok(Gone::Within(Some(m))) => assert_eq!(m.opened, Some(HostPath::local(&other.join("d")))),
        other => panic!("{other:?}"),
    }
}

/// `/r` with the folders `a` (scanned), `p` (being scanned), `m` (a mount not entered) and
/// `n` (a network mount not entered), and the file `f`.
fn tree() -> Tree {
    let mut t = Tree::new();
    t.set_host(Host::Unix);
    let d = meta(Kind::Dir, 0, 1);
    let root = t.new_root(b"/r", &d, true);
    let a = t.add_dir(root, b"a", &d);
    let _ = t.add_dir(root, b"p", &d);
    let m = t.add_dir(root, b"m", &d);
    let n = t.add_dir(root, b"n", &d);
    t.set_flag(n, NodeFlags::NETWORK, true);
    t.add_file(root, b"f", &meta(Kind::File, 1, 2), false, None);
    for (ix, state) in [
        (root, DirState::Complete),
        (a, DirState::Complete),
        (m, DirState::Skipped),
        (n, DirState::Complete),
    ] {
        if let Some(rec) = t.dir_mut(ix) {
            rec.state = state;
        }
    }
    t
}

#[test]
fn the_tree_says_what_is_at_a_path_inside_the_scan() {
    let t = tree();
    let at = |p: &str| reach(&t, &unix(p));
    let kept = |k: fn(HostPath) -> Kept, p: &str| Reach::Kept(k(unix(p)));
    assert_eq!(at("/r"), Reach::Dir(unix("/r")));
    assert_eq!(at("/r/a"), Reach::Dir(unix("/r/a")));
    assert_eq!(at("/r/f"), Reach::NotDir);
    assert_eq!(at("/r/m"), kept(Kept::NotEntered, "/r/m"));
    assert_eq!(at("/r/m/deep/er"), kept(Kept::NotEntered, "/r/m"));
    assert_eq!(at("/r/n/x"), kept(Kept::NotEntered, "/r/n"));
    assert_eq!(at("/r/a/x"), kept(Kept::Missing, "/r/a/x"));
    assert_eq!(at("/r/nope"), kept(Kept::Missing, "/r/nope"));
    assert_eq!(at("/r/p/x"), kept(Kept::NotYet, "/r/p/x"));
    assert_eq!(at("/elsewhere"), kept(Kept::Missing, "/elsewhere"));
}

#[test]
fn reach_chooses_the_most_specific_matching_root() {
    let mut t = Tree::new();
    t.set_host(Host::Unix);
    let d = meta(Kind::Dir, 0, 1);
    let _ = t.new_root(b"/r", &d, true);
    let nested = t.new_root(b"/r/nested", &d, true);
    let _ = t.add_dir(nested, b"child", &d);
    let other = t.new_root(b"/other", &d, true);
    let _ = t.add_dir(other, b"child", &d);
    assert_eq!(
        reach(&t, &unix("/r/nested/child")),
        Reach::Dir(unix("/r/nested/child"))
    );
    assert_eq!(
        reach(&t, &unix("/other/child")),
        Reach::Dir(unix("/other/child"))
    );
}

#[test]
fn a_windows_tree_is_reached_whatever_the_case_typed() {
    let w = |s: &str| HostPath::new(Host::Windows, s.as_bytes().to_vec());
    let mut t = Tree::new();
    let d = meta(Kind::Dir, 0, 1);
    let root = t.new_root(br"C:\Users", &d, true);
    let u = t.add_dir(root, b"Jo", &d);
    let _ = t.add_dir(u, b"Docs", &d);
    t.set_host(Host::Windows);
    assert_eq!(
        reach(&t, &w(r"C:\Users\jo\DOCS")),
        Reach::Dir(w(r"C:\Users\Jo\Docs"))
    );
}

#[test]
fn a_click_above_the_root_of_a_running_scan_it_would_not_keep_asks_first() {
    let t = Instant::now();
    let up = HostPath::from("/");
    let other = HostPath::from("/x");
    // A finished scan, or one the new scan keeps, is replaced at once.
    let mut s = at_root();
    assert_eq!(reopen_above(&mut s, &up, false, t), Some(up.clone()));
    s.snap.busy = true;
    assert_eq!(reopen_above(&mut s, &up, true, t), Some(up.clone()));

    // A running scan it would not keep: the first click says so.
    assert_eq!(reopen_above(&mut s, &up, false, t), None);
    let status = s.status.clone().unwrap_or_default();
    assert!(
        status.contains("not finished") && status.ends_with("scan /"),
        "{status}"
    );
    // A click on another folder asks again, for that one.
    assert_eq!(reopen_above(&mut s, &other, false, t), None);
    assert!(
        s.status
            .as_deref()
            .is_some_and(|st| st.ends_with("scan /x"))
    );
    // The second click on it within the time opens it, and clears the notice.
    let soon = t + ARMED_FOR / 2;
    assert_eq!(
        reopen_above(&mut s, &other, false, soon),
        Some(other.clone())
    );
    assert_eq!(
        (s.reopen_armed.is_none(), s.status.as_deref()),
        (true, None)
    );

    // Too late: it asks again.
    let _ = reopen_above(&mut s, &up, false, t);
    assert_eq!(reopen_above(&mut s, &up, false, t + ARMED_FOR), None);

    // The notice goes when it expires, unless something replaced it.
    assert!(!s.expire_arm(t + ARMED_FOR));
    assert!(s.expire_arm(t + 2 * ARMED_FOR));
    assert_eq!(
        (s.reopen_armed.is_none(), s.status.as_deref()),
        (true, None)
    );
    let _ = reopen_above(&mut s, &up, false, t);
    s.status = Some("other".into());
    assert!(!s.expire_arm(t + ARMED_FOR));
    assert_eq!(s.status.as_deref(), Some("other"));
    // Other input does not disarm it: the click that confirms is input too.
    let _ = reopen_above(&mut s, &up, false, t);
    s.disarm();
    assert_eq!(reopen_above(&mut s, &up, false, t), Some(up));
}
