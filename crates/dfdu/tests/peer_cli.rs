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
fn bind_requires_peer_mode() {
    let output = Command::new(env!("CARGO_BIN_EXE_dfdu"))
        .args(["--bind", "abcdef"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--peer"));
}
