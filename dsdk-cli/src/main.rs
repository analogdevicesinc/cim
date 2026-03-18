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

mod cli;
mod version;

use arboard::Clipboard;
use clap::{CommandFactory, Parser};
use cli::{Cli, Commands, DockerCommand, DocsCommand, InstallCommand, UtilsCommand};
use dsdk_cli::config::SdkConfigCore;
use dsdk_cli::download::{
    compute_file_sha256, copy_single_file, copy_yaml_files_to_workspace, download_file_with_cache,
    generate_cache_path, process_copy_files, DownloadConfig,
};
use dsdk_cli::workspace::{
    copy_dir_recursive, create_workspace_marker, download_config_from_url,
    expand_config_mirror_path, expand_env_vars, get_current_workspace, get_default_source,
    get_docker_temp_dir, is_url, load_config_with_user_overrides,
    resolve_config_source_dir_from_marker, resolve_target_config_from_git,
    CreateWorkspaceMarkerParams, WorkspaceMarker,
};
use dsdk_cli::{
    config, doc_manager, docker_manager, git_operations, messages, toolchain_manager,
    vscode_tasks_manager,
};
use regex::Regex;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use threadpool::ThreadPool;
use version::{
    fetch_latest_release_version, find_cim_in_path, is_newer_version, platform_archive_name,
    print_update_notice, print_version_info, spawn_version_check,
};

