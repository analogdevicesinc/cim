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

use serde_json::{json, Value};
use std::fs;
use std::path::Path;

/// Parse Makefile targets by extracting target names from the file
pub fn parse_makefile_targets(
    makefile_path: &Path,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if !makefile_path.exists() {
        return Err("Makefile not found".into());
    }

    let content = fs::read_to_string(makefile_path)?;
    let mut targets = Vec::new();

    for line in content.lines() {
        // Match target definitions: "sdk-test:", "test-repo:"
        // A target line starts with a non-whitespace character and contains ':'
        if let Some(colon_pos) = line.find(':') {
            let potential_target = &line[..colon_pos];

            // Skip lines that start with whitespace (commands in targets)
            if potential_target.is_empty()
                || potential_target.starts_with('\t')
                || potential_target.starts_with(' ')
            {
                continue;
            }

            // Skip comment lines
            if potential_target.starts_with('#') {
                continue;
            }

            let target_name = potential_target.trim();
            if !target_name.is_empty() {
                targets.push(target_name.to_string());
            }
        }
    }

    Ok(targets)
}

/// Create VS Code task definitions from Makefile targets
fn create_vscode_tasks(targets: &[String]) -> Vec<Value> {
    let mut tasks = Vec::new();

    // Map of SDK special targets with their display labels and groups
    let sdk_target_config: std::collections::HashMap<&str, (&str, &str, bool)> = [
        ("sdk-test", ("SDK: Test", "test", true)),
        ("sdk-build", ("SDK: Build", "build", true)),
        ("sdk-clean", ("SDK: Clean", "", false)),
        ("sdk-flash", ("SDK: Flash", "", false)),
        ("sdk-envsetup", ("SDK: Environment Setup", "", false)),
        ("install-all", ("Install: All Components", "", false)),
    ]
    .iter()
    .cloned()
    .collect();

    // Process SDK special targets first
    for (target_name, (label, group, is_default)) in &sdk_target_config {
        if targets.contains(&target_name.to_string()) {
            let mut task = json!({
                "label": label,
                "type": "shell",
                "command": "make",
                "args": [target_name],
                "presentation": {
                    "reveal": "always",
                    "panel": "shared",
                    "clear": false,
                    "showReuseMessage": true,
                    "group": "sdk-terminal"
                }
            });

            // Add group if specified
            if !group.is_empty() {
                let group_obj = if *is_default {
                    json!({
                        "kind": group,
                        "isDefault": true
                    })
                } else {
                    json!({
                        "kind": group
                    })
                };
                task["group"] = group_obj;
            }

            tasks.push(task);
        }
    }

    // Add install-* targets (excluding install-all which is already handled above)
    for target_name in targets {
        if target_name != "install-all" {
            if let Some(component) = target_name.strip_prefix("install-") {
                let label = format!("Install: {}", component);

                let task = json!({
                    "label": label,
                    "type": "shell",
                    "command": "make",
                    "args": [target_name],
                    "presentation": {
                        "reveal": "always",
                        "panel": "shared",
                        "clear": false,
                        "showReuseMessage": true,
                        "group": "sdk-terminal"
                    }
                });

                tasks.push(task);
            }
        }
    }

    tasks
}

