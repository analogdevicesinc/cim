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

use dsdk_cli::config::{GitConfig, SdkConfig, UserConfig};
use dsdk_cli::doc_manager::DocManager;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

/// Helper function to create a test repository with documentation
fn create_test_repo(
    workspace: &std::path::Path,
    name: &str,
    doc_dir: &str,
    create_index: bool,
) -> PathBuf {
    let repo_path = workspace.join(name);
    let docs_path = if doc_dir == "." {
        repo_path.clone()
    } else {
        repo_path.join(doc_dir)
    };

    fs::create_dir_all(&docs_path).unwrap();

    if create_index {
        let index_content = format!(
            "{}\n{}\n\nThis is test documentation for {}.",
            name.to_uppercase(),
            "=".repeat(name.len()),
            name
        );
        fs::write(docs_path.join("index.rst"), index_content).unwrap();
    }

    repo_path
}

/// Helper function to create a GitConfig
fn create_git_config(name: &str, documentation_dir: Option<String>) -> GitConfig {
    GitConfig {
        name: name.to_string(),
        url: format!("https://example.com/{}.git", name),
        commit: "main".to_string(),
        build_depends_on: None,
        git_depends_on: None,
        build: None,
        documentation_dir,
    }
}

#[test]
fn test_discover_with_default_docs_directory() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    // Create repo with docs/ directory (built-in default)
    create_test_repo(workspace, "repo1", "docs", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", None)],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, None, false)
        .unwrap();

    assert_eq!(doc_sources.len(), 1);
    assert_eq!(doc_sources[0].repo_name, "repo1");
    assert!(doc_sources[0].docs_path.ends_with("docs"));
}

#[test]
fn test_discover_with_doc_directory() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    // Create repo with doc/ directory (built-in default alternative)
    create_test_repo(workspace, "repo1", "doc", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", None)],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, None, false)
        .unwrap();

    assert_eq!(doc_sources.len(), 1);
    assert_eq!(doc_sources[0].repo_name, "repo1");
    assert!(doc_sources[0].docs_path.ends_with("doc"));
}

#[test]
fn test_discover_with_documentation_directory() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    // Create repo with documentation/ directory (built-in default)
    create_test_repo(workspace, "repo1", "documentation", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", None)],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, None, false)
        .unwrap();

    assert_eq!(doc_sources.len(), 1);
    assert_eq!(doc_sources[0].repo_name, "repo1");
    assert!(doc_sources[0].docs_path.ends_with("documentation"));
}

#[test]
fn test_discover_with_documents_directory() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    // Create repo with documents/ directory (built-in default)
    create_test_repo(workspace, "repo1", "documents", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", None)],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, None, false)
        .unwrap();

    assert_eq!(doc_sources.len(), 1);
    assert_eq!(doc_sources[0].repo_name, "repo1");
    assert!(doc_sources[0].docs_path.ends_with("documents"));
}

#[test]
fn test_discover_with_root_directory() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    // Create repo with index.rst in root (.)
    create_test_repo(workspace, "repo1", ".", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", None)],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, None, false)
        .unwrap();

    assert_eq!(doc_sources.len(), 1);
    assert_eq!(doc_sources[0].repo_name, "repo1");
    assert!(doc_sources[0].docs_path.ends_with("repo1"));
}

#[test]
fn test_discover_with_user_config_directories() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    // Create repo with custom directory specified in user config
    create_test_repo(workspace, "repo1", "my_docs", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", None)],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let user_config = UserConfig {
        documentation_dirs: Some("my_docs".to_string()),
        ..Default::default()
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, Some(&user_config), false)
        .unwrap();

    assert_eq!(doc_sources.len(), 1);
    assert_eq!(doc_sources[0].repo_name, "repo1");
    assert!(doc_sources[0].docs_path.ends_with("my_docs"));
}

#[test]
fn test_discover_with_per_git_custom_directory() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    // Create repo with custom directory specified in git config
    create_test_repo(workspace, "repo1", "special_docs", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", Some("special_docs".to_string()))],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, None, false)
        .unwrap();

    assert_eq!(doc_sources.len(), 1);
    assert_eq!(doc_sources[0].repo_name, "repo1");
    assert!(doc_sources[0].docs_path.ends_with("special_docs"));
}

#[test]
fn test_discover_first_match_wins() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    // Create repo with multiple documentation directories
    let repo_path = workspace.join("repo1");
    fs::create_dir_all(repo_path.join("docs")).unwrap();
    fs::create_dir_all(repo_path.join("doc")).unwrap();

    // Only put index.rst in docs/ (should match first based on defaults order)
    let index_content = "REPO1\n=====\n\nTest docs";
    fs::write(repo_path.join("docs").join("index.rst"), index_content).unwrap();
    fs::write(repo_path.join("doc").join("index.rst"), "OTHER").unwrap();

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", None)],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, None, false)
        .unwrap();

    assert_eq!(doc_sources.len(), 1);
    // Should find docs/ first (it's first in defaults list)
    assert!(doc_sources[0].docs_path.ends_with("docs"));
}

