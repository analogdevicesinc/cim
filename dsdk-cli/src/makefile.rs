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

use dsdk_cli::workspace::require_workspace_config;
use dsdk_cli::{config, messages, vscode_tasks_manager};

const WORKSPACE_VARIABLE: &str = "WORKSPACE := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))";

/// Generate a Makefile from the SDK configuration
pub(crate) fn handle_makefile_command(no_dividers: bool) {
    let (workspace_path, config_path) = match require_workspace_config() {
        Ok(paths) => paths,
        Err(e) => {
            messages::error(&e);
            return;
        }
    };

    let output_path = workspace_path.join("Makefile");

    let mut sdk_config = match config::load_config(&config_path) {
        Ok(config) => config,
        Err(e) => {
            messages::error(&format!("Error loading config: {}", e));
            return;
        }
    };

    // Resolve effective no_dividers: CLI flag overrides user config
    let mut effective_no_dividers = false;

    // Load and apply user config overrides if present
    match config::UserConfig::load() {
        Ok(Some(user_config)) => {
            user_config.apply_to_sdk_config(&mut sdk_config, false);
            if let Some(config_no_dividers) = user_config.no_dividers {
                effective_no_dividers = config_no_dividers;
            }
        }
        Ok(None) => {}
        Err(e) => {
            messages::error(&format!("Warning: Failed to load user config: {}", e));
        }
    }

    // CLI --no-dividers always wins
    if no_dividers {
        effective_no_dividers = true;
    }

    let dividers = !effective_no_dividers;
    let makefile = generate_makefile_content(&sdk_config, dividers, Some(&workspace_path));

    match std::fs::write(&output_path, makefile) {
        Ok(_) => messages::success(&format!("Makefile written to {}", output_path.display())),
        Err(e) => messages::error(&format!("Failed to write Makefile: {}", e)),
    }

    // NEW: Generate VS Code tasks.json
    if let Err(e) = vscode_tasks_manager::generate_tasks_json(&workspace_path, &output_path) {
        messages::info(&format!("Could not generate VS Code tasks.json: {}", e));
    }
}

/// Generate a section divider comment banner for a Makefile
fn makefile_divider(title: &str) -> String {
    format!(
        "################################################################################\n\
         # {}\n\
         ################################################################################\n",
        title
    )
}

/// Resolve the `build_folder` value from sdk.yml to an absolute `PathBuf`
/// suitable for probing the file system.
///
/// The raw string may contain:
/// - `${{ WORKSPACE }}` — expanded to the actual workspace path.
/// - Host env-var references (`$HOME`, `~/`, `${VAR}`, `%VAR%`) — expanded
///   by [`dsdk_cli::workspace::expand_env_vars`].
/// - An absolute path — used directly.
/// - A plain relative path — joined with `workspace_path`.
fn resolve_build_folder_for_check(
    workspace_path: &std::path::Path,
    raw: &str,
) -> std::path::PathBuf {
    // Inject WORKSPACE so that ${{ WORKSPACE }} expands to the real path.
    let mut ctx = std::collections::HashMap::new();
    ctx.insert(
        "WORKSPACE".to_string(),
        workspace_path.to_string_lossy().to_string(),
    );
    let expanded = dsdk_cli::workspace::expand_manifest_vars(raw, &ctx);
    let expanded = dsdk_cli::workspace::expand_env_vars(&expanded);
    let path = std::path::PathBuf::from(&expanded);
    if path.is_absolute() {
        path
    } else {
        workspace_path.join(path)
    }
}

/// Discover `<build_folder>/<name>.mk` files for each git in the config.
///
/// `build_folder` is the raw string from sdk.yml's `build_folder:` key.  It
/// may be:
/// - `None` — use the default `"build"` directory (backward-compatible).
/// - A plain relative path (`"mk-files"`) — resolved relative to the workspace.
/// - An absolute path (`"/opt/sdk/fragments"`) — used directly.
/// - A manifest-variable reference (`"${{ WORKSPACE }}/mk-files"`) — the
///   `${{ WORKSPACE }}` token is expanded to the actual workspace path for the
///   file-system check.  The raw string (with the token still present) is
///   returned in the include paths so that [`render_command_for_makefile`] can
///   later convert it to `$(WORKSPACE)/mk-files/<name>.mk` in the Makefile.
///
/// Returns a list of include-path strings (`<folder>/<name>.mk`) for each git
/// whose corresponding fragment exists on disk.
///
/// `exclude` is the list of repository names whose fragments should be
/// suppressed.  An empty slice means no suppression.
fn discover_git_mk_files(
    workspace_path: &std::path::Path,
    gits: &[config::GitConfig],
    build_folder: Option<&str>,
    exclude: &[String],
) -> Vec<String> {
    let folder = build_folder.unwrap_or("build");
    let dir_for_check = resolve_build_folder_for_check(workspace_path, folder);
    gits.iter()
        .filter(|git| !exclude.iter().any(|n| n == &git.name))
        .filter_map(|git| {
            let mk_path = dir_for_check.join(format!("{}.mk", git.name));
            if mk_path.exists() {
                Some(format!("{}/{}.mk", folder, git.name))
            } else {
                None
            }
        })
        .collect()
}

