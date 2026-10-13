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

use crate::cli::UtilsCommand;
use crate::release_cmd::{
    handle_copy_files_hash_command, handle_sync_files_hash_command, handle_toolchains_hash_command,
};
use crate::version::{
    fetch_latest_release_version, find_cim_in_path, is_newer_version, platform_archive_name,
};
use dsdk_cli::messages;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Update the cim binary to the latest release from GitHub.
///
/// Downloads the platform-appropriate archive, extracts the new binary, renames the
/// current binary to `cim.old`, then places the new binary in its location.
pub(crate) fn handle_utils_update_command() {
    let current_version = env!("CARGO_PKG_VERSION");

    let Some(exe_path) = locate_cim_binary() else {
        return;
    };
    messages::status(&format!("Updating: {}", exe_path.display()));

    messages::status(&format!("Current cim version: v{}", current_version));

    // Fetch latest version from GitHub
    let Some(latest_version) = fetch_latest_release_version() else {
        return; // silently ignore — no internet, no error
    };

    if !is_newer_version(current_version, &latest_version) {
        messages::success(&format!("cim is already up to date (v{})", current_version));
        return;
    }

    messages::status(&format!(
        "New version available: v{} → v{}",
        current_version, latest_version
    ));

    // Build the archive name for this platform
    let Some(archive_name) = platform_archive_name(&latest_version) else {
        messages::error(&format!(
            "No prebuilt release for platform '{}'. Please build from source.",
            env!("BUILD_TARGET")
        ));
        return;
    };

    let Ok(tmp_dir) = download_release_archive(&archive_name, current_version) else {
        return;
    };
    let archive_path = tmp_dir.path().join(&archive_name);

    messages::status("Extracting cim binary...");

    // Self-update is not supported on Windows
    #[cfg(target_os = "windows")]
    {
        messages::error(
            "Self-update via 'cim utils update' is not supported on Windows. \
             Please download and install manually from: \
             https://github.com/analogdevicesinc/cim/releases/latest",
        );
    }

    // Extract the `cim` binary from the archive and replace the current binary
    #[cfg(not(target_os = "windows"))]
    {
        let Ok(new_binary_path) = extract_cim_binary(&archive_path, tmp_dir.path()) else {
            return;
        };
        install_new_binary(
            &exe_path,
            &new_binary_path,
            current_version,
            &latest_version,
        );
    }
}

/// Locate the installed cim binary by searching PATH, so we update the one the
/// user actually invokes rather than the binary that happens to be running right now
/// (e.g. a dev build in target/debug/). Falls back to current_exe() if PATH lookup
/// yields nothing.
fn locate_cim_binary() -> Option<PathBuf> {
    let exe_path = find_cim_in_path().or_else(|| std::env::current_exe().ok());
    if exe_path.is_none() {
        messages::error("Cannot locate the installed cim binary in PATH.");
    }
    exe_path
}

/// Download the platform release archive into a fresh temp dir; the returned
/// `TempDir` owns the download and cleans it up when dropped.
fn download_release_archive(
    archive_name: &str,
    current_version: &str,
) -> Result<tempfile::TempDir, ()> {
    let download_url = format!(
        "https://github.com/analogdevicesinc/cim/releases/latest/download/{}",
        archive_name
    );

    messages::status(&format!("Downloading {}...", archive_name));

    // Download into a temporary directory
    let tmp_dir = tempfile::tempdir().map_err(|e| {
        messages::error(&format!("Failed to create temporary directory: {}", e));
    })?;

    let archive_path = tmp_dir.path().join(archive_name);

    // Use the same robust client pattern as the rest of the codebase
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent(format!("cim/{}", current_version))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| {
            messages::error(&format!("Failed to build HTTP client: {}", e));
        })?;

    let response = match client.get(&download_url).send() {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            messages::error(&format!(
                "HTTP {} downloading {}: {}",
                r.status().as_u16(),
                archive_name,
                r.status().canonical_reason().unwrap_or("error")
            ));
            return Err(());
        }
        Err(e) => {
            messages::error(&format!("Download failed: {}", e));
            return Err(());
        }
    };

    let archive_bytes = response.bytes().map_err(|e| {
        messages::error(&format!("Failed to read download: {}", e));
    })?;

    std::fs::write(&archive_path, &archive_bytes).map_err(|e| {
        messages::error(&format!("Failed to save archive: {}", e));
    })?;

    Ok(tmp_dir)
}

