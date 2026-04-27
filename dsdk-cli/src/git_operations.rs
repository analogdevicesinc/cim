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

//! Centralized git operations module
//!
//! This module provides the public API for all git operations in cim.
//! All operations use libgit2 natively via the `git2` crate.

use anyhow::{anyhow, Context, Result};
use git2::{
    AutotagOption, BranchType, Cred, CredentialType, FetchOptions, IndexAddOption, ObjectType,
    ProxyOptions, RemoteCallbacks, Repository, RepositoryInitOptions, StatusOptions,
};
use sha2::{Digest, Sha256};
use std::path::Path;

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

// ---------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------

/// Return the user's home directory in a cross-platform way.
///
/// Checks `HOME` (Unix) first, then `USERPROFILE` (Windows).
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// Build `RemoteCallbacks` with credential handling.
///
/// libgit2 does not automatically resolve credentials the way the git
/// binary does — the application must provide a callback that returns
/// credentials when the server requests authentication.
///
/// The callback inspects `allowed_types` and tries, in order:
/// 1. SSH agent (works on Linux, macOS, Windows with OpenSSH agent)
/// 2. SSH key from default locations (~/.ssh/id_ed25519, id_rsa, …)
/// 3. `credential.helper` from `.gitconfig` (handles OS-specific stores:
///    macOS Keychain, Windows GCM, Linux store/cache)
/// 4. Default credentials (e.g. NTLM/Kerberos for local URLs)
fn make_remote_callbacks<'a>() -> RemoteCallbacks<'a> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|url, username_from_url, allowed_types| {
        // 1. SSH agent
        if allowed_types.contains(CredentialType::SSH_KEY) {
            if let Some(username) = username_from_url {
                if let Ok(cred) = Cred::ssh_key_from_agent(username) {
                    return Ok(cred);
                }

                // 2. SSH key files from home directory (cross-platform)
                if let Some(home) = home_dir() {
                    for key_name in &["id_ed25519", "id_rsa", "id_ecdsa"] {
                        let key_path = home.join(".ssh").join(key_name);
                        if key_path.exists() {
                            if let Ok(cred) = Cred::ssh_key(username, None, &key_path, None) {
                                return Ok(cred);
                            }
                        }
                    }
                }
            }
        }

        // 3. Credential helper from .gitconfig (GCM, osxkeychain, store, etc.)
        if allowed_types.contains(CredentialType::USER_PASS_PLAINTEXT) {
            if let Ok(config) = git2::Config::open_default() {
                if let Ok(cred) = Cred::credential_helper(&config, url, username_from_url) {
                    return Ok(cred);
                }
            }
        }

        // 4. Default credentials (e.g. for local file:// URLs)
        if allowed_types.contains(CredentialType::DEFAULT) {
            return Cred::default();
        }

        Err(git2::Error::from_str("no suitable credentials found"))
    });
    callbacks
}

/// Build `FetchOptions` with standard callbacks and proxy settings.
fn make_fetch_options<'a>() -> FetchOptions<'a> {
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(make_remote_callbacks());
    let mut proxy = ProxyOptions::new();
    proxy.auto();
    fetch_opts.proxy_options(proxy);
    fetch_opts
}

/// Helper to produce a successful `GitResult`.
fn ok_result() -> GitResult {
    GitResult {
        success: true,
        stdout: String::new(),
        stderr: String::new(),
    }
}

/// Log a git-equivalent command in verbose mode, matching the old
/// `$ cd /path && git <args>` format for debugging and user visibility.
fn log_git(cwd: Option<&Path>, args: &str) {
    if crate::messages::is_verbose() {
        if let Some(dir) = cwd {
            crate::messages::verbose(&format!("$ cd {} && git {}", dir.display(), args));
        } else {
            crate::messages::verbose(&format!("$ git {}", args));
        }
    }
}

// ---------------------------------------------------------------
// Clone operations
// ---------------------------------------------------------------