/// Scan a makefile fragment for a specific target definition.
///
/// Looks for lines that define `target_name:` (before the first colon) or
/// list `target_name` in a `.PHONY:` declaration.
fn makefile_contains_target(content: &str, target_name: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(pos) = trimmed.find(':') {
            let before = trimmed[..pos].trim();
            if before == target_name {
                return true;
            }
            if before == ".PHONY" {
                let after = &trimmed[pos + 1..];
                for token in after.split_whitespace() {
                    if token == target_name {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Discover per-repo phase targets inside overlay makefiles.
///
/// For each git that has a `build/<name>.mk` (or equivalent) fragment,
/// scan the file for targets named `<name>-<phase>` for every phase in
/// the provided `phases` list.
///
/// Returns a map from phase name to the list of discovered target names.
fn discover_overlay_phase_targets(
    workspace_path: &std::path::Path,
    gits: &[config::GitConfig],
    build_folder: Option<&str>,
    exclude: &[String],
    phases: &[String],
) -> std::collections::HashMap<String, Vec<String>> {
    let mut phase_deps: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let folder = build_folder.unwrap_or("build");
    let dir_for_check = resolve_build_folder_for_check(workspace_path, folder);

    for git in gits
        .iter()
        .filter(|g| !exclude.iter().any(|n| n == &g.name))
    {
        let mk_path = dir_for_check.join(format!("{}.mk", git.name));
        if !mk_path.exists() {
            continue;
        }
        let content = match std::fs::read_to_string(&mk_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let repo_basename = git.name.rsplit('/').next().unwrap_or(&git.name);
        for phase in phases {
            let target_name = format!("{}-{}", repo_basename, phase);
            if makefile_contains_target(&content, &target_name) {
                phase_deps
                    .entry(phase.clone())
                    .or_default()
                    .push(target_name);
            }
        }
    }

    phase_deps
}

/// Generate the content of the Makefile from SDK configuration.
///
/// When `workspace_path` is provided the workspace `build/` directory is
/// scanned for per-git makefile fragments (`build/<name>.mk`).  Any fragment
/// that exists is automatically included via a `-include build/<name>.mk`
/// directive placed in the same section as the explicit `makefile_include`
/// entries from `sdk.yml`.
///
/// The `makefile_include:` key in `sdk.yml` controls both:
/// - Explicit include directives (via `files:` in structured form, or the
///   bare list in legacy form).
/// - Which auto-discovered fragments to suppress (via `exclude:` in structured
///   form).  An empty or absent `exclude` list means no suppression.
pub(crate) fn generate_makefile_content<T: config::SdkConfigCore>(
    sdk_config: &T,
    dividers: bool,
    workspace_path: Option<&std::path::Path>,
) -> String {
    let mut makefile = String::new();

    if dividers {
        makefile.push_str(&makefile_divider("Workspace and manifest variables"));
    }

    makefile.push_str(WORKSPACE_VARIABLE);
    makefile.push_str("\n\n");

    // Auto-generate _DIR variables from gits section
    let git_dir_vars = match dsdk_cli::workspace::generate_git_dir_vars(
        sdk_config.gits(),
        sdk_config.variables().as_ref(),
    ) {
        Ok(dv) => dv,
        Err(e) => {
            dsdk_cli::messages::error(&e);
            std::collections::HashMap::new()
        }
    };

    if !git_dir_vars.is_empty() {
        makefile.push_str("# Auto-generated directory variables from gits\n");
        let mut sorted_dir_vars: Vec<_> = git_dir_vars.iter().collect();
        sorted_dir_vars.sort_by_key(|(k, _)| k.as_str());
        for (key, value) in sorted_dir_vars {
            let make_value = render_command_for_makefile(value);
            makefile.push_str(&format!("{} := {}\n", key, make_value));
        }
        makefile.push('\n');
    }

    // Emit manifest variables as Make ?= assignments so host env vars override them
    let vars: std::collections::HashMap<String, String> =
        if let Some(raw_vars) = sdk_config.variables() {
            dsdk_cli::workspace::resolve_variables(raw_vars)
        } else {
            std::collections::HashMap::new()
        };

    if !vars.is_empty() {
        makefile
            .push_str("# Manifest variables — override with host env vars before invoking make\n");
        // Sort for deterministic output
        let mut sorted: Vec<_> = vars.iter().collect();
        sorted.sort_by_key(|(k, _)| k.as_str());
        for (key, value) in sorted {
            // Convert ${{ VAR }} references to $(VAR) so Make resolves them at
            // build time. This lets users reference $(WORKSPACE) and other Make
            // variables directly from sdk.yml variable definitions.
            let make_value = render_command_for_makefile(value);
            makefile.push_str(&format!("{} ?= {}\n", key, make_value));
        }
        makefile.push('\n');
    }

    // Derive include directives and exclusion list from the makefile_include config key.
    let config_include = sdk_config.makefile_include();
    let explicit_includes: Vec<String> = config_include
        .map(|mi| mi.files().to_vec())
        .unwrap_or_default();
    let exclude_names: &[String] = config_include.map(|mi| mi.exclude()).unwrap_or(&[]);

    // Auto-discover per-git makefile fragments: <build_folder>/<name>.mk
    let git_mk_includes: Vec<String> = if let Some(ws) = workspace_path {
        discover_git_mk_files(
            ws,
            sdk_config.gits(),
            sdk_config.build_folder().as_deref(),
            exclude_names,
        )
    } else {
        vec![]
    };

    // Emit all includes after variables so included files can reference Make vars
    if !explicit_includes.is_empty() || !git_mk_includes.is_empty() {
        if dividers {
            makefile.push_str(&makefile_divider("Makefile includes"));
        }
        for include_line in &explicit_includes {
            // Convert ${{ VAR }} references in include paths to $(VAR) so
            // Make resolves them at build time (e.g. $(WORKSPACE)/foo.mk).
            let rendered = render_command_for_makefile(include_line);
            makefile.push_str(&format!("-{}\n", rendered));
        }
        for mk_path in &git_mk_includes {
            // Apply ${{ VAR }} → $(VAR) conversion so that manifest-variable
            // references in build_folder (e.g. ${{ WORKSPACE }}/mk-files)
            // become proper Make variable references in the include directive.
            let rendered = render_command_for_makefile(mk_path);
            makefile.push_str(&format!("-include {}\n", rendered));
        }
        makefile.push('\n');
    }

    let phases = sdk_config.phases();

    // Discover overlay phase targets (e.g. u-boot-build, linux-clean)
    let phase_deps: std::collections::HashMap<String, Vec<String>> =
        if let Some(ws) = workspace_path {
            discover_overlay_phase_targets(
                ws,
                sdk_config.gits(),
                sdk_config.build_folder().as_deref(),
                exclude_names,
                &phases,
            )
        } else {
            std::collections::HashMap::new()
        };

    if dividers {
        makefile.push_str(&makefile_divider("High-level SDK targets"));
    }

    // Add .PHONY declarations
    let mut phony_targets = vec!["all".to_string()];
    for phase in &phases {
        let deps = phase_deps.get(phase).map(|v| v.as_slice()).unwrap_or(&[]);
        let has_sdk_target = sdk_config.phase_target(phase).is_some();
        if has_sdk_target || !deps.is_empty() {
            phony_targets.push(format!("sdk-{}", phase));
        }
    }

    // Add install targets to PHONY if install section exists
    if let Some(install_configs) = sdk_config.install() {
        if !install_configs.is_empty() {
            phony_targets.push("install-all".to_string());
        }
    }

    makefile.push_str(&format!(".PHONY: {}\n", phony_targets.join(" ")));

    // Add 'all' target that depends on sdk-build and sdk-test
    let mut all_deps = vec!["sdk-build"];
    let test_has_work = sdk_config.phase_target("test").is_some()
        || !phase_deps.get("test").map(|v| v.is_empty()).unwrap_or(true);
    if test_has_work {
        all_deps.push("sdk-test");
    }
    makefile.push_str(&format!("all: {}\n\n", all_deps.join(" ")));

    // Add sdk-<phase> targets for every configured phase
    for phase in &phases {
        let deps = phase_deps.get(phase).map(|v| v.as_slice()).unwrap_or(&[]);
        let target = sdk_config.phase_target(phase);
        let fallback = match phase.as_str() {
            "build" => Some("No build commands defined in sdk.yml"),
            "clean" => Some("No clean commands defined in sdk.yml"),
            "flash" => Some("No flash commands defined in sdk.yml"),
            _ => None,
        };
        add_phase_target(&mut makefile, phase, target, deps, fallback);
    }

    // Add install-all target if install section exists
    let has_install = match sdk_config.install() {
        Some(configs) => !configs.is_empty(),
        None => false,
    };

    if has_install && dividers {
        makefile.push_str(&makefile_divider("Installation targets"));
    }

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
    let repo_targets_start = makefile.len();
    for git in sdk_config.gits() {
        add_makefile_target(&mut makefile, git);
    }
    if repo_targets_start < makefile.len() && dividers {
        let divider = makefile_divider("Git repository targets");
        makefile.insert_str(repo_targets_start, &divider);
    }

    makefile
}

/// Render a command string for inclusion in a Makefile recipe.
///
/// Converts manifest variable references `${{ VAR }}` to Make variable
/// references `$(VAR)`.  Other content (e.g. existing `$(MAKE)`) is left
/// unchanged.
pub(crate) fn render_command_for_makefile(cmd: &str) -> String {
    let mut result = cmd.to_string();
    let mut search_start = 0;
    while let Some(open) = result[search_start..].find("${{") {
        let open_abs = search_start + open;
        let after_open = open_abs + 3;
        if let Some(close_rel) = result[after_open..].find("}}") {
            let close_abs = after_open + close_rel;
            let var_name = result[after_open..close_abs].trim();
            let make_ref = format!("$({})", var_name);
            let token_end = close_abs + 2;
            result.replace_range(open_abs..token_end, &make_ref);
            search_start = open_abs + make_ref.len();
        } else {
            break;
        }
    }
    result
}

/// Add a single per-repo target to the Makefile.
///
/// Returns `true` if anything was emitted. When the repo has neither
/// `build:` commands nor `build_depends_on:` the function is a no-op
/// so that empty stubs don't clutter the generated Makefile.
pub(crate) fn add_makefile_target(makefile: &mut String, git: &config::GitConfig) -> bool {
    // Skip repos that have no build configuration at all
    if git.build.is_none() && git.build_depends_on.is_none() {
        return false;
    }

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
            let rendered = render_command_for_makefile(cmd);
            let trimmed = rendered.trim();
            if trimmed.starts_with('#') {
                // Write as a Makefile comment (with tab like other commands)
                makefile.push_str(&format!(
                    "\t#{}\n",
                    trimmed.strip_prefix('#').unwrap().trim_start()
                ));
            } else {
                makefile.push_str(&format!("\t{}\n", rendered));
            }
        }
    } else {
        makefile.push_str(&format!("\t@echo Building {}\n", git.name));
    }
    makefile.push('\n');
    true
}

/// Add a generic sdk-<phase> target to the Makefile.
///
/// `phase` is the phase name (e.g. "build", "clean", "test").
/// `phase_target` is the optional SdkTarget from sdk.yml.
/// `extra_deps` are overlay targets (e.g. "u-boot-build") to add as prerequisites.
/// `fallback_message` is an optional echo printed when no sdk.yml target exists
/// and there are no extra deps.
pub(crate) fn add_phase_target(
    makefile: &mut String,
    phase: &str,
    phase_target: Option<&config::SdkTarget>,
    extra_deps: &[String],
    fallback_message: Option<&str>,
) {
    let target_name = format!("sdk-{}", phase);
    let mut all_deps = Vec::new();
    if let Some(target) = phase_target {
        if let Some(deps) = target.depends_on() {
            all_deps.extend(deps.iter().cloned());
        }
    }
    all_deps.extend(extra_deps.iter().cloned());

    if all_deps.is_empty() {
        makefile.push_str(&format!("{}:\n", target_name));
    } else {
        makefile.push_str(&format!("{}: {}\n", target_name, all_deps.join(" ")));
    }

    if let Some(target) = phase_target {
        for command in target.commands() {
            let rendered = render_command_for_makefile(command);
            let trimmed = rendered.trim();

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
            makefile.push_str(&format!("\t{}\n", rendered));
        }
    } else if let Some(msg) = fallback_message {
        if extra_deps.is_empty() {
            makefile.push_str(&format!("\t@echo \"{}\"\n", msg));
        }
    }

    makefile.push('\n');
}

/// Add install-all target that depends on all install targets
pub(crate) fn add_install_all_target(makefile: &mut String, installs: &[config::InstallConfig]) {
    let all_targets: Vec<_> = installs
        .iter()
        .map(|i| format!("install-{}", i.name))
        .collect();
    makefile.push_str(&format!("install-all: {}\n", all_targets.join(" ")));
    makefile.push_str("\t@echo 'All installations complete'\n\n");
}

/// Check if a line contains shell control structures that should be treated as a complete statement
pub(crate) fn contains_complete_control_structure(line: &str) -> bool {
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
pub(crate) fn is_shell_control_keyword(line: &str) -> bool {
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
pub(crate) fn has_shell_control_structure(commands: &[String]) -> bool {
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
pub(crate) fn add_install_target(makefile: &mut String, install: &config::InstallConfig) {
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
                    let rendered = render_command_for_makefile(cmd);
                    let trimmed = rendered.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        // Check if this is a complete single-line control structure
                        if contains_complete_control_structure(trimmed) {
                            // Single-line if/then/else/fi or while/for - add semicolon at end
                            if !trimmed.ends_with(';') {
                                makefile.push_str(&format!("\t    {}; \\\n", rendered));
                            } else {
                                makefile.push_str(&format!("\t    {} \\\n", rendered));
                            }
                        } else {
                            // Multi-line control structure or regular command
                            // Add semicolons for proper shell syntax, but not for control keywords
                            let needs_semicolon = !is_shell_control_keyword(trimmed)
                                && !trimmed.ends_with(';')
                                && !trimmed.ends_with('{')
                                && !trimmed.ends_with('\\');

                            if needs_semicolon {
                                makefile.push_str(&format!("\t    {}; \\\n", rendered));
                            } else {
                                makefile.push_str(&format!("\t    {} \\\n", rendered));
                            }
                        }
                    }
                }

                makefile.push_str("\t  ) && \\\n");
            } else {
                // No control structures - use original && logic
                for cmd in build_cmds {
                    let rendered = render_command_for_makefile(cmd);
                    let trimmed = rendered.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        // Wrap commands containing 'cd' in subshells to avoid affecting subsequent commands
                        if trimmed.contains(" cd ") || trimmed.starts_with("cd ") {
                            makefile.push_str(&format!("\t  ({}) && \\\n", rendered));
                        } else {
                            makefile.push_str(&format!("\t  {} && \\\n", rendered));
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
                let rendered = render_command_for_makefile(cmd);
                let trimmed = rendered.trim();
                if trimmed.starts_with('#') {
                    makefile.push_str(&format!(
                        "\t#{}\n",
                        trimmed.strip_prefix('#').unwrap().trim_start()
                    ));
                } else {
                    makefile.push_str(&format!("\t{}\n", rendered));
                }
            }
        }
        makefile.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_makefile_content_empty() {
        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);
        assert!(
            makefile.starts_with(&format!("{}\n\n", WORKSPACE_VARIABLE)),
            "Expected WORKSPACE variable first in Makefile, got:\n{}",
            makefile
        );
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
            python_deps: None,
        };

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![git_config],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);
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
            python_deps: None,
        };

        let git2 = config::GitConfig {
            name: "dep-repo".to_string(),
            url: "https://github.com/test/dep.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: Some(vec!["base-repo".to_string()]),
            git_depends_on: None,
            build: Some(vec!["# This is a comment".to_string(), "make".to_string()]),
            documentation_dir: None,
            python_deps: None,
        };

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![git1, git2],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);
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
            python_deps: None,
        };

        let emitted = add_makefile_target(&mut makefile, &git_config);

        // When a repo has neither build: nor build_depends_on:,
        // nothing is emitted to keep the Makefile clean.
        assert!(!emitted);
        assert!(makefile.is_empty());
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
            python_deps: None,
        };

        add_makefile_target(&mut makefile, &git_config);

        assert!(makefile.contains("commented-repo:"));
        assert!(makefile.contains("\t#Configure the build"));
        assert!(makefile.contains("\t./configure"));
        assert!(makefile.contains("\t#Another comment"));
        assert!(makefile.contains("\tmake"));
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
            python_deps: None,
        };

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![git_config],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);
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
            python_deps: None,
        };

        let mut makefile = String::new();
        add_makefile_target(&mut makefile, &git_with_many_deps);
        assert!(makefile.contains("many-deps: dep1 dep2 dep3"));
    }

    #[test]
    fn test_envsetup_target_generation() {
        // Test with envsetup commands
        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: Some(config::SdkTarget::Commands(vec![
                "ln -sf qemu_v8.mk build/Makefile".to_string(),
                "cd build && make -j3 toolchains".to_string(),
            ])),
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

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
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
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
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

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
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

        // envsetup target is always generated (not in PHONY when empty)
        assert!(makefile.contains("sdk-envsetup:"));
    }

    #[test]
    fn test_envsetup_target_none() {
        // Test with no envsetup commands
        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

        // envsetup target is always generated (not in PHONY when empty)
        assert!(makefile.contains("sdk-envsetup:"));
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

        add_phase_target(&mut makefile, "envsetup", Some(&target), &[], None);

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
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: Some(config::SdkTarget::Commands(vec![
                "cargo test --release".to_string(),
                "python run_integration_tests.py".to_string(),
            ])),
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

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
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
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
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

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
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

        // test target is always generated (not in PHONY when empty)
        assert!(makefile.contains("sdk-test:"));
    }

    #[test]
    fn test_test_target_none() {
        // Test with no test commands
        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

        // test target is always generated (not in PHONY when empty)
        assert!(makefile.contains("sdk-test:"));
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

        add_phase_target(&mut makefile, "test", Some(&target), &[], None);

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
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: Some(config::SdkTarget::Commands(vec![
                "make configure".to_string()
            ])),
            test: Some(config::SdkTarget::Commands(vec!["make test".to_string()])),
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

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
    fn test_render_command_for_makefile_substitution() {
        // ${{ VAR }} becomes $(VAR)
        assert_eq!(
            render_command_for_makefile("DOCKER_DEFAULT_PLATFORM=${{ PLATFORM }} ./run.sh"),
            "DOCKER_DEFAULT_PLATFORM=$(PLATFORM) ./run.sh"
        );

        // Multiple vars in one command
        assert_eq!(
            render_command_for_makefile("${{ CMD }} --platform ${{ PLATFORM }}"),
            "$(CMD) --platform $(PLATFORM)"
        );

        // Existing Make variable references are preserved
        assert_eq!(
            render_command_for_makefile("$(MAKE) -C build all $(MAKEFLAGS)"),
            "$(MAKE) -C build all $(MAKEFLAGS)"
        );

        // Command without any variables is unchanged
        assert_eq!(
            render_command_for_makefile("cd repo && ./build.sh"),
            "cd repo && ./build.sh"
        );

        // Unclosed ${{ is left unchanged
        assert_eq!(
            render_command_for_makefile("echo ${{ NOCLOSE"),
            "echo ${{ NOCLOSE"
        );
    }

    #[test]
    fn test_manifest_vars_workspace_reference() {
        // ${{ WORKSPACE }} in variable values must become $(WORKSPACE) so Make
        // can resolve the workspace path at build time.
        let mut vars = std::collections::HashMap::new();
        vars.insert(
            "TOOLCHAIN_PATH".to_string(),
            "${{ WORKSPACE }}/toolchains/aarch64-bm/bin".to_string(),
        );
        vars.insert(
            "PLATFORMS_DIR".to_string(),
            "${{ WORKSPACE }}/platforms".to_string(),
        );

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(vars),
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

        assert!(
            makefile.contains("TOOLCHAIN_PATH ?= $(WORKSPACE)/toolchains/aarch64-bm/bin"),
            "Expected WORKSPACE reference converted to Make variable, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("PLATFORMS_DIR ?= $(WORKSPACE)/platforms"),
            "Expected WORKSPACE reference converted to Make variable, got:\n{}",
            makefile
        );
        // Raw ${{ }} syntax must not survive into the generated Makefile.
        assert!(
            !makefile.contains("${{"),
            "Raw ${{{{ }}}} syntax should not appear in Makefile, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_manifest_vars_arbitrary_make_variable_reference() {
        // Any ${{ VAR }} (not just WORKSPACE) in a variable value must be
        // converted to $(VAR), enabling cross-variable composition.
        let mut vars = std::collections::HashMap::new();
        vars.insert("BASE".to_string(), "/opt/sdk".to_string());
        vars.insert("DERIVED".to_string(), "${{ BASE }}/extras".to_string());

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(vars),
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

        assert!(
            makefile.contains("DERIVED ?= $(BASE)/extras"),
            "Expected cross-variable reference converted to Make form, got:\n{}",
            makefile
        );
        assert!(
            !makefile.contains("${{"),
            "Raw ${{{{ }}}} syntax should not appear in Makefile, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_makefile_include_with_workspace_reference() {
        // ${{ WORKSPACE }} in makefile_include paths must become $(WORKSPACE).
        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: Some(config::MakefileInclude::Legacy(vec![
                "include ${{ WORKSPACE }}/shared/common.mk".to_string(),
                "include platform/board.mk".to_string(),
            ])),
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

        assert!(
            makefile.contains("-include $(WORKSPACE)/shared/common.mk"),
            "Expected WORKSPACE in include path converted, got:\n{}",
            makefile
        );
        // Plain include paths must still work unchanged.
        assert!(
            makefile.contains("-include platform/board.mk"),
            "Expected plain include path unchanged, got:\n{}",
            makefile
        );
        assert!(
            !makefile.contains("${{"),
            "Raw ${{{{ }}}} syntax should not appear in Makefile, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_generate_makefile_with_variables() {
        let mut vars = std::collections::HashMap::new();
        vars.insert(
            "DOCKER_DEFAULT_PLATFORM".to_string(),
            "linux/amd64".to_string(),
        );

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: Some(vec![config::InstallConfig {
                name: "devcontainer".to_string(),
                depends_on: None,
                sentinel: Some("opt/.devcontainer-installed".to_string()),
                commands: Some(vec![
                    "cd repo && DOCKER_DEFAULT_PLATFORM=${{ DOCKER_DEFAULT_PLATFORM }} ./run.sh --new".to_string(),
                ]),
            }]),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(vars),
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

        let workspace_index = makefile
            .find(WORKSPACE_VARIABLE)
            .expect("WORKSPACE variable should be emitted");
        let manifest_var_index = makefile
            .find("DOCKER_DEFAULT_PLATFORM ?= linux/amd64")
            .expect("manifest variable should be emitted");

        assert!(
            workspace_index < manifest_var_index,
            "WORKSPACE should be emitted before manifest variables, got:\n{}",
            makefile
        );

        // ?= assignment emitted for the manifest variable
        assert!(
            makefile.contains("DOCKER_DEFAULT_PLATFORM ?= linux/amd64"),
            "Expected ?= assignment in Makefile, got:\n{}",
            makefile
        );

        // ${{ VAR }} in install command becomes $(VAR)
        assert!(
            makefile.contains("$(DOCKER_DEFAULT_PLATFORM)"),
            "Expected Make variable reference in command, got:\n{}",
            makefile
        );

        // The raw ${{ }} syntax should NOT appear in the output
        assert!(
            !makefile.contains("${{"),
            "Raw manifest variable syntax should not appear in Makefile"
        );
    }

    #[test]
    fn test_variables_before_includes() {
        let mut vars = std::collections::HashMap::new();
        vars.insert("PLATFORMS_ROOT".to_string(), "platforms".to_string());

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![],
            copy_files: None,
            makefile_include: Some(config::MakefileInclude::Legacy(vec![
                "include build/extra.mk".to_string(),
            ])),
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(vars),
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

        let vars_pos = makefile.find("?=").expect("variables block missing");
        let include_pos = makefile.find("-include").expect("include block missing");
        assert!(
            vars_pos < include_pos,
            "Expected variables before -include, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_generate_makefile_with_dividers() {
        let git_config = config::GitConfig {
            name: "test-repo".to_string(),
            url: "https://github.com/test/repo.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: Some(vec!["make".to_string()]),
            documentation_dir: None,
            python_deps: None,
        };

        let mut vars = std::collections::HashMap::new();
        vars.insert("MY_VAR".to_string(), "value".to_string());

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: Some(vec![config::InstallConfig {
                name: "my-tool".to_string(),
                depends_on: None,
                sentinel: Some("opt/.my-tool-installed".to_string()),
                commands: Some(vec!["echo install".to_string()]),
            }]),
            gits: vec![git_config],
            copy_files: None,
            makefile_include: Some(config::MakefileInclude::Legacy(vec![
                "include extra.mk".to_string()
            ])),
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(vars),
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, true, None);

        // Verify divider banners are present
        assert!(
            makefile.contains("# Workspace and manifest variables\n"),
            "Expected 'Workspace and manifest variables' divider, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("# Makefile includes\n"),
            "Expected 'Makefile includes' divider, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("# High-level SDK targets\n"),
            "Expected 'High-level SDK targets' divider, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("# Installation targets\n"),
            "Expected 'Installation targets' divider, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("# Git repository targets\n"),
            "Expected 'Git repository targets' divider, got:\n{}",
            makefile
        );

        // Verify the divider format uses 80-char # lines
        assert!(
            makefile.contains("################################################################################\n# Workspace and manifest variables\n################################################################################"),
            "Expected full divider banner format"
        );
    }

    #[test]
    fn test_generate_makefile_without_dividers() {
        let git_config = config::GitConfig {
            name: "test-repo".to_string(),
            url: "https://github.com/test/repo.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: Some(vec!["make".to_string()]),
            documentation_dir: None,
            python_deps: None,
        };

        let mut vars = std::collections::HashMap::new();
        vars.insert("MY_VAR".to_string(), "value".to_string());

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: Some(vec![config::InstallConfig {
                name: "my-tool".to_string(),
                depends_on: None,
                sentinel: Some("opt/.my-tool-installed".to_string()),
                commands: Some(vec!["echo install".to_string()]),
            }]),
            gits: vec![git_config],
            copy_files: None,
            makefile_include: Some(config::MakefileInclude::Legacy(vec![
                "include extra.mk".to_string()
            ])),
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(vars),
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);

        // Verify no divider banners are present
        assert!(
            !makefile.contains("########"),
            "Expected no divider banners when dividers=false, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_makefile_divider_format() {
        let divider = makefile_divider("Test Section");
        assert_eq!(
            divider,
            "################################################################################\n\
             # Test Section\n\
             ################################################################################\n"
        );
    }

    // -------------------------------------------------------------------------
    // Tests for automatic per-git build/<name>.mk discovery
    // -------------------------------------------------------------------------

    #[test]
    fn test_discover_git_mk_files_none_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let gits = vec![config::GitConfig {
            name: "u-boot".to_string(),
            url: "https://example.com/u-boot.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: None,
            documentation_dir: None,
            python_deps: None,
        }];
        let found = discover_git_mk_files(tmp.path(), &gits, None, &[]);
        assert!(
            found.is_empty(),
            "Expected no mk files when build/ dir is absent"
        );
    }

    #[test]
    fn test_discover_git_mk_files_one_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(build_dir.join("u-boot.mk"), "# u-boot rules\n").expect("write u-boot.mk");

        let gits = vec![
            config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
            config::GitConfig {
                name: "linux".to_string(),
                url: "https://example.com/linux.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
        ];

        let found = discover_git_mk_files(tmp.path(), &gits, None, &[]);
        assert_eq!(found, vec!["build/u-boot.mk".to_string()]);
    }

    #[test]
    fn test_discover_git_mk_files_multiple_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(build_dir.join("u-boot.mk"), "").expect("write u-boot.mk");
        std::fs::write(build_dir.join("linux.mk"), "").expect("write linux.mk");

        let gits = vec![
            config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
            config::GitConfig {
                name: "linux".to_string(),
                url: "https://example.com/linux.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
            config::GitConfig {
                name: "trusted-firmware-a".to_string(),
                url: "https://example.com/tfa.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
        ];

        let found = discover_git_mk_files(tmp.path(), &gits, None, &[]);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&"build/u-boot.mk".to_string()));
        assert!(found.contains(&"build/linux.mk".to_string()));
    }

    #[test]
    fn test_generate_makefile_auto_includes_git_mk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(
            build_dir.join("u-boot.mk"),
            "u-boot-build:\n\t$(MAKE) -C u-boot\n",
        )
        .expect("write u-boot.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: Some(vec!["$(MAKE) u-boot-build".to_string()]),
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            makefile.contains("-include build/u-boot.mk"),
            "Expected auto-include for u-boot.mk, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_generate_makefile_no_auto_includes_when_mk_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: Some(vec!["$(MAKE) -C u-boot".to_string()]),
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            !makefile.contains("-include build/u-boot.mk"),
            "Expected no auto-include when u-boot.mk is absent, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_auto_includes_after_explicit_includes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(build_dir.join("u-boot.mk"), "").expect("write u-boot.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: Some(config::MakefileInclude::Legacy(vec![
                "include shared/platform.mk".to_string(),
            ])),
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        let explicit_pos = makefile
            .find("-include shared/platform.mk")
            .expect("explicit include missing");
        let auto_pos = makefile
            .find("-include build/u-boot.mk")
            .expect("auto-include missing");

        assert!(
            explicit_pos < auto_pos,
            "Expected explicit includes before auto-discovered ones, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_auto_includes_with_dividers() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(build_dir.join("linux.mk"), "").expect("write linux.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "linux".to_string(),
                url: "https://example.com/linux.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, true, Some(tmp.path()));

        assert!(
            makefile.contains("# Makefile includes\n"),
            "Expected 'Makefile includes' divider for auto-discovered file, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("-include build/linux.mk"),
            "Expected auto-include for linux.mk, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_auto_includes_variables_before_auto_includes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(build_dir.join("u-boot.mk"), "").expect("write u-boot.mk");

        let mut vars = std::collections::HashMap::new();
        vars.insert(
            "CROSS_COMPILE".to_string(),
            "aarch64-linux-gnu-".to_string(),
        );

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(vars),
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        let vars_pos = makefile.find("?=").expect("variables block missing");
        let include_pos = makefile
            .find("-include build/u-boot.mk")
            .expect("auto-include missing");

        assert!(
            vars_pos < include_pos,
            "Expected variables before auto-discovered -include lines, got:\n{}",
            makefile
        );
    }

    // -------------------------------------------------------------------------
    // Tests for build_folder configuration key
    // -------------------------------------------------------------------------

    #[test]
    fn test_build_folder_overrides_default_build_dir() {
        // When build_folder is set, fragments are looked up under that folder
        // instead of the default "build/" directory.
        let tmp = tempfile::tempdir().expect("tempdir");
        let custom_dir = tmp.path().join("mk-files");
        std::fs::create_dir_all(&custom_dir).expect("create mk-files dir");
        std::fs::write(custom_dir.join("u-boot.mk"), "# custom location\n")
            .expect("write u-boot.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: Some("mk-files".to_string()),
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            makefile.contains("-include mk-files/u-boot.mk"),
            "Expected auto-include from custom folder, got:\n{}",
            makefile
        );
        assert!(
            !makefile.contains("-include build/u-boot.mk"),
            "Expected no auto-include from default build/ folder, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_build_folder_not_found_in_default_dir() {
        // When build_folder is set and the fragment exists in build/ but not in
        // the configured folder, the fragment must NOT be included.
        let tmp = tempfile::tempdir().expect("tempdir");
        // Put the .mk file in the default build/ dir only
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(build_dir.join("u-boot.mk"), "").expect("write u-boot.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: Some("other-dir".to_string()),
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            !makefile.contains("-include"),
            "Expected no auto-include when .mk absent from configured folder, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_discover_git_mk_files_custom_folder() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let custom_dir = tmp.path().join("fragments");
        std::fs::create_dir_all(&custom_dir).expect("create fragments dir");
        std::fs::write(custom_dir.join("linux.mk"), "").expect("write linux.mk");

        let gits = vec![
            config::GitConfig {
                name: "linux".to_string(),
                url: "https://example.com/linux.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
            config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            },
        ];

        let found = discover_git_mk_files(tmp.path(), &gits, Some("fragments"), &[]);
        assert_eq!(found, vec!["fragments/linux.mk".to_string()]);
    }

    #[test]
    fn test_build_folder_absolute_path() {
        // An absolute path in build_folder is used directly for the FS check
        // and emitted verbatim in the -include directive.
        let tmp = tempfile::tempdir().expect("tempdir");
        let abs_dir = tmp.path().join("abs-fragments");
        std::fs::create_dir_all(&abs_dir).expect("create abs-fragments dir");
        std::fs::write(abs_dir.join("u-boot.mk"), "").expect("write u-boot.mk");

        let abs_path = abs_dir.to_str().unwrap().to_string();

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: Some(abs_path.clone()),
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        // Use a *different* directory as the workspace root so we can confirm
        // the absolute path is used for the check rather than workspace-relative.
        let other_ws = tempfile::tempdir().expect("other workspace tempdir");
        let makefile = generate_makefile_content(&config, false, Some(other_ws.path()));

        let expected = format!("-include {}/u-boot.mk", abs_path);
        assert!(
            makefile.contains(&expected),
            "Expected absolute-path include '{}', got:\n{}",
            expected,
            makefile
        );
    }

    #[test]
    fn test_build_folder_workspace_variable_reference() {
        // ${{ WORKSPACE }}/fragments → file-system check uses the real path;
        // the Makefile include directive becomes $(WORKSPACE)/fragments/<name>.mk.
        let tmp = tempfile::tempdir().expect("tempdir");
        let fragments_dir = tmp.path().join("fragments");
        std::fs::create_dir_all(&fragments_dir).expect("create fragments dir");
        std::fs::write(fragments_dir.join("u-boot.mk"), "").expect("write u-boot.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: Some("${{ WORKSPACE }}/fragments".to_string()),
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            makefile.contains("-include $(WORKSPACE)/fragments/u-boot.mk"),
            "Expected Make-variable include for ${{{{ WORKSPACE }}}}/fragments, got:\n{}",
            makefile
        );
        // The raw ${{ }} syntax must not appear in the output.
        assert!(
            !makefile.contains("${{"),
            "Raw ${{{{ }}}} syntax must not appear in Makefile, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_resolve_build_folder_for_check_relative() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let result = resolve_build_folder_for_check(tmp.path(), "my-dir");
        assert_eq!(result, tmp.path().join("my-dir"));
    }

    #[test]
    fn test_resolve_build_folder_for_check_absolute() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let abs = tmp.path().join("abs").to_string_lossy().to_string();
        let result = resolve_build_folder_for_check(tmp.path(), &abs);
        assert_eq!(result, std::path::PathBuf::from(&abs));
    }

    #[test]
    fn test_resolve_build_folder_for_check_workspace_var() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let raw = "${{ WORKSPACE }}/sub";
        let result = resolve_build_folder_for_check(tmp.path(), raw);
        assert_eq!(result, tmp.path().join("sub"));
    }

    // -------------------------------------------------------------------------
    // Tests for makefile_include exclude (config-based fragment suppression)
    // -------------------------------------------------------------------------

    #[test]
    fn test_makefile_include_structured_exclude_named() {
        // Structured form with exclude: only the named repos are suppressed;
        // others are still auto-discovered and included.
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(build_dir.join("qemu.mk"), "").expect("write qemu.mk");
        std::fs::write(build_dir.join("trusted-services.mk"), "")
            .expect("write trusted-services.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![
                config::GitConfig {
                    name: "qemu".to_string(),
                    url: "https://example.com/qemu.git".to_string(),
                    commit: "main".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                    python_deps: None,
                },
                config::GitConfig {
                    name: "trusted-services".to_string(),
                    url: "https://example.com/ts.git".to_string(),
                    commit: "main".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                    python_deps: None,
                },
            ],
            copy_files: None,
            makefile_include: Some(config::MakefileInclude::Structured(
                config::MakefileIncludeConfig {
                    files: vec![],
                    exclude: vec!["qemu".to_string()],
                },
            )),
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            !makefile.contains("-include build/qemu.mk"),
            "Expected qemu.mk excluded, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("-include build/trusted-services.mk"),
            "Expected trusted-services.mk still included, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_makefile_include_structured_exclude_all_listed() {
        // When all repo names are listed in exclude, no auto-discovered
        // fragments appear in the generated Makefile.
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(build_dir.join("qemu.mk"), "").expect("write qemu.mk");
        std::fs::write(build_dir.join("trusted-services.mk"), "")
            .expect("write trusted-services.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![
                config::GitConfig {
                    name: "qemu".to_string(),
                    url: "https://example.com/qemu.git".to_string(),
                    commit: "main".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                    python_deps: None,
                },
                config::GitConfig {
                    name: "trusted-services".to_string(),
                    url: "https://example.com/ts.git".to_string(),
                    commit: "main".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                    python_deps: None,
                },
            ],
            copy_files: None,
            makefile_include: Some(config::MakefileInclude::Structured(
                config::MakefileIncludeConfig {
                    files: vec![],
                    exclude: vec!["qemu".to_string(), "trusted-services".to_string()],
                },
            )),
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            !makefile.contains("-include build/qemu.mk"),
            "Expected qemu.mk suppressed, got:\n{}",
            makefile
        );
        assert!(
            !makefile.contains("-include build/trusted-services.mk"),
            "Expected trusted-services.mk suppressed, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_makefile_include_structured_files_and_exclude() {
        // Structured form: explicit file includes AND named exclude work together.
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(build_dir.join("qemu.mk"), "").expect("write qemu.mk");
        std::fs::write(build_dir.join("trusted-services.mk"), "")
            .expect("write trusted-services.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![
                config::GitConfig {
                    name: "qemu".to_string(),
                    url: "https://example.com/qemu.git".to_string(),
                    commit: "main".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                    python_deps: None,
                },
                config::GitConfig {
                    name: "trusted-services".to_string(),
                    url: "https://example.com/ts.git".to_string(),
                    commit: "main".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                    python_deps: None,
                },
            ],
            copy_files: None,
            makefile_include: Some(config::MakefileInclude::Structured(
                config::MakefileIncludeConfig {
                    files: vec!["include extra.mk".to_string()],
                    exclude: vec!["qemu".to_string()],
                },
            )),
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            makefile.contains("-include extra.mk"),
            "Expected explicit include present, got:\n{}",
            makefile
        );
        assert!(
            !makefile.contains("-include build/qemu.mk"),
            "Expected qemu.mk excluded, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("-include build/trusted-services.mk"),
            "Expected trusted-services.mk still included, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_makefile_include_legacy_no_exclude() {
        // Legacy form: list of include directives, no exclusion applied.
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(build_dir.join("u-boot.mk"), "").expect("write u-boot.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: Some(config::MakefileInclude::Legacy(vec![
                "include platform.mk".to_string()
            ])),
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        // Explicit include present
        assert!(
            makefile.contains("-include platform.mk"),
            "Expected explicit include present, got:\n{}",
            makefile
        );
        // Auto-discovered fragment still included (legacy → no exclusion)
        assert!(
            makefile.contains("-include build/u-boot.mk"),
            "Expected auto-include for u-boot.mk with legacy form, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_makefile_include_deserialization_legacy() {
        // The bare-list YAML form must deserialise into the Legacy variant.
        let yaml = "mirror: /tmp/mirror\ngits: []\nmakefile_include:\n  - include extra.mk\n";
        let config: config::SdkConfig = serde_yaml::from_str(yaml).expect("parse yaml");
        match config
            .makefile_include
            .as_ref()
            .expect("makefile_include absent")
        {
            config::MakefileInclude::Legacy(files) => {
                assert_eq!(files, &["include extra.mk".to_string()]);
            }
            config::MakefileInclude::Structured(_) => {
                panic!("Expected Legacy variant for bare-list YAML form");
            }
        }
    }

    #[test]
    fn test_makefile_include_deserialization_structured() {
        // The structured YAML form must deserialise into the Structured variant.
        let yaml = "mirror: /tmp/mirror\ngits: []\nmakefile_include:\n  files:\n    - include extra.mk\n  exclude:\n    - qemu\n    - trusted-services\n";
        let config: config::SdkConfig = serde_yaml::from_str(yaml).expect("parse yaml");
        match config
            .makefile_include
            .as_ref()
            .expect("makefile_include absent")
        {
            config::MakefileInclude::Structured(c) => {
                assert_eq!(c.files, vec!["include extra.mk".to_string()]);
                assert_eq!(
                    c.exclude,
                    vec!["qemu".to_string(), "trusted-services".to_string()]
                );
            }
            config::MakefileInclude::Legacy(_) => {
                panic!("Expected Structured variant for map YAML form");
            }
        }
    }

    #[test]
    fn test_makefile_include_deserialization_exclude_only() {
        // Structured form with only exclude (no files) must deserialise correctly.
        let yaml = "mirror: /tmp/mirror\ngits: []\nmakefile_include:\n  exclude:\n    - qemu\n";
        let config: config::SdkConfig = serde_yaml::from_str(yaml).expect("parse yaml");
        match config
            .makefile_include
            .as_ref()
            .expect("makefile_include absent")
        {
            config::MakefileInclude::Structured(c) => {
                assert!(c.files.is_empty(), "Expected empty files list");
                assert_eq!(c.exclude, vec!["qemu".to_string()]);
            }
            config::MakefileInclude::Legacy(_) => {
                panic!("Expected Structured variant for exclude-only YAML form");
            }
        }
    }

    #[test]
    fn test_generate_makefile_with_auto_dir_vars() {
        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![
                config::GitConfig {
                    name: "u-boot".to_string(),
                    url: "https://example.com/u-boot.git".to_string(),
                    commit: "master".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                    python_deps: None,
                },
                config::GitConfig {
                    name: "linux-stable".to_string(),
                    url: "https://example.com/linux.git".to_string(),
                    commit: "v6.1".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                    python_deps: None,
                },
            ],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);
        assert!(
            makefile.contains("U_BOOT_DIR := $(WORKSPACE)/u-boot"),
            "Missing U_BOOT_DIR in:\n{}",
            makefile
        );
        assert!(
            makefile.contains("LINUX_STABLE_DIR := $(WORKSPACE)/linux-stable"),
            "Missing LINUX_STABLE_DIR in:\n{}",
            makefile
        );
        assert!(
            makefile.contains("# Auto-generated directory variables from gits"),
            "Missing auto-generated comment in:\n{}",
            makefile
        );
    }

    #[test]
    fn test_generate_makefile_dir_vars_before_user_vars() {
        let mut user_vars = std::collections::HashMap::new();
        user_vars.insert(
            "MY_VAR".to_string(),
            "${{ WORKSPACE }}/something".to_string(),
        );

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "master".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(user_vars),
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);
        let dir_var_pos = makefile
            .find("U_BOOT_DIR :=")
            .expect("U_BOOT_DIR not found");
        let user_var_pos = makefile.find("MY_VAR ?=").expect("MY_VAR not found");
        assert!(
            dir_var_pos < user_var_pos,
            "Auto-generated DIR vars should appear before user-defined vars"
        );
    }

    #[test]
    fn test_generate_makefile_dir_var_user_override_suppresses() {
        let mut user_vars = std::collections::HashMap::new();
        user_vars.insert(
            "U_BOOT_DIR".to_string(),
            "${{ WORKSPACE }}/custom/u-boot".to_string(),
        );

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "master".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(user_vars),
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);
        // Should NOT have auto-generated section since user overrides
        assert!(
            !makefile.contains("# Auto-generated directory variables from gits"),
            "Auto-generated section should be suppressed when user overrides all DIR vars"
        );
        // User-defined var should be present with ?= syntax
        assert!(
            makefile.contains("U_BOOT_DIR ?= $(WORKSPACE)/custom/u-boot"),
            "User-defined U_BOOT_DIR should use ?= syntax:\n{}",
            makefile
        );
    }

    #[test]
    fn test_generate_makefile_dir_var_uses_immediate_assignment() {
        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "zephyrproject/zephyr".to_string(),
                url: "https://example.com/zephyr.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, None);
        assert!(
            makefile.contains("ZEPHYR_DIR := $(WORKSPACE)/zephyrproject/zephyr"),
            "Expected path-based name to produce correct DIR var:\n{}",
            makefile
        );
        // Verify it uses := not ?=
        assert!(
            !makefile.contains("ZEPHYR_DIR ?="),
            "Auto-generated DIR vars should use := not ?="
        );
    }

    // -------------------------------------------------------------------------
    // Tests for overlay phase target auto-wiring
    // -------------------------------------------------------------------------

    #[test]
    fn test_discover_overlay_phase_targets_finds_matching_targets() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(
            build_dir.join("u-boot.mk"),
            ".PHONY: u-boot-build u-boot-clean\nu-boot-build:\n\t@echo building u-boot\n\nu-boot-clean:\n\t@echo cleaning u-boot\n",
        )
        .expect("write u-boot.mk");

        let gits = vec![config::GitConfig {
            name: "u-boot".to_string(),
            url: "https://example.com/u-boot.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: None,
            documentation_dir: None,
            python_deps: None,
        }];

        let phases = config::default_phases();
        let phase_deps = discover_overlay_phase_targets(tmp.path(), &gits, None, &[], &phases);
        assert_eq!(
            phase_deps.get("build"),
            Some(&vec!["u-boot-build".to_string()])
        );
        assert_eq!(
            phase_deps.get("clean"),
            Some(&vec!["u-boot-clean".to_string()])
        );
        assert!(!phase_deps.contains_key("test"));
        assert!(!phase_deps.contains_key("flash"));
        assert!(!phase_deps.contains_key("envsetup"));
    }

    #[test]
    fn test_discover_overlay_phase_targets_no_match() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        // Overlay defines generic targets, not <repo>-<phase> style
        std::fs::write(build_dir.join("linux.mk"), "all:\n\t@echo all\n").expect("write linux.mk");

        let gits = vec![config::GitConfig {
            name: "linux".to_string(),
            url: "https://example.com/linux.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: None,
            documentation_dir: None,
            python_deps: None,
        }];

        let phases = config::default_phases();
        let phase_deps = discover_overlay_phase_targets(tmp.path(), &gits, None, &[], &phases);
        assert!(
            phase_deps.is_empty(),
            "Expected no phase deps when overlay lacks <repo>-<phase> targets"
        );
    }

    #[test]
    fn test_discover_overlay_phase_targets_path_git_name() {
        // Repo name contains a path separator ("zephyrproject/zephyr").
        // The .mk file lives at build/zephyrproject/zephyr.mk and its targets
        // are named "zephyr-build" etc. (basename only, not full path).
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build").join("zephyrproject");
        std::fs::create_dir_all(&build_dir).expect("create build/zephyrproject dir");
        std::fs::write(
            build_dir.join("zephyr.mk"),
            ".PHONY: zephyr-build zephyr-clean zephyr-flash zephyr-envsetup\n\
             zephyr-build:\n\t@echo build\n\n\
             zephyr-clean:\n\t@echo clean\n\n\
             zephyr-flash:\n\t@echo flash\n\n\
             zephyr-envsetup:\n\t@echo envsetup\n",
        )
        .expect("write zephyr.mk");

        let gits = vec![config::GitConfig {
            name: "zephyrproject/zephyr".to_string(),
            url: "https://github.com/zephyrproject-rtos/zephyr".to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: None,
            documentation_dir: None,
            python_deps: None,
        }];

        let phases = config::default_phases();
        let phase_deps = discover_overlay_phase_targets(tmp.path(), &gits, None, &[], &phases);
        assert_eq!(
            phase_deps.get("build"),
            Some(&vec!["zephyr-build".to_string()])
        );
        assert_eq!(
            phase_deps.get("clean"),
            Some(&vec!["zephyr-clean".to_string()])
        );
        assert_eq!(
            phase_deps.get("flash"),
            Some(&vec!["zephyr-flash".to_string()])
        );
        assert_eq!(
            phase_deps.get("envsetup"),
            Some(&vec!["zephyr-envsetup".to_string()])
        );
    }

    #[test]
    fn test_generate_makefile_overlay_build_deps() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(
            build_dir.join("u-boot.mk"),
            "u-boot-build:\n\t$(MAKE) -C u-boot\n",
        )
        .expect("write u-boot.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: Some(vec!["@echo Top-level build".to_string()]),
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            makefile.contains("sdk-build: u-boot-build"),
            "Expected sdk-build to depend on u-boot-build, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("\t@echo Top-level build"),
            "Expected top-level build commands to be preserved"
        );
    }

    #[test]
    fn test_generate_makefile_overlay_clean_without_sdk_config() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(
            build_dir.join("linux.mk"),
            "linux-clean:\n\t$(MAKE) -C linux clean\n",
        )
        .expect("write linux.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "linux".to_string(),
                url: "https://example.com/linux.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            makefile.contains("sdk-clean: linux-clean"),
            "Expected sdk-clean to depend on linux-clean when no sdk.yml clean section, got:\n{}",
            makefile
        );
        // Should NOT contain the fallback echo message when overlay deps exist
        assert!(
            !makefile.contains("No clean commands defined in sdk.yml"),
            "Fallback message should be suppressed when overlay deps exist"
        );
    }

    #[test]
    fn test_generate_makefile_overlay_test_creates_sdk_test() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(build_dir.join("zephyr.mk"), "zephyr-test:\n\twest test\n")
            .expect("write zephyr.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "zephyr".to_string(),
                url: "https://example.com/zephyr.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            makefile.contains(".PHONY: all sdk-test"),
            "Expected sdk-test in PHONY when overlay test target exists"
        );
        assert!(
            makefile.contains("sdk-test: zephyr-test"),
            "Expected sdk-test to depend on zephyr-test, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("all: sdk-build sdk-test"),
            "Expected all target to include sdk-test when overlay test exists"
        );
    }

    #[test]
    fn test_generate_makefile_overlay_multiple_repos_mixed_phases() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(
            build_dir.join("u-boot.mk"),
            "u-boot-build:\n\t@echo uboot\n",
        )
        .expect("write u-boot.mk");
        std::fs::write(
            build_dir.join("linux.mk"),
            "linux-build:\n\t@echo linux\nlinux-clean:\n\t@echo linux-clean\n",
        )
        .expect("write linux.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![
                config::GitConfig {
                    name: "u-boot".to_string(),
                    url: "https://example.com/u-boot.git".to_string(),
                    commit: "main".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                    python_deps: None,
                },
                config::GitConfig {
                    name: "linux".to_string(),
                    url: "https://example.com/linux.git".to_string(),
                    commit: "main".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                    python_deps: None,
                },
            ],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        assert!(
            makefile.contains("sdk-build: u-boot-build linux-build"),
            "Expected sdk-build to depend on both overlay build targets, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("sdk-clean: linux-clean"),
            "Expected sdk-clean to depend on linux-clean only, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_generate_makefile_overlay_excluded_repo_ignored() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(
            build_dir.join("u-boot.mk"),
            "u-boot-build:\n\t@echo uboot\n",
        )
        .expect("write u-boot.mk");

        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: Some(config::MakefileInclude::Structured(
                config::MakefileIncludeConfig {
                    files: vec![],
                    exclude: vec!["u-boot".to_string()],
                },
            )),
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: None,
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        // Because u-boot is excluded, the overlay should not be included and
        // therefore no dependency should be added.
        assert!(
            !makefile.contains("u-boot-build"),
            "Expected excluded repo's overlay target to be ignored, got:\n{}",
            makefile
        );
    }

    #[test]
    fn test_generate_makefile_custom_phases() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let build_dir = tmp.path().join("build");
        std::fs::create_dir_all(&build_dir).expect("create build dir");
        std::fs::write(
            build_dir.join("u-boot.mk"),
            "u-boot-deploy:\n\t@echo deploying u-boot\n",
        )
        .expect("write u-boot.mk");

        // Custom phases are *added* to the five standard ones
        let config = config::SdkConfig {
            mirror: None,
            toolchains: None,
            install: None,
            gits: vec![config::GitConfig {
                name: "u-boot".to_string(),
                url: "https://example.com/u-boot.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
            }],
            copy_files: None,
            makefile_include: None,
            build_folder: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
            phases: Some(vec!["deploy".to_string()]),
            direnv: None,
        };

        let makefile = generate_makefile_content(&config, false, Some(tmp.path()));

        // Standard phases are always present
        assert!(
            makefile.contains("sdk-build:"),
            "Expected sdk-build target, got:\n{}",
            makefile
        );
        assert!(
            makefile.contains("sdk-test:"),
            "Expected sdk-test target (standard phase)"
        );
        assert!(
            makefile.contains("sdk-clean:"),
            "Expected sdk-clean target (standard phase)"
        );
        assert!(
            makefile.contains("sdk-flash:"),
            "Expected sdk-flash target (standard phase)"
        );

        // Custom phase added via phases:
        assert!(
            makefile.contains("sdk-deploy: u-boot-deploy"),
            "Expected sdk-deploy to depend on u-boot-deploy, got:\n{}",
            makefile
        );
    }
}
