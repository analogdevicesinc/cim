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

use dsdk_cli::download::{
    compute_file_sha256, copy_single_file, download_file_with_cache, generate_cache_path,
    DownloadConfig,
};
use dsdk_cli::workspace::{
    copy_dir_recursive, expand_config_mirror_path, expand_env_vars, get_current_workspace, is_url,
    resolve_config_source_dir_from_marker,
};
use dsdk_cli::{config, git_operations, messages};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

/// Handle the release command
pub(crate) fn handle_release_command(
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
pub(crate) fn get_current_commit_hash(repo_path: &std::path::Path) -> Option<String> {
    if !repo_path.exists() {
        return None;
    }

    git_operations::get_current_commit(repo_path).ok()
}

/// Generate a release configuration file
pub(crate) fn generate_release_config(
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

pub(crate) fn ensure_file_in_mirror(
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
pub(crate) fn update_sdk_yaml_hash(
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
pub(crate) fn handle_copy_files_hash_command(
    file_filter: Option<&str>,
    dry_run: bool,
    verbose: bool,
) {
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
pub(crate) fn handle_sync_files_hash_command(
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

        let expanded_dest = expand_env_vars(&copy_file.dest);
        let dest_path = workspace_path.join(&expanded_dest);
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

        let expanded_dest = expand_env_vars(&copy_file.dest);
        let dest_path = workspace_path.join(&expanded_dest);

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
}