/// Clone repository to specified path.
///
/// When `reference` is provided, the reference repo's object store is
/// registered as an alternate **before** the fetch runs — mirroring
/// `git clone --reference <ref_path>`.  This lets libgit2 skip objects
/// that already exist in the reference repo, making local-mirror clones
/// near-instant.
pub fn clone_repo(url: &str, path: &Path, reference: Option<&Path>) -> Result<GitResult> {
    if let Some(ref_path) = reference {
        log_git(
            None,
            &format!(
                "clone --reference {} {} {}",
                ref_path.display(),
                url,
                path.display()
            ),
        );
    } else {
        log_git(None, &format!("clone {} {}", url, path.display()));
    }

    let callbacks = make_remote_callbacks();
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch_opts);

    if let Some(ref_path) = reference {
        // Resolve the objects directory of the reference repo.
        let alt_path = if ref_path.join("objects").exists() {
            ref_path.join("objects")
        } else {
            ref_path.join(".git").join("objects")
        };
        let alt_str = alt_path.to_string_lossy().to_string();

        // `remote_create` is called with the newly-initialised repo
        // *before* the fetch.  We inject the alternate here so the
        // fetch can skip objects already present in the reference repo.
        builder.remote_create(move |repo, name, url_arg| {
            if let Ok(odb) = repo.odb() {
                let _ = odb.add_disk_alternate(&alt_str);
            }
            repo.remote(name, url_arg)
        });
    }

    builder
        .clone(url, path)
        .with_context(|| format!("Failed to clone {} to {}", url, path.display()))?;

    // `add_disk_alternate` only affects the in-memory ODB of the repo
    // instance used during clone.  Persist the alternate to disk so that
    // future `Repository::open()` calls (e.g. checkout) also find the
    // objects.  This matches what `git clone --reference` writes.
    if let Some(ref_path) = reference {
        let alt_path = if ref_path.join("objects").exists() {
            ref_path.join("objects")
        } else {
            ref_path.join(".git").join("objects")
        };
        let alternates_file = path
            .join(".git")
            .join("objects")
            .join("info")
            .join("alternates");
        if let Some(parent) = alternates_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&alternates_file, format!("{}\n", alt_path.display())).with_context(
            || {
                format!(
                    "Failed to write alternates file at {}",
                    alternates_file.display()
                )
            },
        )?;
    }

    Ok(ok_result())
}

/// Clone repository with shallow depth
pub fn clone_repo_shallow(url: &str, path: &Path, depth: u32) -> Result<GitResult> {
    log_git(
        None,
        &format!("clone --depth {} {} {}", depth, url, path.display()),
    );

    let mut fetch_opts = make_fetch_options();
    fetch_opts.depth(depth as i32);

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch_opts);

    builder.clone(url, path).with_context(|| {
        format!(
            "Failed to shallow-clone {} (depth {}) to {}",
            url,
            depth,
            path.display()
        )
    })?;

    Ok(ok_result())
}

/// Clone repository with single branch
pub fn clone_repo_single_branch(url: &str, path: &Path, branch: &str) -> Result<GitResult> {
    log_git(
        None,
        &format!(
            "clone --single-branch --branch {} {} {}",
            branch,
            url,
            path.display()
        ),
    );

    let fetch_opts = make_fetch_options();

    // Restrict fetch to a single branch via a narrow refspec.
    let refspec = format!("+refs/heads/{}:refs/remotes/origin/{}", branch, branch);
    let url_owned = url.to_string();
    let refspec_owned = refspec.clone();

    let mut builder = git2::build::RepoBuilder::new();
    builder.branch(branch);
    builder.fetch_options(fetch_opts);
    builder.remote_create(move |repo, _name, url_arg| {
        let url_to_use = if url_arg.is_empty() {
            url_owned.as_str()
        } else {
            url_arg
        };
        repo.remote_with_fetch("origin", url_to_use, &refspec_owned)
    });

    builder.clone(url, path).with_context(|| {
        format!(
            "Failed to single-branch clone {} (branch {}) to {}",
            url,
            branch,
            path.display()
        )
    })?;

    Ok(ok_result())
}

/// Clone repository with shallow depth and single branch
pub fn clone_repo_shallow_single_branch(
    url: &str,
    path: &Path,
    branch: &str,
    depth: u32,
) -> Result<GitResult> {
    log_git(
        None,
        &format!(
            "clone --depth {} --single-branch --branch {} {} {}",
            depth,
            branch,
            url,
            path.display()
        ),
    );

    let mut fetch_opts = make_fetch_options();
    fetch_opts.depth(depth as i32);

    let refspec = format!("+refs/heads/{}:refs/remotes/origin/{}", branch, branch);
    let url_owned = url.to_string();
    let refspec_owned = refspec.clone();

    let mut builder = git2::build::RepoBuilder::new();
    builder.branch(branch);
    builder.fetch_options(fetch_opts);
    builder.remote_create(move |repo, _name, url_arg| {
        let url_to_use = if url_arg.is_empty() {
            url_owned.as_str()
        } else {
            url_arg
        };
        repo.remote_with_fetch("origin", url_to_use, &refspec_owned)
    });

    builder.clone(url, path).with_context(|| {
        format!(
            "Failed to shallow single-branch clone {} (branch {}, depth {}) to {}",
            url,
            branch,
            depth,
            path.display()
        )
    })?;

    Ok(ok_result())
}

