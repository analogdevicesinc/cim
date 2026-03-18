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
mod init_cmd;
mod install_cmd;
mod makefile;
mod release_cmd;
mod utils_cmd;
mod version;

use clap::{CommandFactory, Parser};
use cli::{Cli, Commands, DockerCommand};
use dsdk_cli::workspace::{
    expand_config_mirror_path, get_current_workspace, get_default_source, get_docker_temp_dir,
    is_url, resolve_target_config_from_git, WorkspaceMarker,
};
use dsdk_cli::{config, docker_manager, git_operations, messages};
use init_cmd::{
    create_filtered_sdk_config, filter_git_configs, get_latest_commit_for_branch,
    handle_add_command, handle_docs_command, handle_foreach_command, handle_init_command,
    is_branch_reference, list_available_targets, list_target_versions, list_targets_from_source,
    resolve_target_config, InitConfig,
};
use install_cmd::handle_install_command;
use makefile::handle_makefile_command;
use regex::Regex;
use release_cmd::handle_release_command;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use threadpool::ThreadPool;
use utils_cmd::handle_utils_command;
use version::{print_update_notice, print_version_info, spawn_version_check};

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
