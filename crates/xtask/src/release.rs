//! `cargo xtask release`: the dist plan, the release checks, the version bump, the package
//! manager manifests and the crates.io publish.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, bail};
use clap::Subcommand;

use crate::cmd::{cargo, need, output, run, steps};
use crate::{assets, deny, toml_value};

#[derive(Subcommand)]
pub(crate) enum Release {
    /// What dist would build and publish for a tag.
    Plan,
    /// The checks a release needs on top of CI: assets current, advisories, dist plan.
    Check,
    /// Set the workspace version, update Cargo.lock, the assets and CHANGELOG.md; with --commit,
    /// commit and tag.
    Bump {
        /// The new version, e.g. 0.2.0; by default the one the conventional commits since the
        /// last tag call for (git-cliff --bumped-version).
        version: Option<String>,
        #[arg(long)]
        commit: bool,
    },
    /// Fill the Scoop, winget and AUR manifests for a published release (scripts/package-manifests.sh).
    Manifests {
        version: String,
        /// Output directory (default target/pkg/VERSION).
        out: Option<PathBuf>,
    },
    /// Publish dfdu and the crates it needs to crates.io, in dependency order (`cargo publish
    /// --workspace`); the crates with `publish = false` stay out. Releases publish from CI
    /// (publish-crates.yml); this is for a new crate's first version, which trusted publishing
    /// can't create.
    Publish {
        /// Package and verify every crate without uploading.
        #[arg(long)]
        dry_run: bool,
    },
}

pub(crate) fn release(r: Release) -> anyhow::Result<()> {
    match r {
        Release::Plan => {
            need("dist")?;
            run(Command::new("dist").arg("plan"))
        }
        Release::Check => steps(vec![
            (
                "assets",
                Box::new(|| run(&mut cargo(["test", "-p", "dfdu", "--test", "assets"]))),
            ),
            ("advisories", Box::new(|| deny(true))),
            ("dist plan", Box::new(|| release(Release::Plan))),
            (
                "crates.io",
                Box::new(|| {
                    run(&mut cargo([
                        "publish",
                        "--workspace",
                        "--locked",
                        "--dry-run",
                    ]))
                }),
            ),
        ]),
        Release::Bump { version, commit } => {
            need("git-cliff")?;
            let version = match version {
                Some(v) => v,
                None => {
                    let v = output(Command::new("git-cliff").arg("--bumped-version"))?;
                    v.trim_start_matches('v').to_owned()
                }
            };
            bump(&version, commit)
        }
        Release::Manifests { version, out } => {
            need("gh")?;
            let mut c = Command::new("scripts/package-manifests.sh");
            c.arg(&version);
            if let Some(o) = out {
                c.arg(o);
            }
            run(&mut c)
        }
        Release::Publish { dry_run } => {
            let mut c = cargo(["publish", "--workspace", "--locked"]);
            if dry_run {
                c.arg("--dry-run");
            }
            run(&mut c)
        }
    }
}

/// Whether `version` reads as a version like 1.2.3: three or more parts, a numeric major.
fn is_version(version: &str) -> bool {
    version.split('.').count() >= 3
        && version
            .split(['.', '-', '+'])
            .next()
            .is_some_and(|m| m.parse::<u64>().is_ok())
}

/// `manifest` with [workspace.package]'s `version = "old"` (the first on a line of its own) set
/// to `new`, and with it the `version = "old"` of the workspace's own crates in
/// [workspace.dependencies] (the inline tables with a `path`), which crates.io needs. Other
/// dependencies keep theirs.
fn set_version(manifest: &str, old: &str, new: &str) -> String {
    let (from, to) = (
        format!("version = \"{old}\""),
        format!("version = \"{new}\""),
    );
    let mut package_done = false;
    manifest
        .split_inclusive('\n')
        .map(|l| {
            let package = !package_done && l.trim_end() == from;
            package_done |= package;
            if package || (l.contains("path = \"crates/") && l.contains(&from)) {
                l.replacen(&from, &to, 1)
            } else {
                l.to_owned()
            }
        })
        .collect()
}