/// Fetch from remote
pub fn fetch(repo_path: &Path, remote: Option<&str>) -> Result<GitResult> {
    let r = remote.unwrap_or("origin");
    log_git(Some(repo_path), &format!("fetch {}", r));

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let remote_name = remote.unwrap_or("origin");
    let mut remote_obj = repo
        .find_remote(remote_name)
        .with_context(|| format!("Remote '{}' not found", remote_name))?;

    let mut fetch_opts = make_fetch_options();
    remote_obj
        .fetch(&[] as &[&str], Some(&mut fetch_opts), None)
        .with_context(|| format!("Fetch from '{}' failed", remote_name))?;

    Ok(ok_result())
}

/// Fetch all remotes
pub fn fetch_all(repo_path: &Path) -> Result<GitResult> {
    log_git(Some(repo_path), "fetch --all");

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;

    let remote_names: Vec<String> = repo
        .remotes()?
        .iter()
        .filter_map(|r| r.map(String::from))
        .collect();

    for name in &remote_names {
        let mut remote = repo.find_remote(name)?;
        let mut fetch_opts = make_fetch_options();
        remote.fetch(&[] as &[&str], Some(&mut fetch_opts), None)?;
    }

    Ok(ok_result())
}

/// Fetch a specific refspec, optionally with a depth limit
pub fn fetch_ref(
    repo_path: &Path,
    remote: &str,
    refspec: &str,
    depth: Option<u32>,
) -> Result<GitResult> {
    if let Some(d) = depth {
        log_git(
            Some(repo_path),
            &format!("fetch {} {} --depth {}", remote, refspec, d),
        );
    } else {
        log_git(Some(repo_path), &format!("fetch {} {}", remote, refspec));
    }

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let mut remote_obj = repo
        .find_remote(remote)
        .with_context(|| format!("Remote '{}' not found", remote))?;

    let mut fetch_opts = make_fetch_options();
    if let Some(d) = depth {
        fetch_opts.depth(d as i32);
    }
    remote_obj
        .fetch(&[refspec], Some(&mut fetch_opts), None)
        .with_context(|| format!("Fetch refspec '{}' from '{}' failed", refspec, remote))?;

    Ok(ok_result())
}

/// Fetch all remotes with tags and prune deleted refs.
///
/// libgit2's fetch is atomic — the 3-level retry cascade previously used
/// with the git binary is unnecessary here.
pub fn fetch_all_with_tags(repo_path: &Path) -> Result<GitResult> {
    log_git(Some(repo_path), "fetch --all --tags --force --prune");

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;

    let remote_names: Vec<String> = repo
        .remotes()?
        .iter()
        .filter_map(|r| r.map(String::from))
        .collect();

    for name in &remote_names {
        let mut remote = repo.find_remote(name)?;
        let mut fetch_opts = make_fetch_options();
        fetch_opts.download_tags(AutotagOption::All);
        fetch_opts.prune(git2::FetchPrune::On);
        remote.fetch(&[] as &[&str], Some(&mut fetch_opts), None)?;
    }

    Ok(ok_result())
}

/// Fetch tags from a specific remote
pub fn fetch_tags(repo_path: &Path, remote: Option<&str>) -> Result<GitResult> {
    let r = remote.unwrap_or("origin");
    log_git(Some(repo_path), &format!("fetch --tags {}", r));

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let remote_name = remote.unwrap_or("origin");
    let mut remote_obj = repo
        .find_remote(remote_name)
        .with_context(|| format!("Remote '{}' not found", remote_name))?;

    let mut fetch_opts = make_fetch_options();
    fetch_opts.download_tags(AutotagOption::All);
    remote_obj
        .fetch(&[] as &[&str], Some(&mut fetch_opts), None)
        .with_context(|| format!("Fetch tags from '{}' failed", remote_name))?;

    Ok(ok_result())
}

