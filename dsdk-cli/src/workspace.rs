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

use crate::{config, git_operations, messages};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Filename for the main SDK configuration manifest
pub const SDK_CONFIG_FILE: &str = "sdk.yml";
/// Filename for OS/system dependency definitions
pub const OS_DEPS_FILE: &str = "os-dependencies.yml";
/// Filename for Python dependency definitions
pub const PYTHON_DEPS_FILE: &str = "python-dependencies.yml";
/// Filename for the workspace marker
pub const WORKSPACE_MARKER_FILE: &str = ".workspace";

/// Get the user's home directory path.
///
/// Tries `HOME` first (Unix/macOS), falls back to `USERPROFILE` (Windows).
/// Returns `None` if neither variable is set.
pub fn get_home_dir() -> Option<PathBuf> {
    env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

/// Hardcoded fallback for default manifest source (no config file I/O)
fn default_source_fallback() -> String {
    let home_path = get_home_dir().unwrap_or_else(|| PathBuf::from("."));
    let devel_dir = home_path.join("devel");

    // Check for new path: $HOME/devel/cim-manifests
    let new_path = devel_dir.join("cim-manifests");
    if new_path.exists() {
        return new_path.to_string_lossy().to_string();
    }

    // Check for legacy path: $HOME/devel/sdk-manager-manifests
    let legacy_path = devel_dir.join("sdk-manager-manifests");
    if legacy_path.exists() {
        messages::verbose(&format!(
            "Using legacy manifest location: {}",
            legacy_path.display()
        ));
        messages::verbose("Consider migrating to: $HOME/devel/cim-manifests");
        return legacy_path.to_string_lossy().to_string();
    }

    // Neither exists, return new path as default (for error messages)
    new_path.to_string_lossy().to_string()
}

/// Get the default manifest source location, considering user config
pub fn get_default_source() -> String {
    if let Ok(Some(user_config)) = config::UserConfig::load() {
        if let Some(ref default_source) = user_config.default_source {
            return default_source.clone();
        }
    }
    default_source_fallback()
}

/// Get all manifest sources (default + alternates), deduplicated.
/// Accepts a pre-loaded UserConfig to avoid redundant file I/O.
pub fn get_all_sources_from_config(user_config: Option<&config::UserConfig>) -> Vec<String> {
    let default = match user_config.and_then(|uc| uc.default_source.as_ref()) {
        Some(ds) => expand_env_vars(ds),
        None => default_source_fallback(),
    };
    let alternates = user_config
        .and_then(|uc| uc.alternate_sources.as_ref())
        .cloned()
        .unwrap_or_default();
    let mut sources = vec![default.clone()];
    for alt in alternates {
        let cleaned = expand_env_vars(alt.url.trim());
        if !cleaned.is_empty() && cleaned != default {
            sources.push(cleaned);
        }
    }
    sources
}

/// Get all manifest sources (default + alternates), deduplicated.
/// Convenience wrapper that loads UserConfig internally.
pub fn get_all_sources() -> Vec<String> {
    let uc = config::UserConfig::load().ok().flatten();
    get_all_sources_from_config(uc.as_ref())
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WorkspaceMarker {
    pub workspace_version: String,
    pub created_at: String,
    pub config_file: String,
    pub target: String,
    pub target_version: String,
    pub config_sha256: String,
    pub mirror_path: String,
    pub cim_version: String,
    pub cim_sha256: String,
    pub cim_commit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_mirror: Option<bool>,
    /// Directory containing the original config file (for resolving relative copy_files paths)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_source_dir: Option<String>,
    /// Regex pattern used to filter repositories during init (--match option)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_pattern: Option<String>,
}

/// Find the workspace root by walking up directories looking for .workspace marker
pub fn find_workspace_root() -> Option<PathBuf> {
    let mut current = env::current_dir().ok()?;

    loop {
        let marker_path = current.join(WORKSPACE_MARKER_FILE);
        if marker_path.exists() {
            return Some(current);
        }

        if !current.pop() {
            break;
        }
    }

    None
}

/// Parameters for creating a workspace marker
pub struct CreateWorkspaceMarkerParams<'a> {
    pub workspace_path: &'a Path,
    pub config_name: &'a str,
    pub original_config_path: &'a Path,
    pub mirror_path: &'a Path,
    pub original_identifier: Option<&'a str>,
    pub target_version: Option<&'a str>,
    pub skip_mirror: bool,
    pub source_url: Option<&'a str>,
    pub match_pattern: Option<&'a str>,
}

/// Create workspace marker file
pub fn create_workspace_marker(
    params: CreateWorkspaceMarkerParams,
) -> Result<(), Box<dyn std::error::Error>> {
    // Calculate SHA256 of the original config file
    let config_content = fs::read(params.original_config_path)?;
    let mut hasher = Sha256::new();
    hasher.update(&config_content);
    let config_sha256 = format!("{:x}", hasher.finalize());

    let target_name = params.original_identifier.unwrap_or_else(|| {
        params
            .original_config_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
    });

    // Get Code in Motion version information
    let cim_version = env!("CARGO_PKG_VERSION").to_string();
    let cim_commit = env!("GIT_HASH").to_string();
    let cim_sha256 = match std::env::current_exe() {
        Ok(exe_path) => match std::fs::read(&exe_path) {
            Ok(binary_data) => {
                let mut h = Sha256::new();
                h.update(&binary_data);
                format!("{:x}", h.finalize())
            }
            Err(_) => "unknown".to_string(),
        },
        Err(_) => "unknown".to_string(),
    };

    // Get the source directory of the original config file
    let config_source_dir = if let Some(url) = params.source_url {
        // For remote git sources, store the URL so sync-copy-files can re-clone
        Some(url.to_string())
    } else {
        params
            .original_config_path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
    };

    let marker_path = params.workspace_path.join(WORKSPACE_MARKER_FILE);
    let marker = WorkspaceMarker {
        workspace_version: "1".to_string(),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string(),
        config_file: params.config_name.to_string(),
        target: target_name.to_string(),
        target_version: params.target_version.unwrap_or("latest").to_string(),
        config_sha256,
        mirror_path: params.mirror_path.to_string_lossy().to_string(),
        cim_version,
        cim_sha256,
        cim_commit,
        no_mirror: if params.skip_mirror { Some(true) } else { None },
        config_source_dir,
        match_pattern: params.match_pattern.map(|s| s.to_string()),
    };

    fs::write(&marker_path, serde_yaml::to_string(&marker)?)?;
    messages::verbose(&format!(
        "Created workspace marker: {}",
        marker_path.display()
    ));
    Ok(())
}

/// Update the match_pattern field in an existing workspace marker file.
/// Pass `None` to clear the stored match pattern.
pub fn update_workspace_marker_match_pattern(
    workspace_path: &Path,
    match_pattern: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let marker_path = workspace_path.join(WORKSPACE_MARKER_FILE);
    let content = fs::read_to_string(&marker_path)?;
    let mut marker: WorkspaceMarker = serde_yaml::from_str(&content)?;
    marker.match_pattern = match_pattern.map(|s| s.to_string());
    fs::write(&marker_path, serde_yaml::to_string(&marker)?)?;
    Ok(())
}

/// Recursively copy all files and directories from src to dst
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Create destination directory if it doesn't exist
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    // Iterate over directory entries
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dst.join(&file_name);

        if path.is_dir() {
            // Recursively copy subdirectories
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            // Copy files
            fs::copy(&path, &dest_path)?;
        }
    }

    Ok(())
}

