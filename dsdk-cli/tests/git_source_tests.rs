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

//! End-to-end tests for manifest sources that are git repositories reached
//! through `file://` URLs or a plain path to a bare repository.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const MARKER_V1: &str = "# manifest-v1";
const MARKER_V2: &str = "# manifest-v2";

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

struct Fixture {
    tmp: TempDir,
}

impl Fixture {
    /// Build:
    /// - `dep/`: a small git repo the manifest references via file://
    /// - `manifest.git`: a bare repo with `targets/demo/sdk.yml`, where tag
    ///   `v1.0` carries MARKER_V1 and the tip of `main` carries MARKER_V2.
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let dep = root.join("dep");
        fs::create_dir_all(&dep).unwrap();
        git(&dep, &["init", "-q", "-b", "main"]);
        fs::write(dep.join("README"), "dep\n").unwrap();
        git(&dep, &["add", "."]);
        git(&dep, &["commit", "-q", "-m", "init"]);

        let wt = root.join("manifest-wt");
        let target_dir = wt.join("targets").join("demo");
        fs::create_dir_all(&target_dir).unwrap();
        git(&wt, &["init", "-q", "-b", "main"]);
        let sdk = |marker: &str| {
            format!(
                "{}\ngits:\n  - name: dep\n    url: file://{}\n    commit: main\n",
                marker,
                dep.display()
            )
        };
        fs::write(target_dir.join("sdk.yml"), sdk(MARKER_V1)).unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-q", "-m", "v1"]);
        git(&wt, &["tag", "-a", "v1.0", "-m", "v1.0"]);
        git(&wt, &["tag", "-a", "demo-1.0", "-m", "demo-1.0"]);
        fs::write(target_dir.join("sdk.yml"), sdk(MARKER_V2)).unwrap();
        git(&wt, &["commit", "-q", "-am", "v2"]);

        git(
            root,
            &["clone", "-q", "--bare", "manifest-wt", "manifest.git"],
        );
        fs::create_dir_all(root.join("home")).unwrap();
        Fixture { tmp }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn bare_path(&self) -> String {
        self.root()
            .join("manifest.git")
            .to_string_lossy()
            .to_string()
    }

    fn file_url(&self) -> String {
        format!("file://{}", self.bare_path())
    }