/// Checkout commit/branch/tag
///
/// Mirrors `git checkout <ref>` DWIM (Do What I Mean) behaviour:
/// when `commit_ref` cannot be resolved directly, fall back to
/// `origin/<commit_ref>` so that remote-only branch names work
/// without requiring a local tracking branch first.
pub fn checkout(repo_path: &Path, commit_ref: &str) -> Result<GitResult> {
    log_git(Some(repo_path), &format!("checkout {}", commit_ref));

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;

    // Resolve the reference to an object.
    // Try the literal name first (covers local branches, tags, SHAs, and
    // fully-qualified refs).  If that fails, try the remote-tracking
    // branch `origin/<name>` — this is the DWIM behaviour that the
    // host `git checkout` provides automatically but libgit2's
    // `revparse_single` does not.
    let obj = match repo.revparse_single(commit_ref) {
        Ok(obj) => obj,
        Err(_) => {
            let remote_ref = format!("origin/{}", commit_ref);
            repo.revparse_single(&remote_ref)
                .with_context(|| format!("Cannot resolve '{}'", commit_ref))?
        }
    };

    // Peel to commit (handles tags pointing to commits).
    let commit = obj
        .peel_to_commit()
        .with_context(|| format!("'{}' does not resolve to a commit", commit_ref))?;

    // Checkout the tree and detach HEAD.
    let mut checkout_builder = git2::build::CheckoutBuilder::new();
    checkout_builder.force();
    repo.checkout_tree(commit.as_object(), Some(&mut checkout_builder))
        .with_context(|| format!("Checkout tree failed for '{}'", commit_ref))?;
    repo.set_head_detached(commit.id())
        .with_context(|| format!("set_head_detached failed for '{}'", commit_ref))?;

    Ok(ok_result())
}

/// List remote references
/// Returns `(sha, ref_name)`
pub fn ls_remote(url: &str, heads: bool, tags: bool) -> Result<Vec<(String, String)>> {
    let mut flags = Vec::new();
    if heads {
        flags.push("--heads");
    }
    if tags {
        flags.push("--tags");
    }
    log_git(None, &format!("ls-remote {} {}", flags.join(" "), url));

    // Each call needs its own temp directory — using only the PID causes lock
    // collisions when multiple threads call ls_remote concurrently.
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory for ls-remote")?;
    let repo = Repository::init_bare(temp_dir.path())?;
    let mut remote = repo.remote_anonymous(url)?;

    remote
        .connect_auth(git2::Direction::Fetch, Some(make_remote_callbacks()), None)
        .with_context(|| format!("Failed to connect to {}", url))?;

    let remote_heads = remote.list()?;
    let refs: Vec<(String, String)> = remote_heads
        .iter()
        .filter_map(|head| {
            let name = head.name().to_string();
            let oid = head.oid().to_string();

            if !heads && !tags {
                // No filter — return everything.
                return Some((oid, name));
            }
            if heads && name.starts_with("refs/heads/") {
                return Some((oid, name));
            }
            if tags && name.starts_with("refs/tags/") {
                return Some((oid, name));
            }
            None
        })
        .collect();

    remote.disconnect()?;
    // temp_dir is dropped here, cleaning up automatically.

    Ok(refs)
}

/// Check if a git object exists in a repository
pub fn cat_file(repo_path: &Path, object: &str) -> bool {
    log_git(Some(repo_path), &format!("cat-file -e {}", object));

    let repo = match Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return false,
    };
    // Try to parse the object spec and find it in the ODB.
    let found = match repo.revparse_single(object) {
        Ok(_) => true,
        Err(_) => {
            // Also try direct ODB lookup for raw SHA.
            if let Ok(oid) = git2::Oid::from_str(object) {
                repo.odb().is_ok_and(|odb| odb.exists(oid))
            } else {
                false
            }
        }
    };
    found
}

/// List all tags from a local git repository
pub fn list_local_tags(repo_path: &Path) -> Result<Vec<String>> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let tags = repo.tag_names(None)?;
    Ok(tags.iter().filter_map(|t| t.map(String::from)).collect())
}

/// List all local branches from a local git repository
pub fn list_local_branches(repo_path: &Path) -> Result<Vec<String>> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let branches = repo.branches(Some(BranchType::Local))?;
    let names: Vec<String> = branches
        .filter_map(|b| {
            b.ok()
                .and_then(|(branch, _)| branch.name().ok().flatten().map(String::from))
        })
        .collect();
    Ok(names)
}

/// Check if reference is a branch
pub fn is_branch_reference(repo_path: &Path, commit_ref: &str) -> bool {
    let repo = match Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return false,
    };

    // Check remote branch.
    let remote_ref = format!("refs/remotes/origin/{}", commit_ref);
    if repo.find_reference(&remote_ref).is_ok() {
        return true;
    }

    // Check local branch.
    let local_ref = format!("refs/heads/{}", commit_ref);
    let is_local = repo.find_reference(&local_ref).is_ok();
    is_local
}