/// Resolve target config from git repository with optional version checkout
///
/// # Arguments
///
/// * `git_source` - Git repository URL or local path
/// * `target` - Target name to extract
/// * `version` - Optional version (branch/tag) to checkout
/// * `persistent_dir` - Optional directory to extract files to. If None, uses tempfile with mem::forget
///
/// When `persistent_dir` is Some, files are extracted there and caller is responsible for cleanup.
/// When `persistent_dir` is None, uses tempfile and mem::forget to keep files alive for process lifetime.
pub fn resolve_target_config_from_git(
    git_source: &str,
    target: &str,
    version: Option<&str>,
    persistent_dir: Option<&Path>,
) -> Result<PathBuf, anyhow::Error> {
    // Create a temporary directory for clone
    let temp_dir = tempfile::tempdir()
        .map_err(|e| anyhow::anyhow!("Failed to create temp directory: {}", e))?;

    let temp_path = temp_dir.path();

    // Clone the repository (shallow if no version specified, full if version needed)
    let clone_result = if version.is_some() {
        // For version checkout, we need full history to access branches/tags
        git_operations::clone_repo(git_source, temp_path, None)?
    } else {
        // For main branch, use shallow clone
        if is_url(git_source) {
            git_operations::clone_repo_shallow(git_source, temp_path, 1)?
        } else {
            // For local repos without version, still do full clone since shallow clone of local repos is not very useful
            git_operations::clone_repo(git_source, temp_path, None)?
        }
    };

    if !clone_result.is_success() {
        return Err(anyhow::anyhow!("Git clone failed: {}", clone_result.stderr));
    }

    // Checkout specific version if requested
    if let Some(v) = version {
        let checkout_result = git_operations::checkout(temp_path, v)?;

        if !checkout_result.is_success() {
            return Err(anyhow::anyhow!(
                "Git checkout of version '{}' failed: {}",
                v,
                checkout_result.stderr
            ));
        }
    }

    // Check if target exists and has sdk.yml
    let target_dir = temp_path.join("targets").join(target);
    let config_path = target_dir.join(SDK_CONFIG_FILE);

    if !config_path.exists() {
        return Err(anyhow::anyhow!(
            "Target '{}' not found or missing sdk.yml in repository{}",
            target,
            if let Some(v) = version {
                format!(" at version '{}'", v)
            } else {
                String::new()
            }
        ));
    }

    // Extract to persistent directory or create a temporary one
    let (extraction_path, temp_dir_keeper) = if let Some(persist_dir) = persistent_dir {
        // Use provided persistent directory
        let target_extract_dir = persist_dir.join(target);
        if target_extract_dir.exists() {
            // Clean up existing directory
            fs::remove_dir_all(&target_extract_dir).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to remove existing directory {}: {}",
                    target_extract_dir.display(),
                    e
                )
            })?;
        }
        fs::create_dir_all(&target_extract_dir).map_err(|e| {
            anyhow::anyhow!(
                "Failed to create directory {}: {}",
                target_extract_dir.display(),
                e
            )
        })?;
        (target_extract_dir, None)
    } else {
        // Create a persistent temporary directory for init command
        let persistent_temp_dir = tempfile::tempdir()
            .map_err(|e| anyhow::anyhow!("Failed to create persistent temp directory: {}", e))?;
        let persistent_path = persistent_temp_dir.path().to_path_buf();
        (persistent_path, Some(persistent_temp_dir))
    };

    // Recursively copy all files and directories from target directory to extraction path
    // This ensures that any files referenced in copy_files are available, including
    // files in subdirectories like patches/qemu/*.patch
    copy_dir_recursive(&target_dir, &extraction_path)
        .map_err(|e| anyhow::anyhow!("Failed to copy target directory contents: {}", e))?;

    let persistent_config = extraction_path.join(SDK_CONFIG_FILE);

    // If using tempfile, keep the directory alive by forgetting it (init command behavior)
    if let Some(temp_keeper) = temp_dir_keeper {
        std::mem::forget(temp_keeper);
    }

    Ok(persistent_config)
}

/// Resolve the config_source_dir from the workspace marker
///
/// If the config_source_dir is a URL, re-clones the repo into a fresh TempDir
/// Returns the directory path and the TempDir (if created) to keep it alive
pub fn resolve_config_source_dir_from_marker(
    workspace_path: &Path,
    config_path: &Path,
) -> (PathBuf, Option<tempfile::TempDir>) {
    let marker_path = workspace_path.join(WORKSPACE_MARKER_FILE);
    if !marker_path.exists() {
        return (workspace_path.to_path_buf(), None);
    }

    let content = match fs::read_to_string(&marker_path) {
        Ok(c) => c,
        Err(_) => return (workspace_path.to_path_buf(), None),
    };

    let marker = match serde_yaml::from_str::<WorkspaceMarker>(&content) {
        Ok(m) => m,
        Err(_) => return (workspace_path.to_path_buf(), None),
    };

    if let Some(ref source_dir_str) = marker.config_source_dir {
        if is_url(source_dir_str) {
            // Remote git source: re-clone to a fresh temp dir
            let temp_dir = match tempfile::tempdir() {
                Ok(d) => d,
                Err(_) => return (workspace_path.to_path_buf(), None),
            };
            let version = if marker.target_version == "latest" {
                None
            } else {
                Some(marker.target_version.as_str())
            };
            match resolve_target_config_from_git(
                source_dir_str,
                &marker.target,
                version,
                Some(temp_dir.path()),
            ) {
                Ok(config_file_path) => {
                    let dir = config_file_path
                        .parent()
                        .unwrap_or(temp_dir.path())
                        .to_path_buf();
                    return (dir, Some(temp_dir));
                }
                Err(e) => {
                    messages::error(&format!("Failed to fetch remote source for sync: {}", e));
                    return (workspace_path.to_path_buf(), None);
                }
            }
        } else {
            // Local path: use if it exists
            let p = PathBuf::from(source_dir_str);
            if p.exists() {
                return (p, None);
            }
        }
    }

    // Fall back to canonicalized config path parent
    config_path
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .map(|p| (p, None))
        .unwrap_or_else(|| (workspace_path.to_path_buf(), None))
}

/// Get workspace root and validate it's a proper workspace
pub fn get_current_workspace() -> Result<PathBuf, String> {
    find_workspace_root().ok_or_else(|| {
        "Not in a workspace. Run 'cim init' first to create a workspace.".to_string()
    })
}

/// Resolve the current workspace and verify that sdk.yml exists.
///
/// Returns the workspace path and the config file path. This is a convenience
/// function that combines the common pattern of getting the workspace, constructing
/// the config path, and checking that it exists.
pub fn require_workspace_config() -> Result<(PathBuf, PathBuf), String> {
    let workspace_path = get_current_workspace()?;
    let config_path = workspace_path.join(SDK_CONFIG_FILE);
    if !config_path.exists() {
        return Err(format!(
            "sdk.yml not found in workspace root: {}. \
             The workspace may be corrupted. Try running 'cim init' to reinitialize.",
            workspace_path.display()
        ));
    }
    Ok((workspace_path, config_path))
}

/// Check if a string represents a URL (http://, https://, or git@ SSH)
pub fn is_url(input: &str) -> bool {
    input.starts_with("http://") || input.starts_with("https://") || input.starts_with("git@")
}

/// Expand environment variables in a path string
///
/// This function expands environment variables in path strings.
/// It supports Unix-style $VAR and ${VAR}, Windows-style %VAR%, and tilde (~) expansion.
/// If an environment variable is not found, it leaves the variable reference unchanged.
///
/// # Examples
///
/// ```text
/// // Unix style (HOME=/Users/alice):
/// expand_env_vars("$HOME/workspace") => "/Users/alice/workspace"
/// expand_env_vars("${HOME}/tmp/mirror") => "/Users/alice/tmp/mirror"
/// expand_env_vars("~/workspace") => "/Users/alice/workspace"
///
/// // Windows style (USERPROFILE=C:\Users\alice):
/// expand_env_vars("%USERPROFILE%\\workspace") => "C:\\Users\\alice\\workspace"
/// expand_env_vars("%HOME%/workspace") => "C:\\Users\\alice/workspace"
/// ```
pub fn expand_env_vars(path: &str) -> String {
    expand_env_vars_with_overrides(path, &std::collections::HashMap::new())
}