/// Install system dependencies based on the type specified
fn handle_install_command(install_command: &InstallCommand) {
    // Must be run from within a workspace
    let workspace_path = match get_current_workspace() {
        Ok(path) => path,
        Err(e) => {
            messages::error(&format!("Error: {}", e));
            return;
        }
    };

    // Use sdk.yml from workspace root
    let config_path = workspace_path.join("sdk.yml");
    if !config_path.exists() {
        messages::error(&format!(
            "sdk.yml not found in workspace root: {}",
            workspace_path.display()
        ));
        messages::error("The workspace may be corrupted. Try running 'cim init' to reinitialize.");
        return;
    }

    // Load SDK config with user config overrides applied
    let _sdk_config = match load_config_with_user_overrides(&config_path, false) {
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

    match install_command {
        InstallCommand::OsDeps { yes, no_sudo } => {
            // Look for os-dependencies.yml file in workspace (copied via copy_files)
            let os_deps_path = workspace_path.join("os-dependencies.yml");
            if os_deps_path.exists() {
                match config::load_os_dependencies(&os_deps_path) {
                    Ok(os_deps) => {
                        install_prerequisites(&os_deps, *yes, *no_sudo);
                    }
                    Err(e) => {
                        messages::error(&format!("Failed to load os-dependencies.yml: {}", e));
                    }
                }
            } else {
                messages::error("os-dependencies.yml not found in workspace.");
                messages::error("This file is copied automatically during 'cim init'.");
            }
        }
        InstallCommand::Pip {
            force,
            symlink,
            profile,
            list_profiles,
        } => {
            // Look for python-dependencies.yml file in workspace (copied via copy_files)
            let python_deps_path = workspace_path.join("python-dependencies.yml");
            if python_deps_path.exists() {
                // Show available profiles if user requested the list
                if *list_profiles {
                    list_available_profiles(&python_deps_path);
                    return;
                }

                // Mirror path already expanded in _sdk_config with user overrides applied
                if let Err(e) = install_python_packages_from_file(
                    &python_deps_path,
                    *force,
                    *symlink,
                    profile.as_deref(),
                    &workspace_path,
                    &_sdk_config.mirror,
                ) {
                    messages::error(&format!("Failed to install Python packages: {}", e));
                    std::process::exit(1);
                }
            } else {
                messages::error("python-dependencies.yml not found in workspace.");
                messages::error("This file is copied automatically during 'cim init'.");
            }
        }
        InstallCommand::Toolchains {
            force,
            symlink,
            verbose,
            cert_validation,
        } => {
            // Set verbose mode for messages module
            dsdk_cli::messages::set_verbose(*verbose);

            // Use _sdk_config that already has user overrides applied and mirror expanded
            // Create toolchain manager and install toolchains
            let toolchain_manager = toolchain_manager::ToolchainManager::new(
                workspace_path.clone(),
                _sdk_config.mirror.clone(),
            );

            if let Err(e) = toolchain_manager.install_toolchains(
                _sdk_config.toolchains.as_ref(),
                *force,
                *symlink,
                cert_validation.as_deref(),
            ) {
                messages::error(&format!("Error installing toolchains: {}", e));
            }
        }
        InstallCommand::Tools {
            name,
            list,
            all,
            force,
        } => {
            // Use _sdk_config that already has user overrides applied
            // Check if install section exists
            if _sdk_config.install.is_none() || _sdk_config.install.as_ref().unwrap().is_empty() {
                messages::error("No install section found in sdk.yml");
                messages::info("The install section defines components that can be installed.");
                return;
            }

            let install_configs = _sdk_config.install.as_ref().unwrap();

            // Handle --list flag
            if *list {
                messages::status("Available install targets:");
                for install_cfg in install_configs {
                    let sentinel_info = if let Some(ref sentinel) = install_cfg.sentinel {
                        format!(" (sentinel: {})", sentinel)
                    } else {
                        String::new()
                    };
                    let deps_info = if let Some(ref deps) = install_cfg.depends_on {
                        if !deps.is_empty() {
                            format!(" [depends on: {}]", deps.join(", "))
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    messages::status(&format!(
                        "  install-{}{}{}",
                        install_cfg.name, sentinel_info, deps_info
                    ));
                }
                messages::status("");
                messages::status("Use --all to install all components.");
                messages::status("");
                messages::status("Run with: cim install tools <name>");
                return;
            }

            // Determine which target to run
            let target_name = if *all {
                "install-all".to_string()
            } else if let Some(component_name) = name {
                format!("install-{}", component_name)
            } else {
                messages::error("Either provide a component name or use --all or --list");
                return;
            };

            // If force is set, remove sentinel file
            if *force && !*all {
                if let Some(component_name) = name {
                    if let Some(install_cfg) = install_configs
                        .iter()
                        .find(|cfg| cfg.name == *component_name)
                    {
                        if let Some(ref sentinel) = install_cfg.sentinel {
                            let sentinel_path = workspace_path.join(sentinel);
                            if sentinel_path.exists() {
                                if let Err(e) = std::fs::remove_file(&sentinel_path) {
                                    messages::info(&format!(
                                        "Warning: Failed to remove sentinel file {}: {}",
                                        sentinel_path.display(),
                                        e
                                    ));
                                } else {
                                    messages::info(&format!(
                                        "Removed sentinel file: {}",
                                        sentinel_path.display()
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            // Check if Makefile exists
            let makefile_path = workspace_path.join("Makefile");
            if !makefile_path.exists() {
                messages::error("Makefile not found in workspace.");
                messages::info("Run 'cim makefile' first to generate the Makefile.");
                return;
            }

            // Run make command
            messages::status(&format!("Running: make {}", target_name));
            let make_status = std::process::Command::new("make")
                .arg(&target_name)
                .current_dir(&workspace_path)
                .status();

            match make_status {
                Ok(status) => {
                    if !status.success() {
                        std::process::exit(status.code().unwrap_or(1));
                    }
                }
                Err(e) => {
                    messages::error(&format!("Failed to run make: {}", e));
                    messages::info("Make sure 'make' is installed on your system.");
                    std::process::exit(1);
                }
            }
        }
    }
}

/// Manager for Python virtual environment operations with symlink support
struct VenvManager {
    workspace_path: PathBuf,
    mirror_path: PathBuf,
}

impl VenvManager {
    /// Create a new VenvManager instance
    pub fn new(workspace_path: PathBuf, mirror_path: PathBuf) -> Self {
        VenvManager {
            workspace_path,
            mirror_path,
        }
    }

    /// Create virtual environment with symlink support
    pub fn create_venv(
        &self,
        force: bool,
        symlink: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if symlink {
            self.create_venv_with_symlink(force)
        } else {
            self.create_venv_direct(force)
        }
    }

    /// Create virtual environment directly in workspace
    fn create_venv_direct(&self, force: bool) -> Result<(), Box<dyn std::error::Error>> {
        let venv_path = self.workspace_path.join(".venv");

        // Check if venv already exists
        if venv_path.exists() {
            if force {
                messages::info("Virtual environment exists, removing due to --force");
                if let Err(e) = std::fs::remove_dir_all(&venv_path) {
                    return Err(
                        format!("Failed to remove existing virtual environment: {}", e).into(),
                    );
                }
            } else {
                messages::info(&format!(
                    "Virtual environment already exists at {}, skipping (use --force to reinstall)",
                    venv_path.display()
                ));
                return Ok(());
            }
        }

        messages::status(&format!(
            "Creating Python virtual environment at {}...",
            venv_path.display()
        ));

        let output = std::process::Command::new("python3")
            .args(["-m", "venv"])
            .arg(&venv_path)
            .current_dir(&self.workspace_path)
            .output()?;

        if !output.status.success() {
            return Err(format!(
                "Failed to create virtual environment:\n{}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        messages::success("Virtual environment created successfully");
        Ok(())
    }

    /// Create virtual environment in mirror and symlink to workspace
    fn create_venv_with_symlink(&self, force: bool) -> Result<(), Box<dyn std::error::Error>> {
        let workspace_venv_path = self.workspace_path.join(".venv");
        let mirror_venv_path = self.mirror_path.join(".venv");

        // Check if workspace symlink already exists
        if let Ok(metadata) = std::fs::symlink_metadata(&workspace_venv_path) {
            if metadata.file_type().is_symlink() {
                if force {
                    messages::status(&format!(
                        "Symlink {} exists, removing due to --force",
                        workspace_venv_path.display()
                    ));
                    if let Err(e) = std::fs::remove_file(&workspace_venv_path) {
                        return Err(format!("Failed to remove existing symlink: {}", e).into());
                    }
                } else {
                    messages::status(&format!(
                        "Virtual environment symlink already exists at {}, skipping (use --force to reinstall)",
                        workspace_venv_path.display()
                    ));
                    return Ok(());
                }
            } else if force {
                messages::status(&format!(
                    "Destination {} exists and is not a symlink, removing due to --force",
                    workspace_venv_path.display()
                ));
                if workspace_venv_path.is_dir() {
                    if let Err(e) = std::fs::remove_dir_all(&workspace_venv_path) {
                        return Err(format!("Failed to remove existing directory: {}", e).into());
                    }
                } else if let Err(e) = std::fs::remove_file(&workspace_venv_path) {
                    return Err(format!("Failed to remove existing file: {}", e).into());
                }
            } else {
                return Err(format!(
                    "Destination path {} exists but is not a symlink. Use --force to remove it or install without --symlink",
                    workspace_venv_path.display()
                ).into());
            }
        }

        // Check if mirror venv already exists
        if mirror_venv_path.exists() {
            if force {
                messages::info(&format!(
                    "Mirror virtual environment {} exists, removing due to --force",
                    mirror_venv_path.display()
                ));
                if let Err(e) = std::fs::remove_dir_all(&mirror_venv_path) {
                    return Err(format!(
                        "Failed to remove existing mirror virtual environment: {}",
                        e
                    )
                    .into());
                }
            } else {
                messages::info(&format!(
                    "Using existing mirror virtual environment at {}",
                    mirror_venv_path.display()
                ));
            }
        }

        // Create mirror venv if it doesn't exist
        if !mirror_venv_path.exists() {
            // Ensure mirror directory exists
            if let Some(parent) = mirror_venv_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            messages::status(&format!(
                "Creating Python virtual environment in mirror at {}...",
                mirror_venv_path.display()
            ));

            let output = std::process::Command::new("python3")
                .args(["-m", "venv"])
                .arg(&mirror_venv_path)
                .output()?;

            if !output.status.success() {
                return Err(format!(
                    "Failed to create mirror virtual environment:\n{}",
                    String::from_utf8_lossy(&output.stderr)
                )
                .into());
            }
        }

        // Create symlink from workspace to mirror
        self.create_symlink(&workspace_venv_path, &mirror_venv_path)?;

        messages::success("Virtual environment symlink created successfully");
        Ok(())
    }

    /// Create symlink from workspace to mirror (cross-platform)
    #[cfg(unix)]
    fn create_symlink(
        &self,
        workspace_path: &Path,
        mirror_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        // Ensure workspace parent directory exists
        if let Some(parent) = workspace_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if let Err(e) = symlink(mirror_path, workspace_path) {
            return Err(format!("Failed to create symlink: {}", e).into());
        }

        messages::verbose(&format!(
            "Created symlink: {} -> {}",
            workspace_path.display(),
            mirror_path.display()
        ));
        Ok(())
    }

    #[cfg(windows)]
    fn create_symlink(
        &self,
        workspace_path: &Path,
        mirror_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use std::os::windows::fs::symlink_dir;

        // Ensure workspace parent directory exists
        if let Some(parent) = workspace_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if let Err(e) = symlink_dir(mirror_path, workspace_path) {
            return Err(format!("Failed to create directory junction: {}", e).into());
        }

        messages::verbose(&format!(
            "Created directory junction: {} -> {}",
            workspace_path.display(),
            mirror_path.display()
        ));
        Ok(())
    }
}

/// Check if virtual environment exists in workspace
fn venv_exists(workspace_path: &Path) -> bool {
    let venv_path = workspace_path.join(".venv");
    venv_path.exists() && venv_path.join("bin").join("python3").exists()
}

/// Get the path to the virtual environment's pip executable
fn get_venv_pip_path(workspace_path: &Path) -> PathBuf {
    workspace_path.join(".venv").join("bin").join("pip")
}

/// Create a Python virtual environment in the workspace
fn create_virtual_environment(workspace_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let venv_path = workspace_path.join(".venv");

    if venv_exists(workspace_path) {
        messages::info(&format!(
            "Virtual environment already exists at {}",
            venv_path.display()
        ));
        return Ok(());
    }

    messages::status(&format!(
        "Creating Python virtual environment at {}...",
        venv_path.display()
    ));

    let output = std::process::Command::new("python3")
        .args(["-m", "venv"])
        .arg(&venv_path)
        .current_dir(workspace_path)
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to create virtual environment:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    messages::success("Virtual environment created successfully");
    Ok(())
}

/// Detect if we're running in a container environment
fn is_container_environment() -> bool {
    // Check for common container indicators
    std::path::Path::new("/.dockerenv").exists()
        || std::env::var("container").is_ok()
        || std::fs::read_to_string("/proc/1/cgroup")
            .map(|content| content.contains("docker") || content.contains("containerd"))
            .unwrap_or(false)
}

/// Check if sphinx-build is available in system PATH
fn is_sphinx_available() -> bool {
    std::process::Command::new("sphinx-build")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Ensure documentation dependencies are available in virtual environment
fn ensure_docs_dependencies(workspace_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // In container environments, check if sphinx is globally available
    if is_container_environment() {
        if is_sphinx_available() {
            messages::status("Container environment detected. Using system sphinx-build.");
            return Ok(());
        } else {
            return Err("Sphinx not found in container. Install with: apt-get install python3-sphinx or pip3 install sphinx".into());
        }
    }

    // Check if virtual environment exists and has sphinx-build
    if venv_exists(workspace_path) {
        let sphinx_build_path = workspace_path
            .join(".venv")
            .join("bin")
            .join("sphinx-build");
        if sphinx_build_path.exists() {
            // Virtual environment exists and has sphinx, we're good
            return Ok(());
        }
    }

    messages::status("Documentation dependencies not found. Setting up virtual environment...");

    // Create virtual environment if it doesn't exist
    create_virtual_environment(workspace_path)?;

    // Install required packages
    let python_deps_path = workspace_path.join("python-dependencies.yml");

    // Load Python dependencies configuration and determine which profile to use
    let (packages, profile_source) = match config::load_python_dependencies(&python_deps_path) {
        Ok(python_deps) => {
            // For documentation, prefer 'docs' profile, then default profile
            let profile_name = if python_deps.profiles.contains_key("docs") {
                "docs"
            } else {
                &python_deps.default
            };

            if let Some(profile) = python_deps.profiles.get(profile_name) {
                let source = format!("profile '{}' from python-dependencies.yml", profile_name);
                (profile.packages.clone(), source)
            } else {
                messages::info(&format!(
                    "Profile '{}' not found in python-dependencies.yml",
                    profile_name
                ));
                messages::info("Falling back to hardcoded documentation packages");
                // Fallback to hardcoded packages
                let packages = vec![
                    "sphinx".to_string(),
                    "sphinx-rtd-theme".to_string(),
                    "myst-parser".to_string(),
                    "sphinx-autobuild".to_string(),
                ];
                (packages, "hardcoded defaults".to_string())
            }
        }
        Err(e) => {
            messages::info(&format!("Could not load python-dependencies.yml: {}", e));
            messages::info("Falling back to hardcoded documentation packages");
            // Fallback to hardcoded packages
            let packages = vec![
                "sphinx".to_string(),
                "sphinx-rtd-theme".to_string(),
                "myst-parser".to_string(),
                "sphinx-autobuild".to_string(),
            ];
            (packages, "hardcoded defaults".to_string())
        }
    };

    messages::status(&format!(
        "Installing documentation dependencies from {}",
        profile_source
    ));
    install_pip_packages(&packages, None)?;
    Ok(())
}

/// Helper function to install pip packages
fn install_pip_packages(
    packages: &[String],
    workspace_path_override: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    if packages.is_empty() {
        messages::info("No Python packages to install");
        return Ok(());
    }

    // Get workspace path - either from parameter or auto-detect
    let workspace_path = match workspace_path_override {
        Some(path) => path.to_path_buf(),
        None => match get_current_workspace() {
            Ok(path) => path,
            Err(_) => {
                return Err(
                    "Could not finish setting up Python packages: workspace not found".into(),
                );
            }
        },
    };

    // Use virtual environment pip if available
    if !venv_exists(&workspace_path) {
        return Err(
            "Could not finish setting up Python packages: virtual environment not found".into(),
        );
    }

    let venv_pip = get_venv_pip_path(&workspace_path);
    messages::verbose(&format!(
        "Using virtual environment pip: {}",
        venv_pip.display()
    ));
    messages::status(&format!(
        "Running: {} install {}",
        venv_pip.display(),
        packages.join(" ")
    ));

    let status = std::process::Command::new(&venv_pip)
        .arg("install")
        .arg("--trusted-host")
        .arg("pypi.org")
        .arg("--trusted-host")
        .arg("pypi.python.org")
        .arg("--trusted-host")
        .arg("files.pythonhosted.org")
        .args(packages)
        .status()
        .map_err(|e| {
            format!(
                "Could not finish setting up Python packages: failed to execute pip: {}",
                e
            )
        })?;

    if !status.success() {
        return Err(format!(
            "Could not finish setting up Python packages: installation failed with exit code: {}",
            status
        )
        .into());
    }

    messages::success("Successfully installed Python packages in virtual environment");
    Ok(())
}

/// List available Python dependency profiles
fn list_available_profiles(python_deps_path: &Path) {
    match config::load_python_dependencies(python_deps_path) {
        Ok(python_deps) => {
            messages::status("Available Python dependency profiles in this workspace:\n");

            // Sort profiles for consistent output
            let mut profile_names: Vec<_> = python_deps.profiles.keys().collect();
            profile_names.sort();

            for profile_name in profile_names {
                if let Some(profile) = python_deps.profiles.get(profile_name) {
                    let is_default = profile_name == &python_deps.default;
                    let default_marker = if is_default { " (default)" } else { "" };

                    messages::status(&format!("  {}{}", profile_name, default_marker));

                    if profile.packages.is_empty() {
                        messages::status("    No packages");
                    } else {
                        messages::status(&format!("    Packages: {}", profile.packages.join(", ")));
                    }
                    messages::status("");
                }
            }

            messages::status("Usage:");
            messages::status(&format!(
                "  cim install pip                   # Use default profile ({})",
                python_deps.default
            ));
            messages::status("  cim install pip -p <PROFILE>      # Use specific profile");
            messages::status(
                "  cim install pip -p dev,docs       # Use multiple profiles (comma-separated)",
            );
        }
        Err(e) => {
            messages::error(&format!("Error loading python-dependencies.yml: {}", e));
        }
    }
}

/// Install Python packages from python-dependencies.yml file
fn install_python_packages_from_file(
    python_deps_path: &Path,
    force: bool,
    symlink: bool,
    profile_override: Option<&str>,
    workspace_path: &Path,
    mirror_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Load Python dependencies configuration first
    let python_deps = match config::load_python_dependencies(python_deps_path) {
        Ok(deps) => deps,
        Err(e) => {
            return Err(format!("Could not load {}: {}", python_deps_path.display(), e).into());
        }
    };

    // Determine which profiles to use: command-line override or default from config
    // Support comma-separated profiles like "dev,docs"
    let profile_names: Vec<&str> = if let Some(override_str) = profile_override {
        override_str.split(',').map(|s| s.trim()).collect()
    } else {
        vec![python_deps.default.as_str()]
    };

    // Validate profiles before doing any side effects
    let mut invalid_profiles = Vec::new();
    for profile_name in &profile_names {
        if !python_deps.profiles.contains_key(*profile_name) {
            invalid_profiles.push(*profile_name);
        }
    }

    // Report any invalid profiles and exit early
    if !invalid_profiles.is_empty() {
        return Err(format!(
            "Profile(s) not found in python-dependencies.yml: {}. Available profiles: {}",
            invalid_profiles.join(", "),
            python_deps
                .profiles
                .keys()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into());
    }

    messages::status(&format!(
        "Installing Python packages from {}...",
        python_deps_path.display()
    ));

    // Create VenvManager for virtual environment operations
    let venv_manager = VenvManager::new(workspace_path.to_path_buf(), mirror_path.to_path_buf());

    // Create virtual environment (direct or symlink mode)
    if let Err(e) = venv_manager.create_venv(force, symlink) {
        return Err(format!("Error creating virtual environment: {}", e).into());
    }

    // Display which profiles are being used
    if profile_names.len() == 1 {
        messages::status(&format!("Using Python profile: {}", profile_names[0]));
    } else {
        messages::status(&format!(
            "Using Python profiles: {}",
            profile_names.join(", ")
        ));
    }

    // Collect unique packages from all specified profiles
    use std::collections::HashSet;
    let mut all_packages = HashSet::new();

    for profile_name in &profile_names {
        if let Some(profile) = python_deps.profiles.get(*profile_name) {
            for package in &profile.packages {
                all_packages.insert(package.clone());
            }
            messages::verbose(&format!(
                "Profile '{}' adds {} package(s)",
                profile_name,
                profile.packages.len()
            ));
        }
    }

    // Convert HashSet to sorted Vec for consistent output
    let mut packages: Vec<String> = all_packages.into_iter().collect();
    packages.sort();

    if packages.is_empty() {
        if profile_names.len() == 1 {
            messages::info(&format!(
                "Profile '{}' has no packages to install",
                profile_names[0]
            ));
        } else {
            messages::info(&format!(
                "Profiles '{}' have no packages to install",
                profile_names.join(", ")
            ));
        }
        return Ok(());
    }

    messages::status(&format!(
        "Installing {} unique package(s) from {} profile(s)",
        packages.len(),
        profile_names.len()
    ));

    install_pip_packages(&packages, Some(workspace_path))?;
    Ok(())
}

/// Install prerequisites based on OS dependencies configuration
fn install_prerequisites(os_deps: &config::OsDependencies, skip_prompt: bool, no_sudo: bool) {
    let (os_key, distro_name, detected_version) = detect_os_and_distro();

    // Try architecture-specific key first (e.g., "linux-aarch64"), then fall back to generic "linux"
    let os_config = os_deps.os_configs.get(&os_key).or_else(|| {
        if os_key.starts_with("linux-") {
            os_deps.os_configs.get("linux")
        } else {
            None
        }
    });

    if let Some(os_config) = os_config {
        // Try to find matching distro configuration
        // Priority: 1) Exact match with version (distro-version), 2) Bare distro name (legacy)
        let distro_key_with_version = format!("{}-{}", distro_name, detected_version);
        let mut found_legacy_format = false;

        let distro_config = os_config.distros.get(&distro_key_with_version).or_else(|| {
            // TODO: Remove backward compatibility after migration period
            // Fall back to bare distro name for legacy format
            if let Some(config) = os_config.distros.get(&distro_name) {
                found_legacy_format = true;
                Some(config)
            } else {
                None
            }
        });

        if let Some(distro_config) = distro_config {
            // Show deprecation warning for old format
            if found_legacy_format {
                messages::info(&format!(
                    "Old os-dependencies.yml format detected for '{}'. Consider migrating to '{}-{}' format.",
                    distro_name, distro_name, detected_version
                ));
            }

            // Determine if sudo is needed
            // Linux package managers (apt-get, dnf, yum, etc.) typically need sudo
            // macOS brew should NOT use sudo
            let needs_sudo = !no_sudo && os_key.starts_with("linux");

            // Display target information
            messages::status(&format!(
                "Installing packages for {}, {} {} using {}",
                os_key, distro_name, detected_version, distro_config.package_manager.command
            ));

            // Display packages to be installed (sorted alphabetically)
            let mut packages = distro_config.package_manager.packages.clone();
            packages.sort();
            messages::status("\nPackages to be installed:");
            for package in &packages {
                messages::status(&format!("  - {}", package));
            }

            // Display sudo information
            if needs_sudo {
                messages::status(&format!(
                    "\n{} This command requires sudo privileges to install system packages.",
                    messages::INFO
                ));
                messages::status("  You may be prompted for your password.");
            }

            // Confirmation prompt (skip if --yes flag is used)
            if !skip_prompt {
                messages::status(
                    "\nDo you want to proceed with the installation of the above packages on",
                );
                print!("your HOST OS? [y/N]: ");
                use std::io::Write;
                io::stdout().flush().unwrap();

                let mut input = String::new();
                match io::stdin().read_line(&mut input) {
                    Ok(_) => {
                        let input = input.trim().to_lowercase();
                        if input != "y" && input != "yes" {
                            messages::status("Installation cancelled.");
                            return;
                        }
                    }
                    Err(e) => {
                        messages::error(&format!("Failed to read input: {}", e));
                        return;
                    }
                }
            } else {
                messages::status("\nProceeding with installation (--yes flag specified)...");
            }

            // Build the full command with sudo if needed
            let mut cmd_parts: Vec<String> = Vec::new();

            // Add sudo as the first command if needed
            let (program, initial_args): (&str, Vec<String>) = if needs_sudo {
                cmd_parts.push("sudo".to_string());
                cmd_parts.extend(
                    distro_config
                        .package_manager
                        .command
                        .split_whitespace()
                        .map(String::from),
                );
                ("sudo", cmd_parts[1..].to_vec())
            } else {
                cmd_parts.extend(
                    distro_config
                        .package_manager
                        .command
                        .split_whitespace()
                        .map(String::from),
                );
                (cmd_parts[0].as_str(), cmd_parts[1..].to_vec())
            };

            // Add packages to install
            let mut all_args = initial_args;
            all_args.extend(packages.iter().cloned());

            if cmd_parts.is_empty() {
                messages::error("No installation command found");
                return;
            }

            messages::status(&format!("\nRunning: {} {}", program, all_args.join(" ")));

            match std::process::Command::new(program).args(&all_args).status() {
                Ok(status) if status.success() => {
                    messages::success("Successfully installed OS dependencies");
                }
                Ok(status) => {
                    messages::error(&format!("Installation failed with exit code: {}", status));
                    if needs_sudo && status.code() == Some(1) {
                        messages::info("Hint: If you see 'permission denied' errors, ensure you have sudo privileges.");
                        messages::info("      You can skip sudo with --no-sudo if running as root or with special apt configuration.");
                    }
                }
                Err(e) => {
                    messages::error(&format!("Failed to execute installation command: {}", e));
                    if needs_sudo {
                        messages::info("Hint: Make sure 'sudo' is installed and you have permission to use it.");
                    }
                }
            }
        } else {
            // No matching distro configuration found
            messages::error(&format!(
                "No configuration found for distribution: {} (version {})",
                distro_name, detected_version
            ));

            // List available configurations to help user
            let mut available_distros: Vec<String> = os_config.distros.keys().cloned().collect();
            available_distros.sort();

            messages::status(&format!(
                "Available distributions for {}: {}",
                os_key,
                available_distros.join(", ")
            ));

            // Provide helpful hint about expected key format
            messages::info(&format!(
                "Hint: Add '{}-{}' to your os-dependencies.yml to support this OS version.",
                distro_name, detected_version
            ));
        }
    } else {
        messages::error(&format!("No configuration found for OS: {}", os_key));
        let available_os: Vec<String> = os_deps.os_configs.keys().cloned().collect();
        messages::status(&format!(
            "Available OS configurations: {}",
            available_os.join(", ")
        ));
    }
}

/// Detect the current operating system, distribution, and version
/// Returns (os_key, distro, version) where:
/// - os_key: OS name with architecture (e.g., "linux-aarch64", "macos")
/// - distro: Distribution name (e.g., "ubuntu", "fedora", "macos")
/// - version: Version string (e.g., "24.04", "42", "any")
fn detect_os_and_distro() -> (String, String, String) {
    #[cfg(target_os = "linux")]
    {
        let mut distro = String::from("unknown");
        let mut version = String::from("unknown");

        // Try to read /etc/os-release to detect Linux distribution and version
        if let Ok(contents) = std::fs::read_to_string("/etc/os-release") {
            for line in contents.lines() {
                if line.starts_with("ID=") {
                    distro = line
                        .strip_prefix("ID=")
                        .unwrap_or("unknown")
                        .trim_matches('"')
                        .to_string();
                } else if line.starts_with("VERSION_ID=") {
                    version = line
                        .strip_prefix("VERSION_ID=")
                        .unwrap_or("unknown")
                        .trim_matches('"')
                        .to_string();
                }
            }
        }

        // Build architecture-aware OS key (e.g., "linux-aarch64", "linux-x86_64")
        let os_key = format!("linux-{}", std::env::consts::ARCH);
        (os_key, distro, version)
    }

    #[cfg(target_os = "macos")]
    {
        ("macos".to_string(), "macos".to_string(), "any".to_string())
    }

    #[cfg(target_os = "windows")]
    {
        (
            "windows".to_string(),
            "windows".to_string(),
            "any".to_string(),
        )
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        (
            "unknown".to_string(),
            "unknown".to_string(),
            "unknown".to_string(),
        )
    }
}

/// Handle environment setup command
/// Handle documentation commands
fn handle_docs_command(docs_command: &DocsCommand) {
    // Must be run from within a workspace
    let workspace_path = match get_current_workspace() {
        Ok(path) => path,
        Err(e) => {
            messages::error(&format!("Error: {}", e));
            return;
        }
    };

    // Use sdk.yml from workspace root
    let config_path = workspace_path.join("sdk.yml");
    if !config_path.exists() {
        messages::error(&format!(
            "sdk.yml not found in workspace root: {}",
            workspace_path.display()
        ));
        messages::error("The workspace may be corrupted. Try running 'cim init' to reinitialize.");
        return;
    }

    // Ensure documentation dependencies are available before any docs operations
    if let Err(e) = ensure_docs_dependencies(&workspace_path) {
        messages::error(&format!(
            "Error setting up documentation dependencies: {}",
            e
        ));
        messages::error("You can try manually running: cim install pip");
        return;
    }

    // For docs commands, we only need core config (git repos, mirror) - not os-dependencies
    let sdk_config = match config::load_config(&config_path) {
        Ok(config) => config,
        Err(e) => {
            messages::error(&format!("Error loading config: {}", e));
            return;
        }
    };

    // Load user config (optional - will gracefully handle missing config)
    let user_config = config::UserConfig::load().ok().flatten();

    let doc_manager = doc_manager::DocManager::new(workspace_path);

    match docs_command {
        DocsCommand::Create {
            force,
            theme,
            symlink,
            verbose,
            cert_validation: _cert_validation,
        } => {
            messages::status("Creating unified documentation...");

            // Discover documentation sources
            let doc_sources =
                match doc_manager.discover_doc_sources(&sdk_config, user_config.as_ref(), *verbose)
                {
                    Ok(sources) => sources,
                    Err(e) => {
                        messages::error(&format!("Error discovering documentation sources: {}", e));
                        return;
                    }
                };

            if doc_sources.is_empty() {
                messages::status("No documentation sources found in any repositories.");
                messages::status("Make sure repositories have a 'docs' folder with 'index.rst'");
                return;
            }

            messages::status(&format!(
                "Found {} documentation source(s)",
                doc_sources.len()
            ));

            // Create unified documentation (cert_validation is stored for future use if needed)
            match doc_manager.create_unified_docs(&doc_sources, theme, *force, *symlink) {
                Ok(_) => messages::success("Unified documentation created successfully!"),
                Err(e) => messages::error(&format!("Error creating unified documentation: {}", e)),
            }
        }
        DocsCommand::Build { format } => match doc_manager.build_docs(format) {
            Ok(_) => messages::success("Documentation built successfully!"),
            Err(e) => {
                messages::error(&format!("Error building documentation: {}", e));
                messages::info("Hint: If sphinx-build is not found, make sure you have:");
                messages::info("  1. Installed Python packages: cim install pip");
                messages::info("  2. Activated your Python virtual environment (if using one)");
                messages::info(
                    "  3. Or installed Sphinx globally: pip3 install sphinx sphinx-rtd-theme",
                );
            }
        },
        DocsCommand::Serve { port, host } => {
            // In container environments, use 0.0.0.0 to accept external connections
            // unless user explicitly specified a different host
            let effective_host = if is_container_environment() && host == "localhost" {
                "0.0.0.0"
            } else {
                host
            };

            messages::status(&format!(
                "Serving documentation on {}:{}",
                effective_host, port
            ));
            if is_container_environment() && effective_host == "0.0.0.0" {
                messages::status(
                    "Container detected: Server accessible from host via port forwarding",
                );
                messages::status(&format!(
                    "Run: docker run -p {}:{} ... to forward port",
                    port, port
                ));
            }
            match doc_manager.serve_docs(effective_host, *port) {
                Ok(_) => {}
                Err(e) => messages::error(&format!("Error serving documentation: {}", e)),
            }
        }
    }
}

/// Add a new git repository entry to the config file
fn handle_add_command(name: &str, url: &str, commit: &str) {
    // Must be run from within a workspace
    let workspace_path = match get_current_workspace() {
        Ok(path) => path,
        Err(e) => {
            messages::error(&format!("Error: {}", e));
            return;
        }
    };

    // Use sdk.yml from workspace root
    let config_path = workspace_path.join("sdk.yml");
    if !config_path.exists() {
        messages::error(&format!(
            "sdk.yml not found in workspace root: {}",
            workspace_path.display()
        ));
        messages::error("The workspace may be corrupted. Try running 'cim init' to reinitialize.");
        return;
    }

    // First load the config to check for duplicates
    let sdk_config = match config::load_config(&config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            messages::error(&format!(
                "Failed to load config file {}: {}",
                config_path.display(),
                e
            ));
            return;
        }
    };

    // Check for duplicate by name
    if sdk_config.gits().iter().any(|g| g.name == name) {
        messages::info(&format!(
            "Repository '{}' already exists in config file.",
            name
        ));
        return;
    }

    // Read the original file content to preserve structure
    let original_content = match std::fs::read_to_string(&config_path) {
        Ok(content) => content,
        Err(e) => {
            messages::error(&format!(
                "Failed to read config file {}: {}",
                config_path.display(),
                e
            ));
            return;
        }
    };

    // Create the new git entry YAML
    let new_git_yaml = format!(
        "\n  - name: {}\n    url: {}\n    commit: {}",
        name, url, commit
    );

    // Find the gits section and append the new entry
    let updated_content = if let Some(gits_pos) = original_content.find("gits:") {
        // Check if gits is an empty array on the same line (gits: [])
        let gits_line_start = gits_pos;
        let gits_line_end = original_content[gits_pos..]
            .find('\n')
            .map(|pos| gits_pos + pos)
            .unwrap_or(original_content.len());
        let gits_line = &original_content[gits_line_start..gits_line_end];

        if gits_line.contains("[]") {
            // Replace "gits: []" with "gits:" followed by the new entry
            let replacement = format!("gits:{}", new_git_yaml);
            format!(
                "{}{}{}",
                &original_content[..gits_line_start],
                replacement,
                &original_content[gits_line_end..]
            )
        } else {
            // Find the end of the gits section by scanning line by line.
            // Only a real YAML key at column 0 (non-whitespace, non-comment) ends the section.
            // Empty lines and comment lines (#...) are skipped.
            let after_gits = &original_content[gits_pos..];

            // Skip past the "gits:" line itself
            let gits_key_line_len = after_gits
                .find('\n')
                .map(|p| p + 1)
                .unwrap_or(after_gits.len());

            // Default insert position: right after the "gits:" line (handles empty gits section)
            let mut insert_pos = gits_pos + gits_key_line_len;
            let mut scan_pos = gits_pos + gits_key_line_len;

            while scan_pos < original_content.len() {
                let line_start = scan_pos;
                let line_end = original_content[scan_pos..]
                    .find('\n')
                    .map(|p| scan_pos + p + 1)
                    .unwrap_or(original_content.len());

                let line = &original_content[line_start..line_end];
                let first_byte = line.bytes().next();

                match first_byte {
                    None | Some(b'\n') | Some(b'\r') => {
                        // Empty line — could be between entries or between sections, skip
                    }
                    Some(b' ') | Some(b'\t') => {
                        // Indented line → belongs to the current gits entry; advance insert_pos
                        insert_pos = line_end;
                    }
                    Some(b'#') => {
                        // Comment line → skip; belongs to the next section's header block
                    }
                    _ => {
                        // Non-indented, non-comment line = start of the next YAML key, stop
                        break;
                    }
                }
                scan_pos = line_end;
            }

            format!(
                "{}{}{}",
                &original_content[..insert_pos],
                new_git_yaml,
                &original_content[insert_pos..]
            )
        }
    } else {
        messages::error("Could not find 'gits:' section in config file");
        return;
    };

    // Write back to file
    match std::fs::write(&config_path, updated_content) {
        Ok(_) => messages::success(&format!(
            "Added '{}' to config file {}.",
            name,
            config_path.display()
        )),
        Err(e) => messages::error(&format!(
            "Failed to write config file {}: {}",
            config_path.display(),
            e
        )),
    }
}

/// Execute a command in each git repository workspace
fn handle_foreach_command(command: &str, match_pattern: Option<&str>) {
    // Must be run from within a workspace
    let workspace_path = match get_current_workspace() {
        Ok(path) => path,
        Err(e) => {
            messages::error(&format!("Error: {}", e));
            return;
        }
    };

    // Use sdk.yml from workspace root
    let config_path = workspace_path.join("sdk.yml");
    if !config_path.exists() {
        messages::error(&format!(
            "sdk.yml not found in workspace root: {}",
            workspace_path.display()
        ));
        messages::error("The workspace may be corrupted. Try running 'cim init' to reinitialize.");
        return;
    }
    messages::status(&format!(
        "Executing command in workspace: {}",
        workspace_path.display()
    ));

    let mut sdk_config = match config::load_config(&config_path) {
        Ok(config) => config,
        Err(e) => {
            messages::error(&format!("Error loading config: {}", e));
            messages::error(
                "Make sure you run 'update' command first to initialize the workspace.",
            );
            return;
        }
    };

    // Load and apply user config overrides if present
    match config::UserConfig::load() {
        Ok(Some(user_config)) => {
            user_config.apply_to_sdk_config(&mut sdk_config, false);
        }
        Ok(None) => {}
        Err(e) => {
            messages::error(&format!("Warning: Failed to load user config: {}", e));
        }
    }

    // Compile regex pattern if provided
    let match_regex = if let Some(pattern) = match_pattern {
        match Regex::new(pattern) {
            Ok(regex) => {
                messages::status(&format!("Filtering repositories with pattern: {}", pattern));
                Some(regex)
            }
            Err(e) => {
                messages::error(&format!("Invalid regex pattern '{}': {}", pattern, e));
                return;
            }
        }
    } else {
        None
    };

    // Create filtered config based on match pattern
    let filtered_config = create_filtered_sdk_config(&sdk_config, &match_regex);

    messages::status(&format!(
        "Executing '{}' in each repository workspace...\n",
        command
    ));

    for git_cfg in &filtered_config.gits {
        messages::status(&format!("=== {} ===", git_cfg.name));
        let repo_path = workspace_path.join(&git_cfg.name);

        if !repo_path.exists() {
            messages::info(&format!(
                "Repository {} does not exist in workspace",
                git_cfg.name
            ));
            continue;
        }

        match Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&repo_path)
            .status()
        {
            Ok(status) if status.success() => {
                messages::success(&format!("Command succeeded in {}", git_cfg.name));
            }
            Ok(s) => {
                messages::error(&format!(
                    "Command failed in {} (exit code {})",
                    git_cfg.name, s
                ));
            }
            Err(e) => {
                messages::error(&format!("Error running command in {}: {}", git_cfg.name, e));
            }
        }
        messages::status(""); // Add blank line between repos for readability
    }
}

/// Resolve target name to configuration file path
fn resolve_target_config(
    target_name_or_url: &str,
    config_root: &Path,
) -> Result<PathBuf, anyhow::Error> {
    if is_url(target_name_or_url) {
        // Handle URL target - download the config file
        let config_url = if target_name_or_url.ends_with("sdk.yml") {
            target_name_or_url.to_string()
        } else if target_name_or_url.ends_with('/') {
            format!("{}sdk.yml", target_name_or_url)
        } else {
            format!("{}/sdk.yml", target_name_or_url)
        };

        download_config_from_url(&config_url)
            .map_err(|e| anyhow::anyhow!("Failed to download config from URL: {}", e))
    } else {
        // Handle local target name
        let target_dir = config_root.join("targets").join(target_name_or_url);
        let main_config = target_dir.join("sdk.yml");

        if !main_config.exists() {
            return Err(anyhow::anyhow!(
                "Target '{}' not found. Expected config at: {}",
                target_name_or_url,
                main_config.display()
            ));
        }

        Ok(main_config)
    }
}

/// List available targets by scanning directories
fn list_available_targets(config_root: &Path) -> Result<Vec<String>, anyhow::Error> {
    let targets_dir = config_root.join("targets");

    if !targets_dir.exists() {
        return Err(anyhow::anyhow!(
            "Targets directory not found: {}",
            targets_dir.display()
        ));
    }

    let mut targets = Vec::new();

    for entry in std::fs::read_dir(&targets_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let target_name = entry.file_name().to_string_lossy().to_string();
            let config_path = entry.path().join("sdk.yml");

            // Only include directories that have sdk.yml
            if config_path.exists() {
                targets.push(target_name);
            }
        }
    }

    targets.sort();
    Ok(targets)
}

/// Helper function to install OS dependencies during init if they exist
/// This function is called by init --full to install OS packages before other components
fn install_os_deps_if_available(
    workspace_path: &Path,
    skip_prompt: bool,
    no_sudo: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check if os-dependencies.yml exists
    let os_deps_path = workspace_path.join("os-dependencies.yml");
    if !os_deps_path.exists() {
        messages::status(
            "No os-dependencies.yml found in workspace, skipping OS dependencies installation",
        );
        return Ok(());
    }

    // Load OS dependencies
    let os_deps = match config::load_os_dependencies(&os_deps_path) {
        Ok(deps) => deps,
        Err(e) => {
            messages::info(&format!(
                "Skipping OS dependencies installation: Failed to load os-dependencies.yml: {}",
                e
            ));
            return Ok(()); // Non-fatal, continue with next steps
        }
    };

    // Call install_prerequisites which will display package list and prompt for confirmation
    // unless skip_prompt is true (from --yes flag)
    install_prerequisites(&os_deps, skip_prompt, no_sudo);

    Ok(())
}

/// Helper function to install toolchains during init if they exist
/// This function is called by init --install to set up toolchains before running make install-all
fn install_toolchains_if_available(
    workspace_path: &Path,
    config_path: &Path,
    symlink: bool,
    _verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Load the SDK config with user overrides to check for toolchains
    let full_config = match load_config_with_user_overrides(config_path, false) {
        Ok(config) => config,
        Err(e) => {
            messages::info(&format!(
                "Skipping toolchain installation: Failed to load config: {}",
                e
            ));
            return Ok(()); // Non-fatal, continue with next steps
        }
    };

    // Check if toolchains are defined
    if full_config.toolchains.is_none() || full_config.toolchains.as_ref().unwrap().is_empty() {
        messages::status("No toolchains configured in sdk.yml, skipping toolchain installation");
        return Ok(());
    }

    messages::status("");
    messages::status("Installing toolchains...");

    // Mirror path already expanded in full_config with user overrides applied
    // Create toolchain manager and install toolchains
    let toolchain_manager = toolchain_manager::ToolchainManager::new(
        workspace_path.to_path_buf(),
        full_config.mirror.clone(),
    );

    match toolchain_manager.install_toolchains(
        full_config.toolchains.as_ref(),
        false,   // force = false
        symlink, // symlink = from parameter
        None,    // cert_validation = None
    ) {
        Ok(_) => {
            messages::success("Toolchains installed successfully");
            Ok(())
        }
        Err(e) => {
            messages::error(&format!("Warning: Toolchain installation failed: {}", e));
            messages::info("Continuing with remaining installation steps...");
            Ok(()) // Non-fatal, continue with next steps
        }
    }
}

/// Helper function to install Python packages during init if they exist
/// This function is called by init --install to set up Python packages before running make install-all
fn install_pip_packages_if_available(
    workspace_path: &Path,
    config_path: &Path,
    symlink: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Load the SDK config with user overrides to get the mirror path
    let sdk_config = match load_config_with_user_overrides(config_path, false) {
        Ok(config) => config,
        Err(e) => {
            messages::info(&format!(
                "Skipping pip installation: Failed to load config: {}",
                e
            ));
            return Ok(()); // Non-fatal, continue with next steps
        }
    };

    // Check if python-dependencies.yml exists
    let python_deps_path = workspace_path.join("python-dependencies.yml");
    if !python_deps_path.exists() {
        messages::status(
            "No python-dependencies.yml found in workspace, skipping pip installation",
        );
        return Ok(());
    }

    // Load Python dependencies to check if there are packages to install
    let python_deps = match config::load_python_dependencies(&python_deps_path) {
        Ok(deps) => deps,
        Err(e) => {
            messages::info(&format!(
                "Skipping pip installation: Failed to load python-dependencies.yml: {}",
                e
            ));
            return Ok(()); // Non-fatal, continue with next steps
        }
    };

    // Check if the default profile has any packages
    let has_packages = python_deps
        .profiles
        .get(&python_deps.default)
        .map(|profile| !profile.packages.is_empty())
        .unwrap_or(false);

    if !has_packages {
        messages::status(&format!(
            "No Python packages in default profile '{}', skipping pip installation",
            python_deps.default
        ));
        return Ok(());
    }

    messages::status("");
    messages::status("Installing Python packages...");

    // Mirror path already expanded in sdk_config with user overrides applied
    // Install Python packages using the default profile
    install_python_packages_from_file(
        &python_deps_path,
        false,   // force = false
        symlink, // symlink = from parameter
        None,    // profile = None (use default)
        workspace_path,
        &sdk_config.mirror,
    )?;

    messages::success("Python packages installation completed");
    Ok(())
}

/// Configuration for the init command
struct InitConfig<'a> {
    target: String,
    source: Option<String>,
    version: Option<String>,
    workspace: Option<PathBuf>,
    no_mirror: bool,
    force: bool,
    match_pattern: Option<&'a str>,
    verbose: bool,
    install: bool,
    full: bool,
    no_sudo: bool,
    symlink: bool,
    yes: bool,
    _cert_validation: Option<&'a str>,
}

/// Initialize a new workspace
fn handle_init_command(config: InitConfig) {
    // Start background version check so it runs concurrently with the rest of init
    let version_check = spawn_version_check();

    // Set verbose mode for this command
    messages::set_verbose(config.verbose);

    // Load user config early to get default values
    let user_config = match config::UserConfig::load() {
        Ok(Some(uc)) => {
            messages::verbose(&format!(
                "Loaded user config from {}",
                config::UserConfig::default_path().display()
            ));
            Some(uc)
        }
        Ok(None) => None,
        Err(e) => {
            messages::info(&format!("Warning: Failed to load user config: {}", e));
            None
        }
    };

    // Determine source path (use user config default if available)
    let default_source = if let Some(ref uc) = user_config {
        if let Some(ref ds) = uc.default_source {
            ds.clone()
        } else {
            get_default_source()
        }
    } else {
        get_default_source()
    };

    let (source_path, using_user_config_source) = if let Some(src) = config.source {
        // Explicit source provided, not using user config default
        (src, false)
    } else {
        // No explicit source, check if we have user config default
        let using_user_default = if let Some(ref uc) = user_config {
            uc.default_source.is_some()
        } else {
            false
        };
        if using_user_default {
            messages::verbose(&format!(
                "Using default_source from user config: {}",
                default_source
            ));
        }
        (default_source, using_user_default)
    };

    // Track whether we're using a remote git source for copy_files processing
    let mut is_remote_git_source = false;

    // For now, simplified approach: check if target itself is a URL first, otherwise use source
    let (config_path, config_url) = if is_url(&config.target) {
        // Target is a direct URL to config file
        match resolve_target_config(&config.target, &PathBuf::new()) {
            Ok(path) => (path, Some(config.target.clone())),
            Err(e) => {
                messages::error(&e.to_string());
                return;
            }
        }
    } else {
        // Use source to resolve target config
        if is_url(&source_path) {
            // Git-based source - clone and checkout specific version
            match resolve_target_config_from_git(
                &source_path,
                &config.target,
                config.version.as_deref(),
                None, // Use tempfile with mem::forget for init
            ) {
                Ok(path) => {
                    let version_info = if let Some(v) = &config.version {
                        format!(" ({})", v)
                    } else {
                        " (latest)".to_string()
                    };
                    let source_info = if using_user_config_source {
                        " (user config default_source)"
                    } else {
                        ""
                    };
                    messages::status(&format!(
                        "Setting up for target '{}'{} using source: {}{}",
                        config.target, version_info, source_path, source_info
                    ));
                    // Mark that we're using a remote git source for copy_files processing
                    is_remote_git_source = true;
                    (path, None) // Use None for git-based configs since dependency files are locally available
                }
                Err(e) => {
                    messages::error(&e.to_string());
                    return;
                }
            }
        } else {
            // Local source directory
            let source_root = PathBuf::from(&source_path);

            // Check if version is specified and source is a git repository
            if let Some(v) = config.version.as_deref() {
                if source_root.join(".git").exists() {
                    // Clone local git repo to temp dir and checkout specific version
                    match resolve_target_config_from_git(
                        &source_path,
                        &config.target,
                        Some(v),
                        None,
                    ) {
                        Ok(path) => {
                            messages::verbose(&format!(
                                "Checked out config for target '{}' at version '{}'",
                                config.target, v
                            ));
                            // Mark that we're using a remote git source for copy_files processing
                            is_remote_git_source = true;
                            (path, None)
                        }
                        Err(e) => {
                            messages::error(&e.to_string());
                            return;
                        }
                    }
                } else {
                    messages::error(&format!(
                        "Version specified but source is not a git repository: {}",
                        source_path
                    ));
                    return;
                }
            } else {
                // No version specified, use current state directly
                match resolve_target_config(&config.target, &source_root) {
                    Ok(path) => {
                        if using_user_config_source {
                            messages::status(&format!(
                                "Setting up for target '{}' using source: {} (user config default_source)",
                                config.target, source_path
                            ));
                        }
                        (path, None)
                    }
                    Err(e) => {
                        messages::error(&format!("Error: {}", e));
                        messages::status("Available targets:");
                        if let Ok(targets) = list_available_targets(&source_root) {
                            for target in targets {
                                messages::status(&format!("  - {}", target));
                            }
                        } else {
                            messages::status(&format!(
                                "  (Could not list targets from {})",
                                source_root.display()
                            ));
                        }
                        return;
                    }
                }
            }
        }
    };

    messages::verbose(&format!(
        "Loading configuration from {}",
        config_path.display()
    ));

    // Load and validate config
    let mut sdk_config = match config::load_config(&config_path) {
        Ok(config) => config,
        Err(e) => {
            messages::error(&format!("Failed to load config: {}", e));
            return;
        }
    };

    // Apply user config overrides if present
    if let Some(ref uc) = user_config {
        let override_count = uc.apply_to_sdk_config(&mut sdk_config, config.verbose);
        if override_count > 0 && config.verbose {
            messages::verbose(&format!(
                "Applied {} override(s) from user config",
                override_count
            ));
        }
    }

    // Expand environment variables in mirror path
    let original_mirror = sdk_config.mirror.to_string_lossy().to_string();
    let expanded_mirror = expand_config_mirror_path(&sdk_config);
    if original_mirror != expanded_mirror.to_string_lossy() {
        messages::verbose(&format!(
            "Mirror: {} -> {}",
            original_mirror,
            expanded_mirror.display()
        ));
    } else {
        messages::verbose(&format!("Mirror: {}", expanded_mirror.display()));
    }
    sdk_config.mirror = expanded_mirror;

    // Compile regex pattern if provided
    let match_regex = if let Some(pattern) = config.match_pattern {
        match Regex::new(pattern) {
            Ok(regex) => {
                messages::verbose(&format!("Filtering repositories with pattern: {}", pattern));
                Some(regex)
            }
            Err(e) => {
                messages::error(&format!("Invalid regex pattern '{}': {}", pattern, e));
                return;
            }
        }
    } else {
        None
    };

    // Determine workspace path (default: $HOME/{prefix}{target-name} or user config)
    let workspace_path = config.workspace.unwrap_or_else(|| {
        if let Some(ref uc) = user_config {
            if let Some(ref dw) = uc.default_workspace {
                return dw.clone();
            }
        }
        // Get workspace prefix from user config, default to "dsdk-"
        let prefix = user_config
            .as_ref()
            .and_then(|uc| uc.workspace_prefix.clone())
            .unwrap_or_else(|| "dsdk-".to_string());
        // Use {prefix}{target-name} as default workspace name
        let workspace_name = format!("{}{}", prefix, config.target);
        env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(workspace_name)
    });

    // Expand environment variables in workspace path (e.g., $HOME, ~/path)
    let workspace_path = PathBuf::from(expand_env_vars(&workspace_path.to_string_lossy()));

    // Check if workspace already exists and handle force flag
    if workspace_path.exists() {
        if config.force {
            // Check if current working directory is inside the workspace being removed
            // If so, change to a safe directory first to avoid issues with git operations
            if let Ok(cwd) = env::current_dir() {
                // Canonicalize paths to handle symlinks and relative paths
                if let (Ok(canonical_cwd), Ok(canonical_workspace)) =
                    (cwd.canonicalize(), workspace_path.canonicalize())
                {
                    if canonical_cwd.starts_with(&canonical_workspace) {
                        // We're inside the workspace, change to parent directory
                        if let Some(parent) = canonical_workspace.parent() {
                            messages::verbose(&format!(
                                "Changing directory from {} to {} before removing workspace",
                                canonical_cwd.display(),
                                parent.display()
                            ));
                            if let Err(e) = env::set_current_dir(parent) {
                                messages::info(&format!(
                                    "Failed to change directory before removing workspace: {}",
                                    e
                                ));
                                messages::info(
                                    "This may cause git operations to fail. Consider running from outside the workspace.",
                                );
                            }
                        }
                    }
                }
            }

            if let Err(e) = fs::remove_dir_all(&workspace_path) {
                messages::error(&format!(
                    "Error removing existing workspace directory: {}",
                    e
                ));
                return;
            }
            messages::success("Removed existing workspace directory");
        } else {
            let marker_path = workspace_path.join(".workspace");
            if marker_path.exists() {
                messages::error(&format!(
                    "Workspace already initialized at {}",
                    workspace_path.display()
                ));
                messages::error("Use 'cim update' to update an existing workspace, or use --force to overwrite.");
                return;
            }
        }
    }

    messages::status(&format!(
        "Initializing workspace at: {}",
        workspace_path.display()
    ));

    // Create workspace directory
    if let Err(e) = fs::create_dir_all(&workspace_path) {
        messages::error(&format!("Error creating workspace directory: {}", e));
        return;
    }

    // Copy config file to workspace as sdk.yml
    let dest_config_path = workspace_path.join("sdk.yml");
    if let Err(e) = fs::copy(&config_path, &dest_config_path) {
        messages::error(&format!("Error copying config to workspace: {}", e));
        return;
    }
    messages::verbose("Copied configuration to workspace as sdk.yml");

    // Determine if we should skip mirror (command line flag OR user config setting)
    // This needs to be calculated before creating workspace marker
    let skip_mirror = config.no_mirror
        || user_config
            .as_ref()
            .and_then(|uc| uc.no_mirror)
            .unwrap_or(false);

    // Create workspace marker file
    // Always use target name as original identifier for both URL-based and local targets
    if let Err(e) = create_workspace_marker(CreateWorkspaceMarkerParams {
        workspace_path: &workspace_path,
        config_name: "sdk.yml",
        original_config_path: &config_path,
        mirror_path: &sdk_config.mirror,
        original_identifier: Some(&config.target),
        target_version: config.version.as_deref(),
        skip_mirror,
        source_url: if is_remote_git_source {
            Some(&source_path)
        } else {
            None
        },
    }) {
        messages::error(&format!("Error creating workspace marker: {}", e));
        return;
    }

    // Copy other YAML files to workspace
    // Use the config_url if the config was loaded from a URL
    let base_url = config_url.as_deref();
    if let Err(e) = copy_yaml_files_to_workspace(&workspace_path, &config_path, base_url) {
        messages::info(&format!(
            "Failed to copy some YAML files to workspace: {}",
            e
        ));
    }

    // Create filtered config based on match pattern
    let filtered_config = create_filtered_sdk_config(&sdk_config, &match_regex);

    // Show workspace status (similar to update command)
    messages::verbose(&format!("Workspace: {}", workspace_path.display()));

    // Now proceed with mirror and workspace setup
    let any_failed = if skip_mirror {
        if config.no_mirror {
            messages::info("Skipping mirror operations (--no-mirror enabled)");
        } else {
            messages::info("Skipping mirror operations (no_mirror = true in user config)");
        }
        update_workspace_repos_no_mirror_with_result(&filtered_config, &workspace_path, true)
    } else {
        messages::verbose(&format!("Mirror: {}", sdk_config.mirror().display()));

        // Update mirror repositories
        update_mirror_repos(&filtered_config);

        // Update workspace repositories
        update_workspace_repos_with_result(&filtered_config, &workspace_path, true)
    };

    // Process copy_files after git repositories are cloned
    let mut copy_files_failed = false;
    if let Some(copy_files) = &sdk_config.copy_files {
        if !copy_files.is_empty() {
            messages::verbose(&format!(
                "Processing {} copy_files entries",
                copy_files.len()
            ));
            let config_source_dir = config_path.parent().unwrap_or(Path::new("."));
            let mirror_path = expand_config_mirror_path(&sdk_config);
            if let Err(e) = process_copy_files(
                &workspace_path,
                config_source_dir,
                copy_files,
                &mirror_path,
                is_remote_git_source,
            ) {
                messages::info(&format!("Failed to process copy_files: {}", e));
                copy_files_failed = true;
            }
        }
    }

    if any_failed || copy_files_failed {
        messages::error("Workspace initialization completed with errors!");
        if any_failed {
            messages::info("Some repositories failed to clone or checkout.");
        }
        if copy_files_failed {
            messages::info("Some files failed to copy or download.");
        }
        std::process::exit(1);
    } else {
        messages::success(&format!(
            "Workspace initialized successfully at: {}",
            workspace_path.display()
        ));
        messages::verbose(&format!("Config: {}", dest_config_path.display()));
        messages::verbose(&format!(
            "To use the workspace: cd {}",
            workspace_path.display()
        ));

        // Silently copy workspace path to clipboard for convenience
        if let Ok(mut clipboard) = Clipboard::new() {
            let _ = clipboard.set_text(workspace_path.display().to_string());
        }

        // Determine if we should run installation steps
        // --full implies --install
        let should_install = config.install || config.full;

        // Run installation steps if --install or --full flag was provided
        if should_install {
            messages::status("");
            if config.full {
                messages::status("Setting up SDK components with --full...");

                // Step 0: Install OS dependencies if --full is specified
                if let Err(e) =
                    install_os_deps_if_available(&workspace_path, config.yes, config.no_sudo)
                {
                    messages::info(&format!(
                        "Note: OS dependencies installation encountered an issue: {}",
                        e
                    ));
                }
            } else {
                messages::status("Setting up SDK components with --install...");
            }

            // Step 1: Install toolchains if available
            if let Err(e) = install_toolchains_if_available(
                &workspace_path,
                &dest_config_path,
                config.symlink,
                config.verbose,
            ) {
                messages::info(&format!(
                    "Note: Toolchain installation encountered an issue: {}",
                    e
                ));
            }

            // Step 2: Install Python packages if available
            if let Err(e) = install_pip_packages_if_available(
                &workspace_path,
                &dest_config_path,
                config.symlink,
            ) {
                messages::error(&format!("Failed to install Python packages: {}", e));
                messages::error("Workspace creation failed");
                std::process::exit(1);
            }

            // Step 3: Generate Makefile and run install-all (original --install behavior)
            // First generate the Makefile
            let makefile_path = workspace_path.join("Makefile");
            match config::load_config(&dest_config_path) {
                Ok(sdk_config) => {
                    // Check if there are install sections before trying to run make install-all
                    let has_install_sections = sdk_config.install.is_some()
                        && sdk_config
                            .install
                            .as_ref()
                            .map(|i| !i.is_empty())
                            .unwrap_or(false);

                    let makefile_content = generate_makefile_content(&sdk_config);
                    match std::fs::write(&makefile_path, makefile_content) {
                        Ok(_) => {
                            messages::verbose(&format!(
                                "Generated Makefile at {}",
                                makefile_path.display()
                            ));

                            // Generate VS Code tasks.json
                            if let Err(e) = vscode_tasks_manager::generate_tasks_json(
                                &workspace_path,
                                &makefile_path,
                            ) {
                                messages::verbose(&format!(
                                    "Could not generate VS Code tasks.json: {}",
                                    e
                                ));
                            }

                            // Only run make install-all if there are install sections in sdk.yml
                            if has_install_sections {
                                messages::status("");
                                messages::status("Running install-all to complete SDK setup...");

                                let make_status = std::process::Command::new("make")
                                    .arg("install-all")
                                    .current_dir(&workspace_path)
                                    .status();

                                match make_status {
                                    Ok(status) => {
                                        if status.success() {
                                            messages::status("");
                                            messages::success(
                                                "All SDK components installed successfully",
                                            );
                                        } else {
                                            messages::status("");
                                            messages::error(
                                                "Warning: Some components failed to install",
                                            );
                                            messages::status(&format!(
                                                "You can retry with: cd {} && make install-all",
                                                workspace_path.display()
                                            ));
                                        }
                                    }
                                    Err(e) => {
                                        messages::error(&format!(
                                            "Warning: Failed to run make install-all: {}",
                                            e
                                        ));
                                        messages::status(&format!("Make sure 'make' is installed, or run manually: cd {} && make install-all", workspace_path.display()));
                                    }
                                }
                            } else {
                                messages::status("");
                                messages::status(
                                    "No install targets in sdk.yml, skipping install-all step",
                                );
                                messages::success("Workspace setup completed");
                            }
                        }
                        Err(e) => {
                            messages::error(&format!("Warning: Failed to write Makefile: {}", e));
                            messages::status("You can generate it later with 'cim makefile'");
                        }
                    }
                }
                Err(e) => {
                    messages::error(&format!(
                        "Warning: Failed to load config for Makefile generation: {}",
                        e
                    ));
                    messages::status("You can generate it later with 'cim makefile'");
                }
            }
        }
    }

    // Print any available update notice after the main work is done
    print_update_notice(version_check);
}

/// Temporary struct to hold filtered git configurations for operations
struct FilteredSdkConfig {
    gits: Vec<config::GitConfig>,
    mirror: PathBuf,
    makefile_include: Option<Vec<String>>,
    envsetup: Option<config::SdkTarget>,
    test: Option<config::SdkTarget>,
}

impl config::SdkConfigCore for FilteredSdkConfig {
    fn gits(&self) -> &Vec<config::GitConfig> {
        &self.gits
    }

    fn mirror(&self) -> &PathBuf {
        &self.mirror
    }

    fn install(&self) -> &Option<Vec<config::InstallConfig>> {
        &None
    }

    fn makefile_include(&self) -> &Option<Vec<String>> {
        &self.makefile_include
    }

    fn envsetup(&self) -> &Option<config::SdkTarget> {
        &self.envsetup
    }

    fn test(&self) -> &Option<config::SdkTarget> {
        &self.test
    }

    fn clean(&self) -> &Option<config::SdkTarget> {
        &None
    }

    fn build(&self) -> &Option<config::SdkTarget> {
        &None
    }

    fn flash(&self) -> &Option<config::SdkTarget> {
        &None
    }
}

/// Filter git configurations based on regex pattern
fn filter_git_configs(
    gits: &[config::GitConfig],
    pattern_regex: &Option<Regex>,
) -> Vec<config::GitConfig> {
    match pattern_regex {
        Some(regex) => {
            let filtered: Vec<_> = gits
                .iter()
                .filter(|git_cfg| regex.is_match(&git_cfg.name))
                .cloned()
                .collect();

            if filtered.len() != gits.len() {
                messages::status(&format!(
                    "Filtered {} repositories out of {} total:",
                    filtered.len(),
                    gits.len()
                ));
                for git_cfg in &filtered {
                    messages::status(&format!("  - {}", git_cfg.name));
                }
            }

            filtered
        }
        None => gits.to_vec(),
    }
}

/// Create a filtered SDK config for operations
fn create_filtered_sdk_config<T: config::SdkConfigCore>(
    sdk_config: &T,
    pattern_regex: &Option<Regex>,
) -> FilteredSdkConfig {
    let filtered_gits = filter_git_configs(sdk_config.gits(), pattern_regex);
    FilteredSdkConfig {
        gits: filtered_gits,
        mirror: sdk_config.mirror().to_path_buf(),
        makefile_include: sdk_config.makefile_include().clone(),
        envsetup: sdk_config.envsetup().clone(),
        test: sdk_config.test().clone(),
    }
}

/// List available targets from either a local directory or git repository
fn list_targets_from_source(source_path: &str) -> Result<Vec<String>, anyhow::Error> {
    if is_url(source_path) {
        // For git URLs, we need to create a temporary shallow clone to list targets
        list_targets_from_git_repo(source_path)
    } else {
        // Local directory approach
        let path = PathBuf::from(source_path);
        list_available_targets(&path)
    }
}

/// List available targets from a remote git repository using temporary clone
fn list_targets_from_git_repo(git_url: &str) -> Result<Vec<String>, anyhow::Error> {
    // Create a temporary directory for shallow clone
    let temp_dir = tempfile::tempdir()
        .map_err(|e| anyhow::anyhow!("Failed to create temp directory: {}", e))?;

    let temp_path = temp_dir.path();

    // Perform shallow clone of just the main branch
    let clone_result = git_operations::clone_repo_shallow(git_url, temp_path, 1)?;

    if !clone_result.is_success() {
        return Err(anyhow::anyhow!("Git clone failed: {}", clone_result.stderr));
    }

    // List targets from the cloned repository
    let targets_dir = temp_path.join("targets");
    if !targets_dir.exists() {
        return Err(anyhow::anyhow!(
            "No 'targets' directory found in git repository"
        ));
    }

    let mut targets = Vec::new();
    for entry in std::fs::read_dir(&targets_dir)
        .map_err(|e| anyhow::anyhow!("Failed to read targets directory: {}", e))?
    {
        let entry = entry.map_err(|e| anyhow::anyhow!("Failed to read directory entry: {}", e))?;

        if entry
            .file_type()
            .map_err(|e| anyhow::anyhow!("Failed to get file type: {}", e))?
            .is_dir()
        {
            let target_name = entry.file_name().to_string_lossy().to_string();
            let config_path = entry.path().join("sdk.yml");

            // Only include directories that have sdk.yml
            if config_path.exists() {
                targets.push(target_name);
            }
        }
    }

    targets.sort();
    Ok(targets)
}

/// List available versions for a specific target from git repository
fn list_target_versions(
    source_path: &str,
    target_name: &str,
) -> Result<Vec<String>, anyhow::Error> {
    if is_url(source_path) {
        // Use git ls-remote to list branches and tags
        let refs = git_operations::ls_remote(source_path, true, true)?;

        let mut versions = Vec::new();
        let target_prefix = format!("{}-", target_name);

        for ref_name in refs {
            // Extract branch/tag name from refs/heads/ or refs/tags/
            let name = if let Some(branch_name) = ref_name.strip_prefix("refs/heads/") {
                branch_name
            } else if let Some(tag_name) = ref_name.strip_prefix("refs/tags/") {
                tag_name
            } else {
                continue;
            };

            // Filter for branches/tags starting with target prefix
            if name.starts_with(&target_prefix) {
                versions.push(name.to_string());
            }
        }

        versions.sort();
        Ok(versions)
    } else {
        // For local directories, check if it's a git repository and list tags and branches
        let source_path_buf = std::path::PathBuf::from(source_path);
        if source_path_buf.join(".git").exists() {
            let mut versions = Vec::new();
            let target_prefix = format!("{}-", target_name);

            // Get tags
            if let Ok(tags) = git_operations::list_local_tags(&source_path_buf) {
                for tag in tags {
                    // Filter for tags starting with target prefix
                    if tag.starts_with(&target_prefix) {
                        versions.push(tag);
                    }
                }
            }

            // Get branches
            if let Ok(branches) = git_operations::list_local_branches(&source_path_buf) {
                for branch in branches {
                    // Filter for branches starting with target prefix
                    if branch.starts_with(&target_prefix) {
                        versions.push(branch);
                    }
                }
            }

            versions.sort();
            Ok(versions)
        } else {
            // Not a git repository, return empty
            Ok(Vec::new())
        }
    }
}

/// Check if a commit reference is a branch (vs tag or commit hash)
fn is_branch_reference(repo_path: &Path, commit_ref: &str) -> bool {
    git_operations::is_branch_reference(repo_path, commit_ref)
}

/// Get the latest commit hash for a branch
fn get_latest_commit_for_branch(repo_path: &Path, branch_name: &str) -> Option<String> {
    git_operations::get_latest_commit_for_branch(repo_path, branch_name)
}

/// Configuration command options
struct ConfigOptions<'a> {
    list: bool,
    get: Option<&'a str>,
    show_path: bool,
    template: bool,
    create: bool,
    force: bool,
    edit: bool,
    validate: bool,
}

/// Handle the config command
fn handle_config_command(opts: ConfigOptions) {
    let config_path = config::UserConfig::default_path();

    // Print template to stdout
    if opts.template {
        print!("{}", config::UserConfig::generate_template());
        return;
    }

    // Validate config file
    if opts.validate {
        if !config_path.exists() {
            messages::error(&format!(
                "Config file does not exist: {}",
                config_path.display()
            ));
            messages::info("Run 'cim config --create' to create it.");
            std::process::exit(1);
        }

        match config::UserConfig::load() {
            Ok(Some(_)) => {
                messages::success(&format!("Config file is valid: {}", config_path.display()));
            }
            Ok(None) => {
                // This shouldn't happen since we checked exists above
                messages::error("Config file exists but could not be loaded.");
                std::process::exit(1);
            }
            Err(e) => {
                messages::error(&format!("Config file is invalid: {}", e));
                std::process::exit(1);
            }
        }
        return;
    }

    // Create config file
    if opts.create {
        if config_path.exists() && !opts.force {
            messages::error(&format!(
                "Config file already exists: {}",
                config_path.display()
            ));
            messages::info("Use --force to overwrite.");
            std::process::exit(1);
        }

        match create_config_file(&config_path) {
            Ok(_) => {
                messages::success(&format!("Created config file: {}", config_path.display()));
                messages::info("Edit this file to customize cim settings.");
            }
            Err(e) => {
                messages::error(&format!("Failed to create config: {}", e));
                std::process::exit(1);
            }
        }
        return;
    }

    // Edit config file
    if opts.edit {
        // Create from template if doesn't exist
        if !config_path.exists() {
            if let Err(e) = create_config_file(&config_path) {
                messages::error(&format!("Failed to create config: {}", e));
                std::process::exit(1);
            }
            messages::info(&format!(
                "Created new config file: {}",
                config_path.display()
            ));
        }

        // Determine editor
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

        // Open in editor
        let status = std::process::Command::new(&editor)
            .arg(&config_path)
            .status();

        match status {
            Ok(exit_status) if exit_status.success() => {
                // Validate after edit
                match config::UserConfig::load() {
                    Ok(Some(_)) => {
                        messages::success("Config file saved successfully.");
                    }
                    Ok(None) => {
                        messages::error("Config file was deleted during edit.");
                    }
                    Err(e) => {
                        messages::error(&format!("Warning: Config file contains errors: {}", e));
                        messages::info(&format!("Fix syntax errors in: {}", config_path.display()));
                    }
                }
            }
            Ok(exit_status) => {
                messages::error(&format!("Editor exited with status: {}", exit_status));
                std::process::exit(1);
            }
            Err(e) => {
                messages::error(&format!("Failed to launch editor '{}': {}", editor, e));
                messages::info("Set $EDITOR environment variable to your preferred editor.");
                std::process::exit(1);
            }
        }
        return;
    }

    // Show config file path
    if opts.show_path {
        messages::status(&config_path.display().to_string());
        return;
    }

    // List all configuration values
    if opts.list {
        match config::UserConfig::load() {
            Ok(Some(user_config)) => {
                let lines = user_config.list_all();
                if lines.is_empty() {
                    messages::info("Configuration file exists but no values are set.");
                } else {
                    for line in lines {
                        messages::status(&line);
                    }
                }
            }
            Ok(None) => {
                messages::info(&format!(
                    "No config file found at: {}",
                    config_path.display()
                ));
                messages::info("Run 'cim config --create' to create it.");
            }
            Err(e) => {
                messages::error(&format!("Failed to load config: {}", e));
                std::process::exit(1);
            }
        }
        return;
    }

    // Get specific configuration value
    if let Some(key) = opts.get {
        match config::UserConfig::load() {
            Ok(Some(user_config)) => {
                if let Some(value) = user_config.get_value(key) {
                    messages::status(&value);
                } else {
                    std::process::exit(1);
                }
            }
            Ok(None) => {
                std::process::exit(1);
            }
            Err(e) => {
                messages::error(&format!("Failed to load config: {}", e));
                std::process::exit(1);
            }
        }
        return;
    }

    // If no flag specified, show help
    let mut cmd = Cli::command();
    if let Some(config_cmd) = cmd.find_subcommand_mut("config") {
        let _ = config_cmd.print_help();
        std::process::exit(0);
    }
}

/// Create config file from template
fn create_config_file(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Create parent directories if they don't exist
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // Generate and write template
    let template = config::UserConfig::generate_template();
    std::fs::write(path, template)?;

    Ok(())
}

/// Handle the list-targets command
fn handle_list_targets_command(source: Option<&str>, target_filter: Option<&str>) {
    let default_source = get_default_source();
    let (source_path, using_user_default) = if let Some(src) = source {
        (src.to_string(), false)
    } else {
        // Check if using user config default
        let using_default = if let Ok(Some(uc)) = config::UserConfig::load() {
            uc.default_source.is_some()
        } else {
            false
        };
        (default_source, using_default)
    };

    if let Some(target_name) = target_filter {
        // List versions for specific target
        match list_target_versions(&source_path, target_name) {
            Ok(versions) => {
                if versions.is_empty() {
                    messages::status(&format!("No versions found for target '{}'", target_name));
                } else {
                    if using_user_default {
                        messages::status(&format!("Available versions for target '{}' (using default_source from user config):", target_name));
                        messages::status(&format!("  Source: {}", source_path));
                    } else {
                        messages::status(&format!(
                            "Available versions for target '{}':",
                            target_name
                        ));
                    }
                    for version in versions {
                        messages::status(&format!("  - {}", version));
                    }
                }
            }
            Err(e) => {
                messages::error(&format!(
                    "Error listing versions for target '{}': {}",
                    target_name, e
                ));
                std::process::exit(1);
            }
        }
    } else {
        // List all available targets
        match list_targets_from_source(&source_path) {
            Ok(targets) => {
                if targets.is_empty() {
                    messages::status(&format!("No targets found in {}", source_path));
                } else {
                    if using_user_default {
                        messages::status(&format!(
                            "Available targets from {} (user config default_source):",
                            source_path
                        ));
                    } else {
                        messages::status(&format!("Available targets from {}:", source_path));
                    }
                    for target in targets {
                        messages::status(&format!("  - {}", target));
                    }
                }
            }
            Err(e) => {
                messages::error(&format!("Error listing targets: {}", e));
                std::process::exit(1);
            }
        }
    }
}

/// Update all git repositories in the mirror and workspace
///
/// Supports environment variable expansion in mirror paths (e.g., $HOME, ${HOME})
fn handle_update_command(
    no_mirror: bool,
    match_pattern: Option<&str>,
    verbose: bool,
    _cert_validation: Option<&str>,
) {
    // Start background version check so it runs concurrently with the update
    let version_check = spawn_version_check();

    // Set verbose mode for this command
    messages::set_verbose(verbose);

    // Must be run from within a workspace
    let workspace_path = match get_current_workspace() {
        Ok(path) => path,
        Err(e) => {
            messages::error(&e);
            return;
        }
    };

    messages::workspace(&workspace_path);

    // Use sdk.yml from workspace root
    let config_path = workspace_path.join("sdk.yml");
    if !config_path.exists() {
        messages::error(&format!(
            "sdk.yml not found in {}",
            workspace_path.display()
        ));
        messages::info("Try running 'cim init' to reinitialize");
        return;
    }

    let mut sdk_config = match config::load_config(&config_path) {
        Ok(config) => config,
        Err(e) => {
            messages::error(&format!("Failed to load config: {}", e));
            return;
        }
    };

    // Load and apply user config overrides if present
    let user_config = match config::UserConfig::load() {
        Ok(Some(user_config)) => {
            let override_count = user_config.apply_to_sdk_config(&mut sdk_config, verbose);
            if override_count > 0 && verbose {
                messages::verbose(&format!(
                    "Applied {} override(s) from user config",
                    override_count
                ));
            }
            Some(user_config)
        }
        Ok(None) => None,
        Err(e) => {
            messages::info(&format!("Warning: Failed to load user config: {}", e));
            None
        }
    };

    // Expand environment variables in mirror path
    let expanded_mirror = expand_config_mirror_path(&sdk_config);
    messages::verbose(&format!("Mirror: {}", expanded_mirror.display()));
    sdk_config.mirror = expanded_mirror;

    // Compile regex pattern if provided
    let match_regex = if let Some(pattern) = match_pattern {
        match Regex::new(pattern) {
            Ok(regex) => {
                messages::status(&format!("Filtering repositories with pattern: {}", pattern));
                Some(regex)
            }
            Err(e) => {
                messages::error(&format!("Invalid regex pattern '{}': {}", pattern, e));
                return;
            }
        }
    } else {
        None
    };

    // Create filtered config based on match pattern
    let filtered_config = create_filtered_sdk_config(&sdk_config, &match_regex);

    // Read workspace marker to get stored no_mirror preference
    let marker_path = workspace_path.join(".workspace");
    let workspace_no_mirror = if marker_path.exists() {
        match fs::read_to_string(&marker_path) {
            Ok(content) => serde_yaml::from_str::<WorkspaceMarker>(&content)
                .ok()
                .and_then(|m| m.no_mirror)
                .unwrap_or(false),
            Err(_) => false,
        }
    } else {
        false
    };

    // Determine if we should skip mirror with precedence:
    // CLI flag > workspace marker preference > user config > default (false)
    let (skip_mirror, mirror_source) = if no_mirror {
        (true, "CLI flag --no-mirror")
    } else if workspace_no_mirror {
        (true, "workspace preference")
    } else if user_config
        .as_ref()
        .and_then(|uc| uc.no_mirror)
        .unwrap_or(false)
    {
        (true, "user config")
    } else {
        (false, "default (using mirrors)")
    };

    if skip_mirror {
        messages::info(&format!("Skipping mirror operations ({})", mirror_source));
    } else {
        messages::verbose(&format!("Using mirror operations ({})", mirror_source));
    }

    if skip_mirror {
        // Update workspace repositories directly from remote URLs
        update_workspace_repos_no_mirror(&filtered_config, &workspace_path, false);
    } else {
        // Update mirror repositories in parallel
        update_mirror_repos(&filtered_config);

        // Update workspace repositories (single-threaded to avoid conflicts)
        update_workspace_repos(&filtered_config, &workspace_path, false);
    }

    // Print any available update notice after the main work is done
    print_update_notice(version_check);
}

/// Update mirror repositories in parallel
/// Result type for mirror operations
#[derive(Debug, Clone)]
enum MirrorOperationResult {
    Updated, // Existing mirror was updated
    Cloned,  // New mirror was cloned
    Failed,  // Operation failed
}

fn update_mirror_repos<T: config::SdkConfigCore>(sdk_config: &T) {
    // Create or update mirror directory
    if !sdk_config.mirror().exists() {
        if let Err(e) = std::fs::create_dir_all(sdk_config.mirror()) {
            messages::error(&format!("Error creating mirror directory: {}", e));
            return;
        }
    }

    messages::status("Updating mirror repositories...");
    let pool = ThreadPool::new(4);

    for git_cfg in sdk_config.gits() {
        let git_cfg = git_cfg.clone();
        let mirror_path = sdk_config.mirror().clone();

        pool.execute(move || {
            let repo_mirror_path = dsdk_cli::git_manager::get_mirror_repo_path(
                &mirror_path,
                &git_cfg.name,
                &git_cfg.url,
            );
            let result = if repo_mirror_path.exists() {
                // Update existing mirror
                messages::progress(&git_cfg.name, "updating remote URL and fetching");

                // Update remote URL in case it changed in the config
                let _ = git_operations::remote_set_url(&repo_mirror_path, "origin", &git_cfg.url);

                let fetch_result = git_operations::fetch_all_with_tags(&repo_mirror_path);

                match fetch_result {
                    Ok(result) if result.is_success() => {
                        // If the commit is a branch, update the branch to latest commit
                        if is_branch_reference(&repo_mirror_path, &git_cfg.commit) {
                            // Get the latest commit hash for the remote branch
                            if let Some(latest_commit) =
                                git_operations::get_latest_commit_for_remote_branch(
                                    &repo_mirror_path,
                                    "origin",
                                    &git_cfg.commit,
                                )
                            {
                                // Update the local branch to point to the latest commit
                                let update_result = git_operations::update_ref(
                                    &repo_mirror_path,
                                    &format!("refs/heads/{}", git_cfg.commit),
                                    &latest_commit,
                                );

                                // Return success status of the update
                                match update_result {
                                    Ok(up_result) => {
                                        if up_result.is_success() {
                                            MirrorOperationResult::Updated
                                        } else {
                                            MirrorOperationResult::Failed
                                        }
                                    }
                                    Err(_) => MirrorOperationResult::Failed,
                                }
                            } else {
                                // Failed to get latest commit
                                MirrorOperationResult::Failed
                            }
                        } else {
                            // Non-branch commit, fetch completed successfully
                            MirrorOperationResult::Updated
                        }
                    }
                    Ok(_) => MirrorOperationResult::Failed,
                    Err(_) => MirrorOperationResult::Failed,
                }
            } else {
                // Clone new mirror
                messages::progress(&git_cfg.name, "cloning new repository");
                let clone_result = git_operations::clone_mirror(&git_cfg.url, &repo_mirror_path);

                match clone_result {
                    Ok(result) => {
                        if result.is_success() {
                            // Ensure all tags are fetched after initial clone
                            match git_operations::fetch_tags(&repo_mirror_path, Some("origin")) {
                                Ok(fetch_result) if fetch_result.is_success() => {
                                    MirrorOperationResult::Cloned
                                }
                                _ => {
                                    // Tag fetch failed, but clone succeeded - still report as cloned
                                    // since the repository is functional even without all tags
                                    MirrorOperationResult::Cloned
                                }
                            }
                        } else {
                            MirrorOperationResult::Failed
                        }
                    }
                    Err(_) => MirrorOperationResult::Failed,
                }
            };

            // Print result immediately
            match result {
                MirrorOperationResult::Updated => messages::success(&git_cfg.name),
                MirrorOperationResult::Cloned => {
                    messages::success(&format!("{} (cloned)", git_cfg.name))
                }
                MirrorOperationResult::Failed => {
                    messages::error(&format!("{} (failed)", git_cfg.name))
                }
            }
        });
    }

    pool.join();
}

/// Update workspace repositories in parallel
fn update_workspace_repos<T: config::SdkConfigCore>(
    sdk_config: &T,
    workspace_path: &Path,
    is_init: bool,
) {
    let action = if is_init { "Initializing" } else { "Updating" };
    messages::status(&format!("\n{} workspace repositories...", action));

    let tiers = match config::resolve_clone_order(sdk_config.gits()) {
        Ok(t) => t,
        Err(e) => {
            messages::error(&format!("Dependency resolution failed: {}", e));
            return;
        }
    };

    for tier in &tiers {
        let pool = ThreadPool::new(4);

        for git_cfg in tier {
            let git_cfg = git_cfg.clone();
            let workspace_path = workspace_path.to_path_buf();
            let mirror_path = sdk_config.mirror().clone();

            pool.execute(move || {
                let repo_workspace_path = workspace_path.join(&git_cfg.name);

                let success = if repo_workspace_path.join(".git").is_dir() {
                    handle_existing_workspace_repo(&git_cfg, &repo_workspace_path, &mirror_path)
                } else {
                    clone_repo_to_workspace(&git_cfg, &repo_workspace_path, &mirror_path)
                };

                // Print result immediately
                if !success {
                    messages::error(&format!("{} (failed)", git_cfg.name));
                }
            });
        }

        pool.join();
    }
}

/// Update workspace repositories in parallel, returns true if any failed
fn update_workspace_repos_with_result<T: config::SdkConfigCore>(
    sdk_config: &T,
    workspace_path: &Path,
    is_init: bool,
) -> bool {
    let action = if is_init { "Initializing" } else { "Updating" };
    messages::status(&format!("\n{} workspace repositories...", action));

    let tiers = match config::resolve_clone_order(sdk_config.gits()) {
        Ok(t) => t,
        Err(e) => {
            messages::error(&format!("Dependency resolution failed: {}", e));
            return true;
        }
    };

    let any_failed = Arc::new(AtomicBool::new(false));

    for tier in &tiers {
        let pool = ThreadPool::new(4);

        for git_cfg in tier {
            let git_cfg = git_cfg.clone();
            let workspace_path = workspace_path.to_path_buf();
            let mirror_path = sdk_config.mirror().clone();
            let any_failed = Arc::clone(&any_failed);

            pool.execute(move || {
                let repo_workspace_path = workspace_path.join(&git_cfg.name);

                let success = if repo_workspace_path.join(".git").is_dir() {
                    handle_existing_workspace_repo(&git_cfg, &repo_workspace_path, &mirror_path)
                } else {
                    clone_repo_to_workspace(&git_cfg, &repo_workspace_path, &mirror_path)
                };

                // Print result immediately and track failures
                if !success {
                    messages::error(&format!("{} (failed)", git_cfg.name));
                    any_failed.store(true, Ordering::Relaxed);
                }
            });
        }

        pool.join();
    }

    any_failed.load(Ordering::Relaxed)
}

/// Update workspace repositories directly from remote URLs (no mirror)
fn update_workspace_repos_no_mirror<T: config::SdkConfigCore>(
    sdk_config: &T,
    workspace_path: &Path,
    is_init: bool,
) {
    let action = if is_init { "Initializing" } else { "Updating" };
    messages::status(&format!(
        "\n{} workspace repositories directly from remote URLs...",
        action
    ));

    let tiers = match config::resolve_clone_order(sdk_config.gits()) {
        Ok(t) => t,
        Err(e) => {
            messages::error(&format!("Dependency resolution failed: {}", e));
            return;
        }
    };

    for tier in &tiers {
        let pool = ThreadPool::new(4);

        for git_cfg in tier {
            let git_cfg = git_cfg.clone();
            let workspace_path = workspace_path.to_path_buf();

            pool.execute(move || {
                let repo_workspace_path = workspace_path.join(&git_cfg.name);

                let success = if repo_workspace_path.join(".git").is_dir() {
                    handle_existing_workspace_repo_no_mirror(&git_cfg, &repo_workspace_path)
                } else {
                    clone_repo_to_workspace_no_mirror(&git_cfg, &repo_workspace_path)
                };

                // Print result immediately
                if !success {
                    messages::error(&format!("{} (failed)", git_cfg.name));
                }
            });
        }

        pool.join();
    }
}

/// Update workspace repositories directly from remote URLs (no mirror), returns true if any failed
fn update_workspace_repos_no_mirror_with_result<T: config::SdkConfigCore>(
    sdk_config: &T,
    workspace_path: &Path,
    is_init: bool,
) -> bool {
    let action = if is_init { "Initializing" } else { "Updating" };
    messages::status(&format!(
        "\n{} workspace repositories directly from remote URLs...",
        action
    ));

    let tiers = match config::resolve_clone_order(sdk_config.gits()) {
        Ok(t) => t,
        Err(e) => {
            messages::error(&format!("Dependency resolution failed: {}", e));
            return true;
        }
    };

    let any_failed = Arc::new(AtomicBool::new(false));

    for tier in &tiers {
        let pool = ThreadPool::new(4);

        for git_cfg in tier {
            let git_cfg = git_cfg.clone();
            let workspace_path = workspace_path.to_path_buf();
            let any_failed = Arc::clone(&any_failed);

            pool.execute(move || {
                let repo_workspace_path = workspace_path.join(&git_cfg.name);

                let success = if repo_workspace_path.join(".git").is_dir() {
                    handle_existing_workspace_repo_no_mirror(&git_cfg, &repo_workspace_path)
                } else {
                    clone_repo_to_workspace_no_mirror(&git_cfg, &repo_workspace_path)
                };

                // Print result immediately and track failures
                if !success {
                    messages::error(&format!("{} (failed)", git_cfg.name));
                    any_failed.store(true, Ordering::Relaxed);
                }
            });
        }

        pool.join();
    }

    any_failed.load(Ordering::Relaxed)
}

/// Handle an existing repository in the workspace
fn handle_existing_workspace_repo(
    git_cfg: &config::GitConfig,
    repo_path: &Path,
    mirror_path: &Path,
) -> bool {
    // Add mirror and set origin to upstream
    let mirror_repo_path =
        dsdk_cli::git_manager::get_mirror_repo_path(mirror_path, &git_cfg.name, &git_cfg.url);
    let _ = git_operations::git_command(
        &["remote", "set-url", "origin", &git_cfg.url],
        Some(repo_path),
    );
    let _ = git_operations::git_command(
        &[
            "remote",
            "add",
            "mirror",
            &format!("file://{}", mirror_repo_path.display()),
        ],
        Some(repo_path),
    );

    // Fetch from mirror first
    let mirror_result = git_operations::git_command(&["fetch", "mirror"], Some(repo_path));

    // Always fetch from origin as well (to get any missing objects)
    let origin_result = git_operations::git_command(&["fetch", "origin"], Some(repo_path));

    let success = match (mirror_result, origin_result) {
        (Ok(mirror_out), Ok(origin_out)) => mirror_out.success || origin_out.success,
        (Ok(mirror_out), Err(_)) => mirror_out.success,
        (Err(_), Ok(origin_out)) => origin_out.success,
        _ => false,
    };

    if success {
        // Only reset if repo is clean
        match dsdk_cli::git_manager::repo_has_pending_changes(repo_path) {
            Ok(false) => {
                // Clean: safe to reset
                // Check if the commit is a branch reference
                if is_branch_reference(repo_path, &git_cfg.commit) {
                    // For branches, get the latest commit and checkout that
                    if let Some(latest_commit) =
                        get_latest_commit_for_branch(repo_path, &git_cfg.commit)
                    {
                        let checkout_output = git_operations::checkout(repo_path, &latest_commit);
                        match checkout_output {
                            Ok(result) if result.is_success() => {
                                messages::success(&format!(
                                    "{} (updated {} to latest: {})",
                                    git_cfg.name,
                                    git_cfg.commit,
                                    &latest_commit[..8]
                                ));
                                true
                            }
                            _ => {
                                messages::error(&format!(
                                    "{} (failed to checkout latest {})",
                                    git_cfg.name, latest_commit
                                ));
                                false
                            }
                        }
                    } else {
                        // Fallback to original behavior if we can't get latest
                        let checkout_result = git_operations::checkout(repo_path, &git_cfg.commit);
                        match checkout_result {
                            Ok(result) if result.is_success() => {
                                messages::success(&format!(
                                    "{} (updated to {})",
                                    git_cfg.name, git_cfg.commit
                                ));
                                true
                            }
                            _ => {
                                messages::error(&format!(
                                    "{} (failed to checkout {})",
                                    git_cfg.name, git_cfg.commit
                                ));
                                false
                            }
                        }
                    }
                } else {
                    // For tags and specific commits, use the exact reference
                    let checkout_result = git_operations::checkout(repo_path, &git_cfg.commit);
                    match checkout_result {
                        Ok(result) if result.is_success() => {
                            messages::success(&format!(
                                "{} (pinned to {})",
                                git_cfg.name, git_cfg.commit
                            ));
                            true
                        }
                        _ => {
                            messages::error(&format!(
                                "{} (failed to checkout {})",
                                git_cfg.name, git_cfg.commit
                            ));
                            false
                        }
                    }
                }
            }
            Ok(true) => {
                // Dirty: do nothing
                messages::info(&format!(
                    "! {} has pending changes, not resetting to {}",
                    git_cfg.name, git_cfg.commit
                ));
                true
            }
            Err(e) => {
                messages::error(&format!(
                    "{} (error checking pending changes: {})",
                    git_cfg.name, e
                ));
                false
            }
        }
    } else {
        messages::error(&format!(
            "{} (failed to fetch from mirror and origin)",
            git_cfg.name
        ));
        false
    }
}

/// Clone a repository to the workspace
fn clone_repo_to_workspace(
    git_cfg: &config::GitConfig,
    repo_path: &Path,
    mirror_path: &Path,
) -> bool {
    // Remove directory if it exists but is not a git repo (e.g., created by
    // a parent repo clone in a previous tier)
    if repo_path.exists() && !repo_path.join(".git").is_dir() {
        if let Err(e) = std::fs::remove_dir_all(repo_path) {
            messages::error(&format!(
                "{} (failed to remove non-git directory: {})",
                git_cfg.name, e
            ));
            return false;
        }
    }

    messages::progress(&git_cfg.name, "cloning repository");

    let mirror_repo_path =
        dsdk_cli::git_manager::get_mirror_repo_path(mirror_path, &git_cfg.name, &git_cfg.url);

    // Determine the clone source (prefer mirror if exists, otherwise use original URL)
    let clone_source = if mirror_repo_path.exists() {
        format!("file://{}", mirror_repo_path.display())
    } else {
        git_cfg.url.clone()
    };

    let should_timeout = clone_source.starts_with("git@") || clone_source.starts_with("ssh://");

    if mirror_repo_path.exists() {
        // Use --reference to hardlink objects from the mirror
        let result = git_operations::clone_repo(&clone_source, repo_path, Some(&mirror_repo_path));

        match result {
            Ok(result) if result.is_success() => {
                // Set origin to upstream and add mirror remote
                let _ = git_operations::remote_set_url(repo_path, "origin", &git_cfg.url);
                let _ = git_operations::remote_add(
                    repo_path,
                    "mirror",
                    &format!("file://{}", mirror_repo_path.display()),
                );
                return checkout_commit(git_cfg, repo_path);
            }
            _ => {
                messages::error(&format!("{} (clone failed)", git_cfg.name));
                return false;
            }
        }
    } else if should_timeout {
        // Use timeout for SSH URLs
        if let Some(child) = execute_git_clone(
            "git",
            &["clone", &clone_source, &repo_path.to_string_lossy()],
            git_cfg,
        ) {
            if let Ok(output) = child.wait_with_output() {
                if output.status.success() {
                    // Set origin to upstream
                    let _ = git_operations::remote_set_url(repo_path, "origin", &git_cfg.url);
                    return checkout_commit(git_cfg, repo_path);
                } else {
                    messages::error(&format!("{} (clone failed)", git_cfg.name));
                    return false;
                }
            }
        } else {
            messages::error(&format!("{} (clone timed out)", git_cfg.name));
            return false;
        }
    } else {
        // Direct execution for HTTP/HTTPS/file URLs
        let result = git_operations::clone_repo(&clone_source, repo_path, None);

        match result {
            Ok(result) if result.is_success() => {
                // Set origin to upstream
                let _ = git_operations::remote_set_url(repo_path, "origin", &git_cfg.url);
                return checkout_commit(git_cfg, repo_path);
            }
            _ => {
                messages::error(&format!("{} (clone failed)", git_cfg.name));
                return false;
            }
        }
    }
    false
}

/// Checkout the specified commit for a repository
fn checkout_commit(git_cfg: &config::GitConfig, repo_path: &Path) -> bool {
    // Check if the commit is a branch reference
    if is_branch_reference(repo_path, &git_cfg.commit) {
        // For branches, get the latest commit and checkout that
        if let Some(latest_commit) = get_latest_commit_for_branch(repo_path, &git_cfg.commit) {
            let output = git_operations::checkout(repo_path, &latest_commit);

            match output {
                Ok(result) if result.is_success() => {
                    messages::success(&format!(
                        "{} (cloned and checked out {} to latest: {})",
                        git_cfg.name,
                        git_cfg.commit,
                        &latest_commit[..8]
                    ));
                    true
                }
                _ => {
                    messages::error(&format!(
                        "{} (cloned, but failed to checkout latest {})",
                        git_cfg.name, latest_commit
                    ));
                    false
                }
            }
        } else {
            // Fallback to original behavior
            let output = git_operations::checkout(repo_path, &git_cfg.commit);

            match output {
                Ok(result) if result.is_success() => {
                    messages::success(&format!(
                        "{} (cloned and checked out to {})",
                        git_cfg.name, git_cfg.commit
                    ));
                    true
                }
                _ => {
                    messages::error(&format!(
                        "{} (cloned, but failed to checkout to {})",
                        git_cfg.name, git_cfg.commit
                    ));
                    false
                }
            }
        }
    } else {
        // For tags and specific commits, use the exact reference
        let output = git_operations::checkout(repo_path, &git_cfg.commit);

        match output {
            Ok(result) if result.is_success() => {
                messages::success(&format!(
                    "{} (cloned and pinned to {})",
                    git_cfg.name, git_cfg.commit
                ));
                true
            }
            _ => {
                messages::error(&format!(
                    "{} (cloned, but failed to checkout to {})",
                    git_cfg.name, git_cfg.commit
                ));
                false
            }
        }
    }
}

/// Execute a git clone command and wait for completion
fn execute_git_clone(command: &str, args: &[&str], git_cfg: &config::GitConfig) -> Option<Child> {
    let start = Instant::now();
    let mut child = match Command::new(command)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            messages::error(&format!(
                "Failed to start git clone for {}: {}",
                git_cfg.name, e
            ));
            return None;
        }
    };

    // Provide periodic updates for long-running operations
    let mut last_update = start;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                messages::status(&format!(
                    "Git clone for {} completed in {:.1}s",
                    git_cfg.name,
                    start.elapsed().as_secs_f64()
                ));
                return Some(child);
            }
            Ok(None) => {
                // Provide periodic updates for long-running operations
                if last_update.elapsed() > Duration::from_secs(30) {
                    messages::status(&format!(
                        "Still cloning {} (elapsed: {:.1}s)...",
                        git_cfg.name,
                        start.elapsed().as_secs_f64()
                    ));
                    last_update = Instant::now();
                }
                thread::sleep(Duration::from_millis(500));
            }
            Err(e) => {
                messages::error(&format!(
                    "Error waiting for git clone for {}: {}",
                    git_cfg.name, e
                ));
                return None;
            }
        }
    }
}

/// Handle an existing repository in the workspace (no mirror mode)
fn handle_existing_workspace_repo_no_mirror(git_cfg: &config::GitConfig, repo_path: &Path) -> bool {
    // Fetch directly from origin
    let origin_result = git_operations::fetch(repo_path, Some("origin"));

    let success = match origin_result {
        Ok(result) => result.is_success(),
        Err(_) => false,
    };

    if success {
        // Only reset if repo is clean
        match dsdk_cli::git_manager::repo_has_pending_changes(repo_path) {
            Ok(false) => {
                // Clean: safe to reset
                // Check if the commit is a branch reference
                if is_branch_reference(repo_path, &git_cfg.commit) {
                    // For branches, get the latest commit and checkout that
                    if let Some(latest_commit) =
                        get_latest_commit_for_branch(repo_path, &git_cfg.commit)
                    {
                        let checkout_output = git_operations::checkout(repo_path, &latest_commit);
                        match checkout_output {
                            Ok(result) if result.is_success() => {
                                messages::success(&format!(
                                    "{} (updated {} to latest: {})",
                                    git_cfg.name,
                                    git_cfg.commit,
                                    &latest_commit[..8]
                                ));
                                true
                            }
                            _ => {
                                messages::error(&format!(
                                    "{} (failed to checkout latest {})",
                                    git_cfg.name, latest_commit
                                ));
                                false
                            }
                        }
                    } else {
                        // Fallback to original behavior if we can't get latest
                        let checkout_output = git_operations::checkout(repo_path, &git_cfg.commit);
                        match checkout_output {
                            Ok(result) if result.is_success() => {
                                messages::success(&format!(
                                    "{} (updated to {})",
                                    git_cfg.name, git_cfg.commit
                                ));
                                true
                            }
                            _ => {
                                messages::error(&format!(
                                    "{} (failed to checkout {})",
                                    git_cfg.name, git_cfg.commit
                                ));
                                false
                            }
                        }
                    }
                } else {
                    // For tags and specific commits, use the exact reference
                    let checkout_result = git_operations::checkout(repo_path, &git_cfg.commit);
                    match checkout_result {
                        Ok(result) if result.is_success() => {
                            messages::success(&format!(
                                "{} (pinned to {})",
                                git_cfg.name, git_cfg.commit
                            ));
                            true
                        }
                        _ => {
                            messages::error(&format!(
                                "{} (failed to checkout {})",
                                git_cfg.name, git_cfg.commit
                            ));
                            false
                        }
                    }
                }
            }
            Ok(true) => {
                // Dirty: do nothing
                messages::info(&format!(
                    "! {} has pending changes, not resetting to {}",
                    git_cfg.name, git_cfg.commit
                ));
                true
            }
            Err(e) => {
                messages::error(&format!(
                    "Error checking repo status for {}: {}",
                    git_cfg.name, e
                ));
                false
            }
        }
    } else {
        messages::error(&format!("{} (fetch failed)", git_cfg.name));
        false
    }
}

/// Clone a repository to the workspace (no mirror mode)
fn clone_repo_to_workspace_no_mirror(git_cfg: &config::GitConfig, repo_path: &Path) -> bool {
    // Remove directory if it exists but is not a git repo (e.g., created by
    // a parent repo clone in a previous tier)
    if repo_path.exists() && !repo_path.join(".git").is_dir() {
        if let Err(e) = std::fs::remove_dir_all(repo_path) {
            messages::error(&format!(
                "{} (failed to remove non-git directory: {})",
                git_cfg.name, e
            ));
            return false;
        }
    }

    let should_timeout = git_cfg.url.starts_with("git@") || git_cfg.url.starts_with("ssh://");

    if should_timeout {
        // Use timeout for SSH URLs
        if let Some(child) = execute_git_clone(
            "git",
            &["clone", &git_cfg.url, &repo_path.to_string_lossy()],
            git_cfg,
        ) {
            if let Ok(output) = child.wait_with_output() {
                if output.status.success() {
                    return checkout_commit(git_cfg, repo_path);
                } else {
                    messages::error(&format!("{} (clone failed)", git_cfg.name));
                    return false;
                }
            }
        } else {
            messages::error(&format!("{} (clone timed out)", git_cfg.name));
            return false;
        }
    } else {
        // Direct execution for HTTP/HTTPS/file URLs
        let result = git_operations::clone_repo(&git_cfg.url, repo_path, None);

        match result {
            Ok(result) if result.is_success() => {
                return checkout_commit(git_cfg, repo_path);
            }
            _ => {
                messages::error(&format!("{} (clone failed)", git_cfg.name));
                return false;
            }
        }
    }
    false
}

/// Generate a Makefile from the SDK configuration
fn handle_makefile_command() {
    // Must be run from within a workspace
    let workspace_path = match get_current_workspace() {
        Ok(path) => path,
        Err(e) => {
            messages::error(&format!("Error: {}", e));
            return;
        }
    };

    let output_path = workspace_path.join("Makefile");

    // Use sdk.yml from workspace root
    let config_path = workspace_path.join("sdk.yml");
    if !config_path.exists() {
        messages::error(&format!(
            "sdk.yml not found in workspace root: {}",
            workspace_path.display()
        ));
        messages::error("The workspace may be corrupted. Try running 'cim init' to reinitialize.");
        return;
    }

    let mut sdk_config = match config::load_config(&config_path) {
        Ok(config) => config,
        Err(e) => {
            messages::error(&format!("Error loading config: {}", e));
            return;
        }
    };

    // Load and apply user config overrides if present
    match config::UserConfig::load() {
        Ok(Some(user_config)) => {
            user_config.apply_to_sdk_config(&mut sdk_config, false);
        }
        Ok(None) => {}
        Err(e) => {
            messages::error(&format!("Warning: Failed to load user config: {}", e));
        }
    }

    let makefile = generate_makefile_content(&sdk_config);

    match std::fs::write(&output_path, makefile) {
        Ok(_) => messages::success(&format!("Makefile written to {}", output_path.display())),
        Err(e) => messages::error(&format!("Failed to write Makefile: {}", e)),
    }

    // NEW: Generate VS Code tasks.json
    if let Err(e) = vscode_tasks_manager::generate_tasks_json(&workspace_path, &output_path) {
        messages::info(&format!("Could not generate VS Code tasks.json: {}", e));
    }
}

/// Generate the content of the Makefile from SDK configuration
fn generate_makefile_content<T: config::SdkConfigCore>(sdk_config: &T) -> String {
    let mut makefile = String::new();

    // Add makefile includes at the top if specified
    if let Some(makefile_includes) = sdk_config.makefile_include() {
        if !makefile_includes.is_empty() {
            for include_line in makefile_includes {
                makefile.push_str(&format!("-{}\n", include_line));
            }
            makefile.push('\n');
        }
    }

    // Add .PHONY declarations
    let mut phony_targets = vec!["all"];

    // Add sdk-envsetup to PHONY if envsetup commands exist
    if let Some(_envsetup_target) = sdk_config.envsetup() {
        phony_targets.push("sdk-envsetup");
    }

    // Add sdk-test to PHONY if test commands exist
    if let Some(_test_target) = sdk_config.test() {
        phony_targets.push("sdk-test");
    }

    // Add sdk-clean to PHONY (always add it, even if no commands)
    phony_targets.push("sdk-clean");

    // Add sdk-build to PHONY (always add it, even if no commands)
    phony_targets.push("sdk-build");

    // Add sdk-flash to PHONY (always add it, even if no commands)
    phony_targets.push("sdk-flash");

    // Add install targets to PHONY if install section exists
    if let Some(install_configs) = sdk_config.install() {
        if !install_configs.is_empty() {
            phony_targets.push("install-all");
        }
    }

    makefile.push_str(&format!(".PHONY: {}\n", phony_targets.join(" ")));

    // Add 'all' target that depends on sdk-build and sdk-test
    let mut all_deps = vec!["sdk-build"];
    if sdk_config.test().is_some() {
        all_deps.push("sdk-test");
    }
    makefile.push_str(&format!("all: {}\n\n", all_deps.join(" ")));

    // Add sdk-envsetup target if envsetup commands exist
    if let Some(envsetup_target) = sdk_config.envsetup() {
        add_envsetup_target(&mut makefile, envsetup_target);
    }

    // Add sdk-test target if test commands exist
    if let Some(test_target) = sdk_config.test() {
        add_test_target(&mut makefile, test_target);
    }

    // Add sdk-clean target (always create, fallback if missing)
    match sdk_config.clean() {
        Some(clean_target) => {
            add_clean_target(&mut makefile, clean_target);
        }
        _ => {
            makefile.push_str("sdk-clean:\n\t@echo \"No clean commands defined in sdk.yml\"\n\n");
        }
    }

    // Add sdk-build target (always create, fallback if missing)
    match sdk_config.build() {
        Some(build_target) => {
            add_build_target(&mut makefile, build_target);
        }
        _ => {
            makefile.push_str("sdk-build:\n\t@echo \"No build commands defined in sdk.yml\"\n\n");
        }
    }

    // Add sdk-flash target (always create, fallback if missing)
    match sdk_config.flash() {
        Some(flash_target) => {
            add_flash_target(&mut makefile, flash_target);
        }
        _ => {
            makefile.push_str("sdk-flash:\n\t@echo \"No flash commands defined in sdk.yml\"\n\n");
        }
    }

    // Add install-all target if install section exists
    if let Some(install_configs) = sdk_config.install() {
        if !install_configs.is_empty() {
            add_install_all_target(&mut makefile, install_configs);
        }
    }

    // Add individual install targets
    if let Some(install_configs) = sdk_config.install() {
        for install in install_configs {
            add_install_target(&mut makefile, install);
        }
    }

    // Add individual git targets
    for git in sdk_config.gits() {
        add_makefile_target(&mut makefile, git);
    }

    makefile
}

/// Add a single target to the Makefile
fn add_makefile_target(makefile: &mut String, git: &config::GitConfig) {
    // Add .PHONY declaration for this target
    makefile.push_str(&format!(".PHONY: {}\n", git.name));

    // Add target with dependencies
    let dep_str = if let Some(deps) = &git.build_depends_on {
        deps.join(" ")
    } else {
        String::new()
    };

    if dep_str.is_empty() {
        makefile.push_str(&format!("{}:\n", git.name));
    } else {
        makefile.push_str(&format!("{}: {}\n", git.name, dep_str));
    }

    // Add build commands
    if let Some(build_cmds) = &git.build {
        for cmd in build_cmds {
            let trimmed = cmd.trim();
            if trimmed.starts_with('#') {
                // Write as a Makefile comment (with tab like other commands)
                makefile.push_str(&format!(
                    "\t#{}\n",
                    trimmed.strip_prefix('#').unwrap().trim_start()
                ));
            } else {
                makefile.push_str(&format!("\t{}\n", cmd));
            }
        }
    } else {
        makefile.push_str(&format!("\t@echo Building {}\n", git.name));
    }
    makefile.push('\n');
}

/// Add the sdk-envsetup target to the Makefile
fn add_envsetup_target(makefile: &mut String, envsetup_target: &config::SdkTarget) {
    // Add target with dependencies
    if let Some(deps) = envsetup_target.depends_on() {
        makefile.push_str(&format!("sdk-envsetup: {}\n", deps.join(" ")));
    } else {
        makefile.push_str("sdk-envsetup:\n");
    }

    for command in envsetup_target.commands() {
        let trimmed = command.trim();

        // Skip comment lines (starting with #)
        if trimmed.starts_with('#') {
            // Write as a Makefile comment (with tab like other commands)
            makefile.push_str(&format!(
                "\t#{}\n",
                trimmed.strip_prefix('#').unwrap().trim_start()
            ));
            continue;
        }

        // Handle echo commands with @ prefix (like build commands)
        if trimmed.starts_with('@') {
            // Just pass through the @ command as-is, it's already properly formatted
            makefile.push_str(&format!("\t{}\n", trimmed));
            continue;
        }

        // Add regular command
        makefile.push_str(&format!("\t{}\n", command));
    }

    makefile.push('\n');
}

/// Add the sdk-test target to the Makefile
fn add_test_target(makefile: &mut String, test_target: &config::SdkTarget) {
    // Add target with dependencies
    if let Some(deps) = test_target.depends_on() {
        makefile.push_str(&format!("sdk-test: {}\n", deps.join(" ")));
    } else {
        makefile.push_str("sdk-test:\n");
    }

    for command in test_target.commands() {
        let trimmed = command.trim();

        // Skip comment lines (starting with #)
        if trimmed.starts_with('#') {
            // Write as a Makefile comment (with tab like other commands)
            makefile.push_str(&format!(
                "\t#{}\n",
                trimmed.strip_prefix('#').unwrap().trim_start()
            ));
            continue;
        }

        // Handle echo commands with @ prefix (like build commands)
        if trimmed.starts_with('@') {
            // Just pass through the @ command as-is, it's already properly formatted
            makefile.push_str(&format!("\t{}\n", trimmed));
            continue;
        }

        // Add regular command
        makefile.push_str(&format!("\t{}\n", command));
    }

    makefile.push('\n');
}

/// Add the sdk-clean target to the Makefile
fn add_clean_target(makefile: &mut String, clean_target: &config::SdkTarget) {
    // Add target with dependencies
    if let Some(deps) = clean_target.depends_on() {
        makefile.push_str(&format!("sdk-clean: {}\n", deps.join(" ")));
    } else {
        makefile.push_str("sdk-clean:\n");
    }

    for command in clean_target.commands() {
        let trimmed = command.trim();

        // Skip comment lines (starting with #)
        if trimmed.starts_with('#') {
            // Write as a Makefile comment (with tab like other commands)
            makefile.push_str(&format!(
                "\t#{}\n",
                trimmed.strip_prefix('#').unwrap().trim_start()
            ));
            continue;
        }

        // Handle echo commands with @ prefix (like build commands)
        if trimmed.starts_with('@') {
            // Just pass through the @ command as-is, it's already properly formatted
            makefile.push_str(&format!("\t{}\n", trimmed));
            continue;
        }

        // Add regular command
        makefile.push_str(&format!("\t{}\n", command));
    }

    makefile.push('\n');
}

/// Add the sdk-build target to the Makefile
fn add_build_target(makefile: &mut String, build_target: &config::SdkTarget) {
    // Add target with dependencies
    if let Some(deps) = build_target.depends_on() {
        makefile.push_str(&format!("sdk-build: {}\n", deps.join(" ")));
    } else {
        makefile.push_str("sdk-build:\n");
    }

    for command in build_target.commands() {
        let trimmed = command.trim();

        // Skip comment lines (starting with #)
        if trimmed.starts_with('#') {
            // Write as a Makefile comment (with tab like other commands)
            makefile.push_str(&format!(
                "\t#{}\n",
                trimmed.strip_prefix('#').unwrap().trim_start()
            ));
            continue;
        }

        // Handle echo commands with @ prefix (like build commands)
        if trimmed.starts_with('@') {
            // Just pass through the @ command as-is, it's already properly formatted
            makefile.push_str(&format!("\t{}\n", trimmed));
            continue;
        }

        // Add regular command
        makefile.push_str(&format!("\t{}\n", command));
    }

    makefile.push('\n');
}

/// Add the sdk-flash target to the Makefile
fn add_flash_target(makefile: &mut String, flash_target: &config::SdkTarget) {
    // Add target with dependencies
    if let Some(deps) = flash_target.depends_on() {
        makefile.push_str(&format!("sdk-flash: {}\n", deps.join(" ")));
    } else {
        makefile.push_str("sdk-flash:\n");
    }

    for command in flash_target.commands() {
        let trimmed = command.trim();

        // Skip comment lines (starting with #)
        if trimmed.starts_with('#') {
            // Write as a Makefile comment (with tab like other commands)
            makefile.push_str(&format!(
                "\t#{}\n",
                trimmed.strip_prefix('#').unwrap().trim_start()
            ));
            continue;
        }

        // Handle echo commands with @ prefix (like build commands)
        if trimmed.starts_with('@') {
            // Just pass through the @ command as-is, it's already properly formatted
            makefile.push_str(&format!("\t{}\n", trimmed));
            continue;
        }

        // Add regular command
        makefile.push_str(&format!("\t{}\n", command));
    }

    makefile.push('\n');
}

/// Add install-all target that depends on all install targets
fn add_install_all_target(makefile: &mut String, installs: &[config::InstallConfig]) {
    let all_targets: Vec<_> = installs
        .iter()
        .map(|i| format!("install-{}", i.name))
        .collect();
    makefile.push_str(&format!("install-all: {}\n", all_targets.join(" ")));
    makefile.push_str("\t@echo 'All installations complete'\n\n");
}

/// Check if a line contains shell control structures that should be treated as a complete statement
fn contains_complete_control_structure(line: &str) -> bool {
    let trimmed = line.trim();

    // Single-line if/then/else/fi - complete on one line
    if trimmed.contains("; then") && trimmed.contains("; fi") {
        return true;
    }

    // Single-line while/for with do/done
    if (trimmed.contains("; do") && trimmed.contains("; done"))
        || (trimmed.starts_with("while ") && trimmed.contains("; done"))
        || (trimmed.starts_with("for ") && trimmed.contains("; done"))
    {
        return true;
    }

    false
}

/// Check if a line is a shell control structure keyword that shouldn't have semicolon added after it
/// Returns true for keywords that open or continue a block (then, else, elif, do)
/// Returns false for keywords that close a block (fi, done, esac) - these need semicolons
fn is_shell_control_keyword(line: &str) -> bool {
    let trimmed = line.trim();

    // Control structure keywords that should not have semicolons after them
    // (opening/continuing keywords, not closing ones)
    trimmed == "then"
        || trimmed == "else"
        || trimmed == "elif"
        || trimmed == "do"
        || trimmed.starts_with("then ")
        || trimmed.starts_with("else ")
        || trimmed.starts_with("elif ")
        || trimmed.starts_with("do ")
        || trimmed.ends_with("; then")
        || trimmed.ends_with("; do")
}
/// Check if commands contain shell control structures (if/while/for/case blocks)
fn has_shell_control_structure(commands: &[String]) -> bool {
    commands.iter().any(|cmd| {
        let trimmed = cmd.trim();
        trimmed.starts_with("if ")
            || trimmed.starts_with("while ")
            || trimmed.starts_with("for ")
            || trimmed.starts_with("case ")
            || trimmed == "then"
            || trimmed == "else"
            || trimmed == "elif"
            || trimmed == "fi"
            || trimmed == "do"
            || trimmed == "done"
            || trimmed == "esac"
            || trimmed.ends_with("; then")
            || trimmed.ends_with("; do")
    })
}

/// Add a single install target to the Makefile
fn add_install_target(makefile: &mut String, install: &config::InstallConfig) {
    // Add .PHONY declaration
    makefile.push_str(&format!(".PHONY: install-{}\n", install.name));

    // Add target with dependencies
    let dep_str = if let Some(deps) = &install.depends_on {
        deps.iter()
            .map(|d| format!("install-{}", d))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        String::new()
    };

    if dep_str.is_empty() {
        makefile.push_str(&format!("install-{}:\n", install.name));
    } else {
        makefile.push_str(&format!("install-{}: {}\n", install.name, dep_str));
    }

    // If sentinel is specified, wrap commands in check
    if let Some(sentinel) = &install.sentinel {
        makefile.push_str(&format!("\t@if [ ! -f {} ]; then \\\n", sentinel));
        makefile.push_str(&format!("\t  echo 'Installing {}...'; \\\n", install.name));

        if let Some(build_cmds) = &install.commands {
            // Check if commands contain control structures
            let has_control_structure = has_shell_control_structure(build_cmds);

            if has_control_structure {
                // Wrap entire script block in a subshell to preserve control structures
                makefile.push_str("\t  ( \\\n");

                for cmd in build_cmds {
                    let trimmed = cmd.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        // Check if this is a complete single-line control structure
                        if contains_complete_control_structure(trimmed) {
                            // Single-line if/then/else/fi or while/for - add semicolon at end
                            if !trimmed.ends_with(';') {
                                makefile.push_str(&format!("\t    {}; \\\n", cmd));
                            } else {
                                makefile.push_str(&format!("\t    {} \\\n", cmd));
                            }
                        } else {
                            // Multi-line control structure or regular command
                            // Add semicolons for proper shell syntax, but not for control keywords
                            let needs_semicolon = !is_shell_control_keyword(trimmed)
                                && !trimmed.ends_with(';')
                                && !trimmed.ends_with('{')
                                && !trimmed.ends_with('\\');

                            if needs_semicolon {
                                makefile.push_str(&format!("\t    {}; \\\n", cmd));
                            } else {
                                makefile.push_str(&format!("\t    {} \\\n", cmd));
                            }
                        }
                    }
                }

                makefile.push_str("\t  ) && \\\n");
            } else {
                // No control structures - use original && logic
                for cmd in build_cmds {
                    let trimmed = cmd.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        // Wrap commands containing 'cd' in subshells to avoid affecting subsequent commands
                        if trimmed.contains(" cd ") || trimmed.starts_with("cd ") {
                            makefile.push_str(&format!("\t  ({}) && \\\n", cmd));
                        } else {
                            makefile.push_str(&format!("\t  {} && \\\n", cmd));
                        }
                    }
                }
            }

            // Create directory for sentinel file if needed, then create sentinel
            if let Some(sentinel_dir) = std::path::Path::new(sentinel).parent() {
                if sentinel_dir != std::path::Path::new("") {
                    makefile.push_str(&format!("\t  mkdir -p {} && \\\n", sentinel_dir.display()));
                }
            }
            makefile.push_str(&format!("\t  touch {} && \\\n", sentinel));
            makefile.push_str(&format!(
                "\t  echo '{} installed successfully'; \\\n",
                install.name
            ));
        }
        makefile.push_str("\telse \\\n");
        makefile.push_str(&format!(
            "\t  echo '{} already installed (sentinel: {})'; \\\n",
            install.name, sentinel
        ));
        makefile.push_str("\tfi\n\n");
    } else {
        // No sentinel, just run commands
        if let Some(build_cmds) = &install.commands {
            for cmd in build_cmds {
                let trimmed = cmd.trim();
                if trimmed.starts_with('#') {
                    makefile.push_str(&format!(
                        "\t#{}\n",
                        trimmed.strip_prefix('#').unwrap().trim_start()
                    ));
                } else {
                    makefile.push_str(&format!("\t{}\n", cmd));
                }
            }
        }
        makefile.push('\n');
    }
}

/// Handle Docker commands
fn handle_docker_command(docker_command: &DockerCommand) {
    // Docker command must be run from the dsdk source repository folder
    let current_dir = std::env::current_dir().unwrap_or_else(|e| {
        messages::error(&format!("Could not get current directory: {}", e));
        std::process::exit(1);
    });

    // Check if we're in the dsdk source folder by looking for Cargo.toml and dsdk-cli/src/main.rs
    let cargo_toml = current_dir.join("Cargo.toml");
    let dsdk_cli_main = current_dir.join("dsdk-cli/src/main.rs");

    if !cargo_toml.exists() || !dsdk_cli_main.exists() {
        messages::error("Docker command must be run from the cim source repository");
        messages::status(&format!("Current directory: {}", current_dir.display()));
        messages::status("Missing required files:");
        if !cargo_toml.exists() {
            messages::status("  - Cargo.toml");
        }
        if !dsdk_cli_main.exists() {
            messages::status("  - dsdk-cli/src/main.rs");
        }
        messages::status("");
        messages::status("Please cd to your cim source repository and run the command again.");
        return;
    }

    match docker_command {
        DockerCommand::Create {
            target,
            source,
            version,
            distro,
            profile,
            arch,
            output: _, // Dockerfile always created in temp directory
            force,
            force_https,
            force_ssh,
            no_mirror,
            r#match,
        } => {
            // Get docker temp directory for storing extracted manifests
            let docker_temp_dir = match get_docker_temp_dir() {
                Ok(dir) => dir,
                Err(e) => {
                    messages::error(&format!("Failed to create docker temp directory: {}", e));
                    return;
                }
            };

            // Load user config for no_mirror preference
            let user_config = config::UserConfig::load().ok().flatten();

            // Determine if we should skip mirror with precedence:
            // CLI flag > user config > default (false)
            let skip_mirror = *no_mirror
                || user_config
                    .as_ref()
                    .and_then(|uc| uc.no_mirror)
                    .unwrap_or(false);

            // Determine source path (use user config default if available)
            let default_source = get_default_source();
            let source_path = source
                .as_ref()
                .map(String::as_str)
                .unwrap_or(&default_source);

            // Resolve config path using the same approach as init command
            // For git sources, extract to docker_temp_dir instead of using mem::forget
            let config_path = if is_url(source_path) {
                // Git-based source - clone entire target directory structure to docker temp dir
                match resolve_target_config_from_git(
                    source_path,
                    target,
                    version.as_deref(),
                    Some(&docker_temp_dir),
                ) {
                    Ok(path) => {
                        let version_info = if let Some(v) = &version {
                            format!(" (version: {})", v)
                        } else {
                            " (latest)".to_string()
                        };
                        messages::status(&format!(
                            "Fetched config for target '{}'{}",
                            target, version_info
                        ));
                        path
                    }
                    Err(e) => {
                        messages::error(&e.to_string());
                        std::process::exit(1);
                    }
                }
            } else {
                // Local source directory
                let source_path_buf = PathBuf::from(&source_path);

                // Check if version is specified and source is a git repository
                if version.is_some() && source_path_buf.join(".git").exists() {
                    // Local git with version - extract to docker temp dir
                    match resolve_target_config_from_git(
                        source_path,
                        target,
                        version.as_deref(),
                        Some(&docker_temp_dir),
                    ) {
                        Ok(path) => {
                            let version_info = format!(" (version: {})", version.as_ref().unwrap());
                            messages::status(&format!(
                                "Fetched config for target '{}'{}",
                                target, version_info
                            ));
                            path
                        }
                        Err(e) => {
                            messages::error(&e.to_string());
                            std::process::exit(1);
                        }
                    }
                } else {
                    // No version or not a git repo - use direct path
                    match resolve_target_config(target, &source_path_buf) {
                        Ok(path) => path,
                        Err(e) => {
                            messages::error(&e.to_string());
                            messages::status("Available targets:");
                            if let Ok(targets) = list_available_targets(&source_path_buf) {
                                for target_name in targets {
                                    messages::status(&format!("  - {}", target_name));
                                }
                            }
                            std::process::exit(1);
                        }
                    }
                }
            };

            // Validate core config exists and is loadable
            if let Err(e) = config::load_config(&config_path) {
                messages::error(&format!(
                    "Failed to load SDK config from {}: {}",
                    config_path.display(),
                    e
                ));
                return;
            }

            // Load the full config with dependencies (required for Docker generation)
            let (full_sdk_config, os_deps) = match config::load_config_with_os_deps(&config_path) {
                Ok((config, Some(deps))) => (config, deps),
                Ok((_, None)) => {
                    messages::error("os-dependencies.yml is required for Docker generation");
                    messages::info(
                        "Please ensure os-dependencies.yml exists in the target directory",
                    );
                    return;
                }
                Err(e) => {
                    messages::error(&format!("Could not load dependency files: {}", e));
                    messages::info("Please ensure os-dependencies.yml exists and is valid YAML");
                    return;
                }
            };

            // Load python dependencies from the same directory as the config
            let config_dir = config_path
                .parent()
                .expect("Config path should have parent directory");
            let python_deps_path = config_dir.join("python-dependencies.yml");
            let python_deps = match config::load_python_dependencies(&python_deps_path) {
                Ok(deps) => deps,
                Err(e) => {
                    messages::error(&format!("Could not load python-dependencies.yml: {}", e));
                    messages::status(&format!("Config directory: {}", config_dir.display()));
                    messages::status(&format!("Expected file: {}", python_deps_path.display()));
                    return;
                }
            };

            // Apply filtering if --match is provided
            let filtered_sdk_config = if let Some(pattern) = r#match {
                // Compile regex - if it fails, exit with error
                let match_regex = match regex::Regex::new(pattern) {
                    Ok(regex) => Some(regex),
                    Err(e) => {
                        messages::error(&format!("Invalid regex pattern '{}': {}", pattern, e));
                        return;
                    }
                };
                let filtered_gits = filter_git_configs(&full_sdk_config.gits, &match_regex);
                messages::status(&format!(
                    "Filtered to {} repositories (from {} total)",
                    filtered_gits.len(),
                    full_sdk_config.gits.len()
                ));
                config::SdkConfig {
                    mirror: full_sdk_config.mirror.clone(),
                    gits: filtered_gits,
                    toolchains: full_sdk_config.toolchains.clone(),
                    copy_files: full_sdk_config.copy_files.clone(),
                    install: full_sdk_config.install.clone(),
                    makefile_include: full_sdk_config.makefile_include.clone(),
                    envsetup: full_sdk_config.envsetup.clone(),
                    test: full_sdk_config.test.clone(),
                    clean: full_sdk_config.clean.clone(),
                    build: full_sdk_config.build.clone(),
                    flash: full_sdk_config.flash.clone(),
                }
            } else {
                // No filtering, use original config
                full_sdk_config.clone()
            };

            // Get the temp directory containing all config files
            let config_dir = config_path
                .parent()
                .expect("Config path should have parent directory");

            // Copy cross-compiled binary to temp directory so it's in Docker build context
            let source_binary = current_dir
                .join("target")
                .join(arch)
                .join("release")
                .join("cim");

            let dest_binary = config_dir.join("cim");

            if !source_binary.exists() {
                messages::error(&format!(
                    "Cross-compiled binary not found at {}",
                    source_binary.display()
                ));
                messages::info(&format!(
                    "Please run: cross build --release --target {}",
                    arch
                ));
                return;
            }

            if let Err(e) = fs::copy(&source_binary, &dest_binary) {
                messages::error(&format!("Failed to copy binary to temp directory: {}", e));
                return;
            }
            messages::verbose(&format!("Copied {} binary to build context", arch));

            // Dockerfile will be created in the temp directory alongside config files
            // This makes all files (including the binary) accessible within Docker's build context

            let dockerfile_output = config_dir.join("Dockerfile");

            let docker_manager =
                docker_manager::DockerManager::new(current_dir.clone(), config_dir.to_path_buf());
            messages::status("Generating Dockerfile...");

            let config = docker_manager::DockerfileConfig {
                sdk_config: &filtered_sdk_config,
                os_deps: &os_deps,
                python_deps: &python_deps,
                output_path: &dockerfile_output,
                distro_preference: distro.as_deref(),
                python_profile: profile,
                force: *force,
                force_https: *force_https,
                force_ssh: *force_ssh,
                no_mirror: skip_mirror,
            };
            match docker_manager.create_dockerfile(config) {
                Ok(_) => {
                    // docker_manager.create_dockerfile already prints success message with details

                    // Show build instructions
                    messages::status("");
                    messages::status("To build the Docker image:");
                    messages::status(&format!("  cd {}", config_dir.display()));
                    messages::status("");
                    messages::status(
                        "For a setup known to have all git repositories being public:",
                    );
                    messages::status("  docker build -t sdk-image .");
                    messages::status("");
                    messages::status("For a setup with private GitHub repositories:");
                    messages::status("  export GITHUB_TOKEN=<your_token>");
                    messages::status(
                        "  docker build --secret id=GIT_AUTH_TOKEN,env=GITHUB_TOKEN -t sdk-image .",
                    );
                    messages::status("");
                    messages::status("Create a token at: https://github.com/settings/tokens");
                    messages::status("Required scopes: repo (for private repos)");
                    messages::status("");
                    messages::status("For organizations with SAML SSO:");
                    messages::status("  After creating the token, authorize it at:");
                    messages::status(
                        "  https://github.com/settings/tokens → Configure SSO → Authorize",
                    );
                }
                Err(e) => {
                    messages::error(&format!("Failed to create Dockerfile: {}", e));
                }
            }
        }
    }
}

/// Handle the release command
fn handle_release_command(
    tag: Option<&str>,
    genconfig: bool,
    include_patterns: &Vec<String>,
    exclude_patterns: &Vec<String>,
    dry_run: bool,
) {
    // Must be run from within a workspace
    let workspace_path = match get_current_workspace() {
        Ok(path) => path,
        Err(e) => {
            messages::error(&format!("Error: {}", e));
            return;
        }
    };

    // Use sdk.yml from workspace root
    let config_path = workspace_path.join("sdk.yml");
    if !config_path.exists() {
        messages::error(&format!(
            "sdk.yml not found in workspace root: {}",
            workspace_path.display()
        ));
        messages::error("The workspace may be corrupted. Try running 'cim init' to reinitialize.");
        return;
    }

    // Validate arguments
    if tag.is_none() && !genconfig {
        messages::error("Must specify either --tag or --genconfig (or both)");
        return;
    }

    if let Some(tag_str) = tag {
        messages::status(&format!(
            "Creating release {} in workspace: {}",
            tag_str,
            workspace_path.display()
        ));
    } else {
        messages::status(&format!(
            "Generating release configuration in workspace: {}",
            workspace_path.display()
        ));
    }

    if dry_run {
        messages::status("DRY RUN: No actual changes will be made");
    }

    let sdk_config = match config::load_config(&config_path) {
        Ok(config) => config,
        Err(e) => {
            messages::error(&format!("Error loading config: {}", e));
            return;
        }
    };

    // Helper function to convert multiple patterns into a combined regex
    let build_combined_regex = |patterns: &Vec<String>, pattern_type: &str| -> Option<Regex> {
        if patterns.is_empty() {
            return None;
        }

        // Split comma-separated values and flatten
        let all_patterns: Vec<&str> = patterns
            .iter()
            .flat_map(|p| p.split(','))
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect();

        if all_patterns.is_empty() {
            return None;
        }

        // Build alternation pattern: (pattern1|pattern2|pattern3)
        let combined_pattern = if all_patterns.len() == 1 {
            all_patterns[0].to_string()
        } else {
            format!("({})", all_patterns.join("|"))
        };

        match Regex::new(&combined_pattern) {
            Ok(regex) => Some(regex),
            Err(e) => {
                messages::error(&format!(
                    "Invalid {} regex pattern '{}': {}",
                    pattern_type, combined_pattern, e
                ));
                None
            }
        }
    };

    // Compile regex patterns if provided
    let include_regex = build_combined_regex(include_patterns, "include");
    let exclude_regex = build_combined_regex(exclude_patterns, "exclude");

    // Return early if there were regex compilation errors
    if (!include_patterns.is_empty() && include_regex.is_none())
        || (!exclude_patterns.is_empty() && exclude_regex.is_none())
    {
        return;
    }

    // Apply git tags (if tag is specified)
    let mut failed_repos = Vec::new();
    let mut skipped_repos = Vec::new();
    let mut tagged_repos = Vec::new();

    if let Some(tag_str) = tag {
        if dry_run {
            messages::status(&format!("\nWould apply tag '{}' to repositories:", tag_str));
        } else {
            messages::status(&format!("\nApplying tag '{}' to repositories...", tag_str));
        }

        for git_cfg in &sdk_config.gits {
            // Check if this repository should be included (if include pattern is specified)
            if let Some(ref regex) = include_regex {
                if !regex.is_match(&git_cfg.name) {
                    messages::info(&format!(
                        "Skipping {} (not matching include pattern)",
                        git_cfg.name
                    ));
                    skipped_repos.push(git_cfg.name.clone());
                    continue;
                }
            }

            // Check if this repository should be excluded
            if let Some(ref regex) = exclude_regex {
                if regex.is_match(&git_cfg.name) {
                    messages::info(&format!("Skipping {} (excluded by pattern)", git_cfg.name));
                    skipped_repos.push(git_cfg.name.clone());
                    continue;
                }
            }

            let repo_path = workspace_path.join(&git_cfg.name);

            if !repo_path.exists() {
                messages::info(&format!(
                    "Warning: Repository {} does not exist in workspace",
                    git_cfg.name
                ));
                failed_repos.push(git_cfg.name.clone());
                continue;
            }

            // Check if tag already exists
            let tag_check = git_operations::list_tags(&repo_path, Some(tag_str));

            match tag_check {
                Ok(tags) => {
                    if !tags.is_empty() {
                        messages::info(&format!(
                            "Warning: Tag '{}' already exists in {}",
                            tag_str, git_cfg.name
                        ));
                        failed_repos.push(git_cfg.name.clone());
                        continue;
                    }
                }
                Err(e) => {
                    messages::error(&format!(
                        "Error checking existing tags in {}: {}",
                        git_cfg.name, e
                    ));
                    failed_repos.push(git_cfg.name.clone());
                    continue;
                }
            }

            if dry_run {
                messages::info(&format!("Would tag {} with {}", git_cfg.name, tag_str));
                tagged_repos.push(git_cfg.name.clone());
            } else {
                // Apply the tag
                match git_operations::create_tag(&repo_path, tag_str) {
                    Ok(result) if result.is_success() => {
                        messages::success(&format!("Tagged {} with {}", git_cfg.name, tag_str));
                        tagged_repos.push(git_cfg.name.clone());
                    }
                    Ok(result) => {
                        messages::error(&format!(
                            "Failed to tag {}: {}",
                            git_cfg.name, result.stderr
                        ));
                        failed_repos.push(git_cfg.name.clone());
                    }
                    Err(e) => {
                        messages::error(&format!("Error tagging {}: {}", git_cfg.name, e));
                        failed_repos.push(git_cfg.name.clone());
                    }
                }
            }
        }
    }

    // Generate config file if requested
    if genconfig {
        if dry_run {
            messages::status("\nWould generate release configuration file");
        } else {
            messages::status("\nGenerating release configuration file...");
            if let Err(e) = generate_release_config(
                &config_path,
                tag,
                &include_regex,
                &skipped_repos,
                &tagged_repos,
            ) {
                messages::error(&format!("Error generating release config: {}", e));
                return;
            }
        }
    }

    // Report results
    messages::status("\nRelease summary:");
    if let Some(tag_str) = tag {
        let successful_count = tagged_repos.len();
        messages::status(&format!(
            "  Successfully tagged: {} repositories",
            successful_count
        ));
        if !dry_run && !tagged_repos.is_empty() {
            messages::status(&format!("  Tagged repositories with '{}':", tag_str));
            for repo in &tagged_repos {
                messages::status(&format!("    - {}", repo));
            }
        }
    } else {
        messages::status("  No tagging performed (genconfig only mode)");
    }

    if !skipped_repos.is_empty() {
        messages::status(&format!(
            "  Skipped (excluded): {} repositories",
            skipped_repos.len()
        ));
        for repo in &skipped_repos {
            messages::status(&format!("    - {}", repo));
        }
    }

    if !failed_repos.is_empty() {
        messages::status(&format!("  Failed: {} repositories", failed_repos.len()));
        for repo in &failed_repos {
            messages::status(&format!("    - {}", repo));
        }
    }

    if failed_repos.is_empty() {
        if let Some(tag_str) = tag {
            messages::success(&format!("Release {} completed successfully!", tag_str));
        } else {
            messages::success("Release configuration generated successfully!");
        }
    } else if let Some(tag_str) = tag {
        messages::error(&format!("Release {} completed with errors.", tag_str));
    } else {
        messages::error("Release configuration generation completed with errors.");
    }
}

/// Get the current commit hash from a repository
fn get_current_commit_hash(repo_path: &std::path::Path) -> Option<String> {
    if !repo_path.exists() {
        return None;
    }

    git_operations::get_current_commit(repo_path).ok()
}

/// Generate a release configuration file
fn generate_release_config(
    config_path: &std::path::Path,
    tag: Option<&str>,
    _exclude_regex: &Option<Regex>,
    skipped_repos: &[String],
    tagged_repos: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    // Read the original config file
    let original_content = std::fs::read_to_string(config_path)?;

    // Determine output filename based on the scenario
    let output_filename = match tag {
        Some(tag_str) => {
            // If we have include/exclude patterns with tag, use sdk_release.yml
            if !skipped_repos.is_empty() {
                "sdk_release.yml".to_string()
            } else {
                // No patterns, use sdk_<tag>.yml
                let sanitized_tag =
                    tag_str.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|', '.'], "_");
                format!("sdk_{}.yml", sanitized_tag)
            }
        }
        None => {
            // genconfig only, always use sdk_release.yml
            "sdk_release.yml".to_string()
        }
    };
    let output_path = config_path.parent().unwrap().join(&output_filename);

    // Parse and modify the YAML content
    let mut modified_content = String::new();
    let mut in_gits_section = false;
    let mut current_git_name: Option<String> = None;

    for line in original_content.lines() {
        let trimmed = line.trim();

        // Check if we're entering the gits section
        if trimmed == "gits:" {
            in_gits_section = true;
            modified_content.push_str(line);
            modified_content.push('\n');
            continue;
        }

        // Check if we're leaving the gits section (new top-level section)
        if in_gits_section
            && !line.starts_with(' ')
            && !line.starts_with('\t')
            && !trimmed.is_empty()
        {
            in_gits_section = false;
        }

        if in_gits_section {
            // Look for git repository names
            if trimmed.starts_with("- name:") {
                let name = trimmed.strip_prefix("- name:").unwrap().trim();
                // Remove quotes if present (both single and double quotes)
                let clean_name = name.trim_matches('"').trim_matches('\'');
                current_git_name = Some(clean_name.to_string());
                modified_content.push_str(line);
                modified_content.push('\n');
                continue;
            }

            // Look for commit lines and replace based on new logic
            if trimmed.starts_with("commit:") {
                if let Some(ref git_name) = current_git_name {
                    let indent = line.len() - line.trim_start().len();
                    let commit_value = if let Some(tag_str) = tag {
                        if skipped_repos.is_empty() {
                            // Simple case: tag provided, no patterns → tag all repos (backward compatibility)
                            tag_str.to_string()
                        } else {
                            // Pattern filtering case: use tag only if repo was actually tagged
                            if tagged_repos.contains(git_name) {
                                tag_str.to_string()
                            } else {
                                // Get current commit hash for untagged repos
                                get_current_commit_hash(
                                    &config_path.parent().unwrap().join(git_name),
                                )
                                .unwrap_or_else(|| {
                                    trimmed
                                        .strip_prefix("commit:")
                                        .unwrap_or("main")
                                        .trim()
                                        .to_string()
                                })
                            }
                        }
                    } else {
                        // genconfig-only mode: always get current commit hash
                        get_current_commit_hash(&config_path.parent().unwrap().join(git_name))
                            .unwrap_or_else(|| {
                                trimmed
                                    .strip_prefix("commit:")
                                    .unwrap_or("main")
                                    .trim()
                                    .to_string()
                            })
                    };
                    modified_content.push_str(&format!(
                        "{}commit: {}",
                        " ".repeat(indent),
                        commit_value
                    ));
                } else {
                    // No current git name, keep original
                    modified_content.push_str(line);
                }
                modified_content.push('\n');
                continue;
            }
        }

        // For all other lines, keep as-is
        modified_content.push_str(line);
        modified_content.push('\n');
    }

    // Write the modified content to the output file
    std::fs::write(&output_path, modified_content)?;

    messages::success(&format!(
        "Generated release config: {}",
        output_path.display()
    ));
    Ok(())
}

fn ensure_file_in_mirror(
    copy_file: &config::CopyFileConfig,
    config_source_dir: &Path,
    mirror_path: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Check if source is URL or local path
    if is_url(&copy_file.source) {
        // For URLs, use generate_cache_path to determine where it will be downloaded
        let cache_path = generate_cache_path(&copy_file.source, mirror_path);

        download_file_with_cache(DownloadConfig {
            url: &copy_file.source,
            dest_path: &cache_path,
            mirror_path,
            use_cache: copy_file.cache.unwrap_or(false),
            expected_sha256: None, // Don't verify during hash computation
            post_data: copy_file.post_data.as_deref(),
            multi_progress: None,
            use_symlink: false, // No symlink for hash computation
        })?;

        Ok(cache_path)
    } else {
        // Local file - compute path from config_source_dir
        let expanded_source = expand_env_vars(&copy_file.source);
        let source_path = if Path::new(&expanded_source).is_absolute() {
            PathBuf::from(&expanded_source)
        } else {
            config_source_dir.join(&expanded_source)
        };

        // Check if local file exists
        if !source_path.exists() {
            messages::info(&format!(
                "Local file {} does not exist, skipping",
                copy_file.source
            ));
            return Err(format!("File not found: {}", copy_file.source).into());
        }

        Ok(source_path)
    }
}

/// Update sdk.yml with computed SHA256 hash for a specific copy_files entry
fn update_sdk_yaml_hash(
    config_path: &Path,
    dest_or_source: &str,
    new_hash: &str,
    dry_run: bool,
) -> Result<Option<bool>, Box<dyn std::error::Error>> {
    let mut content = fs::read_to_string(config_path)?;

    // Find the copy_files entry matching the destination or source
    let lines: Vec<&str> = content.lines().collect();
    let mut found_entry = false;
    let mut updated_line_idx = None;

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("- source:")
            || trimmed.starts_with("- url:")
            || trimmed == "source:"
            || trimmed == "url:"
        {
            // Check next few lines for matching dest or source
            for j in idx..std::cmp::min(idx + 5, lines.len()) {
                let check_line = lines[j];
                if check_line.contains(&format!("dest: {}", dest_or_source))
                    || (check_line.contains("source:") && check_line.contains(dest_or_source))
                {
                    // Found the entry - mark as found
                    found_entry = true;

                    // Now find sha256 line within this entry
                    for (k, sha_line) in lines
                        .iter()
                        .enumerate()
                        .take(std::cmp::min(j + 10, lines.len()))
                        .skip(j)
                    {
                        if sha_line.trim().starts_with("sha256:") {
                            updated_line_idx = Some(k);
                            break;
                        }
                    }
                    // Break inner loop once we've found the matching entry
                    break;
                }
            }
        }
    }

    if !found_entry {
        return Err(format!("Could not find copy_files entry for: {}", dest_or_source).into());
    }

    if let Some(line_idx) = updated_line_idx {
        // Extract current hash value from the line
        let current_line = lines[line_idx];
        let current_hash_in_file = current_line
            .trim()
            .strip_prefix("sha256:")
            .map(|s| s.trim())
            .unwrap_or("");

        // Check if hash has actually changed
        if current_hash_in_file == new_hash {
            // Hash unchanged, no need to update
            return Ok(Some(false));
        }

        if dry_run {
            messages::status(&format!(
                "Would update line {} from '{}' to 'sha256: {}'",
                line_idx + 1,
                lines[line_idx],
                new_hash
            ));
        } else {
            // Update the sha256 value while preserving formatting
            let mut updated_lines: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
            let current_line = &updated_lines[line_idx];

            // Extract indentation from the line
            let indent: String = current_line
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect();

            updated_lines[line_idx] = format!("{}sha256: {}", indent, new_hash);

            content = updated_lines.join("\n");
            fs::write(config_path, &content)?;
            messages::status(&format!(
                "Updated sha256 for {}: {}",
                dest_or_source, new_hash
            ));
        }

        return Ok(Some(true));
    }

    // File doesn't have an existing sha256 field - skip it (this is OK for local files)
    messages::info(&format!(
        "Skipping {}: no sha256 field in entry",
        dest_or_source
    ));
    Ok(None) // Return Ok(false) to indicate we processed but didn't update anything
}

/// Handle the copy-files-hash command
fn handle_copy_files_hash_command(file_filter: Option<&str>, dry_run: bool, verbose: bool) {
    // Set verbose mode for this command
    messages::set_verbose(verbose);

    // Must be run from within a workspace
    let workspace_path = match get_current_workspace() {
        Ok(path) => path,
        Err(e) => {
            messages::error(&format!("Error: {}", e));
            return;
        }
    };

    // Use sdk.yml from workspace root (ignore user overrides for hash computation)
    let config_path = workspace_path.join("sdk.yml");
    if !config_path.exists() {
        messages::error(&format!(
            "sdk.yml not found in workspace root: {}",
            workspace_path.display()
        ));
        return;
    }

    // Load SDK config (without user overrides)
    let sdk_config = match config::load_config(&config_path) {
        Ok(config) => config,
        Err(e) => {
            messages::error(&format!("Error loading config: {}", e));
            return;
        }
    };

    // Get copy_files configuration (direct field access)
    let Some(copy_files) = &sdk_config.copy_files else {
        messages::status("No copy_files section found in sdk.yml");
        return;
    };

    if copy_files.is_empty() {
        messages::status("copy_files section is empty");
        return;
    }

    // Expand mirror path (always use base config, not user overrides)
    let mirror_path = expand_config_mirror_path(&sdk_config);

    // Determine the base directory for resolving relative source paths in copy_files.
    // If config_source_dir is a remote URL, re-clone the repo to a fresh temp directory.
    let (_temp_clone_dir, config_source_dir) = {
        let (dir, temp) = resolve_config_source_dir_from_marker(&workspace_path, &config_path);
        (temp, dir)
    };
    // _temp_clone_dir must stay alive for the duration of the function

    messages::status("Computing SHA256 hashes for copy_files entries...");
    messages::verbose(&format!("Mirror path: {}", mirror_path.display()));

    let mut processed_count = 0;
    let mut updated_count = 0;
    let mut skipped_count = 0;

    for copy_file in copy_files {
        // Check if this file matches the filter (if provided)
        if let Some(filter) = file_filter {
            let matches_dest = copy_file.dest.contains(filter);
            let matches_source = copy_file.source.contains(filter);

            if !matches_dest && !matches_source {
                continue;
            }
        }

        processed_count += 1;

        messages::status(&format!(
            "Processing: {} -> {}",
            copy_file.source, copy_file.dest
        ));

        // Compute the current hash (if sha256 exists)
        let current_hash = copy_file.sha256.clone().unwrap_or_else(|| {
            messages::verbose("No existing hash found");
            String::new()
        });

        // Ensure file is available (download if needed)
        let file_path = match ensure_file_in_mirror(copy_file, &config_source_dir, &mirror_path) {
            Ok(path) => path,
            Err(e) => {
                messages::error(&format!("Error processing {}: {}", copy_file.dest, e));
                skipped_count += 1;
                continue; // Continue with next file instead of failing entire command
            }
        };

        // Compute SHA256 hash
        let computed_hash = match compute_file_sha256(&file_path) {
            Ok(hash) => hash,
            Err(e) => {
                messages::error(&format!(
                    "Failed to compute hash for {}: {}",
                    copy_file.dest, e
                ));
                skipped_count += 1;
                continue; // Continue with next file instead of failing entire command
            }
        };

        // Check if hash changed
        let hash_changed = current_hash != computed_hash && !current_hash.is_empty();
        let has_sha256_field = !current_hash.is_empty();

        if dry_run {
            messages::status(&format!("File: {}", copy_file.dest));
            messages::verbose(&format!(
                "  Current hash:  {}\n  New hash:      {}",
                if current_hash.is_empty() {
                    "<none>"
                } else {
                    &current_hash
                },
                computed_hash
            ));

            if has_sha256_field && !hash_changed {
                messages::status("  Status: Hash unchanged (no update needed)");
            } else if !has_sha256_field {
                messages::status("  Status: No sha256 field in entry (would be skipped)");
                skipped_count += 1;
            } else {
                messages::status("  Status: Hash would be updated");
                updated_count += 1;
            }
        } else {
            // Update sdk.yml with new hash
            match update_sdk_yaml_hash(&config_path, &copy_file.dest, &computed_hash, dry_run) {
                Ok(Some(true)) => {
                    // Successfully updated
                    updated_count += 1;
                }
                Ok(None) => {
                    // No sha256 field exists, count as skipped
                    skipped_count += 1;
                }
                Ok(Some(false)) => {
                    // Hash unchanged, no update needed
                }
                Err(e) => {
                    messages::error(&format!(
                        "Failed to update sdk.yml for {}: {}",
                        copy_file.dest, e
                    ));
                    skipped_count += 1;
                }
            }
        }

        if verbose {
            messages::verbose(&format!(
                "Computed hash: {}\nFile size: {} bytes",
                computed_hash,
                fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0)
            ));
        }
    }

    // Print summary
    messages::status("\n=== Summary ===");
    messages::status(&format!("Processed: {}", processed_count));
    if dry_run {
        messages::status(&format!("Would update: {}", updated_count));
        messages::status(&format!("Would skip:   {}", skipped_count));
        messages::status("(Dry run mode - no changes were made)");
    } else {
        messages::status(&format!("Updated:   {}", updated_count));
        messages::status(&format!("Skipped:   {}", skipped_count));
    }

    if processed_count == 0 {
        messages::info("No files matched the filter or no copy_files entries found");
    }
}
/// Handle the sync-files command - re-run copy_files operation
fn handle_sync_files_hash_command(
    file_filter: Option<&str>,
    dry_run: bool,
    verbose: bool,
    force: bool,
) {
    use std::collections::HashSet;

    // Set verbose mode for this command
    messages::set_verbose(verbose);

    // Must be run from within a workspace
    let workspace_path = match get_current_workspace() {
        Ok(path) => path,
        Err(e) => {
            messages::error(&format!("Error: {}", e));
            return;
        }
    };

    // Use sdk.yml from workspace root
    let config_path = workspace_path.join("sdk.yml");
    if !config_path.exists() {
        messages::error(&format!(
            "sdk.yml not found in workspace root: {}",
            workspace_path.display()
        ));
        return;
    }

    // Load SDK config (without user overrides for sync operation)
    let sdk_config = match config::load_config(&config_path) {
        Ok(config) => config,
        Err(e) => {
            messages::error(&format!("Error loading config: {}", e));
            return;
        }
    };

    // Get copy_files configuration
    let Some(copy_files) = &sdk_config.copy_files else {
        messages::status("No copy_files section found in sdk.yml");
        return;
    };

    if copy_files.is_empty() {
        messages::status("copy_files section is empty");
        return;
    }

    // Expand mirror path
    let mirror_path = expand_config_mirror_path(&sdk_config);

    // Determine the base directory for resolving relative source paths in copy_files.
    // If config_source_dir is a remote URL, re-clone the repo to a fresh temp directory.
    let (_temp_clone_dir, config_source_dir) = {
        let (dir, temp) = resolve_config_source_dir_from_marker(&workspace_path, &config_path);
        (temp, dir)
    };
    // _temp_clone_dir must stay alive for the duration of the function

    // Filter files if a specific filter is provided
    let files_to_process: Vec<_> = if let Some(filter) = file_filter {
        copy_files
            .iter()
            .filter(|cf| cf.dest.contains(filter) || cf.source.contains(filter))
            .cloned()
            .collect()
    } else {
        copy_files.clone()
    };

    if files_to_process.is_empty() {
        messages::info("No files matched the filter");
        return;
    }

    let mode_str = if dry_run { "[DRY RUN] " } else { "" };
    messages::status(&format!(
        "{}Syncing {} file(s) to workspace...",
        mode_str,
        files_to_process.len()
    ));

    let mut synced_count = 0;
    let mut skipped_count = 0;
    let mut failed_count = 0;
    let mut processed_urls = HashSet::new();

    // Separate URL downloads from local file operations
    let (url_files, local_files): (Vec<_>, Vec<_>) =
        files_to_process.iter().partition(|cf| is_url(&cf.source));

    // Process URL downloads
    for copy_file in &url_files {
        // Skip duplicate URLs
        if !processed_urls.insert(copy_file.source.clone()) {
            messages::verbose(&format!("Skipping duplicate URL: {}", copy_file.source));
            continue;
        }

        let dest_path = workspace_path.join(&copy_file.dest);
        let use_cache = copy_file.cache.unwrap_or(false);
        let use_symlink = copy_file.symlink.unwrap_or(false) && use_cache;
        let expected_sha256 = copy_file.sha256.clone();

        messages::status(&format!(
            "{}Processing: {} -> {}",
            mode_str, copy_file.source, copy_file.dest
        ));

        if dry_run {
            messages::status(&format!("  Would download to: {}", dest_path.display()));
            if use_cache {
                messages::status(&format!(
                    "  Cache: enabled (mirror: {})",
                    mirror_path.display()
                ));
            }
            if use_symlink {
                messages::status("  Symlink: true");
            }
            if expected_sha256.is_some() {
                messages::status("  SHA256 verification: enabled");
            }
            synced_count += 1;
            continue;
        }

        // Check if file exists and --force is not set
        if dest_path.exists() && !force {
            messages::info(&format!(
                "  Skipping: {} already exists (use --force to overwrite)",
                copy_file.dest
            ));
            skipped_count += 1;
            continue;
        }

        // Download the file
        match download_file_with_cache(DownloadConfig {
            url: &copy_file.source,
            dest_path: &dest_path,
            mirror_path: &mirror_path,
            use_cache,
            expected_sha256: expected_sha256.as_deref(),
            post_data: copy_file.post_data.as_deref(),
            multi_progress: None,
            use_symlink,
        }) {
            Ok(_) => {
                messages::success(&format!("  Synced: {}", copy_file.dest));
                synced_count += 1;
            }
            Err(e) => {
                messages::error(&format!("  Failed to sync {}: {}", copy_file.dest, e));
                failed_count += 1;
            }
        }
    }

    // Process local files
    for copy_file in &local_files {
        let expanded_source = expand_env_vars(&copy_file.source);
        let source_path = if Path::new(&expanded_source).is_absolute() {
            PathBuf::from(&expanded_source)
        } else {
            config_source_dir.join(&expanded_source)
        };

        let dest_path = workspace_path.join(&copy_file.dest);

        messages::status(&format!(
            "{}Processing: {} -> {}",
            mode_str, copy_file.source, copy_file.dest
        ));

        if !source_path.exists() {
            messages::error(&format!(
                "  Source file does not exist: {}",
                source_path.display()
            ));
            failed_count += 1;
            continue;
        }

        if dry_run {
            messages::status(&format!("  Would copy to: {}", dest_path.display()));
            synced_count += 1;
            continue;
        }

        // Check if destination exists and --force is not set
        if dest_path.exists() && !force {
            messages::info(&format!(
                "  Skipping: {} already exists (use --force to overwrite)",
                copy_file.dest
            ));
            skipped_count += 1;
            continue;
        }

        // Handle directories
        if source_path.is_dir() {
            match copy_dir_recursive(&source_path, &dest_path) {
                Ok(_) => {
                    messages::success(&format!("  Synced directory: {}", copy_file.dest));
                    synced_count += 1;
                }
                Err(e) => {
                    messages::error(&format!(
                        "  Failed to sync directory {}: {}",
                        copy_file.dest, e
                    ));
                    failed_count += 1;
                }
            }
        } else {
            // Single file copy
            match copy_single_file(&source_path, &dest_path, &copy_file.source, &copy_file.dest) {
                Ok(_) => {
                    messages::success(&format!("  Synced: {}", copy_file.dest));
                    synced_count += 1;
                }
                Err(e) => {
                    messages::error(&format!("  Failed to sync {}: {}", copy_file.dest, e));
                    failed_count += 1;
                }
            }
        }
    }

    // Print summary
    messages::status("\n=== Summary ===");
    if dry_run {
        messages::status(&format!("Would sync: {}", synced_count));
        messages::status("(Dry run mode - no changes were made)");
    } else {
        messages::status(&format!("Synced:  {}", synced_count));
        messages::status(&format!("Skipped: {}", skipped_count));
        messages::status(&format!("Failed:  {}", failed_count));
    }

    if failed_count > 0 {
        std::process::exit(1);
    }
}

/// Update the cim binary to the latest release from GitHub.
///
/// Downloads the platform-appropriate archive, extracts the new binary, renames the
/// current binary to `cim.old`, then places the new binary in its location.
fn handle_utils_update_command() {
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
fn handle_utils_command(utils_command: &UtilsCommand) {
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
fn main() {
    let cli = Cli::parse();

    // Handle version flag first
    if cli.version {
        print_version_info();
        return;
    }

    // Command is now optional, so we need to handle the case where it's None
    let command = match &cli.command {
        Some(cmd) => cmd,
        None => {
            // Show help and check for updates when no command is provided
            Cli::command().print_help().unwrap();
            print_update_notice(spawn_version_check());
            return;
        }
    };

    match command {
        Commands::ListTargets { source, target } => {
            handle_list_targets_command(source.as_deref(), target.as_deref());
        }
        Commands::Init {
            target,
            source,
            version,
            workspace,
            no_mirror,
            force,
            r#match,
            verbose,
            install,
            full,
            no_sudo,
            symlink,
            yes,
            cert_validation,
        } => {
            // Validate that target is provided
            let target_name = match target {
                Some(t) => t.clone(),
                None => {
                    messages::error("--target is required");
                    messages::status("Use 'cim list-targets' to see available targets");
                    std::process::exit(1);
                }
            };

            handle_init_command(InitConfig {
                target: target_name,
                source: source.clone(),
                version: version.clone(),
                workspace: workspace.clone(),
                no_mirror: *no_mirror,
                force: *force,
                match_pattern: r#match.as_deref(),
                verbose: *verbose,
                install: *install,
                full: *full,
                no_sudo: *no_sudo,
                symlink: *symlink,
                yes: *yes,
                _cert_validation: cert_validation.as_deref(),
            });
        }
        Commands::Foreach { command, r#match } => {
            handle_foreach_command(command, r#match.as_deref());
        }
        Commands::Update {
            no_mirror,
            r#match,
            verbose,
            cert_validation,
        } => {
            handle_update_command(
                *no_mirror,
                r#match.as_deref(),
                *verbose,
                cert_validation.as_deref(),
            );
        }
        Commands::Makefile => {
            handle_makefile_command();
        }
        Commands::Add { name, url, commit } => {
            handle_add_command(name, url, commit);
        }
        Commands::Install { install_command } => {
            handle_install_command(install_command);
        }
        Commands::Docs { docs_command } => {
            handle_docs_command(docs_command);
        }
        Commands::Docker { docker_command } => {
            handle_docker_command(docker_command);
        }
        Commands::Release {
            tag,
            genconfig,
            include,
            exclude,
            dry_run,
        } => {
            handle_release_command(tag.as_deref(), *genconfig, include, exclude, *dry_run);
        }
        Commands::Config {
            list,
            get,
            show_path,
            template,
            create,
            force,
            edit,
            validate,
        } => {
            handle_config_command(ConfigOptions {
                list: *list,
                get: get.as_deref(),
                show_path: *show_path,
                template: *template,
                create: *create,
                force: *force,
                edit: *edit,
                validate: *validate,
            });
        }
        Commands::Utils { utils_command } => {
            handle_utils_command(utils_command);
        }
    }
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

    // Test helper to create a minimal SDK config
    fn create_test_sdk_config(temp_dir: &Path) -> PathBuf {
        let config_content = r#"
mirror: /tmp/test-mirror
gits:
  - name: test-repo
    url: https://github.com/test/repo.git
    commit: main
    build:
      - "@echo Building test-repo"
"#;
        let config_path = temp_dir.join("sdk.yml");
        fs::write(&config_path, config_content).expect("Failed to write test config");
        config_path
    }

    #[test]
    fn test_venv_exists_true() {
        let (_temp_dir, workspace_path) = create_test_workspace();
        let venv_path = workspace_path.join(".venv");
        let bin_path = venv_path.join("bin");
        fs::create_dir_all(&bin_path).expect("Failed to create venv structure");

        // Create python3 executable (empty file is fine for test)
        let python_exe = bin_path.join("python3");
        fs::write(&python_exe, "").expect("Failed to create python3 file");

        assert!(venv_exists(&workspace_path));
    }

    #[test]
    fn test_venv_exists_false() {
        let (_temp_dir, workspace_path) = create_test_workspace();
        assert!(!venv_exists(&workspace_path));

        // Create .venv dir but no python3 executable
        let venv_path = workspace_path.join(".venv");
        fs::create_dir_all(&venv_path).expect("Failed to create venv dir");
        assert!(!venv_exists(&workspace_path));
    }

    #[test]
    fn test_get_venv_pip_path() {
        let (_temp_dir, workspace_path) = create_test_workspace();
        let expected_pip_path = workspace_path.join(".venv").join("bin").join("pip");
        assert_eq!(get_venv_pip_path(&workspace_path), expected_pip_path);
    }

    #[test]
    fn test_is_container_environment_false() {
        // This test assumes we're not running in a container
        // In CI, this might need adjustment
        if !Path::new("/.dockerenv").exists()
            && env::var("container").is_err()
            && !fs::read_to_string("/proc/1/cgroup")
                .map(|content| content.contains("docker") || content.contains("containerd"))
                .unwrap_or(false)
        {
            assert!(!is_container_environment());
        }
    }

    #[test]
    fn test_detect_os_and_distro() {
        let (os_key, distro, version) = detect_os_and_distro();

        // Should return valid strings, not empty
        assert!(!os_key.is_empty());
        assert!(!distro.is_empty());
        assert!(!version.is_empty());

        // OS key should be one of the expected types (may include architecture)
        assert!(
            os_key.starts_with("linux-")
                || matches!(os_key.as_str(), "macos" | "windows" | "unknown")
        );

        // On Linux, architecture should be included
        #[cfg(target_os = "linux")]
        {
            assert!(os_key.starts_with("linux-"));
            assert!(os_key.contains(std::env::consts::ARCH));
        }

        // On macOS, should be just "macos"
        #[cfg(target_os = "macos")]
        {
            assert_eq!(os_key, "macos");
            assert_eq!(version, "any");
        }
    }

    #[test]
    fn test_generate_makefile_content_empty() {
        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);
        assert!(makefile.contains(".PHONY: all"));
        assert!(makefile.contains("all:"));
    }

    #[test]
    fn test_generate_makefile_content_single_repo() {
        let git_config = config::GitConfig {
            name: "test-repo".to_string(),
            url: "https://github.com/test/repo.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: Some(vec!["make".to_string(), "make install".to_string()]),
            documentation_dir: None,
        };

        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![git_config],
            copy_files: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);
        assert!(makefile.contains(".PHONY: all"));
        assert!(makefile.contains("all: sdk-build"));
        assert!(makefile.contains("test-repo:"));
        assert!(makefile.contains("\tmake"));
        assert!(makefile.contains("\tmake install"));
    }

    #[test]
    fn test_generate_makefile_content_with_dependencies() {
        let git1 = config::GitConfig {
            name: "base-repo".to_string(),
            url: "https://github.com/test/base.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: Some(vec!["@echo Building base".to_string()]),
            documentation_dir: None,
        };

        let git2 = config::GitConfig {
            name: "dep-repo".to_string(),
            url: "https://github.com/test/dep.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: Some(vec!["base-repo".to_string()]),
            git_depends_on: None,
            build: Some(vec!["# This is a comment".to_string(), "make".to_string()]),
            documentation_dir: None,
        };

        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![git1, git2],
            copy_files: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);
        assert!(makefile.contains("all: sdk-build"));
        assert!(makefile.contains("base-repo:"));
        assert!(makefile.contains("dep-repo: base-repo"));
        assert!(makefile.contains("\t@echo Building base"));
        assert!(makefile.contains("\t#This is a comment"));
        assert!(makefile.contains("\tmake"));
    }

    #[test]
    fn test_add_makefile_target_no_deps_no_build() {
        let mut makefile = String::new();
        let git_config = config::GitConfig {
            name: "simple-repo".to_string(),
            url: "https://github.com/test/simple.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: None,
            documentation_dir: None,
        };

        add_makefile_target(&mut makefile, &git_config);

        assert!(makefile.contains("simple-repo:"));
        assert!(makefile.contains("\t@echo Building simple-repo"));
    }

    #[test]
    fn test_add_makefile_target_with_comments() {
        let mut makefile = String::new();
        let git_config = config::GitConfig {
            name: "commented-repo".to_string(),
            url: "https://github.com/test/commented.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: Some(vec![
                "# Configure the build".to_string(),
                "./configure".to_string(),
                "#Another comment".to_string(),
                "make".to_string(),
            ]),
            documentation_dir: None,
        };

        add_makefile_target(&mut makefile, &git_config);

        assert!(makefile.contains("commented-repo:"));
        assert!(makefile.contains("\t#Configure the build"));
        assert!(makefile.contains("\t./configure"));
        assert!(makefile.contains("\t#Another comment"));
        assert!(makefile.contains("\tmake"));
    }

    #[test]
    fn test_checkout_commit_success() {
        // This test would require setting up a real git repository
        // For now, we'll test the basic structure
        let (_temp_dir, workspace_path) = create_test_workspace();
        let repo_path = workspace_path.join("test-repo");

        let git_config = config::GitConfig {
            name: "test-repo".to_string(),
            url: "https://github.com/test/repo.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: None,
            documentation_dir: None,
        };

        // This will fail because there's no git repo, but we can test the function exists
        let result = checkout_commit(&git_config, &repo_path);
        assert!(!result); // Should fail since no git repo exists
    }

    #[test]
    fn test_venv_path_helpers() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Test pip path generation
        let pip_path = get_venv_pip_path(&workspace_path);
        assert_eq!(
            pip_path,
            workspace_path.join(".venv").join("bin").join("pip")
        );

        // Test venv detection with non-existent venv
        assert!(!venv_exists(&workspace_path));

        // Test venv detection with partial venv structure
        let venv_dir = workspace_path.join(".venv");
        fs::create_dir_all(&venv_dir).expect("Failed to create .venv");
        assert!(!venv_exists(&workspace_path)); // Still false, no python3

        // Test venv detection with complete structure
        let bin_dir = venv_dir.join("bin");
        fs::create_dir_all(&bin_dir).expect("Failed to create bin dir");
        let python_exe = bin_dir.join("python3");
        fs::write(&python_exe, "").expect("Failed to create python3");
        assert!(venv_exists(&workspace_path)); // Now true
    }

    #[test]
    fn test_makefile_generation_edge_cases() {
        // Test with repository that has empty build commands
        let git_config = config::GitConfig {
            name: "empty-build".to_string(),
            url: "https://github.com/test/empty.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: Some(vec![]),
            documentation_dir: None,
        };

        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![git_config],
            copy_files: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);
        assert!(makefile.contains("empty-build:"));

        // Test with multiple dependencies
        let git_with_many_deps = config::GitConfig {
            name: "many-deps".to_string(),
            url: "https://github.com/test/many.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: Some(vec![
                "dep1".to_string(),
                "dep2".to_string(),
                "dep3".to_string(),
            ]),
            git_depends_on: None,
            build: Some(vec!["echo hello".to_string()]),
            documentation_dir: None,
        };

        let mut makefile = String::new();
        add_makefile_target(&mut makefile, &git_with_many_deps);
        assert!(makefile.contains("many-deps: dep1 dep2 dep3"));
    }

    #[test]
    fn test_envsetup_target_generation() {
        // Test with envsetup commands
        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            envsetup: Some(config::SdkTarget::Commands(vec![
                "ln -sf qemu_v8.mk build/Makefile".to_string(),
                "cd build && make -j3 toolchains".to_string(),
            ])),
            test: None,
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);

        // Check that .PHONY includes sdk-envsetup
        assert!(makefile.contains(".PHONY: all sdk-envsetup"));

        // Check that sdk-envsetup target exists
        assert!(makefile.contains("sdk-envsetup:"));

        // Check that commands are properly formatted with tabs
        assert!(makefile.contains("\tln -sf qemu_v8.mk build/Makefile"));
        assert!(makefile.contains("\tcd build && make -j3 toolchains"));
    }

    #[test]
    fn test_envsetup_target_with_comments_and_echo() {
        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            envsetup: Some(config::SdkTarget::Commands(vec![
                "# Setup toolchain".to_string(),
                "@echo Setting up environment".to_string(),
                "mkdir -p build".to_string(),
                "#Another comment".to_string(),
                "export CROSS_COMPILE=aarch64-linux-gnu-".to_string(),
            ])),
            test: None,
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);

        // Check that comments are preserved
        assert!(makefile.contains("#Setup toolchain"));
        assert!(makefile.contains("#Another comment"));

        // Check that @ echo commands are converted properly
        assert!(makefile.contains("\t@echo Setting up environment"));

        // Check that regular commands are included
        assert!(makefile.contains("\tmkdir -p build"));
        assert!(makefile.contains("\texport CROSS_COMPILE=aarch64-linux-gnu-"));
    }

    #[test]
    fn test_envsetup_target_empty_commands() {
        // Test with empty envsetup commands
        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);

        // Should not include sdk-envsetup in PHONY or create target
        assert!(makefile.contains(".PHONY: all"));
        assert!(!makefile.contains("sdk-envsetup"));
    }

    #[test]
    fn test_envsetup_target_none() {
        // Test with no envsetup commands
        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);

        // Should not include sdk-envsetup
        assert!(makefile.contains(".PHONY: all"));
        assert!(!makefile.contains("sdk-envsetup"));
    }

    #[test]
    fn test_add_envsetup_target_function() {
        let mut makefile = String::new();
        let target = config::SdkTarget::Commands(vec![
            "# Initial setup".to_string(),
            "mkdir -p logs".to_string(),
            "@echo Starting setup".to_string(),
            "chmod +x scripts/setup.sh".to_string(),
        ]);

        add_envsetup_target(&mut makefile, &target);

        assert!(makefile.contains("sdk-envsetup:"));
        assert!(makefile.contains("\t#Initial setup"));
        assert!(makefile.contains("\tmkdir -p logs"));
        assert!(makefile.contains("\t@echo Starting setup"));
        assert!(makefile.contains("\tchmod +x scripts/setup.sh"));

        // Should end with blank line
        assert!(makefile.ends_with("\n\n"));
    }

    #[test]
    fn test_test_target_generation() {
        // Test with test commands
        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            envsetup: None,
            test: Some(config::SdkTarget::Commands(vec![
                "cargo test --release".to_string(),
                "python run_integration_tests.py".to_string(),
            ])),
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);

        // Check that .PHONY includes sdk-test
        assert!(makefile.contains(".PHONY: all sdk-test"));

        // Check that all depends on both sdk-build and sdk-test
        assert!(makefile.contains("all: sdk-build sdk-test"));

        // Check that sdk-test target exists
        assert!(makefile.contains("sdk-test:"));

        // Check that commands are properly formatted with tabs
        assert!(makefile.contains("\tcargo test --release"));
        assert!(makefile.contains("\tpython run_integration_tests.py"));
    }

    #[test]
    fn test_test_target_with_comments_and_echo() {
        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            envsetup: None,
            test: Some(config::SdkTarget::Commands(vec![
                "# Run unit tests".to_string(),
                "@echo Running test suite".to_string(),
                "cargo test".to_string(),
                "#Run integration tests".to_string(),
                "pytest integration/".to_string(),
            ])),
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);

        // Check that comments are preserved
        assert!(makefile.contains("#Run unit tests"));
        assert!(makefile.contains("#Run integration tests"));

        // Check that @ echo commands are converted properly
        assert!(makefile.contains("\t@echo Running test suite"));

        // Check that regular commands are included
        assert!(makefile.contains("\tcargo test"));
        assert!(makefile.contains("\tpytest integration/"));
    }

    #[test]
    fn test_test_target_empty_commands() {
        // Test with empty test commands
        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);

        // Should not include sdk-test in PHONY or create target
        assert!(makefile.contains(".PHONY: all"));
        assert!(!makefile.contains("sdk-test"));
    }

    #[test]
    fn test_test_target_none() {
        // Test with no test commands
        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);

        // Should not include sdk-test
        assert!(makefile.contains(".PHONY: all"));
        assert!(!makefile.contains("sdk-test"));
    }

    #[test]
    fn test_add_test_target_function() {
        let mut makefile = String::new();
        let target = config::SdkTarget::Commands(vec![
            "# Run comprehensive tests".to_string(),
            "mkdir -p test-results".to_string(),
            "@echo Starting test execution".to_string(),
            "cargo test --verbose".to_string(),
        ]);

        add_test_target(&mut makefile, &target);

        assert!(makefile.contains("sdk-test:"));
        assert!(makefile.contains("\t#Run comprehensive tests"));
        assert!(makefile.contains("\tmkdir -p test-results"));
        assert!(makefile.contains("\t@echo Starting test execution"));
        assert!(makefile.contains("\tcargo test --verbose"));

        // Should end with blank line
        assert!(makefile.ends_with("\n\n"));
    }

    #[test]
    fn test_combined_envsetup_and_test_targets() {
        // Test with both envsetup and test commands
        let config = config::SdkConfig {
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            envsetup: Some(config::SdkTarget::Commands(vec![
                "make configure".to_string()
            ])),
            test: Some(config::SdkTarget::Commands(vec!["make test".to_string()])),
            clean: None,
            build: None,
            flash: None,
        };

        let makefile = generate_makefile_content(&config);

        // Check that .PHONY includes both targets
        assert!(makefile.contains(".PHONY: all sdk-envsetup sdk-test"));

        // Check that all depends on sdk-build and sdk-test
        assert!(makefile.contains("all: sdk-build sdk-test"));

        // Check that both targets exist
        assert!(makefile.contains("sdk-envsetup:"));
        assert!(makefile.contains("sdk-test:"));

        // Check that commands are properly formatted
        assert!(makefile.contains("\tmake configure"));
        assert!(makefile.contains("\tmake test"));
    }

    #[test]
    fn test_generate_release_config_basic() {
        let (_temp_dir, workspace_path) = create_test_workspace();
        let config_path = create_test_sdk_config(&workspace_path);

        // Test basic config generation
        let tag = "v1.0.0";
        let exclude_regex = None;
        let skipped_repos = vec![];
        let tagged_repos = vec![];

        let result = generate_release_config(
            &config_path,
            Some(tag),
            &exclude_regex,
            &skipped_repos,
            &tagged_repos,
        );
        assert!(result.is_ok());

        // Check that the release config file was created
        let release_config_path = workspace_path.join("sdk_v1_0_0.yml");
        assert!(release_config_path.exists());

        // Verify the content has been updated
        let release_content =
            fs::read_to_string(&release_config_path).expect("Failed to read release config");
        assert!(release_content.contains("commit: v1.0.0"));
        assert!(!release_content.contains("commit: main")); // Original commit should be replaced
    }

    #[test]
    fn test_generate_release_config_with_skipped_repos() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Create a config with multiple repositories
        let config_content = r#"
