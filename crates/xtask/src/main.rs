//! `cargo xtask`: the repository's common tasks in one place. Each task runs the same commands
//! as the CI job of the same name (`.github/workflows/ci.yml`), so a local run reproduces CI.
//! Pinned values (the nightly for fuzzing, the coverage floor, the MSRV) are read from ci.yml
//! and Cargo.toml rather than repeated here.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};

mod cmd;
mod demo;
mod release;
mod website;
mod wine;

use cmd::{
    Step, cargo, docker_run, has, has_cargo, need, need_cargo, output, quote, root, run, steps,
};
use demo::{DemoArgs, demo};
use release::{Release, release};
use website::{Website, website};
use wine::{WineArgs, wine};

#[derive(Parser)]
#[command(about, bin_name = "cargo xtask")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Format the workspace (`--check` only reports).
    Fmt {
        #[arg(long)]
        check: bool,
    },
    /// Clippy on every target and feature, warnings as errors.
    Clippy {
        /// Apply clippy's suggestions.
        #[arg(long)]
        fix: bool,
    },
    /// The fast checks: fmt, clippy, unused dependencies, cargo-deny, zizmor, shellcheck and the
    /// fuzz build.
    Lint,
    /// Find unused dependencies.
    Machete,
    /// cargo-deny: bans, licenses and sources (`--advisories` for `RustSec` advisories).
    Deny {
        #[arg(long)]
        advisories: bool,
    },
    /// Tests with nextest (cargo test when it is not installed), then doc tests.
    Test(TestArgs),
    /// The deterministic simulation (crates/dfdu-sim).
    Sim(SimArgs),
    /// The loom model tests, in their own target directory so the main build is kept.
    Loom,
    /// Kill dfdu at random points while it writes and deletes, and check recovery.
    Crash {
        #[arg(long, default_value_t = 40)]
        rounds: u32,
    },
    /// Tests with line coverage, failing below ci.yml's floor.
    Coverage {
        /// Also write an HTML report to target/llvm-cov/html.
        #[arg(long)]
        html: bool,
        /// Open the HTML report.
        #[arg(long)]
        open: bool,
        /// Do not fail below the floor (CI reports first, then checks).
        #[arg(long)]
        no_fail: bool,
    },
    /// Check the workspace builds on the MSRV from Cargo.toml.
    Msrv,
    /// Fuzz one target, or every target in turn. Uses Docker when cargo-fuzz is not installed.
    Fuzz(FuzzArgs),
    /// Run a command in a Linux container (default: the workspace tests).
    Linux(LinuxArgs),
    /// Type-check dfdu for Windows (x86_64-pc-windows-msvc) without the bundled SQLite build,
    /// or with `--test` run the tests for Windows under Wine.
    Windows(WindowsArgs),
    /// Run the local part of CI and print a summary.
    Ci(CiArgs),
    /// Time dfdu against other disk usage tools (scripts/bench.sh; arguments are passed on).
    Bench {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        args: Vec<String>,
    },
    /// Rewrite the man pages and completions in assets/.
    Assets,
    /// Record the demo GIFs (demo/tapes) with VHS in Docker, against made-up files; with --gui,
    /// take the desktop app's screenshots (demo/gui/shots).
    Demo(DemoArgs),
    /// Release tasks.
    #[command(subcommand)]
    Release(Release),
    /// The dfdu.app site in website/ (needs Bun).
    #[command(subcommand)]
    Website(Website),
}

#[derive(Args)]
struct TestArgs {
    /// Only these packages.
    #[arg(short, long = "package")]
    packages: Vec<String>,
    /// CI's profiles: optimised with debug assertions, retries for flaky tests.
    #[arg(long)]
    ci: bool,
    /// Skip the doc tests.
    #[arg(long)]
    no_doc: bool,
    /// Also the tests that wait on the FSEvents daemon ([`DAEMON_TESTS`]), one at a time after
    /// the rest (macOS). They are ignored otherwise: a busy machine makes them fail. `--ci`
    /// runs them.
    #[arg(long)]
    daemon: bool,
    /// Passed to the test runner.
    #[arg(last = true)]
    args: Vec<String>,
}

