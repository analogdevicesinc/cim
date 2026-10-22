// Copyright (c) 2026 Analog Devices, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! End-to-end tests for the git clone/mirror lifecycle of a target's own
//! `gits:` entries, across all three entry points that can trigger it:
//! `cim init`, `cim update` and `cim bootstrap`. Everything runs over local
//! `file://` repositories and plain-directory manifests — no network.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "cim test")
        .env("GIT_AUTHOR_EMAIL", "cim@test.invalid")
        .env("GIT_COMMITTER_NAME", "cim test")
        .env("GIT_COMMITTER_EMAIL", "cim@test.invalid")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .output()
        .expect("failed to run git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Commit hash `rev` resolves to in the git repository at `dir`.
fn rev_parse(dir: &Path, rev: &str) -> String {
    let out = Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(dir)
        .output()
        .expect("failed to run git");
    assert!(
        out.status.success(),
        "git rev-parse {} failed in {}: {}",
        rev,
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Add and commit a file in `dir`.
fn commit_file(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", name]);
}

struct Fixture {
    tmp: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("home")).unwrap();
        Fixture { tmp }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    /// Create a non-bare git repo at `root/<name>` with one commit on `main`,
    /// suitable for use as a `gits:` upstream reached via `file://`.
    fn upstream(&self, name: &str) -> PathBuf {
        let dir = self.root().join(name);
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        fs::write(dir.join("README"), "init\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "init"]);
        dir
    }

    fn file_url(path: &Path) -> String {
        format!("file://{}", path.display())
    }

    /// Write `targets/<target>/sdk.yml` under a fresh plain-directory
    /// manifest at `root/manifest` (no git required: a local directory
    /// with a `targets/` subfolder is read in place as a manifest source).
    fn manifest(&self, target: &str, sdk_yml: &str) -> PathBuf {
        let manifest = self.root().join("manifest");
        let target_dir = manifest.join("targets").join(target);
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("sdk.yml"), sdk_yml).unwrap();
        manifest
    }

    fn cim(&self, cwd: &Path, args: &[&str]) -> Output {
        let home = self.root().join("home");
        Command::new(env!("CARGO_BIN_EXE_cim"))
            .args(args)
            .current_dir(cwd)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("failed to run cim")
    }

    fn cim_ok(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.cim(cwd, args);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            out.status.success(),
            "cim {:?} failed\nstdout: {}\nstderr: {}",
            args,
            stdout,
            String::from_utf8_lossy(&out.stderr)
        );
        stdout
    }
}

/// `cim init` with a mirror enabled must expand a `${{ VAR }}` manifest
/// variable in a target's own `gits: url:` before cloning, creating exactly
/// one mirror directory whose `origin` is the resolved URL (not the literal
/// `${{ ... }}` string).
#[test]
fn test_init_clones_new_mirror_from_variable_url() {
    let f = Fixture::new();
    let dep = f.upstream("dep");

    let sdk = format!(
        "variables:\n  GIT_BASE: {}\ngits:\n  - name: dep\n    url: ${{{{ GIT_BASE }}}}/dep\n    commit: main\n",
        Fixture::file_url(f.root())
    );
    let manifest = f.manifest("demo", &sdk);

    let ws = f.root().join("ws");
    let mirror = f.root().join("mirror");
    f.cim_ok(
        f.root(),
        &[
            "init",
            "-t",
            "demo",
            "-s",
            &manifest.to_string_lossy(),
            "-w",
            &ws.to_string_lossy(),
            "-m",
            &mirror.to_string_lossy(),
        ],
    );

    let tip = rev_parse(&dep, "main");
    assert_eq!(rev_parse(&ws.join("dep"), "HEAD"), tip);

    let mirror_repo = mirror.join("dep");
    assert!(
        mirror_repo.join("HEAD").exists(),
        "expected a mirror at {}",
        mirror_repo.display()
    );
    assert_eq!(rev_parse(&mirror_repo, "refs/heads/main"), tip);

    let origin = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(&mirror_repo)
        .output()
        .expect("git remote get-url");
    let origin_url = String::from_utf8_lossy(&origin.stdout).trim().to_string();
    assert!(
        !origin_url.contains("${{"),
        "mirror origin was never expanded: {}",
        origin_url
    );
}

/// `cim update` must follow sequential upstream commits through a mirror
/// whose repo was declared with a `${{ VAR }}` URL (the regression this
/// guards: `load_update_sdk_config` used to skip variable expansion
/// entirely, so the literal `${{ GIT_BASE }}/...` string was handed to git
/// and every mirror/workspace operation failed). A no-op update (no new
/// upstream commits) must also succeed cleanly, and the persistent mirror
/// must never pick up a `shallow` marker (a shallow mirror later refuses
/// `git clone --reference`).
#[test]
fn test_update_tracks_sequential_deltas_from_variable_url() {
    let f = Fixture::new();
    let dep = f.upstream("dep");

    let sdk = format!(
        "variables:\n  GIT_BASE: {}\ngits:\n  - name: dep\n    url: ${{{{ GIT_BASE }}}}/dep\n    commit: main\n",
        Fixture::file_url(f.root())
    );
    let manifest = f.manifest("demo", &sdk);

    let ws = f.root().join("ws");
    let mirror = f.root().join("mirror");
    f.cim_ok(
        f.root(),
        &[
            "init",
            "-t",
            "demo",
            "-s",
            &manifest.to_string_lossy(),
            "-w",
            &ws.to_string_lossy(),
            "-m",
            &mirror.to_string_lossy(),
        ],
    );
    let mirror_repo = mirror.join("dep");

    for delta in ["delta-1", "delta-2"] {
        commit_file(&dep, delta, delta);
        let tip = rev_parse(&dep, "main");
        f.cim_ok(&ws, &["update", "-m", &mirror.to_string_lossy()]);
        assert_eq!(rev_parse(&ws.join("dep"), "HEAD"), tip, "after {}", delta);
        assert_eq!(
            rev_parse(&mirror_repo, "refs/heads/main"),
            tip,
            "mirror after {}",
            delta
        );
        assert!(
            !mirror_repo.join("shallow").exists(),
            "persistent mirror must never become shallow (after {})",
            delta
        );
    }

    // A third update with no new upstream commits must be a clean no-op.
    let tip = rev_parse(&dep, "main");
    f.cim_ok(&ws, &["update", "-m", &mirror.to_string_lossy()]);
    assert_eq!(rev_parse(&ws.join("dep"), "HEAD"), tip, "no-op update");
}

/// `cim init --no-mirror` must clone a target's `gits:` directly from the
/// upstream URL and never create a mirror directory at all.
#[test]
fn test_init_no_mirror_clones_directly_without_mirror_dir() {
    let f = Fixture::new();
    let dep = f.upstream("dep");

    let sdk = format!(
        "gits:\n  - name: dep\n    url: {}\n    commit: main\n",
        Fixture::file_url(&dep)
    );
    let manifest = f.manifest("demo", &sdk);

    let ws = f.root().join("ws");
    let mirror = f.root().join("mirror");
    f.cim_ok(
        f.root(),
        &[
            "init",
            "-t",
            "demo",
            "-s",
            &manifest.to_string_lossy(),
            "-w",
            &ws.to_string_lossy(),
            "-m",
            &mirror.to_string_lossy(),
            "--no-mirror",
        ],
    );

    let tip = rev_parse(&dep, "main");
    assert_eq!(rev_parse(&ws.join("dep"), "HEAD"), tip);
    assert!(
        !mirror.exists() || fs::read_dir(&mirror).unwrap().next().is_none(),
        "no-mirror init must not create any mirror directory"
    );
}

/// `cim update` on a workspace whose marker recorded the `--no-mirror`
/// preference from init must fix up a stale `origin` remote before
/// fetching, not just fetch whatever `origin` already happens to point at.
/// This is the regression test for the `fetch_existing_workspace_repo` fix:
/// without it, this test fails because the fetch targets the stale URL.
#[test]
fn test_update_no_mirror_fixes_stale_origin_and_fetches() {
    let f = Fixture::new();
    let dep = f.upstream("dep");

    let sdk = format!(
        "gits:\n  - name: dep\n    url: {}\n    commit: main\n",
        Fixture::file_url(&dep)
    );
    let manifest = f.manifest("demo", &sdk);

    let ws = f.root().join("ws");
    f.cim_ok(
        f.root(),
        &[
            "init",
            "-t",
            "demo",
            "-s",
            &manifest.to_string_lossy(),
            "-w",
            &ws.to_string_lossy(),
            "--no-mirror",
        ],
    );

    // Simulate the real-world stale state: origin now points somewhere else.
    let bogus = f.root().join("nonexistent-remote");
    git(
        &ws.join("dep"),
        &["remote", "set-url", "origin", &Fixture::file_url(&bogus)],
    );

    commit_file(&dep, "delta", "1");
    let tip = rev_parse(&dep, "main");

    // No `--no-mirror` flag here: it must come from the workspace marker.
    f.cim_ok(&ws, &["update"]);

    assert_eq!(rev_parse(&ws.join("dep"), "HEAD"), tip);
    let origin = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(ws.join("dep"))
        .output()
        .expect("git remote get-url");
    assert_eq!(
        String::from_utf8_lossy(&origin.stdout).trim(),
        Fixture::file_url(&dep)
    );
}

/// `cim update --match` must limit both the mirror and workspace update to
/// the matching git, leaving the other git's mirror and workspace checkout
/// untouched even though its upstream also moved.
#[test]
fn test_update_match_filter_limits_which_gits_are_touched() {
    let f = Fixture::new();
    let dep_a = f.upstream("dep-a");
    let dep_b = f.upstream("dep-b");

    let sdk = format!(
        "gits:\n  - name: dep-a\n    url: {}\n    commit: main\n  - name: dep-b\n    url: {}\n    commit: main\n",
        Fixture::file_url(&dep_a),
        Fixture::file_url(&dep_b)
    );
    let manifest = f.manifest("demo", &sdk);

    let ws = f.root().join("ws");
    let mirror = f.root().join("mirror");
    f.cim_ok(
        f.root(),
        &[
            "init",
            "-t",
            "demo",
            "-s",
            &manifest.to_string_lossy(),
            "-w",
            &ws.to_string_lossy(),
            "-m",
            &mirror.to_string_lossy(),
        ],
    );
    let unchanged_tip = rev_parse(&ws.join("dep-b"), "HEAD");

    commit_file(&dep_a, "delta", "1");
    commit_file(&dep_b, "delta", "1");
    let new_tip_a = rev_parse(&dep_a, "main");

    f.cim_ok(
        &ws,
        &["update", "-m", &mirror.to_string_lossy(), "-p", "^dep-a$"],
    );

    assert_eq!(rev_parse(&ws.join("dep-a"), "HEAD"), new_tip_a);
    assert_eq!(
        rev_parse(&ws.join("dep-b"), "HEAD"),
        unchanged_tip,
        "unmatched git must not be touched by a filtered update"
    );
}

/// `cim update --include-group` must limit the update to gits whose group
/// set intersects the requested group(s), leaving a git in a different
/// group untouched even though its upstream also moved. `dep-a` has no
/// `group:` set, so it implicitly belongs to the "default" group; `dep-b`
/// is explicitly tagged `group: extra`.
#[test]
fn test_update_include_group_limits_which_gits_are_touched() {
    let f = Fixture::new();
    let dep_a = f.upstream("dep-a");
    let dep_b = f.upstream("dep-b");

    let sdk = format!(
        "gits:\n  - name: dep-a\n    url: {}\n    commit: main\n  - name: dep-b\n    url: {}\n    commit: main\n    group: extra\n",
        Fixture::file_url(&dep_a),
        Fixture::file_url(&dep_b)
    );
    let manifest = f.manifest("demo", &sdk);

    let ws = f.root().join("ws");
    let mirror = f.root().join("mirror");
    f.cim_ok(
        f.root(),
        &[
            "init",
            "-t",
            "demo",
            "-s",
            &manifest.to_string_lossy(),
            "-w",
            &ws.to_string_lossy(),
            "-m",
            &mirror.to_string_lossy(),
        ],
    );
    let unchanged_tip = rev_parse(&ws.join("dep-b"), "HEAD");

    commit_file(&dep_a, "delta", "1");
    commit_file(&dep_b, "delta", "1");
    let new_tip_a = rev_parse(&dep_a, "main");

    f.cim_ok(
        &ws,
        &["update", "-m", &mirror.to_string_lossy(), "-g", "default"],
    );

    assert_eq!(rev_parse(&ws.join("dep-a"), "HEAD"), new_tip_a);
    assert_eq!(
        rev_parse(&ws.join("dep-b"), "HEAD"),
        unchanged_tip,
        "a git outside the included group must not be touched"
    );
}

/// `cim update --exclude-group` must drop gits whose group set intersects
/// the excluded group(s) from the update, leaving the rest untouched by the
/// exclusion (mirroring `--include-group` from the other direction).
#[test]
fn test_update_exclude_group_limits_which_gits_are_touched() {
    let f = Fixture::new();
    let dep_a = f.upstream("dep-a");
    let dep_b = f.upstream("dep-b");

    let sdk = format!(
        "gits:\n  - name: dep-a\n    url: {}\n    commit: main\n  - name: dep-b\n    url: {}\n    commit: main\n    group: extra\n",
        Fixture::file_url(&dep_a),
        Fixture::file_url(&dep_b)
    );
    let manifest = f.manifest("demo", &sdk);

    let ws = f.root().join("ws");
    let mirror = f.root().join("mirror");
    f.cim_ok(
        f.root(),
        &[
            "init",
            "-t",
            "demo",
            "-s",
            &manifest.to_string_lossy(),
            "-w",
            &ws.to_string_lossy(),
            "-m",
            &mirror.to_string_lossy(),
        ],
    );
    let unchanged_tip = rev_parse(&ws.join("dep-b"), "HEAD");

    commit_file(&dep_a, "delta", "1");
    commit_file(&dep_b, "delta", "1");
    let new_tip_a = rev_parse(&dep_a, "main");

    f.cim_ok(
        &ws,
        &["update", "-m", &mirror.to_string_lossy(), "-G", "extra"],
    );

    assert_eq!(rev_parse(&ws.join("dep-a"), "HEAD"), new_tip_a);
    assert_eq!(
        rev_parse(&ws.join("dep-b"), "HEAD"),
        unchanged_tip,
        "an excluded group's git must not be touched"
    );
}

/// `cim bootstrap` delegates all cloning to `cim init`'s own machinery; this
/// is a smoke test that the flag plumbing (`--target`, `--source`,
/// `--no-mirror`, `--yes` for non-interactive use) actually reaches it and
/// produces a built, cloned workspace end to end.
#[test]
fn test_bootstrap_clones_target_and_completes_non_interactively() {
    let f = Fixture::new();
    let dep = f.upstream("dep");

    let sdk = format!(
        "gits:\n  - name: dep\n    url: {}\n    commit: main\n",
        Fixture::file_url(&dep)
    );
    let manifest = f.manifest("demo", &sdk);

    let ws = f.root().join("ws");
    f.cim_ok(
        f.root(),
        &[
            "bootstrap",
            "-t",
            "demo",
            "-s",
            &manifest.to_string_lossy(),
            "-w",
            &ws.to_string_lossy(),
            "--no-mirror",
            "--yes",
        ],
    );

    let tip = rev_parse(&dep, "main");
    assert_eq!(rev_parse(&ws.join("dep"), "HEAD"), tip);
    assert!(
        ws.join("Makefile").exists(),
        "bootstrap must generate a Makefile"
    );
}
