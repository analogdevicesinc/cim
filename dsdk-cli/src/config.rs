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

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::messages;

/// Custom deserializer for commit field that handles both strings and numbers
/// This allows YAML values like `2025.05` to be converted to strings
fn deserialize_commit<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum CommitValue {
        String(String),
        Float(f64),
        Int(i64),
    }

    let value = CommitValue::deserialize(deserializer)?;
    Ok(match value {
        CommitValue::String(s) => s,
        CommitValue::Float(f) => f.to_string(),
        CommitValue::Int(i) => i.to_string(),
    })
}

/// Custom deserializer for fields that can be either a sequence of strings or a single multiline string
/// This allows YAML to use either list format or block scalar format with |
fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        String(String),
        Vec(Vec<String>),
    }

    let value = Option::<StringOrVec>::deserialize(deserializer)?;
    Ok(match value {
        Some(StringOrVec::String(s)) => {
            // Split the string on newlines and filter out empty lines
            let lines: Vec<String> = s
                .lines()
                .map(|line| line.to_string())
                .filter(|line| !line.trim().is_empty())
                .collect();
            if lines.is_empty() {
                None
            } else {
                Some(lines)
            }
        }
        Some(StringOrVec::Vec(v)) => {
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        }
        None => None,
    })
}

/// SDK target configuration that supports both legacy array format and new object format
/// Legacy: `build: [command1, command2]`
/// New: `build: {commands: [command1, command2], depends_on: [repo1, repo2]}`
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum SdkTarget {
    /// Legacy format: just an array of commands
    Commands(Vec<String>),
    /// New format: object with commands and optional depends_on
    CommandsWithDeps {
        commands: Vec<String>,
        #[serde(default)]
        depends_on: Option<Vec<String>>,
    },
}

impl SdkTarget {
    /// Get the commands from either format
    pub fn commands(&self) -> &[String] {
        match self {
            SdkTarget::Commands(cmds) => cmds,
            SdkTarget::CommandsWithDeps { commands, .. } => commands,
        }
    }

    /// Get the dependencies if present
    pub fn depends_on(&self) -> Option<&[String]> {
        match self {
            SdkTarget::Commands(_) => None,
            SdkTarget::CommandsWithDeps { depends_on, .. } => {
                depends_on.as_ref().map(|v| v.as_slice())
            }
        }
    }
}

/// Custom deserializer for SDK targets that handles legacy array, multiline strings, and new object format
fn deserialize_sdk_target<'de, D>(deserializer: D) -> Result<Option<SdkTarget>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum SdkTargetValue {
        String(String),
        Vec(Vec<String>),
        Object {
            commands: StringOrVecCommands,
            #[serde(default)]
            depends_on: Option<Vec<String>>,
        },
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVecCommands {
        String(String),
        Vec(Vec<String>),
    }

    let value = Option::<SdkTargetValue>::deserialize(deserializer)?;
    Ok(match value {
        Some(SdkTargetValue::String(s)) => {
            let lines: Vec<String> = s
                .lines()
                .map(|line| line.to_string())
                .filter(|line| !line.trim().is_empty())
                .collect();
            if lines.is_empty() {
                None
            } else {
                Some(SdkTarget::Commands(lines))
            }
        }
        Some(SdkTargetValue::Vec(v)) => {
            if v.is_empty() {
                None
            } else {
                Some(SdkTarget::Commands(v))
            }
        }
        Some(SdkTargetValue::Object {
            commands,
            depends_on,
        }) => {
            let cmd_vec = match commands {
                StringOrVecCommands::String(s) => {
                    let lines: Vec<String> = s
                        .lines()
                        .map(|line| line.to_string())
                        .filter(|line| !line.trim().is_empty())
                        .collect();
                    lines
                }
                StringOrVecCommands::Vec(v) => v,
            };
            if cmd_vec.is_empty() {
                None
            } else {
                Some(SdkTarget::CommandsWithDeps {
                    commands: cmd_vec,
                    depends_on,
                })
            }
        }
        None => None,
    })
}