/// Get latest commit hash for branch
pub fn get_latest_commit_for_branch(repo_path: &Path, branch_name: &str) -> Option<String> {
    let repo = Repository::open(repo_path).ok()?;

    // Try remote branch first.
    let remote_ref = format!("origin/{}", branch_name);
    if let Ok(obj) = repo.revparse_single(&remote_ref) {
        if let Ok(commit) = obj.peel_to_commit() {
            return Some(commit.id().to_string());
        }
    }

    // Fallback to local branch.
    if let Ok(obj) = repo.revparse_single(branch_name) {
        if let Ok(commit) = obj.peel_to_commit() {
            return Some(commit.id().to_string());
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
    let repo = Repository::open(repo_path).ok()?;

    // In bare repos after fetch, try remote/branch notation.
    let remote_branch = format!("{}/{}", remote, branch_name);
    if let Ok(obj) = repo.revparse_single(&remote_branch) {
        if let Ok(commit) = obj.peel_to_commit() {
            return Some(commit.id().to_string());
        }
    }

    // For mirror repos, branches are stored as refs/heads/{branch}.
    let heads_ref = format!("refs/heads/{}", branch_name);
    if let Ok(reference) = repo.find_reference(&heads_ref) {
        if let Ok(commit) = reference.peel_to_commit() {
            return Some(commit.id().to_string());
        }
    }

    // Final fallback: just the branch name.
    if let Ok(obj) = repo.revparse_single(branch_name) {
        if let Ok(commit) = obj.peel_to_commit() {
            return Some(commit.id().to_string());
        }
    }

    None
}

/// Determine the fetch refspec, update-ref target, and resolved SHA from ls-remote pairs.
///
/// Returns `(fetch_refspec, update_ref_name, sha)`, in order of precedence:
/// - Tag `v1.0`    : `(refs/tags/v1.0, refs/tags/v1.0, Some(sha))`
/// - Branch `foo`  : `(refs/heads/foo, refs/heads/foo, Some(sha))`
/// - Commit SHA    : `(sha, refs/heads/trunk, None)`
pub fn resolve_fetch_refspec(
    refs: &[(String, String)],
    ref_name: &str,
) -> (String, String, Option<String>) {
    // Explicit reference
    if ref_name.starts_with("refs/heads/") || ref_name.starts_with("refs/tags/") {
        let sha = refs
            .iter()
            .find(|(_, r)| r == ref_name)
            .map(|(s, _)| s.clone());
        return (ref_name.to_string(), ref_name.to_string(), sha);
    }
    // Iterate in reverse so tags take precedence over a same-named branch.
    for (sha, full_ref) in refs.iter().rev() {
        if full_ref == &format!("refs/tags/{}", ref_name) {
            return (
                format!("refs/tags/{}", ref_name),
                format!("refs/tags/{}", ref_name),
                Some(sha.clone()),
            );
        }
        if full_ref == &format!("refs/heads/{}", ref_name) {
            return (
                format!("refs/heads/{}", ref_name),
                format!("refs/heads/{}", ref_name),
                Some(sha.clone()),
            );
        }
    }
    // Commit hash — fetch directly, anchor under trunk
    (ref_name.to_string(), "refs/heads/trunk".to_string(), None)
}

/// Get current commit hash
pub fn get_current_commit(repo_path: &Path) -> Result<String> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let head = repo.head().context("Failed to get HEAD")?;
    let commit = head
        .peel_to_commit()
        .context("HEAD does not point to a commit")?;
    Ok(commit.id().to_string())
}

/// Check if repository is dirty (has uncommitted changes)
pub fn is_repo_dirty(repo_path: &Path) -> Result<bool> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true);
    let statuses = repo
        .statuses(Some(&mut opts))
        .context("Failed to get repository status")?;
    Ok(!statuses.is_empty())
}

/// Update reference (for mirrors)
pub fn update_ref(repo_path: &Path, ref_name: &str, commit_hash: &str) -> Result<GitResult> {
    log_git(
        Some(repo_path),
        &format!("update-ref {} {}", ref_name, commit_hash),
    );

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let oid = git2::Oid::from_str(commit_hash)
        .with_context(|| format!("Invalid OID '{}'", commit_hash))?;
    repo.reference(ref_name, oid, true, "cim: update-ref")
        .with_context(|| format!("update_ref '{}' failed", ref_name))?;
    Ok(ok_result())
}

/// Create branch
pub fn create_branch(
    repo_path: &Path,
    branch_name: &str,
    commit_ref: Option<&str>,
) -> Result<GitResult> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;

    let commit = if let Some(ref_str) = commit_ref {
        repo.revparse_single(ref_str)?
            .peel_to_commit()
            .with_context(|| format!("'{}' is not a commit", ref_str))?
    } else {
        repo.head()?
            .peel_to_commit()
            .context("HEAD is not a commit")?
    };

    repo.branch(branch_name, &commit, false)
        .with_context(|| format!("Failed to create branch '{}'", branch_name))?;

    Ok(ok_result())
}

