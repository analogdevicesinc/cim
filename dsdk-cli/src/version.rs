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

use dsdk_cli::messages;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

/// Code in Motion version information
pub(crate) struct CimVersion {
    pub version: String,
    pub sha256: String,
    pub commit: String,
}

/// Get the current Code in Motion version information
pub(crate) fn get_cim_version() -> CimVersion {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let commit = env!("GIT_HASH").to_string();

    let sha256 = match std::env::current_exe() {
        Ok(exe_path) => match std::fs::read(&exe_path) {
            Ok(binary_data) => {
                let mut hasher = Sha256::new();
                hasher.update(&binary_data);
                format!("{:x}", hasher.finalize())
            }
            Err(_) => "unknown".to_string(),
        },
        Err(_) => "unknown".to_string(),
    };

    CimVersion {
        version,
        sha256,
        commit,
    }
}

/// Print version information including SHA256 hash of the current binary and git commit
pub(crate) fn print_version_info() {
    let version_info = get_cim_version();

    messages::status(&format!("cim: v{}", version_info.version));
    messages::status(&format!("  SHA256: {}", version_info.sha256));
    messages::status(&format!("  Commit: {}", version_info.commit));
}

/// Compare two semver version strings (e.g. "1.2.3"). Returns true if `latest` is newer than `current`.
pub(crate) fn is_newer_version(current: &str, latest: &str) -> bool {
    fn parse(v: &str) -> Option<(u64, u64, u64)> {
        let v = v.trim_start_matches('v');
        let parts: Vec<&str> = v.splitn(3, '.').collect();
        if parts.len() < 3 {
            return None;
        }
        let major = parts[0].parse::<u64>().ok()?;
        let minor = parts[1].parse::<u64>().ok()?;
        let patch = parts[2].split('-').next()?.parse::<u64>().ok()?;
        Some((major, minor, patch))
    }
    match (parse(current), parse(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// Build the archive filename for the given version and the current platform.
/// Returns None if the current platform is not a recognised release target.
pub(crate) fn platform_archive_name(version: &str) -> Option<String> {
    let target = env!("BUILD_TARGET");
    let ext = if cfg!(target_os = "windows") {
        "zip"
    } else {
        "tar.gz"
    };
    // Only produce a name for the targets we actually ship
    match target {
        "x86_64-unknown-linux-gnu"
        | "x86_64-unknown-linux-musl"
        | "aarch64-unknown-linux-gnu"
        | "x86_64-apple-darwin"
        | "aarch64-apple-darwin"
        | "x86_64-pc-windows-msvc" => Some(format!("cim-suite-v{}-{}.{}", version, target, ext)),
        _ => None,
    }
}

/// Query the GitHub releases API and return the latest version tag (without leading 'v').
/// Returns None on any network or parse error – callers should treat None as "no update info".
pub(crate) fn fetch_latest_release_version() -> Option<String> {
    let url = "https://api.github.com/repos/analogdevicesinc/cim/releases/latest";
    let version = env!("CARGO_PKG_VERSION");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!("cim/{}", version))
        .build()
        .ok()?;

    let body = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .ok()?
        .text()
        .ok()?;

    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let tag = json["tag_name"].as_str()?;
    Some(tag.trim_start_matches('v').to_string())
}

/// Find the `cim` binary that is first on PATH — i.e. the one the user actually
/// invokes — so the self-update replaces the installed binary rather than whatever
/// binary happens to be running (which could be a dev/debug build).
pub(crate) fn find_cim_in_path() -> Option<PathBuf> {
    let exe_name = if cfg!(target_os = "windows") {
        "cim.exe"
    } else {
        "cim"
    };
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Spawn a background thread that checks for a newer cim release.
/// The caller should join the handle when it is ready to display the result.
pub(crate) fn spawn_version_check() -> thread::JoinHandle<Option<String>> {
    thread::spawn(|| {
        let latest = fetch_latest_release_version()?;
        let current = env!("CARGO_PKG_VERSION");
        if is_newer_version(current, &latest) {
            Some(latest)
        } else {
            None
        }
    })
}

/// If the version-check thread found a newer release, print an update notice.
pub(crate) fn print_update_notice(handle: thread::JoinHandle<Option<String>>) {
    if let Ok(Some(latest)) = handle.join() {
        messages::status(&format!(
            "\nNote: A newer version of cim is available (v{}). Run 'cim utils update' to upgrade.",
            latest
        ));
    }
}