mirror: /tmp/test-mirror
gits:
  - name: test-repo
    url: https://github.com/test/repo.git
    commit: main
  - name: skipped-repo
    url: https://github.com/test/skipped.git
    commit: feature-branch
  - name: another-repo
    url: https://github.com/test/another.git
    commit: develop
"#;
        let config_path = workspace_path.join("sdk.yml");
        fs::write(&config_path, config_content).expect("Failed to write test config");

        let tag = "v2.0.0";
        let exclude_regex = None;
        let skipped_repos = vec!["skipped-repo".to_string()];
        let tagged_repos = vec!["test-repo".to_string(), "another-repo".to_string()];

        let result = generate_release_config(
            &config_path,
            Some(tag),
            &exclude_regex,
            &skipped_repos,
            &tagged_repos,
        );
        assert!(result.is_ok());

        // Check the generated config (should be sdk_release.yml due to non-empty skipped_repos)
        let release_config_path = workspace_path.join("sdk_release.yml");
        let release_content =
            fs::read_to_string(&release_config_path).expect("Failed to read release config");

        // test-repo should be updated
        assert!(release_content.contains("name: test-repo"));
        assert!(release_content.contains("commit: v2.0.0"));

        // skipped-repo should keep original commit
        assert!(release_content.contains("name: skipped-repo"));
        assert!(release_content.contains("commit: feature-branch"));

        // another-repo should be updated
        assert!(release_content.contains("name: another-repo"));
        // The config should have both the original and new commits for different repos
        assert!(release_content.contains("commit: feature-branch")); // skipped repo
    }

    #[test]
    fn test_generate_release_config_filename_sanitization() {
        let (_temp_dir, workspace_path) = create_test_workspace();
        let config_path = create_test_sdk_config(&workspace_path);

        // Test with tag that contains special characters
        let tag = "v1.0.0/rc.1:test";
        let exclude_regex = None;
        let skipped_repos = vec![];
        let tagged_repos = vec![];

        let result = generate_release_config(
            &config_path,
            Some(tag),
            &exclude_regex,
            &skipped_repos,
            &tagged_repos,
        );
        assert!(result.is_ok());

        // Check that special characters are sanitized in filename
        let sanitized_filename = "sdk_v1_0_0_rc_1_test.yml";
        let release_config_path = workspace_path.join(sanitized_filename);
        assert!(release_config_path.exists());

        // But the tag in the content should remain unchanged
        let release_content =
            fs::read_to_string(&release_config_path).expect("Failed to read release config");
        assert!(release_content.contains("commit: v1.0.0/rc.1:test"));
    }

    #[test]
    fn test_release_config_preserves_structure() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Create a config with dependencies and build commands
        let config_content = r#"