/// Expand environment variables in a path string, with caller-supplied overrides.
///
/// Overrides are checked before the process environment. This allows callers to
/// inject context-specific variables (e.g. PWD, WORKSPACE) that shadow any
/// real environment variable with the same name.
///
/// Supports Unix-style `$VAR` and `${VAR}`, Windows-style `%VAR%`, and tilde
/// (`~`) expansion.  Unknown variables are left unchanged.
pub fn expand_env_vars_with_overrides(
    path: &str,
    overrides: &std::collections::HashMap<&str, &str>,
) -> String {
    let mut result = path.to_string();

    /// Resolve a variable name: check overrides first, then env, with HOME fallback.
    fn resolve_var(
        name: &str,
        overrides: &std::collections::HashMap<&str, &str>,
    ) -> Option<String> {
        if let Some(&val) = overrides.get(name) {
            return Some(val.to_string());
        }
        if name == "HOME" {
            env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok()
        } else {
            env::var(name).ok()
        }
    }

    // Handle tilde expansion first
    if result.starts_with("~/") || result == "~" {
        if let Some(home) = resolve_var("HOME", overrides) {
            if result == "~" {
                result = home;
            } else {
                // Use PathBuf to handle path separators correctly across platforms
                let rest_of_path = &result[2..]; // Skip "~/"
                result = PathBuf::from(home)
                    .join(rest_of_path)
                    .to_string_lossy()
                    .to_string();
            }
        }
    }

    // Handle Windows %VAR% syntax
    while let Some(start) = result.find('%') {
        if let Some(end) = result[start + 1..].find('%') {
            let var_name = &result[start + 1..start + 1 + end].to_string();
            if let Some(value) = resolve_var(var_name, overrides) {
                result.replace_range(start..start + end + 2, &value);
            } else {
                // If variable not found, break to avoid infinite loop
                break;
            }
        } else {
            break;
        }
    }

    // Handle ${VAR} syntax
    while let Some(start) = result.find("${") {
        if let Some(end) = result[start..].find('}') {
            let var_name = &result[start + 2..start + end].to_string();
            if let Some(value) = resolve_var(var_name, overrides) {
                result.replace_range(start..start + end + 1, &value);
            } else {
                // If variable not found, break to avoid infinite loop
                break;
            }
        } else {
            break;
        }
    }

    // Handle $VAR syntax (without braces)
    let mut chars: Vec<char> = result.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() {
            // Find the end of the variable name
            let var_start = i + 1;
            let mut var_end = var_start;

            while var_end < chars.len()
                && (chars[var_end].is_alphanumeric() || chars[var_end] == '_')
            {
                var_end += 1;
            }

            if var_end > var_start {
                let var_name: String = chars[var_start..var_end].iter().collect();
                if let Some(value) = resolve_var(&var_name, overrides) {
                    // Replace $VAR with the actual value
                    let replacement: Vec<char> = value.chars().collect();
                    chars.splice(i..var_end, replacement);
                    i += value.len();
                } else {
                    i = var_end;
                }
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    chars.into_iter().collect()
}

/// Return `true` if `s` contains a `$` that is **not** part of a `${{ … }}`
/// manifest-variable reference.
///
/// `${{ VAR }}` patterns are intentionally kept as-is so that Makefile
/// generation can later convert them to `$(VAR)` Make references.  They must
/// not trigger the "unresolved host env var" warning.
///
/// # Examples
///
/// ```
/// use dsdk_cli::workspace::has_unresolved_env_var_refs;
///
/// assert!(!has_unresolved_env_var_refs("https://example.com"));
/// assert!(!has_unresolved_env_var_refs("${{ WORKSPACE }}/bin"));
/// assert!(!has_unresolved_env_var_refs("${{ A }}/${{ B }}"));
/// assert!(has_unresolved_env_var_refs("$UNSET_HOST_VAR/path"));
/// assert!(has_unresolved_env_var_refs("${{ WORKSPACE }}/$UNSET"));
/// ```
pub fn has_unresolved_env_var_refs(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            // Check for ${{ … }} pattern — skip it entirely.
            if bytes.get(i + 1) == Some(&b'{') && bytes.get(i + 2) == Some(&b'{') {
                if let Some(rel) = s[i + 3..].find("}}") {
                    i = i + 3 + rel + 2;
                    continue;
                }
            }
            // A bare `$` that is not part of a manifest-variable reference.
            return true;
        }
        i += 1;
    }
    false
}

/// Resolve manifest variable values by expanding host env vars within them.
///
/// Each value in the raw map is passed through [`expand_env_vars`]. If a value
/// still contains a `$` character after expansion — and that `$` is **not**
/// part of a `${{ VAR }}` manifest-variable reference — a warning is logged so
/// the user knows the variable is unresolved.
///
/// `${{ VAR }}` patterns are left in place intentionally: Makefile generation
/// will convert them to `$(VAR)` Make references at a later stage.
///
/// # Example
///
/// ```text
/// variables:
///   DOCKER_DEFAULT_PLATFORM: $HOST_DEFAULT_PLATFORM    # resolved from env
///   SDK_BASE_URL: https://example.com/sdk               # literal
///   TOOLCHAIN_PATH: ${{ WORKSPACE }}/toolchains/bin    # passed to Make
/// ```
pub fn resolve_variables(
    raw: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    raw.iter()
        .map(|(key, value)| {
            let resolved = expand_env_vars(value);
            if has_unresolved_env_var_refs(&resolved) {
                messages::info(&format!(
                    "Warning: manifest variable '{}' has an unresolved reference in value: '{}'",
                    key, resolved
                ));
            }
            (key.clone(), resolved)
        })
        .collect()
}

/// Convert a git entry name to its `_DIR` variable name.
///
/// Rules:
/// - Take the last path component (e.g., "zephyrproject/zephyr" -> "zephyr")
/// - Uppercase the result
/// - Replace hyphens and dots with underscores
/// - Append `_DIR`
///
/// # Examples
///
/// ```
/// use dsdk_cli::workspace::git_name_to_dir_var;
///
/// assert_eq!(git_name_to_dir_var("u-boot"), "U_BOOT_DIR");
/// assert_eq!(git_name_to_dir_var("linux-stable"), "LINUX_STABLE_DIR");
/// assert_eq!(git_name_to_dir_var("zephyrproject/zephyr"), "ZEPHYR_DIR");
/// ```
pub fn git_name_to_dir_var(name: &str) -> String {
    let component = name.rsplit('/').next().unwrap_or(name);
    let upper = component.to_uppercase();
    let sanitized = upper.replace(['-', '.'], "_");
    format!("{}_DIR", sanitized)
}

/// Per-git Python virtual environment path: `<workspace>/.cim/<git-name>/.venv`.
///
/// Each git entry that declares `python-deps` gets its own isolated venv so
/// that repositories needing conflicting versions of the same package can
/// coexist in one workspace. The full git `name` (including any `/`) is used
/// for the path so that nested names like `zephyrproject/zephyr` map to a
/// stable, collision-free location.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use dsdk_cli::workspace::git_venv_path;
///
/// let ws = Path::new("/ws");
/// assert_eq!(git_venv_path(ws, "u-boot"), Path::new("/ws/.cim/u-boot/.venv"));
/// assert_eq!(
///     git_venv_path(ws, "zephyrproject/zephyr"),
///     Path::new("/ws/.cim/zephyrproject/zephyr/.venv")
/// );
/// ```
pub fn git_venv_path(workspace: &Path, git_name: &str) -> PathBuf {
    workspace.join(".cim").join(git_name).join(".venv")
}

