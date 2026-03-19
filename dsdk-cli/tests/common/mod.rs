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

//! Common test utilities and fixtures for integration tests.
//!
//! This module provides shared functionality for all integration tests including:
//! - Mock git repository creation
//! - Temporary directory management
//! - Test fixture data
//! - Helper functions for common test scenarios

use dsdk_cli::config::{GitConfig, SdkConfig};
use dsdk_cli::git_operations;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

/// Test fixture that provides a temporary directory and cleans up automatically
pub struct TestFixture {
    pub temp_dir: TempDir,
}

impl TestFixture {
    /// Create a new test fixture with a temporary directory
    pub fn new() -> Self {
        Self {
            temp_dir: tempdir().expect("Failed to create temp directory"),
        }
    }

    /// Get the path to the temporary directory
    pub fn path(&self) -> &Path {
        self.temp_dir.path()
    }

    /// Create a subdirectory within the temp directory
    pub fn create_dir(&self, name: &str) -> PathBuf {
        let dir = self.path().join(name);
        fs::create_dir_all(&dir).expect("Failed to create subdirectory");
        dir
    }

    /// Write a file within the temp directory
    pub fn write_file(&self, name: &str, content: &str) {
        let path = self.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("Failed to create parent directory");
        }
        fs::write(&path, content).expect("Failed to write file");
    }

    /// Read a file from the temp directory
    pub fn read_file(&self, name: &str) -> String {
        let path = self.path().join(name);
        fs::read_to_string(path).expect("Failed to read file")
    }
}

impl Default for TestFixture {
    fn default() -> Self {
        Self::new()
    }
}

/// Mock git repository for testing
pub struct MockGitRepo {
    pub path: PathBuf,
    #[allow(dead_code)]
    pub fixture: TestFixture,
}

impl MockGitRepo {
    /// Create a new mock git repository with initial content
    pub fn new(name: &str) -> Self {
        let fixture = TestFixture::new();
        let repo_path = fixture.create_dir(name);

        // Initialize git repository
        git_operations::init_repo(&repo_path, false).expect("Failed to initialize git repository");

        // Configure git user for commits
        git_operations::config(&repo_path, "user.name", "Test User")
            .expect("Failed to configure git user.name");
        git_operations::config(&repo_path, "user.email", "test@example.com")
            .expect("Failed to configure git user.email");

        Self {
            path: repo_path,
            fixture,
        }
    }

    /// Add a file to the repository
    pub fn add_file(&self, name: &str, content: &str) {
        let file_path = self.path.join(name);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create parent directory");
        }
        fs::write(&file_path, content).expect("Failed to write file");
    }

    /// Commit all changes
    pub fn commit(&self, message: &str) {
        git_operations::add_all(&self.path).expect("Failed to add files");
        git_operations::commit(&self.path, message).expect("Failed to commit");
    }

    /// Create a tag
    #[allow(dead_code)]
    pub fn create_tag(&self, tag: &str) {
        git_operations::create_tag(&self.path, tag).expect("Failed to create tag");
    }

    /// Create a bare repository suitable for use as a remote
    #[allow(dead_code)]
    pub fn create_bare(name: &str) -> Self {
        let fixture = TestFixture::new();
        let repo_path = fixture.create_dir(name);

        // Initialize bare repository
        git_operations::init_repo(&repo_path, true).expect("Failed to initialize bare repository");

        Self {
            path: repo_path,
            fixture,
        }
    }

    /// Get file:// URL for this repository
    #[allow(dead_code)]
    pub fn file_url(&self) -> String {
        format!("file://{}", self.path.display())
    }
}

/// Create a minimal SDK configuration for testing
pub fn create_minimal_sdk_config(mirror_path: &Path) -> SdkConfig {
    SdkConfig {
        mirror: mirror_path.to_path_buf(),
        gits: vec![],
        toolchains: None,
        install: None,
        copy_files: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    }
}

/// Create a basic SDK configuration with one repository
#[allow(dead_code)]
pub fn create_basic_sdk_config(mirror_path: &Path, repo_url: &str) -> SdkConfig {
    SdkConfig {
        mirror: mirror_path.to_path_buf(),
        gits: vec![GitConfig {
            name: "test-repo".to_string(),
            url: repo_url.to_string(),
            commit: "main".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: Some(vec!["make".to_string()]),
            documentation_dir: None,
        }],
        toolchains: None,
        install: None,
        copy_files: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    }
}