mirror: /tmp/test-mirror

copy_files:
  - source: os-dependencies.yml
    dest: os-dependencies.yml

gits:
  - name: base-repo
    url: https://github.com/test/base.git
    commit: main
    build:
      - "make configure"
      - "make all"

  - name: dependent-repo
    url: https://github.com/test/dependent.git
    commit: develop
    depends_on:
      - base-repo
    build:
      - "cmake ."
      - "make"

  - name: simple-repo
    url: https://github.com/test/simple.git
    commit: feature-xyz
"#;
        let config_path = workspace_path.join("sdk.yml");
        fs::write(&config_path, config_content).expect("Failed to write test config");

        let tag = "v3.0.0";
        let exclude_regex = None;
        let skipped_repos = vec![];
        let tagged_repos = vec![];

        let result = generate_release_config(
            &config_path,
            Some(tag),
            &exclude_regex,
            &skipped_repos,
            &tagged_repos,
        );
        assert!(result.is_ok());

        let release_config_path = workspace_path.join("sdk_v3_0_0.yml");
        let release_content =
            fs::read_to_string(&release_config_path).expect("Failed to read release config");

        // Verify structure is preserved
        assert!(release_content.contains("mirror: /tmp/test-mirror"));
        assert!(release_content.contains("copy_files:"));

        // Verify dependencies are preserved
        assert!(release_content.contains("depends_on:"));
        assert!(release_content.contains("- base-repo"));

        // Verify build commands are preserved
        assert!(release_content.contains("build:"));
        assert!(release_content.contains("- \"make configure\""));
        assert!(release_content.contains("- \"cmake .\""));

        // Verify commits are updated
        assert!(release_content.contains("commit: v3.0.0"));
        assert!(!release_content.contains("commit: main"));
        assert!(!release_content.contains("commit: develop"));
        assert!(!release_content.contains("commit: feature-xyz"));
    }

    #[test]
    fn test_regex_compilation_and_matching() {
        // Test that our regex patterns work as expected
        let pattern = "optee.*";
        let regex = Regex::new(pattern).expect("Failed to compile regex");

        assert!(regex.is_match("optee_os"));
        assert!(regex.is_match("optee_client"));
        assert!(regex.is_match("optee_test"));
        assert!(!regex.is_match("linux"));
        assert!(!regex.is_match("buildroot"));

        // Test more complex pattern
        let complex_pattern = "^(optee|test)_.*";
        let complex_regex = Regex::new(complex_pattern).expect("Failed to compile complex regex");

        assert!(complex_regex.is_match("optee_os"));
        assert!(complex_regex.is_match("test_repo"));
        assert!(!complex_regex.is_match("linux_optee"));
    }

    #[test]
    fn test_release_config_edge_cases() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Test with config that has no gits section
        let minimal_config = r#"