#[test]
fn test_discover_combined_search_list() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    // Create three repos, each with docs in different locations
    create_test_repo(workspace, "repo1", "docs", true); // Built-in default
    create_test_repo(workspace, "repo2", "user_docs", true); // User config
    create_test_repo(workspace, "repo3", "git_docs", true); // Per-git config

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![
            create_git_config("repo1", None),
            create_git_config("repo2", None),
            create_git_config("repo3", Some("git_docs".to_string())),
        ],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let user_config = UserConfig {
        documentation_dirs: Some("user_docs".to_string()),
        ..Default::default()
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, Some(&user_config), false)
        .unwrap();

    assert_eq!(doc_sources.len(), 3);

    // Verify each repo was found
    let repo_names: Vec<&str> = doc_sources.iter().map(|s| s.repo_name.as_str()).collect();
    assert!(repo_names.contains(&"repo1"));
    assert!(repo_names.contains(&"repo2"));
    assert!(repo_names.contains(&"repo3"));

    // Verify correct paths
    let repo1_source = doc_sources.iter().find(|s| s.repo_name == "repo1").unwrap();
    assert!(repo1_source.docs_path.ends_with("docs"));

    let repo2_source = doc_sources.iter().find(|s| s.repo_name == "repo2").unwrap();
    assert!(repo2_source.docs_path.ends_with("user_docs"));

    let repo3_source = doc_sources.iter().find(|s| s.repo_name == "repo3").unwrap();
    assert!(repo3_source.docs_path.ends_with("git_docs"));
}

#[test]
fn test_discover_empty_strings_skipped() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    create_test_repo(workspace, "repo1", "docs", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", Some("".to_string()))], // Empty string
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let user_config = UserConfig {
        documentation_dirs: Some("".to_string()), // Empty string
        ..Default::default()
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, Some(&user_config), false)
        .unwrap();

    // Should still find docs using built-in defaults
    assert_eq!(doc_sources.len(), 1);
    assert!(doc_sources[0].docs_path.ends_with("docs"));
}

#[test]
fn test_discover_deduplication() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    create_test_repo(workspace, "repo1", "docs", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", Some("docs".to_string()))], // Duplicate
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let user_config = UserConfig {
        documentation_dirs: Some("docs".to_string()), // Duplicate
        ..Default::default()
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());

    // This shouldn't panic or check docs/ multiple times
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, Some(&user_config), false)
        .unwrap();

    assert_eq!(doc_sources.len(), 1);
}

#[test]
fn test_discover_no_documentation_found() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    // Create repo without any documentation
    create_test_repo(workspace, "repo1", "docs", false); // No index.rst

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", None)],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, None, false)
        .unwrap();

    assert_eq!(doc_sources.len(), 0);
}

#[test]
fn test_discover_without_user_config() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    create_test_repo(workspace, "repo1", "docs", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", None)],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    // Pass None for user_config - should still work with built-in defaults
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, None, false)
        .unwrap();

    assert_eq!(doc_sources.len(), 1);
}

#[test]
fn test_discover_with_comma_separated_user_dirs() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    // Create repos with different custom directories
    create_test_repo(workspace, "repo1", "custom1", true);
    create_test_repo(workspace, "repo2", "custom2", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![
            create_git_config("repo1", None),
            create_git_config("repo2", None),
        ],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    // Test comma-separated list with various whitespace patterns
    let user_config = UserConfig {
        documentation_dirs: Some("custom1, custom2,  custom3  ".to_string()),
        ..Default::default()
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, Some(&user_config), false)
        .unwrap();

    // Should find both repos using comma-separated user config dirs
    assert_eq!(doc_sources.len(), 2);
}

#[test]
fn test_discover_with_empty_entries_in_comma_list() {
    let temp_dir = tempdir().unwrap();
    let workspace = temp_dir.path();

    create_test_repo(workspace, "repo1", "docs", true);

    let sdk_config = SdkConfig {
        mirror: PathBuf::from("/tmp/mirror"),
        gits: vec![create_git_config("repo1", None)],
        toolchains: None,
        copy_files: None,
        install: None,
        makefile_include: None,
        envsetup: None,
        test: None,
        clean: None,
        build: None,
        flash: None,
        variables: None,
    };

    // Test with empty entries and whitespace-only entries
    let user_config = UserConfig {
        documentation_dirs: Some("custom1, , custom2,   , custom3".to_string()),
        ..Default::default()
    };

    let doc_manager = DocManager::new(workspace.to_path_buf());
    let doc_sources = doc_manager
        .discover_doc_sources(&sdk_config, Some(&user_config), false)
        .unwrap();

    // Should still find docs using built-in defaults, empty entries skipped
    assert_eq!(doc_sources.len(), 1);
}
