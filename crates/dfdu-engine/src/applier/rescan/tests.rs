use dfdu_rules::protected::State;
use dfdu_tree::Protection;

use super::*;
use crate::applier::Command;
use crate::applier::test_util::{Listing, Rig, chunk, close, job_class, project};

/// A rescan classifies against the job's rule path continued by the names below its root,
/// with the hits its parent's listing gives it there, siblings included.
#[test]
fn a_rescan_classifies_where_the_job_saw_the_directory() {
    let job = job_class();
    let (t, [r, p, nm]) = project(true);
    let c = rescan_class(&t, nm, Some(r), &job);
    assert_eq!(c.path, b"/home/u/r/p/node_modules");
    let rule = job.rules.index("node.node_modules").unwrap();
    assert!(c.chain.iter().any(|h| h.rule == rule), "{:?}", c.chain);
    assert!(c.inside.is_empty(), "{:?}", c.inside);
    assert_eq!(c.protection, State::Off);
    assert_eq!(rescan_class(&t, p, Some(r), &job).path, b"/home/u/r/p");

    let (mut t, [r, _, nm]) = project(false);
    t.set_protection(nm, Protection::Protected);
    let c = rescan_class(&t, nm, Some(r), &job);
    assert!(
        c.chain.iter().all(|h| h.rule != rule),
        "no package.json beside it"
    );
    assert_eq!(c.protection, State::On);
}

/// Without the job's root above it, the path is the directory's own.
#[test]
fn a_rescan_outside_its_jobs_root_takes_its_own_path() {
    let (t, [r, ..]) = project(true);
    assert_eq!(rescan_class(&t, r, None, &job_class()).path, b"/r");
}

/// What a rescan refuses: a file, a duplicate, kernel or network mount, a directory on another
/// filesystem; and an enter of anything but a mount the scan left out.
#[test]
fn what_is_not_rescanned_says_why() {
    let (mut t, [r, p, _]) = project(true);
    let f = t.children(p).find(|&c| t.dir(c).is_none()).unwrap();
    assert_eq!(rescan_owner(&t, r, false), Ok(0));
    assert_eq!(rescan_owner(&t, f, false), Err(NotRescanned::NotADirectory));
    let cases = [
        (NodeFlags::DUP_MOUNT, NotRescanned::CountedElsewhere),
        (NodeFlags::KERNEL, NotRescanned::Kernel),
        (NodeFlags::NETWORK, NotRescanned::Network),
    ];
    for (flag, why) in cases {
        t.set_flag(p, flag, true);
        assert_eq!(rescan_owner(&t, p, false), Err(why));
        t.set_flag(p, flag, false);
    }
    assert_eq!(rescan_owner(&t, p, true), Err(NotRescanned::NotLeftOut));
    t.dir_mut(p).unwrap().state = DirState::Skipped;
    assert_eq!(
        rescan_owner(&t, p, false),
        Err(NotRescanned::OtherFilesystem)
    );
    assert!(NotRescanned::Network.to_string().contains("--network"));
    assert!(!left_out(&t, p), "not a mount root");
    t.set_flag(p, NodeFlags::MOUNT_ROOT, true);
    assert!(left_out(&t, p));
    assert_eq!(rescan_owner(&t, p, true), Ok(0), "entered");
    t.set_flag(p, NodeFlags::KERNEL, true);
    assert!(!left_out(&t, p), "a kernel filesystem is never scanned");
}

/// The mounts entered this session are entered again by every rescan, each once.
#[test]
fn every_rescan_enters_the_mounts_entered_so_far() {
    let mut r = Rig::scan();
    let root = NodeRef::from_bits(1);
    let other = NodeRef::from_bits(2);
    let mut opts = JobOptions::default();
    r.a.enter_again(root, &mut opts, Some((1, 2)));
    r.a.enter_again(root, &mut opts, Some((1, 2)));
    assert_eq!(opts.enter, [(1, 2)]);
    let mut opts = JobOptions::default();
    r.a.enter_again(root, &mut opts, Some((3, 4)));
    assert_eq!(opts.enter, [(1, 2), (3, 4)]);
    let mut other_opts = JobOptions::default();
    r.a.enter_again(other, &mut other_opts, None);
    assert!(other_opts.enter.is_empty());
}

/// Rescans of job 1's directories, other than the first scan.
fn rescans(r: &Rig) -> Vec<NodeRef> {
    let jobs = r.a.jobs.values();
    jobs.filter(|j| j.kind == JobKind::Rescan && j.swap_pending())
        .map(|j| j.target)
        .collect()
}

/// A rescan of a directory whose parent is still being listed waits for that listing to
/// close, then starts.
#[test]
fn a_rescan_waits_for_its_parents_listing() {
    let mut r = Rig::scan();
    let l = Listing {
        subdirs: &["a"],
        ..Listing::default()
    };
    r.apply(chunk(0, 0, 1, &l));
    let a = r.m.tree.node_ref(r.at("a"));
    r.command(Command::Rescan { node: a });
    r.command(Command::Rescan { node: a });
    assert_eq!(rescans(&r), [], "waiting");
    assert_eq!(r.a.rescans_after_listing, [(a, false)], "waiting, once");
    r.apply(close(0, 1));
    r.a.retry_rescans_after_listing(&mut r.m);
    assert_eq!(rescans(&r), [a]);
    assert_eq!(r.a.rescans_after_listing, []);
}

/// A rescan replaces the running rescans below it, and is refused while one of it or above
/// it has not swapped in.
#[test]
fn a_rescan_supersedes_those_below_it() {
    let mut r = Rig::two_pending();
    let (root, a) = (r.m.tree.node_ref(r.at("")), r.m.tree.node_ref(r.at("a")));
    r.command(Command::Rescan { node: a });
    assert_eq!(rescans(&r), [a]);
    r.command(Command::Rescan { node: root });
    assert_eq!(rescans(&r), [root], "the rescan of a is cancelled");
    r.command(Command::Rescan { node: a });
    assert_eq!(rescans(&r), [root]);
    let last = r.m.status.notices.last().unwrap();
    assert_eq!(
        last,
        "rescan: already rescanning this directory or a parent"
    );
}