#[derive(Args)]
struct SimArgs {
    /// Seeds per suite (CI runs 5000).
    #[arg(long)]
    seeds: Option<u64>,
    /// Rerun one seed.
    #[arg(long)]
    seed: Option<u64>,
    /// Trace one path through the run (with --seed); shows the output.
    #[arg(long)]
    trace: Option<String>,
    /// Passed to the test binary.
    #[arg(last = true)]
    args: Vec<String>,
}

#[derive(Args)]
struct FuzzArgs {
    /// The target (`--list` for the names); every target when left out.
    target: Option<String>,
    /// Seconds per target.
    #[arg(long, default_value_t = 60)]
    seconds: u64,
    /// List the targets.
    #[arg(long)]
    list: bool,
    /// Run in the dfdu-fuzz Docker image even when cargo-fuzz is installed.
    #[arg(long)]
    docker: bool,
}

#[derive(Args)]
struct LinuxArgs {
    /// Needed by tests that mount (tmpfs, FUSE, btrfs).
    #[arg(long)]
    privileged: bool,
    #[arg(long, default_value = "rust:1.99")]
    image: String,
    /// The command (default: cargo test --workspace --no-fail-fast).
    #[arg(last = true)]
    cmd: Vec<String>,
}

#[derive(Args)]
struct WindowsArgs {
    /// Build the tests for Windows and run them under Wine in a Linux container
    /// (crates/xtask/src/wine.rs).
    #[arg(long)]
    test: bool,
    #[command(flatten)]
    wine: WineArgs,
}

#[derive(Args)]
struct CiArgs {
    /// Only fmt, clippy and the tests.
    #[arg(long, conflicts_with = "full")]
    quick: bool,
    /// Also the MSRV, coverage, CI's 5000 simulation seeds and a minute of fuzzing per target.
    #[arg(long)]
    full: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    std::env::set_current_dir(root())?;
    match cli.cmd {
        Cmd::Fmt { check } => fmt(check),
        Cmd::Clippy { fix } => clippy(fix),
        Cmd::Lint => lint(),
        Cmd::Deny { advisories } => deny(advisories),
        Cmd::Machete => machete(),
        Cmd::Test(a) => test(&a),
        Cmd::Sim(a) => sim(&a),
        Cmd::Loom => loom(),
        Cmd::Crash { rounds } => run(cargo(["test", "--locked", "-p", "dfdu", "--test", "crash"])
            .env("DFDU_CRASH_ROUNDS", rounds.to_string())),
        Cmd::Coverage {
            html,
            open,
            no_fail,
        } => coverage(html, open, no_fail),
        Cmd::Msrv => msrv(),
        Cmd::Fuzz(a) => fuzz(&a),
        Cmd::Linux(a) => linux(&a),
        Cmd::Windows(a) if a.test => wine(&a.wine),
        Cmd::Windows(_) => windows(),
        Cmd::Ci(a) => ci(&a),
        Cmd::Bench { args } => run(Command::new("scripts/bench.sh").args(args)),
        Cmd::Assets => assets(),
        Cmd::Demo(a) => demo(&a),
        Cmd::Release(r) => release(r),
        Cmd::Website(w) => website(w),
    }
}

fn fmt(check: bool) -> anyhow::Result<()> {
    let mut c = cargo(["fmt", "--all"]);
    if check {
        c.arg("--check");
    }
    run(&mut c)
}

fn clippy(fix: bool) -> anyhow::Result<()> {
    let mut c = cargo(["clippy", "--workspace", "--all-targets", "--all-features"]);
    if fix {
        c.args(["--fix", "--allow-dirty", "--allow-staged"]);
    } else {
        c.arg("--locked");
    }
    run(c.args(["--", "-D", "warnings"]))
}

fn lint() -> anyhow::Result<()> {
    steps(vec![
        ("fmt", Box::new(|| fmt(true))),
        ("clippy", Box::new(|| clippy(false))),
        ("machete", Box::new(machete)),
        ("deny", Box::new(|| deny(false))),
        ("zizmor", Box::new(zizmor)),
        ("shellcheck", Box::new(shellcheck)),
        ("fuzz-build", Box::new(fuzz_build)),
    ])
}

