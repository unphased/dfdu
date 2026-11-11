#![cfg(unix)]
use std::{fs, net::UdpSocket, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn discovery_loopback_and_fake_tools() {
    if std::env::var_os("DFDU_DISCOVERY_CHILD").is_some() {
        let peer = dfdu_peer::Peer::start(vec!["/secret-root".into()]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let hosts = dfdu_peer::discover().unwrap();
        assert!(
            hosts
                .iter()
                .any(|host| host.ids.iter().any(|id| id == peer.id()))
        );
        let json = serde_json::to_string(&hosts).unwrap();
        assert!(!json.contains("secret-root"));
        assert!(
            dfdu_peer::remote_peers("example", "~/bin/dfdu")
                .unwrap()
                .is_empty()
        );
        let calls = fs::read_to_string(std::env::var("CALLS").unwrap()).unwrap();
        assert!(calls.contains("BatchMode=yes"));
        assert!(calls.contains("peers --json"));
        return;
    }
    let dir = std::path::Path::new("/tmp").join(format!("dfdu-d-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let tailscale = dir.join("tailscale");
    fs::write(&tailscale, "#!/bin/sh\nprintf '%s' '{\"Peer\":{\"local\":{\"Online\":true,\"TailscaleIPs\":[\"127.0.0.1\"]}}}'\n").unwrap();
    let ssh = dir.join("ssh");
    fs::write(
        &ssh,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CALLS\"\nprintf '[]'\n",
    )
    .unwrap();
    for path in [&tailscale, &ssh] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let port = UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "discovery_loopback_and_fake_tools",
            "--nocapture",
        ])
        .env("DFDU_DISCOVERY_CHILD", "1")
        .env("HOME", &dir)
        .env("DFDU_TAILSCALE", tailscale)
        .env("DFDU_SSH", ssh)
        .env("CALLS", dir.join("calls"))
        .env("DFDU_DISCOVERY_PORT", port.to_string())
        .output()
        .unwrap();
    fs::remove_dir_all(dir).unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