/// Trait for SDK configurations that provide core repository and mirror information
pub trait SdkConfigCore {
    fn mirror(&self) -> &PathBuf;
    fn gits(&self) -> &Vec<GitConfig>;
    fn install(&self) -> &Option<Vec<InstallConfig>>;
    fn makefile_include(&self) -> &Option<Vec<String>>;
    fn envsetup(&self) -> &Option<SdkTarget>;
    fn test(&self) -> &Option<SdkTarget>;
    fn clean(&self) -> &Option<SdkTarget>;
    fn build(&self) -> &Option<SdkTarget>;
    fn flash(&self) -> &Option<SdkTarget>;
    fn variables(&self) -> &Option<HashMap<String, String>>;
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitConfig {
    pub name: String,
    pub url: String,
    #[serde(deserialize_with = "deserialize_commit")]
    pub commit: String,
    /// Build ordering dependencies for Makefile generation.
    /// Accepts the legacy field name `depends_on` for backward compatibility.
    #[serde(default, alias = "depends_on")]
    pub build_depends_on: Option<Vec<String>>,
    /// Clone ordering dependencies. Repositories listed here will be cloned
    /// before this one, which is needed when repos are nested inside each other.
    #[serde(default)]
    pub git_depends_on: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub build: Option<Vec<String>>,
    /// Custom documentation directory within the git repository
    /// If specified, this directory will be searched for index.rst in addition to default locations
    #[serde(default)]
    pub documentation_dir: Option<String>,
}

/// Configuration for installing a component/tool in the workspace.
/// These are typically used for extracting downloaded archives and setting up
/// tools that don't require git repository management.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InstallConfig {
    /// Name of the installation target (e.g., "ninja", "zephyr-sdk")
    pub name: String,
    /// Optional dependencies on other install targets
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,
    /// Optional sentinel file path for idempotency check.
    /// If specified, installation only runs if this file doesn't exist
    /// and creates it upon successful completion.
    #[serde(default)]
    pub sentinel: Option<String>,
    /// Installation commands to execute
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub commands: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ToolchainConfig {
    /// Optional name for the toolchain file. If not specified, derived from URL
    #[serde(default)]
    pub name: Option<String>,
    pub url: String,
    pub destination: String,
    #[serde(default)]
    pub strip_components: Option<u32>,
    /// Optional OS filter (e.g., "darwin", "linux", "windows")
    #[serde(default)]
    pub os: Option<String>,
    /// Optional architecture filter (e.g., "x86_64", "arm64", "i386")
    #[serde(default)]
    pub arch: Option<String>,
    /// Optional mirror destination path (relative to mirror directory)
    /// If not specified, uses the same path as destination
    #[serde(default)]
    pub mirror_destination: Option<String>,
    /// Optional environment variables to set when running post-install commands
    /// Supports variable expansion:
    /// - `$PWD` or `${PWD}` - expands to the toolchain installation directory
    /// - `$WORKSPACE` or `${WORKSPACE}` - expands to the workspace root directory
    /// - `$HOME` or `${HOME}` - expands to the user's home directory
    /// - Standard environment variables (e.g., `$PATH`) are also expanded
    ///
    /// Example:
    /// ```yaml
    /// environment:
    ///   CARGO_HOME: "$PWD/cargo"
    ///   RUSTUP_HOME: "$PWD/rustup"
    ///   PATH: "$PWD/cargo/bin:$PATH"
    /// ```
    #[serde(default)]
    pub environment: Option<HashMap<String, String>>,
    /// Optional post-install commands to run after extraction
    /// These commands are executed in the destination directory
    /// Only executed when actually extracting (not when reusing existing symlinks)
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub post_install_commands: Option<Vec<String>>,
}

impl ToolchainConfig {
    /// Get the effective name, either from explicit name field or derived from URL
    pub fn get_name(&self) -> String {
        if let Some(ref name) = self.name {
            if !name.is_empty() {
                return name.clone();
            }
        }

        // Derive name from URL - extract filename from path
        if let Some(filename) = self.url.split('/').next_back() {
            if !filename.is_empty() && filename.contains('.') {
                return filename.to_string();
            }
        }

        // Fallback: use URL as name
        self.url.clone()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CopyFileConfig {
    pub source: String,
    pub dest: String,
    /// Optional: cache downloaded files in mirror for reuse
    #[serde(default)]
    pub cache: Option<bool>,
    /// Optional: SHA256 checksum for verification
    #[serde(default)]
    pub sha256: Option<String>,
    /// Optional: POST data for HTTP POST requests (form-urlencoded)
    #[serde(default)]
    pub post_data: Option<String>,
    /// Optional: create symlink instead of copying when cache is true
    #[serde(default)]
    pub symlink: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PackageManagerConfig {
    pub command: String,
    pub packages: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DistroConfig {
    // TODO: Remove backward compatibility after migration period
    /// Legacy version field - used in old format where distro key is just "ubuntu"
    /// In new format, version is extracted from key like "ubuntu-22.04"
    #[serde(default)]
    pub version: Option<String>,
    #[serde(flatten)]
    pub package_manager: PackageManagerConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OsConfig {
    #[serde(flatten)]
    pub distros: HashMap<String, DistroConfig>,
}

/// Custom deserializer for OsDependencies that skips non-OsConfig entries (like anchor definitions)
fn deserialize_os_dependencies<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, OsConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Deserialize, Error};
    use serde_yaml::Value;

    let value = Value::deserialize(deserializer)?;
    let mut result = HashMap::new();

    if let Value::Mapping(map) = value {
        for (key, val) in map {
            if let Value::String(key_str) = key {
                // Try to deserialize as OsConfig
                // If it fails (e.g., it's a sequence for an anchor definition), skip it
                if let Ok(os_config) = OsConfig::deserialize(val) {
                    result.insert(key_str, os_config);
                }
                // Silently skip entries that aren't OsConfig (like anchor definitions)
            }
        }
    } else {
        return Err(D::Error::custom("Expected a mapping"));
    }

    Ok(result)
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OsDependencies {
    #[serde(deserialize_with = "deserialize_os_dependencies")]
    #[serde(flatten)]
    pub os_configs: HashMap<String, OsConfig>,
}

impl OsDependencies {
    /// Parse a distro key to extract distro name and version.
    ///
    /// Supports new format: "ubuntu-22.04", "debian-12", "rocky-linux-9.0"
    /// Regex pattern: `^(.+?)-(\d+(?:\.\d+)*)$`
    ///
    /// # Arguments
    /// * `distro_key` - The distro key from os-dependencies.yml (e.g., "ubuntu-22.04", "debian-12")
    ///
    /// # Returns
    /// * `Some((distro_name, version))` if key contains version (e.g., ("ubuntu", "22.04"), ("debian", "12"))
    /// * `None` if key doesn't match the pattern (legacy format without version)
    ///
    /// # Examples
    /// ```
    /// # use dsdk_cli::config::OsDependencies;
    /// assert_eq!(
    ///     OsDependencies::parse_distro_key("ubuntu-22.04"),
    ///     Some(("ubuntu".to_string(), "22.04".to_string()))
    /// );
    /// assert_eq!(
    ///     OsDependencies::parse_distro_key("debian-12"),
    ///     Some(("debian".to_string(), "12".to_string()))
    /// );
    /// assert_eq!(
    ///     OsDependencies::parse_distro_key("rocky-linux-9.0"),
    ///     Some(("rocky-linux".to_string(), "9.0".to_string()))
    /// );
    /// assert_eq!(OsDependencies::parse_distro_key("ubuntu"), None);
    /// ```
    pub fn parse_distro_key(distro_key: &str) -> Option<(String, String)> {
        // Match pattern: distro-name-VERSION where VERSION is numeric
        // Example: "ubuntu-22.04" -> ("ubuntu", "22.04")
        // Example: "debian-12" -> ("debian", "12")
        // Example: "rocky-linux-9.0" -> ("rocky-linux", "9.0")
        // Pattern: ^(.+?)-(\d+(?:\.\d+)*)$
        //   ^(.+?) - capture one or more characters (non-greedy) from start
        //   - - literal dash before version
        //   (\d+(?:\.\d+)*)$ - capture version number (digits with optional .digits) at end

        // Find the last occurrence of '-' followed by a version pattern
        if let Some(last_dash_pos) = distro_key.rfind('-') {
            let potential_version = &distro_key[last_dash_pos + 1..];

            // Check if what follows the dash looks like a version number
            // Must start with a digit and contain only digits and dots
            // Examples: "12", "22.04", "9.0", "3.18"
            if !potential_version.is_empty()
                && potential_version.chars().next().unwrap().is_ascii_digit()
                && potential_version
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.')
            {
                let distro_name = distro_key[..last_dash_pos].to_string();
                let version = potential_version.to_string();
                return Some((distro_name, version));
            }
        }

        None
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PythonProfile {
    pub packages: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PythonDependencies {
    pub profiles: HashMap<String, PythonProfile>,
    #[serde(default = "default_python_profile")]
    pub default: String,
}

fn default_python_profile() -> String {
    "docs".to_string()
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SdkConfig {
    pub mirror: PathBuf,
    pub gits: Vec<GitConfig>,
    #[serde(default)]
    pub toolchains: Option<Vec<ToolchainConfig>>,
    #[serde(default)]
    pub copy_files: Option<Vec<CopyFileConfig>>,
    #[serde(default)]
    pub install: Option<Vec<InstallConfig>>,
    #[serde(default)]
    pub makefile_include: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_sdk_target")]
    pub envsetup: Option<SdkTarget>,
    #[serde(default, deserialize_with = "deserialize_sdk_target")]
    pub test: Option<SdkTarget>,
    #[serde(default, deserialize_with = "deserialize_sdk_target")]
    pub clean: Option<SdkTarget>,
    #[serde(default, deserialize_with = "deserialize_sdk_target")]
    pub build: Option<SdkTarget>,
    #[serde(default, deserialize_with = "deserialize_sdk_target")]
    pub flash: Option<SdkTarget>,
    /// Optional manifest-level variables. Values may reference host env vars
    /// using `$VAR` syntax, which are resolved at cim invocation time.
    /// In YAML commands, use `${{ VAR }}` to reference these variables.
    #[serde(default)]
    pub variables: Option<HashMap<String, String>>,
}

impl SdkConfigCore for SdkConfig {
    fn mirror(&self) -> &PathBuf {
        &self.mirror
    }

    fn gits(&self) -> &Vec<GitConfig> {
        &self.gits
    }

    fn install(&self) -> &Option<Vec<InstallConfig>> {
        &self.install
    }

    fn makefile_include(&self) -> &Option<Vec<String>> {
        &self.makefile_include
    }

    fn envsetup(&self) -> &Option<SdkTarget> {
        &self.envsetup
    }

    fn test(&self) -> &Option<SdkTarget> {
        &self.test
    }

    fn clean(&self) -> &Option<SdkTarget> {
        &self.clean
    }

    fn build(&self) -> &Option<SdkTarget> {
        &self.build
    }

    fn flash(&self) -> &Option<SdkTarget> {
        &self.flash
    }

    fn variables(&self) -> &Option<HashMap<String, String>> {
        &self.variables
    }
}

/// Load SDK configuration from a YAML file with include support.
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be read
/// - The YAML format is invalid
/// - The YAML structure doesn't match the expected `SdkConfig` schema
pub fn load_config<P: AsRef<Path>>(path: P) -> Result<SdkConfig, Box<dyn std::error::Error>> {
    let path_buf = PathBuf::from(path.as_ref());

    // Check if file exists first to provide a better error message
    if !path_buf.exists() {
        return Err(format!("Config file not found: {}", path_buf.display()).into());
    }

    // Check if file is readable
    if let Err(e) = fs::read_to_string(&path_buf) {
        return Err(format!("Cannot read config file {}: {}", path_buf.display(), e).into());
    }

    // Load and parse YAML directly without include processing
    let file_content = fs::read_to_string(&path_buf)
        .map_err(|e| format!("Cannot read config file {}: {}", path_buf.display(), e))?;

    let config: SdkConfig = serde_yaml::from_str(&file_content)
        .map_err(|e| format!("Config validation error in {}: {e}", path_buf.display()))?;
    Ok(config)
}

/// Load OS dependencies configuration from a YAML file.
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be read
/// - The YAML format is invalid
/// - The YAML structure doesn't match the expected `OsDependencies` schema
pub fn load_os_dependencies<P: AsRef<Path>>(
    path: P,
) -> Result<OsDependencies, Box<dyn std::error::Error>> {
    let file_content = fs::read_to_string(path)?;
    let os_deps: OsDependencies = serde_yaml::from_str(&file_content)?;
    Ok(os_deps)
}

/// Load Python dependencies configuration from a YAML file.
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be read
/// - The YAML format is invalid
/// - The YAML structure doesn't match the expected `PythonDependencies` schema
pub fn load_python_dependencies<P: AsRef<Path>>(
    path: P,
) -> Result<PythonDependencies, Box<dyn std::error::Error>> {
    let file_content = fs::read_to_string(path)?;
    let python_deps: PythonDependencies = serde_yaml::from_str(&file_content)?;
    Ok(python_deps)
}

/// User configuration file for personal preferences.
/// Located at $HOME/.config/cim/config.toml
///
/// This config contains user-level preferences that apply across all SDK workspaces.
/// Workspace-specific settings (toolchains, build commands, etc.) should remain in
/// the manifest sdk.yml files.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct UserConfig {
    /// Override default mirror directory
    #[serde(default)]
    pub mirror: Option<PathBuf>,

    /// Default workspace directory
    #[serde(default)]
    pub default_workspace: Option<PathBuf>,

    /// Workspace name prefix (default: "dsdk-")
    #[serde(default)]
    pub workspace_prefix: Option<String>,

    /// Default manifest source location
    #[serde(default)]
    pub default_source: Option<String>,

    /// Skip mirror operations (behaves as if --no-mirror flag is always specified)
    #[serde(default)]
    pub no_mirror: Option<bool>,

    /// Directory for storing temporary files during docker create operations
    #[serde(default)]
    pub docker_temp_dir: Option<PathBuf>,

    /// Additional files to copy during initialization (merged with manifest)
    #[serde(default)]
    pub copy_files: Option<Vec<CopyFileConfig>>,

    /// Override default shell for post-install commands
    #[serde(default)]
    pub shell: Option<String>,

    /// Override default shell argument (e.g., "-c" for bash, "/C" for cmd)
    #[serde(default)]
    pub shell_arg: Option<String>,

    /// Additional documentation directories to search for index.rst files
    /// Comma-separated list of directories (e.g., "my_docs, custom_docs")
    /// These are combined with built-in defaults and per-repository settings
    #[serde(default)]
    pub documentation_dirs: Option<String>,

    /// TLS certificate validation mode for downloads
    /// Values: "strict" (never bypass SSL), "relaxed" (always bypass), "auto" (fallback)
    /// Default: "strict" - requires valid certificates, recommended for security
    /// Use "relaxed" or "auto" only in corporate environments with SSL inspection
    #[serde(default)]
    pub cert_validation: Option<String>,

    /// Skip section dividers in generated Makefiles
    /// When true, `cim makefile` will not insert comment banners between sections
    #[serde(default)]
    pub no_dividers: Option<bool>,
}

impl UserConfig {
    /// Get the default path for user config file
    /// On Windows: %LOCALAPPDATA%\cim\config.toml (machine-specific config)
    /// On Unix: ~/.config/cim/config.toml
    pub fn default_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            // Use LOCALAPPDATA instead of APPDATA since config contains machine-specific paths
            // (mirror directories, workspace locations, etc.) that shouldn't roam
            if let Ok(localappdata) = env::var("LOCALAPPDATA") {
                return PathBuf::from(localappdata).join("cim").join("config.toml");
            }
            // Fallback to APPDATA if LOCALAPPDATA is not set (shouldn't happen on modern Windows)
            if let Ok(appdata) = env::var("APPDATA") {
                return PathBuf::from(appdata).join("cim").join("config.toml");
            }
        }

        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".config")
            .join("cim")
            .join("config.toml")
    }

    /// Generate a template config.toml with all supported options and examples
    pub fn generate_template() -> String {
        r#"# Code in Motion Configuration
# Location: ~/.config/cim/config.toml
#
# This file allows you to override manifest settings with personal preferences
# that apply across all SDK targets and workspaces.
#
# All settings are optional. Uncomment and modify values as needed.
# Changes take effect immediately on next cim command.

# =============================================================================
# Mirror Directory
# =============================================================================
# Override the default mirror directory location.
# The mirror stores git repositories and toolchains for fast workspace creation.
#
# Default: $HOME/tmp/mirror
# Use cases:
#   - Store on faster SSD for better performance
#   - Use shared network location for team collaboration
#   - Separate location with more disk space
#
# Examples:
# mirror = "/fast-ssd/sdk-mirror"
# mirror = "/Volumes/ExternalDrive/mirror"
# mirror = "/mnt/shared/team-mirror"

# =============================================================================
# Default Workspace Directory
# =============================================================================
# Set the default workspace directory for 'init' command.
# Used when --workspace flag is not provided.
#
# Default: $HOME/dsdk-{target-name}
# Use cases:
#   - Organize all workspaces in a dedicated projects directory
#   - Use a different drive with more space
#   - Keep workspaces separate from personal files
#
# Examples:
# default_workspace = "/projects/workspaces"
# default_workspace = "/Volumes/Work/sdk-workspaces"
# default_workspace = "/home/user/development/sdks"

# =============================================================================
# Workspace Name Prefix
# =============================================================================
# Set the prefix for auto-generated workspace names.
# When --workspace is not specified, workspace name is: {prefix}{target-name}
#
# Default: "dsdk-"
# Use cases:
#   - Use custom prefix like "sdk-" or "dev-"
#   - Remove prefix entirely with empty string: workspace_prefix = ""
#   - Match your organization's naming convention
#
# Examples:
# workspace_prefix = "dsdk-"     # Default: ~/dsdk-adi-sdk
# workspace_prefix = "sdk-"      # Creates: ~/sdk-adi-sdk
# workspace_prefix = ""          # Creates: ~/adi-sdk (no prefix)
# workspace_prefix = "project-"  # Creates: ~/project-adi-sdk

# =============================================================================
# Default Manifest Source
# =============================================================================
# Set the default manifest source location.
# Used when --source flag is not provided to 'init' or 'list-targets'.
# Can be a local path or git URL.
#
# Default: $HOME/devel/cim-manifests
# Note: cim also checks legacy path $HOME/devel/sdk-manager-manifests automatically
# Use cases:
#   - Use company-internal manifest repository
#   - Point to local development manifests
#   - Use fork with custom targets
#
# Examples:
# default_source = "https://github.com/mycompany/sdk-manifests"
# default_source = "git@github.com:myteam/custom-manifests.git"
# default_source = "/home/user/projects/my-manifests"
# default_source = "https://github.com/analogdevicesinc/cim-manifests"

# =============================================================================
# Skip Mirror Operations
# =============================================================================
# Skip mirror operations and clone repositories directly from remote URLs.
# Behaves as if --no-mirror flag is always specified.
#
# Default: false
# Use cases:
#   - Avoid local mirror overhead for single workspace use
#   - Reduce disk space usage when mirror is not needed
#   - Simplify workflow when always working with latest remote versions
#   - CI/CD environments where mirror adds no value
#
# Examples:
# no_mirror = true
# no_mirror = false

# =============================================================================
# Docker Temporary Directory
# =============================================================================
# Directory for storing temporary manifest files during 'docker create' operations.
# These files are extracted from the manifest repository and used to generate
# the Dockerfile, then cleaned up after successful generation.
#
# Default: /tmp/cim-docker
# Use cases:
#   - Control where temporary files are stored
#   - Use a different partition with more space
#   - Organize temp files in a predictable location
#   - Debug manifest extraction issues
#
# Examples:
# docker_temp_dir = "/tmp/cim-docker"
# docker_temp_dir = "/Users/user/.cache/cim/docker"
# docker_temp_dir = "/var/tmp/cim-docker"

# =============================================================================
# Additional Copy Files
# =============================================================================
# Additional files to copy during workspace initialization.
# These are merged with copy_files from the manifest target.
# Useful for personal scripts, patches, or configuration files.
#
# Use cases:
#   - Add personal initialization scripts to all workspaces
#   - Include custom patches or configuration files
#   - Add team-specific tools or helpers
#
# Format:
#   [[copy_files]]
#   source = "path/to/source/file"
#   dest = "destination/in/workspace"
#
# Examples:
# [[copy_files]]
# source = "~/.my-scripts/personal-init.sh"
# dest = "personal-init.sh"
#
# [[copy_files]]
# source = "/path/to/patches/my-custom-fix.patch"
# dest = "patches/my-custom-fix.patch"
#
# [[copy_files]]
# source = "~/dotfiles/.vimrc"
# dest = ".vimrc"

# =============================================================================
# Shell Configuration
# =============================================================================
# Override the default shell used for post-install commands and scripts.
# Useful for Windows environments or custom shell preferences.
#
# Default (Linux/macOS): bash -c
# Default (Windows): cmd /C
# Use cases:
#   - Use PowerShell instead of cmd on Windows
#   - Use specific bash version or zsh on Unix systems
#   - Support corporate environments with restricted shells
#
# Examples (Windows):
# shell = "powershell"
# shell_arg = "-Command"
#
# Examples (Unix):
# shell = "zsh"
# shell_arg = "-c"
#
# shell = "/bin/bash"
# shell_arg = "-c"

# =============================================================================
# Documentation Directories
# =============================================================================
# Additional directories to search for index.rst files when discovering documentation.
# Comma-separated list of directory names (e.g., "my_docs, custom_docs").
# These are combined with built-in defaults and per-repository settings.
#
# Built-in defaults: docs, doc, documentation, documents, . (root)
# Use cases:
#   - Add custom documentation directory names used in your repositories
#   - Support legacy or non-standard documentation locations
#   - Search additional directories across all repositories
#
# Note: Per-repository directories can be specified in sdk.yml using:
#   gits:
#     - name: my-repo
#       documentation_dir: "custom_docs"
#
# Examples:
# documentation_dirs = "my_docs"                    # Single custom directory
# documentation_dirs = "my_docs, custom_docs"       # Multiple directories
# documentation_dirs = "wiki, manual, reference"    # Multiple alternatives

# =============================================================================
# TLS Certificate Validation (Security)
# =============================================================================
# Control TLS certificate validation for toolchain and file downloads.
# This setting affects security and should be changed with caution.
#
# Values:
#   - "strict" (default): Never bypass SSL certificate validation
#     Recommended for security. Rejects downloads from servers with invalid/self-signed certs.
#   - "relaxed": Always bypass SSL certificate validation
#     Use only in corporate environments with SSL inspection/MITM proxies.
#     WARNING: Exposes you to man-in-the-middle attacks!
#   - "auto": Try strict first, fallback to relaxed if it fails
#     Attempts secure download, uses insecure fallback if needed.
#     WARNING: May allow MITM attacks on fallback!
#
# Default: "strict"
# Use cases:
#   - Corporate proxy with SSL inspection: Use "relaxed" or "auto"
#   - Standard environments: Keep "strict" (recommended)
#   - CI/CD with valid certificates: Keep "strict"
#
# SECURITY WARNING:
#   Using "relaxed" or "auto" can expose you to man-in-the-middle attacks
#   where an attacker intercepts and modifies downloaded files.
#   Only use these modes if you understand the risks and are in a controlled
#   corporate environment with SSL inspection.
#
# To temporarily override for a single command, use --cert-validation flag:
#   cim init --cert-validation relaxed
#   cim install toolchains --cert-validation auto
#
# Examples:
# cert_validation = "strict"     # Default: Maximum security, valid certs required
# cert_validation = "relaxed"    # Corporate: Bypass all cert validation (INSECURE!)
# cert_validation = "auto"       # Fallback: Try strict, use relaxed if fails (INSECURE!)

# =============================================================================
# Makefile Section Dividers
# =============================================================================
# Control whether section dividers (comment banners) are inserted between
# sections in the generated Makefile. Dividers improve readability for large
# Makefiles by visually separating variables, SDK targets, install targets,
# and git repository targets.
#
# Default: false (dividers are included)
# Use cases:
#   - Set to true if you prefer minimal, compact Makefiles
#   - Keep false (or omit) for readable Makefiles with section headers
#
# To temporarily override for a single command, use --no-dividers flag:
#   cim makefile --no-dividers
#
# Examples:
# no_dividers = true     # Never add dividers to generated Makefiles
# no_dividers = false    # Always add dividers (default behavior)
"#
        .to_string()
    }

    /// Load user config from the default location
    /// Returns None if file doesn't exist, or Ok(UserConfig) if successfully loaded
    pub fn load() -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let path = Self::default_path();
        Self::load_from(&path)
    }

    /// Load user config from a specific path
    /// Returns None if file doesn't exist, or Ok(UserConfig) if successfully loaded
    pub fn load_from<P: AsRef<Path>>(path: P) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let path = path.as_ref();

        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read user config {}: {}", path.display(), e))?;

        let config: UserConfig = toml::from_str(&content)
            .map_err(|e| format!("Failed to parse user config {}: {}", path.display(), e))?;

        Ok(Some(config))
    }

    /// Ensure template config file exists, creating it if necessary
    /// This should be called when running cim without arguments
    /// Returns true if template was created, false if it already existed
    pub fn ensure_template_exists() -> Result<bool, Box<dyn std::error::Error>> {
        let path = Self::default_path();

        if path.exists() {
            return Ok(false);
        }

        // Create parent directories if they don't exist
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create config directory {}: {}",
                        parent.display(),
                        e
                    )
                })?;
            }
        }

        // Generate and write template
        let template = Self::generate_template();
        fs::write(&path, template).map_err(|e| {
            format!(
                "Failed to write template config to {}: {}",
                path.display(),
                e
            )
        })?;

        messages::status("");
        messages::info(&format!(
            "Created user config template at: {}",
            path.display()
        ));
        messages::info("Edit this file to customize cim settings.");
        messages::info("To regenerate: delete the file and run 'cim' again.");
        messages::status("");

        Ok(true)
    }

    /// Apply user config overrides to a SdkConfig
    /// Returns the number of overrides applied
    pub fn apply_to_sdk_config(&self, config: &mut SdkConfig, verbose: bool) -> usize {
        let mut override_count = 0;

        if let Some(ref mirror) = self.mirror {
            if verbose {
                messages::verbose(&format!(
                    "User config override: mirror {} -> {}",
                    config.mirror.display(),
                    mirror.display()
                ));
            }
            config.mirror = mirror.clone();
            override_count += 1;
        }

        if let Some(ref user_copy_files) = self.copy_files {
            if verbose {
                messages::verbose(&format!(
                    "User config: adding {} additional copy_files entries",
                    user_copy_files.len()
                ));
            }
            let mut merged = config.copy_files.clone().unwrap_or_default();
            merged.extend(user_copy_files.clone());
            config.copy_files = Some(merged);
            override_count += 1;
        }

        override_count
    }

    /// List all configuration values in key=value format (similar to git config -l)
    /// Uses dot notation for nested structures (e.g., copy_files.0.src=/path)
    pub fn list_all(&self) -> Vec<String> {
        let mut lines = Vec::new();

        if let Some(ref mirror) = self.mirror {
            lines.push(format!("mirror={}", mirror.display()));
        }
        if let Some(ref ws) = self.default_workspace {
            lines.push(format!("default_workspace={}", ws.display()));
        }
        if let Some(ref prefix) = self.workspace_prefix {
            lines.push(format!("workspace_prefix={}", prefix));
        }
        if let Some(ref source) = self.default_source {
            lines.push(format!("default_source={}", source));
        }
        if let Some(no_mirror) = self.no_mirror {
            lines.push(format!("no_mirror={}", no_mirror));
        }
        if let Some(ref temp_dir) = self.docker_temp_dir {
            lines.push(format!("docker_temp_dir={}", temp_dir.display()));
        }
        if let Some(ref files) = self.copy_files {
            for (idx, file) in files.iter().enumerate() {
                lines.push(format!("copy_files.{}.source={}", idx, file.source));
                lines.push(format!("copy_files.{}.dest={}", idx, file.dest));
            }
        }
        if let Some(ref shell) = self.shell {
            lines.push(format!("shell={}", shell));
        }
        if let Some(ref shell_arg) = self.shell_arg {
            lines.push(format!("shell_arg={}", shell_arg));
        }
        if let Some(ref docs_dirs) = self.documentation_dirs {
            lines.push(format!("documentation_dirs={}", docs_dirs));
        }
        if let Some(ref cert_val) = self.cert_validation {
            lines.push(format!("cert_validation={}", cert_val));
        }
        if let Some(no_dividers) = self.no_dividers {
            lines.push(format!("no_dividers={}", no_dividers));
        }

        lines
    }

    /// Get a specific configuration value by key
    /// Supports dot notation for nested values (e.g., "copy_files.0.src")
    pub fn get_value(&self, key: &str) -> Option<String> {
        match key {
            "mirror" => self.mirror.as_ref().map(|p| p.display().to_string()),
            "default_workspace" => self
                .default_workspace
                .as_ref()
                .map(|p| p.display().to_string()),
            "workspace_prefix" => self.workspace_prefix.clone(),
            "default_source" => self.default_source.clone(),
            "no_mirror" => self.no_mirror.map(|b| b.to_string()),
            "docker_temp_dir" => self
                .docker_temp_dir
                .as_ref()
                .map(|p| p.display().to_string()),
            "shell" => self.shell.clone(),
            "shell_arg" => self.shell_arg.clone(),
            "documentation_dirs" => self.documentation_dirs.clone(),
            "cert_validation" => self.cert_validation.clone(),
            "no_dividers" => self.no_dividers.map(|b| b.to_string()),
            _ => {
                // Handle nested keys like copy_files.0.src
                if key.starts_with("copy_files.") {
                    let parts: Vec<&str> = key.split('.').collect();
                    if parts.len() == 3 {
                        if let Ok(idx) = parts[1].parse::<usize>() {
                            if let Some(ref files) = self.copy_files {
                                if let Some(file) = files.get(idx) {
                                    return match parts[2] {
                                        "source" => Some(file.source.clone()),
                                        "dest" => Some(file.dest.clone()),
                                        _ => None,
                                    };
                                }
                            }
                        }
                    }
                }
                None
            }
        }
    }
}