/// Create branch with force flag
pub fn create_branch_force(
    repo_path: &Path,
    branch_name: &str,
    commit_ref: &str,
) -> Result<GitResult> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let commit = repo
        .revparse_single(commit_ref)?
        .peel_to_commit()
        .with_context(|| format!("'{}' is not a commit", commit_ref))?;
    repo.branch(branch_name, &commit, true)
        .with_context(|| format!("Failed to force-create branch '{}'", branch_name))?;
    Ok(ok_result())
}

/// Clone repository as bare (for mirrors)
pub fn clone_bare(url: &str, path: &Path) -> Result<GitResult> {
    log_git(None, &format!("clone --bare {} {}", url, path.display()));

    let callbacks = make_remote_callbacks();
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);

    let mut builder = git2::build::RepoBuilder::new();
    builder.bare(true);
    builder.fetch_options(fetch_opts);

    builder
        .clone(url, path)
        .with_context(|| format!("Failed to bare-clone {} to {}", url, path.display()))?;

    Ok(ok_result())
}

/// Clone repository as mirror (for mirroring)
pub fn clone_mirror(url: &str, path: &Path) -> Result<GitResult> {
    log_git(None, &format!("clone --mirror {} {}", url, path.display()));

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow!("Failed to create parent directory: {}", e))?;
    }

    // Mirror = bare repo + fetch refspec `+refs/*:refs/*`.
    let mut opts = RepositoryInitOptions::new();
    opts.bare(true);
    let repo = Repository::init_opts(path, &opts)
        .with_context(|| format!("Failed to init bare repo at {}", path.display()))?;

    // Configure origin with mirror refspec.
    repo.remote_with_fetch("origin", url, "+refs/*:refs/*")
        .with_context(|| format!("Failed to add mirror remote for {}", url))?;

    // Fetch everything.
    let mut remote = repo
        .find_remote("origin")
        .context("Failed to find origin remote")?;
    let mut fetch_opts = make_fetch_options();
    remote
        .fetch(&[] as &[&str], Some(&mut fetch_opts), None)
        .with_context(|| format!("Failed to fetch mirror from {}", url))?;

    // Set HEAD to the default branch so clones from this mirror work.
    // Try common default branch names.
    for branch in &["main", "master"] {
        let ref_name = format!("refs/heads/{}", branch);
        if repo.find_reference(&ref_name).is_ok() {
            let _ = repo.set_head(&ref_name);
            break;
        }
    }

    Ok(ok_result())
}

/// Get the URL of a remote
pub fn get_remote_url(repo_path: &Path, remote_name: &str) -> Result<String> {
    log_git(Some(repo_path), &format!("remote get-url {}", remote_name));

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let remote = repo
        .find_remote(remote_name)
        .with_context(|| format!("Remote '{}' not found", remote_name))?;
    remote
        .url()
        .map(|u| u.to_string())
        .ok_or_else(|| anyhow!("Remote '{}' has no URL", remote_name))
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
    log_git(
        Some(repo_path),
        &format!("remote set-url {} {}", remote_name, url),
    );

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    repo.remote_set_url(remote_name, url)
        .with_context(|| format!("Failed to set URL for remote '{}'", remote_name))?;
    Ok(ok_result())
}

/// Add remote
pub fn remote_add(repo_path: &Path, remote_name: &str, url: &str) -> Result<GitResult> {
    log_git(
        Some(repo_path),
        &format!("remote add {} {}", remote_name, url),
    );

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    repo.remote(remote_name, url)
        .with_context(|| format!("Failed to add remote '{}'", remote_name))?;
    Ok(ok_result())
}

/// Initialize git repository
pub fn init_repo(path: &Path, bare: bool) -> Result<GitResult> {
    if bare {
        log_git(None, &format!("init --bare {}", path.display()));
    } else {
        log_git(None, &format!("init {}", path.display()));
    }

    let mut opts = RepositoryInitOptions::new();
    opts.initial_head("main");
    if bare {
        opts.bare(true);
    }

    Repository::init_opts(path, &opts)
        .with_context(|| format!("Failed to init repo at {}", path.display()))?;

    Ok(ok_result())
}

/// Add files to staging
pub fn add_files(repo_path: &Path, files: &[&str]) -> Result<GitResult> {
    log_git(Some(repo_path), &format!("add {}", files.join(" ")));

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let mut index = repo.index().context("Failed to get index")?;

    for file in files {
        index
            .add_path(Path::new(file))
            .with_context(|| format!("Failed to add '{}'", file))?;
    }
    index.write().context("Failed to write index")?;

    Ok(ok_result())
}