/// Create an SDK configuration with multiple repositories and dependencies
pub fn create_complex_sdk_config(mirror_path: &Path) -> SdkConfig {
    SdkConfig {
        mirror: mirror_path.to_path_buf(),
        gits: vec![
            GitConfig {
                name: "base-lib".to_string(),
                url: "https://github.com/example/base-lib.git".to_string(),
                commit: "v1.0.0".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: Some(vec!["make base".to_string()]),
                documentation_dir: None,
            },
            GitConfig {
                name: "middleware".to_string(),
                url: "https://github.com/example/middleware.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: Some(vec!["base-lib".to_string()]),
                git_depends_on: None,
                build: Some(vec!["make middleware".to_string()]),
                documentation_dir: None,
            },
            GitConfig {
                name: "application".to_string(),
                url: "https://github.com/example/application.git".to_string(),
                commit: "develop".to_string(),
                build_depends_on: Some(vec!["base-lib".to_string(), "middleware".to_string()]),
                git_depends_on: None,
                build: Some(vec!["make app".to_string()]),
                documentation_dir: None,
            },
        ],
        toolchains: None,
        install: None,
        copy_files: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    }
}

/// Write an SDK config to a YAML file
#[allow(dead_code)]
pub fn write_sdk_config(config: &SdkConfig, path: &Path) {
    let yaml = serde_yaml::to_string(config).expect("Failed to serialize SDK config");
    fs::write(path, yaml).expect("Failed to write SDK config file");
}

/// Create a workspace marker file
#[allow(dead_code)]
pub fn create_workspace_marker(workspace_path: &Path, config_filename: &str) {
    use sha2::{Digest, Sha256};

    let marker_content = format!(
        r#"workspace_version: "1"
created_at: "{}"
config_file: {}
target: {}
target_version: "latest"
config_sha256: {}
mirror_path: "/tmp/test-mirror"
sdk_manager_version: "test"
sdk_manager_sha256: "test-sha256"
sdk_manager_commit: "test-commit"
"#,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        config_filename,
        config_filename,
        hex::encode(Sha256::digest(config_filename.as_bytes()))
    );

    fs::write(workspace_path.join(".workspace"), marker_content)
        .expect("Failed to create workspace marker");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixture_creates_temp_dir() {
        let fixture = TestFixture::new();
        assert!(fixture.path().exists());
    }

    #[test]
    fn test_fixture_write_and_read() {
        let fixture = TestFixture::new();
        fixture.write_file("test.txt", "Hello, World!");
        let content = fixture.read_file("test.txt");
        assert_eq!(content, "Hello, World!");
    }

    #[test]
    fn test_mock_git_repo_creation() {
        let repo = MockGitRepo::new("test-repo");
        assert!(repo.path.exists());
        assert!(repo.path.join(".git").exists());
    }

    #[test]
    fn test_mock_git_repo_commit() {
        let repo = MockGitRepo::new("test-repo");
        repo.add_file("README.md", "# Test Repository\n");
        repo.commit("Initial commit");

        // Verify the file exists
        assert!(repo.path.join("README.md").exists());
    }

    #[test]
    fn test_create_minimal_sdk_config() {
        let fixture = TestFixture::new();
        let config = create_minimal_sdk_config(fixture.path());
        assert_eq!(config.gits.len(), 0);
        assert_eq!(config.mirror, fixture.path());
    }

    #[test]
    fn test_create_complex_sdk_config() {
        let fixture = TestFixture::new();
        let config = create_complex_sdk_config(fixture.path());
        assert_eq!(config.gits.len(), 3);
        assert_eq!(config.gits[0].name, "base-lib");
        assert!(config.gits[0].build_depends_on.is_none());
        assert_eq!(config.gits[1].build_depends_on.as_ref().unwrap().len(), 1);
        assert_eq!(config.gits[2].build_depends_on.as_ref().unwrap().len(), 2);
    }
}
