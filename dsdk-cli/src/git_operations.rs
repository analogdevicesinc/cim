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

//! Centralized git operations using host git binary
//!
//! This module provides a consistent interface for all git operations
//! using the host system's git command rather than libgit2.

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub struct GitResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl GitResult {
    pub fn is_success(&self) -> bool {
        self.success
    }
}

/// Execute git command with consistent error handling
pub fn git_command(args: &[&str], cwd: Option<&Path>) -> Result<GitResult> {
    // Print verbose output showing the full command
    if crate::messages::is_verbose() {
        let cmd_str = format!("git {}", args.join(" "));
        if let Some(dir) = cwd {
            crate::messages::verbose(&format!("$ cd {} && {}", dir.display(), cmd_str));
        } else {
            crate::messages::verbose(&format!("$ {}", cmd_str));
        }
    }

    let mut cmd = Command::new("git");
    cmd.args(args);

    // Disable interactive authentication prompts (fixes Windows /dev/tty issue)
    // This prevents git from trying to access /dev/tty which doesn't exist on Windows
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_ASKPASS", "echo");

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let output = cmd
        .output()
        .map_err(|e| anyhow!("Failed to execute git {}: {}", args.join(" "), e))?;

    Ok(GitResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// Clone repository to specified path
/// On Windows, if SSH fails, automatically retries with HTTPS by temporarily disabling insteadOf config
pub fn clone_repo(url: &str, path: &Path, reference: Option<&Path>) -> Result<GitResult> {
    let mut args = vec!["clone".to_string()];

    if let Some(ref_path) = reference {
        args.push("--reference".to_string());
        args.push(ref_path.to_string_lossy().to_string());
    }

    args.push(url.to_string());
    args.push(path.to_string_lossy().to_string());

    let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let result = git_command(&args_str, None)?;

    // On Windows, if clone fails with SSH error, retry with HTTPS by disabling insteadOf
    #[cfg(target_os = "windows")]
    {
        if !result.is_success()
            && (result.stderr.contains("kex_exchange_identification")
                || result.stderr.contains("Connection to") && result.stderr.contains("port 22")
                || result
                    .stderr
                    .contains("Could not read from remote repository"))
        {
            crate::messages::info("SSH connection failed, retrying with HTTPS...");

            // Try again with git config to disable insteadOf rewriting
            let mut cmd = Command::new("git");
            cmd.args(&args_str);

            // Disable interactive authentication prompts
            cmd.env("GIT_TERMINAL_PROMPT", "0");
            cmd.env("GIT_ASKPASS", "echo");

            // Override insteadOf configuration to prevent SSH URL rewriting
            cmd.env("GIT_CONFIG_COUNT", "1");
            cmd.env("GIT_CONFIG_KEY_0", "url.https://github.com/.insteadOf");
            cmd.env("GIT_CONFIG_VALUE_0", "");

            let output = cmd
                .output()
                .map_err(|e| anyhow!("Failed to execute git {}: {}", args_str.join(" "), e))?;

            return Ok(GitResult {
                success: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }
    }

    Ok(result)
}

/// Clone repository with shallow depth
/// On Windows, if SSH fails, automatically retries with HTTPS by temporarily disabling insteadOf config
pub fn clone_repo_shallow(url: &str, path: &Path, depth: u32) -> Result<GitResult> {
    let depth_str = depth.to_string();
    let path_str = path.to_string_lossy().to_string();
    let args = vec!["clone", "--depth", &depth_str, url, &path_str];

    let result = git_command(&args, None)?;

    // On Windows, if clone fails with SSH error, retry with HTTPS by disabling insteadOf
    #[cfg(target_os = "windows")]
    {
        if !result.is_success()
            && (result.stderr.contains("kex_exchange_identification")
                || result.stderr.contains("Connection to") && result.stderr.contains("port 22")
                || result
                    .stderr
                    .contains("Could not read from remote repository"))
        {
            crate::messages::info("SSH connection failed, retrying with HTTPS...");

            // Try again with git config to disable insteadOf rewriting
            let mut cmd = Command::new("git");
            cmd.args(&args);

            // Disable interactive authentication prompts
            cmd.env("GIT_TERMINAL_PROMPT", "0");
            cmd.env("GIT_ASKPASS", "echo");

            // Override insteadOf configuration to prevent SSH URL rewriting
            cmd.env("GIT_CONFIG_COUNT", "1");
            cmd.env("GIT_CONFIG_KEY_0", "url.https://github.com/.insteadOf");
            cmd.env("GIT_CONFIG_VALUE_0", "");

            let output = cmd
                .output()
                .map_err(|e| anyhow!("Failed to execute git {}: {}", args.join(" "), e))?;

            return Ok(GitResult {
                success: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }
    }

    Ok(result)
}

/// Clone repository with single branch
pub fn clone_repo_single_branch(url: &str, path: &Path, branch: &str) -> Result<GitResult> {
    let path_str = path.to_string_lossy().to_string();
    let args = vec![
        "clone",
        "--single-branch",
        "--branch",
        branch,
        url,
        &path_str,
    ];
    git_command(&args, None)
}

/// Clone repository with shallow depth and single branch
pub fn clone_repo_shallow_single_branch(
    url: &str,
    path: &Path,
    branch: &str,
    depth: u32,
) -> Result<GitResult> {
    let depth_str = depth.to_string();
    let path_str = path.to_string_lossy().to_string();
    let args = vec![
        "clone",
        "--depth",
        &depth_str,
        "--single-branch",
        "--branch",
        branch,
        url,
        &path_str,
    ];
    git_command(&args, None)
}

/// Fetch from remote
pub fn fetch(repo_path: &Path, remote: Option<&str>) -> Result<GitResult> {
    let remote = remote.unwrap_or("origin");
    git_command(&["fetch", remote], Some(repo_path))
}

/// Fetch all remotes
pub fn fetch_all(repo_path: &Path) -> Result<GitResult> {
    git_command(&["fetch", "--all"], Some(repo_path))
}

/// Fetch all remotes with tags
/// Uses aggressive fetch strategy with retry logic to ensure all branches are discovered
pub fn fetch_all_with_tags(repo_path: &Path) -> Result<GitResult> {
    // Try aggressive fetch first (--force --prune to rediscover all remote branches)
    let result = git_command(
        &["fetch", "--all", "--tags", "--force", "--prune"],
        Some(repo_path),
    );

    if let Ok(ref git_result) = result {
        if git_result.is_success() {
            return result;
        }
    }

    // If --force fails, retry without --force but keep --prune
    let result = git_command(&["fetch", "--all", "--tags", "--prune"], Some(repo_path));

    if let Ok(ref git_result) = result {
        if git_result.is_success() {
            return result;
        }
    }

    // Final fallback: basic fetch without aggressive flags
    git_command(&["fetch", "--all", "--tags"], Some(repo_path))
}

/// Fetch tags from a specific remote
pub fn fetch_tags(repo_path: &Path, remote: Option<&str>) -> Result<GitResult> {
    let remote = remote.unwrap_or("origin");
    git_command(&["fetch", remote, "--tags"], Some(repo_path))
}

/// Checkout commit/branch/tag
pub fn checkout(repo_path: &Path, commit_ref: &str) -> Result<GitResult> {
    git_command(&["checkout", commit_ref], Some(repo_path))
}

/// List remote references
pub fn ls_remote(url: &str, heads: bool, tags: bool) -> Result<Vec<String>> {
    let mut args = vec!["ls-remote"];
    if heads {
        args.push("--heads");
    }
    if tags {
        args.push("--tags");
    }
    args.push(url);

    let result = git_command(&args, None)?;
    if !result.success {
        return Err(anyhow!("git ls-remote failed: {}", result.stderr));
    }

    let refs: Vec<String> = result
        .stdout
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1).map(|s| s.to_string()))
        .collect();

    Ok(refs)
}

/// List all tags from a local git repository
pub fn list_local_tags(repo_path: &Path) -> Result<Vec<String>> {
    let result = git_command(&["tag", "--list"], Some(repo_path))?;
    if !result.success {
        return Err(anyhow!("git tag --list failed: {}", result.stderr));
    }

    let tags: Vec<String> = result
        .stdout
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();

    Ok(tags)
}

/// List all local branches from a local git repository
pub fn list_local_branches(repo_path: &Path) -> Result<Vec<String>> {
    let result = git_command(&["branch", "--list"], Some(repo_path))?;
    if !result.success {
        return Err(anyhow!("git branch --list failed: {}", result.stderr));
    }

    let branches: Vec<String> = result
        .stdout
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .map(|line| {
            // Remove the '*' marker for current branch and trim whitespace
            if let Some(stripped) = line.strip_prefix("* ") {
                stripped.trim().to_string()
            } else {
                line.trim().to_string()
            }
        })
        .filter(|line| !line.is_empty())
        .collect();

    Ok(branches)
}

/// Check if reference is a branch
pub fn is_branch_reference(repo_path: &Path, commit_ref: &str) -> bool {
    // Check remote branch
    let remote_ref = format!("refs/remotes/origin/{}", commit_ref);
    if git_command(&["show-ref", "--verify", &remote_ref], Some(repo_path)).is_ok_and(|r| r.success)
    {
        return true;
    }

    // Check local branch
    let local_ref = format!("refs/heads/{}", commit_ref);
    git_command(&["show-ref", "--verify", &local_ref], Some(repo_path)).is_ok_and(|r| r.success)
}

/// Get latest commit hash for branch
pub fn get_latest_commit_for_branch(repo_path: &Path, branch_name: &str) -> Option<String> {
    // Try remote branch first
    let remote_branch = format!("origin/{}", branch_name);
    if let Ok(result) = git_command(&["rev-parse", &remote_branch], Some(repo_path)) {
        if result.success {
            return Some(result.stdout.trim().to_string());
        }
    }

    // Fallback to local branch
    if let Ok(result) = git_command(&["rev-parse", branch_name], Some(repo_path)) {
        if result.success {
            return Some(result.stdout.trim().to_string());
        }
    }

    None
}

/// Get latest commit hash for remote branch after fetch (works in bare repos)
pub fn get_latest_commit_for_remote_branch(
    repo_path: &Path,
    remote: &str,
    branch_name: &str,
) -> Option<String> {
    // In bare repos after fetch, we can use origin/branch notation
    let remote_branch = format!("{}/{}", remote, branch_name);
    if let Ok(result) = git_command(&["rev-parse", &remote_branch], Some(repo_path)) {
        if result.success {
            return Some(result.stdout.trim().to_string());
        }
    }

    // For mirror repos, branches are stored directly as refs/heads/{branch}
    // Try refs/heads/{branch} directly (this works for mirror repos)
    let heads_ref = format!("refs/heads/{}", branch_name);
    if let Ok(result) = git_command(&["rev-parse", &heads_ref], Some(repo_path)) {
        if result.success {
            return Some(result.stdout.trim().to_string());
        }
    }

    // Try just the branch name as a final fallback
    if let Ok(result) = git_command(&["rev-parse", branch_name], Some(repo_path)) {
        if result.success {
            return Some(result.stdout.trim().to_string());
        }
    }

    None
}

/// Get current commit hash
pub fn get_current_commit(repo_path: &Path) -> Result<String> {
    let result = git_command(&["rev-parse", "HEAD"], Some(repo_path))?;
    if !result.success {
        return Err(anyhow!("Failed to get current commit: {}", result.stderr));
    }
    Ok(result.stdout.trim().to_string())
}

/// Check if repository is dirty (has uncommitted changes)
pub fn is_repo_dirty(repo_path: &Path) -> Result<bool> {
    let result = git_command(&["status", "--porcelain"], Some(repo_path))?;
    if !result.success {
        return Err(anyhow!(
            "Failed to check repository status: {}",
            result.stderr
        ));
    }
    Ok(!result.stdout.trim().is_empty())
}

/// Update reference (for mirrors)
pub fn update_ref(repo_path: &Path, ref_name: &str, commit_hash: &str) -> Result<GitResult> {
    git_command(&["update-ref", ref_name, commit_hash], Some(repo_path))
}

/// Create branch
pub fn create_branch(
    repo_path: &Path,
    branch_name: &str,
    commit_ref: Option<&str>,
) -> Result<GitResult> {
    let mut args = vec!["branch", branch_name];
    if let Some(commit) = commit_ref {
        args.push(commit);
    }
    git_command(&args, Some(repo_path))
}

/// Create branch with force flag
pub fn create_branch_force(
    repo_path: &Path,
    branch_name: &str,
    commit_ref: &str,
) -> Result<GitResult> {
    git_command(&["branch", "-f", branch_name, commit_ref], Some(repo_path))
}

/// Clone repository as bare (for mirrors)
pub fn clone_bare(url: &str, path: &Path) -> Result<GitResult> {
    let path_str = path.to_string_lossy().to_string();
    let args = vec!["clone", "--bare", url, &path_str];
    git_command(&args, None)
}

/// Clone repository as mirror (for mirroring)
pub fn clone_mirror(url: &str, path: &Path) -> Result<GitResult> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow!("Failed to create parent directory: {}", e))?;
    }

    let path_str = path.to_string_lossy().to_string();
    let args = vec!["clone", "--mirror", url, &path_str];
    git_command(&args, None)
}

/// Get the URL of a remote
pub fn get_remote_url(repo_path: &Path, remote_name: &str) -> Result<String> {
    let result = git_command(&["remote", "get-url", remote_name], Some(repo_path))?;
    if result.is_success() {
        Ok(result.stdout.trim().to_string())
    } else {
        Err(anyhow!("Failed to get URL for remote '{}'", remote_name))
    }
}

/// Normalize a git URL for comparison purposes.
/// Strips trailing `/` and `.git`, then lowercases.
pub fn normalize_git_url(url: &str) -> String {
    let mut s = url.trim().to_lowercase();
    // Strip trailing slashes
    while s.ends_with('/') {
        s.pop();
    }
    // Strip trailing .git
    if let Some(stripped) = s.strip_suffix(".git") {
        s = stripped.to_string();
    }
    s
}

/// Generate a truncated hash of a URL for mirror disambiguation.
/// Returns the first 8 characters of the SHA-256 hash.
pub fn hash_url(url: &str) -> String {
    let normalized = normalize_git_url(url);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)[..8].to_string()
}

