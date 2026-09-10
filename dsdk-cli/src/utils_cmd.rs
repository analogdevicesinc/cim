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
use crate::install_cmd::{
    install_pip_from_workspace, symlink_resolves_to, venv_exists, VenvManager,
};
use crate::release_cmd::{
    handle_copy_files_hash_command, handle_sync_files_hash_command, handle_toolchains_hash_command,
};
use crate::version::{
    fetch_latest_release_version, find_cim_in_path, is_newer_version, platform_archive_name,
};
use dsdk_cli::config::SdkConfigCore;
use dsdk_cli::messages;
use dsdk_cli::workspace::{
    load_config_with_user_overrides, require_workspace_config, resolve_mirror,
};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
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
        UtilsCommand::Repair { target, yes, force } => {
            handle_utils_repair_command(target, *yes, *force);
        }
    }
}

/// What `cim utils repair -t pip` decided to do, computed before touching the
/// disk so the user can confirm first.
struct PipRepairPlan {
    /// Paths of stale venv state to remove (the workspace venv and/or the
    /// shared mirror `.venv`).
    removals: Vec<PathBuf>,
    /// Whether the workspace venv should be recreated as a symlink into the
    /// mirror (true) or as a real local venv (false) — matching the mode the
    /// workspace was set up in.
    symlink_mode: bool,
    /// Whether a fresh workspace venv must be created after the removals.
    recreate: bool,
    /// Whether any stale state was found at all.
    has_work: bool,
}

/// Entry point for `cim utils repair`.
pub(crate) fn handle_utils_repair_command(target: &str, yes: bool, force: bool) {
    match target {
        "pip" => repair_pip_venv(yes, force),
        unknown => {
            messages::error(&format!(
                "Unknown repair target '{}'. Supported targets: pip (e.g. 'cim utils repair -t pip').",
                unknown
            ));
        }
    }
}

/// Repair the workspace's Python virtual environment state.
///
/// Wipes stale venv state — a leftover `--symlink` symlink in the workspace
/// and/or a broken cached mirror `.venv` — recreates a fresh functional venv in
/// the mode the workspace was originally set up with, and reinstalls the
/// python-dependencies.yml packages so a plain `cim install pip` afterwards is
/// not needed. With `--force` the mirror venv is wiped even when it looks
/// healthy (the "wipe the cache and try again" workflow for a stale-but-
/// undetectable mirror cache).
fn repair_pip_venv(yes: bool, force: bool) {
    let (workspace_path, config_path) = match require_workspace_config() {
        Ok(paths) => paths,
        Err(e) => {
            messages::error(&e);
            return;
        }
    };

    let sdk_config = match load_config_with_user_overrides(&config_path, false) {
        Ok(config) => config,
        Err(e) => {
            messages::error(&format!(
                "Failed to load config file {}: {}",
                config_path.display(),
                e
            ));
            return;
        }
    };

    // Honor a custom `direnv.venv_path` from the manifest, same as
    // `cim install pip`.
    let venv_dir_name = sdk_config
        .direnv()
        .map(|d| d.venv_path_or_default().to_string())
        .unwrap_or_else(|| ".venv".to_string());

    // Resolve the mirror exactly like `cim install pip`/`cim update`
    // (CLI flag > user config > default) so repair and install agree on what
    // the "configured mirror" is. Using a different resolution here would make
    // repair "fix" a venv that plain `cim install pip` still considers stale.
    let mirror_path = resolve_mirror(None);

    let plan = plan_pip_repair(&workspace_path, &mirror_path, &venv_dir_name, force);
    if !plan.has_work {
        // Distinguish "no venv yet" from a healthy shared-mirror (symlink)
        // workspace from a healthy local one so the verdict is informative,
        // never confusing.
        let ws_venv = workspace_path.join(&venv_dir_name);
        match std::fs::symlink_metadata(&ws_venv) {
            Err(_) => {
                messages::status(
                    "No Python virtual environment found in this workspace yet; \
                     run 'cim install pip' to create one.",
                );
            }
            Ok(meta) if meta.file_type().is_symlink() => {
                messages::success(
                    "Python virtual environment state looks healthy (shared mirror venv via symlink), nothing to repair.",
                );
                messages::status(
                    "Plain 'cim install pip' installs into the shared mirror venv. \
                     Use 'cim install pip --force' or 'cim utils repair -t pip -f' to convert to a local venv instead.",
                );
            }
            Ok(_) => {
                messages::success(
                    "Python virtual environment state looks healthy, nothing to repair.",
                );
            }
        }
        return;
    }

    messages::status("The following stale Python virtual environment state will be removed:");
    for path in &plan.removals {
        messages::status(&format!("  - {}", path.display()));
    }

    if !yes {
        print!("Proceed? [y/N]: ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            messages::error("Failed to read confirmation.");
            return;
        }
        let answer = input.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            messages::status("Repair cancelled.");
            return;
        }
    }

    if let Err(e) = execute_pip_repair(&plan, &workspace_path, &mirror_path, &venv_dir_name) {
        messages::error(&e);
        return;
    }

    // Reinstall the workspace's Python packages into the fresh venv so the
    // repair is complete in one step. Only needed when the workspace venv was
    // actually recreated.
    if !plan.recreate {
        return;
    }
    messages::status("");
    match install_pip_from_workspace(
        &workspace_path,
        false, // the venv was just recreated; force would wipe it again
        plan.symlink_mode,
        None, // each file's own default profile
        &mirror_path,
        sdk_config.direnv(),
        None,
    ) {
        Ok(true) => {
            messages::success("Python packages reinstalled.");
        }
        Ok(false) => {
            messages::status(
                "No python-dependencies.yml found in the workspace; the virtual environment was \
                 recreated but no packages were installed.",
            );
        }
        Err(e) => {
            messages::error(&format!(
                "Virtual environment repaired, but reinstalling Python packages failed: {}",
                e
            ));
        }
    }
}

