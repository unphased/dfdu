use dfdu_tree::NodeFlags;

use super::*;
use crate::applier::test_util::{Listing, Rig, chunk, close};

/// Job 1 with the root listed and closed: the files `f` and `g`.
fn rig() -> Rig {
    let mut r = Rig::scan();
    let l = Listing {
        files: &[("f", 10), ("g", 20)],
        ..Listing::default()
    };
    r.apply(chunk(0, 0, 1, &l));
    r.apply(close(0, 1));
    r.settle();
    r
}

/// One command of each kind the refusals tell apart, on `node`.
fn commands(node: NodeRef) -> [Command; 7] {
    let native = |dry_run| Command::RunNative {
        id: 1,
        node,
        variant: None,
        dry_run,
    };
    [
        Command::Scan {
            path: "/s".into(),
            opts: JobOptions::default(),
        },
        Command::Rescan { node },
        Command::Delete {
            items: vec![node],
            mode: Mode::Permanent,
        },
        native(false),
        native(true),
        Command::NativeSizes,
        Command::Estimate {
            id: 1,
            items: vec![node],
        },
    ]
}

/// Which of [`commands`] `r` refuses.
fn refused(r: &Rig) -> Vec<bool> {
    let node = r.m.tree.node_ref(r.at(""));
    (commands(node).iter())
        .map(|c| r.a.refusal(&r.m, c).is_some())
        .collect()
}

/// An imported tree refuses what would scan, delete or run a command, dry runs included;
/// read-only mode refuses deletes and real native runs; a lost agent refuses scans. Nothing
/// refuses an estimate.
#[test]
fn refusals_follow_the_mode() {
    let mut r = rig();
    assert_eq!(refused(&r), [false; 7]);
    r.a.settings.imported = true;
    assert_eq!(refused(&r), [true, true, true, true, true, true, false]);
    r.a.settings.imported = false;
    r.a.settings.read_only = true;
    assert_eq!(refused(&r), [false, false, true, true, false, false, false]);
    r.a.settings.read_only = false;
    r.m.status.disconnected = Some("eof".into());
    assert_eq!(refused(&r), [true, true, false, false, false, false, false]);
}

/// Multi-root scans refuse destructive native commands before queuing a deleter request,
/// but still queue a native dry run.
#[test]
fn multi_root_native_runs_are_refused_but_dry_runs_are_queued() {
    let mut r = rig();
    let (rules, errors) = dfdu_rules::load_from(
        &dfdu_rules::Env::current(),
        &[(
            dfdu_rules::Source::Builtin,
            r#"schema_version = 1
[[rule]]
id = "test.native"
category = "dev.build"
name = "Test native"
safety = "safe"
action = "native"
explanation = "Test."
lost = "Nothing."
[rule.match]
kind = "dir"
names = ["r"]
[rule.native]
command = ["remove", "{path}"]
dry_run = ["preview", "{path}"]
"#
            .into(),
        )],
    );
    assert!(errors.is_empty(), "{errors:?}");
    let rule = rules.index("test.native").unwrap();
    r.m.rules = Some(std::sync::Arc::new(rules));
    let root = r.at("");
    r.m.tree.set_class(
        root,
        Some(dfdu_tree::Class {
            rule,
            safety: dfdu_tree::Safety::Safe,
            degraded: None,
            flags: dfdu_tree::ClassFlags::NONE,
        }),
    );
    let node = r.m.tree.node_ref(root);
    r.command(Command::Scan {
        path: "/s".into(),
        opts: JobOptions::default(),
    });
    assert_eq!(r.m.tree.roots().count(), 2);
    r.command(Command::RunNative {
        id: 1,
        node,
        variant: None,
        dry_run: false,
    });
    assert!(r.a.take_deleter_controls().is_empty());
    assert!(r.a.take_plans().is_empty());
    assert!(r.m.status.native.is_none());
    assert_eq!(
        r.m.status.notices.last().map(String::as_str),
        Some("native command: unavailable for scans with multiple roots")
    );

    r.command(Command::RunNative {
        id: 2,
        node,
        variant: None,
        dry_run: true,
    });
    let requests = r.a.take_deleter_controls();
    assert!(matches!(requests.as_slice(), [DeleterReq::Native(req)]
        if req.dry_run && req.id == 2 && req.command[0] == "preview"));
    assert!(r.m.status.native.as_ref().is_some_and(|n| n.dry_run));
}

/// A refused command is said why, and still counted and recorded as applied.
#[test]
fn a_refused_command_is_counted_and_said_why() {
    let mut r = rig();
    r.a.settings.read_only = true;
    let (applied, recorded) = (r.m.status.commands_applied, r.m.recent.total());
    let f = r.m.tree.node_ref(r.at("f"));
    r.command(Command::Delete {
        items: vec![f],
        mode: Mode::Permanent,
    });
    assert_eq!(
        r.m.status.notices.last().map(String::as_str),
        Some("delete: read-only mode")
    );
    assert_eq!(r.m.status.commands_applied, applied + 1);
    assert_eq!(r.m.recent.total(), recorded + 1);
    assert!(r.a.take_plans().is_empty());
}

/// Committing staged items is a request to the deleter; an undo of a plan with nothing to
/// put back is not.
#[test]
fn commits_go_to_the_deleter_and_undos_only_of_undoable_plans() {
    let mut r = rig();
    r.command(Command::Undo { plan: 9 });
    assert!(r.a.take_deleter_controls().is_empty());
    assert_eq!(r.m.status.undoing, None);
    r.command(Command::CommitStaged);
    let sent = r.a.take_deleter_controls();
    assert!(matches!(sent[..], [DeleterReq::Commit]), "{sent:?}");
}

/// Aborting drops the queued plans and unmarks their items; the running plan's items stay
/// marked, for the runtime stops it after its current item.
#[test]
fn an_abort_drops_the_queued_plans() {
    let mut r = rig();
    let (f, g) = (r.at("f"), r.at("g"));
    for ix in [f, g] {
        let items = vec![r.m.tree.node_ref(ix)];
        r.command(Command::Delete {
            items,
            mode: Mode::Permanent,
        });
    }
    let deleting = |r: &Rig, ix| r.m.tree.flags(ix).has(NodeFlags::DELETING);
    assert!(deleting(&r, f) && deleting(&r, g));
    r.command(Command::AbortDelete);
    assert!(deleting(&r, f), "the running plan");
    assert!(!deleting(&r, g), "the queued plan");
    assert_eq!(r.a.take_plans().len(), 1, "only the running plan was sent");
}