/// Extract `org-repo` slug from a git URL for disambiguation.
/// E.g., `https://github.com/analogdevicesinc/u-boot-xlnx.git` -> `analogdevicesinc-u-boot-xlnx`
pub fn extract_org_and_repo(url: &str) -> String {
    let normalized = normalize_git_url(url);
    // Handle ssh-style URLs like git@github.com:org/repo
    let path_part = if let Some((_host, path)) = normalized.split_once(':') {
        if !normalized.contains("://") {
            path.to_string()
        } else {
            normalized.clone()
        }
    } else {
        normalized.clone()
    };
    let segments: Vec<&str> = path_part.rsplit('/').take(2).collect();
    match segments.len() {
        2 => format!("{}-{}", segments[1], segments[0]),
        1 => segments[0].to_string(),
        _ => normalized,
    }
}

/// Set remote URL
pub fn remote_set_url(repo_path: &Path, remote_name: &str, url: &str) -> Result<GitResult> {
    git_command(&["remote", "set-url", remote_name, url], Some(repo_path))
}

/// Add remote
pub fn remote_add(repo_path: &Path, remote_name: &str, url: &str) -> Result<GitResult> {
    git_command(&["remote", "add", remote_name, url], Some(repo_path))
}

/// Initialize git repository
pub fn init_repo(path: &Path, bare: bool) -> Result<GitResult> {
    let path_str = path.to_string_lossy().to_string();
    let mut args = vec!["init"];
    if bare {
        args.push("--bare");
    }
    // Set default branch to 'main' for consistency across different git configurations
    args.push("-b");
    args.push("main");
    args.push(&path_str);
    git_command(&args, None)
}