/// Inspect the workspace and mirror venv state and decide what to repair.
///
/// Pure inspection: performs no filesystem changes, so the caller can show the
/// plan and ask for confirmation before anything is removed.
///
/// Only *detectably stale* state is repaired: dangling/broken symlinks,
/// symlinks pointing at a different mirror than configured, and venvs with a
/// missing interpreter. A healthy functional symlink pointing at the expected
/// mirror is left alone unless `force` is set, which always wipes the shared
/// mirror venv (the manual "wipe the cache and try again" workflow).
fn plan_pip_repair(
    workspace_path: &Path,
    mirror_path: &Path,
    venv_dir_name: &str,
    force: bool,
) -> PipRepairPlan {
    let workspace_venv = workspace_path.join(venv_dir_name);
    let mirror_venv = mirror_path.join(".venv");

    let ws_meta = std::fs::symlink_metadata(&workspace_venv).ok();
    let ws_is_symlink = ws_meta.as_ref().is_some_and(|m| m.file_type().is_symlink());
    let ws_functional = venv_exists(&workspace_venv);

    let mirror_meta = std::fs::symlink_metadata(&mirror_venv).ok();
    let mirror_is_dir = mirror_meta.as_ref().is_some_and(|m| m.file_type().is_dir());
    let mirror_functional = venv_exists(&mirror_venv);

    // A functional symlink that does not resolve to the currently configured
    // mirror venv is stale (e.g. the user moved/changed the mirror since the
    // last `--symlink` install).
    let points_at_expected_mirror = symlink_resolves_to(&workspace_venv, &mirror_venv);

    let mut removals = Vec::new();
    let mut symlink_mode = false;
    let mut workspace_removed = false;

    if ws_is_symlink {
        symlink_mode = true;
        if force || !ws_functional || !points_at_expected_mirror {
            removals.push(workspace_venv);
            workspace_removed = true;
            // Only wipe the configured mirror venv if it is itself broken (or
            // `--force` asked for a full wipe) — the workspace symlink's own
            // health says nothing about a mirror it may not even point at
            // (e.g. a dangling symlink left over from a different, since-
            // reconfigured mirror), and that mirror may still be a healthy
            // cache shared by other workspaces.
            if mirror_meta.is_some() && (force || !mirror_functional) {
                removals.push(mirror_venv);
            }
        }
    } else if ws_meta.is_some() && !ws_functional {
        // Broken/incomplete direct venv (missing interpreter, half-built).
        removals.push(workspace_venv);
        workspace_removed = true;
        if force || (mirror_is_dir && !mirror_functional) {
            removals.push(mirror_venv);
        }
    } else if mirror_is_dir && (force || !mirror_functional) {
        // Healthy local venv, but the shared cache is broken (or `--force`
        // asked for a full wipe) — safe to clean up.
        removals.push(mirror_venv);
    }

    let has_work = !removals.is_empty();
    PipRepairPlan {
        removals,
        symlink_mode,
        recreate: workspace_removed,
        has_work,
    }
}