/// Add all files to staging
pub fn add_all(repo_path: &Path) -> Result<GitResult> {
    log_git(Some(repo_path), "add --all");

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let mut index = repo.index().context("Failed to get index")?;

    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .context("Failed to add all files")?;
    index.write().context("Failed to write index")?;

    Ok(ok_result())
}

/// Commit changes
pub fn commit(repo_path: &Path, message: &str) -> Result<GitResult> {
    log_git(Some(repo_path), &format!("commit -m '{}'", message));

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;

    let sig = repo
        .signature()
        .context("Failed to get default signature")?;
    let mut index = repo.index().context("Failed to get index")?;
    let tree_oid = index
        .write_tree()
        .context("Failed to write tree from index")?;
    let tree = repo.find_tree(tree_oid).context("Failed to find tree")?;

    // Find parent commit (if any — handles initial commit case).
    let parent_commit = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit<'_>> = parent_commit.iter().collect();

    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .context("Commit failed")?;

    Ok(ok_result())
}

/// Push changes
pub fn push(repo_path: &Path, remote: Option<&str>, branch: Option<&str>) -> Result<GitResult> {
    let r = remote.unwrap_or("origin");
    if let Some(b) = branch {
        log_git(Some(repo_path), &format!("push {} {}", r, b));
    } else {
        log_git(Some(repo_path), &format!("push {}", r));
    }

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let remote_name = remote.unwrap_or("origin");
    let mut remote_obj = repo
        .find_remote(remote_name)
        .with_context(|| format!("Remote '{}' not found", remote_name))?;

    let refspec = if let Some(b) = branch {
        format!("refs/heads/{}:refs/heads/{}", b, b)
    } else {
        // Push current branch.
        let head = repo.head().context("Failed to get HEAD")?;
        let name = head
            .name()
            .ok_or_else(|| anyhow!("HEAD is detached, cannot push"))?;
        format!("{}:{}", name, name)
    };

    let callbacks = make_remote_callbacks();
    let mut push_opts = git2::PushOptions::new();
    push_opts.remote_callbacks(callbacks);

    remote_obj
        .push(&[&refspec], Some(&mut push_opts))
        .with_context(|| format!("Push to '{}' failed", remote_name))?;

    Ok(ok_result())
}

/// Push all branches
pub fn push_all(repo_path: &Path, remote: &str) -> Result<GitResult> {
    log_git(Some(repo_path), &format!("push --all {}", remote));

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let mut remote_obj = repo
        .find_remote(remote)
        .with_context(|| format!("Remote '{}' not found", remote))?;

    // Enumerate all local branches and push them.
    let branches = repo.branches(Some(BranchType::Local))?;
    let refspecs: Vec<String> = branches
        .filter_map(|b| {
            b.ok().and_then(|(branch, _)| {
                branch
                    .name()
                    .ok()
                    .flatten()
                    .map(|n| format!("refs/heads/{}:refs/heads/{}", n, n))
            })
        })
        .collect();

    if refspecs.is_empty() {
        return Ok(ok_result());
    }

    let str_refs: Vec<&str> = refspecs.iter().map(|s| s.as_str()).collect();
    let callbacks = make_remote_callbacks();
    let mut push_opts = git2::PushOptions::new();
    push_opts.remote_callbacks(callbacks);

    remote_obj
        .push(&str_refs, Some(&mut push_opts))
        .with_context(|| format!("Push --all to '{}' failed", remote))?;

    Ok(ok_result())
}

/// Push all tags
pub fn push_tags(repo_path: &Path, remote: &str) -> Result<GitResult> {
    log_git(Some(repo_path), &format!("push --tags {}", remote));

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let mut remote_obj = repo
        .find_remote(remote)
        .with_context(|| format!("Remote '{}' not found", remote))?;

    // Enumerate all tags and push them.
    let tag_names = repo.tag_names(None)?;
    let refspecs: Vec<String> = tag_names
        .iter()
        .filter_map(|t| t.map(|name| format!("refs/tags/{}:refs/tags/{}", name, name)))
        .collect();

    if refspecs.is_empty() {
        return Ok(ok_result());
    }

    let str_refs: Vec<&str> = refspecs.iter().map(|s| s.as_str()).collect();
    let callbacks = make_remote_callbacks();
    let mut push_opts = git2::PushOptions::new();
    push_opts.remote_callbacks(callbacks);

    remote_obj
        .push(&str_refs, Some(&mut push_opts))
        .with_context(|| format!("Push --tags to '{}' failed", remote))?;

    Ok(ok_result())
}

