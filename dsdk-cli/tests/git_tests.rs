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

//! Integration tests for git operations.
//!
//! These tests use real git operations with local repositories to verify
//! that git commands work correctly without requiring network access.

mod common;

use common::{MockGitRepo, TestFixture};
use dsdk_cli::git_manager;
use dsdk_cli::git_operations;
use std::fs;

#[test]
fn test_init_repository() {
    let fixture = TestFixture::new();
    let repo_path = fixture.create_dir("test-repo");

    let result =
        git_operations::init_repo(&repo_path, false).expect("Should initialize repository");

    assert!(result.is_success());
    assert!(repo_path.join(".git").exists());
}

#[test]
fn test_init_bare_repository() {
    let fixture = TestFixture::new();
    let repo_path = fixture.create_dir("bare-repo.git");

    let result =
        git_operations::init_repo(&repo_path, true).expect("Should initialize bare repository");

    assert!(result.is_success());
    assert!(repo_path.join("HEAD").exists());
    assert!(repo_path.join("refs").exists());
}

#[test]
fn test_clone_repository() {
    let source_repo = MockGitRepo::new("source");
    source_repo.add_file("README.md", "# Test\n");
    source_repo.commit("Initial commit");

    let fixture = TestFixture::new();
    let clone_path = fixture.path().join("clone");

    let result = git_operations::clone_repo(&source_repo.file_url(), &clone_path, None)
        .expect("Should clone repository");

    assert!(result.is_success());
    assert!(clone_path.join("README.md").exists());
    assert_eq!(
        fs::read_to_string(clone_path.join("README.md")).unwrap(),
        "# Test\n"
    );
}

#[test]
fn test_clone_with_reference() {
    let source_repo = MockGitRepo::new("source");
    source_repo.add_file("data.txt", "content\n");
    source_repo.commit("Initial commit");

    let fixture = TestFixture::new();

    // Create a mirror clone first
    let mirror_path = fixture.path().join("mirror");
    git_operations::clone_repo(&source_repo.file_url(), &mirror_path, None)
        .expect("Should clone to mirror");

    // Clone with reference to mirror
    let workspace_path = fixture.path().join("workspace");
    let result =
        git_operations::clone_repo(&source_repo.file_url(), &workspace_path, Some(&mirror_path))
            .expect("Should clone with reference");

    assert!(result.is_success());
    assert!(workspace_path.join("data.txt").exists());
}

#[test]
fn test_add_and_commit() {
    let repo = MockGitRepo::new("test-repo");
    repo.add_file("file1.txt", "content 1\n");

    let add_result = git_operations::add_all(&repo.path).expect("Should add files");
    assert!(add_result.is_success());

    let commit_result = git_operations::commit(&repo.path, "Add file1").expect("Should commit");
    assert!(commit_result.is_success());

    // Verify commit was created
    let current_commit =
        git_operations::get_current_commit(&repo.path).expect("Should get current commit");
    assert!(!current_commit.is_empty());
}

#[test]
fn test_checkout_commit() {
    let repo = MockGitRepo::new("test-repo");

    // Create first commit
    repo.add_file("version.txt", "1.0\n");
    repo.commit("Version 1.0");
    let commit1 = git_operations::get_current_commit(&repo.path).expect("Should get first commit");

    // Create second commit
    repo.add_file("version.txt", "2.0\n");
    repo.commit("Version 2.0");

    // Checkout first commit
    let checkout_result =
        git_operations::checkout(&repo.path, &commit1).expect("Should checkout commit");
    assert!(checkout_result.is_success());

    // Verify file content from first commit
    let content = fs::read_to_string(repo.path.join("version.txt")).unwrap();
    assert_eq!(content, "1.0\n");
}