/// Apply a previously confirmed repair plan: remove the stale state and
/// recreate a fresh, functional venv in the workspace's original mode.
fn execute_pip_repair(
    plan: &PipRepairPlan,
    workspace_path: &Path,
    mirror_path: &Path,
    venv_dir_name: &str,
) -> Result<(), String> {
    for path in &plan.removals {
        match std::fs::symlink_metadata(path) {
            Ok(meta) => {
                let result = if meta.file_type().is_symlink() || meta.file_type().is_file() {
                    std::fs::remove_file(path)
                } else {
                    std::fs::remove_dir_all(path)
                };
                if let Err(e) = result {
                    return Err(format!("Failed to remove {}: {}", path.display(), e));
                }
                messages::status(&format!("Removed {}", path.display()));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Failed to inspect {}: {}", path.display(), e)),
        }
    }

    if !plan.recreate {
        return Ok(());
    }

    let manager = VenvManager::new(workspace_path.to_path_buf(), mirror_path.to_path_buf())
        .with_venv_dir_name(venv_dir_name.to_string());
    manager
        .create_venv(true, plan.symlink_mode)
        .map_err(|e| format!("Failed to recreate virtual environment: {}", e))?;

    messages::success(if plan.symlink_mode {
        "Fresh virtual environment created (symlinked into the mirror)"
    } else {
        "Fresh virtual environment created in the workspace"
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_temp_dir() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let path = temp_dir.path().to_path_buf();
        (temp_dir, path)
    }

    /// Build a minimal functional venv layout (venv dir with bin/python3).
    fn make_functional_venv(venv_dir: &Path) {
        let bin = crate::install_cmd::get_venv_bin_dir(venv_dir);
        fs::create_dir_all(&bin).expect("Failed to create venv bin dir");
        fs::write(crate::install_cmd::get_venv_python_path(venv_dir), "")
            .expect("Failed to create venv python");
    }

    /// Build a broken venv layout (dir exists, no interpreter).
    fn make_broken_venv(venv_dir: &Path) {
        fs::create_dir_all(venv_dir).expect("Failed to create broken venv dir");
    }

    #[test]
    fn test_plan_pip_repair_healthy_direct_venv_is_noop() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        make_functional_venv(&ws.join(".venv"));

        let plan = plan_pip_repair(&ws, &mirror, ".venv", false);
        assert!(!plan.has_work);
        assert!(plan.removals.is_empty());
        assert!(!plan.recreate);
    }

    #[test]
    fn test_plan_pip_repair_healthy_symlink_is_noop() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        let mirror_venv = mirror.join(".venv");
        make_functional_venv(&mirror_venv);
        std::os::unix::fs::symlink(&mirror_venv, ws.join(".venv")).expect("symlink fixture");

        // A functional symlink pointing at the expected mirror venv is healthy.
        let plan = plan_pip_repair(&ws, &mirror, ".venv", false);
        assert!(!plan.has_work);
        assert!(plan.removals.is_empty());
    }

    #[test]
    fn test_plan_pip_repair_force_wipes_healthy_symlink_and_mirror() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        let mirror_venv = mirror.join(".venv");
        make_functional_venv(&mirror_venv);
        std::os::unix::fs::symlink(&mirror_venv, ws.join(".venv")).expect("symlink fixture");

        let plan = plan_pip_repair(&ws, &mirror, ".venv", true);
        assert!(plan.has_work);
        assert_eq!(
            plan.removals.len(),
            2,
            "expected both the workspace symlink and the mirror venv to be removed"
        );
        assert!(plan.symlink_mode);
        assert!(plan.recreate);
    }

    #[test]
    fn test_plan_pip_repair_dangling_symlink_is_stale() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        // Symlink points at a mirror venv that does not exist -> dangling.
        std::os::unix::fs::symlink(mirror.join(".venv"), ws.join(".venv"))
            .expect("symlink fixture");

        let plan = plan_pip_repair(&ws, &mirror, ".venv", false);
        assert!(plan.has_work);
        assert_eq!(plan.removals.len(), 1, "expected just the dangling symlink");
        assert!(plan.symlink_mode);
        assert!(plan.recreate);
    }

    #[test]
    fn test_plan_pip_repair_symlink_to_wrong_mirror_is_stale() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        let (_t3, other) = create_temp_dir();
        let other_venv = other.join(".venv");
        make_functional_venv(&other_venv);
        std::os::unix::fs::symlink(&other_venv, ws.join(".venv")).expect("symlink fixture");

        let plan = plan_pip_repair(&ws, &mirror, ".venv", false);
        assert!(plan.has_work);
        assert_eq!(
            plan.removals.len(),
            1,
            "expected just the misdirected symlink"
        );
        assert!(plan.symlink_mode);
        assert!(plan.recreate);
    }

    #[test]
    fn test_plan_pip_repair_symlink_to_wrong_mirror_also_wipes_broken_configured_mirror() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        let (_t3, other) = create_temp_dir();
        let other_venv = other.join(".venv");
        make_functional_venv(&other_venv);
        std::os::unix::fs::symlink(&other_venv, ws.join(".venv")).expect("symlink fixture");
        // The currently configured mirror (distinct from what the symlink
        // actually points at) is itself broken, so it should be wiped too.
        make_broken_venv(&mirror.join(".venv"));

        let plan = plan_pip_repair(&ws, &mirror, ".venv", false);
        assert!(plan.has_work);
        assert_eq!(
            plan.removals,
            vec![ws.join(".venv"), mirror.join(".venv")],
            "both the misdirected symlink and the broken configured mirror should be removed"
        );
    }

    #[test]
    fn test_plan_pip_repair_dangling_symlink_spares_healthy_unrelated_mirror() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        // The configured mirror venv is healthy and possibly shared by other
        // workspaces...
        make_functional_venv(&mirror.join(".venv"));
        // ...but this workspace's symlink is dangling, pointing at some other,
        // now-gone location unrelated to the configured mirror.
        let (_t3, gone) = create_temp_dir();
        std::os::unix::fs::symlink(gone.join(".venv"), ws.join(".venv")).expect("symlink fixture");

        let plan = plan_pip_repair(&ws, &mirror, ".venv", false);
        assert!(plan.has_work);
        assert_eq!(
            plan.removals,
            vec![ws.join(".venv")],
            "the healthy, unrelated configured mirror venv must not be wiped"
        );
    }

    #[test]
    fn test_plan_pip_repair_broken_direct_venv() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        make_broken_venv(&ws.join(".venv"));

        let plan = plan_pip_repair(&ws, &mirror, ".venv", false);
        assert!(plan.has_work);
        assert_eq!(plan.removals.len(), 1);
        assert!(!plan.symlink_mode);
        assert!(plan.recreate);
    }

    #[test]
    fn test_plan_pip_repair_broken_mirror_cache_with_healthy_direct_venv() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        make_functional_venv(&ws.join(".venv"));
        make_broken_venv(&mirror.join(".venv"));

        // Healthy local venv; only the broken shared cache is cleaned up.
        let plan = plan_pip_repair(&ws, &mirror, ".venv", false);
        assert!(plan.has_work);
        assert_eq!(plan.removals.len(), 1);
        assert!(!plan.recreate, "healthy direct venv must not be recreated");
    }

    #[test]
    fn test_execute_pip_repair_stale_symlink_recreates_symlink_mode() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        // Broken mirror venv with a symlink in the workspace pointing at it.
        let mirror_venv = mirror.join(".venv");
        make_broken_venv(&mirror_venv);
        std::os::unix::fs::symlink(&mirror_venv, ws.join(".venv")).expect("symlink fixture");

        let plan = plan_pip_repair(&ws, &mirror, ".venv", false);
        assert!(plan.has_work);
        execute_pip_repair(&plan, &ws, &mirror, ".venv").expect("repair should succeed");

        // Workspace .venv must be a fresh symlink pointing at a functional mirror venv.
        let meta = fs::symlink_metadata(ws.join(".venv")).expect("workspace .venv exists");
        assert!(meta.file_type().is_symlink());
        assert!(venv_exists(&ws.join(".venv")));
        assert!(venv_exists(&mirror.join(".venv")));
    }

    #[test]
    fn test_execute_pip_repair_broken_direct_venv_recreates_direct() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        make_broken_venv(&ws.join(".venv"));

        let plan = plan_pip_repair(&ws, &mirror, ".venv", false);
        execute_pip_repair(&plan, &ws, &mirror, ".venv").expect("repair should succeed");

        let meta = fs::symlink_metadata(ws.join(".venv")).expect("workspace .venv exists");
        assert!(!meta.file_type().is_symlink());
        assert!(venv_exists(&ws.join(".venv")));
    }

    #[test]
    fn test_execute_pip_repair_compiles_a_healthy_mirror_cleanup() {
        let (_t, ws) = create_temp_dir();
        let (_t2, mirror) = create_temp_dir();
        make_functional_venv(&ws.join(".venv"));
        make_broken_venv(&mirror.join(".venv"));

        let plan = plan_pip_repair(&ws, &mirror, ".venv", false);
        execute_pip_repair(&plan, &ws, &mirror, ".venv").expect("repair should succeed");

        // The healthy direct venv must be untouched, broken mirror cache gone.
        assert!(venv_exists(&ws.join(".venv")));
        assert!(!mirror.join(".venv").exists());
    }
}