/// Convert a git entry name to its `_VENV` variable name.
///
/// Uses the same rules as [`git_name_to_dir_var`] (last path component,
/// uppercased, hyphens and dots replaced with underscores) but appends
/// `_VENV`. This variable points at [`git_venv_path`] and is emitted into the
/// generated Makefile so `.mk` fragments can activate the right venv with
/// `. $(ZEPHYR_VENV)/bin/activate`.
///
/// # Examples
///
/// ```
/// use dsdk_cli::workspace::git_name_to_venv_var;
///
/// assert_eq!(git_name_to_venv_var("u-boot"), "U_BOOT_VENV");
/// assert_eq!(git_name_to_venv_var("linux-stable"), "LINUX_STABLE_VENV");
/// assert_eq!(git_name_to_venv_var("zephyrproject/zephyr"), "ZEPHYR_VENV");
/// ```
pub fn git_name_to_venv_var(name: &str) -> String {
    let component = name.rsplit('/').next().unwrap_or(name);
    let upper = component.to_uppercase();
    let sanitized = upper.replace(['-', '.'], "_");
    format!("{}_VENV", sanitized)
}

/// Generate auto-derived `_DIR` variables from git entries.
///
/// Returns a map of variable names (e.g., `U_BOOT_DIR`) to their raw values
/// (e.g., `${{ WORKSPACE }}/u-boot`). The values use the `${{ WORKSPACE }}`
/// placeholder so they can be expanded by [`expand_manifest_vars`] or rendered
/// into Make syntax by `render_command_for_makefile`.
///
/// User-defined variables (from the `variables:` section) take precedence:
/// if a user already defines `U_BOOT_DIR`, the auto-generated one is skipped.
///
/// Returns `Err` with a descriptive message if two git entries produce the
/// same variable name (a naming conflict).
pub fn generate_git_dir_vars(
    gits: &[config::GitConfig],
    user_vars: Option<&std::collections::HashMap<String, String>>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let mut dir_vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for git in gits {
        let var_name = git_name_to_dir_var(&git.name);

        if let Some(uv) = user_vars {
            if uv.contains_key(&var_name) {
                continue;
            }
        }

        if let Some(existing_value) = dir_vars.get(&var_name) {
            let existing_name = existing_value
                .strip_prefix("${{ WORKSPACE }}/")
                .unwrap_or(existing_value);
            return Err(format!(
                "Git name conflict: '{}' and '{}' both produce variable '{}'. \
                 Resolve by adding an explicit entry in the 'variables:' section.",
                existing_name, git.name, var_name
            ));
        }

        let value = format!("${{{{ WORKSPACE }}}}/{}", git.name);
        dir_vars.insert(var_name, value);
    }

    Ok(dir_vars)
}

/// Generate auto-derived `_VENV` variables for git entries that declare
/// `python-deps`.
///
/// Returns a map of variable names (e.g., `ZEPHYR_VENV`) to their raw values
/// (e.g., `${{ WORKSPACE }}/.cim/zephyrproject/zephyr/.venv`). These point at
/// the per-git virtual environment created by `cim install pip` so that
/// generated Makefile fragments can activate the right venv with
/// `. $(ZEPHYR_VENV)/bin/activate`.
///
/// Only gits with `python-deps` get a variable, to avoid cluttering the
/// Makefile with venvs that are never created. User-defined variables take
/// precedence, mirroring [`generate_git_dir_vars`]. Returns `Err` on a naming
/// conflict between two git entries.
pub fn generate_git_venv_vars(
    gits: &[config::GitConfig],
    user_vars: Option<&std::collections::HashMap<String, String>>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let mut venv_vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for git in gits {
        if git.python_deps.is_none() {
            continue;
        }

        let var_name = git_name_to_venv_var(&git.name);

        if let Some(uv) = user_vars {
            if uv.contains_key(&var_name) {
                continue;
            }
        }

        if venv_vars.contains_key(&var_name) {
            return Err(format!(
                "Git name conflict: '{}' produces an already-used variable '{}'. \
                 Resolve by adding an explicit entry in the 'variables:' section.",
                git.name, var_name
            ));
        }

        let value = format!("${{{{ WORKSPACE }}}}/.cim/{}/.venv", git.name);
        venv_vars.insert(var_name, value);
    }

    Ok(venv_vars)
}

/// Expand `${{ VAR }}` manifest variable references in a string.
///
/// Looks up each `VAR` in the provided resolved variables map.  Unknown
/// variables are left as-is so that the caller can detect them or pass them
/// through to a later stage.
///
/// # Example
///
/// ```
/// use std::collections::HashMap;
/// use dsdk_cli::workspace::expand_manifest_vars;
///
/// let mut vars = HashMap::new();
/// vars.insert("PLATFORM".to_string(), "linux/amd64".to_string());
///
/// let result = expand_manifest_vars("DOCKER_DEFAULT_PLATFORM=${{ PLATFORM }}", &vars);
/// assert_eq!(result, "DOCKER_DEFAULT_PLATFORM=linux/amd64");
///
/// // Unknown variables are left unchanged
/// let result = expand_manifest_vars("path/${{ UNKNOWN }}/file", &vars);
/// assert_eq!(result, "path/${{ UNKNOWN }}/file");
/// ```
pub fn expand_manifest_vars(s: &str, vars: &std::collections::HashMap<String, String>) -> String {
    let mut result = s.to_string();
    // Repeatedly scan for ${{ ... }} patterns
    let mut search_start = 0;
    while let Some(open) = result[search_start..].find("${{") {
        let open_abs = search_start + open;
        let after_open = open_abs + 3; // skip "${{"
        if let Some(close_rel) = result[after_open..].find("}}") {
            let close_abs = after_open + close_rel;
            let var_name = result[after_open..close_abs].trim();
            if let Some(value) = vars.get(var_name) {
                // Substitute the entire "${{ VAR }}" token with the value
                let token_end = close_abs + 2; // skip "}}"
                result.replace_range(open_abs..token_end, value);
                // Next search from the insertion point so chained vars work
                search_start = open_abs + value.len();
            } else {
                // Unknown variable — skip past this token to avoid infinite loop
                search_start = after_open;
            }
        } else {
            break; // No closing "}}", stop searching
        }
    }
    result
}

/// Load SDK config from a path and apply user config overrides if available
///
/// This function encapsulates the common pattern of:
/// 1. Loading SDK config from sdk.yml
/// 2. Loading user config
/// 3. Applying user config overrides to SDK config
/// 4. Expanding environment variables in git repository URLs and manifest vars
///
/// # Arguments
///
/// * `config_path` - Path to the sdk.yml file
/// * `verbose` - Whether to print verbose output about config loading and overrides
///
/// # Returns
///
/// Result containing the SdkConfig with user overrides applied
pub fn load_config_with_user_overrides(
    config_path: &Path,
    verbose: bool,
) -> Result<config::SdkConfig, Box<dyn std::error::Error>> {
    // Load SDK config
    let mut sdk_config = config::load_config(config_path)?;

    // Load and apply user config overrides if present
    match config::UserConfig::load() {
        Ok(Some(user_config)) => {
            if verbose {
                messages::verbose(&format!(
                    "Loaded user config from {}",
                    config::UserConfig::default_path().display()
                ));
            }
            let override_count = user_config.apply_to_sdk_config(&mut sdk_config, verbose);
            if override_count > 0 && verbose {
                messages::verbose(&format!(
                    "Applied {} override(s) from user config",
                    override_count
                ));
            }
        }
        Ok(None) => {
            if verbose {
                messages::verbose("No user config found");
            }
        }
        Err(e) => {
            messages::info(&format!("Warning: Failed to load user config: {}", e));
        }
    }

    // Expand environment variables in git repository URLs
    for git in &mut sdk_config.gits {
        let expanded = expand_env_vars(&git.url);
        if expanded != git.url {
            if verbose {
                messages::verbose(&format!("Expanded git URL: {} -> {}", git.url, expanded));
            }
            git.url = expanded;
        }
    }

    // Expand manifest ${{ VAR }} variables in path/URL fields
    expand_manifest_vars_in_config(&mut sdk_config);

    Ok(sdk_config)
}