#[test]
fn test_create_and_checkout_tag() {
    let repo = MockGitRepo::new("test-repo");
    repo.add_file("README.md", "# Version 1.0\n");
    repo.commit("Version 1.0");

    let tag_result = git_operations::create_tag(&repo.path, "v1.0.0").expect("Should create tag");
    assert!(tag_result.is_success());

    // Create another commit
    repo.add_file("README.md", "# Version 2.0\n");
    repo.commit("Version 2.0");

    // Checkout tag
    let checkout_result =
        git_operations::checkout(&repo.path, "v1.0.0").expect("Should checkout tag");
    assert!(checkout_result.is_success());

    // Verify content from tagged commit
    let content = fs::read_to_string(repo.path.join("README.md")).unwrap();
    assert_eq!(content, "# Version 1.0\n");
}

#[test]
fn test_fetch_operations() {
    // Create upstream repository
    let upstream = MockGitRepo::create_bare("upstream.git");

    // Create working repository and push to upstream
    let working = MockGitRepo::new("working");
    git_operations::remote_add(&working.path, "origin", &upstream.file_url())
        .expect("Should add remote");

    working.add_file("initial.txt", "initial\n");
    working.commit("Initial commit");
    git_operations::push(&working.path, Some("origin"), Some("main"))
        .expect("Should push to upstream");

    // Clone from upstream
    let fixture = TestFixture::new();
    let clone_path = fixture.path().join("clone");
    git_operations::clone_repo(&upstream.file_url(), &clone_path, None).expect("Should clone");

    // Add new commit to working repo
    working.add_file("new.txt", "new content\n");
    working.commit("Add new file");
    git_operations::push(&working.path, Some("origin"), Some("main"))
        .expect("Should push new commit");

    // Fetch in cloned repo
    let fetch_result = git_operations::fetch(&clone_path, Some("origin")).expect("Should fetch");
    assert!(fetch_result.is_success());
}

#[test]
fn test_is_repo_dirty() {
    let repo = MockGitRepo::new("test-repo");
    repo.add_file("file.txt", "initial\n");
    repo.commit("Initial commit");

    // Clean repository
    let is_dirty = git_operations::is_repo_dirty(&repo.path).expect("Should check if dirty");
    assert!(!is_dirty);

    // Make repository dirty
    repo.add_file("file.txt", "modified\n");
    let is_dirty = git_operations::is_repo_dirty(&repo.path).expect("Should check if dirty");
    assert!(is_dirty);
}

#[test]
fn test_list_tags() {
    let repo = MockGitRepo::new("test-repo");
    repo.add_file("v1.txt", "version 1\n");
    repo.commit("Version 1");
    repo.create_tag("v1.0.0");

    repo.add_file("v2.txt", "version 2\n");
    repo.commit("Version 2");
    repo.create_tag("v2.0.0");

    let tags = git_operations::list_local_tags(&repo.path).expect("Should list tags");

    assert_eq!(tags.len(), 2);
    assert!(tags.contains(&"v1.0.0".to_string()));
    assert!(tags.contains(&"v2.0.0".to_string()));
}

#[test]
fn test_list_branches() {
    let repo = MockGitRepo::new("test-repo");
    repo.add_file("main.txt", "main branch\n");
    repo.commit("Main commit");

    // Create a new branch
    git_operations::create_branch(&repo.path, "develop", None).expect("Should create branch");

    let branches = git_operations::list_local_branches(&repo.path).expect("Should list branches");

    // Should have at least main (or master) and develop
    assert!(branches.len() >= 2);
}

#[test]
fn test_remote_operations() {
    let fixture = TestFixture::new();
    let repo_path = fixture.create_dir("test-repo");

    git_operations::init_repo(&repo_path, false).expect("Should initialize repository");
    git_operations::config(&repo_path, "user.name", "Test").expect("Should configure user");
    git_operations::config(&repo_path, "user.email", "test@example.com")
        .expect("Should configure email");

    // Add remote
    let remote_url = "https://github.com/example/test.git";
    let result =
        git_operations::remote_add(&repo_path, "origin", remote_url).expect("Should add remote");
    assert!(result.is_success());

    // Verify remote was added by trying to set URL (which requires the remote to exist)
    let result = git_operations::remote_set_url(&repo_path, "origin", remote_url)
        .expect("Should set remote URL");
    assert!(result.is_success());
}

