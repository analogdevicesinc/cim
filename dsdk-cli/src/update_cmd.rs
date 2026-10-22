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
    get_latest_commit_for_branch_with_remote, is_branch_reference, list_target_versions,
    list_targets_from_source, parse_group_list, setup_direnv, source_label,
};
use crate::version::{print_update_notice, spawn_version_check};
use clap::CommandFactory;
use dsdk_cli::config::SdkConfigCore;
use dsdk_cli::workspace::{
    get_all_sources, load_config_with_extends, require_workspace_config, resolve_mirror,
    WorkspaceMarker, WORKSPACE_MARKER_FILE,
};
use dsdk_cli::{config, docker_manager, git_operations, messages};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
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

    if opts.template {
        print!("{}", config::UserConfig::generate_template());
        return;
    }
    if opts.validate {
        handle_config_validate(&config_path);
        return;
    }
    if opts.create {
        handle_config_create(&config_path, opts.force);
        return;
    }
    if opts.edit {
        handle_config_edit(&config_path);
        return;
    }
    if opts.show_path {
        messages::status(&config_path.display().to_string());
        return;
    }
    if opts.list {
        handle_config_list(&config_path);
        return;
    }
    if let Some(key) = opts.get {
        handle_config_get(key);
        return;
    }

    // If no flag specified, show help
    let mut cmd = Cli::command();
    if let Some(config_cmd) = cmd.find_subcommand_mut("config") {
        let _ = config_cmd.print_help();
        std::process::exit(0);
    }
}

/// Validate that the config file exists and can be loaded (`cim config --validate`)
fn handle_config_validate(config_path: &Path) {
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
}

/// Create the config file from the built-in template (`cim config --create`)
fn handle_config_create(config_path: &Path, force: bool) {
    if config_path.exists() && !force {
        messages::error(&format!(
            "Config file already exists: {}",
            config_path.display()
        ));
        messages::info("Use --force to overwrite.");
        std::process::exit(1);
    }

    match create_config_file(config_path) {
        Ok(_) => {
            messages::success(&format!("Created config file: {}", config_path.display()));
            messages::info("Edit this file to customize cim settings.");
        }
        Err(e) => {
            messages::error(&format!("Failed to create config: {}", e));
            std::process::exit(1);
        }
    }
}

