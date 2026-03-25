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

use dsdk_cli::workspace::get_current_workspace;
use dsdk_cli::{config, messages, vscode_tasks_manager};

const WORKSPACE_VARIABLE: &str = "WORKSPACE := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))";

/// Generate a Makefile from the SDK configuration
pub(crate) fn handle_makefile_command() {
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

    // Resolve dividers preference from user config
    let mut dividers = true;

    // Load and apply user config overrides if present
    match config::UserConfig::load() {
        Ok(Some(user_config)) => {
            user_config.apply_to_sdk_config(&mut sdk_config, false);
            if let Some(true) = user_config.no_dividers {
                dividers = false;
            }
        }
        Ok(None) => {}
        Err(e) => {
            messages::error(&format!("Warning: Failed to load user config: {}", e));
        }
    }

    let makefile = generate_makefile_content(&sdk_config, dividers);

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

/// Generate the content of the Makefile from SDK configuration
pub(crate) fn generate_makefile_content<T: config::SdkConfigCore>(
    sdk_config: &T,
    dividers: bool,
) -> String {
    let mut makefile = String::new();

    if dividers {
        makefile.push_str(&makefile_divider("Workspace and manifest variables"));
    }

    makefile.push_str(WORKSPACE_VARIABLE);
    makefile.push_str("\n\n");

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
            makefile.push_str(&format!("{} ?= {}\n", key, value));
        }
        makefile.push('\n');
    }

    // Add makefile includes after variables so included files can reference them
    if let Some(makefile_includes) = sdk_config.makefile_include() {
        if !makefile_includes.is_empty() {
            if dividers {
                makefile.push_str(&makefile_divider("Makefile includes"));
            }
            for include_line in makefile_includes {
                makefile.push_str(&format!("-{}\n", include_line));
            }
            makefile.push('\n');
        }
    }

    if dividers {
        makefile.push_str(&makefile_divider("High-level SDK targets"));
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
    if !sdk_config.gits().is_empty() && dividers {
        makefile.push_str(&makefile_divider("Git repository targets"));
    }
    for git in sdk_config.gits() {
        add_makefile_target(&mut makefile, git);
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

/// Add a single target to the Makefile
pub(crate) fn add_makefile_target(makefile: &mut String, git: &config::GitConfig) {
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
}

/// Add the sdk-envsetup target to the Makefile
pub(crate) fn add_envsetup_target(makefile: &mut String, envsetup_target: &config::SdkTarget) {
    // Add target with dependencies
    if let Some(deps) = envsetup_target.depends_on() {
        makefile.push_str(&format!("sdk-envsetup: {}\n", deps.join(" ")));
    } else {
        makefile.push_str("sdk-envsetup:\n");
    }

    for command in envsetup_target.commands() {
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

    makefile.push('\n');
}

/// Add the sdk-test target to the Makefile
pub(crate) fn add_test_target(makefile: &mut String, test_target: &config::SdkTarget) {
    // Add target with dependencies
    if let Some(deps) = test_target.depends_on() {
        makefile.push_str(&format!("sdk-test: {}\n", deps.join(" ")));
    } else {
        makefile.push_str("sdk-test:\n");
    }

    for command in test_target.commands() {
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

    makefile.push('\n');
}

/// Add the sdk-clean target to the Makefile
pub(crate) fn add_clean_target(makefile: &mut String, clean_target: &config::SdkTarget) {
    // Add target with dependencies
    if let Some(deps) = clean_target.depends_on() {
        makefile.push_str(&format!("sdk-clean: {}\n", deps.join(" ")));
    } else {
        makefile.push_str("sdk-clean:\n");
    }

    for command in clean_target.commands() {
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

    makefile.push('\n');
}

/// Add the sdk-build target to the Makefile
pub(crate) fn add_build_target(makefile: &mut String, build_target: &config::SdkTarget) {
    // Add target with dependencies
    if let Some(deps) = build_target.depends_on() {
        makefile.push_str(&format!("sdk-build: {}\n", deps.join(" ")));
    } else {
        makefile.push_str("sdk-build:\n");
    }

    for command in build_target.commands() {
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

    makefile.push('\n');
}

/// Add the sdk-flash target to the Makefile
pub(crate) fn add_flash_target(makefile: &mut String, flash_target: &config::SdkTarget) {
    // Add target with dependencies
    if let Some(deps) = flash_target.depends_on() {
        makefile.push_str(&format!("sdk-flash: {}\n", deps.join(" ")));
    } else {
        makefile.push_str("sdk-flash:\n");
    }

    for command in flash_target.commands() {
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
    use std::path::PathBuf;

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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);
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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);
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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);
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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);
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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);

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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);

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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);

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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);

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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);

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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);

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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);

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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);

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
            variables: None,
        };

        let makefile = generate_makefile_content(&config, false);

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
    fn test_generate_makefile_with_variables() {
        let mut vars = std::collections::HashMap::new();
        vars.insert(
            "DOCKER_DEFAULT_PLATFORM".to_string(),
            "linux/amd64".to_string(),
        );

        let config = config::SdkConfig {
            toolchains: None,
            install: Some(vec![config::InstallConfig {
                name: "devcontainer".to_string(),
                depends_on: None,
                sentinel: Some("opt/.devcontainer-installed".to_string()),
                commands: Some(vec![
                    "cd repo && DOCKER_DEFAULT_PLATFORM=${{ DOCKER_DEFAULT_PLATFORM }} ./run.sh --new".to_string(),
                ]),
            }]),
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(vars),
        };

        let makefile = generate_makefile_content(&config, false);

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
            toolchains: None,
            install: None,
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![],
            copy_files: None,
            makefile_include: Some(vec!["include build/extra.mk".to_string()]),
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(vars),
        };

        let makefile = generate_makefile_content(&config, false);

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
        };

        let mut vars = std::collections::HashMap::new();
        vars.insert("MY_VAR".to_string(), "value".to_string());

        let config = config::SdkConfig {
            toolchains: None,
            install: Some(vec![config::InstallConfig {
                name: "my-tool".to_string(),
                depends_on: None,
                sentinel: Some("opt/.my-tool-installed".to_string()),
                commands: Some(vec!["echo install".to_string()]),
            }]),
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![git_config],
            copy_files: None,
            makefile_include: Some(vec!["include extra.mk".to_string()]),
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(vars),
        };

        let makefile = generate_makefile_content(&config, true);

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
        };

        let mut vars = std::collections::HashMap::new();
        vars.insert("MY_VAR".to_string(), "value".to_string());

        let config = config::SdkConfig {
            toolchains: None,
            install: Some(vec![config::InstallConfig {
                name: "my-tool".to_string(),
                depends_on: None,
                sentinel: Some("opt/.my-tool-installed".to_string()),
                commands: Some(vec!["echo install".to_string()]),
            }]),
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![git_config],
            copy_files: None,
            makefile_include: Some(vec!["include extra.mk".to_string()]),
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: Some(vars),
        };

        let makefile = generate_makefile_content(&config, false);

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
}