#[test]
fn test_config_operations() {
    let fixture = TestFixture::new();
    let repo_path = fixture.create_dir("test-repo");

    git_operations::init_repo(&repo_path, false).expect("Should initialize repository");

    // Set config values
    git_operations::config(&repo_path, "user.name", "Test User").expect("Should set user.name");
    git_operations::config(&repo_path, "user.email", "test@example.com")
        .expect("Should set user.email");

    // Verify config was set by using git command directly
    let result = git_operations::git_command(&["config", "user.name"], Some(&repo_path))
        .expect("Should get config");
    assert!(result.is_success());
    assert!(result.stdout.contains("Test User"));
}

#[test]
fn test_get_current_commit() {
    let repo = MockGitRepo::new("test-repo");
    repo.add_file("test.txt", "test\n");
    repo.commit("Test commit");

    let commit = git_operations::get_current_commit(&repo.path).expect("Should get current commit");

    // Commit hash should be 40 characters (SHA-1)
    assert_eq!(commit.len(), 40);
}

#[test]
fn test_mirror_workflow() {
    // This tests the complete mirror workflow used by cim
    let fixture = TestFixture::new();

    // 1. Create upstream repository
    let upstream = MockGitRepo::create_bare("upstream.git");
    let working = MockGitRepo::new("working");
    git_operations::remote_add(&working.path, "origin", &upstream.file_url())
        .expect("Should add remote");
    working.add_file("data.txt", "important data\n");
    working.commit("Add data");
    git_operations::push(&working.path, Some("origin"), Some("main")).expect("Should push");

    // 2. Create mirror
    let mirror_path = fixture.path().join("mirror/test-repo.git");
    let mirror_result = git_operations::clone_mirror(&upstream.file_url(), &mirror_path)
        .expect("Should create mirror");
    assert!(mirror_result.is_success());

    // 3. Clone from mirror to workspace
    let workspace_path = fixture.path().join("workspace/test-repo");
    let clone_result = git_operations::clone_repo(
        &format!("file://{}", mirror_path.display()),
        &workspace_path,
        None,
    )
    .expect("Should clone from mirror");
    assert!(clone_result.is_success());

    // 4. Verify data in workspace
    assert!(workspace_path.join("data.txt").exists());
    let content = fs::read_to_string(workspace_path.join("data.txt")).unwrap();
    assert_eq!(content, "important data\n");
}

#[test]
fn test_branch_detection() {
    let repo = MockGitRepo::new("test-repo");
    repo.add_file("test.txt", "test\n");
    repo.commit("Initial commit");

    // Main/master is a branch
    let is_branch = git_operations::is_branch_reference(&repo.path, "main");
    let is_branch_master = git_operations::is_branch_reference(&repo.path, "master");
    assert!(is_branch || is_branch_master);

    // Create and checkout a tag
    repo.create_tag("v1.0.0");

    // Tag is not a branch
    let is_branch = git_operations::is_branch_reference(&repo.path, "v1.0.0");
    assert!(!is_branch);
}