/// Open the config file in $EDITOR, creating it from template first if missing
/// (`cim config --edit`)
fn handle_config_edit(config_path: &Path) {
    // Create from template if doesn't exist
    if !config_path.exists() {
        if let Err(e) = create_config_file(config_path) {
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
        .arg(config_path)
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
}

/// List all configuration values (`cim config --list`)
fn handle_config_list(config_path: &Path) {
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
}

/// Print a single configuration value (`cim config --get <key>`)
fn handle_config_get(key: &str) {
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

/// One manifest source and the targets it exposes, for `--format json`.
#[derive(serde::Serialize)]
struct JsonSourceTargets {
    source: String,
    kind: &'static str,
    targets: Vec<String>,
}

/// A single target's available versions, for `--format json`.
#[derive(serde::Serialize)]
struct JsonTargetVersions {
    target: String,
    source: String,
    versions: Vec<String>,
}

/// Handle the list-targets command
pub(crate) fn handle_list_targets_command(
    source: Option<&str>,
    target_filter: Option<&str>,
    format: &str,
) {
    let json = format.eq_ignore_ascii_case("json");

    // If --source is explicitly given, use only that single source (no alternates)
    if let Some(src) = source {
        list_targets_single_source(src, target_filter, json);
        return;
    }

    // No --source: use all configured sources (default + alternates)
    let sources = get_all_sources();
    let has_alternates = sources.len() > 1;

    if let Some(target_name) = target_filter {
        list_versions_all_sources(target_name, &sources, has_alternates, json);
    } else {
        list_targets_all_sources(&sources, has_alternates, json);
    }
}

/// `cim list-targets --source <src>` (with or without `--target`): a single explicit
/// source, no alternates to fall back to.
fn list_targets_single_source(src: &str, target_filter: Option<&str>, json: bool) {
    let source_path = src.to_string();
    if let Some(target_name) = target_filter {
        match list_target_versions(&source_path, target_name) {
            Ok(versions) => {
                if json {
                    print_json(&JsonTargetVersions {
                        target: target_name.to_string(),
                        source: source_path,
                        versions,
                    });
                } else if versions.is_empty() {
                    messages::status(&format!("No versions found for target '{}'", target_name));
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
        return;
    }

    match list_targets_from_source(&source_path) {
        Ok(targets) => {
            if json {
                print_json(&JsonSourceTargets {
                    source: source_path,
                    kind: "default",
                    targets,
                });
            } else if targets.is_empty() {
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

/// `cim list-targets --target <name>` across all configured sources (no `--source`).
fn list_versions_all_sources(
    target_name: &str,
    sources: &[String],
    has_alternates: bool,
    json: bool,
) {
    let mut any_versions = false;
    let mut json_results: Vec<JsonTargetVersions> = Vec::new();
    for (i, source_path) in sources.iter().enumerate() {
        let versions = match list_target_versions(source_path, target_name) {
            Ok(versions) => versions,
            Err(e) if has_alternates => {
                messages::verbose(&format!("  Skipping {} (error: {})", source_path, e));
                continue;
            }
            Err(e) => {
                messages::error(&format!(
                    "Error listing versions for target '{}': {}",
                    target_name, e
                ));
                std::process::exit(1);
            }
        };
        if versions.is_empty() {
            continue;
        }
        any_versions = true;
        if json {
            json_results.push(JsonTargetVersions {
                target: target_name.to_string(),
                source: source_path.clone(),
                versions,
            });
        } else if has_alternates {
            messages::status(&format!("  Source: {} ({})", source_path, source_label(i)));
            for version in versions {
                messages::status(&format!("    - {}", version));
            }
        } else {
            messages::status(&format!("  Source: {}", source_path));
            for version in versions {
                messages::status(&format!("    - {}", version));
            }
        }
    }
    if json {
        print_json(&json_results);
    } else if !any_versions {
        messages::status(&format!("No versions found for target '{}'", target_name));
    }
}

/// `cim list-targets` (no `--target`, no `--source`): list targets from every
/// configured source.
fn list_targets_all_sources(sources: &[String], has_alternates: bool, json: bool) {
    let mut any_targets = false;
    let mut json_results: Vec<JsonSourceTargets> = Vec::new();
    for (i, source_path) in sources.iter().enumerate() {
        let targets = match list_targets_from_source(source_path) {
            Ok(targets) => targets,
            Err(e) if has_alternates => {
                messages::verbose(&format!("  Skipping {} (error: {})", source_path, e));
                continue;
            }
            Err(e) => {
                messages::error(&format!("Error listing targets: {}", e));
                std::process::exit(1);
            }
        };
        if targets.is_empty() {
            if !has_alternates && !json {
                messages::status(&format!("No targets found in {}", source_path));
            }
            continue;
        }
        any_targets = true;
        if json {
            json_results.push(JsonSourceTargets {
                source: source_path.clone(),
                kind: source_label(i),
                targets,
            });
        } else if has_alternates {
            messages::status(&format!("  Source: {} ({})", source_path, source_label(i)));
            for target in targets {
                messages::status(&format!("    - {}", target));
            }
        } else {
            messages::status(&format!("Available targets from {}:", source_path));
            for target in targets {
                messages::status(&format!("    - {}", target));
            }
        }
    }
    if json {
        print_json(&json_results);
    } else if !any_targets {
        messages::status("No targets found in any configured manifest source");
    }
}

/// Serialize `value` to pretty JSON on stdout; any serialization failure is a bug, not a
/// runtime condition, so it aborts rather than silently printing nothing.
fn print_json<T: serde::Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("JSON serialization must not fail")
    );
}

/// Update all git repositories in the mirror and workspace
///
/// Supports environment variable expansion in mirror paths (e.g., $HOME, ${HOME})
pub(crate) struct UpdateOptions<'a> {
    pub(crate) no_mirror: bool,
    pub(crate) mirror_override: Option<PathBuf>,
    pub(crate) match_pattern: Option<&'a str>,
    pub(crate) include_group: Option<&'a str>,
    pub(crate) exclude_group: Option<&'a str>,
    pub(crate) all: bool,
    pub(crate) verbose: bool,
    pub(crate) _cert_validation: Option<&'a str>,
}

pub(crate) fn handle_update_command(opts: UpdateOptions) {
    // Start background version check so it runs concurrently with the update
    let version_check = spawn_version_check();

    // Set verbose mode for this command
    messages::set_verbose(opts.verbose);

    let Ok((workspace_path, config_path)) = resolve_update_workspace() else {
        return;
    };

    let Ok((sdk_config, user_config)) = load_update_sdk_config(&config_path, opts.verbose) else {
        return;
    };

    // Resolve the mirror directory: --mirror flag > user config > built-in default.
    let mirror_path = resolve_mirror(opts.mirror_override.as_deref());
    messages::verbose(&format!("Mirror: {}", mirror_path.display()));

    let effective_match_pattern =
        determine_match_pattern(&workspace_path, opts.match_pattern, opts.all);
    let Ok(match_regex) = compile_effective_match_regex(
        effective_match_pattern.as_deref(),
        opts.match_pattern.is_some(),
    ) else {
        return;
    };

    let (include_groups, exclude_groups) = determine_group_filters(
        &workspace_path,
        opts.include_group,
        opts.exclude_group,
        opts.all,
    );

    // Create filtered config based on match pattern and group selection
    let filtered_config =
        create_filtered_sdk_config(&sdk_config, &match_regex, &include_groups, &exclude_groups);

    let skip_mirror = determine_skip_mirror(&workspace_path, opts.no_mirror, user_config.as_ref());
    run_update_sync(&filtered_config, &workspace_path, &mirror_path, skip_mirror);

    // When --all is used, clear the stored match pattern from the workspace
    // marker so subsequent updates without --all still update everything.
    if opts.all {
        clear_stored_update_filters(&workspace_path);
    }

    // Idempotently set up direnv if configured and .envrc is not yet present.
    finalize_direnv_for_update(&sdk_config, &workspace_path);

    // Print any available update notice after the main work is done
    print_update_notice(version_check);
}

/// Resolve the current workspace + config paths, printing the workspace status line.
fn resolve_update_workspace() -> Result<(PathBuf, PathBuf), ()> {
    let (workspace_path, config_path) = require_workspace_config().map_err(|e| {
        messages::error(&e);
    })?;
    messages::workspace(&workspace_path);
    Ok((workspace_path, config_path))
}

/// Load the sdk.yml config and apply any user config overrides on top of it.
fn load_update_sdk_config(
    config_path: &Path,
    verbose: bool,
) -> Result<(config::SdkConfig, Option<config::UserConfig>), ()> {
    let mut sdk_config = load_config_with_extends(config_path).map_err(|e| {
        messages::error(&format!("Failed to load config: {}", e));
    })?;

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

    Ok((sdk_config, user_config))
}

/// Read and parse the `.workspace` marker file, if present and valid. A missing
/// file, unreadable file, or parse error all quietly become `None` — the marker
/// is a best-effort convenience, not required for `update` to function.
fn read_workspace_marker(workspace_path: &Path) -> Option<WorkspaceMarker> {
    let marker_path = workspace_path.join(WORKSPACE_MARKER_FILE);
    if !marker_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&marker_path).ok()?;
    noyalib::from_str::<WorkspaceMarker>(&content).ok()
}

/// Determine the effective `--match` pattern: `--all` disables filtering, CLI
/// `--match` takes precedence over the stored workspace marker pattern.
fn determine_match_pattern(
    workspace_path: &Path,
    match_pattern: Option<&str>,
    all: bool,
) -> Option<String> {
    if all {
        messages::status("Updating all repositories (--all flag, ignoring stored match filter)");
        return None;
    }
    if let Some(pattern) = match_pattern {
        return Some(pattern.to_string());
    }
    read_workspace_marker(workspace_path).and_then(|m| m.match_pattern)
}

/// Compile the effective match pattern into a regex, printing the filtering
/// status line (and its source) on success.
fn compile_effective_match_regex(
    effective_match_pattern: Option<&str>,
    used_cli_pattern: bool,
) -> Result<Option<Regex>, ()> {
    let Some(pattern) = effective_match_pattern else {
        return Ok(None);
    };
    match compile_match_regex(pattern) {
        Ok(regex) => {
            let source = if used_cli_pattern {
                "CLI"
            } else {
                "workspace marker"
            };
            messages::status(&format!(
                "Filtering repositories with pattern: {} (from {})",
                pattern, source
            ));
            Ok(Some(regex))
        }
        Err(e) => {
            messages::error(&e);
            Err(())
        }
    }
}

/// Determine the effective include/exclude group selection: `--all` disables
/// filtering, CLI flags take precedence over the stored workspace marker
/// selection.
fn determine_group_filters(
    workspace_path: &Path,
    include_group: Option<&str>,
    exclude_group: Option<&str>,
    all: bool,
) -> (Vec<String>, Vec<String>) {
    let (effective_include_groups, effective_exclude_groups): (Option<String>, Option<String>) =
        if all {
            (None, None)
        } else if include_group.is_some() || exclude_group.is_some() {
            (
                include_group.map(|s| s.to_string()),
                exclude_group.map(|s| s.to_string()),
            )
        } else {
            let marker = read_workspace_marker(workspace_path);
            (
                marker.as_ref().and_then(|m| m.include_groups.clone()),
                marker.as_ref().and_then(|m| m.exclude_groups.clone()),
            )
        };

    (
        effective_include_groups
            .as_deref()
            .map(parse_group_list)
            .unwrap_or_default(),
        effective_exclude_groups
            .as_deref()
            .map(parse_group_list)
            .unwrap_or_default(),
    )
}

/// Determine whether mirror operations should be skipped, with precedence
/// CLI flag > workspace marker preference > user config > default (false).
/// Prints the resulting status line.
fn determine_skip_mirror(
    workspace_path: &Path,
    no_mirror: bool,
    user_config: Option<&config::UserConfig>,
) -> bool {
    let workspace_no_mirror = read_workspace_marker(workspace_path)
        .and_then(|m| m.no_mirror)
        .unwrap_or(false);

    let (skip_mirror, mirror_source) = if no_mirror {
        (true, "CLI flag --no-mirror")
    } else if workspace_no_mirror {
        (true, "workspace preference")
    } else if user_config
        .and_then(|uc| uc.workspace.no_mirror)
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

    skip_mirror
}

/// Run the actual mirror + workspace repo sync, honoring `skip_mirror`.
fn run_update_sync<T: config::SdkConfigCore>(
    filtered_config: &T,
    workspace_path: &Path,
    mirror_path: &Path,
    skip_mirror: bool,
) {
    if skip_mirror {
        // Update workspace repositories directly from remote URLs
        update_workspace_repos(filtered_config, workspace_path, false, None);
    } else {
        // Update mirror repositories in parallel
        update_mirror_repos(filtered_config, mirror_path);

        // Update workspace repositories (single-threaded to avoid conflicts)
        update_workspace_repos(filtered_config, workspace_path, false, Some(mirror_path));
    }
}

/// After `--all`, clear the stored match/group filters from the workspace
/// marker so a subsequent plain `cim update` still updates everything.
fn clear_stored_update_filters(workspace_path: &Path) {
    use dsdk_cli::workspace::{
        update_workspace_marker_groups, update_workspace_marker_match_pattern,
    };
    if let Err(e) = update_workspace_marker_match_pattern(workspace_path, None) {
        messages::info(&format!(
            "Note: failed to clear match pattern from workspace marker: {}",
            e
        ));
    }
    if let Err(e) = update_workspace_marker_groups(workspace_path, None, None) {
        messages::info(&format!(
            "Note: failed to clear group filter from workspace marker: {}",
            e
        ));
    }
}

/// Idempotently set up direnv if configured and `.envrc` is not yet present.
fn finalize_direnv_for_update(sdk_config: &config::SdkConfig, workspace_path: &Path) {
    let Some(direnv_cfg) = sdk_config.direnv() else {
        return;
    };
    if !direnv_cfg.used || workspace_path.join(".envrc").exists() {
        return;
    }
    // `WorkspaceMarker` doesn't persist whether `cim init`/`cim install pip`
    // used `--symlink`, so probe the venv's actual on-disk state instead of
    // assuming direct mode: a real `--symlink` workspace's venv path is a
    // symlink into the mirror, and `setup_direnv` needs to know that to
    // write the symlink-aware `.envrc` form.
    let venv_path = workspace_path.join(direnv_cfg.venv_path_or_default());
    let symlink_mode = fs::symlink_metadata(&venv_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if let Err(e) = setup_direnv(workspace_path, direnv_cfg, symlink_mode) {
        messages::info(&format!("Note: direnv setup encountered an issue: {}", e));
    }
}

/// Result type for mirror operations
#[derive(Debug, Clone)]
pub(crate) enum MirrorOperationResult {
    Updated, // Existing mirror was updated
    Cloned,  // New mirror was cloned
    Failed,  // Operation failed
}

/// Update mirror repositories in parallel
pub(crate) fn update_mirror_repos<T: config::SdkConfigCore>(sdk_config: &T, mirror_path: &Path) {
    // Create or update mirror directory
    if !mirror_path.exists() {
        if let Err(e) = std::fs::create_dir_all(mirror_path) {
            messages::error(&format!("Error creating mirror directory: {}", e));
            return;
        }
    }

    messages::status("Updating mirror repositories...");
    let pool = ThreadPool::new(4);

    for git_cfg in sdk_config.gits() {
        let git_cfg = git_cfg.clone();
        let mirror_path = mirror_path.to_path_buf();

        pool.execute(move || {
            let repo_mirror_path = dsdk_cli::git_manager::get_mirror_repo_path(
                &mirror_path,
                &git_cfg.name,
                &git_cfg.url,
            );
            let result = if repo_mirror_path.exists() {
                update_existing_mirror_repo(&repo_mirror_path, &git_cfg)
            } else {
                clone_new_mirror_repo(&repo_mirror_path, &git_cfg)
            };
            report_mirror_result(&git_cfg.name, result);
        });
    }

    pool.join();
}

/// Fetch an existing mirror repo up to date.
///
/// Mirrors are `git clone --mirror`, so the fetch refspec is
/// `+refs/*:refs/*` and the fetch alone moves `refs/heads/<branch>` to the
/// source's branch tip. Nothing must be re-resolved afterwards: a mirror of
/// a non-bare working checkout also holds that checkout's own
/// `refs/remotes/origin/*` tracking refs, which can be behind its branches.
fn update_existing_mirror_repo(
    repo_mirror_path: &Path,
    git_cfg: &config::GitConfig,
) -> MirrorOperationResult {
    messages::progress(&git_cfg.name, "updating remote URL and fetching");

    // Update remote URL in case it changed in the config
    let _ = git_operations::remote_set_url(repo_mirror_path, "origin", &git_cfg.url);

    match git_operations::fetch_all_with_tags(repo_mirror_path) {
        Ok(result) if result.is_success() => MirrorOperationResult::Updated,
        _ => MirrorOperationResult::Failed,
    }
}

/// Clone a brand-new mirror repo and fetch its tags.
fn clone_new_mirror_repo(
    repo_mirror_path: &Path,
    git_cfg: &config::GitConfig,
) -> MirrorOperationResult {
    messages::progress(&git_cfg.name, "cloning new repository");
    let spinner = messages::Spinner::start(&git_cfg.name, "cloning mirror…");
    let clone_result = git_operations::clone_mirror(&git_cfg.url, repo_mirror_path);
    spinner.finish();

    match clone_result {
        Ok(result) if result.is_success() => {
            // Ensure all tags are fetched after initial clone. A tag fetch
            // failure doesn't invalidate the clone - the repository is still
            // functional even without all tags.
            let _ = git_operations::fetch_tags(repo_mirror_path, Some("origin"));
            MirrorOperationResult::Cloned
        }
        _ => {
            // A failed clone can still leave a partial mirror dir
            // on disk (e.g. network drop mid-clone). Remove it so
            // a leftover broken mirror doesn't get treated as a
            // valid, existing one on a later run or in the
            // workspace-clone phase of this same run.
            let _ = std::fs::remove_dir_all(repo_mirror_path);
            MirrorOperationResult::Failed
        }
    }
}

/// Print the immediate per-repo result of a mirror update/clone.
fn report_mirror_result(name: &str, result: MirrorOperationResult) {
    match result {
        MirrorOperationResult::Updated => messages::success(name),
        MirrorOperationResult::Cloned => messages::success(&format!("{} (cloned)", name)),
        MirrorOperationResult::Failed => messages::error(&format!("{} (failed)", name)),
    }
}

/// Update workspace repositories in parallel
pub(crate) fn update_workspace_repos<T: config::SdkConfigCore>(
    sdk_config: &T,
    workspace_path: &Path,
    is_init: bool,
    mirror_path: Option<&Path>,
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

    let mirror_path = mirror_path.map(|p| p.to_path_buf());

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
    mirror_path: Option<&Path>,
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

    let mirror_path = mirror_path.map(|p| p.to_path_buf());

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

    // Track which remote was used to fetch so we can resolve "latest" against
    // the right remote-tracking ref.  When a mirror is in use the workspace
    // fetches from "mirror", so origin/* is stale and mirror/* is current.
    let preferred_remote: Option<&str> = mirror_path.map(|_| "mirror");

    let success = fetch_existing_workspace_repo(
        git_cfg,
        repo_path,
        mirror_path,
        &fetch_refspec,
        &update_ref_name,
        &target,
    );

    if !success {
        messages::error(&format!("{} (fetch failed)", git_cfg.name));
        return false;
    }

    reset_existing_workspace_repo(git_cfg, repo_path, preferred_remote)
}

/// Fetch the latest commits for an already-cloned workspace repo, either via
/// its mirror remote (creating/updating it as needed) or directly from origin.
fn fetch_existing_workspace_repo(
    git_cfg: &config::GitConfig,
    repo_path: &Path,
    mirror_path: Option<&Path>,
    fetch_refspec: &str,
    update_ref_name: &str,
    target: &str,
) -> bool {
    let Some(mirror_path) = mirror_path else {
        return git_operations::fetch_ref(repo_path, "origin", fetch_refspec, Some(1))
            .is_ok_and(|r| r.is_success());
    };

    let mirror_repo_path =
        dsdk_cli::git_manager::get_mirror_repo_path(mirror_path, &git_cfg.name, &git_cfg.url);

    let _ = git_operations::remote_set_url(repo_path, "origin", &git_cfg.url);
    let _ = git_operations::remote_add(
        repo_path,
        "mirror",
        &git_operations::path_to_file_url(&mirror_repo_path),
    );

    if !mirror_repo_path.exists() {
        return git_operations::fetch_ref(repo_path, "origin", fetch_refspec, Some(1))
            .is_ok_and(|r| r.is_success());
    }

    if !git_operations::cat_file(&mirror_repo_path, target) {
        // No depth limit: a shallow fetch here would write a .git/shallow
        // file into the persistent mirror, which later makes `git clone
        // --reference` refuse to use it.
        let fetch_ok = git_operations::fetch_ref(&mirror_repo_path, "origin", fetch_refspec, None)
            .is_ok_and(|r| r.is_success());
        if fetch_ok {
            let _ = git_operations::update_ref(&mirror_repo_path, update_ref_name, "FETCH_HEAD");
        }
    }
    // Local mirror: no depth limit — objects are on disk, no network cost,
    // and shallow fetches would write a .git/shallow file that cuts history.
    //
    // Use a mapping refspec for branches so that git updates the
    // refs/remotes/mirror/<branch> tracking ref in the workspace.
    // Without this, the fetch only writes FETCH_HEAD and
    // get_latest_commit_for_branch_with_remote("mirror") would find nothing.
    let mirror_fetch_refspec = if let Some(branch) = fetch_refspec.strip_prefix("refs/heads/") {
        format!("refs/heads/{}:refs/remotes/mirror/{}", branch, branch)
    } else {
        fetch_refspec.to_string()
    };
    git_operations::fetch_ref(repo_path, "mirror", &mirror_fetch_refspec, None)
        .is_ok_and(|r| r.is_success())
}

/// After a successful fetch, reset an existing workspace repo to the pinned
/// commit/branch/tag — but only if the working tree is clean.
fn reset_existing_workspace_repo(
    git_cfg: &config::GitConfig,
    repo_path: &Path,
    preferred_remote: Option<&str>,
) -> bool {
    match dsdk_cli::git_manager::repo_has_pending_changes(repo_path) {
        Ok(false) => checkout_clean_existing_workspace_repo(git_cfg, repo_path, preferred_remote),
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
}

/// Check out the pinned commit/branch/tag in a clean existing workspace repo.
fn checkout_clean_existing_workspace_repo(
    git_cfg: &config::GitConfig,
    repo_path: &Path,
    preferred_remote: Option<&str>,
) -> bool {
    if !is_branch_reference(repo_path, &git_cfg.commit) {
        // For tags and specific commits, use the exact reference
        return checkout_and_report(
            repo_path,
            &git_cfg.commit,
            format!("{} (pinned to {})", git_cfg.name, git_cfg.commit),
            format!("{} (failed to checkout {})", git_cfg.name, git_cfg.commit),
        );
    }

    // For branches, get the latest commit and checkout that.
    // Pass the preferred remote so we read the freshly-fetched
    // tracking ref (mirror/*) rather than the stale origin/*.
    let Some(latest_commit) =
        get_latest_commit_for_branch_with_remote(repo_path, &git_cfg.commit, preferred_remote)
    else {
        // Fallback to original behavior if we can't get latest
        return checkout_and_report(
            repo_path,
            &git_cfg.commit,
            format!("{} (updated to {})", git_cfg.name, git_cfg.commit),
            format!("{} (failed to checkout {})", git_cfg.name, git_cfg.commit),
        );
    };

    checkout_and_report(
        repo_path,
        &latest_commit,
        format!(
            "{} (updated {} to latest: {})",
            git_cfg.name,
            git_cfg.commit,
            &latest_commit[..8]
        ),
        format!(
            "{} (failed to checkout latest {})",
            git_cfg.name, latest_commit
        ),
    )
}

/// Check out `target_ref` in `repo_path`, printing `success_msg` or `fail_msg`.
fn checkout_and_report(
    repo_path: &Path,
    target_ref: &str,
    success_msg: String,
    fail_msg: String,
) -> bool {
    match git_operations::checkout(repo_path, target_ref) {
        Ok(result) if result.is_success() => {
            messages::success(&success_msg);
            true
        }
        _ => {
            messages::error(&fail_msg);
            false
        }
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
    let spinner = messages::Spinner::start(&git_cfg.name, "cloning…");

    if let Some(mirror_path) = mirror_path {
        let mirror_repo_path =
            dsdk_cli::git_manager::get_mirror_repo_path(mirror_path, &git_cfg.name, &git_cfg.url);

        if mirror_repo_path.exists() {
            return clone_from_mirror(git_cfg, repo_path, &mirror_repo_path, spinner);
        }
    }

    // Direct clone (no mirror, or mirror path not yet on disk)
    clone_directly(git_cfg, repo_path, spinner)
}

/// Clone `repo_path` using the local mirror as a `--reference`, unshallowing
/// the mirror and/or fetching the pinned commit into it first if needed.
fn clone_from_mirror(
    git_cfg: &config::GitConfig,
    repo_path: &Path,
    mirror_repo_path: &Path,
    spinner: messages::Spinner,
) -> bool {
    // git clone --reference refuses shallow repositories; unshallow before cloning
    if mirror_repo_path.join("shallow").exists() {
        messages::progress(&git_cfg.name, "mirror is shallow, unshallowing");
        let ok = git_operations::fetch_unshallow(mirror_repo_path).is_ok_and(|r| r.is_success());
        if !ok {
            messages::error(&format!("{} (failed to unshallow mirror)", git_cfg.name));
            return false;
        }
    }

    spinner.set_action(&git_cfg.name, "resolving refs…");
    let refs = git_operations::ls_remote(&git_cfg.url, true, true).unwrap_or_default();
    let (fetch_refspec, update_ref_name, sha) =
        git_operations::resolve_fetch_refspec(&refs, &git_cfg.commit);
    let target_sha = sha.unwrap_or_else(|| git_cfg.commit.clone());

    // Ensure the commit is present in the mirror, fetching it if needed.
    // No depth limit here: a shallow fetch would write a .git/shallow
    // file into the persistent mirror, which later makes `git clone
    // --reference` refuse to use it (as seen with no-OS).
    if !git_operations::cat_file(mirror_repo_path, &target_sha) {
        spinner.set_action(&git_cfg.name, "fetching into mirror…");
        let fetch_ok = git_operations::fetch_ref(mirror_repo_path, "origin", &fetch_refspec, None)
            .is_ok_and(|r| r.is_success());
        if !fetch_ok {
            spinner.finish();
            messages::error(&format!(
                "{} (commit {} not found in remote — check the commit hash in sdk.yml)",
                git_cfg.name, target_sha
            ));
            return false;
        }
        let _ = git_operations::update_ref(mirror_repo_path, &update_ref_name, "FETCH_HEAD");
    }

    spinner.set_action(&git_cfg.name, "cloning from mirror…");
    let mirror_url = git_operations::path_to_file_url(mirror_repo_path);
    match git_operations::clone_repo(&mirror_url, repo_path, Some(mirror_repo_path)) {
        Ok(result) if result.is_success() => {}
        Ok(result) => {
            spinner.finish();
            messages::error(&format!(
                "{} (clone failed: {})",
                git_cfg.name,
                result.stderr.trim()
            ));
            return false;
        }
        Err(e) => {
            spinner.finish();
            messages::error(&format!("{} (clone failed: {})", git_cfg.name, e));
            return false;
        }
    }

    // Set origin to upstream and add mirror remote
    let _ = git_operations::remote_set_url(repo_path, "origin", &git_cfg.url);
    let _ = git_operations::remote_add(
        repo_path,
        "mirror",
        &git_operations::path_to_file_url(mirror_repo_path),
    );
    spinner.set_action(&git_cfg.name, "checking out…");
    let ok = checkout_commit(git_cfg, repo_path);
    spinner.finish();
    ok
}

/// Clone `repo_path` directly from the upstream URL (no mirror available).
fn clone_directly(
    git_cfg: &config::GitConfig,
    repo_path: &Path,
    spinner: messages::Spinner,
) -> bool {
    match git_operations::clone_repo(&git_cfg.url, repo_path, None) {
        Ok(result) if result.is_success() => {
            spinner.set_action(&git_cfg.name, "checking out…");
            let ok = checkout_commit(git_cfg, repo_path);
            spinner.finish();
            ok
        }
        Ok(result) => {
            spinner.finish();
            messages::error(&format!(
                "{} (clone failed: {})",
                git_cfg.name,
                result.stderr.trim()
            ));
            false
        }
        Err(e) => {
            spinner.finish();
            messages::error(&format!("{} (clone failed: {})", git_cfg.name, e));
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
