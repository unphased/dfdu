#![cfg(unix)]

use std::process::Command;

#[test]
fn peer_commands_and_flags_are_available() {
    let bin = env!("CARGO_BIN_EXE_dfdu");
    for args in [
        vec!["--peer", "--bind", "abcdef", "--help"],
        vec!["peers", "--help"],
        vec!["peer-attach", "--help"],
    ] {
        let output = Command::new(bin).args(&args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn peers_json_is_an_empty_array_in_a_fresh_home() {
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_dfdu"))
        .args(["peers", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!([])
    );
}

#[test]
fn peer_flags_are_harmless_without_a_terminal() {
    let tree = tempfile::tempdir().unwrap();
    for flags in [vec![], vec!["--peer"], vec!["--no-peer"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_dfdu"))
            .args(&flags)
            .arg(tree.path())
            .env("DFDU_STATE_DIR", tree.path().join("state"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{flags:?}: {:?}", output.stderr);
        assert!(!tree.path().join("state/peers").exists());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_dfdu"))
        .args(["scan", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn bind_implies_peer_mode_and_requires_a_terminal() {
    let output = Command::new(env!("CARGO_BIN_EXE_dfdu"))
        .args(["--bind", "abcdef", "/tmp"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--bind requires the interactive browser on a terminal")
    );
}