/// Generate tasks.json file for VS Code
pub fn generate_tasks_json(
    workspace_path: &Path,
    makefile_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Parse Makefile targets
    let targets = parse_makefile_targets(makefile_path)?;

    // Create VS Code tasks from targets (filters to only SDK targets)
    let tasks = create_vscode_tasks(&targets);

    if tasks.is_empty() {
        // If no special SDK tasks found, just return without creating tasks.json
        // This is acceptable - the Makefile exists but has no sdk-* targets yet
        return Ok(());
    }

    // Create .vscode directory if it doesn't exist
    let vscode_dir = workspace_path.join(".vscode");
    if !vscode_dir.exists() {
        fs::create_dir_all(&vscode_dir)?;
    }

    // Create tasks.json structure
    let tasks_json = json!({
        "version": "2.0.0",
        "tasks": tasks
    });

    // Write tasks.json
    let tasks_json_path = vscode_dir.join("tasks.json");
    let tasks_json_str = serde_json::to_string_pretty(&tasks_json)?;
    fs::write(&tasks_json_path, tasks_json_str)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_makefile_targets_simple() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            ".PHONY: all\nall:\n\techo building\n\nsdk-test:\n\techo testing"
        )
        .unwrap();

        let targets = parse_makefile_targets(temp_file.path()).unwrap();
        assert!(targets.contains(&"all".to_string()));
        assert!(targets.contains(&"sdk-test".to_string()));
    }

    #[test]
    fn test_parse_makefile_targets_with_dependencies() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            "sdk-build: sdk-test\n\techo building\n\nsdk-test:\n\techo testing"
        )
        .unwrap();

        let targets = parse_makefile_targets(temp_file.path()).unwrap();
        assert!(targets.contains(&"sdk-build".to_string()));
        assert!(targets.contains(&"sdk-test".to_string()));
    }

    #[test]
    fn test_parse_makefile_targets_skips_comments() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            "# This is a comment\nsdk-test:\n\techo testing\n# Another comment\nall:"
        )
        .unwrap();

        let targets = parse_makefile_targets(temp_file.path()).unwrap();
        assert!(targets.contains(&"sdk-test".to_string()));
        assert!(targets.contains(&"all".to_string()));
        assert!(!targets.iter().any(|t| t.starts_with('#')));
    }

    #[test]
    fn test_create_vscode_tasks_all_sdk_targets() {
        let targets = vec![
            "sdk-test".to_string(),
            "sdk-build".to_string(),
            "sdk-clean".to_string(),
            "sdk-flash".to_string(),
        ];

        let tasks = create_vscode_tasks(&targets);
        assert_eq!(tasks.len(), 4);

        // Check that labels are correct
        let labels: Vec<&str> = tasks
            .iter()
            .filter_map(|t| t.get("label").and_then(|v| v.as_str()))
            .collect();

        assert!(labels.contains(&"SDK: Test"));
        assert!(labels.contains(&"SDK: Build"));
        assert!(labels.contains(&"SDK: Clean"));
        assert!(labels.contains(&"SDK: Flash"));
    }

    #[test]
    fn test_create_vscode_tasks_with_install_targets() {
        let targets = vec![
            "sdk-build".to_string(),
            "install-all".to_string(),
            "install-ninja".to_string(),
            "install-ccache".to_string(),
        ];

        let tasks = create_vscode_tasks(&targets);
        // Should have: sdk-build, install-all, install-ninja, install-ccache = 4 tasks
        assert_eq!(tasks.len(), 4);

        let labels: Vec<&str> = tasks
            .iter()
            .filter_map(|t| t.get("label").and_then(|v| v.as_str()))
            .collect();

        assert!(labels.contains(&"SDK: Build"));
        assert!(labels.contains(&"Install: All Components"));
        assert!(labels.contains(&"Install: ninja"));
        assert!(labels.contains(&"Install: ccache"));
    }

    #[test]
    fn test_create_vscode_tasks_partial() {
        let targets = vec!["sdk-test".to_string(), "sdk-build".to_string()];

        let tasks = create_vscode_tasks(&targets);
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn test_create_vscode_tasks_build_is_default() {
        let targets = vec!["sdk-build".to_string()];

        let tasks = create_vscode_tasks(&targets);
        assert_eq!(tasks.len(), 1);

        let task = &tasks[0];
        assert_eq!(
            task.get("label").and_then(|v| v.as_str()),
            Some("SDK: Build")
        );

        let group = task.get("group").and_then(|g| g.get("isDefault"));
        assert_eq!(group.and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_create_vscode_tasks_test_is_default() {
        let targets = vec!["sdk-test".to_string()];

        let tasks = create_vscode_tasks(&targets);
        assert_eq!(tasks.len(), 1);

        let task = &tasks[0];
        assert_eq!(
            task.get("label").and_then(|v| v.as_str()),
            Some("SDK: Test")
        );

        let group = task.get("group").and_then(|g| g.get("isDefault"));
        assert_eq!(group.and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_generate_tasks_json_creates_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let workspace_path = temp_dir.path();

        let makefile_path = workspace_path.join("Makefile");
        let mut makefile = NamedTempFile::new_in(workspace_path).unwrap();
        writeln!(
            makefile,
            "sdk-test:\n\techo test\n\nsdk-build:\n\techo build"
        )
        .unwrap();
        fs::rename(makefile.path(), &makefile_path).unwrap();

        generate_tasks_json(workspace_path, &makefile_path).unwrap();

        let tasks_json_path = workspace_path.join(".vscode").join("tasks.json");
        assert!(tasks_json_path.exists());

        let content = fs::read_to_string(&tasks_json_path).unwrap();
        assert!(content.contains("SDK: Test"));
        assert!(content.contains("SDK: Build"));
    }

    #[test]
    fn test_generate_tasks_json_empty_makefile() {
        let temp_dir = tempfile::tempdir().unwrap();
        let workspace_path = temp_dir.path();

        let makefile_path = workspace_path.join("Makefile");
        fs::write(&makefile_path, "# Empty Makefile\n").unwrap();

        let result = generate_tasks_json(workspace_path, &makefile_path);
        // Should succeed but not create any tasks (no SDK targets)
        assert!(result.is_ok());
    }

    #[test]
    fn test_generate_tasks_json_missing_makefile() {
        let temp_dir = tempfile::tempdir().unwrap();
        let workspace_path = temp_dir.path();
        let makefile_path = workspace_path.join("Makefile");

        let result = generate_tasks_json(workspace_path, &makefile_path);
        assert!(result.is_err());
    }
}