/// Load SDK configuration along with any embedded OS dependencies.
///
/// # Errors
///
/// Returns an error if:
/// - The config file cannot be read or parsed
/// - The YAML format is invalid
/// - The YAML structure doesn't match the expected schema
pub fn load_config_with_os_deps<P: AsRef<Path>>(
    path: P,
) -> Result<(SdkConfig, Option<OsDependencies>), Box<dyn std::error::Error>> {
    let path_buf = path.as_ref().to_path_buf();
    let config = load_config(&path_buf)?;

    // Try to load os-dependencies.yml from the same directory as sdk.yml
    let os_deps = if let Some(config_dir) = path_buf.parent() {
        let os_deps_path = config_dir.join("os-dependencies.yml");
        if os_deps_path.exists() {
            match load_os_dependencies(&os_deps_path) {
                Ok(deps) => Some(deps),
                Err(e) => {
                    messages::verbose(&format!(
                        "Warning: Failed to load os-dependencies.yml: {}",
                        e
                    ));
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok((config, os_deps))
}

/// Determines the certificate validation mode based on CLI flag or user config.
///
/// # Arguments
///
/// * `cli_override` - Optional CLI flag value that takes precedence over config
///
/// # Returns
///
/// Returns a tuple of (mode, show_warning) where:
/// - mode: "strict", "relaxed", or "auto"
/// - show_warning: Whether to display security warning to user
///
/// # Behavior
///
/// Priority order:
/// 1. CLI flag (--cert-validation) if provided
/// 2. User config cert_validation setting
/// 3. Default to "strict" if neither is set
///
/// Security warnings are shown when:
/// - Mode is "relaxed" or "auto"
/// - The setting comes from config (not CLI flag, as CLI users are aware)
pub fn get_cert_validation_mode(cli_override: Option<&str>) -> (String, bool) {
    // CLI flag takes highest priority
    if let Some(mode) = cli_override {
        let mode_str = mode.to_string();
        // No warning for CLI override - user explicitly chose this
        return (mode_str, false);
    }

    // Try to load from user config
    if let Ok(Some(user_config)) = UserConfig::load() {
        if let Some(mode) = user_config.cert_validation {
            let show_warning = mode == "relaxed" || mode == "auto";
            return (mode, show_warning);
        }
    }

    // Default to strict mode
    ("strict".to_string(), false)
}

/// Resolve the clone order for git repositories based on their
/// `git_depends_on` fields. Returns a vector of tiers, where each tier is
/// a vector of `GitConfig` entries that can be cloned in parallel. Tiers
/// must be processed sequentially: tier 0 first, then tier 1, etc.
///
/// Uses Kahn's algorithm (BFS-based topological sort) to compute the order.
/// Returns an error if a circular dependency is detected or if a
/// `git_depends_on` entry references a repository name not in the gits list.
pub fn resolve_clone_order(gits: &[GitConfig]) -> Result<Vec<Vec<GitConfig>>, String> {
    if gits.is_empty() {
        return Ok(vec![]);
    }

    // Build a name → index lookup
    let name_to_index: HashMap<&str, usize> = gits
        .iter()
        .enumerate()
        .map(|(i, g)| (g.name.as_str(), i))
        .collect();

    // Validate all git_depends_on references exist in the gits list
    for git_cfg in gits {
        if let Some(deps) = &git_cfg.git_depends_on {
            for dep in deps {
                if !name_to_index.contains_key(dep.as_str()) {
                    return Err(format!(
                        "Repository '{}' has git_depends_on '{}', \
                         which is not defined in the gits list",
                        git_cfg.name, dep
                    ));
                }
            }
        }
    }

    // Build adjacency list and compute in-degrees for Kahn's algorithm
    let n = gits.len();
    let mut in_degree = vec![0usize; n];
    let mut dependents: Vec<Vec<usize>> = vec![vec![]; n];

    for (i, git_cfg) in gits.iter().enumerate() {
        if let Some(deps) = &git_cfg.git_depends_on {
            let unique_deps: HashSet<&str> = deps.iter().map(|s| s.as_str()).collect();
            in_degree[i] = unique_deps.len();
            for dep in &unique_deps {
                let dep_idx = name_to_index[dep];
                dependents[dep_idx].push(i);
            }
        }
    }

    // BFS: process nodes tier by tier
    let mut tiers: Vec<Vec<GitConfig>> = Vec::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    let mut processed = 0usize;

    // Seed with all repos that have no dependencies (in_degree == 0)
    for (i, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(i);
        }
    }

    while !queue.is_empty() {
        // Drain the current queue into a tier
        let tier_indices: Vec<usize> = queue.drain(..).collect();
        let mut tier = Vec::with_capacity(tier_indices.len());

        for &idx in &tier_indices {
            processed += 1;
            tier.push(gits[idx].clone());

            // Reduce in-degree for dependents and enqueue those that become ready
            for &dep_idx in &dependents[idx] {
                in_degree[dep_idx] -= 1;
                if in_degree[dep_idx] == 0 {
                    queue.push_back(dep_idx);
                }
            }
        }

        tiers.push(tier);
    }

    if processed != n {
        // Some repos were never processed — there's a cycle
        let unprocessed: Vec<&str> = in_degree
            .iter()
            .enumerate()
            .filter(|(_, &deg)| deg > 0)
            .map(|(i, _)| gits[i].name.as_str())
            .collect();
        return Err(format!(
            "Circular dependency detected among repositories: {}",
            unprocessed.join(", ")
        ));
    }

    Ok(tiers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    const YAML: &str = r#"
mirror: /tmp/mirror

gits:
  - name: git
    url: git@github.com:git/git.git
    commit: v2.38.1
  - name: busybox
    url: https://github.com/mirror/busybox.git
    commit: master
    depends_on:
      - git
"#;

    #[test]
    fn test_load_config_valid() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("sdk.yml");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(YAML.as_bytes()).unwrap();
        let config = load_config(&file_path).unwrap();
        assert_eq!(config.mirror.to_str().unwrap(), "/tmp/mirror");
        assert_eq!(config.gits.len(), 2);
        assert_eq!(config.gits[0].name, "git");
        assert_eq!(
            config.gits[1].build_depends_on.as_ref().unwrap(),
            &vec!["git".to_string()]
        );
    }

    #[test]
    fn test_load_config_invalid() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("bad.yml");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"not: yaml: - at all").unwrap();
        let result = load_config(&file_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_yaml_anchors_and_aliases() {
        let yaml_with_anchors = r#"
mirror: /tmp/mirror

# Define common build steps with anchor
common_steps: &shared_build
  - "@echo Building"
  - make all

gits:
  - name: repo1
    url: https://github.com/example/repo1.git
    commit: main
    build: *shared_build
  
  - name: repo2
    url: https://github.com/example/repo2.git
    commit: main
    build: *shared_build
"#;
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("anchor_test.yml");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(yaml_with_anchors.as_bytes()).unwrap();

        let config = load_config(&file_path).unwrap();

        // Verify both repos have the same build steps from the anchor
        assert_eq!(config.gits.len(), 2);
        assert_eq!(config.gits[0].name, "repo1");
        assert_eq!(config.gits[1].name, "repo2");

        let build1 = config.gits[0].build.as_ref().unwrap();
        let build2 = config.gits[1].build.as_ref().unwrap();

        assert_eq!(build1.len(), 2);
        assert_eq!(build1[0], "@echo Building");
        assert_eq!(build1[1], "make all");

        // Both should have identical content from the anchor
        assert_eq!(build1, build2);
    }

    #[test]
    fn test_quoted_reserved_characters() {
        let yaml_with_quotes = r#"
mirror: /tmp/mirror

gits:
  - name: test
    url: https://github.com/example/test.git
    commit: main
    build:
      - "@echo This must be quoted"
      - make test
"#;
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("quoted_test.yml");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(yaml_with_quotes.as_bytes()).unwrap();

        let config = load_config(&file_path).unwrap();
        let build = config.gits[0].build.as_ref().unwrap();

        assert_eq!(build[0], "@echo This must be quoted");
        assert_eq!(build[1], "make test");
    }

    #[test]
    fn test_user_config_template_generation() {
        let template = UserConfig::generate_template();

        // Verify template is not empty
        assert!(!template.is_empty());

        // Verify template contains all supported fields as comments
        assert!(template.contains("mirror"));
        assert!(template.contains("default_workspace"));
        assert!(template.contains("default_source"));
        assert!(template.contains("copy_files"));

        // Verify template has proper structure
        assert!(template.contains("Code in Motion Configuration"));
        assert!(template.contains("~/.config/cim/config.toml"));

        // Verify template is valid TOML
        let parsed: Result<UserConfig, _> = toml::from_str(&template);
        assert!(parsed.is_ok());

        let config = parsed.unwrap();
        // Template has most fields commented out, so they should be None/empty
        assert!(config.mirror.is_none());
        assert!(config.default_workspace.is_none());
        assert!(config.default_source.is_none());
        assert!(config.no_mirror.is_none());
        assert!(config.copy_files.is_none());
    }

    #[test]
    fn test_user_config_load_nonexistent() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");

        // Verify file doesn't exist initially
        assert!(!config_path.exists());

        // Load should return None without creating file
        let result = UserConfig::load_from(&config_path);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());

        // Verify file was NOT created (new behavior)
        assert!(!config_path.exists());
    }

    #[test]
    fn test_user_config_ensure_template_exists() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("subdir").join("config.toml");

        // Verify file doesn't exist initially
        assert!(!config_path.exists());

        // Manually call the template creation logic
        // Create parent directories
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();

        // Generate and write template
        let template = UserConfig::generate_template();
        fs::write(&config_path, template).unwrap();

        // Verify file was created
        assert!(config_path.exists());

        // Verify created file is valid TOML
        let content = fs::read_to_string(&config_path).unwrap();
        let parsed: Result<UserConfig, _> = toml::from_str(&content);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_user_config_load_with_values() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");

        // Create a config with actual values
        let config_content = r#"
mirror = "/custom/mirror"
default_workspace = "/custom/workspace"
default_source = "https://example.com/manifests"

[[copy_files]]
source = "file1.txt"
dest = "dest1.txt"

[[copy_files]]
source = "file2.txt"
dest = "dest2.txt"
"#;
        fs::write(&config_path, config_content).unwrap();

        // Load the config
        let result = UserConfig::load_from(&config_path);
        assert!(result.is_ok());

        let config = result.unwrap();
        assert!(config.is_some());

        let config = config.unwrap();
        assert_eq!(config.mirror.unwrap().to_str().unwrap(), "/custom/mirror");
        assert_eq!(
            config.default_workspace.unwrap().to_str().unwrap(),
            "/custom/workspace"
        );
        assert_eq!(
            config.default_source.unwrap(),
            "https://example.com/manifests"
        );

        let copy_files = config.copy_files.unwrap();
        assert_eq!(copy_files.len(), 2);
        assert_eq!(copy_files[0].source, "file1.txt");
        assert_eq!(copy_files[0].dest, "dest1.txt");
    }

    #[test]
    fn test_user_config_partial_values() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");

        // Create a config with only some values set
        let config_content = r#"
mirror = "/only/mirror/set"
# default_workspace not set
# default_source not set
"#;
        fs::write(&config_path, config_content).unwrap();

        // Load the config
        let result = UserConfig::load_from(&config_path);
        assert!(result.is_ok());

        let config = result.unwrap().unwrap();
        assert!(config.mirror.is_some());
        assert!(config.default_workspace.is_none());
        assert!(config.default_source.is_none());
        assert!(config.copy_files.is_none());
    }

    /// Helper to create a GitConfig for testing resolve_clone_order
    fn make_git(name: &str, git_depends_on: Option<Vec<&str>>) -> GitConfig {
        GitConfig {
            name: name.to_string(),
            url: format!("https://example.com/{}.git", name),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: git_depends_on.map(|v| v.into_iter().map(String::from).collect()),
            build: None,
            documentation_dir: None,
        }
    }

    #[test]
    fn test_resolve_clone_order_empty() {
        let result = resolve_clone_order(&[]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_resolve_clone_order_no_dependencies() {
        let gits = vec![
            make_git("repo-a", None),
            make_git("repo-b", None),
            make_git("repo-c", None),
        ];
        let tiers = resolve_clone_order(&gits).unwrap();
        // All repos should be in a single tier since none have dependencies
        assert_eq!(tiers.len(), 1);
        assert_eq!(tiers[0].len(), 3);
    }

    #[test]
    fn test_resolve_clone_order_linear_chain() {
        // c depends on b, b depends on a
        let gits = vec![
            make_git("a", None),
            make_git("b", Some(vec!["a"])),
            make_git("c", Some(vec!["b"])),
        ];
        let tiers = resolve_clone_order(&gits).unwrap();
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].len(), 1);
        assert_eq!(tiers[0][0].name, "a");
        assert_eq!(tiers[1].len(), 1);
        assert_eq!(tiers[1][0].name, "b");
        assert_eq!(tiers[2].len(), 1);
        assert_eq!(tiers[2][0].name, "c");
    }

    #[test]
    fn test_resolve_clone_order_fan_out() {
        // b, c, d all depend on a (the nested repo pattern)
        let gits = vec![
            make_git("parent", None),
            make_git("parent/child1", Some(vec!["parent"])),
            make_git("parent/child2", Some(vec!["parent"])),
            make_git("parent/child3", Some(vec!["parent"])),
        ];
        let tiers = resolve_clone_order(&gits).unwrap();
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].len(), 1);
        assert_eq!(tiers[0][0].name, "parent");
        assert_eq!(tiers[1].len(), 3);
        let tier1_names: HashSet<String> = tiers[1].iter().map(|g| g.name.clone()).collect();
        assert!(tier1_names.contains("parent/child1"));
        assert!(tier1_names.contains("parent/child2"));
        assert!(tier1_names.contains("parent/child3"));
    }

    #[test]
    fn test_resolve_clone_order_circular_dependency() {
        let gits = vec![
            make_git("a", Some(vec!["b"])),
            make_git("b", Some(vec!["a"])),
        ];
        let result = resolve_clone_order(&gits);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Circular dependency"));
    }

    #[test]
    fn test_resolve_clone_order_missing_dependency() {
        let gits = vec![make_git("a", Some(vec!["nonexistent"]))];
        let result = resolve_clone_order(&gits);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("nonexistent"));
        assert!(err.contains("not defined"));
    }

    #[test]
    fn test_resolve_clone_order_ignores_build_depends_on() {
        // build_depends_on references non-git targets — should not affect clone order
        let gits = vec![GitConfig {
            name: "my-repo".to_string(),
            url: "https://example.com/my-repo.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: Some(vec!["sdk-envsetup".to_string()]),
            git_depends_on: None,
            build: None,
            documentation_dir: None,
        }];
        let tiers = resolve_clone_order(&gits).unwrap();
        assert_eq!(tiers.len(), 1);
        assert_eq!(tiers[0][0].name, "my-repo");
    }

    #[test]
    fn test_resolve_clone_order_multiple_dependencies() {
        // d depends on both b and c; b and c depend on a
        let gits = vec![
            make_git("a", None),
            make_git("b", Some(vec!["a"])),
            make_git("c", Some(vec!["a"])),
            make_git("d", Some(vec!["b", "c"])),
        ];
        let tiers = resolve_clone_order(&gits).unwrap();
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0][0].name, "a");
        assert_eq!(tiers[1].len(), 2);
        assert_eq!(tiers[2].len(), 1);
        assert_eq!(tiers[2][0].name, "d");
    }
}
