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

use crate::cli::InstallCommand;
use dsdk_cli::workspace::{
    get_current_workspace, load_config_with_user_overrides, require_workspace_config,
    resolve_mirror, OS_DEPS_FILE, PYTHON_DEPS_FILE,
};
use dsdk_cli::{config, messages, toolchain_manager};
use std::io;
use std::path::{Path, PathBuf};

/// Install system dependencies based on the type specified
pub(crate) fn handle_install_command(install_command: &InstallCommand) {
    let (workspace_path, config_path) = match require_workspace_config() {
        Ok(paths) => paths,
        Err(e) => {
            messages::error(&e);
            return;
        }
    };

    // Load SDK config with user config overrides applied
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

    match install_command {
        InstallCommand::OsDeps { yes, no_sudo } => {
            // Look for os-dependencies.yml file in workspace (copied via copy_files)
            let os_deps_path = workspace_path.join(OS_DEPS_FILE);
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
            let python_deps_path = workspace_path.join(PYTHON_DEPS_FILE);

            // Show available profiles if user requested the list
            if *list_profiles {
                if python_deps_path.exists() {
                    list_available_profiles(&python_deps_path);
                } else {
                    messages::error("python-dependencies.yml not found in workspace.");
                }
                return;
            }

            // Per-repo Python deps declared in sdk.yml gits: entries are installed
            // into isolated venvs at .cim/<git>/.venv, independent of the shared
            // workspace venv populated from python-dependencies.yml profiles.
            for git in &sdk_config.gits {
                if let Some(reqs) = &git.python_deps {
                    if let Err(e) =
                        install_git_python_deps(&workspace_path, &git.name, reqs, *force)
                    {
                        messages::error(&format!(
                            "Failed to install Python deps for '{}': {}",
                            git.name, e
                        ));
                        std::process::exit(1);
                    }
                }
            }

            // Shared workspace venv from python-dependencies.yml profiles (docs,
            // lint, and other workspace-wide cim tooling).
            if python_deps_path.exists() {
                // Resolve mirror from user config / built-in default.
                if let Err(e) = install_python_packages_from_file(
                    &python_deps_path,
                    *force,
                    *symlink,
                    profile.as_deref(),
                    &workspace_path,
                    &resolve_mirror(None, sdk_config.mirror.as_deref()),
                ) {
                    messages::error(&format!("Failed to install Python packages: {}", e));
                    std::process::exit(1);
                }
            } else if !sdk_config.gits.iter().any(|g| g.python_deps.is_some()) {
                // Nothing to do from either source: report the missing profiles file.
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

            // Resolve mirror from user config / built-in default.
            // Create toolchain manager and install toolchains
            let toolchain_manager = toolchain_manager::ToolchainManager::new(
                workspace_path.clone(),
                resolve_mirror(None, sdk_config.mirror.as_deref()),
            );

            if let Err(e) = toolchain_manager.install_toolchains(
                sdk_config.toolchains.as_ref(),
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
            // Use sdk_config that already has user overrides applied
            // Check if install section exists
            if sdk_config.install.is_none() || sdk_config.install.as_ref().unwrap().is_empty() {
                messages::error("No install section found in sdk.yml");
                messages::info("The install section defines components that can be installed.");
                return;
            }

            let install_configs = sdk_config.install.as_ref().unwrap();

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
pub(crate) struct VenvManager {
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

        run_python_venv_creation(&self.workspace_path)
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

            run_python_venv_creation(&self.mirror_path)?;
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

/// Get the platform-specific path to the venv's bin/Scripts directory.
pub(crate) fn get_venv_bin_dir(workspace_path: &Path) -> PathBuf {
    workspace_path.join(".venv").join({
        #[cfg(windows)]
        {
            "Scripts"
        }

        #[cfg(not(windows))]
        {
            "bin"
        }
    })
}

/// Get the platform-specific Python executable path inside the venv.
pub(crate) fn get_venv_python_path(workspace_path: &Path) -> PathBuf {
    get_venv_bin_dir(workspace_path).join({
        #[cfg(windows)]
        {
            "python.exe"
        }

        #[cfg(not(windows))]
        {
            "python3"
        }
    })
}

/// Check if a virtual environment exists in the workspace.
pub(crate) fn venv_exists(workspace_path: &Path) -> bool {
    let venv_path = workspace_path.join(".venv");
    venv_path.exists() && get_venv_python_path(workspace_path).exists()
}

/// Get the platform-specific Python interpreter command.
fn python_command() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}

/// Resolve the absolute path of the system Python interpreter that the stdlib
/// venv path would use, so the uv backend can be pinned to the same one.
///
/// Returns `None` if the interpreter cannot be located, in which case uv falls
/// back to its own interpreter discovery.
fn resolve_system_python() -> Option<PathBuf> {
    let output = std::process::Command::new(python_command())
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

/// Check whether the `uv` binary is available on PATH.
///
/// When present, uv is used as a faster, drop-in backend for creating virtual
/// environments and installing packages. uv produces a standard PEP 405 venv,
/// so the resulting environment is interchangeable with one created by
/// `python3 -m venv`. When absent, cim falls back to the stdlib venv + pip path.
fn uv_available() -> bool {
    std::process::Command::new("uv")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Create a Python venv at the given path
fn run_python_venv_creation(workspace_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let venv_path = workspace_path.join(".venv");

    messages::status(&format!(
        "Creating Python virtual environment at {}...",
        venv_path.display()
    ));

    // Prefer uv when available: `uv venv` creates a standard venv and does not
    // need a separate ensurepip/upgrade step (uv manages installs itself).
    //
    // Pin the interpreter to the same `python3` the stdlib fallback would use.
    // Without `--python`, uv applies its own discovery and may prefer a
    // uv-managed CPython download over the system interpreter, producing a venv
    // on a different Python version than the fallback path — defeating the
    // "identical with or without uv" guarantee.
    if uv_available() {
        let mut command = std::process::Command::new("uv");
        command.arg("venv");
        if let Some(python) = resolve_system_python() {
            command.arg("--python").arg(python);
        }
        let output = command.arg(&venv_path).output()?;

        if !output.status.success() {
            return Err(format!(
                "Failed to create virtual environment with uv:\n{}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        messages::success("Virtual environment created successfully");
        return Ok(());
    }

    // Create the venv.
    let output = std::process::Command::new(python_command())
        .args(["-m", "venv"])
        .arg(venv_path)
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to create virtual environment:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    // Determine the python executable inside the newly-created venv
    let venv_python = get_venv_python_path(workspace_path);

    // Bootstrap pip inside the venv
    let ensurepip_output = std::process::Command::new(&venv_python)
        .args(["-m", "ensurepip"])
        .output()?;

    if !ensurepip_output.status.success() {
        return Err(format!(
            "Failed ensure pip in virtual environment:\n{}",
            String::from_utf8_lossy(&ensurepip_output.stderr)
        )
        .into());
    }
    let upgrade_pip_output = std::process::Command::new(&venv_python)
        .args(["-m", "pip", "install", "pip", "--upgrade"])
        .output()?;

    if !upgrade_pip_output.status.success() {
        return Err(format!(
            "Failed to upgrade pip in virtual environment:\n{}",
            String::from_utf8_lossy(&upgrade_pip_output.stderr)
        )
        .into());
    }

    messages::success("Virtual environment created successfully");
    Ok(())
}

/// Create a Python virtual environment in the workspace
pub(crate) fn create_virtual_environment(
    workspace_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if venv_exists(workspace_path) {
        messages::info(&format!(
            "Virtual environment already exists at {}",
            workspace_path.display()
        ));
        return Ok(());
    }

    run_python_venv_creation(workspace_path)
}

/// Detect if we're running in a container environment
pub(crate) fn is_container_environment() -> bool {
    // Check for common container indicators
    std::path::Path::new("/.dockerenv").exists()
        || std::env::var("container").is_ok()
        || std::fs::read_to_string("/proc/1/cgroup")
            .map(|content| content.contains("docker") || content.contains("containerd"))
            .unwrap_or(false)
}

/// Check if sphinx-build is available in system PATH
pub(crate) fn is_sphinx_available() -> bool {
    std::process::Command::new("sphinx-build")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Ensure documentation dependencies are available in virtual environment
pub(crate) fn ensure_docs_dependencies(
    workspace_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
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
        let sphinx_build_name = if cfg!(windows) {
            "sphinx-build.exe"
        } else {
            "sphinx-build"
        };
        let sphinx_build_path = get_venv_bin_dir(workspace_path).join(sphinx_build_name);
        if sphinx_build_path.exists() {
            // Virtual environment exists and has sphinx, we're good
            return Ok(());
        }
    }

    messages::status("Documentation dependencies not found. Setting up virtual environment...");

    // Create virtual environment if it doesn't exist
    create_virtual_environment(workspace_path)?;

    // Install required packages
    let python_deps_path = workspace_path.join(PYTHON_DEPS_FILE);

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

/// Resolve a workspace path from an optional override, falling back to the
/// current workspace, and verify a virtual environment exists in it.
fn resolve_venv_python(
    workspace_path_override: Option<&Path>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
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

    if !venv_exists(&workspace_path) {
        return Err(
            "Could not finish setting up Python packages: virtual environment not found".into(),
        );
    }

    Ok(get_venv_python_path(&workspace_path))
}

/// Run a pip install into the given venv with the supplied trailing arguments
/// (package specifiers and/or `-r <file>` pairs). Prefers `uv` when available,
/// otherwise invokes pip directly inside the venv with the trusted-host flags.
fn run_pip_install(
    venv_python: &Path,
    install_args: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let status = if uv_available() {
        messages::status(&format!(
            "Running: uv pip install --python {} {}",
            venv_python.display(),
            install_args.join(" ")
        ));
        std::process::Command::new("uv")
            .args(["pip", "install", "--python"])
            .arg(venv_python)
            .args(install_args)
            .status()
            .map_err(|e| {
                format!(
                    "Could not finish setting up Python packages: failed to execute uv: {}",
                    e
                )
            })?
    } else {
        messages::status(&format!(
            "Running: {} -m pip install {}",
            venv_python.display(),
            install_args.join(" ")
        ));
        std::process::Command::new(venv_python)
            .args(["-m", "pip", "install"])
            .arg("--trusted-host")
            .arg("pypi.org")
            .arg("--trusted-host")
            .arg("pypi.python.org")
            .arg("--trusted-host")
            .arg("files.pythonhosted.org")
            .args(install_args)
            .status()
            .map_err(|e| {
                format!(
                    "Could not finish setting up Python packages: failed to execute pip: {}",
                    e
                )
            })?
    };

    if !status.success() {
        return Err(format!(
            "Could not finish setting up Python packages: installation failed with exit code: {}",
            status
        )
        .into());
    }

    Ok(())
}

/// Helper function to install pip packages
pub(crate) fn install_pip_packages(
    packages: &[String],
    workspace_path_override: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    if packages.is_empty() {
        messages::info("No Python packages to install");
        return Ok(());
    }

    let venv_python = resolve_venv_python(workspace_path_override)?;
    messages::verbose(&format!(
        "Using virtual environment python: {}",
        venv_python.display()
    ));

    run_pip_install(&venv_python, packages)?;

    messages::success("Successfully installed Python packages in virtual environment");
    Ok(())
}

/// Build the trailing `-r <abs-path>` arguments for a list of requirements
/// files, resolving each path relative to the workspace root. Returns an error
/// if any file is missing.
fn build_requirements_args(
    requirements: &[String],
    workspace_path: &Path,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut install_args: Vec<String> = Vec::with_capacity(requirements.len() * 2);
    for req in requirements {
        let req_path = workspace_path.join(req);
        if !req_path.exists() {
            return Err(format!(
                "Could not finish setting up Python packages: requirements file not found: {}",
                req_path.display()
            )
            .into());
        }
        install_args.push("-r".to_string());
        install_args.push(req_path.to_string_lossy().into_owned());
    }
    Ok(install_args)
}

/// Install packages listed in one or more `requirements.txt` files into the
/// workspace virtual environment. Paths are resolved relative to the workspace
/// root. Missing files are reported as errors.
pub(crate) fn install_pip_requirements(
    requirements: &[String],
    workspace_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if requirements.is_empty() {
        return Ok(());
    }

    let venv_python = resolve_venv_python(Some(workspace_path))?;
    let install_args = build_requirements_args(requirements, workspace_path)?;

    messages::verbose(&format!(
        "Using virtual environment python: {}",
        venv_python.display()
    ));

    run_pip_install(&venv_python, &install_args)?;

    messages::success("Successfully installed Python requirements in virtual environment");
    Ok(())
}

/// Create (or reuse) an isolated virtual environment for a single git and
/// install its `requirements.txt` files into it.
///
/// The venv lives at `<workspace>/.cim/<git-name>/.venv` (see
/// [`dsdk_cli::workspace::git_venv_path`]). Requirement paths are resolved
/// relative to the git's own checkout directory (`<workspace>/<git-name>`), so
/// `python-deps: scripts/requirements.txt` refers to the file inside that
/// repository. With `force`, an existing venv is recreated.
pub(crate) fn install_git_python_deps(
    workspace_path: &Path,
    git_name: &str,
    requirements: &[String],
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if requirements.is_empty() {
        return Ok(());
    }

    // Validate requirements files up front before any venv side effects.
    // Paths are relative to the git's checkout directory.
    let git_checkout = workspace_path.join(git_name);
    let install_args = build_requirements_args(requirements, &git_checkout)?;

    // The venv primitives take a base directory and append `.venv`, so the
    // base for this git's venv is the parent of git_venv_path(..).
    let venv_path = dsdk_cli::workspace::git_venv_path(workspace_path, git_name);
    let venv_base = venv_path
        .parent()
        .expect("git_venv_path always has a parent")
        .to_path_buf();

    if venv_path.exists() {
        if force {
            messages::info(&format!(
                "Virtual environment for '{}' exists, removing due to --force",
                git_name
            ));
            std::fs::remove_dir_all(&venv_path)?;
        } else {
            messages::info(&format!(
                "Virtual environment for '{}' already exists at {}, reusing (use --force to reinstall)",
                git_name,
                venv_path.display()
            ));
        }
    }

    std::fs::create_dir_all(&venv_base)?;

    if !venv_exists(&venv_base) {
        run_python_venv_creation(&venv_base)?;
    }

    let venv_python = get_venv_python_path(&venv_base);
    messages::status(&format!(
        "Installing Python requirements for '{}' into {}",
        git_name,
        venv_path.display()
    ));
    run_pip_install(&venv_python, &install_args)?;

    messages::success(&format!(
        "Successfully installed Python requirements for '{}'",
        git_name
    ));
    Ok(())
}

/// List available Python dependency profiles
pub(crate) fn list_available_profiles(python_deps_path: &Path) {
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
pub(crate) fn install_python_packages_from_file(
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

    // Collect unique packages and requirements files from all specified profiles
    use std::collections::HashSet;
    let mut all_packages = HashSet::new();
    let mut requirements: Vec<String> = Vec::new();

    for profile_name in &profile_names {
        if let Some(profile) = python_deps.profiles.get(*profile_name) {
            for package in &profile.packages {
                all_packages.insert(package.clone());
            }
            if let Some(reqs) = &profile.requirements {
                for req in reqs {
                    if !requirements.contains(req) {
                        requirements.push(req.clone());
                    }
                }
            }
            messages::verbose(&format!(
                "Profile '{}' adds {} package(s) and {} requirements file(s)",
                profile_name,
                profile.packages.len(),
                profile.requirements.as_ref().map_or(0, |r| r.len()),
            ));
        }
    }

    // Convert HashSet to sorted Vec for consistent output
    let mut packages: Vec<String> = all_packages.into_iter().collect();
    packages.sort();

    if packages.is_empty() && requirements.is_empty() {
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

    if !packages.is_empty() {
        messages::status(&format!(
            "Installing {} unique package(s) from {} profile(s)",
            packages.len(),
            profile_names.len()
        ));
        install_pip_packages(&packages, Some(workspace_path))?;
    }

    if !requirements.is_empty() {
        messages::status(&format!(
            "Installing {} requirements file(s) from {} profile(s)",
            requirements.len(),
            profile_names.len()
        ));
        install_pip_requirements(&requirements, workspace_path)?;
    }

    Ok(())
}

/// Install prerequisites based on OS dependencies configuration
pub(crate) fn install_prerequisites(
    os_deps: &config::OsDependencies,
    skip_prompt: bool,
    no_sudo: bool,
) {
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
            let mut packages = distro_config.package_manager.resolved_packages();
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
pub(crate) fn detect_os_and_distro() -> (String, String, String) {
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
    fn test_venv_exists_true() {
        let (_temp_dir, workspace_path) = create_test_workspace();
        let bin_dir = get_venv_bin_dir(&workspace_path);
        fs::create_dir_all(&bin_dir).expect("Failed to create venv structure");

        // Create python3 executable (empty file is fine for test)
        let python_exe = get_venv_python_path(&workspace_path);
        fs::write(&python_exe, "").expect("Failed to create python file");

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
    fn test_is_container_environment_false() {
        // This test assumes we're not running in a container
        // In CI, this might need adjustment
        if !Path::new("/.dockerenv").exists()
            && std::env::var("container").is_err()
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
    fn test_venv_path_helpers() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        // Test venv detection with non-existent venv
        assert!(!venv_exists(&workspace_path));

        // Test venv detection with partial venv structure
        let venv_dir = workspace_path.join(".venv");
        fs::create_dir_all(&venv_dir).expect("Failed to create .venv");
        assert!(!venv_exists(&workspace_path)); // Still false, no python3

        // Test venv detection with complete structure
        let bin_dir = get_venv_bin_dir(&workspace_path);
        fs::create_dir_all(&bin_dir).expect("Failed to create bin dir");
        let python_exe = get_venv_python_path(&workspace_path);
        fs::write(&python_exe, "").expect("Failed to create python");
        assert!(venv_exists(&workspace_path)); // Now true
    }

    #[test]
    fn test_venv_python_has_virtual_env_set() {
        let (_temp_dir, workspace_path) = create_test_workspace();

        run_python_venv_creation(&workspace_path).expect("Failed to create virtual environment");

        // Under venv, prefix refers to the venv prefix. The base_prefix
        // does not change, and always points to the base Python installation
        // https://docs.python.org/3/library/sys.html#sys.base_prefix
        let venv_python = get_venv_python_path(&workspace_path);
        let output = std::process::Command::new(&venv_python)
            .args(["-c", "import sys; assert sys.prefix != sys.base_prefix"])
            .output()
            .expect("Failed to assert Python prefix");

        assert!(output.status.success());
    }
}
