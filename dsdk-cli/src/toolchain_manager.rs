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

use crate::config::{self, ToolchainConfig};
use crate::workspace::expand_env_vars;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// Import messages module from main for consistent output
use crate::messages;

/// Architecture detection result
#[derive(Debug, Clone, PartialEq)]
pub struct HostInfo {
    pub os: String,
    pub arch: String,
}

/// Detect host OS and architecture
pub fn detect_host_info() -> HostInfo {
    let os = if cfg!(target_os = "macos") {
        "darwin".to_string()
    } else if cfg!(target_os = "linux") {
        "linux".to_string()
    } else if cfg!(target_os = "windows") {
        "windows".to_string()
    } else {
        "unknown".to_string()
    };

    // Detect architecture at runtime using system calls
    let arch = detect_runtime_arch();

    HostInfo { os, arch }
}

/// Detect the actual runtime architecture using platform-specific methods
fn detect_runtime_arch() -> String {
    // Platform-specific runtime architecture detection
    #[cfg(target_os = "macos")]
    {
        detect_macos_runtime_arch()
    }

    #[cfg(target_os = "linux")]
    {
        detect_linux_runtime_arch()
    }

    #[cfg(target_os = "windows")]
    {
        detect_windows_runtime_arch()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        // Fallback to compile-time detection for unsupported platforms
        detect_compile_time_arch()
    }
}

/// Detect runtime architecture on macOS
#[cfg(target_os = "macos")]
fn detect_macos_runtime_arch() -> String {
    // On macOS, check if we're running under Rosetta emulation
    // If ARCHPREFERENCE is set to "i386,x86_64", we're running x86_64 code on ARM64
    if let Ok(arch_pref) = std::env::var("ARCHPREFERENCE") {
        if arch_pref.contains("i386") || arch_pref.contains("x86_64") {
            // We're likely running under Rosetta, but we want the actual hardware arch
            // Try to get the actual hardware architecture via sysctl
            if let Some(hw_arch) = get_macos_hw_arch() {
                return normalize_arch(&hw_arch);
            }
        }
    }

    // Try to read hardware architecture directly
    if let Some(hw_arch) = get_macos_hw_arch() {
        return normalize_arch(&hw_arch);
    }

    // Fallback to compile-time detection
    detect_compile_time_arch()
}

/// Get macOS hardware architecture using sysctl (if available)
#[cfg(target_os = "macos")]
fn get_macos_hw_arch() -> Option<String> {
    use std::process::Command;

    // Try sysctl hw.optional.arm64 to detect Apple Silicon
    if let Ok(output) = Command::new("sysctl")
        .arg("-n")
        .arg("hw.optional.arm64")
        .output()
    {
        if output.status.success() {
            let result_raw = String::from_utf8_lossy(&output.stdout);
            let result = result_raw.trim();
            if result == "1" {
                return Some("arm64".to_string());
            }
        }
    }

    // Try sysctl hw.machine to get machine type
    if let Ok(output) = Command::new("sysctl").arg("-n").arg("hw.machine").output() {
        if output.status.success() {
            let machine_raw = String::from_utf8_lossy(&output.stdout);
            let machine = machine_raw.trim();
            return Some(machine.to_string());
        }
    }

    None
}

/// Detect runtime architecture on Linux
#[cfg(target_os = "linux")]
fn detect_linux_runtime_arch() -> String {
    // Try reading from /proc/cpuinfo
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        // Look for architecture indicators in cpuinfo
        if cpuinfo.contains("aarch64") || cpuinfo.contains("arm64") {
            return "arm64".to_string();
        }
        if cpuinfo.contains("x86_64") || cpuinfo.contains("AMD64") {
            return "x86_64".to_string();
        }
        // Look for ARM indicators
        if cpuinfo.contains("ARM") || cpuinfo.contains("arm") {
            return "arm64".to_string(); // Modern ARM is typically arm64
        }
    }

    // Try uname -m as fallback (only on Unix-like systems)
    if let Ok(output) = Command::new("uname").arg("-m").output() {
        if output.status.success() {
            let arch_raw = String::from_utf8_lossy(&output.stdout);
            let arch_str = arch_raw.trim();
            return normalize_arch(arch_str);
        }
    }

    // Fallback to compile-time detection
    detect_compile_time_arch()
}

/// Detect runtime architecture on Windows
#[cfg(target_os = "windows")]
fn detect_windows_runtime_arch() -> String {
    // Check PROCESSOR_ARCHITEW6432 first (indicates running 32-bit on 64-bit)
    if let Ok(arch) = std::env::var("PROCESSOR_ARCHITEW6432") {
        return normalize_arch(&arch);
    }

    // Check PROCESSOR_ARCHITECTURE
    if let Ok(arch) = std::env::var("PROCESSOR_ARCHITECTURE") {
        return normalize_arch(&arch);
    }

    // Fallback to compile-time detection
    detect_compile_time_arch()
}

/// Normalize architecture name to standard format
fn normalize_arch(arch_str: &str) -> String {
    match arch_str.to_lowercase().as_str() {
        "x86_64" | "amd64" | "x64" => "x86_64".to_string(),
        "aarch64" | "arm64" | "arm64ec" => "arm64".to_string(),
        "i386" | "i686" | "x86" => "i386".to_string(),
        "arm14t2" | "arm64t2" => "arm64".to_string(), // Apple Silicon variants
        other => other.to_string(),
    }
}