/// Add files to staging
pub fn add_files(repo_path: &Path, files: &[&str]) -> Result<GitResult> {
    let mut args = vec!["add"];
    args.extend(files);
    git_command(&args, Some(repo_path))
}

/// Add all files to staging
pub fn add_all(repo_path: &Path) -> Result<GitResult> {
    git_command(&["add", "."], Some(repo_path))
}

/// Commit changes
pub fn commit(repo_path: &Path, message: &str) -> Result<GitResult> {
    git_command(&["commit", "-m", message], Some(repo_path))
}

/// Push changes
pub fn push(repo_path: &Path, remote: Option<&str>, branch: Option<&str>) -> Result<GitResult> {
    let mut args = vec!["push"];
    if let Some(r) = remote {
        args.push(r);
    }
    if let Some(b) = branch {
        args.push(b);
    }
    git_command(&args, Some(repo_path))
}

/// Push all branches
pub fn push_all(repo_path: &Path, remote: &str) -> Result<GitResult> {
    git_command(&["push", remote, "--all"], Some(repo_path))
}

/// Push all tags
pub fn push_tags(repo_path: &Path, remote: &str) -> Result<GitResult> {
    git_command(&["push", remote, "--tags"], Some(repo_path))
}

/// Create tag
pub fn create_tag(repo_path: &Path, tag_name: &str) -> Result<GitResult> {
    git_command(&["tag", tag_name], Some(repo_path))
}