/// Create tag
pub fn create_tag(repo_path: &Path, tag_name: &str) -> Result<GitResult> {
    log_git(Some(repo_path), &format!("tag {}", tag_name));

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let head = repo.head().context("Failed to get HEAD")?;
    let commit = head
        .peel(ObjectType::Commit)
        .context("HEAD does not point to a commit")?;
    repo.tag_lightweight(tag_name, &commit, false)
        .with_context(|| format!("Failed to create tag '{}'", tag_name))?;
    Ok(ok_result())
}

/// List tags
pub fn list_tags(repo_path: &Path, pattern: Option<&str>) -> Result<Vec<String>> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let tag_names = repo.tag_names(pattern)?;
    Ok(tag_names
        .iter()
        .filter_map(|t| t.map(String::from))
        .collect())
}

/// Configure git setting
pub fn config(repo_path: &Path, key: &str, value: &str) -> Result<GitResult> {
    log_git(Some(repo_path), &format!("config {} {}", key, value));

    let repo = Repository::open(repo_path)
        .with_context(|| format!("Cannot open {}", repo_path.display()))?;
    let mut config = repo.config().context("Failed to get config")?;
    config
        .set_str(key, value)
        .with_context(|| format!("Failed to set config {}={}", key, value))?;
    Ok(ok_result())
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
    fn test_resolve_fetch_refspec_explicit_refs_heads() {
        let refs = vec![
            ("abc123".to_string(), "refs/heads/main".to_string()),
            ("def456".to_string(), "refs/tags/v1.0.0".to_string()),
        ];

        // Explicit refs/heads/ — SHA resolved from ls-remote
        let (fetch, update, sha) = resolve_fetch_refspec(&refs, "refs/heads/main");
        assert_eq!(fetch, "refs/heads/main");
        assert_eq!(update, "refs/heads/main");
        assert_eq!(sha, Some("abc123".to_string()));

        // Explicit refs/heads/ not in ls-remote — SHA is None
        let (fetch, update, sha) = resolve_fetch_refspec(&refs, "refs/heads/missing");
        assert_eq!(fetch, "refs/heads/missing");
        assert_eq!(update, "refs/heads/missing");
        assert_eq!(sha, None);
    }

    #[test]
    fn test_resolve_fetch_refspec_explicit_refs_tags() {
        let refs = vec![
            ("abc123".to_string(), "refs/heads/main".to_string()),
            ("def456".to_string(), "refs/tags/v1.0.0".to_string()),
        ];

        // Explicit refs/tags/ — update_ref_name strips "refs/tags/" and uses "refs/heads/"
        let (fetch, update, sha) = resolve_fetch_refspec(&refs, "refs/tags/v1.0.0");
        assert_eq!(fetch, "refs/tags/v1.0.0");
        assert_eq!(update, "refs/tags/v1.0.0");
        assert_eq!(sha, Some("def456".to_string()));

        // Explicit refs/tags/ not in ls-remote — SHA is None
        let (fetch, update, sha) = resolve_fetch_refspec(&refs, "refs/tags/missing");
        assert_eq!(fetch, "refs/tags/missing");
        assert_eq!(update, "refs/tags/missing");
        assert_eq!(sha, None);
    }

    #[test]
    fn test_resolve_fetch_refspec_short_names() {
        let refs = vec![
            ("abc123".to_string(), "refs/heads/main".to_string()),
            ("def456".to_string(), "refs/tags/v1.0.0".to_string()),
        ];

        // Short branch name
        let (fetch, update, sha) = resolve_fetch_refspec(&refs, "main");
        assert_eq!(fetch, "refs/heads/main");
        assert_eq!(update, "refs/heads/main");
        assert_eq!(sha, Some("abc123".to_string()));

        // Short tag name
        let (fetch, update, sha) = resolve_fetch_refspec(&refs, "v1.0.0");
        assert_eq!(fetch, "refs/tags/v1.0.0");
        assert_eq!(update, "refs/tags/v1.0.0");
        assert_eq!(sha, Some("def456".to_string()));

        // Commit SHA fallthrough
        let (fetch, update, sha) = resolve_fetch_refspec(&refs, "deadbeef");
        assert_eq!(fetch, "deadbeef");
        assert_eq!(update, "refs/heads/trunk");
        assert_eq!(sha, None);
    }

    #[test]
    fn test_ls_remote_format() {
        // Test with a known public repository
        if let Ok(refs) = ls_remote("https://github.com/git/git.git", true, false) {
            assert!(!refs.is_empty());
            // Check that we get proper ref format
            assert!(refs.iter().any(|(_, r)| r.starts_with("refs/heads/")));
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