    /// A stand-in for ssh that runs the remote command locally, so scp-like
    /// `user@host:path` sources can be exercised without an ssh server.
    /// With GIT_SSH_VARIANT=simple git calls it as `<ssh> <host> <command>`.
    fn fake_ssh(&self) -> String {
        let script = self.root().join("fake-ssh");
        if !script.exists() {
            fs::write(&script, "#!/bin/sh\nshift\nexec sh -c \"$*\"\n").unwrap();
            let mut perm = fs::metadata(&script).unwrap().permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
            fs::set_permissions(&script, perm).unwrap();
        }
        script.to_string_lossy().to_string()
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
            .env("GIT_SSH_COMMAND", self.fake_ssh())
            .env("GIT_SSH_VARIANT", "simple")
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

    fn init(&self, source: &str, version: Option<&str>, ws_name: &str) -> PathBuf {
        let ws = self.root().join(ws_name);
        let ws_str = ws.to_string_lossy().to_string();
        let mut args = vec![
            "init",
            "-t",
            "demo",
            "-s",
            source,
            "-w",
            &ws_str,
            "--no-mirror",
        ];
        if let Some(v) = version {
            args.push("-v");
            args.push(v);
        }
        self.cim_ok(self.root(), &args);
        assert!(ws.join("dep").join("README").exists(), "dep not cloned");
        ws
    }
}

fn workspace_sdk(ws: &Path) -> String {
    fs::read_to_string(ws.join("sdk.yml")).unwrap()
}

#[test]
fn test_list_targets_from_file_url_and_bare_path() {
    let f = Fixture::new();
    for source in [f.file_url(), f.bare_path()] {
        let out = f.cim_ok(f.root(), &["list-targets", "-s", &source]);
        assert!(out.contains("- demo"), "missing target in: {}", out);
    }
}

#[test]
fn test_list_target_versions_from_git_sources() {
    let f = Fixture::new();
    for source in [f.file_url(), f.bare_path()] {
        let out = f.cim_ok(f.root(), &["list-targets", "-s", &source, "-t", "demo"]);
        assert!(out.contains("- demo-1.0"), "missing version in: {}", out);
        assert!(!out.contains("^{}"), "peeled tag ref leaked: {}", out);
    }
}

#[test]
fn test_init_from_file_url() {
    let f = Fixture::new();
    let latest = f.init(&f.file_url(), None, "ws-file-latest");
    assert!(workspace_sdk(&latest).contains(MARKER_V2));

    let tagged = f.init(&f.file_url(), Some("v1.0"), "ws-file-tag");
    assert!(workspace_sdk(&tagged).contains(MARKER_V1));
    let marker = fs::read_to_string(tagged.join(".workspace")).unwrap();
    assert!(marker.contains(&f.file_url()), "marker: {}", marker);
    assert!(marker.contains("v1.0"), "marker: {}", marker);

    // cim update must work in a workspace created from a file:// source
    f.cim_ok(&tagged, &["update"]);
}

#[test]
fn test_init_from_bare_path() {
    let f = Fixture::new();
    let latest = f.init(&f.bare_path(), None, "ws-bare-latest");
    assert!(workspace_sdk(&latest).contains(MARKER_V2));

    let tagged = f.init(&f.bare_path(), Some("v1.0"), "ws-bare-tag");
    assert!(workspace_sdk(&tagged).contains(MARKER_V1));

    f.cim_ok(&tagged, &["update"]);
}

#[test]
fn test_init_from_relative_bare_path_records_absolute_source() {
    let f = Fixture::new();
    let ws = f.root().join("ws-rel");
    let ws_str = ws.to_string_lossy().to_string();
    f.cim_ok(
        f.root(),
        &[
            "init",
            "-t",
            "demo",
            "-s",
            "./manifest.git",
            "-w",
            &ws_str,
            "--no-mirror",
        ],
    );
    let marker = fs::read_to_string(ws.join(".workspace")).unwrap();
    let canonical = fs::canonicalize(f.root().join("manifest.git")).unwrap();
    assert!(
        marker.contains(&*canonical.to_string_lossy()),
        "marker should record an absolute source: {}",
        marker
    );
}

#[test]
fn test_scp_like_source_that_exists_locally_stays_local() {
    // "host:dir" has the scp-like shape, but names an existing manifest
    // directory relative to the current directory, so it is read in place.
    let f = Fixture::new();
    let dir = f.root().join("host:dir").join("targets").join("local-only");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("sdk.yml"), "gits: []\n").unwrap();
    let out = f.cim_ok(f.root(), &["list-targets", "-s", "host:dir"]);
    assert!(out.contains("- local-only"), "missing target in: {}", out);
}

#[test]
fn test_list_targets_and_init_from_scp_like_source() {
    let f = Fixture::new();
    let scp = format!("someone@somehost:{}", f.bare_path());
    let out = f.cim_ok(f.root(), &["list-targets", "-s", &scp]);
    assert!(out.contains("- demo"), "missing target in: {}", out);
    let out = f.cim_ok(f.root(), &["list-targets", "-s", &scp, "-t", "demo"]);
    assert!(out.contains("- demo-1.0"), "missing version in: {}", out);

    let ws = f.init(&scp, Some("v1.0"), "ws-scp-tag");
    assert!(workspace_sdk(&ws).contains(MARKER_V1));
    let marker = fs::read_to_string(ws.join(".workspace")).unwrap();
    assert!(marker.contains(&scp), "marker: {}", marker);
}

#[test]
fn test_marker_source_for_local_checkout_with_version_is_refetched() {
    // A non-bare checkout used with -v must be re-read from git at that
    // version by later commands (cim release resolves copy_files sources
    // against this directory), not from the repository's live working tree.
    let f = Fixture::new();
    let wt = f.root().join("manifest-wt");
    let wt_str = wt.to_string_lossy().to_string();
    // Uncommitted change on top of main: must not leak into either result.
    fs::write(wt.join("targets/demo/sdk.yml"), "# dirty\ngits: []\n").unwrap();

    let ws = f.root().join("ws-wt-tag");
    let ws_str = ws.to_string_lossy().to_string();
    f.cim_ok(
        f.root(),
        &[
            "init",
            "-t",
            "demo",
            "-s",
            &wt_str,
            "-v",
            "v1.0",
            "-w",
            &ws_str,
            "--no-mirror",
        ],
    );
    assert!(workspace_sdk(&ws).contains(MARKER_V1));

    let (dir, _keep) =
        dsdk_cli::workspace::resolve_config_source_dir_from_marker(&ws, &ws.join("sdk.yml"));
    let sdk = fs::read_to_string(dir.join("sdk.yml")).unwrap();
    assert!(
        sdk.contains(MARKER_V1),
        "source dir {} is not the target at v1.0: {}",
        dir.display(),
        sdk
    );
}