/// Fallback compile-time architecture detection
fn detect_compile_time_arch() -> String {
    if cfg!(target_arch = "x86_64") {
        "x86_64".to_string()
    } else if cfg!(target_arch = "aarch64") {
        "arm64".to_string()
    } else if cfg!(target_arch = "x86") {
        "i386".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Extract a friendly display name from a toolchain filename
/// Example: "arm-gnu-toolchain-14.3.rel1-darwin-arm64-arm-none-eabi.tar.xz" -> "arm-none-eabi 14.3.rel1"
fn extract_friendly_name(full_name: &str) -> String {
    // Remove file extension(s)
    let name_without_ext = full_name
        .trim_end_matches(".tar.xz")
        .trim_end_matches(".tar.gz")
        .trim_end_matches(".zip");

    // Try to extract version and target from common patterns
    // Pattern: <prefix>-<version>-<os>-<arch>-<target>
    let parts: Vec<&str> = name_without_ext.split('-').collect();

    // Look for version number (contains dots and "rel" suffix often)
    let version_idx = parts
        .iter()
        .position(|p| p.contains('.') || p.contains("rel"));

    // Look for target (usually the last meaningful part like "arm-none-eabi")
    if parts.len() >= 3 {
        if let Some(ver_idx) = version_idx {
            // Try to get target from the end (last 2-3 parts if they look like a target)
            let target_parts: Vec<&str> = parts[parts.len().saturating_sub(3)..].to_vec();
            let target = target_parts.join("-");
            let version = parts[ver_idx];

            // Return "target version" format
            return format!("{} {}", target, version);
        }
    }

    // Fallback: use the full name without extension
    name_without_ext.to_string()
}

/// Strip archive extension from a toolchain name to get the base directory name
/// Handles common archive formats: .tar.xz, .tar.gz, .tar.bz2, .tar, .zip, .sh
/// Also handles cases where name may contain URL path components
///
/// Examples:
/// - "arm-gnu-toolchain-14.3.rel1-darwin-arm64-arm-none-eabi.tar.xz" -> "arm-gnu-toolchain-14.3.rel1-darwin-arm64-arm-none-eabi"
/// - "path/to/toolchain.tar.gz" -> "toolchain"
/// - "rust-1.75.0-x86_64-unknown-linux-gnu.tar.gz" -> "rust-1.75.0-x86_64-unknown-linux-gnu"
fn strip_archive_extension(name: &str) -> String {
    // First, extract just the filename if there's a path
    let filename = name.rsplit('/').next().unwrap_or(name);

    // Strip known archive extensions
    let without_ext = filename
        .trim_end_matches(".tar.xz")
        .trim_end_matches(".tar.gz")
        .trim_end_matches(".tar.bz2")
        .trim_end_matches(".tar")
        .trim_end_matches(".zip")
        .trim_end_matches(".sh");

    without_ext.to_string()
}

/// Manager for downloading and extracting toolchains
pub struct ToolchainManager {
    workspace_path: PathBuf,
    mirror_path: PathBuf,
}

impl ToolchainManager {
    /// Create a new toolchain manager
    pub fn new(workspace_path: PathBuf, mirror_path: PathBuf) -> Self {
        Self {
            workspace_path,
            mirror_path,
        }
    }

    /// Construct full download URL by combining base URL and filename
    /// Handles trailing slashes properly - adds one if needed
    /// Special case: if URL already contains the filename or filename is empty, use URL as-is
    fn construct_download_url(&self, base_url: &str, filename: &str) -> String {
        // If filename is empty, the URL is already complete
        if filename.is_empty() {
            return base_url.to_string();
        }

        // If the URL already ends with the filename, it's a direct file URL
        if base_url.ends_with(filename) || base_url == filename {
            return base_url.to_string();
        }

        // Otherwise, combine base URL with filename
        if base_url.ends_with('/') {
            format!("{}{}", base_url, filename)
        } else {
            format!("{}/{}", base_url, filename)
        }
    }

    /// Install all toolchains from configuration
    pub fn install_toolchains(
        &self,
        toolchains: Option<&Vec<ToolchainConfig>>,
        force: bool,
        symlink: bool,
        cert_validation: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let toolchains = match toolchains {
            Some(toolchains) => toolchains,
            None => {
                messages::info("No toolchains configured in sdk.yml");
                return Ok(());
            }
        };

        if toolchains.is_empty() {
            messages::info("No toolchains to install");
            return Ok(());
        }

        // Detect host OS and architecture
        let host_info = detect_host_info();
        messages::status(&format!(
            "Detected host: {} {}",
            host_info.os, host_info.arch
        ));

        // Add debug information to help diagnose architecture detection issues
        let compile_time_arch = if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else if cfg!(target_arch = "x86") {
            "x86"
        } else {
            "unknown"
        };

        messages::verbose(&format!(
            "Debug: Runtime arch: {}, Compile-time arch: {}",
            host_info.arch, compile_time_arch
        ));

        // Show platform-specific debug info
        #[cfg(target_os = "macos")]
        {
            if let Ok(arch_pref) = std::env::var("ARCHPREFERENCE") {
                messages::verbose(&format!("Debug: macOS ARCHPREFERENCE: {}", arch_pref));
            }
        }

        #[cfg(target_os = "windows")]
        {
            if let Ok(proc_arch) = std::env::var("PROCESSOR_ARCHITECTURE") {
                messages::verbose(&format!(
                    "Debug: Windows PROCESSOR_ARCHITECTURE: {}",
                    proc_arch
                ));
            }
            if let Ok(proc_arch_wow) = std::env::var("PROCESSOR_ARCHITEW6432") {
                messages::verbose(&format!(
                    "Debug: Windows PROCESSOR_ARCHITEW6432: {}",
                    proc_arch_wow
                ));
            }
        }

        // Filter toolchains based on host OS and architecture
        let applicable_toolchains: Vec<&ToolchainConfig> = toolchains
            .iter()
            .filter(|tc| self.is_toolchain_applicable(tc, &host_info))
            .collect();

        if applicable_toolchains.is_empty() {
            messages::info(&format!(
                "No toolchains applicable for {} {}",
                host_info.os, host_info.arch
            ));
            return Ok(());
        }

        // Validate destinations for duplicates
        self.validate_toolchain_destinations(&applicable_toolchains)?;

        // Create mirror directory if it doesn't exist
        if let Err(e) = fs::create_dir_all(&self.mirror_path) {
            return Err(format!("Failed to create mirror directory: {}", e).into());
        }

        messages::status(&format!(
            "Installing {} applicable toolchain(s)...",
            applicable_toolchains.len()
        ));

        // Show installation plan in verbose mode
        if symlink {
            messages::verbose("Installation plan (--symlink mode):");
            for toolchain in &applicable_toolchains {
                let mirror_path = if let Some(ref mirror_dest) = toolchain.mirror_destination {
                    format!("{}/{}", self.mirror_path.display(), mirror_dest)
                } else {
                    let base_name = strip_archive_extension(&toolchain.get_name());
                    format!("{}/toolchains/{}", self.mirror_path.display(), base_name)
                };
                let workspace_path = format!(
                    "{}/{}",
                    self.workspace_path.display(),
                    toolchain.destination
                );
                messages::verbose(&format!("  • {} → {}", workspace_path, mirror_path));
            }
            messages::verbose("");
        } else {
            messages::verbose("Installation plan (direct mode):");
            for toolchain in &applicable_toolchains {
                let workspace_path = format!(
                    "{}/{}",
                    self.workspace_path.display(),
                    toolchain.destination
                );
                messages::verbose(&format!("  • Extract to: {}", workspace_path));
            }
            messages::verbose("");
        }

        let mut failed_toolchains = Vec::new();
        for toolchain in applicable_toolchains {
            if let Err(e) =
                self.install_single_toolchain(toolchain, force, symlink, cert_validation)
            {
                messages::error(&format!(
                    "Failed to install toolchain '{}': {}",
                    toolchain.get_name(),
                    e
                ));
                failed_toolchains.push(toolchain.get_name().clone());
                // Continue with other toolchains instead of failing completely
            }
        }

        if !failed_toolchains.is_empty() {
            return Err(format!(
                "Failed to install {} toolchain(s): {}",
                failed_toolchains.len(),
                failed_toolchains.join(", ")
            )
            .into());
        }

        Ok(())
    }

    /// Check if a toolchain is applicable for the current host
    fn is_toolchain_applicable(&self, toolchain: &ToolchainConfig, host_info: &HostInfo) -> bool {
        // Check OS filter
        if let Some(ref required_os) = toolchain.os {
            if required_os != &host_info.os {
                return false;
            }
        }

        // Check architecture filter
        if let Some(ref required_arch) = toolchain.arch {
            if required_arch != &host_info.arch {
                return false;
            }
        }

        true
    }

    /// Validate that no toolchains have duplicate destination paths
    fn validate_toolchain_destinations(
        &self,
        toolchains: &[&ToolchainConfig],
    ) -> Result<(), Box<dyn std::error::Error>> {
        use std::collections::HashMap;

        let mut destination_map: HashMap<String, Vec<String>> = HashMap::new();

        // Collect toolchains by their destination paths
        for toolchain in toolchains {
            destination_map
                .entry(toolchain.destination.clone())
                .or_default()
                .push(toolchain.get_name().clone());
        }

        // Check for duplicates
        let mut errors = Vec::new();
        for (destination, toolchain_names) in destination_map {
            if toolchain_names.len() > 1 {
                errors.push(format!(
                    "Multiple toolchains target the same destination '{}': {}",
                    destination,
                    toolchain_names.join(", ")
                ));
            }
        }

        if !errors.is_empty() {
            let error_message = format!("Destination validation failed:\n{}", errors.join("\n"));
            return Err(error_message.into());
        }

        Ok(())
    }

    /// Install a single toolchain
    fn install_single_toolchain(
        &self,
        toolchain: &ToolchainConfig,
        force: bool,
        symlink: bool,
        cert_validation: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let friendly_name = extract_friendly_name(&toolchain.get_name());
        println!("Installing {}...", friendly_name);

        // If --symlink is provided, always use symlink mode (reuse from mirror)
        // mirror_destination can be omitted and will fall back to destination
        if symlink {
            self.install_toolchain_with_symlink(toolchain, force, cert_validation)?;
        } else {
            self.install_toolchain_direct(toolchain, force, cert_validation)?;
        }

        messages::success(&format!("Installed {}", friendly_name));
        Ok(())
    }

    /// Install toolchain directly to workspace (traditional mode)
    fn install_toolchain_direct(
        &self,
        toolchain: &ToolchainConfig,
        force: bool,
        cert_validation: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Determine destination path
        let dest_path = self
            .workspace_path
            .join(expand_env_vars(&toolchain.destination));

        // Check if destination already exists
        if dest_path.exists() {
            if force {
                messages::info(&format!(
                    "Destination {} exists, removing due to --force",
                    dest_path.display()
                ));
                if let Err(e) = fs::remove_dir_all(&dest_path) {
                    return Err(
                        format!("Failed to remove existing destination directory: {}", e).into(),
                    );
                }
            } else {
                messages::info("Already installed, skipping (use --force to reinstall)");
                return Ok(());
            }
        }

        // Get and extract archive
        let archive_path = self.ensure_archive_downloaded(toolchain, cert_validation)?;

        // Create destination directory
        if let Err(e) = fs::create_dir_all(&dest_path) {
            return Err(format!("Failed to create destination directory: {}", e).into());
        }

        // Extract archive
        let dest_dir_name = dest_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("toolchain");
        messages::verbose(&format!("Extracting to {}...", dest_dir_name));
        self.extract_archive(&archive_path, &dest_path, toolchain.strip_components)?;

        // Execute post-install commands
        self.execute_post_install_commands(toolchain, &dest_path)?;

        Ok(())
    }

    /// Install toolchain to mirror and create symlink in workspace
    fn install_toolchain_with_symlink(
        &self,
        toolchain: &ToolchainConfig,
        force: bool,
        cert_validation: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let workspace_dest_path = self
            .workspace_path
            .join(expand_env_vars(&toolchain.destination));

        // Determine mirror destination path:
        // - If mirror_destination is explicitly specified, use it (backward compatible)
        // - Otherwise, use toolchains/{name-without-extension} for unique identification
        let (mirror_relative_path, is_custom_location) =
            if let Some(ref mirror_dest) = toolchain.mirror_destination {
                // User specified mirror_destination, respect it
                (mirror_dest.clone(), true)
            } else {
                // Default behavior: use unique toolchain name as subdirectory
                // This prevents conflicts when multiple workspaces use same destination
                let base_name = strip_archive_extension(&toolchain.get_name());
                (format!("toolchains/{}", base_name), false)
            };
        let mirror_dest_path = self.mirror_path.join(&mirror_relative_path);

        // Show mirror path resolution in verbose mode
        messages::verbose(&format!(
            "Mirror path: {} ({})",
            mirror_dest_path.display(),
            if is_custom_location {
                "from config"
            } else {
                "auto-generated"
            }
        ));

        // Check if workspace symlink already exists
        if let Ok(metadata) = fs::symlink_metadata(&workspace_dest_path) {
            if metadata.file_type().is_symlink() {
                if force {
                    messages::info(&format!(
                        "Symlink {} exists, removing due to --force",
                        workspace_dest_path.display()
                    ));
                    if let Err(e) = fs::remove_file(&workspace_dest_path) {
                        return Err(format!("Failed to remove existing symlink: {}", e).into());
                    }
                } else {
                    messages::info("Already installed, skipping (use --force to reinstall)");
                    return Ok(());
                }
            } else if force {
                messages::info(&format!(
                    "Destination {} exists and is not a symlink, removing due to --force",
                    workspace_dest_path.display()
                ));
                if workspace_dest_path.is_dir() {
                    if let Err(e) = fs::remove_dir_all(&workspace_dest_path) {
                        return Err(format!("Failed to remove existing directory: {}", e).into());
                    }
                } else if let Err(e) = fs::remove_file(&workspace_dest_path) {
                    return Err(format!("Failed to remove existing file: {}", e).into());
                }
            } else {
                return Err(format!(
                    "Destination path {} exists but is not a symlink. Use --force to remove it or install without --symlink",
                    workspace_dest_path.display()
                ).into());
            }
        }

        // Check if mirror destination already exists
        if mirror_dest_path.exists() {
            if force {
                messages::info(&format!(
                    "Mirror destination {} exists, removing due to --force",
                    mirror_dest_path.display()
                ));
                if let Err(e) = fs::remove_dir_all(&mirror_dest_path) {
                    return Err(format!(
                        "Failed to remove existing mirror destination directory: {}",
                        e
                    )
                    .into());
                }
            } else {
                // First, check if we need to download to know what type of toolchain this is
                let archive_path = self.ensure_archive_downloaded(toolchain, cert_validation)?;
                let is_shell_script_toolchain = self.is_shell_script(&archive_path);

                // Validate that the existing installation is actually valid
                let mut installation_valid = false;
                let mut found_shell_script = false;

                if let Ok(entries) = fs::read_dir(&mirror_dest_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        // Check if this is a shell script by content or extension
                        if path.is_file()
                            && (path.extension().and_then(|s| s.to_str()) == Some("sh")
                                || self.is_shell_script(&path))
                        {
                            found_shell_script = true;
                            // Validate the shell script
                            if let Err(e) = self.validate_shell_script(&path) {
                                messages::info(&format!(
                                    "Mirror destination for '{}' exists but shell script '{}' appears corrupted or invalid: {}",
                                    toolchain.get_name(),
                                    path.display(),
                                    e
                                ));
                                messages::info(&format!(
                                    "Removing invalid installation at {}",
                                    mirror_dest_path.display()
                                ));
                                if let Err(remove_err) = fs::remove_dir_all(&mirror_dest_path) {
                                    return Err(format!(
                                        "Failed to remove invalid mirror destination: {}",
                                        remove_err
                                    )
                                    .into());
                                }
                                break;
                            } else {
                                // Found a valid shell script
                                installation_valid = true;
                            }
                        }
                    }
                }

                // For shell script toolchains, we MUST find a valid shell script
                // For archive toolchains, check if directory has content
                if is_shell_script_toolchain {
                    if !found_shell_script {
                        messages::info(&format!(
                            "Mirror destination for '{}' exists but no shell script found, removing",
                            toolchain.get_name()
                        ));
                        if let Err(e) = fs::remove_dir_all(&mirror_dest_path) {
                            return Err(format!(
                                "Failed to remove invalid mirror destination: {}",
                                e
                            )
                            .into());
                        }
                    } else if installation_valid {
                        let friendly_name = extract_friendly_name(&toolchain.get_name());
                        messages::info(&format!(
                            "Toolchain {} already installed in mirror",
                            friendly_name
                        ));
                        messages::verbose(&format!("  Location: {}", mirror_dest_path.display()));
                        // Still create the symlink
                        self.create_symlink(&mirror_dest_path, &workspace_dest_path)?;
                        return Ok(());
                    }
                } else {
                    // For archive toolchains, check if directory is non-empty
                    if let Ok(mut entries) = fs::read_dir(&mirror_dest_path) {
                        if entries.next().is_some() {
                            let friendly_name = extract_friendly_name(&toolchain.get_name());
                            messages::info(&format!(
                                "Toolchain {} already installed in mirror",
                                friendly_name
                            ));
                            messages::verbose(&format!(
                                "  Location: {}",
                                mirror_dest_path.display()
                            ));
                            // Still create the symlink
                            self.create_symlink(&mirror_dest_path, &workspace_dest_path)?;
                            return Ok(());
                        } else {
                            messages::info(&format!(
                                "Mirror destination for '{}' exists but is empty, removing",
                                toolchain.get_name()
                            ));
                            if let Err(e) = fs::remove_dir_all(&mirror_dest_path) {
                                return Err(format!(
                                    "Failed to remove empty mirror destination: {}",
                                    e
                                )
                                .into());
                            }
                        }
                    }
                }
            }
        }

        // Get and extract archive to mirror
        let archive_path = self.ensure_archive_downloaded(toolchain, cert_validation)?;

        // Create mirror destination directory
        if let Err(e) = fs::create_dir_all(&mirror_dest_path) {
            return Err(format!("Failed to create mirror destination directory: {}", e).into());
        }

        // Extract archive to mirror
        let dest_dir_name = mirror_dest_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("mirror");
        messages::verbose(&format!("Extracting to mirror ({})", dest_dir_name));

        // Always respect strip_components setting to properly extract the archive contents
        // The unique mirror path (based on archive name) prevents conflicts
        self.extract_archive(&archive_path, &mirror_dest_path, toolchain.strip_components)?;

        // Execute post-install commands on the mirror installation
        self.execute_post_install_commands(toolchain, &mirror_dest_path)?;

        // Create symlink from workspace to mirror
        self.create_symlink(&mirror_dest_path, &workspace_dest_path)?;

        Ok(())
    }

    /// Ensure archive is downloaded and return its path
    fn ensure_archive_downloaded(
        &self,
        toolchain: &ToolchainConfig,
        cert_validation: Option<&str>,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let archive_path = self.mirror_path.join(toolchain.get_name());

        // Remove stale zero-byte files left by previously failed downloads
        if archive_path.exists() {
            if let Ok(metadata) = fs::metadata(&archive_path) {
                if metadata.len() == 0 {
                    messages::info(&format!(
                        "Removing stale empty file from mirror: {}",
                        archive_path.display()
                    ));
                    let _ = fs::remove_file(&archive_path);
                }
            }
        }

        // Download archive if not already in mirror
        if !archive_path.exists() {
            let expanded_url = expand_env_vars(&toolchain.url);
            let download_url = self.construct_download_url(&expanded_url, &toolchain.get_name());
            messages::status(&format!(
                "Downloading {} from {}...",
                toolchain.get_name(),
                download_url
            ));
            self.download_toolchain(&download_url, &archive_path, cert_validation)?;
        } else {
            messages::verbose(&format!("Using cached: {}", archive_path.display()));
        }

        Ok(archive_path)
    }

    /// Expand environment variables in a value string for toolchain configuration
    ///
    /// Supports cim-specific variables:
    /// - `$PWD` / `${PWD}` - expands to toolchain installation directory (dest_path)
    /// - `$WORKSPACE` / `${WORKSPACE}` - expands to workspace root directory
    /// - `$HOME` / `${HOME}` - expands to user home directory
    /// - Standard environment variables via `std::env::var`
    ///
    /// Both `$VAR` and `${VAR}` syntax are supported.
    fn expand_toolchain_env_vars(&self, value: &str, dest_path: &Path) -> String {
        let mut result = value.to_string();

        // First, replace cim-specific variables with their absolute paths
        // We need to handle both ${VAR} and $VAR syntax

        // Replace $PWD and ${PWD} with dest_path
        let pwd_value = dest_path.to_string_lossy().to_string();
        result = result.replace("${PWD}", &pwd_value);
        result = result.replace("$PWD", &pwd_value);

        // Replace $WORKSPACE and ${WORKSPACE} with workspace_path
        let workspace_value = self.workspace_path.to_string_lossy().to_string();
        result = result.replace("${WORKSPACE}", &workspace_value);
        result = result.replace("$WORKSPACE", &workspace_value);

        // Now handle standard environment variable expansion (including $HOME)
        // This follows the same pattern as expand_env_vars in main.rs

        // Handle tilde expansion first
        if result.starts_with("~/") || result == "~" {
            let home = env::var("HOME").or_else(|_| env::var("USERPROFILE"));
            if let Ok(home) = home {
                if result == "~" {
                    result = home;
                } else {
                    let rest_of_path = &result[2..];
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
                let value = if var_name == "HOME" {
                    env::var("HOME").or_else(|_| env::var("USERPROFILE"))
                } else {
                    env::var(var_name)
                };

                if let Ok(value) = value {
                    result.replace_range(start..start + end + 2, &value);
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Handle ${VAR} syntax for remaining variables
        while let Some(start) = result.find("${") {
            if let Some(end) = result[start..].find('}') {
                let var_name = &result[start + 2..start + end];
                let value = if var_name == "HOME" {
                    env::var("HOME").or_else(|_| env::var("USERPROFILE"))
                } else {
                    env::var(var_name)
                };

                if let Ok(value) = value {
                    result.replace_range(start..start + end + 1, &value);
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Handle $VAR syntax (without braces) for remaining variables
        let mut chars: Vec<char> = result.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '$' && i + 1 < chars.len() {
                let var_start = i + 1;
                let mut var_end = var_start;

                while var_end < chars.len()
                    && (chars[var_end].is_alphanumeric() || chars[var_end] == '_')
                {
                    var_end += 1;
                }

                if var_end > var_start {
                    let var_name: String = chars[var_start..var_end].iter().collect();
                    let value = if var_name == "HOME" {
                        env::var("HOME").or_else(|_| env::var("USERPROFILE"))
                    } else {
                        env::var(&var_name)
                    };

                    if let Ok(value) = value {
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

    /// Create symlink from mirror to workspace
    fn create_symlink(
        &self,
        mirror_path: &Path,
        workspace_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Create parent directories for the symlink
        if let Some(parent) = workspace_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                return Err(format!("Failed to create parent directory for symlink: {}", e).into());
            }
        }

        // Create symlink
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            if let Err(e) = symlink(mirror_path, workspace_path) {
                return Err(format!("Failed to create symlink: {}", e).into());
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::fs::symlink_dir;
            if let Err(e) = symlink_dir(mirror_path, workspace_path) {
                return Err(format!("Failed to create symlink: {}", e).into());
            }
        }

        messages::verbose(&format!(
            "Created symlink: {} -> {}",
            workspace_path.display(),
            mirror_path.display()
        ));

        Ok(())
    }

    /// Execute post-install commands for a toolchain
    fn execute_post_install_commands(
        &self,
        toolchain: &ToolchainConfig,
        dest_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if toolchain.post_install_commands.is_none()
            || toolchain.post_install_commands.as_ref().unwrap().is_empty()
        {
            return Ok(());
        }

        let commands = toolchain.post_install_commands.as_ref().unwrap();

        // Build environment variables map from toolchain configuration
        let mut env_vars = HashMap::new();
        if let Some(ref env_config) = toolchain.environment {
            for (key, value) in env_config {
                let expanded = self.expand_toolchain_env_vars(value, dest_path);
                env_vars.insert(key.clone(), expanded);
            }

            // Show expanded environment variables in verbose mode
            if !env_vars.is_empty() {
                messages::verbose("Environment variables:");
                for (key, value) in &env_vars {
                    messages::verbose(&format!("  {}={}", key, value));
                }
            }
        }

        // Determine shell based on user config or platform defaults
        let (shell, shell_arg) = if let Ok(Some(user_config)) = crate::config::UserConfig::load() {
            // Use user-configured shell if available
            if let (Some(s), Some(a)) = (user_config.shell, user_config.shell_arg) {
                (s, a)
            } else {
                get_default_shell()
            }
        } else {
            get_default_shell()
        };

        // On Windows, check if git-credential-manager is available for authenticated repos
        #[cfg(target_os = "windows")]
        {
            if let Ok(output) = Command::new("git")
                .args(&["credential-manager", "--version"])
                .output()
            {
                if !output.status.success() {
                    messages::info(
                        "Git Credential Manager not detected. If you encounter authentication issues,\n\
                         please install it from: https://github.com/git-ecosystem/git-credential-manager/releases"
                    );
                }
            }
        }

        messages::status(&format!(
            "Running {} post-install command(s) for {} using shell: {}...",
            commands.len(),
            toolchain.get_name(),
            shell
        ));

        for (idx, command) in commands.iter().enumerate() {
            messages::verbose(&format!(
                "Post-install command {}/{}: {}",
                idx + 1,
                commands.len(),
                command
            ));

            let mut cmd = Command::new(&shell);
            cmd.arg(&shell_arg).arg(command).current_dir(dest_path);

            // Apply environment variables if configured
            for (key, value) in &env_vars {
                cmd.env(key, value);
            }

            let output = cmd.output()?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                return Err(format!(
                    "Post-install command {} failed for {}: {}\nStdout: {}",
                    idx + 1,
                    toolchain.get_name(),
                    stderr,
                    stdout
                )
                .into());
            }

            // Print command output if verbose
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.is_empty() {
                messages::verbose(&format!("Output: {}", stdout.trim()));
            }
        }

        Ok(())
    }

    /// Download toolchain archive from URL
    fn download_toolchain(
        &self,
        url: &str,
        dest_path: &Path,
        cert_validation: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Use a wget-like user agent that is generally accepted by most servers
        // This avoids user-agent blocking from servers like rustup.rs
        let user_agent = "Wget/1.21.2";

        // Determine certificate validation mode
        let (mode, show_warning) = config::get_cert_validation_mode(cert_validation);

        // Show security warning if needed
        if show_warning {
            if mode == "relaxed" {
                messages::info("Certificate validation is DISABLED - connections are vulnerable to MITM attacks");
                messages::info("This should only be used in trusted networks or for testing");
            } else if mode == "auto" {
                messages::verbose(
                    "Using auto mode: will try strict validation first, then fallback to relaxed",
                );
            }
        }

        // Build client list based on mode
        let mut clients: Vec<reqwest::blocking::Client> = Vec::new();

        match mode.as_str() {
            "strict" => {
                // Only strict validation
                clients.push(
                    reqwest::blocking::Client::builder()
                        .user_agent(user_agent)
                        .timeout(std::time::Duration::from_secs(300))
                        .build()?,
                );
            }
            "relaxed" => {
                // Only insecure client
                clients.push(
                    reqwest::blocking::Client::builder()
                        .user_agent(user_agent)
                        .timeout(std::time::Duration::from_secs(300))
                        .danger_accept_invalid_certs(true)
                        .build()?,
                );
            }
            "auto" => {
                // Try strict first, then relaxed
                clients.push(
                    reqwest::blocking::Client::builder()
                        .user_agent(user_agent)
                        .timeout(std::time::Duration::from_secs(300))
                        .build()?,
                );
                clients.push(
                    reqwest::blocking::Client::builder()
                        .user_agent(user_agent)
                        .timeout(std::time::Duration::from_secs(300))
                        .danger_accept_invalid_certs(true)
                        .build()?,
                );
            }
            _ => {
                return Err(format!(
                    "Invalid cert_validation mode: {}. Use 'strict', 'relaxed', or 'auto'",
                    mode
                )
                .into());
            }
        }

        let mut last_error = None;

        for (i, client) in clients.iter().enumerate() {
            match client.get(url).send() {
                Ok(response) => {
                    if response.status().is_success() {
                        if mode == "auto" && i == 1 {
                            messages::info("Strict SSL verification failed, used relaxed mode");
                            messages::info("Consider fixing SSL certificates or use --cert-validation=relaxed explicitly");
                        }
                        messages::verbose(&format!(
                            "Download successful using client configuration {}",
                            i + 1
                        ));
                        let content = response.bytes()?;
                        fs::write(dest_path, content)?;
                        return Ok(());
                    } else {
                        let error = format!(
                            "HTTP error {}: {}",
                            response.status(),
                            response.status().canonical_reason().unwrap_or("Unknown")
                        );
                        last_error = Some(error);
                    }
                }
                Err(e) => {
                    if mode == "auto" && i == 0 {
                        messages::verbose(
                            "Standard SSL verification failed, trying with relaxed SSL settings...",
                        );
                    }
                    last_error = Some(format!("Request failed: {}", e));
                }
            }
        }

        // In strict mode, provide helpful error message about enabling relaxed mode
        if mode == "strict" {
            let error_msg = format!(
                "Download failed with strict SSL validation: {}.\n\
                If you're behind a corporate proxy with SSL inspection, you can:\n\
                1. Use --cert-validation=relaxed flag (INSECURE, testing only)\n\
                2. Set cert_validation = \"auto\" in ~/.config/cim/config.toml\n\
                3. Fix your SSL certificate chain",
                last_error.as_ref().unwrap_or(&"Unknown error".to_string())
            );
            return Err(error_msg.into());
        }

        // For relaxed and auto modes, try wget as fallback
        if mode == "relaxed" || mode == "auto" {
            messages::verbose("HTTP clients failed, trying wget as fallback...");
            if let Ok(output) = Command::new("wget")
                .arg("--no-check-certificate")
                .arg(format!("--user-agent={}", user_agent))
                .arg("--timeout=300")
                .arg("-O")
                .arg(dest_path)
                .arg(url)
                .output()
            {
                if output.status.success() {
                    messages::verbose("Download successful using wget");
                    if mode == "auto" {
                        messages::info(
                            "Download succeeded using wget with disabled certificate validation",
                        );
                    }
                    return Ok(());
                } else {
                    messages::verbose(&format!(
                        "wget failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
                    // Remove partial/empty file left by wget
                    if dest_path.exists() {
                        let _ = fs::remove_file(dest_path);
                    }
                }
            }

            // If wget is not available or failed, try curl
            messages::verbose("wget not available or failed, trying curl as fallback...");
            if let Ok(output) = Command::new("curl")
                .arg("--insecure")
                .arg("--user-agent")
                .arg(user_agent)
                .arg("--max-time")
                .arg("300")
                .arg("-L") // follow redirects
                .arg("-o")
                .arg(dest_path)
                .arg(url)
                .output()
            {
                if output.status.success() {
                    messages::verbose("Download successful using curl");
                    if mode == "auto" {
                        messages::info(
                            "Download succeeded using curl with disabled certificate validation",
                        );
                    }
                    return Ok(());
                } else {
                    messages::verbose(&format!(
                        "curl failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
                    // Remove partial/empty file left by curl
                    if dest_path.exists() {
                        let _ = fs::remove_file(dest_path);
                    }
                }
            }
        }

        // All methods failed
        Err(format!(
            "All download methods failed. Last error: {}. Please check your network connection and SSL certificates.",
            last_error.unwrap_or_else(|| "Unknown error".to_string())
        )
        .into())
    }

    /// Extract archive to destination directory
    fn extract_archive(
        &self,
        archive_path: &Path,
        dest_path: &Path,
        strip_components: Option<u32>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let archive_name = archive_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        // Check if it's a shell script first (by content), before checking extensions
        if self.is_shell_script(archive_path) {
            // For shell scripts, just copy to destination
            return self.copy_script(archive_path, dest_path);
        }

        if archive_name.ends_with(".tar.xz") || archive_name.ends_with(".txz") {
            self.extract_tar_xz(archive_path, dest_path, strip_components)
        } else if archive_name.ends_with(".tar.gz") || archive_name.ends_with(".tgz") {
            self.extract_tar_gz(archive_path, dest_path, strip_components)
        } else if archive_name.ends_with(".tar") {
            self.extract_tar(archive_path, dest_path, strip_components)
        } else if archive_name.ends_with(".zip") {
            self.extract_zip(archive_path, dest_path, strip_components)
        } else {
            Err(format!("Unsupported archive format: {}", archive_name).into())
        }
    }

    /// Check if a file is a shell script by checking for shebang
    fn is_shell_script(&self, path: &Path) -> bool {
        if let Ok(content) = fs::read_to_string(path) {
            content
                .lines()
                .next()
                .is_some_and(|line| line.starts_with("#!"))
        } else {
            false
        }
    }

    /// Extract .tar.xz archive
    fn extract_tar_xz(
        &self,
        archive_path: &Path,
        dest_path: &Path,
        strip_components: Option<u32>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut cmd = Command::new("tar");
        cmd.arg("-xf").arg(archive_path).arg("-C").arg(dest_path);

        if let Some(strip) = strip_components {
            cmd.arg(format!("--strip-components={}", strip));
        }

        let output = cmd.output()?;
        if !output.status.success() {
            return Err(format!(
                "tar extraction failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        Ok(())
    }

    /// Extract .tar.gz archive
    fn extract_tar_gz(
        &self,
        archive_path: &Path,
        dest_path: &Path,
        strip_components: Option<u32>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut cmd = Command::new("tar");
        cmd.arg("-xzf").arg(archive_path).arg("-C").arg(dest_path);

        if let Some(strip) = strip_components {
            cmd.arg(format!("--strip-components={}", strip));
        }

        let output = cmd.output()?;
        if !output.status.success() {
            return Err(format!(
                "tar extraction failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        Ok(())
    }

    /// Extract .tar archive
    fn extract_tar(
        &self,
        archive_path: &Path,
        dest_path: &Path,
        strip_components: Option<u32>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut cmd = Command::new("tar");
        cmd.arg("-xf").arg(archive_path).arg("-C").arg(dest_path);

        if let Some(strip) = strip_components {
            cmd.arg(format!("--strip-components={}", strip));
        }

        let output = cmd.output()?;
        if !output.status.success() {
            return Err(format!(
                "tar extraction failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        Ok(())
    }

    /// Extract .zip archive
    fn extract_zip(
        &self,
        archive_path: &Path,
        dest_path: &Path,
        _strip_components: Option<u32>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Note: strip_components is not directly supported by unzip
        // We could implement this by listing contents and filtering, but for now
        // we'll just extract directly

        let output = Command::new("unzip")
            .arg("-o") // overwrite files
            .arg("-q") // quiet mode
            .arg(archive_path)
            .arg("-d")
            .arg(dest_path)
            .output()?;

        if !output.status.success() {
            return Err(format!(
                "unzip extraction failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        Ok(())
    }

    /// Validate that a shell script is actually a shell script, not HTML or other content
    fn validate_shell_script(&self, script_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let content = fs::read_to_string(script_path)?;
        let first_line = content.lines().next().unwrap_or("");

        // Check if it starts with a shebang
        if !first_line.starts_with("#!") {
            return Err(format!(
                "Shell script does not start with a shebang (#!). First line: {}",
                first_line
            )
            .into());
        }

        // Check if it looks like HTML (common indicator of user-agent blocking)
        if content.contains("<!doctype")
            || content.contains("<html")
            || content.contains("<!DOCTYPE")
        {
            return Err(
                "Downloaded file appears to be HTML instead of a shell script. \
                 This usually indicates the server blocked the request (user-agent or IP restriction). \
                 Try downloading manually with: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
                    .into(),
            );
        }

        Ok(())
    }

    /// Copy shell script to destination directory
    fn copy_script(
        &self,
        script_path: &Path,
        dest_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Validate that the downloaded file is actually a shell script
        self.validate_shell_script(script_path)?;

        // Get the script filename
        let script_name = script_path
            .file_name()
            .ok_or("Failed to get script filename")?;

        let dest_file = dest_path.join(script_name);

        // Copy the script
        fs::copy(script_path, &dest_file)?;

        // Make it executable on Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o755);
            fs::set_permissions(&dest_file, perms)?;
        }

        Ok(())
    }
}

/// Get the default shell and shell argument for the current platform
fn get_default_shell() -> (String, String) {
    #[cfg(target_os = "windows")]
    {
        ("cmd".to_string(), "/C".to_string())
    }

    #[cfg(not(target_os = "windows"))]
    {
        ("bash".to_string(), "-c".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_test_toolchain(name: &str, destination: &str) -> ToolchainConfig {
        ToolchainConfig {
            name: Some(name.to_string()),
            url: format!("https://example.com/{}.tar.gz", name),
            destination: destination.to_string(),
            strip_components: None,
            os: None,
            arch: None,
            sha256: None,
            mirror_destination: None,
            environment: None,
            post_install_commands: None,
        }
    }

    fn create_test_manager() -> ToolchainManager {
        ToolchainManager::new(PathBuf::from("/workspace"), PathBuf::from("/mirror"))
    }

    #[test]
    fn test_validate_no_duplicate_destinations() {
        let manager = create_test_manager();
        let toolchain1 = create_test_toolchain("gcc", "toolchains/gcc");
        let toolchain2 = create_test_toolchain("clang", "toolchains/clang");
        let toolchain3 = create_test_toolchain("llvm", "toolchains/llvm");

        let toolchains = vec![&toolchain1, &toolchain2, &toolchain3];
        let result = manager.validate_toolchain_destinations(&toolchains);

        assert!(
            result.is_ok(),
            "Validation should pass when no duplicates exist"
        );
    }

    #[test]
    fn test_validate_duplicate_destinations_error() {
        let manager = create_test_manager();
        let toolchain1 = create_test_toolchain("gcc-9", "toolchains/gcc");
        let toolchain2 = create_test_toolchain("gcc-10", "toolchains/gcc");
        let toolchain3 = create_test_toolchain("clang", "toolchains/clang");

        let toolchains = vec![&toolchain1, &toolchain2, &toolchain3];
        let result = manager.validate_toolchain_destinations(&toolchains);

        assert!(
            result.is_err(),
            "Validation should fail when duplicates exist"
        );

        let error_message = result.unwrap_err().to_string();
        assert!(error_message.contains("gcc-9"));
        assert!(error_message.contains("gcc-10"));
        assert!(error_message.contains("toolchains/gcc"));
        assert!(error_message.contains("Multiple toolchains target the same destination"));
    }

    #[test]
    fn test_validate_multiple_duplicate_groups() {
        let manager = create_test_manager();
        let toolchain1 = create_test_toolchain("gcc-9", "toolchains/gcc");
        let toolchain2 = create_test_toolchain("gcc-10", "toolchains/gcc");
        let toolchain3 = create_test_toolchain("llvm-12", "toolchains/llvm");
        let toolchain4 = create_test_toolchain("llvm-13", "toolchains/llvm");

        let toolchains = vec![&toolchain1, &toolchain2, &toolchain3, &toolchain4];
        let result = manager.validate_toolchain_destinations(&toolchains);

        assert!(
            result.is_err(),
            "Validation should fail when multiple duplicate groups exist"
        );

        let error_message = result.unwrap_err().to_string();
        assert!(error_message.contains("toolchains/gcc"));
        assert!(error_message.contains("toolchains/llvm"));
        assert!(error_message.contains("gcc-9"));
        assert!(error_message.contains("gcc-10"));
        assert!(error_message.contains("llvm-12"));
        assert!(error_message.contains("llvm-13"));
    }

    #[test]
    fn test_validate_empty_toolchain_list() {
        let manager = create_test_manager();
        let toolchains: Vec<&ToolchainConfig> = vec![];
        let result = manager.validate_toolchain_destinations(&toolchains);

        assert!(
            result.is_ok(),
            "Validation should pass for empty toolchain list"
        );
    }

    #[test]
    fn test_validate_single_toolchain() {
        let manager = create_test_manager();
        let toolchain = create_test_toolchain("gcc", "toolchains/gcc");
        let toolchains = vec![&toolchain];
        let result = manager.validate_toolchain_destinations(&toolchains);

        assert!(
            result.is_ok(),
            "Validation should pass for single toolchain"
        );
    }

    #[test]
    fn test_host_info_detection() {
        let host_info = detect_host_info();

        // The actual values will depend on the platform running the test
        assert!(!host_info.os.is_empty(), "OS should be detected");
        assert!(
            !host_info.arch.is_empty(),
            "Architecture should be detected"
        );

        // Check that the values are expected ones
        assert!(
            matches!(
                host_info.os.as_str(),
                "darwin" | "linux" | "windows" | "unknown"
            ),
            "OS should be one of the expected values"
        );
        assert!(
            matches!(
                host_info.arch.as_str(),
                "x86_64" | "arm64" | "i386" | "unknown"
            ),
            "Architecture should be one of the expected values"
        );
    }

    #[test]
    fn test_runtime_arch_detection() {
        let runtime_arch = detect_runtime_arch();

        // Architecture should be detected (either runtime or compile-time fallback)
        assert!(
            !runtime_arch.is_empty(),
            "Runtime architecture should be detected"
        );

        // Should be one of the standard architecture names
        assert!(
            matches!(
                runtime_arch.as_str(),
                "x86_64" | "arm64" | "i386" | "unknown"
            ),
            "Runtime architecture should be normalized to standard format, got: {}",
            runtime_arch
        );

        // Verify that runtime detection produces consistent results
        let second_detection = detect_runtime_arch();
        assert_eq!(
            runtime_arch, second_detection,
            "Runtime detection should be consistent across calls"
        );

        // Verify that normalize_arch works correctly
        assert_eq!(normalize_arch("x86_64"), "x86_64");
        assert_eq!(normalize_arch("amd64"), "x86_64");
        assert_eq!(normalize_arch("aarch64"), "arm64");
        assert_eq!(normalize_arch("arm64"), "arm64");
        assert_eq!(normalize_arch("i386"), "i386");
        assert_eq!(normalize_arch("i686"), "i386");
    }

    #[test]
    fn test_normalize_arch() {
        // Test x86_64 variants
        assert_eq!(normalize_arch("x86_64"), "x86_64");
        assert_eq!(normalize_arch("amd64"), "x86_64");
        assert_eq!(normalize_arch("x64"), "x86_64");
        assert_eq!(normalize_arch("AMD64"), "x86_64");

        // Test ARM64 variants
        assert_eq!(normalize_arch("aarch64"), "arm64");
        assert_eq!(normalize_arch("arm64"), "arm64");
        assert_eq!(normalize_arch("ARM64"), "arm64");
        assert_eq!(normalize_arch("arm64ec"), "arm64");

        // Test x86 variants
        assert_eq!(normalize_arch("i386"), "i386");
        assert_eq!(normalize_arch("i686"), "i386");
        assert_eq!(normalize_arch("x86"), "i386");

        // Test unknown architecture
        assert_eq!(normalize_arch("riscv64"), "riscv64");
        assert_eq!(normalize_arch("unknown"), "unknown");
    }

    #[test]
    fn test_is_toolchain_applicable_no_filters() {
        let manager = create_test_manager();
        let toolchain = create_test_toolchain("universal", "toolchains/universal");
        let host_info = HostInfo {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        };

        let result = manager.is_toolchain_applicable(&toolchain, &host_info);
        assert!(
            result,
            "Toolchain with no OS/arch filters should be applicable"
        );
    }

    #[test]
    fn test_is_toolchain_applicable_matching_filters() {
        let manager = create_test_manager();
        let mut toolchain = create_test_toolchain("linux-gcc", "toolchains/gcc");
        toolchain.os = Some("linux".to_string());
        toolchain.arch = Some("x86_64".to_string());

        let host_info = HostInfo {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        };

        let result = manager.is_toolchain_applicable(&toolchain, &host_info);
        assert!(
            result,
            "Toolchain with matching OS/arch filters should be applicable"
        );
    }

    #[test]
    fn test_is_toolchain_applicable_mismatched_os() {
        let manager = create_test_manager();
        let mut toolchain = create_test_toolchain("windows-gcc", "toolchains/gcc");
        toolchain.os = Some("windows".to_string());

        let host_info = HostInfo {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        };

        let result = manager.is_toolchain_applicable(&toolchain, &host_info);
        assert!(
            !result,
            "Toolchain with mismatched OS should not be applicable"
        );
    }

    #[test]
    fn test_is_toolchain_applicable_mismatched_arch() {
        let manager = create_test_manager();
        let mut toolchain = create_test_toolchain("arm-gcc", "toolchains/gcc");
        toolchain.arch = Some("arm64".to_string());

        let host_info = HostInfo {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        };

        let result = manager.is_toolchain_applicable(&toolchain, &host_info);
        assert!(
            !result,
            "Toolchain with mismatched architecture should not be applicable"
        );
    }

    #[test]
    fn test_construct_download_url_without_trailing_slash() {
        let manager = create_test_manager();
        let base_url = "https://example.com/files";
        let filename = "toolchain.tar.xz";

        let result = manager.construct_download_url(base_url, filename);
        assert_eq!(result, "https://example.com/files/toolchain.tar.xz");
    }

    #[test]
    fn test_construct_download_url_with_trailing_slash() {
        let manager = create_test_manager();
        let base_url = "https://example.com/files/";
        let filename = "toolchain.tar.xz";

        let result = manager.construct_download_url(base_url, filename);
        assert_eq!(result, "https://example.com/files/toolchain.tar.xz");
    }

    #[test]
    fn test_construct_download_url_empty_filename() {
        let manager = create_test_manager();
        let base_url = "https://example.com/files";
        let filename = "";

        // Empty filename means the URL is already complete
        let result = manager.construct_download_url(base_url, filename);
        assert_eq!(result, "https://example.com/files");
    }

    #[test]
    fn test_construct_download_url_filename_with_slash() {
        let manager = create_test_manager();
        let base_url = "https://example.com/files";
        let filename = "subdir/toolchain.tar.xz";

        let result = manager.construct_download_url(base_url, filename);
        assert_eq!(result, "https://example.com/files/subdir/toolchain.tar.xz");
    }

    #[test]
    fn test_construct_download_url_real_world_example() {
        let manager = create_test_manager();
        let base_url = "https://developer.arm.com/-/media/Files/downloads/gnu/14.3.rel1/binrel/";
        let filename = "arm-gnu-toolchain-14.3.rel1-darwin-arm64-arm-none-eabi.tar.xz";

        let result = manager.construct_download_url(base_url, filename);
        assert_eq!(
            result,
            "https://developer.arm.com/-/media/Files/downloads/gnu/14.3.rel1/binrel/arm-gnu-toolchain-14.3.rel1-darwin-arm64-arm-none-eabi.tar.xz"
        );
    }

    #[test]
    fn test_construct_download_url_real_world_without_slash() {
        let manager = create_test_manager();
        let base_url =
            "https://armkeil.blob.core.windows.net/developer/Files/downloads/gnu/13.3.rel1/binrel";
        let filename = "arm-gnu-toolchain-13.3.rel1-x86_64-arm-none-eabi.tar.xz";

        let result = manager.construct_download_url(base_url, filename);
        assert_eq!(
            result,
            "https://armkeil.blob.core.windows.net/developer/Files/downloads/gnu/13.3.rel1/binrel/arm-gnu-toolchain-13.3.rel1-x86_64-arm-none-eabi.tar.xz"
        );
    }

    #[test]
    fn test_construct_download_url_direct_file_url() {
        let manager = create_test_manager();
        // Case 1: URL ends with filename (rustup case)
        let base_url = "https://sh.rustup.rs";
        let filename = "sh.rustup.rs";
        let result = manager.construct_download_url(base_url, filename);
        assert_eq!(result, "https://sh.rustup.rs");

        // Case 2: URL with path ends with filename
        let base_url2 = "https://example.com/downloads/installer.sh";
        let filename2 = "installer.sh";
        let result2 = manager.construct_download_url(base_url2, filename2);
        assert_eq!(result2, "https://example.com/downloads/installer.sh");
    }

    #[test]
    fn test_construct_download_url_url_equals_filename() {
        let manager = create_test_manager();
        let url = "https://sh.rustup.rs";
        let filename = "https://sh.rustup.rs";
        let result = manager.construct_download_url(url, filename);
        assert_eq!(result, "https://sh.rustup.rs");
    }

    #[test]
    fn test_strip_archive_extension_tar_xz() {
        let name = "arm-gnu-toolchain-14.3.rel1-darwin-arm64-arm-none-eabi.tar.xz";
        let result = strip_archive_extension(name);
        assert_eq!(
            result,
            "arm-gnu-toolchain-14.3.rel1-darwin-arm64-arm-none-eabi"
        );
    }

    #[test]
    fn test_strip_archive_extension_tar_gz() {
        let name = "rust-1.75.0-x86_64-unknown-linux-gnu.tar.gz";
        let result = strip_archive_extension(name);
        assert_eq!(result, "rust-1.75.0-x86_64-unknown-linux-gnu");
    }

    #[test]
    fn test_strip_archive_extension_tar_bz2() {
        let name = "toolchain-archive.tar.bz2";
        let result = strip_archive_extension(name);
        assert_eq!(result, "toolchain-archive");
    }

    #[test]
    fn test_strip_archive_extension_tar() {
        let name = "simple-toolchain.tar";
        let result = strip_archive_extension(name);
        assert_eq!(result, "simple-toolchain");
    }

    #[test]
    fn test_strip_archive_extension_zip() {
        let name = "windows-toolchain.zip";
        let result = strip_archive_extension(name);
        assert_eq!(result, "windows-toolchain");
    }

    #[test]
    fn test_strip_archive_extension_shell_script() {
        let name = "rustup-init.sh";
        let result = strip_archive_extension(name);
        assert_eq!(result, "rustup-init");
    }

    #[test]
    fn test_strip_archive_extension_with_path() {
        let name = "subdir/path/to/toolchain.tar.xz";
        let result = strip_archive_extension(name);
        assert_eq!(result, "toolchain");
    }

    #[test]
    fn test_strip_archive_extension_no_extension() {
        let name = "toolchain-no-ext";
        let result = strip_archive_extension(name);
        assert_eq!(result, "toolchain-no-ext");
    }

    #[test]
    fn test_strip_archive_extension_unknown_extension() {
        let name = "toolchain.exe";
        let result = strip_archive_extension(name);
        assert_eq!(result, "toolchain.exe");
    }

    #[test]
    fn test_strip_archive_extension_multiple_dots() {
        let name = "gcc-12.2.0-final.tar.gz";
        let result = strip_archive_extension(name);
        assert_eq!(result, "gcc-12.2.0-final");
    }

    #[test]
    fn test_strip_archive_extension_url_path() {
        let name = "https://example.com/downloads/toolchain.tar.xz";
        let result = strip_archive_extension(name);
        assert_eq!(result, "toolchain");
    }

    #[test]
    fn test_expand_toolchain_env_vars_pwd() {
        let manager = create_test_manager();
        let dest_path = PathBuf::from("/workspace/toolchains/rust");

        // Test $PWD expansion
        let result = manager.expand_toolchain_env_vars("$PWD/cargo", &dest_path);
        assert_eq!(result, "/workspace/toolchains/rust/cargo");

        // Test ${PWD} expansion
        let result = manager.expand_toolchain_env_vars("${PWD}/rustup", &dest_path);
        assert_eq!(result, "/workspace/toolchains/rust/rustup");

        // Test $PWD in middle of path
        let result = manager.expand_toolchain_env_vars("$PWD/bin:$PWD/lib", &dest_path);
        assert_eq!(
            result,
            "/workspace/toolchains/rust/bin:/workspace/toolchains/rust/lib"
        );
    }

    #[test]
    fn test_expand_toolchain_env_vars_workspace() {
        let manager = create_test_manager();
        let dest_path = PathBuf::from("/workspace/toolchains/rust");

        // Test $WORKSPACE expansion
        let result = manager.expand_toolchain_env_vars("$WORKSPACE/build", &dest_path);
        assert_eq!(result, "/workspace/build");

        // Test ${WORKSPACE} expansion
        let result = manager.expand_toolchain_env_vars("${WORKSPACE}/output", &dest_path);
        assert_eq!(result, "/workspace/output");

        // Test mixed $PWD and $WORKSPACE
        let result = manager.expand_toolchain_env_vars("$WORKSPACE/toolchains/$PWD", &dest_path);
        // Note: This expands literally, so we get the second $PWD expanded too
        assert!(result.contains("/workspace/toolchains/"));
    }

    #[test]
    fn test_expand_toolchain_env_vars_home() {
        let manager = create_test_manager();
        let dest_path = PathBuf::from("/workspace/toolchains/rust");

        // Set test HOME variable
        std::env::set_var("TEST_HOME_VAR", "/home/testuser");

        // Test standard env var expansion
        let result = manager.expand_toolchain_env_vars("$TEST_HOME_VAR/.config", &dest_path);
        assert_eq!(result, "/home/testuser/.config");

        // Test ${VAR} syntax
        let result = manager.expand_toolchain_env_vars("${TEST_HOME_VAR}/data", &dest_path);
        assert_eq!(result, "/home/testuser/data");

        // Cleanup
        std::env::remove_var("TEST_HOME_VAR");
    }

    #[test]
    fn test_expand_toolchain_env_vars_path_like() {
        let manager = create_test_manager();
        let dest_path = PathBuf::from("/workspace/toolchains/rust");

        // Set up PATH-like test variable
        std::env::set_var("TEST_PATH", "/usr/bin:/bin");

        // Test PATH prepending (common use case)
        let result = manager.expand_toolchain_env_vars("$PWD/bin:$TEST_PATH", &dest_path);
        assert_eq!(result, "/workspace/toolchains/rust/bin:/usr/bin:/bin");

        // Cleanup
        std::env::remove_var("TEST_PATH");
    }

    #[test]
    fn test_expand_toolchain_env_vars_mixed_syntax() {
        let manager = create_test_manager();
        let dest_path = PathBuf::from("/workspace/toolchains/node");

        std::env::set_var("TEST_VAR", "test_value");

        // Test mixed variable types and syntax
        let result =
            manager.expand_toolchain_env_vars("$PWD/bin:${WORKSPACE}/shared:$TEST_VAR", &dest_path);
        assert_eq!(
            result,
            "/workspace/toolchains/node/bin:/workspace/shared:test_value"
        );

        // Cleanup
        std::env::remove_var("TEST_VAR");
    }

    #[test]
    fn test_expand_toolchain_env_vars_undefined() {
        let manager = create_test_manager();
        let dest_path = PathBuf::from("/workspace/toolchains/rust");

        // Undefined variables should be left unchanged (this is the current behavior)
        // Note: The implementation tries to expand and leaves undefined vars as-is
        let result = manager.expand_toolchain_env_vars("$UNDEFINED_VAR/path", &dest_path);
        // The $UNDEFINED_VAR pattern should remain if not found
        assert!(result.contains("UNDEFINED_VAR") || result == "/path");
    }

    #[test]
    fn test_expand_toolchain_env_vars_no_expansion() {
        let manager = create_test_manager();
        let dest_path = PathBuf::from("/workspace/toolchains/rust");

        // Test strings without variables
        let result = manager.expand_toolchain_env_vars("/absolute/path", &dest_path);
        assert_eq!(result, "/absolute/path");

        let result = manager.expand_toolchain_env_vars("relative/path", &dest_path);
        assert_eq!(result, "relative/path");

        let result = manager.expand_toolchain_env_vars("simple_value", &dest_path);
        assert_eq!(result, "simple_value");
    }

    #[test]
    fn test_expand_toolchain_env_vars_complex_path() {
        let manager = ToolchainManager::new(
            PathBuf::from("/home/user/my-workspace"),
            PathBuf::from("/home/user/mirror"),
        );
        let dest_path = PathBuf::from("/home/user/mirror/toolchains/rust-1.75.0");

        // Test complex real-world example
        let cargo_home = manager.expand_toolchain_env_vars("$PWD/cargo", &dest_path);
        assert_eq!(cargo_home, "/home/user/mirror/toolchains/rust-1.75.0/cargo");

        let rustup_home = manager.expand_toolchain_env_vars("${PWD}/rustup", &dest_path);
        assert_eq!(
            rustup_home,
            "/home/user/mirror/toolchains/rust-1.75.0/rustup"
        );
    }
}