#[test]
fn test_mirror_discovers_new_branches() {
    // This tests that fetch_all_with_tags discovers branches added after mirror creation
    let fixture = TestFixture::new();

    // 1. Create upstream repository with initial branch
    let upstream = MockGitRepo::create_bare("upstream.git");
    let working = MockGitRepo::new("working");
    git_operations::remote_add(&working.path, "origin", &upstream.file_url())
        .expect("Should add remote");
    working.add_file("initial.txt", "initial data\n");
    working.commit("Initial commit");
    git_operations::push(&working.path, Some("origin"), Some("main")).expect("Should push");

    // 2. Create mirror
    let mirror_path = fixture.path().join("mirror/test-repo.git");
    let mirror_result = git_operations::clone_mirror(&upstream.file_url(), &mirror_path)
        .expect("Should create mirror");
    assert!(mirror_result.is_success());

    // 3. Add a new branch to upstream (simulating the dev/gensecbl scenario)
    git_operations::create_branch(&working.path, "dev/new-feature", None)
        .expect("Should create new branch");
    git_operations::checkout(&working.path, "dev/new-feature").expect("Should checkout new branch");
    working.add_file("feature.txt", "feature data\n");
    working.commit("Add feature");
    git_operations::push(&working.path, Some("origin"), Some("dev/new-feature"))
        .expect("Should push new branch");

    // 4. Update mirror with fetch_all_with_tags (should discover new branch)
    let fetch_result =
        git_operations::fetch_all_with_tags(&mirror_path).expect("Should fetch all with tags");
    assert!(
        fetch_result.is_success(),
        "fetch_all_with_tags should succeed"
    );

    // 5. Verify new branch exists in mirror
    // In a mirror repository, all branches are in refs/heads/
    let branches = git_operations::list_local_branches(&mirror_path).expect("Should list branches");
    assert!(
        branches.iter().any(|b| b.contains("dev/new-feature")),
        "Mirror should have discovered the new dev/new-feature branch. Found branches: {:?}",
        branches
    );
}

#[test]
fn test_get_mirror_repo_path_no_existing_mirror() {
    let fixture = TestFixture::new();
    let mirror_path = fixture.create_dir("mirror");

    let result = git_manager::get_mirror_repo_path(
        &mirror_path,
        "u-boot",
        "https://github.com/u-boot/u-boot.git",
    );
    assert_eq!(result, mirror_path.join("u-boot"));
}

#[test]
fn test_get_mirror_repo_path_matching_url() {
    let fixture = TestFixture::new();
    let mirror_path = fixture.create_dir("mirror");
    let upstream = MockGitRepo::new("upstream-uboot");

    // Create a mirror clone with the upstream URL
    let mirror_repo = mirror_path.join("u-boot");
    git_operations::clone_mirror(&upstream.file_url(), &mirror_repo).expect("Should create mirror");

    // Same URL should resolve to the same path
    let result = git_manager::get_mirror_repo_path(&mirror_path, "u-boot", &upstream.file_url());
    assert_eq!(result, mirror_path.join("u-boot"));
}

#[test]
fn test_get_mirror_repo_path_url_mismatch_uses_hash() {
    let fixture = TestFixture::new();
    let mirror_path = fixture.create_dir("mirror");
    let upstream1 = MockGitRepo::new("upstream1");

    // Create a mirror for the first upstream
    let mirror_repo = mirror_path.join("u-boot");
    git_operations::clone_mirror(&upstream1.file_url(), &mirror_repo)
        .expect("Should create mirror");

    // Different URL should resolve to hash-based path
    let result = git_manager::get_mirror_repo_path(
        &mirror_path,
        "u-boot",
        "https://github.com/analogdevicesinc/u-boot-xlnx.git",
    );
    let hash = git_operations::hash_url("https://github.com/analogdevicesinc/u-boot-xlnx.git");
    assert_eq!(result, mirror_path.join(format!("u-boot-{}", hash)));
}

#[test]
fn test_get_mirror_repo_path_different_urls_get_different_paths() {
    let fixture = TestFixture::new();
    let mirror_path = fixture.create_dir("mirror");
    let upstream1 = MockGitRepo::new("upstream1");

    // Create a mirror at the default name
    let mirror1 = mirror_path.join("u-boot");
    git_operations::clone_mirror(&upstream1.file_url(), &mirror1).expect("Should create mirror 1");

    // Different URL should get a unique hash-based path
    let result = git_manager::get_mirror_repo_path(
        &mirror_path,
        "u-boot",
        "https://github.com/analogdevicesinc/u-boot-xlnx.git",
    );
    let hash = git_operations::hash_url("https://github.com/analogdevicesinc/u-boot-xlnx.git");
    assert_eq!(result, mirror_path.join(format!("u-boot-{}", hash)));
    // Ensure it's different from the default path
    assert_ne!(result, mirror_path.join("u-boot"));
}