/// The fuzz crate is not a workspace member, and only the nightly fuzz run builds it. No
/// `--locked`, as in CI: dependency bumps refresh only the root `Cargo.lock`, and the weekly lock
/// file maintenance refreshes `fuzz/Cargo.lock`.
fn fuzz_build() -> anyhow::Result<()> {
    run(&mut cargo([
        "check",
        "--manifest-path",
        "fuzz/Cargo.toml",
        "--bins",
    ]))
}

/// The script `--remote` runs on hosts without dfdu.
fn shellcheck() -> anyhow::Result<()> {
    need("shellcheck")?;
    run(Command::new("shellcheck").args(["--shell=sh", "crates/dfdu-session/src/remote/boot.sh"]))
}

fn deny(advisories: bool) -> anyhow::Result<()> {
    need_cargo("deny")?;
    let checks: &[&str] = if advisories {
        &["advisories"]
    } else {
        &["bans", "licenses", "sources"]
    };
    run(cargo(["deny", "check"]).args(checks))
}

fn machete() -> anyhow::Result<()> {
    need_cargo("machete")?;
    run(&mut cargo(["machete"]))
}

fn zizmor() -> anyhow::Result<()> {
    need("zizmor")?;
    run(Command::new("zizmor").args([".github/workflows"]))
}

/// The ignored tests that wait on the FSEvents daemon, which falls behind while the machine
/// is busy (several builds at once) and holds their changes past their patience. `--daemon`
/// and CI run them alone, after the rest.
const DAEMON_TESTS: [&str; 3] = [
    "macos::fsevents::tests::changes_after_an_id_are_reported",
    "macos::fsevents::tests::a_change_just_before_the_call_is_reported",
    "journal::tests::changes_come_back_relative_to_the_root",
];

fn test(a: &TestArgs) -> anyhow::Result<()> {
    let packages = a.packages.iter().flat_map(|p| ["-p", p]);
    let scope: Vec<&str> = if a.packages.is_empty() {
        vec!["--workspace"]
    } else {
        packages.collect()
    };
    let nextest = has_cargo("nextest");
    let mut c = if nextest {
        let mut c = cargo(["nextest", "run"]);
        c.args(&scope);
        if a.ci {
            c.args(["--cargo-profile", "ci", "--profile", "ci", "--locked"]);
        }
        c
    } else {
        eprintln!("cargo-nextest not installed; using cargo test");
        let mut c = cargo(["test"]);
        c.args(&scope);
        if a.ci {
            c.args(["--profile", "ci", "--locked"]);
        }
        c
    };
    if !a.args.is_empty() {
        c.arg("--").args(&a.args);
    }
    run(&mut c)?;
    if (a.ci || a.daemon) && cfg!(target_os = "macos") {
        let mut c = if nextest {
            let mut c = cargo([
                "nextest",
                "run",
                "--run-ignored",
                "only",
                "--no-tests",
                "pass",
            ]);
            c.args(&scope);
            if a.ci {
                c.args(["--cargo-profile", "ci", "--profile", "ci", "--locked"]);
            }
            c.args(["--test-threads", "1", "-E"]);
            let filter: Vec<String> = DAEMON_TESTS.iter().map(|t| format!("test(={t})")).collect();
            c.arg(filter.join(" | "));
            c
        } else {
            let mut c = cargo(["test"]);
            c.args(&scope);
            if a.ci {
                c.args(["--profile", "ci", "--locked"]);
            }
            c.args(["--", "--ignored", "--exact", "--test-threads", "1"]);
            c.args(DAEMON_TESTS);
            c
        };
        run(&mut c)?;
    }
    // nextest does not run doc tests; cargo test already did.
    if nextest && !a.no_doc {
        let mut c = cargo(["test", "--doc"]);
        c.args(&scope);
        if a.ci {
            c.args(["--profile", "ci", "--locked"]);
        }
        run(&mut c)?;
    }
    Ok(())
}