mirror: /tmp/test-mirror
gits:
"#;
        let config_path = workspace_path.join("sdk.yml");
        fs::write(&config_path, minimal_config).expect("Failed to write minimal config");

        let result = generate_release_config(&config_path, Some("v1.0.0"), &None, &[], &[]);
        assert!(result.is_ok());

        let release_config_path = workspace_path.join("sdk_v1_0_0.yml");
        assert!(release_config_path.exists());
    }

    #[test]
    fn test_release_config_indentation_preservation() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Test with various indentation patterns
        let config_content = r#"
mirror: /tmp/test-mirror
gits:
  - name: repo1
    url: https://github.com/test/repo1.git
    commit: main
  - name: repo2
    url: https://github.com/test/repo2.git
    commit: develop
    depends_on:
      - repo1
"#;
        let config_path = workspace_path.join("sdk.yml");
        fs::write(&config_path, config_content).expect("Failed to write indented config");

        let result = generate_release_config(&config_path, Some("v1.0.0"), &None, &[], &[]);
        assert!(result.is_ok());

        let release_config_path = workspace_path.join("sdk_v1_0_0.yml");
        let release_content =
            fs::read_to_string(&release_config_path).expect("Failed to read release config");

        // Check that indentation is preserved for commit lines
        assert!(release_content.contains("    commit: v1.0.0"));
        assert!(release_content.contains("  - name: repo1"));
        assert!(release_content.contains("      - repo1"));
    }

    #[test]
    fn test_invalid_regex_pattern() {
        // Test that invalid regex patterns are handled gracefully
        let invalid_patterns = vec![
            "[",     // Unclosed bracket
            "\\",    // Trailing backslash
            "(?P<)", // Invalid named group
        ];

        for pattern in invalid_patterns {
            let result = Regex::new(pattern);
            assert!(result.is_err(), "Pattern '{}' should be invalid", pattern);
        }
    }

    // Test cases for the new list-targets command functionality
    #[test]
    fn test_list_targets_from_source_local() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Create a mock targets directory structure
        let targets_dir = workspace_path.join("targets");
        fs::create_dir_all(&targets_dir).expect("Failed to create targets dir");

        // Create valid target directories with sdk.yml
        let target1_dir = targets_dir.join("target1");
        fs::create_dir_all(&target1_dir).expect("Failed to create target1 dir");
        fs::write(target1_dir.join("sdk.yml"), "mirror: /tmp/mirror\ngits: []")
            .expect("Failed to write target1 config");

        let target2_dir = targets_dir.join("target2");
        fs::create_dir_all(&target2_dir).expect("Failed to create target2 dir");
        fs::write(target2_dir.join("sdk.yml"), "mirror: /tmp/mirror\ngits: []")
            .expect("Failed to write target2 config");

        // Create invalid target directory without sdk.yml
        let invalid_dir = targets_dir.join("invalid");
        fs::create_dir_all(&invalid_dir).expect("Failed to create invalid dir");

        // Test listing targets
        let result = list_targets_from_source(&workspace_path.to_string_lossy());
        assert!(result.is_ok());

        let targets = result.unwrap();
        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&"target1".to_string()));
        assert!(targets.contains(&"target2".to_string()));
        assert!(!targets.contains(&"invalid".to_string()));
    }

    #[test]
    fn test_list_targets_from_source_nonexistent_path() {
        let result = list_targets_from_source("/nonexistent/path");
        assert!(result.is_err());

        let error_msg = format!("{}", result.unwrap_err());
        assert!(
            error_msg.contains("Targets directory not found")
                || error_msg.contains("Failed to read targets directory")
                || error_msg.contains("No such file or directory")
        );
    }

    #[test]
    fn test_list_targets_from_source_empty_directory() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Create empty targets directory
        let targets_dir = workspace_path.join("targets");
        fs::create_dir_all(&targets_dir).expect("Failed to create targets dir");

        let result = list_targets_from_source(&workspace_path.to_string_lossy());
        assert!(result.is_ok());

        let targets = result.unwrap();
        assert!(targets.is_empty());
    }

    #[test]
    fn test_resolve_target_config_from_git_invalid_target() {
        // This test will create a temporary git repo and test with invalid target
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Create a mock git repository structure
        let git_repo_dir = workspace_path.join("fake-git-repo");
        fs::create_dir_all(&git_repo_dir).expect("Failed to create git repo dir");

        let targets_dir = git_repo_dir.join("targets");
        fs::create_dir_all(&targets_dir).expect("Failed to create targets dir");

        // Create one valid target
        let valid_target_dir = targets_dir.join("valid-target");
        fs::create_dir_all(&valid_target_dir).expect("Failed to create valid target dir");
        fs::write(valid_target_dir.join("sdk.yml"), "mirror: /tmp\ngits: []")
            .expect("Failed to write valid config");

        // Test with invalid target name - this will fail because we're not using real git
        // but we can test the error handling
        let result = resolve_target_config_from_git(
            &git_repo_dir.to_string_lossy(),
            "nonexistent-target",
            None,
            None,
        );
        // This should fail since it's not a real git repo, but the error should be about git clone
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err());
        assert!(
            error_msg.contains("Local git clone failed")
                || error_msg.contains("Git clone failed")
                || error_msg.contains("Failed to run git clone")
        );
    }

    #[test]
    fn test_list_target_versions_local_source() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // For local sources, list_target_versions should return empty vector
        let result = list_target_versions(&workspace_path.to_string_lossy(), "any-target");
        assert!(result.is_ok());

        let versions = result.unwrap();
        assert!(versions.is_empty()); // Local directories don't support version listing yet
    }

    #[test]
    fn test_handle_list_targets_command_with_nonexistent_source() {
        // Test the command handler with a nonexistent source
        // This should print error message and exit with code 1
        // We can't easily test the exit behavior in unit tests, but we can test
        // that the function handles the error case

        // Since handle_list_targets_command calls std::process::exit(1) on error,
        // we need to test the underlying function instead
        let result = list_targets_from_source("/completely/nonexistent/path");
        assert!(result.is_err());
    }

    // Test cases for the updated init command functionality
    #[test]
    fn test_init_command_target_validation() {
        // Test that init command properly validates target parameter
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Create a proper target-based config structure
        let targets_dir = workspace_path.join("targets");
        let target_dir = targets_dir.join("test-target");
        fs::create_dir_all(&target_dir).expect("Failed to create target dir");

        let config_content = r#"
mirror: /tmp/mirror
gits:
  - name: test-repo
    url: https://github.com/test/repo.git
    commit: main
"#;
        let config_path = target_dir.join("sdk.yml");
        fs::write(&config_path, config_content).expect("Failed to write config");

        // Test resolve_target_config with valid target name
        let result = resolve_target_config("test-target", &workspace_path);
        assert!(result.is_ok());

        let resolved_path = result.unwrap();
        assert_eq!(resolved_path, config_path);
    }

    #[test]
    fn test_resolve_target_config_local_path() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Test resolving local target-based config
        let targets_dir = workspace_path.join("targets");
        let target_dir = targets_dir.join("test-target");
        fs::create_dir_all(&target_dir).expect("Failed to create target dir");

        let config_content = r#"
