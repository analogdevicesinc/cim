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

/// Get the default manifest source location, considering user config
pub fn get_default_source() -> String {
    // Try to load user config to get default_source
    if let Ok(Some(user_config)) = config::UserConfig::load() {
        if let Some(ref default_source) = user_config.default_source {
            return default_source.clone();
        }
    }

    // Fall back to hardcoded default with legacy path support
    // On Windows, use USERPROFILE if HOME is not set
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());

    let home_path = PathBuf::from(&home);
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

/// Get the docker temporary directory, considering user config
/// Creates the directory if it doesn't exist
pub fn get_docker_temp_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Try to load user config to get docker_temp_dir
    let temp_dir = if let Ok(Some(user_config)) = config::UserConfig::load() {
        if let Some(ref docker_temp) = user_config.docker_temp_dir {
            docker_temp.clone()
        } else {
            PathBuf::from("/tmp/cim-docker")
        }
    } else {
        PathBuf::from("/tmp/cim-docker")
    };

    // Create directory if it doesn't exist
    if !temp_dir.exists() {
        fs::create_dir_all(&temp_dir)?;
        messages::verbose(&format!(
            "Created docker temp directory: {}",
            temp_dir.display()
        ));
    }

    Ok(temp_dir)
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
}

/// Find the workspace root by walking up directories looking for .workspace marker
pub fn find_workspace_root() -> Option<PathBuf> {
    let mut current = env::current_dir().ok()?;

    loop {
        let marker_path = current.join(".workspace");
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

    let marker_path = params.workspace_path.join(".workspace");
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
    };

    fs::write(&marker_path, serde_yaml::to_string(&marker)?)?;
    messages::verbose(&format!(
        "Created workspace marker: {}",
        marker_path.display()
    ));
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
    let config_path = target_dir.join("sdk.yml");

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

    let persistent_config = extraction_path.join("sdk.yml");

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
    let marker_path = workspace_path.join(".workspace");
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

/// Check if a string represents a URL (http:// or https://)
pub fn is_url(input: &str) -> bool {
    input.starts_with("http://") || input.starts_with("https://")
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
    let mut result = path.to_string();

    // Handle tilde expansion first
    if result.starts_with("~/") || result == "~" {
        // Try HOME first (Unix), then USERPROFILE (Windows)
        let home = env::var("HOME").or_else(|_| env::var("USERPROFILE"));
        if let Ok(home) = home {
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
            let var_name = &result[start + 1..start + 1 + end];
            // Special handling for HOME: try HOME first, then USERPROFILE on Windows
            let value = if var_name == "HOME" {
                env::var("HOME").or_else(|_| env::var("USERPROFILE"))
            } else {
                env::var(var_name)
            };

            if let Ok(value) = value {
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
            let var_name = &result[start + 2..start + end];
            // Special handling for HOME: try HOME first, then USERPROFILE on Windows
            let value = if var_name == "HOME" {
                env::var("HOME").or_else(|_| env::var("USERPROFILE"))
            } else {
                env::var(var_name)
            };

            if let Ok(value) = value {
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
                // Special handling for HOME: try HOME first, then USERPROFILE on Windows
                let value = if var_name == "HOME" {
                    env::var("HOME").or_else(|_| env::var("USERPROFILE"))
                } else {
                    env::var(&var_name)
                };

                if let Ok(value) = value {
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

/// Resolve manifest variable values by expanding host env vars within them.
///
/// Each value in the raw map is passed through [`expand_env_vars`]. If a value
/// still contains a `$` character after expansion, the referenced host env var
/// was not set — a warning is logged so the user knows the variable is unresolved.
///
/// # Example
///
/// ```text
/// variables:
///   DOCKER_DEFAULT_PLATFORM: $HOST_DEFAULT_PLATFORM   # resolved from env
///   SDK_BASE_URL: https://example.com/sdk              # literal
/// ```
pub fn resolve_variables(
    raw: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    raw.iter()
        .map(|(key, value)| {
            let resolved = expand_env_vars(value);
            if resolved.contains('$') {
                messages::info(&format!(
                    "Warning: manifest variable '{}' has an unresolved reference in value: '{}'",
                    key, resolved
                ));
            }
            (key.clone(), resolved)
        })
        .collect()
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
/// 4. Expanding environment variables in mirror path
///
/// # Arguments
///
/// * `config_path` - Path to the sdk.yml file
/// * `verbose` - Whether to print verbose output about config loading and overrides
///
/// # Returns
///
/// Result containing the SdkConfig with user overrides applied and mirror path expanded
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

    // Expand environment variables in mirror path
    let original_mirror = sdk_config.mirror.to_string_lossy().to_string();
    let expanded_mirror = expand_config_mirror_path(&sdk_config);
    if verbose && original_mirror != expanded_mirror.to_string_lossy() {
        messages::verbose(&format!(
            "Expanded mirror: {} -> {}",
            original_mirror,
            expanded_mirror.display()
        ));
    }
    sdk_config.mirror = expanded_mirror;

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
    if let Some(raw_vars) = sdk_config.variables.clone() {
        let vars = resolve_variables(&raw_vars);

        // Expand in git URLs
        for git in &mut sdk_config.gits {
            git.url = expand_manifest_vars(&git.url, &vars);
        }

        // Expand in toolchain URL and destination
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

        // Expand in copy_files source and dest
        if let Some(ref mut copy_files) = sdk_config.copy_files {
            for cf in copy_files.iter_mut() {
                cf.source = expand_manifest_vars(&cf.source.clone(), &vars);
                cf.dest = expand_manifest_vars(&cf.dest.clone(), &vars);
            }
        }
    }

    Ok(sdk_config)
}

/// Expand environment variables in the mirror path of a config
///
/// This function takes a config and returns a new PathBuf with environment variables
/// expanded in the mirror path. This is needed because the YAML parser treats
/// $HOME and similar variables as literal strings. Used by init and update commands.
pub fn expand_config_mirror_path<T: config::SdkConfigCore>(config: &T) -> PathBuf {
    let mirror_str = config.mirror().to_string_lossy();
    let expanded = expand_env_vars(&mirror_str);
    PathBuf::from(expanded)
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

        let config_name = "sdk.yml";
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
        });
        assert!(result.is_ok());

        let marker_path = workspace_path.join(".workspace");
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
    fn test_expand_config_mirror_path() {
        // Create a test config with environment variable in mirror path
        std::env::set_var("TEST_MIRROR_VAR", "/tmp/test-mirror");

        let test_config = config::SdkConfig {
            makefile_include: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            toolchains: None,
            install: None,
            copy_files: None,
            mirror: PathBuf::from("$TEST_MIRROR_VAR/repos"),
            gits: vec![],
        };

        let expanded = expand_config_mirror_path(&test_config);
        assert_eq!(expanded, PathBuf::from("/tmp/test-mirror/repos"));

        // Cleanup
        std::env::remove_var("TEST_MIRROR_VAR");
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
}