fn sim(a: &SimArgs) -> anyhow::Result<()> {
    let mut c = cargo(["test", "--profile", "ci", "--locked", "-p", "dfdu-sim"]);
    if let Some(n) = a.seeds {
        c.env("DFDU_SIM_SEEDS", n.to_string());
    }
    if let Some(s) = a.seed {
        c.env("DFDU_SIM_SEED", s.to_string());
    }
    let mut args = a.args.clone();
    if let Some(t) = &a.trace {
        if a.seed.is_none() {
            bail!("--trace needs --seed");
        }
        c.env("DFDU_SIM_TRACE", t);
        args.push("--nocapture".into());
    }
    if !args.is_empty() {
        c.arg("--").args(args);
    }
    run(&mut c)
}

fn loom() -> anyhow::Result<()> {
    run(cargo([
        "test",
        "--profile",
        "ci",
        "--locked",
        "-p",
        "dfdu-scan",
        "-p",
        "dfdu-engine",
        "--test",
        "loom",
    ])
    .env("RUSTFLAGS", "--cfg loom")
    .env("CARGO_TARGET_DIR", "target/loom"))
}

fn coverage(html: bool, open: bool, no_fail: bool) -> anyhow::Result<()> {
    need_cargo("llvm-cov")?;
    need_cargo("nextest")?;
    let min = ci_value("MIN_LINES")?;
    // `--no-report` does not clean, and the report reads every test binary in the target
    // directory: binaries from older builds (or the CI cache) map stale functions onto the
    // current sources and drag the numbers down.
    run(&mut cargo(["llvm-cov", "clean", "--workspace"]))?;
    run(&mut cargo([
        "llvm-cov",
        "nextest",
        "--workspace",
        "--exclude",
        "xtask",
        "--all-features",
        "--locked",
        "--no-report",
        "--profile",
        "ci",
    ]))?;
    if html || open {
        let mut c = cargo(["llvm-cov", "report", "--html"]);
        if open {
            c.arg("--open");
        }
        run(&mut c)?;
    }
    if no_fail {
        return Ok(());
    }
    run(&mut cargo([
        "llvm-cov",
        "report",
        "--summary-only",
        "--fail-under-lines",
        &min,
    ]))
}

fn msrv() -> anyhow::Result<()> {
    let manifest = fs::read_to_string("Cargo.toml")?;
    let msrv = toml_value(&manifest, "rust-version").context("no rust-version in Cargo.toml")?;
    run(&mut cargo([
        &format!("+{msrv}"),
        "check",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--locked",
    ]))
}

fn fuzz(a: &FuzzArgs) -> anyhow::Result<()> {
    let docker = a.docker || !has_cargo("fuzz");
    // The dfdu-fuzz image has one nightly; on the host, CI's pinned one.
    let nightly = if docker {
        "nightly".to_owned()
    } else {
        ci_value("NIGHTLY")?
    };
    let script = match (&a.target, a.list) {
        (_, true) => format!("cd fuzz && cargo +{nightly} fuzz list"),
        (None, false) => format!(".github/scripts/fuzz.sh {nightly} {}", a.seconds),
        (Some(t), false) => {
            let dirs: Vec<String> = ["corpus", "seeds", "regressions"]
                .iter()
                .map(|d| format!("{d}/{t}"))
                .filter(|d| d.starts_with("corpus") || Path::new("fuzz").join(d).is_dir())
                .collect();
            format!(
                "cd fuzz && mkdir -p corpus/{t} && cargo +{nightly} fuzz run {} {} -- \
                 -max_total_time={} -timeout=10 -rss_limit_mb=2048",
                quote(t),
                dirs.iter().map(|d| quote(d)).collect::<Vec<_>>().join(" "),
                a.seconds,
            )
        }
    };
    if docker {
        eprintln!("running in the dfdu-fuzz Docker image");
        need("docker")?;
        run(docker_run("dfdu-fuzz", "dfdu-fuzz-target", false).args(["bash", "-c", &script]))
    } else {
        run(Command::new("bash").args(["-c", &script]))
    }
}

fn linux(a: &LinuxArgs) -> anyhow::Result<()> {
    need("docker")?;
    let cmd = if a.cmd.is_empty() {
        "cargo test --workspace --no-fail-fast".to_owned()
    } else {
        a.cmd.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
    };
    // The image has no git; the remote classification test runs `git init`.
    let script = format!("apt-get -qq update && apt-get -qq install -y git >/dev/null && {cmd}");
    run(docker_run(&a.image, "dfdu-target", a.privileged).args(["bash", "-c", &script]))
}