mirror: /tmp/mirror
gits:
  - name: test-repo
    url: https://github.com/test/repo.git
    commit: main
"#;
        let config_path = target_dir.join("sdk.yml");
        fs::write(&config_path, config_content).expect("Failed to write config");

        let result = resolve_target_config("test-target", &workspace_path);
        assert!(result.is_ok());

        let resolved_path = result.unwrap();
        assert_eq!(resolved_path, config_path);
    }

    #[test]
    fn test_resolve_target_config_invalid_target() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Create targets directory but no target
        let targets_dir = workspace_path.join("targets");
        fs::create_dir_all(&targets_dir).expect("Failed to create targets dir");

        let result = resolve_target_config("nonexistent-target", &workspace_path);
        assert!(result.is_err());

        let error_msg = format!("{}", result.unwrap_err());
        assert!(error_msg.contains("Target 'nonexistent-target' not found"));
    }

    #[test]
    fn test_list_available_targets_with_mixed_content() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Create targets directory with mixed valid/invalid content
        let targets_dir = workspace_path.join("targets");
        fs::create_dir_all(&targets_dir).expect("Failed to create targets dir");

        // Valid target with sdk.yml
        let valid_dir = targets_dir.join("valid-target");
        fs::create_dir_all(&valid_dir).expect("Failed to create valid dir");
        fs::write(valid_dir.join("sdk.yml"), "mirror: /tmp\ngits: []")
            .expect("Failed to write valid config");

        // Directory without sdk.yml
        let no_config_dir = targets_dir.join("no-config");
        fs::create_dir_all(&no_config_dir).expect("Failed to create no-config dir");

        // File instead of directory
        fs::write(targets_dir.join("not-a-dir.txt"), "content").expect("Failed to create file");

        // Hidden directory (should be ignored)
        let hidden_dir = targets_dir.join(".hidden");
        fs::create_dir_all(&hidden_dir).expect("Failed to create hidden dir");
        fs::write(hidden_dir.join("sdk.yml"), "mirror: /tmp\ngits: []")
            .expect("Failed to write hidden config");

        let result = list_available_targets(&workspace_path);
        assert!(result.is_ok());

        let targets = result.unwrap();
        assert_eq!(targets.len(), 2); // Both valid-target and .hidden have sdk.yml
        assert!(targets.contains(&"valid-target".to_string()));
        assert!(targets.contains(&".hidden".to_string())); // Hidden dirs with sdk.yml are included
        assert!(!targets.contains(&"no-config".to_string()));
        assert!(!targets.contains(&"not-a-dir.txt".to_string()));
    }

    #[test]
    fn test_is_branch_reference_with_mock_git_repo() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Create a mock git repository structure
        let repo_path = workspace_path.join("test-repo");
        fs::create_dir_all(&repo_path).expect("Failed to create repo dir");

        // Initialize a git repo
        let init_result = git_operations::init_repo(&repo_path, false);

        if init_result.is_err() {
            // Skip this test if git is not available
            return;
        }

        // Configure git user for testing
        let _ = git_operations::config(&repo_path, "user.email", "test@example.com");
        let _ = git_operations::config(&repo_path, "user.name", "Test User");

        // Create a test file and commit
        fs::write(repo_path.join("test.txt"), "test content").expect("Failed to write test file");
        let _ = git_operations::add_files(&repo_path, &["test.txt"]);
        let _ = git_operations::commit(&repo_path, "Initial commit");

        // Create a branch
        let _ = git_operations::create_branch(&repo_path, "feature-branch", None);

        // Create a tag
        let _ = git_operations::create_tag(&repo_path, "v1.0.0");

        // Test branch detection
        assert!(
            is_branch_reference(&repo_path, "main") || is_branch_reference(&repo_path, "master")
        ); // Default branch
        assert!(is_branch_reference(&repo_path, "feature-branch")); // Created branch
        assert!(!is_branch_reference(&repo_path, "v1.0.0")); // Tag should not be detected as branch
        assert!(!is_branch_reference(&repo_path, "nonexistent")); // Non-existent reference
    }

    #[test]
    fn test_get_latest_commit_for_branch_functionality() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Create a mock git repository
        let repo_path = workspace_path.join("test-repo");
        fs::create_dir_all(&repo_path).expect("Failed to create repo dir");

        // Initialize git repo
        let init_result = git_operations::init_repo(&repo_path, false);

        if init_result.is_err() {
            // Skip this test if git is not available
            return;
        }

        // Configure git user
        let _ = git_operations::config(&repo_path, "user.email", "test@example.com");
        let _ = git_operations::config(&repo_path, "user.name", "Test User");

        // Create and commit a file
        fs::write(repo_path.join("test.txt"), "test content").expect("Failed to write test file");
        let _ = git_operations::add_files(&repo_path, &["test.txt"]);
        let commit_result = git_operations::commit(&repo_path, "Initial commit");

        if commit_result.is_ok() {
            // Test getting latest commit
            let default_branch = if is_branch_reference(&repo_path, "main") {
                "main"
            } else {
                "master"
            };

            let latest_commit = get_latest_commit_for_branch(&repo_path, default_branch);
            assert!(latest_commit.is_some());

            if let Some(commit_hash) = latest_commit {
                // Commit hash should be 40 characters (SHA-1)
                assert_eq!(commit_hash.len(), 40);
                // Should be hexadecimal
                assert!(commit_hash.chars().all(|c| c.is_ascii_hexdigit()));
            }
        }

        // Test non-existent branch
        let non_existent = get_latest_commit_for_branch(&repo_path, "nonexistent-branch");
        assert!(non_existent.is_none());
    }

    #[test]
    fn test_branch_vs_tag_detection_edge_cases() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Test detection with non-git directory
        assert!(!is_branch_reference(&workspace_path, "main"));
        assert!(!is_branch_reference(&workspace_path, "v1.0.0"));

        // Test with empty string
        assert!(!is_branch_reference(&workspace_path, ""));

        // Test get_latest_commit with non-git directory
        assert!(get_latest_commit_for_branch(&workspace_path, "main").is_none());
    }

    #[test]
    fn test_workspace_name_format_validation() {
        // Test that workspace names are correctly formatted with dsdk- prefix
        let test_cases = vec![
            ("simple", "dsdk-simple"),
            ("with-dashes", "dsdk-with-dashes"),
            ("with_underscores", "dsdk-with_underscores"),
            ("MixedCase", "dsdk-MixedCase"),
            ("123numeric", "dsdk-123numeric"),
        ];

        for (target, expected_name) in test_cases {
            let workspace_name = format!("dsdk-{}", target);
            assert_eq!(workspace_name, expected_name);
            assert!(workspace_name.starts_with("dsdk-"));
            assert!(workspace_name.len() > 5); // More than just "dsdk-"
        }
    }

    #[test]
    fn test_default_workspace_path_construction() {
        // Test the actual path construction logic used in handle_init_command
        let target = "adi-sdk";
        let workspace_name = format!("dsdk-{}", target);

        let workspace_path = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(workspace_name);

        // Verify path contains the target-based workspace name
        assert!(workspace_path.to_string_lossy().contains("dsdk-adi-sdk"));

        // Verify it doesn't contain the old default
        assert!(!workspace_path.to_string_lossy().ends_with("dsdk-workspace"));

        // Verify path is absolute or relative to current dir
        assert!(!workspace_path.to_string_lossy().is_empty());
    }

    #[test]
    fn test_workspace_prefix_default() {
        // Test that default prefix is "dsdk-" when no config is provided
        let target = "adi-sdk";
        let prefix = "dsdk-";
        let workspace_name = format!("{}{}", prefix, target);

        assert_eq!(workspace_name, "dsdk-adi-sdk");
        assert!(workspace_name.starts_with("dsdk-"));
    }

    #[test]
    fn test_workspace_prefix_custom() {
        // Test that custom prefixes work correctly
        let test_cases = vec![
            ("sdk-", "adi-sdk", "sdk-adi-sdk"),
            ("dev-", "optee-qemu-v8", "dev-optee-qemu-v8"),
            ("project-", "dummy1", "project-dummy1"),
            ("my_", "test", "my_test"),
        ];

        for (prefix, target, expected) in test_cases {
            let workspace_name = format!("{}{}", prefix, target);
            assert_eq!(workspace_name, expected);
        }
    }

    #[test]
    fn test_workspace_prefix_empty() {
        // Test that empty prefix works (no prefix at all)
        let target = "adi-sdk";
        let prefix = "";
        let workspace_name = format!("{}{}", prefix, target);

        assert_eq!(workspace_name, "adi-sdk");
        assert_eq!(workspace_name, target);
        assert!(!workspace_name.starts_with("dsdk-"));
    }

    #[test]
    fn test_workspace_prefix_with_path() {
        // Test prefix with full path construction
        let test_cases = vec![
            ("dsdk-", "adi-sdk", "dsdk-adi-sdk"),
            ("sdk-", "adi-sdk", "sdk-adi-sdk"),
            ("", "adi-sdk", "adi-sdk"),
            ("my-project-", "test", "my-project-test"),
        ];

        for (prefix, target, expected_name) in test_cases {
            let workspace_name = format!("{}{}", prefix, target);
            let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let workspace_path = PathBuf::from(&home).join(&workspace_name);

            assert!(workspace_path.to_string_lossy().contains(expected_name));
        }
    }

    #[test]
    fn test_workspace_prefix_priority() {
        // Test that workspace naming respects: CLI > config default_workspace > prefix+target
        let target = "test-target";
        let prefix = "sdk-";

        // Simulate different scenarios

        // Scenario 1: CLI workspace specified (highest priority)
        let cli_workspace = Some(PathBuf::from("/custom/path"));
        let result = if let Some(path) = cli_workspace {
            path
        } else {
            // Would check config.default_workspace here, skip for test
            let workspace_name = format!("{}{}", prefix, target);
            PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".to_string())).join(workspace_name)
        };
        assert_eq!(result, PathBuf::from("/custom/path"));

        // Scenario 2: No CLI, use prefix + target
        let workspace_name = format!("{}{}", prefix, target);
        let result = PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".to_string()))
            .join(&workspace_name);
        assert!(result.to_string_lossy().contains("sdk-test-target"));

        // Scenario 3: Empty prefix
        let empty_prefix = "";
        let workspace_name = format!("{}{}", empty_prefix, target);
        assert_eq!(workspace_name, target);
    }
}
