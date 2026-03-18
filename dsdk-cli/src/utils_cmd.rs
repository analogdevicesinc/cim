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
use crate::release_cmd::{handle_copy_files_hash_command, handle_sync_files_hash_command};
use crate::version::{
    fetch_latest_release_version, find_cim_in_path, is_newer_version, platform_archive_name,
};
use dsdk_cli::messages;
use std::time::Duration;

/// Update the cim binary to the latest release from GitHub.
///
/// Downloads the platform-appropriate archive, extracts the new binary, renames the
/// current binary to `cim.old`, then places the new binary in its location.
pub(crate) fn handle_utils_update_command() {
    let current_version = env!("CARGO_PKG_VERSION");

    // Locate the installed cim binary by searching PATH, so we update the one the
    // user actually invokes rather than the binary that happens to be running right now
    // (e.g. a dev build in target/debug/).  Fall back to current_exe() if PATH lookup
    // yields nothing.
    let exe_path = find_cim_in_path().or_else(|| std::env::current_exe().ok());
    let exe_path = match exe_path {
        Some(p) => p,
        None => {
            messages::error("Cannot locate the installed cim binary in PATH.");
            return;
        }
    };
    messages::status(&format!("Updating: {}", exe_path.display()));

    messages::status(&format!("Current cim version: v{}", current_version));

    // Fetch latest version from GitHub
    let latest_version = match fetch_latest_release_version() {
        Some(v) => v,
        None => return, // silently ignore — no internet, no error
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
    let archive_name = match platform_archive_name(&latest_version) {
        Some(n) => n,
        None => {
            messages::error(&format!(
                "No prebuilt release for platform '{}'. Please build from source.",
                env!("BUILD_TARGET")
            ));
            return;
        }
    };

    let download_url = format!(
        "https://github.com/analogdevicesinc/cim/releases/latest/download/{}",
        archive_name
    );

    messages::status(&format!("Downloading {}...", archive_name));

    // Download into a temporary directory
    let tmp_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            messages::error(&format!("Failed to create temporary directory: {}", e));
            return;
        }
    };

    let archive_path = tmp_dir.path().join(&archive_name);

    // Use the same robust client pattern as the rest of the codebase
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent(format!("cim/{}", current_version))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            messages::error(&format!("Failed to build HTTP client: {}", e));
            return;
        }
    };

    let response = match client.get(&download_url).send() {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            messages::error(&format!(
                "HTTP {} downloading {}: {}",
                r.status().as_u16(),
                archive_name,
                r.status().canonical_reason().unwrap_or("error")
            ));
            return;
        }
        Err(e) => {
            messages::error(&format!("Download failed: {}", e));
            return;
        }
    };

    let archive_bytes = match response.bytes() {
        Ok(b) => b,
        Err(e) => {
            messages::error(&format!("Failed to read download: {}", e));
            return;
        }
    };

    if let Err(e) = std::fs::write(&archive_path, &archive_bytes) {
        messages::error(&format!("Failed to save archive: {}", e));
        return;
    }

    messages::status("Extracting cim binary...");

    // Self-update is not supported on Windows
    #[cfg(target_os = "windows")]
    {
        messages::error(
            "Self-update via 'cim utils update' is not supported on Windows. \
             Please download and install manually from: \
             https://github.com/analogdevicesinc/cim/releases/latest",
        );
        return;
    }

    // Extract the `cim` binary from the archive and replace the current binary
    #[cfg(not(target_os = "windows"))]
    {
        use flate2::read::GzDecoder;
        use tar::Archive;

        let archive_file = match std::fs::File::open(&archive_path) {
            Ok(f) => f,
            Err(e) => {
                messages::error(&format!("Failed to open archive: {}", e));
                return;
            }
        };

        let gz = GzDecoder::new(archive_file);
        let mut tar = Archive::new(gz);

        let extract_path = tmp_dir.path().join("cim");
        let mut found = false;

        let entries = match tar.entries() {
            Ok(e) => e,
            Err(e) => {
                messages::error(&format!("Failed to read archive entries: {}", e));
                return;
            }
        };

        for entry in entries {
            let mut entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    messages::error(&format!("Failed to read archive entry: {}", e));
                    return;
                }
            };

            let entry_path = match entry.path() {
                Ok(p) => p.to_path_buf(),
                Err(_) => continue,
            };

            // Match any entry whose filename is exactly "cim"
            if entry_path.file_name().map(|n| n == "cim").unwrap_or(false) {
                if let Err(e) = entry.unpack(&extract_path) {
                    messages::error(&format!("Failed to extract cim binary: {}", e));
                    return;
                }
                found = true;
                break;
            }
        }

        if !found {
            messages::error("Archive does not contain a 'cim' binary.");
            return;
        }

        let new_binary_path = extract_path;

        // Set executable bit on the new binary
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) =
                std::fs::set_permissions(&new_binary_path, std::fs::Permissions::from_mode(0o755))
            {
                messages::error(&format!("Failed to set executable permissions: {}", e));
                return;
            }
        }

        // Rename the old binary to cim.old, then copy the new one into place
        let old_path = exe_path.with_file_name("cim.old");

        if let Err(e) = std::fs::rename(&exe_path, &old_path) {
            messages::error(&format!(
                "Failed to rename current binary to cim.old: {}",
                e
            ));
            return;
        }

        if let Err(e) = std::fs::copy(&new_binary_path, &exe_path) {
            messages::error(&format!(
                "Failed to copy new binary: {}. Old binary preserved at {}",
                e,
                old_path.display()
            ));
            // Attempt to restore the old binary
            let _ = std::fs::rename(&old_path, &exe_path);
            return;
        }

        messages::success(&format!(
            "Successfully updated cim from v{} to v{}",
            current_version, latest_version,
        ));
    }
}

/// Handle utility commands for workspace maintenance
pub(crate) fn handle_utils_command(utils_command: &UtilsCommand) {
    match utils_command {
        UtilsCommand::HashCopyFiles {
            file,
            dry_run,
            verbose,
        } => {
            handle_copy_files_hash_command(file.as_deref(), *dry_run, *verbose);
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