fn windows() -> anyhow::Result<()> {
    const TARGET: &str = "x86_64-pc-windows-msvc";
    let installed = output(Command::new("rustup").args(["target", "list", "--installed"]))?;
    if !installed.lines().any(|l| l == TARGET) {
        bail!("the {TARGET} target is not installed: rustup target add {TARGET}");
    }
    // `check` never links, so pointing libsqlite3-sys at an empty directory skips its C build.
    run(cargo(["check", "--target", TARGET, "-p", "dfdu"])
        .env("LIBSQLITE3_SYS_USE_PKG_CONFIG", "1")
        .env("SQLITE3_LIB_DIR", std::env::temp_dir()))
}

fn ci(a: &CiArgs) -> anyhow::Result<()> {
    let test_args = TestArgs {
        packages: vec![],
        ci: true,
        no_doc: false,
        daemon: false,
        args: vec![],
    };
    let mut list: Vec<Step> = vec![
        ("fmt", Box::new(|| fmt(true))),
        ("clippy", Box::new(|| clippy(false))),
        ("test", Box::new(move || test(&test_args))),
    ];
    if !a.quick {
        let seeds = a.full.then_some(5000);
        list.extend::<Vec<Step>>(vec![
            (
                "simulation",
                Box::new(move || {
                    sim(&SimArgs {
                        seeds,
                        seed: None,
                        trace: None,
                        args: vec![],
                    })
                }),
            ),
            ("loom", Box::new(loom)),
            ("deny", Box::new(|| deny(false))),
            ("machete", Box::new(machete)),
            ("zizmor", Box::new(zizmor)),
        ]);
        // The site needs Bun and Node, which a Rust-only checkout may not have; CI checks it in
        // website.yml either way.
        if has("bun") && has("node") {
            list.push(("website", Box::new(|| website(Website::Ci))));
        } else {
            eprintln!("skipping the website check: Bun or Node is not installed");
        }
    }
    if a.full {
        list.extend::<Vec<Step>>(vec![
            ("msrv", Box::new(msrv)),
            ("coverage", Box::new(|| coverage(false, false, false))),
            (
                "fuzz",
                Box::new(|| {
                    fuzz(&FuzzArgs {
                        target: None,
                        seconds: 60,
                        list: false,
                        docker: false,
                    })
                }),
            ),
        ]);
    }
    steps(list)
}

fn assets() -> anyhow::Result<()> {
    run(cargo(["test", "-p", "dfdu", "--test", "assets"]).env("DFDU_UPDATE_ASSETS", "1"))
}

/// A `KEY: value` from the `env:` blocks of ci.yml or, for the fuzzing toolchain, fuzz.yml.
fn ci_value(key: &str) -> anyhow::Result<String> {
    for file in ["ci.yml", "fuzz.yml"] {
        let yml = fs::read_to_string(format!(".github/workflows/{file}"))?;
        let value = yml
            .lines()
            .find_map(|l| l.trim().strip_prefix(key)?.strip_prefix(':'));
        if let Some(v) = value {
            return Ok(v.trim().trim_matches('"').to_owned());
        }
    }
    bail!("no {key} in ci.yml or fuzz.yml")
}

/// The first `key = "value"` in a TOML file.
fn toml_value(toml: &str, key: &str) -> Option<String> {
    toml.lines().find_map(|l| {
        let v = l.strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
        Some(v.strip_prefix('"')?.strip_suffix('"')?.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_first_toml_value() {
        let toml = "[workspace.package]\nversion = \"0.1.0\"\nrust-version = \"1.95\"\n";
        assert_eq!(toml_value(toml, "version").as_deref(), Some("0.1.0"));
        assert_eq!(toml_value(toml, "rust-version").as_deref(), Some("1.95"));
        assert_eq!(toml_value(toml, "edition"), None);
    }

    #[test]
    fn ci_pins_are_found() {
        std::env::set_current_dir(root()).unwrap();
        assert!(ci_value("NIGHTLY").unwrap().starts_with("nightly-"));
        assert!(ci_value("MIN_LINES").unwrap().parse::<u32>().is_ok());
    }
}