/// Expand `${{ VAR }}` manifest variables in all relevant fields of an SdkConfig.
///
/// Expands variables in git URLs, toolchain URLs/destinations/names, and
/// copy_files source/dest fields. The expansion context includes both
/// user-defined variables from the `variables:` section and auto-generated
/// `_DIR` variables derived from git entry names.
pub fn expand_manifest_vars_in_config(sdk_config: &mut config::SdkConfig) {
    let mut vars = if let Some(ref raw_vars) = sdk_config.variables {
        resolve_variables(raw_vars)
    } else {
        std::collections::HashMap::new()
    };

    // Merge auto-generated DIR vars (user-defined take precedence)
    if let Ok(dir_vars) = generate_git_dir_vars(&sdk_config.gits, sdk_config.variables.as_ref()) {
        for (k, v) in dir_vars {
            vars.entry(k).or_insert(v);
        }
    }

    if vars.is_empty() {
        return;
    }

    for git in &mut sdk_config.gits {
        git.url = expand_manifest_vars(&git.url, &vars);
    }

    if let Some(ref mut toolchains) = sdk_config.toolchains {
        for tc in toolchains.iter_mut() {
            if let Some(ref name) = tc.name.clone() {
                tc.name = Some(expand_manifest_vars(name, &vars));
            }
            tc.url = expand_manifest_vars(&tc.url.clone(), &vars);
            tc.destination = expand_manifest_vars(&tc.destination.clone(), &vars);
            if let Some(ref md) = tc.mirror_destination.clone() {
                tc.mirror_destination = Some(expand_manifest_vars(md, &vars));
            }
        }
    }

    if let Some(ref mut copy_files) = sdk_config.copy_files {
        for cf in copy_files.iter_mut() {
            cf.source = expand_manifest_vars(&cf.source.clone(), &vars);
            cf.dest = expand_manifest_vars(&cf.dest.clone(), &vars);
        }
    }
}

/// Resolve the effective mirror cache directory, highest priority first:
///
/// 1. `cli_override` — the `--mirror` flag passed to `init` / `update`.
/// 2. `mirror` in `~/.config/cim/config.toml` (the user config).
/// 3. `manifest_mirror` — the DEPRECATED `mirror:` key in `sdk.yml`. A one-time
///    deprecation warning is printed when this layer is the one used.
/// 4. The built-in default, `config::default_mirror()` (`$HOME/tmp/mirror`).
///
/// Environment variables (e.g. `$HOME`) in the chosen value are expanded.
pub fn resolve_mirror(cli_override: Option<&Path>, manifest_mirror: Option<&Path>) -> PathBuf {
    let raw = if let Some(cli) = cli_override {
        cli.to_path_buf()
    } else if let Ok(Some(user_config)) = config::UserConfig::load() {
        // User config wins over the manifest, mirroring pre-refactor behavior.
        // If the user config exists but has no `mirror`, fall through to the
        // manifest value, then the built-in default.
        match user_config.mirror {
            Some(m) => m,
            None => manifest_mirror
                .map(|m| {
                    warn_manifest_mirror_deprecated();
                    m.to_path_buf()
                })
                .unwrap_or_else(config::default_mirror),
        }
    } else if let Some(m) = manifest_mirror {
        warn_manifest_mirror_deprecated();
        m.to_path_buf()
    } else {
        config::default_mirror()
    };

    PathBuf::from(expand_env_vars(&raw.to_string_lossy()))
}

/// Emit the `sdk.yml` `mirror:` deprecation notice at most once per process.
fn warn_manifest_mirror_deprecated() {
    use std::sync::Once;
    static WARN_ONCE: Once = Once::new();
    WARN_ONCE.call_once(|| {
        messages::info(
            "Deprecation: the `mirror:` key in sdk.yml is deprecated and will be \
             removed in a future release. Set the mirror via `--mirror` or the \
             `mirror` key in ~/.config/cim/config.toml instead.",
        );
    });
}

/// Download a file from URL to a temporary location
pub fn download_config_from_url(url: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    messages::status(&format!("Downloading configuration from {}...", url));

    let response = reqwest::blocking::get(url)?;

    if !response.status().is_success() {
        return Err(format!(
            "HTTP error {}: {}",
            response.status(),
            response.status().canonical_reason().unwrap_or("Unknown")
        )
        .into());
    }

    let content = response.text()?;

    // Create a named temporary file that won't be deleted until the process ends
    let temp_file = tempfile::NamedTempFile::new()?;
    let temp_file_path = temp_file.path().to_path_buf();

    fs::write(&temp_file_path, content)?;

    // Keep the tempfile alive by forgetting it (it will be cleaned up when process exits)
    std::mem::forget(temp_file);

    Ok(temp_file_path)
}

