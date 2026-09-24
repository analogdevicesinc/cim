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

use crate::init_cmd::{
    handle_init_command, list_target_versions, list_targets_from_source, resolve_workspace_path,
    InitConfig,
};
use dsdk_cli::config::{self, SdkConfigCore};
use dsdk_cli::messages;
use dsdk_cli::workspace::{get_all_sources_from_config, load_config_with_extends};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

/// Configuration for the bootstrap command
pub(crate) struct BootstrapConfig {
    pub(crate) target: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) no_mirror: bool,
    pub(crate) mirror: Option<PathBuf>,
    pub(crate) match_pattern: Option<String>,
    pub(crate) include_group: Option<String>,
    pub(crate) exclude_group: Option<String>,
    pub(crate) verbose: bool,
    pub(crate) yes: bool,
    pub(crate) cert_validation: Option<String>,
}

/// A target found while searching manifest sources, paired with the exact
/// source it was found under (a target of the same name can differ between
/// sources, so the pairing must be kept, not just the bare name).
struct TargetChoice {
    target: String,
    source: String,
}

/// Pick (or accept), initialize, and build a target in one go: replaces the
/// former standalone build-cim script. `--target`/`--version` are honored
/// verbatim when given; otherwise an interactive picker is shown when stdin
/// is a TTY, or an error is raised when it isn't (CI).
pub(crate) fn handle_bootstrap_command(cfg: BootstrapConfig) {
    messages::set_verbose(cfg.verbose);

    let user_config = match config::UserConfig::load() {
        Ok(uc) => uc,
        Err(e) => {
            messages::info(&format!("Warning: Failed to load user config: {}", e));
            None
        }
    };

    let interactive = io::stdin().is_terminal();

    let sources: Vec<String> = match &cfg.source {
        Some(src) => vec![src.clone()],
        None => get_all_sources_from_config(user_config.as_ref()),
    };

    let (target, resolved_source) = match &cfg.target {
        Some(t) => (t.clone(), cfg.source.clone()),
        None => {
            if !interactive {
                messages::error(
                    "--target is required when not running interactively (no TTY detected)",
                );
                messages::status("Use 'cim list-targets' to see available targets");
                std::process::exit(1);
            }
            match pick_target(&sources) {
                Some(choice) => (choice.target, Some(choice.source)),
                None => {
                    messages::error("No targets found in any configured manifest source");
                    std::process::exit(1);
                }
            }
        }
    };

    let version = match &cfg.version {
        Some(v) => Some(v.clone()),
        None if interactive => {
            // Version picker needs a single, concrete source to query. When the target
            // came from an explicit --target with no --source, fall back to the default
            // version resolution used by `cim init` instead of guessing a source here.
            resolved_source
                .as_deref()
                .and_then(|source| pick_version(&target, source))
        }
        None => None,
    };

    let workspace_path =
        resolve_workspace_path(cfg.workspace.clone(), &target, user_config.as_ref());

    let force = user_config
        .as_ref()
        .and_then(|uc| uc.bootstrap.force)
        .unwrap_or(false);
    let symlink = user_config
        .as_ref()
        .and_then(|uc| uc.bootstrap.symlink)
        .unwrap_or(false);

    handle_init_command(InitConfig {
        target: target.clone(),
        source: resolved_source,
        version,
        workspace: Some(workspace_path.clone()),
        no_mirror: cfg.no_mirror,
        mirror: cfg.mirror.clone(),
        force,
        match_pattern: cfg.match_pattern.as_deref(),
        include_group: cfg.include_group.as_deref(),
        exclude_group: cfg.exclude_group.as_deref(),
        verbose: cfg.verbose,
        install: true,
        full: false,
        no_sudo: false,
        symlink,
        yes: cfg.yes,
        _cert_validation: cfg.cert_validation.as_deref(),
    });

    // handle_init_command doesn't return a success/failure signal (some of its error
    // paths exit the process directly, others just print and return), so the presence
    // of a generated Makefile is used as the proxy for "the workspace is buildable".
    let makefile_path = workspace_path.join("Makefile");
    if !makefile_path.exists() {
        messages::error("Bootstrap failed: workspace was not fully created (see errors above)");
        std::process::exit(1);
    }

    let phases = user_config
        .as_ref()
        .and_then(|uc| uc.bootstrap.phases.clone())
        .unwrap_or_else(config::default_bootstrap_phases);

    if phases.is_empty() {
        messages::status("");
        messages::success(&format!("Workspace ready at {}", workspace_path.display()));
        return;
    }

    let sdk_config = match load_config_with_extends(&workspace_path.join("sdk.yml")) {
        Ok(c) => c,
        Err(e) => {
            messages::error(&format!(
                "Bootstrap failed: could not reload workspace config: {}",
                e
            ));
            std::process::exit(1);
        }
    };
    let known_phases = sdk_config.phases();
    for phase in &phases {
        if !known_phases.contains(phase) {
            messages::error(&format!(
                "Unknown bootstrap phase '{}': target '{}' does not declare it (available: {})",
                phase,
                target,
                known_phases.join(", ")
            ));
            std::process::exit(1);
        }
    }

    let jobs = user_config
        .as_ref()
        .and_then(|uc| uc.bootstrap.jobs)
        .filter(|&j| j > 0)
        .map(|j| j as usize)
        .unwrap_or_else(config::default_bootstrap_jobs);

    for phase in &phases {
        messages::status("");
        messages::status(&format!("Running make sdk-{} (-j{})...", phase, jobs));
        let status = std::process::Command::new("make")
            .arg(format!("sdk-{}", phase))
            .arg(format!("-j{}", jobs))
            .current_dir(&workspace_path)
            .status();
        match status {
            Ok(s) if s.success() => {
                messages::success(&format!("sdk-{} completed", phase));
            }
            Ok(s) => {
                messages::error(&format!("sdk-{} failed (exit code {:?})", phase, s.code()));
                std::process::exit(s.code().unwrap_or(1));
            }
            Err(e) => {
                messages::error(&format!("Failed to run make sdk-{}: {}", phase, e));
                std::process::exit(1);
            }
        }
    }

    messages::status("");
    messages::success(&format!("Bootstrap completed for target '{}'", target));
    messages::status(&format!("Workspace: {}", workspace_path.display()));
}

