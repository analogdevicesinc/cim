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

use crate::cli::{Cli, DockerCommand};
use crate::init_cmd::{
    compile_match_regex, create_filtered_sdk_config, get_latest_commit_for_branch,
    is_branch_reference, list_target_versions, list_targets_from_source, setup_direnv,
    source_label,
};
use crate::version::{print_update_notice, spawn_version_check};
use clap::CommandFactory;
use dsdk_cli::config::SdkConfigCore;
use dsdk_cli::workspace::{
    expand_config_mirror_path, get_all_sources, require_workspace_config, WorkspaceMarker,
    WORKSPACE_MARKER_FILE,
};
use dsdk_cli::{config, docker_manager, git_operations, messages};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use threadpool::ThreadPool;

/// Configuration command options
pub(crate) struct ConfigOptions<'a> {
    pub(crate) list: bool,
    pub(crate) get: Option<&'a str>,
    pub(crate) show_path: bool,
    pub(crate) template: bool,
    pub(crate) create: bool,
    pub(crate) force: bool,
    pub(crate) edit: bool,
    pub(crate) validate: bool,
}

/// Handle the config command
pub(crate) fn handle_config_command(opts: ConfigOptions) {
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
pub(crate) fn handle_list_targets_command(source: Option<&str>, target_filter: Option<&str>) {
    // If --source is explicitly given, use only that single source (no alternates)
    if let Some(src) = source {
        let source_path = src.to_string();
        if let Some(target_name) = target_filter {
            match list_target_versions(&source_path, target_name) {
                Ok(versions) => {
                    if versions.is_empty() {
                        messages::status(&format!(
                            "No versions found for target '{}'",
                            target_name
                        ));
                    } else {
                        messages::status(&format!(
                            "Available versions for target '{}' from {}:",
                            target_name, source_path
                        ));
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
            match list_targets_from_source(&source_path) {
                Ok(targets) => {
                    if targets.is_empty() {
                        messages::status(&format!("No targets found in {}", source_path));
                    } else {
                        messages::status(&format!("Available targets from {}:", source_path));
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
        return;
    }

    // No --source: use all configured sources (default + alternates)
    let sources = get_all_sources();
    let has_alternates = sources.len() > 1;

    if let Some(target_name) = target_filter {
        let mut any_versions = false;
        for (i, source_path) in sources.iter().enumerate() {
            match list_target_versions(source_path, target_name) {
                Ok(versions) => {
                    if !versions.is_empty() {
                        any_versions = true;
                        if has_alternates {
                            messages::status(&format!(
                                "  Source: {} ({})",
                                source_path,
                                source_label(i)
                            ));
                        } else {
                            messages::status(&format!("  Source: {}", source_path));
                        }
                        for version in versions {
                            messages::status(&format!("    - {}", version));
                        }
                    }
                }
                Err(e) => {
                    if has_alternates {
                        messages::verbose(&format!("  Skipping {} (error: {})", source_path, e));
                    } else {
                        messages::error(&format!(
                            "Error listing versions for target '{}': {}",
                            target_name, e
                        ));
                        std::process::exit(1);
                    }
                }
            }
        }
        if !any_versions {
            messages::status(&format!("No versions found for target '{}'", target_name));
        }
    } else {
        let mut any_targets = false;
        for (i, source_path) in sources.iter().enumerate() {
            match list_targets_from_source(source_path) {
                Ok(targets) => {
                    if !targets.is_empty() {
                        any_targets = true;
                        if has_alternates {
                            messages::status(&format!(
                                "  Source: {} ({})",
                                source_path,
                                source_label(i)
                            ));
                        } else {
                            messages::status(&format!("Available targets from {}:", source_path));
                        }
                        for target in targets {
                            messages::status(&format!("    - {}", target));
                        }
                    } else if !has_alternates {
                        messages::status(&format!("No targets found in {}", source_path));
                    }
                }
                Err(e) => {
                    if has_alternates {
                        messages::verbose(&format!("  Skipping {} (error: {})", source_path, e));
                    } else {
                        messages::error(&format!("Error listing targets: {}", e));
                        std::process::exit(1);
                    }
                }
            }
        }
        if !any_targets {
            messages::status("No targets found in any configured manifest source");
        }
    }
}

/// Update all git repositories in the mirror and workspace
///
/// Supports environment variable expansion in mirror paths (e.g., $HOME, ${HOME})
pub(crate) fn handle_update_command(
    no_mirror: bool,
    match_pattern: Option<&str>,
    all: bool,
    verbose: bool,
    _cert_validation: Option<&str>,
) {
    // Start background version check so it runs concurrently with the update
    let version_check = spawn_version_check();

    // Set verbose mode for this command
    messages::set_verbose(verbose);

    let (workspace_path, config_path) = match require_workspace_config() {
        Ok(paths) => paths,
        Err(e) => {
            messages::error(&e);
            return;
        }
    };

    messages::workspace(&workspace_path);

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

    // Determine match pattern: --all disables filtering, CLI --match takes
    // precedence over stored workspace marker pattern.
    let effective_match_pattern: Option<String> = if all {
        messages::status("Updating all repositories (--all flag, ignoring stored match filter)");
        None
    } else if match_pattern.is_some() {
        match_pattern.map(|s| s.to_string())
    } else {
        // Read stored match pattern from workspace marker
        let marker_path = workspace_path.join(WORKSPACE_MARKER_FILE);
        if marker_path.exists() {
            match fs::read_to_string(&marker_path) {
                Ok(content) => serde_yaml::from_str::<WorkspaceMarker>(&content)
                    .ok()
                    .and_then(|m| m.match_pattern),
                Err(_) => None,
            }
        } else {
            None
        }
    };

    // Compile regex pattern if provided
    let match_regex = if let Some(ref pattern) = effective_match_pattern {
        match compile_match_regex(pattern) {
            Ok(regex) => {
                let source = if match_pattern.is_some() {
                    "CLI"
                } else {
                    "workspace marker"
                };
                messages::status(&format!(
                    "Filtering repositories with pattern: {} (from {})",
                    pattern, source
                ));
                Some(regex)
            }
            Err(e) => {
                messages::error(&e);
                return;
            }
        }
    } else {
        None
    };

    // Create filtered config based on match pattern
    let filtered_config = create_filtered_sdk_config(&sdk_config, &match_regex);

    // Read workspace marker to get stored no_mirror preference
    let marker_path = workspace_path.join(WORKSPACE_MARKER_FILE);
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
        update_workspace_repos(&filtered_config, &workspace_path, false, true);
    } else {
        // Update mirror repositories in parallel
        update_mirror_repos(&filtered_config);

        // Update workspace repositories (single-threaded to avoid conflicts)
        update_workspace_repos(&filtered_config, &workspace_path, false, false);
    }

    // When --all is used, clear the stored match pattern from the workspace
    // marker so subsequent updates without --all still update everything.
    if all {
        use dsdk_cli::workspace::update_workspace_marker_match_pattern;
        if let Err(e) = update_workspace_marker_match_pattern(&workspace_path, None) {
            messages::info(&format!(
                "Note: failed to clear match pattern from workspace marker: {}",
                e
            ));
        }
    }

    // Idempotently set up direnv if configured and .envrc is not yet present.
    if let Some(direnv_cfg) = sdk_config.direnv() {
        if direnv_cfg.used && !workspace_path.join(".envrc").exists() {
            if let Err(e) = setup_direnv(&workspace_path, direnv_cfg, false) {
                messages::info(&format!("Note: direnv setup encountered an issue: {}", e));
            }
        }
    }

    // Print any available update notice after the main work is done
    print_update_notice(version_check);
}

/// Result type for mirror operations
#[derive(Debug, Clone)]
pub(crate) enum MirrorOperationResult {
    Updated, // Existing mirror was updated
    Cloned,  // New mirror was cloned
    Failed,  // Operation failed
}

/// Update mirror repositories in parallel
pub(crate) fn update_mirror_repos<T: config::SdkConfigCore>(sdk_config: &T) {
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
pub(crate) fn update_workspace_repos<T: config::SdkConfigCore>(
    sdk_config: &T,
    workspace_path: &Path,
    is_init: bool,
    no_mirror: bool,
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

    let mirror_path = (!no_mirror).then(|| sdk_config.mirror().clone());

    for tier in &tiers {
        let pool = ThreadPool::new(4);

        for git_cfg in tier {
            let git_cfg = git_cfg.clone();
            let workspace_path = workspace_path.to_path_buf();
            let mirror_path = mirror_path.clone();

            pool.execute(move || {
                let mirror_path = mirror_path.as_deref();
                let repo_workspace_path = workspace_path.join(&git_cfg.name);

                let success = if repo_workspace_path.join(".git").is_dir() {
                    handle_existing_workspace_repo(&git_cfg, &repo_workspace_path, mirror_path)
                } else {
                    clone_repo_to_workspace(&git_cfg, &repo_workspace_path, mirror_path)
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
pub(crate) fn update_workspace_repos_with_result<T: config::SdkConfigCore>(
    sdk_config: &T,
    workspace_path: &Path,
    is_init: bool,
    no_mirror: bool,
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

    let mirror_path = (!no_mirror).then(|| sdk_config.mirror().clone());

    for tier in &tiers {
        let pool = ThreadPool::new(4);

        for git_cfg in tier {
            let git_cfg = git_cfg.clone();
            let workspace_path = workspace_path.to_path_buf();
            let mirror_path = mirror_path.clone();
            let any_failed = Arc::clone(&any_failed);

            pool.execute(move || {
                let repo_workspace_path = workspace_path.join(&git_cfg.name);

                let success = if repo_workspace_path.join(".git").is_dir() {
                    handle_existing_workspace_repo(
                        &git_cfg,
                        &repo_workspace_path,
                        mirror_path.as_deref(),
                    )
                } else {
                    clone_repo_to_workspace(&git_cfg, &repo_workspace_path, mirror_path.as_deref())
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
pub(crate) fn handle_existing_workspace_repo(
    git_cfg: &config::GitConfig,
    repo_path: &Path,
    mirror_path: Option<&Path>,
) -> bool {
    let refs = git_operations::ls_remote(&git_cfg.url, true, true).unwrap_or_default();
    let (fetch_refspec, update_ref_name, sha) =
        git_operations::resolve_fetch_refspec(&refs, &git_cfg.commit);
    let target = sha.unwrap_or_else(|| git_cfg.commit.clone());

    let success = if let Some(mirror_path) = mirror_path {
        let mirror_repo_path =
            dsdk_cli::git_manager::get_mirror_repo_path(mirror_path, &git_cfg.name, &git_cfg.url);

        let _ = git_operations::remote_set_url(repo_path, "origin", &git_cfg.url);
        let _ = git_operations::remote_add(
            repo_path,
            "mirror",
            &git_operations::path_to_file_url(&mirror_repo_path),
        );

        if mirror_repo_path.exists() {
            if !git_operations::cat_file(&mirror_repo_path, &target) {
                let fetch_ok =
                    git_operations::fetch_ref(&mirror_repo_path, "origin", &fetch_refspec, Some(1))
                        .is_ok_and(|r| r.is_success());
                if fetch_ok {
                    let _ = git_operations::update_ref(
                        &mirror_repo_path,
                        &update_ref_name,
                        "FETCH_HEAD",
                    );
                }
            }
            // Local mirror: no depth limit — objects are on disk, no network cost,
            // and shallow fetches would write a .git/shallow file that cuts history.
            git_operations::fetch_ref(repo_path, "mirror", &fetch_refspec, None)
                .is_ok_and(|r| r.is_success())
        } else {
            git_operations::fetch_ref(repo_path, "origin", &fetch_refspec, Some(1))
                .is_ok_and(|r| r.is_success())
        }
    } else {
        git_operations::fetch_ref(repo_path, "origin", &fetch_refspec, Some(1))
            .is_ok_and(|r| r.is_success())
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
        messages::error(&format!("{} (fetch failed)", git_cfg.name));
        false
    }
}

/// Clone a repository to the workspace
pub(crate) fn clone_repo_to_workspace(
    git_cfg: &config::GitConfig,
    repo_path: &Path,
    mirror_path: Option<&Path>,
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

    if let Some(mirror_path) = mirror_path {
        let mirror_repo_path =
            dsdk_cli::git_manager::get_mirror_repo_path(mirror_path, &git_cfg.name, &git_cfg.url);

        if mirror_repo_path.exists() {
            let refs = git_operations::ls_remote(&git_cfg.url, true, true).unwrap_or_default();
            let (fetch_refspec, update_ref_name, sha) =
                git_operations::resolve_fetch_refspec(&refs, &git_cfg.commit);
            let target_sha = sha.unwrap_or_else(|| git_cfg.commit.clone());

            // Ensure the commit is present in the mirror, fetching it if needed
            if !git_operations::cat_file(&mirror_repo_path, &target_sha) {
                let fetch_ok =
                    git_operations::fetch_ref(&mirror_repo_path, "origin", &fetch_refspec, Some(1))
                        .is_ok_and(|r| r.is_success());
                if fetch_ok {
                    let _ = git_operations::update_ref(
                        &mirror_repo_path,
                        &update_ref_name,
                        "FETCH_HEAD",
                    );
                } else {
                    messages::error(&format!(
                        "{} (commit {} not found in remote — check the commit hash in sdk.yml)",
                        git_cfg.name, target_sha
                    ));
                    return false;
                }
            }

            let mirror_url = git_operations::path_to_file_url(&mirror_repo_path);
            let result =
                git_operations::clone_repo(&mirror_url, repo_path, Some(&mirror_repo_path));
            match result {
                Ok(result) if result.is_success() => {
                    // Set origin to upstream and add mirror remote
                    let _ = git_operations::remote_set_url(repo_path, "origin", &git_cfg.url);
                    let _ = git_operations::remote_add(
                        repo_path,
                        "mirror",
                        &git_operations::path_to_file_url(&mirror_repo_path),
                    );
                    return checkout_commit(git_cfg, repo_path);
                }
                _ => {
                    messages::error(&format!("{} (clone failed)", git_cfg.name));
                    return false;
                }
            }
        }
    }

    // Direct clone (no mirror, or mirror path not yet on disk)
    let result = git_operations::clone_repo(&git_cfg.url, repo_path, None);
    match result {
        Ok(result) if result.is_success() => checkout_commit(git_cfg, repo_path),
        _ => {
            messages::error(&format!("{} (clone failed)", git_cfg.name));
            false
        }
    }
}

/// Checkout the specified commit for a repository
pub(crate) fn checkout_commit(git_cfg: &config::GitConfig, repo_path: &Path) -> bool {
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

/// Handle Docker commands
pub(crate) fn handle_docker_command(docker_command: &DockerCommand) {
    match docker_command {
        DockerCommand::Create {
            target,
            source,
            version,
            distro,
            output,
            force,
        } => {
            let distro_str = distro.as_deref().unwrap_or("ubuntu:22.04");

            let config = docker_manager::DockerfileConfig {
                target,
                version: version.as_deref(),
                source: source.as_deref(),
                distro: distro_str,
                output_path: output,
                force: *force,
            };

            match docker_manager::create_dockerfile(&config) {
                Ok(()) => {
                    messages::success(&format!("Dockerfile created at '{}'", output.display()));
                    messages::status("");
                    messages::status("To build the Docker image:");
                    messages::status(&format!(
                        "  docker build -t sdk-image -f {} .",
                        output.display()
                    ));
                    messages::status("");
                    messages::status("To run the image:");
                    messages::status("  docker run -it sdk-image");
                }
                Err(e) => {
                    messages::error(&format!("{}", e));
                }
            }
        }
    }
}