/// Insert a YAML list entry at the end of a top-level section in a YAML document.
///
/// Preserves the original formatting and comments. The `section_key` should be a
/// top-level key (e.g., "gits"). The `entry` should be the pre-formatted YAML text
/// to insert (including any leading newline if desired).
///
/// Returns `Ok(modified_content)` or `Err(message)` if the section is not found.
pub fn insert_yaml_section_entry(
    content: &str,
    section_key: &str,
    entry: &str,
) -> Result<String, String> {
    let key_pattern = format!("{}:", section_key);
    let Some(section_pos) = content.find(&key_pattern) else {
        return Err(format!(
            "Could not find '{}:' section in YAML content",
            section_key
        ));
    };

    let line_end = content[section_pos..]
        .find('\n')
        .map(|pos| section_pos + pos)
        .unwrap_or(content.len());
    let section_line = &content[section_pos..line_end];

    // Handle empty array syntax (e.g., "gits: []")
    if section_line.contains("[]") {
        let replacement = format!("{}:{}", section_key, entry);
        return Ok(format!(
            "{}{}{}",
            &content[..section_pos],
            replacement,
            &content[line_end..]
        ));
    }

    // Find the end of the section by scanning line by line.
    // Only a real YAML key at column 0 (non-whitespace, non-comment) ends the section.
    let key_line_len = content[section_pos..]
        .find('\n')
        .map(|p| p + 1)
        .unwrap_or(content.len() - section_pos);

    let mut insert_pos = section_pos + key_line_len;
    let mut scan_pos = section_pos + key_line_len;

    while scan_pos < content.len() {
        let line_end = content[scan_pos..]
            .find('\n')
            .map(|p| scan_pos + p + 1)
            .unwrap_or(content.len());

        let line = &content[scan_pos..line_end];
        let first_byte = line.bytes().next();

        match first_byte {
            None | Some(b'\n') | Some(b'\r') => {
                // Empty line — skip
            }
            Some(b' ') | Some(b'\t') => {
                // Indented line → belongs to the current section; advance insert_pos
                insert_pos = line_end;
            }
            Some(b'#') => {
                // Comment line → skip; belongs to the next section
            }
            _ => {
                // Non-indented, non-comment line = start of the next YAML key
                break;
            }
        }
        scan_pos = line_end;
    }

    Ok(format!(
        "{}{}{}",
        &content[..insert_pos],
        entry,
        &content[insert_pos..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // Test helper function to create a temporary workspace
    fn create_test_workspace() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let workspace_path = temp_dir.path().to_path_buf();
        (temp_dir, workspace_path)
    }

    #[test]
    fn test_workspace_marker_serialization() {
        let marker = WorkspaceMarker {
            workspace_version: "1".to_string(),
            created_at: "1234567890".to_string(),
            config_file: "sdk.yml".to_string(),
            target: "my-config.yml".to_string(),
            target_version: "v1.0.0".to_string(),
            config_sha256: "abcd1234567890".to_string(),
            mirror_path: "/tmp/mirror".to_string(),
            cim_version: "0.6.2".to_string(),
            cim_sha256: "909481f5438ec71ea6c44712bd62513ec71b02c3a149e8577b997743da5a539c"
                .to_string(),
            cim_commit: "6b4768b7".to_string(),
            no_mirror: Some(true),
            config_source_dir: Some("/path/to/source".to_string()),
            match_pattern: None,
        };

        let serialized = serde_yaml::to_string(&marker).expect("Failed to serialize marker");
        let deserialized: WorkspaceMarker =
            serde_yaml::from_str(&serialized).expect("Failed to deserialize marker");

        assert_eq!(marker.workspace_version, deserialized.workspace_version);
        assert_eq!(marker.created_at, deserialized.created_at);
        assert_eq!(marker.config_file, deserialized.config_file);
        assert_eq!(marker.target, deserialized.target);
        assert_eq!(marker.target_version, deserialized.target_version);
        assert_eq!(marker.config_sha256, deserialized.config_sha256);
        assert_eq!(marker.mirror_path, deserialized.mirror_path);
        assert_eq!(marker.cim_version, deserialized.cim_version);
        assert_eq!(marker.cim_sha256, deserialized.cim_sha256);
        assert_eq!(marker.cim_commit, deserialized.cim_commit);
        assert_eq!(marker.no_mirror, deserialized.no_mirror);
        assert_eq!(marker.config_source_dir, deserialized.config_source_dir);
    }

    #[test]
    fn test_create_workspace_marker() {
        let (_temp_dir, workspace_path) = create_test_workspace();
        fs::create_dir_all(&workspace_path).expect("Failed to create workspace dir");

        // Create a test config file with known content
        let original_config_path = workspace_path.join("test-config.yml");
        let test_config_content = "test: config\ndata: value";
        fs::write(&original_config_path, test_config_content).expect("Failed to write test config");

        let config_name = SDK_CONFIG_FILE;
        let mirror_path = Path::new("/tmp/test-mirror");

        let result = create_workspace_marker(CreateWorkspaceMarkerParams {
            workspace_path: &workspace_path,
            config_name,
            original_config_path: &original_config_path,
            mirror_path,
            original_identifier: None,
            target_version: None,
            skip_mirror: false,
            source_url: None,
            match_pattern: None,
        });
        assert!(result.is_ok());

        let marker_path = workspace_path.join(WORKSPACE_MARKER_FILE);
        assert!(marker_path.exists());

        let marker_content = fs::read_to_string(&marker_path).expect("Failed to read marker");
        let marker: WorkspaceMarker =
            serde_yaml::from_str(&marker_content).expect("Failed to parse marker");

        assert_eq!(marker.workspace_version, "1");
        assert_eq!(marker.config_file, config_name);
        assert_eq!(marker.target, "test-config.yml");
        assert!(!marker.config_sha256.is_empty());
        assert_eq!(marker.mirror_path, "/tmp/test-mirror");
    }

    #[test]
    fn test_find_workspace_root_not_found() {
        let (_temp_dir, workspace_path) = create_test_workspace();
        fs::create_dir_all(&workspace_path).expect("Failed to create workspace dir");
        // Don't create marker file

        let original_dir = env::current_dir().expect("Failed to get current dir");
        env::set_current_dir(&workspace_path).expect("Failed to change dir");

        let found_workspace = find_workspace_root();
        assert_eq!(found_workspace, None);

        // Restore original directory
        env::set_current_dir(original_dir).expect("Failed to restore dir");
    }

    #[test]
    fn test_is_url() {
        // Test valid URLs
        assert!(is_url("http://example.com/config.yml"));
        assert!(is_url("https://example.com/config.yml"));
        assert!(is_url(
            "https://raw.githubusercontent.com/user/repo/main/sdk.yml"
        ));

        // Test local paths
        assert!(!is_url("sdk.yml"));
        assert!(!is_url("./sdk.yml"));
        assert!(!is_url("/absolute/path/sdk.yml"));
        assert!(!is_url("../relative/path/sdk.yml"));
        assert!(!is_url("file://local/path"));
    }

    #[test]
    fn test_expand_env_vars() {
        // Test basic environment variable expansion
        std::env::set_var("TEST_VAR", "/test/path");

        // Test $VAR syntax
        assert_eq!(expand_env_vars("$TEST_VAR/subdir"), "/test/path/subdir");

        // Test ${VAR} syntax
        assert_eq!(expand_env_vars("${TEST_VAR}/subdir"), "/test/path/subdir");

        // Test paths without variables
        assert_eq!(expand_env_vars("/absolute/path"), "/absolute/path");
        assert_eq!(expand_env_vars("relative/path"), "relative/path");

        // Test multiple variables
        std::env::set_var("TEST_VAR2", "second");
        assert_eq!(expand_env_vars("$TEST_VAR/$TEST_VAR2"), "/test/path/second");

        // Test nonexistent variable (should leave unchanged)
        assert_eq!(
            expand_env_vars("$NONEXISTENT_VAR/path"),
            "$NONEXISTENT_VAR/path"
        );

        // Test Windows-style %VAR% syntax
        assert_eq!(expand_env_vars("%TEST_VAR%/subdir"), "/test/path/subdir");
        assert_eq!(
            expand_env_vars("%TEST_VAR%/%TEST_VAR2%"),
            "/test/path/second"
        );

        // Test nonexistent Windows variable (should leave unchanged)
        assert_eq!(
            expand_env_vars("%NONEXISTENT_VAR%/path"),
            "%NONEXISTENT_VAR%/path"
        );

        // Test HOME expansion if available
        if let Ok(home_path) = std::env::var("HOME") {
            assert_eq!(
                expand_env_vars("$HOME/workspace"),
                format!("{}/workspace", home_path)
            );

            // Test tilde expansion
            assert_eq!(
                expand_env_vars("~/workspace"),
                format!("{}/workspace", home_path)
            );

            assert_eq!(
                expand_env_vars("~/.config/cim/CLAUDE.md"),
                format!("{}/.config/cim/CLAUDE.md", home_path)
            );

            // Test bare tilde
            assert_eq!(expand_env_vars("~"), home_path);
        }

        // Cleanup
        std::env::remove_var("TEST_VAR");
        std::env::remove_var("TEST_VAR2");
    }

    #[test]
    fn test_resolve_mirror_cli_override_is_expanded() {
        // A --mirror override containing an environment variable is expanded.
        std::env::set_var("TEST_MIRROR_VAR", "/tmp/test-mirror");

        let resolved = resolve_mirror(Some(Path::new("$TEST_MIRROR_VAR/repos")), None);
        assert_eq!(resolved, PathBuf::from("/tmp/test-mirror/repos"));

        // Cleanup
        std::env::remove_var("TEST_MIRROR_VAR");
    }

    #[test]
    fn test_resolve_mirror_cli_override_beats_manifest() {
        // The --mirror flag short-circuits before UserConfig::load() and wins
        // over a manifest `mirror:` value.
        let resolved = resolve_mirror(
            Some(Path::new("/from/cli")),
            Some(Path::new("/from/manifest")),
        );
        assert_eq!(resolved, PathBuf::from("/from/cli"));
    }

    #[test]
    fn test_expand_manifest_vars_known_var() {
        let mut vars = std::collections::HashMap::new();
        vars.insert("PLATFORM".to_string(), "linux/amd64".to_string());
        vars.insert("BASE_URL".to_string(), "https://example.com".to_string());

        // Basic substitution
        assert_eq!(
            expand_manifest_vars("DOCKER_DEFAULT_PLATFORM=${{ PLATFORM }}", &vars),
            "DOCKER_DEFAULT_PLATFORM=linux/amd64"
        );

        // Substitution inside a URL
        assert_eq!(
            expand_manifest_vars("${{ BASE_URL }}/tool.tar.gz", &vars),
            "https://example.com/tool.tar.gz"
        );

        // Multiple vars in one string
        assert_eq!(
            expand_manifest_vars("${{ BASE_URL }}/${{ PLATFORM }}/file", &vars),
            "https://example.com/linux/amd64/file"
        );
    }

    #[test]
    fn test_expand_manifest_vars_unknown_var_unchanged() {
        let vars = std::collections::HashMap::new();

        // Unknown vars are left as-is
        assert_eq!(
            expand_manifest_vars("path/${{ UNKNOWN }}/file", &vars),
            "path/${{ UNKNOWN }}/file"
        );
    }

    #[test]
    fn test_has_unresolved_env_var_refs_no_dollar() {
        assert!(!has_unresolved_env_var_refs("https://example.com/sdk"));
        assert!(!has_unresolved_env_var_refs(""));
        assert!(!has_unresolved_env_var_refs("plain-value"));
    }

    #[test]
    fn test_has_unresolved_env_var_refs_manifest_pattern_only() {
        // ${{ VAR }} patterns are intentional Make references — not unresolved.
        assert!(!has_unresolved_env_var_refs("${{ WORKSPACE }}/bin"));
        assert!(!has_unresolved_env_var_refs("${{ A }}/${{ B }}"));
        assert!(!has_unresolved_env_var_refs(
            "prefix/${{ WORKSPACE }}/suffix"
        ));
    }

    #[test]
    fn test_has_unresolved_env_var_refs_bare_dollar() {
        // A bare $VAR or ${VAR} that was not expanded is a real warning case.
        assert!(has_unresolved_env_var_refs("$UNSET_HOST_VAR/path"));
        assert!(has_unresolved_env_var_refs("${UNSET_VAR}/path"));
    }

    #[test]
    fn test_has_unresolved_env_var_refs_mixed() {
        // A mix: the ${{ }} part is fine, but the bare $UNSET is a real warning.
        assert!(has_unresolved_env_var_refs("${{ WORKSPACE }}/$UNSET"));
    }

    #[test]
    fn test_resolve_variables_manifest_ref_passes_through() {
        // ${{ WORKSPACE }} must survive resolve_variables unchanged so that
        // Makefile generation can convert it to $(WORKSPACE) later.
        let mut raw = std::collections::HashMap::new();
        raw.insert(
            "TOOLCHAIN_PATH".to_string(),
            "${{ WORKSPACE }}/toolchains/bin".to_string(),
        );
        raw.insert("PLAIN".to_string(), "no-dollar-here".to_string());

        let resolved = resolve_variables(&raw);

        assert_eq!(
            resolved.get("TOOLCHAIN_PATH").map(String::as_str),
            Some("${{ WORKSPACE }}/toolchains/bin"),
            "Manifest variable reference should pass through resolve_variables unchanged"
        );
        assert_eq!(
            resolved.get("PLAIN").map(String::as_str),
            Some("no-dollar-here")
        );
    }

    #[test]
    fn test_resolve_variables_literal() {
        let mut raw = std::collections::HashMap::new();
        raw.insert(
            "SDK_BASE_URL".to_string(),
            "https://example.com/sdk".to_string(),
        );

        let resolved = resolve_variables(&raw);
        assert_eq!(
            resolved.get("SDK_BASE_URL").map(String::as_str),
            Some("https://example.com/sdk")
        );
    }

    #[test]
    fn test_resolve_variables_from_host_env() {
        std::env::set_var("TEST_HOST_PLATFORM", "linux/arm64");

        let mut raw = std::collections::HashMap::new();
        raw.insert(
            "DOCKER_DEFAULT_PLATFORM".to_string(),
            "$TEST_HOST_PLATFORM".to_string(),
        );

        let resolved = resolve_variables(&raw);
        assert_eq!(
            resolved.get("DOCKER_DEFAULT_PLATFORM").map(String::as_str),
            Some("linux/arm64")
        );

        std::env::remove_var("TEST_HOST_PLATFORM");
    }

    #[test]
    fn test_workspace_naming_with_target() {
        // Test that workspace path uses dsdk-{target-name} pattern
        use std::env;

        let test_targets = vec![
            ("adi-sdk", "dsdk-adi-sdk"),
            ("optee-qemu-v8", "dsdk-optee-qemu-v8"),
            ("dummy1", "dsdk-dummy1"),
            ("my-custom-target", "dsdk-my-custom-target"),
        ];

        for (target, expected_workspace_name) in test_targets {
            let home = env::var("HOME")
                .or_else(|_| env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());

            let expected_path = PathBuf::from(&home).join(expected_workspace_name);
            let workspace_name = format!("dsdk-{}", target);
            let actual_path = PathBuf::from(&home).join(workspace_name);

            assert_eq!(actual_path, expected_path);
            assert!(expected_path
                .to_string_lossy()
                .contains(expected_workspace_name));
        }
    }

    #[test]
    fn test_workspace_naming_priority() {
        // Test that priority order is: CLI flag > config.toml > default (dsdk-{target})
        // This test verifies the logic conceptually since we can't easily test the full init flow

        let target = "test-target";
        let cli_workspace = Some(PathBuf::from("/custom/cli/path"));
        let config_workspace = Some(PathBuf::from("/custom/config/path"));

        // Priority 1: CLI flag should take precedence
        let result = cli_workspace.clone().unwrap_or_else(|| {
            config_workspace.clone().unwrap_or_else(|| {
                PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".to_string()))
                    .join(format!("dsdk-{}", target))
            })
        });
        assert_eq!(result, PathBuf::from("/custom/cli/path"));

        // Priority 2: Config.toml when no CLI flag
        let result = config_workspace.clone().unwrap_or_else(|| {
            PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".to_string()))
                .join(format!("dsdk-{}", target))
        });
        assert_eq!(result, PathBuf::from("/custom/config/path"));

        // Priority 3: Default dsdk-{target} when neither CLI nor config specified
        let result = PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".to_string()))
            .join(format!("dsdk-{}", target));
        let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        assert_eq!(result, PathBuf::from(home).join("dsdk-test-target"));
    }

    #[test]
    fn test_insert_yaml_section_entry_appends_to_existing() {
        let content = "name: test\ngits:\n  - name: repo1\n    url: http://a\n    commit: abc\nother_key: val\n";
        let entry = "\n  - name: repo2\n    url: http://b\n    commit: def";
        let result = insert_yaml_section_entry(content, "gits", entry).unwrap();
        assert!(result.contains("repo1"));
        assert!(result.contains("repo2"));
        assert!(result.contains("other_key: val"));
        // repo2 should appear after repo1 and before other_key
        let pos_repo2 = result.find("repo2").unwrap();
        let pos_other = result.find("other_key").unwrap();
        assert!(pos_repo2 < pos_other);
    }

    #[test]
    fn test_insert_yaml_section_entry_empty_array() {
        let content = "name: test\ngits: []\nother_key: val\n";
        let entry = "\n  - name: repo1\n    url: http://a\n    commit: abc";
        let result = insert_yaml_section_entry(content, "gits", entry).unwrap();
        assert!(result.contains("gits:"));
        assert!(!result.contains("[]"));
        assert!(result.contains("repo1"));
        assert!(result.contains("other_key: val"));
    }

    #[test]
    fn test_insert_yaml_section_entry_missing_section() {
        let content = "name: test\nother: val\n";
        let entry = "\n  - name: repo1";
        let result = insert_yaml_section_entry(content, "gits", entry);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Could not find"));
    }

    #[test]
    fn test_insert_yaml_section_entry_at_end_of_file() {
        let content = "name: test\ngits:\n  - name: repo1\n    url: http://a\n    commit: abc\n";
        let entry = "\n  - name: repo2\n    url: http://b\n    commit: def";
        let result = insert_yaml_section_entry(content, "gits", entry).unwrap();
        assert!(result.contains("repo2"));
        // Should end with the entry (no trailing section to preserve)
        assert!(result.ends_with("def"));
    }

    #[test]
    fn test_git_name_to_dir_var_simple() {
        assert_eq!(git_name_to_dir_var("u-boot"), "U_BOOT_DIR");
        assert_eq!(git_name_to_dir_var("linux-stable"), "LINUX_STABLE_DIR");
        assert_eq!(git_name_to_dir_var("optee_os"), "OPTEE_OS_DIR");
    }

    #[test]
    fn test_git_name_to_dir_var_with_path() {
        assert_eq!(git_name_to_dir_var("zephyrproject/zephyr"), "ZEPHYR_DIR");
        assert_eq!(git_name_to_dir_var("foo/bar/baz"), "BAZ_DIR");
    }

    #[test]
    fn test_git_name_to_dir_var_with_dots() {
        assert_eq!(git_name_to_dir_var("my.project"), "MY_PROJECT_DIR");
        assert_eq!(git_name_to_dir_var("v2.0-release"), "V2_0_RELEASE_DIR");
    }

    #[test]
    fn test_git_name_to_venv_var() {
        assert_eq!(git_name_to_venv_var("u-boot"), "U_BOOT_VENV");
        assert_eq!(git_name_to_venv_var("linux-stable"), "LINUX_STABLE_VENV");
        assert_eq!(git_name_to_venv_var("optee_os"), "OPTEE_OS_VENV");
        assert_eq!(git_name_to_venv_var("zephyrproject/zephyr"), "ZEPHYR_VENV");
        assert_eq!(git_name_to_venv_var("v2.0-release"), "V2_0_RELEASE_VENV");
    }

    #[test]
    fn test_git_venv_path() {
        let ws = Path::new("/ws");
        assert_eq!(
            git_venv_path(ws, "u-boot"),
            Path::new("/ws/.cim/u-boot/.venv")
        );
        assert_eq!(
            git_venv_path(ws, "zephyrproject/zephyr"),
            Path::new("/ws/.cim/zephyrproject/zephyr/.venv")
        );
    }

    #[test]
    fn test_generate_git_dir_vars_basic() {
        let gits = vec![
            config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "master".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
            config::GitConfig {
                name: "linux-stable".to_string(),
                url: "https://example.com/linux.git".to_string(),
                commit: "v6.1".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
        ];

        let result = generate_git_dir_vars(&gits, None).unwrap();
        assert_eq!(result.get("U_BOOT_DIR").unwrap(), "${{ WORKSPACE }}/u-boot");
        assert_eq!(
            result.get("LINUX_STABLE_DIR").unwrap(),
            "${{ WORKSPACE }}/linux-stable"
        );
    }

    #[test]
    fn test_generate_git_dir_vars_conflict() {
        let gits = vec![
            config::GitConfig {
                name: "foo/zephyr".to_string(),
                url: "https://example.com/foo/zephyr.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
            config::GitConfig {
                name: "bar/zephyr".to_string(),
                url: "https://example.com/bar/zephyr.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
        ];

        let result = generate_git_dir_vars(&gits, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("foo/zephyr"));
        assert!(err.contains("bar/zephyr"));
        assert!(err.contains("ZEPHYR_DIR"));
    }

    #[test]
    fn test_generate_git_venv_vars_only_for_python_deps() {
        let gits = vec![
            config::GitConfig {
                name: "zephyrproject/zephyr".to_string(),
                url: "https://example.com/zephyr.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: Some(vec!["zephyrproject/zephyr/requirements.txt".to_string()]),
            },
            config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "master".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
        ];

        let result = generate_git_venv_vars(&gits, None).unwrap();
        // Only the git with python-deps gets a venv variable.
        assert_eq!(result.len(), 1);
        assert_eq!(
            result.get("ZEPHYR_VENV").unwrap(),
            "${{ WORKSPACE }}/.cim/zephyrproject/zephyr/.venv"
        );
        assert!(!result.contains_key("U_BOOT_VENV"));
    }

    #[test]
    fn test_generate_git_venv_vars_user_override() {
        let gits = vec![config::GitConfig {
            name: "zephyr".to_string(),
            url: "https://example.com/zephyr.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: None,
            documentation_dir: None,
            python_deps: Some(vec!["zephyr/requirements.txt".to_string()]),
        }];

        let mut user_vars = std::collections::HashMap::new();
        user_vars.insert("ZEPHYR_VENV".to_string(), "/custom/venv".to_string());

        // User-defined variable wins: no auto-generated entry.
        let result = generate_git_venv_vars(&gits, Some(&user_vars)).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_generate_git_dir_vars_user_override() {
        let gits = vec![config::GitConfig {
            name: "u-boot".to_string(),
            url: "https://example.com/u-boot.git".to_string(),
            commit: "master".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: None,
            documentation_dir: None,
            python_deps: None,
        }];

        let mut user_vars = std::collections::HashMap::new();
        user_vars.insert(
            "U_BOOT_DIR".to_string(),
            "${{ WORKSPACE }}/custom/u-boot".to_string(),
        );

        let result = generate_git_dir_vars(&gits, Some(&user_vars)).unwrap();
        assert!(!result.contains_key("U_BOOT_DIR"));
    }

    #[test]
    fn test_is_url_http() {
        assert!(is_url("http://example.com/manifests"));
    }

    #[test]
    fn test_is_url_https() {
        assert!(is_url("https://github.com/org/repo.git"));
    }

    #[test]
    fn test_is_url_ssh() {
        assert!(is_url("git@github.com:org/repo.git"));
    }

    #[test]
    fn test_is_url_local_path() {
        assert!(!is_url("/home/user/manifests"));
        assert!(!is_url("./relative/path"));
        assert!(!is_url("manifests"));
    }

    #[test]
    fn test_get_all_sources_from_config_default_only() {
        let uc = config::UserConfig {
            default_source: Some("/my/source".to_string()),
            alternate_sources: None,
            ..Default::default()
        };
        let sources = get_all_sources_from_config(Some(&uc));
        assert_eq!(sources, vec!["/my/source"]);
    }

    fn alt(url: &str) -> config::AlternateSourceConfig {
        config::AlternateSourceConfig {
            url: url.to_string(),
        }
    }

    #[test]
    fn test_get_all_sources_from_config_with_alternates() {
        let uc = config::UserConfig {
            default_source: Some("/default".to_string()),
            alternate_sources: Some(vec![alt("/alt1"), alt("/alt2")]),
            ..Default::default()
        };
        let sources = get_all_sources_from_config(Some(&uc));
        assert_eq!(sources, vec!["/default", "/alt1", "/alt2"]);
    }

    #[test]
    fn test_get_all_sources_from_config_dedup() {
        let uc = config::UserConfig {
            default_source: Some("/default".to_string()),
            alternate_sources: Some(vec![
                alt("/default"), // duplicate of default
                alt("/alt1"),
            ]),
            ..Default::default()
        };
        let sources = get_all_sources_from_config(Some(&uc));
        assert_eq!(sources, vec!["/default", "/alt1"]);
    }

    #[test]
    fn test_get_all_sources_from_config_filters_empty() {
        let uc = config::UserConfig {
            default_source: Some("/default".to_string()),
            alternate_sources: Some(vec![alt(""), alt("   "), alt("/valid")]),
            ..Default::default()
        };
        let sources = get_all_sources_from_config(Some(&uc));
        assert_eq!(sources, vec!["/default", "/valid"]);
    }

    #[test]
    fn test_get_all_sources_from_config_no_config() {
        let sources = get_all_sources_from_config(None);
        // Should return the hardcoded fallback path
        assert_eq!(sources.len(), 1);
    }
}