/// List tags
pub fn list_tags(repo_path: &Path, pattern: Option<&str>) -> Result<Vec<String>> {
    let mut args = vec!["tag", "-l"];
    if let Some(p) = pattern {
        args.push(p);
    }

    let result = git_command(&args, Some(repo_path))?;
    if !result.success {
        return Err(anyhow!("git tag -l failed: {}", result.stderr));
    }

    let tags: Vec<String> = result
        .stdout
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();

    Ok(tags)
}

/// Configure git setting
pub fn config(repo_path: &Path, key: &str, value: &str) -> Result<GitResult> {
    git_command(&["config", key, value], Some(repo_path))
}

/// Enhanced error message for git failures
pub fn enhanced_git_error(operation: &str, result: &GitResult, context: Option<&str>) -> String {
    let context_str = context.map_or(String::new(), |c| format!(" ({})", c));
    format!(
        "Git {} failed{}: {}\nCommand output: {}",
        operation,
        context_str,
        result.stderr.trim(),
        result.stdout.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_command_success() {
        let result = git_command(&["--version"], None).unwrap();
        assert!(result.is_success());
        assert!(result.stdout.contains("git version"));
    }

    #[test]
    fn test_git_command_failure() {
        let result = git_command(&["invalid-command"], None).unwrap();
        assert!(!result.is_success());
        assert!(!result.stderr.is_empty());
    }

    #[test]
    fn test_enhanced_git_error() {
        let result = GitResult {
            success: false,
            stdout: "some output".to_string(),
            stderr: "error message".to_string(),
        };

        let error_msg = enhanced_git_error("clone", &result, Some("test context"));
        assert!(error_msg.contains("Git clone failed"));
        assert!(error_msg.contains("test context"));
        assert!(error_msg.contains("error message"));
    }

    #[test]
    fn test_normalize_git_url() {
        assert_eq!(
            normalize_git_url("https://github.com/u-boot/u-boot.git"),
            "https://github.com/u-boot/u-boot"
        );
        assert_eq!(
            normalize_git_url("https://github.com/u-boot/u-boot.git/"),
            "https://github.com/u-boot/u-boot"
        );
        assert_eq!(
            normalize_git_url("https://GitHub.com/U-Boot/U-Boot.GIT"),
            "https://github.com/u-boot/u-boot"
        );
        assert_eq!(
            normalize_git_url("git@github.com:org/repo.git"),
            "git@github.com:org/repo"
        );
        assert_eq!(
            normalize_git_url("  https://example.com/repo  "),
            "https://example.com/repo"
        );
    }

    #[test]
    fn test_hash_url() {
        let hash1 = hash_url("https://github.com/u-boot/u-boot.git");
        assert_eq!(hash1.len(), 8);
        // Same URL should produce same hash
        assert_eq!(hash1, hash_url("https://github.com/u-boot/u-boot.git"));
        // Different URL should produce different hash
        let hash2 = hash_url("https://github.com/analogdevicesinc/u-boot-xlnx.git");
        assert_ne!(hash1, hash2);
        // URLs that normalize to same should produce same hash
        assert_eq!(
            hash_url("https://github.com/u-boot/u-boot.git"),
            hash_url("https://github.com/u-boot/u-boot.git/")
        );
        // Test specific URLs from test cases
        let hash_analog = hash_url("https://github.com/analogdevicesinc/u-boot-xlnx.git");
        let hash_someorg = hash_url("https://github.com/someorg/u-boot-xlnx.git");
        assert_ne!(
            hash_analog, hash_someorg,
            "Different orgs should have different hashes"
        );
    }

    #[test]
    fn test_extract_org_and_repo() {
        assert_eq!(
            extract_org_and_repo("https://github.com/u-boot/u-boot.git"),
            "u-boot-u-boot"
        );
        assert_eq!(
            extract_org_and_repo("https://github.com/analogdevicesinc/u-boot-xlnx.git"),
            "analogdevicesinc-u-boot-xlnx"
        );
        assert_eq!(
            extract_org_and_repo("git@github.com:org/repo.git"),
            "org-repo"
        );
    }

    #[test]
    fn test_ls_remote_format() {
        // Test with a known public repository
        if let Ok(refs) = ls_remote("https://github.com/git/git.git", true, false) {
            assert!(!refs.is_empty());
            // Check that we get proper ref format
            assert!(refs.iter().any(|r| r.starts_with("refs/heads/")));
        }
    }

    #[test]
    fn test_fetch_tags_functionality() {
        use std::fs;
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let upstream_dir = temp_dir.path().join("upstream");
        let mirror_dir = temp_dir.path().join("mirror");

        // Create upstream repository with a tag
        fs::create_dir_all(&upstream_dir).unwrap();
        init_repo(&upstream_dir, false).unwrap();

        config(&upstream_dir, "user.name", "Test User").unwrap();
        config(&upstream_dir, "user.email", "test@example.com").unwrap();

        fs::write(upstream_dir.join("file.txt"), "test content\n").unwrap();
        add_all(&upstream_dir).unwrap();
        commit(&upstream_dir, "Initial commit").unwrap();
        create_tag(&upstream_dir, "v1.0.0").unwrap();

        // Create bare clone (mirror)
        clone_bare(&format!("file://{}", upstream_dir.display()), &mirror_dir).unwrap();

        // Verify tag was cloned
        let tags = list_local_tags(&mirror_dir).unwrap();
        assert!(
            tags.contains(&"v1.0.0".to_string()),
            "Tag v1.0.0 should be present after clone --bare"
        );

        // Add another tag to upstream
        fs::write(upstream_dir.join("file2.txt"), "more content\n").unwrap();
        add_all(&upstream_dir).unwrap();
        commit(&upstream_dir, "Second commit").unwrap();
        create_tag(&upstream_dir, "v2.0.0").unwrap();

        // Test fetch_all_with_tags
        let result = fetch_all_with_tags(&mirror_dir).unwrap();
        assert!(result.is_success(), "fetch_all_with_tags should succeed");

        // Verify new tag was fetched
        let updated_tags = list_local_tags(&mirror_dir).unwrap();
        assert!(
            updated_tags.contains(&"v1.0.0".to_string()),
            "Original tag should still be present"
        );
        assert!(
            updated_tags.contains(&"v2.0.0".to_string()),
            "New tag should be fetched"
        );

        // Test fetch_tags specifically
        fs::write(upstream_dir.join("file3.txt"), "even more content\n").unwrap();
        add_all(&upstream_dir).unwrap();
        commit(&upstream_dir, "Third commit").unwrap();
        create_tag(&upstream_dir, "v3.0.0").unwrap();

        let tags_result = fetch_tags(&mirror_dir, Some("origin")).unwrap();
        assert!(tags_result.is_success(), "fetch_tags should succeed");

        let final_tags = list_local_tags(&mirror_dir).unwrap();
        assert!(
            final_tags.contains(&"v3.0.0".to_string()),
            "Latest tag should be fetched via fetch_tags"
        );
        assert_eq!(final_tags.len(), 3, "Should have all three tags");
    }
}