/// Extract the `cim` binary from the downloaded `.tar.gz` archive and mark it
/// executable, returning its path inside the temp dir.
#[cfg(not(target_os = "windows"))]
fn extract_cim_binary(archive_path: &Path, tmp_dir_path: &Path) -> Result<PathBuf, ()> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let archive_file = std::fs::File::open(archive_path).map_err(|e| {
        messages::error(&format!("Failed to open archive: {}", e));
    })?;

    let gz = GzDecoder::new(archive_file);
    let mut tar = Archive::new(gz);

    let extract_path = tmp_dir_path.join("cim");
    let mut found = false;

    let entries = tar.entries().map_err(|e| {
        messages::error(&format!("Failed to read archive entries: {}", e));
    })?;

    for entry in entries {
        let mut entry = entry.map_err(|e| {
            messages::error(&format!("Failed to read archive entry: {}", e));
        })?;

        let entry_path = match entry.path() {
            Ok(p) => p.to_path_buf(),
            Err(_) => continue,
        };

        // Match any entry whose filename is exactly "cim"
        if entry_path.file_name().map(|n| n == "cim").unwrap_or(false) {
            entry.unpack(&extract_path).map_err(|e| {
                messages::error(&format!("Failed to extract cim binary: {}", e));
            })?;
            found = true;
            break;
        }
    }

    if !found {
        messages::error("Archive does not contain a 'cim' binary.");
        return Err(());
    }

    // Set executable bit on the new binary
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&extract_path, std::fs::Permissions::from_mode(0o755)).map_err(
            |e| {
                messages::error(&format!("Failed to set executable permissions: {}", e));
            },
        )?;
    }

    Ok(extract_path)
}

/// Rename the current binary to `cim.old` and copy the newly extracted binary
/// into its place.
#[cfg(not(target_os = "windows"))]
fn install_new_binary(
    exe_path: &Path,
    new_binary_path: &Path,
    current_version: &str,
    latest_version: &str,
) {
    // Rename the old binary to cim.old, then copy the new one into place
    let old_path = exe_path.with_file_name("cim.old");

    if let Err(e) = std::fs::rename(exe_path, &old_path) {
        messages::error(&format!(
            "Failed to rename current binary to cim.old: {}",
            e
        ));
        return;
    }

    if let Err(e) = std::fs::copy(new_binary_path, exe_path) {
        messages::error(&format!(
            "Failed to copy new binary: {}. Old binary preserved at {}",
            e,
            old_path.display()
        ));
        // Attempt to restore the old binary
        let _ = std::fs::rename(&old_path, exe_path);
        return;
    }

    messages::success(&format!(
        "Successfully updated cim from v{} to v{}",
        current_version, latest_version,
    ));
}

/// Handle utility commands for workspace maintenance
pub(crate) fn handle_utils_command(utils_command: &UtilsCommand) {
    match utils_command {
        UtilsCommand::HashCopyFiles {
            file,
            dry_run,
            verbose,
            add_missing,
        } => {
            handle_copy_files_hash_command(file.as_deref(), *dry_run, *verbose, *add_missing);
        }
        UtilsCommand::HashToolchains {
            file,
            dry_run,
            verbose,
            add_missing,
        } => {
            handle_toolchains_hash_command(file.as_deref(), *dry_run, *verbose, *add_missing);
        }
        UtilsCommand::SyncCopyFiles {
            file,
            dry_run,
            verbose,
            force,
        } => {
            handle_sync_files_hash_command(file.as_deref(), *dry_run, *verbose, *force);
        }
        UtilsCommand::Update => {
            handle_utils_update_command();
        }
    }
}