fn bump(version: &str, commit: bool) -> anyhow::Result<()> {
    if !is_version(version) {
        bail!("{version:?} is not a version like 1.2.3");
    }
    if commit && !output(Command::new("git").args(["status", "--porcelain"]))?.is_empty() {
        bail!("the working tree has changes; commit or stash them first");
    }
    let manifest = fs::read_to_string("Cargo.toml")?;
    let old = toml_value(&manifest, "version").context("no version in Cargo.toml")?;
    if old == version {
        bail!(
            "the version is already {version} (git-cliff finds nothing to bump without a feat, fix or perf commit since the last tag)"
        );
    }
    let changes = fs::read_to_string("CHANGELOG.md").context("reading CHANGELOG.md")?;
    if has_section(&changes, version) {
        bail!("CHANGELOG.md already has a section for {version}");
    }
    fs::write("Cargo.toml", set_version(&manifest, &old, version))?;
    eprintln!("version {old} -> {version}");
    run(&mut cargo(["update", "--workspace"]))?;
    // The fuzz targets are a workspace of their own, with their own lock file.
    run(&mut cargo([
        "update",
        "--workspace",
        "--manifest-path",
        "fuzz/Cargo.toml",
    ]))?;
    assets()?;
    changelog(version)?;
    if commit {
        let tag = format!("v{version}");
        run(Command::new("git").args(["commit", "-am", &format!("chore(release): {tag}")]))?;
        run(Command::new("git").args(["tag", "-a", &tag, "-m", &tag]))?;
        eprintln!("\npush to publish: git push origin HEAD {tag}");
        eprintln!("the release workflow publishes to crates.io (publish-crates.yml)");
    } else {
        eprintln!("\nedit CHANGELOG.md's new section, then commit and tag v{version}");
    }
    Ok(())
}

/// Inserts `version`'s section, from the commits since the last tag (cliff.toml), into
/// CHANGELOG.md above the newest release's, so the sections edited by hand stay as they are.
fn changelog(version: &str) -> anyhow::Result<()> {
    let tag = format!("v{version}");
    let section = output(Command::new("git-cliff").args([
        "--unreleased",
        "--tag",
        &tag,
        "--strip",
        "header",
    ]))?;
    let file = fs::read_to_string("CHANGELOG.md").context("reading CHANGELOG.md")?;
    fs::write("CHANGELOG.md", insert_section(&file, &section))?;
    eprintln!("CHANGELOG.md: added {tag}'s section");
    Ok(())
}

/// Whether `changelog` has a `## [version]` heading.
fn has_section(changelog: &str, version: &str) -> bool {
    let heading = format!("## [{version}]");
    changelog.lines().any(|l| l.starts_with(&heading))
}

/// `changelog` with `section` before its first `## ` heading, or at the end if it has none.
fn insert_section(changelog: &str, section: &str) -> String {
    let section = section.trim();
    let at = if changelog.starts_with("## ") {
        Some(0)
    } else {
        changelog.find("\n## ").map(|i| i + 1)
    };
    match at {
        Some(i) => format!("{}{section}\n\n{}", &changelog[..i], &changelog[i..]),
        None => format!("{}\n\n{section}\n", changelog.trim_end()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_has_a_section_only_under_its_own_heading() {
        let file = "# Changelog\n\n## [0.3.0] - 2026-11-03\n\n- Mentions [0.2.0]\n";
        assert!(has_section(file, "0.3.0"));
        assert!(!has_section(file, "0.2.0"));
        assert!(!has_section(file, "0.3"));
    }

    #[test]
    fn a_section_goes_above_the_newest_release() {
        let file = "# Changelog\n\nIntro.\n\n## [0.1.0] - 2026-10-30\n\n- One\n";
        assert_eq!(
            insert_section(file, "## [0.2.0] - 2026-11-01\n\n- Two\n"),
            "# Changelog\n\nIntro.\n\n## [0.2.0] - 2026-11-01\n\n- Two\n\n## [0.1.0] - 2026-10-30\n\n- One\n"
        );
        assert_eq!(
            insert_section("# Changelog\n", "## [0.1.0]\n\n- One"),
            "# Changelog\n\n## [0.1.0]\n\n- One\n"
        );
    }

    #[test]
    fn versions_need_three_parts_and_a_numeric_major() {
        for v in ["0.2.0", "1.2.3-rc.1", "10.0.0+build"] {
            assert!(is_version(v), "{v}");
        }
        for v in ["1.2", "v1.2.3", "", "x.2.3"] {
            assert!(!is_version(v), "{v}");
        }
    }

    #[test]
    fn the_workspace_version_and_its_crates_dependencies_are_set() {
        let manifest = "[workspace.package]\nversion = \"0.1.0\"\n\n[workspace.dependencies]\n\
                        dfdu-tree = { path = \"crates/dfdu-tree\", version = \"0.1.0\" }\n\
                        dfdu-bench = { path = \"crates/dfdu-bench\" }\n\
                        dfdu-scan = { path = \"crates/dfdu-scan\", version = \"0.1.0\" }\n\
                        other = { version = \"0.1.0\" }\n\
                        another = \"0.1.0\"\n";
        let got = set_version(manifest, "0.1.0", "0.2.0");
        assert_eq!(
            got,
            "[workspace.package]\nversion = \"0.2.0\"\n\n[workspace.dependencies]\n\
             dfdu-tree = { path = \"crates/dfdu-tree\", version = \"0.2.0\" }\n\
             dfdu-bench = { path = \"crates/dfdu-bench\" }\n\
             dfdu-scan = { path = \"crates/dfdu-scan\", version = \"0.2.0\" }\n\
             other = { version = \"0.1.0\" }\n\
             another = \"0.1.0\"\n"
        );
        assert_eq!(toml_value(&got, "version").as_deref(), Some("0.2.0"));
    }
}
