use std::path::Path;
use std::time::{Duration, Instant};

use dfdu_tree::HostPath;

use crate::run_loop::OnSelect;

#[test]
#[cfg(unix)]
fn runs_only_for_changed_local_selections_with_path_as_argument() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("output");
    let command = format!("printf '%s\\n' \"$1\" >> '{}'", output.display());
    let mut hook = OnSelect::new(Some(command.into()));
    let first = HostPath::local(Path::new("/tmp/a path;literal"));
    let second = HostPath::local(Path::new("/tmp/second"));
    hook.update(true, Some(first.clone()));
    hook.update(true, Some(first.clone()));
    hook.update(false, Some(second.clone()));
    hook.update(true, None);
    hook.update(true, Some(second));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = std::fs::read_to_string(&output).unwrap_or_default();
        if text.lines().count() >= 2 {
            let mut lines: Vec<_> = text.lines().collect();
            lines.sort_unstable();
            assert_eq!(lines, ["/tmp/a path;literal", "/tmp/second"]);
            break;
        }
        assert!(Instant::now() < deadline, "hook output: {text:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
