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

use anyhow::{anyhow, Result};
use std::process::Command;
use tokio::process::Command as TokioCommand;

/// Fetch available targets using `cim list-targets`
pub fn fetch_targets(source: &str) -> Result<Vec<String>> {

    let mut cmd = Command::new("cim");
    cmd.arg("list-targets");

    if !source.is_empty() {
        cmd.args(["--source", source]);
    }


    let output = cmd.output()?;


    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Failed to fetch targets: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut targets = Vec::new();

    // Parse output looking for lines with "  - target_name"
    for line in stdout.lines() {
        if let Some(target) = line.strip_prefix("  - ") {
            targets.push(target.trim().to_string());
        }
    }


    Ok(targets)
}

/// Fetch available versions for a specific target
pub fn fetch_versions(source: &str, target: &str) -> Result<Vec<String>> {
    let mut cmd = Command::new("cim");
    cmd.arg("list-targets");
    cmd.args(["--target", target]);

    if !source.is_empty() {
        cmd.args(["--source", source]);
    }

    let output = cmd.output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Failed to fetch versions: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut versions = Vec::new();

    // Parse output looking for lines with "  - version_name"
    for line in stdout.lines() {
        if let Some(version) = line.strip_prefix("  - ") {
            versions.push(version.trim().to_string());
        }
    }

    Ok(versions)
}

/// Build and execute the cim init command
pub fn run_init(
    target: &str,
    source: &str,
    version: Option<&str>,
    workspace: Option<&str>,
    no_mirror: bool,
    force: bool,
    match_pattern: Option<&str>,
    verbose: bool,
    install: bool,
    full: bool,
    no_sudo: bool,
    symlink: bool,
    yes: bool,
    cert_validation: Option<&str>,
) -> Result<tokio::process::Child> {
    let mut cmd = TokioCommand::new("cim");
    cmd.arg("init");
    cmd.args(["--target", target]);

    if !source.is_empty() {
        cmd.args(["--source", source]);
    }

    if let Some(v) = version {
        cmd.args(["--version", v]);
    }

    if let Some(w) = workspace {
        if !w.is_empty() {
            cmd.args(["--workspace", w]);
        }
    }

    if no_mirror {
        cmd.arg("--no-mirror");
    }

    if force {
        cmd.arg("--force");
    }

    if let Some(m) = match_pattern {
        if !m.is_empty() {
            cmd.args(["--match", m]);
        }
    }

    if verbose {
        cmd.arg("--verbose");
    }

    if install {
        cmd.arg("--install");
    }

    if full {
        cmd.arg("--full");
    }

    if no_sudo {
        cmd.arg("--no-sudo");
    }

    if symlink {
        cmd.arg("--symlink");
    }

    if yes {
        cmd.arg("--yes");
    }

    if let Some(cv) = cert_validation {
        if !cv.is_empty() && cv != "strict" {
            cmd.args(["--cert-validation", cv]);
        }
    }

    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn cim init: {}", e))
}
