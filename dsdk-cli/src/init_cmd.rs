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

use crate::cli::DocsCommand;
use crate::install_cmd::{
    ensure_docs_dependencies, install_prerequisites, install_python_packages_from_file,
    is_container_environment,
};
use crate::makefile::generate_makefile_content;
use crate::update_cmd::{
    update_mirror_repos, update_workspace_repos_no_mirror_with_result,
    update_workspace_repos_with_result,
};
use crate::version::spawn_version_check;
use dsdk_cli::config::SdkConfigCore;
use dsdk_cli::download::{copy_yaml_files_to_workspace, process_copy_files};
use dsdk_cli::workspace::{
    create_workspace_marker, download_config_from_url, expand_config_mirror_path, expand_env_vars,
    expand_manifest_vars, get_current_workspace, get_default_source, is_url,
    load_config_with_user_overrides, resolve_target_config_from_git, resolve_variables,
    CreateWorkspaceMarkerParams,
};
use dsdk_cli::{
    config, doc_manager, git_operations, messages, toolchain_manager, vscode_tasks_manager,
};
use regex::Regex;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Handle documentation commands
pub(crate) fn handle_docs_command(docs_command: &DocsCommand) {
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
pub(crate) fn handle_add_command(name: &str, url: &str, commit: &str) {
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
pub(crate) fn handle_foreach_command(command: &str, match_pattern: Option<&str>) {
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
pub(crate) fn resolve_target_config(
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
pub(crate) fn list_available_targets(config_root: &Path) -> Result<Vec<String>, anyhow::Error> {
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
pub(crate) fn install_os_deps_if_available(
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
pub(crate) fn install_toolchains_if_available(
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
pub(crate) fn install_pip_packages_if_available(
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
pub(crate) struct InitConfig<'a> {
    pub(crate) target: String,
    pub(crate) source: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) no_mirror: bool,
    pub(crate) force: bool,
    pub(crate) match_pattern: Option<&'a str>,
    pub(crate) verbose: bool,
    pub(crate) install: bool,
    pub(crate) full: bool,
    pub(crate) no_sudo: bool,
    pub(crate) symlink: bool,
    pub(crate) yes: bool,
    pub(crate) _cert_validation: Option<&'a str>,
}

/// Initialize a new workspace
pub(crate) fn handle_init_command(config: InitConfig) {
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

    // Expand environment variables in git repository URLs
    for git in &mut sdk_config.gits {
        let expanded = expand_env_vars(&git.url);
        if expanded != git.url {
            messages::verbose(&format!("Expanded git URL: {} -> {}", git.url, expanded));
            git.url = expanded;
        }
    }

    // Expand manifest ${{ VAR }} variables in path/URL fields
    if let Some(raw_vars) = sdk_config.variables.clone() {
        let vars = resolve_variables(&raw_vars);

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
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
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
    crate::version::print_update_notice(version_check);
}

/// Temporary struct to hold filtered git configurations for operations
pub(crate) struct FilteredSdkConfig {
    pub(crate) gits: Vec<config::GitConfig>,
    pub(crate) mirror: PathBuf,
    pub(crate) makefile_include: Option<Vec<String>>,
    pub(crate) envsetup: Option<config::SdkTarget>,
    pub(crate) test: Option<config::SdkTarget>,
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

    fn variables(&self) -> &Option<std::collections::HashMap<String, String>> {
        &None
    }
}

/// Filter git configurations based on regex pattern
pub(crate) fn filter_git_configs(
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
pub(crate) fn create_filtered_sdk_config<T: config::SdkConfigCore>(
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
pub(crate) fn list_targets_from_source(source_path: &str) -> Result<Vec<String>, anyhow::Error> {
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
pub(crate) fn list_targets_from_git_repo(git_url: &str) -> Result<Vec<String>, anyhow::Error> {
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
pub(crate) fn list_target_versions(
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
pub(crate) fn is_branch_reference(repo_path: &Path, commit_ref: &str) -> bool {
    git_operations::is_branch_reference(repo_path, commit_ref)
}

/// Get the latest commit hash for a branch
pub(crate) fn get_latest_commit_for_branch(repo_path: &Path, branch_name: &str) -> Option<String> {
    git_operations::get_latest_commit_for_branch(repo_path, branch_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dsdk_cli::git_operations;
    use std::fs;
    use tempfile::TempDir;

    // Test helper function to create a temporary workspace
    fn create_test_workspace() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let workspace_path = temp_dir.path().to_path_buf();
        (temp_dir, workspace_path)
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