/// Show a numbered list of every target across every configured manifest source and
/// prompt for a selection. Waits for real input (no timeout) since this path is only
/// reached when stdin is already known to be a TTY. Returns `None` if no source has
/// any targets.
fn pick_target(sources: &[String]) -> Option<TargetChoice> {
    let mut choices: Vec<TargetChoice> = Vec::new();

    println!("Available targets:");
    println!();
    for source in sources {
        match list_targets_from_source(source) {
            Ok(targets) if !targets.is_empty() => {
                println!("  Source: {}", source);
                for target in targets {
                    println!("    {}) {}", choices.len(), target);
                    choices.push(TargetChoice {
                        target,
                        source: source.clone(),
                    });
                }
            }
            Ok(_) => {}
            Err(e) => messages::verbose(&format!("  Skipping {} (error: {})", source, e)),
        }
    }
    println!();

    if choices.is_empty() {
        return None;
    }

    let max = choices.len() - 1;
    let index = prompt_index(
        &format!("Please select a target [0-{}] (default 0): ", max),
        max,
    );
    Some(choices.remove(index))
}

/// Show a numbered list of a target's available versions and prompt for a selection.
/// Index 0 always means "use the default version" (`None`), matching `cim init`'s own
/// behavior when `--version` is omitted, so picking it never has to duplicate that
/// default-resolution logic here.
fn pick_version(target: &str, source: &str) -> Option<String> {
    let versions = match list_target_versions(source, target) {
        Ok(v) => v,
        Err(e) => {
            messages::verbose(&format!("Could not list versions for '{}': {}", target, e));
            return None;
        }
    };
    if versions.is_empty() {
        return None;
    }

    println!("Available versions:");
    println!();
    println!(" 0) (default)");
    for (i, version) in versions.iter().enumerate() {
        println!(" {}) {}", i + 1, version);
    }
    println!();

    let max = versions.len();
    let index = prompt_index(
        &format!("Please select a version [0-{}] (default 0): ", max),
        max,
    );
    if index == 0 {
        None
    } else {
        Some(versions[index - 1].clone())
    }
}

/// Prompt on stdout and read a single index from stdin, clamped to `[0, max]`.
/// Empty or unparsable input falls back to 0, matching the previous standalone
/// build-cim script's behavior when nothing usable is entered.
fn prompt_index(prompt: &str, max: usize) -> usize {
    print!("{}", prompt);
    let _ = io::stdout().flush();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return 0;
    }
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return 0;
    }
    match trimmed.parse::<usize>() {
        Ok(n) if n <= max => n,
        _ => {
            messages::status(&format!(
                "Invalid selection '{}', using default (0).",
                trimmed
            ));
            0
        }
    }
}
