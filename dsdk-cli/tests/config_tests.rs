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

//! Integration tests for configuration loading and validation.
//!
//! These tests verify that SDK configurations can be loaded correctly,
//! validated, and handle various edge cases properly.

mod common;

use common::{create_complex_sdk_config, create_minimal_sdk_config, write_sdk_config, TestFixture};
use dsdk_cli::config::{load_config, load_os_dependencies, load_python_dependencies, GitConfig};

#[test]
fn test_load_valid_minimal_config() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    let config = create_minimal_sdk_config();
    write_sdk_config(&config, &config_path);

    let loaded = load_config(&config_path).expect("Should load minimal config");
    assert_eq!(loaded.gits.len(), 0);
}

#[test]
fn test_load_config_with_repositories() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    let mut config = create_minimal_sdk_config();
    config.gits = vec![
        GitConfig {
            name: "repo1".to_string(),
            url: "https://github.com/example/repo1.git".to_string(),
            commit: "v1.0.0".to_string(),
            build_depends_on: None,
            git_depends_on: None,
            build: Some(vec!["make".to_string()]),
            documentation_dir: None,
            python_deps: None,
        },
        GitConfig {
            name: "repo2".to_string(),
            url: "https://github.com/example/repo2.git".to_string(),
            commit: "main".to_string(),
            build_depends_on: Some(vec!["repo1".to_string()]),
            git_depends_on: None,
            build: None,
            documentation_dir: None,
            python_deps: None,
        },
    ];

    write_sdk_config(&config, &config_path);

    let loaded = load_config(&config_path).expect("Should load config with repositories");
    assert_eq!(loaded.gits.len(), 2);
    assert_eq!(loaded.gits[0].name, "repo1");
    assert_eq!(loaded.gits[1].name, "repo2");
    assert!(loaded.gits[1].build_depends_on.is_some());
}

#[test]
fn test_load_config_with_numeric_commit() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    // Test that numeric commits (like version numbers) are handled correctly
    let yaml_content = format!(
        "mirror: {}\ngits:\n  - name: test\n    url: https://example.com/test.git\n    commit: 2025.05\n",
        fixture.path().display()
    );

    fixture.write_file("sdk.yml", &yaml_content);

    let loaded = load_config(&config_path).expect("Should handle numeric commit");
    assert_eq!(loaded.gits[0].commit, "2025.05");
}

#[test]
fn test_load_config_with_build_commands() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    let yaml_content = format!(
        r#"mirror: {}
gits:
  - name: test
    url: https://example.com/test.git
    commit: main
    build:
      - "@echo Building"
      - make all
      - make install
"#,
        fixture.path().display()
    );

    fixture.write_file("sdk.yml", &yaml_content);

    let loaded = load_config(&config_path).expect("Should load config with build commands");
    let build = loaded.gits[0]
        .build
        .as_ref()
        .expect("Should have build commands");
    assert_eq!(build.len(), 3);
    assert_eq!(build[0], "@echo Building");
    assert_eq!(build[1], "make all");
    assert_eq!(build[2], "make install");
}

#[test]
fn test_load_config_with_python_deps() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    // python-deps accepts both a single path (string) and a list of paths.
    let yaml_content = format!(
        r#"mirror: {}
gits:
  - name: single
    url: https://example.com/single.git
    commit: main
    python-deps: single/requirements.txt
  - name: multi
    url: https://example.com/multi.git
    commit: main
    python-deps:
      - multi/requirements.txt
      - multi/docs/requirements.txt
  - name: none
    url: https://example.com/none.git
    commit: main
"#,
        fixture.path().display()
    );

    fixture.write_file("sdk.yml", &yaml_content);

    let loaded = load_config(&config_path).expect("Should load config with python-deps");
    assert_eq!(
        loaded.gits[0].python_deps.as_deref(),
        Some(&["single/requirements.txt".to_string()][..])
    );
    assert_eq!(
        loaded.gits[1].python_deps.as_deref(),
        Some(
            &[
                "multi/requirements.txt".to_string(),
                "multi/docs/requirements.txt".to_string()
            ][..]
        )
    );
    assert!(loaded.gits[2].python_deps.is_none());
}

#[test]
fn test_load_config_with_dependencies() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    let config = create_complex_sdk_config();
    write_sdk_config(&config, &config_path);

    let loaded = load_config(&config_path).expect("Should load complex config");
    assert_eq!(loaded.gits.len(), 3);

    // Verify dependency chain
    assert!(loaded.gits[0].build_depends_on.is_none());
    assert_eq!(loaded.gits[1].build_depends_on.as_ref().unwrap().len(), 1);
    assert_eq!(loaded.gits[2].build_depends_on.as_ref().unwrap().len(), 2);
}

#[test]
fn test_load_config_missing_file() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("nonexistent.yml");

    let result = load_config(&config_path);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_load_config_invalid_yaml() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("invalid.yml");

    fixture.write_file("invalid.yml", "not: valid: yaml: [[[");

    let result = load_config(&config_path);
    assert!(result.is_err());
}

#[test]
fn test_load_config_without_mirror_field() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("incomplete.yml");

    // 'mirror' is no longer a required manifest field. A config without it
    // loads successfully; the mirror is resolved separately.
    fixture.write_file("incomplete.yml", "gits: []");

    let loaded = load_config(&config_path).expect("Config without mirror should load");
    assert_eq!(loaded.gits.len(), 0);
}

#[test]
fn test_load_core_config() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    let config = create_minimal_sdk_config();
    write_sdk_config(&config, &config_path);

    let loaded = load_config(&config_path).expect("Should load core config");
    assert_eq!(loaded.gits.len(), 0);
}

#[test]
fn test_load_os_dependencies() {
    let fixture = TestFixture::new();
    let deps_path = fixture.path().join("os-deps.yml");

    let deps_yaml = r#"
linux:
  ubuntu:
    version: "22.04"
    command: "apt-get install"
    packages:
      - build-essential
      - git
      - cmake

macos:
  homebrew:
    version: "latest"
    command: "brew install"
    packages:
      - git
      - cmake
"#;

    fixture.write_file("os-deps.yml", deps_yaml);

    let loaded = load_os_dependencies(&deps_path).expect("Should load OS dependencies");
    assert!(loaded.os_configs.contains_key("linux"));
    assert!(loaded.os_configs.contains_key("macos"));

    let linux_config = &loaded.os_configs["linux"];
    assert!(linux_config.distros.contains_key("ubuntu"));
}

#[test]
fn test_load_python_dependencies() {
    let fixture = TestFixture::new();
    let deps_path = fixture.path().join("python-deps.yml");

    let deps_yaml = r#"
profiles:
  minimal:
    packages:
      - pyyaml
  
  dev:
    packages:
      - pytest
      - black
      - mypy

default: minimal
"#;

    fixture.write_file("python-deps.yml", deps_yaml);

    let loaded = load_python_dependencies(&deps_path).expect("Should load Python dependencies");
    assert_eq!(loaded.profiles.len(), 2);
    assert!(loaded.profiles.contains_key("minimal"));
    assert!(loaded.profiles.contains_key("dev"));
    assert_eq!(loaded.default, "minimal");
}

#[test]
fn test_load_python_dependencies_with_requirements() {
    let fixture = TestFixture::new();
    let deps_path = fixture.path().join("python-deps.yml");

    // A profile may list requirements.txt paths, with or without inline packages.
    let deps_yaml = r#"
profiles:
  docs:
    packages:
      - sphinx
    requirements:
      - docs/requirements.txt
  reqs-only:
    requirements:
      - tools/requirements.txt

default: docs
"#;

    fixture.write_file("python-deps.yml", deps_yaml);

    let loaded = load_python_dependencies(&deps_path).expect("Should load Python dependencies");
    let docs = loaded.profiles.get("docs").expect("docs profile present");
    assert_eq!(docs.packages, vec!["sphinx".to_string()]);
    assert_eq!(
        docs.requirements.as_deref(),
        Some(&["docs/requirements.txt".to_string()][..])
    );

    // Profiles with only requirements (no inline packages) must still parse.
    let reqs_only = loaded
        .profiles
        .get("reqs-only")
        .expect("reqs-only profile present");
    assert!(reqs_only.packages.is_empty());
    assert_eq!(
        reqs_only.requirements.as_deref(),
        Some(&["tools/requirements.txt".to_string()][..])
    );
}

#[test]
fn test_config_with_yaml_anchors() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    let yaml_with_anchors = format!(
        r#"mirror: {}

common_build: &standard_build
  - make all
  - make install

gits:
  - name: repo1
    url: https://github.com/example/repo1.git
    commit: main
    build: *standard_build
  
  - name: repo2
    url: https://github.com/example/repo2.git
    commit: main
    build: *standard_build
"#,
        fixture.path().display()
    );

    fixture.write_file("sdk.yml", &yaml_with_anchors);

    let loaded = load_config(&config_path).expect("Should load config with anchors");
    assert_eq!(loaded.gits.len(), 2);

    // Both repos should have identical build commands from the anchor
    let build1 = loaded.gits[0].build.as_ref().unwrap();
    let build2 = loaded.gits[1].build.as_ref().unwrap();
    assert_eq!(build1, build2);
    assert_eq!(build1.len(), 2);
}

#[test]
fn test_config_serialization_roundtrip() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    let original = create_complex_sdk_config();
    write_sdk_config(&original, &config_path);

    let loaded = load_config(&config_path).expect("Should load config");

    // Verify all fields match.
    assert_eq!(loaded.gits.len(), original.gits.len());

    for (loaded_git, original_git) in loaded.gits.iter().zip(original.gits.iter()) {
        assert_eq!(loaded_git.name, original_git.name);
        assert_eq!(loaded_git.url, original_git.url);
        assert_eq!(loaded_git.commit, original_git.commit);
        assert_eq!(loaded_git.build_depends_on, original_git.build_depends_on);
        assert_eq!(loaded_git.build, original_git.build);
    }
}

#[test]
fn test_config_with_multiline_build_commands() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    let yaml_content = format!(
        r#"mirror: {}
gits:
  - name: test
    url: https://example.com/test.git
    commit: main
    build: |
      make clean
      make all
      make test
"#,
        fixture.path().display()
    );

    fixture.write_file("sdk.yml", &yaml_content);

    let loaded = load_config(&config_path).expect("Should load config with multiline build");
    let build = loaded.gits[0]
        .build
        .as_ref()
        .expect("Should have build commands");
    assert_eq!(build.len(), 3);
    assert_eq!(build[0], "make clean");
    assert_eq!(build[1], "make all");
    assert_eq!(build[2], "make test");
}

#[test]
fn test_manifest_mirror_key_is_parsed_but_deprecated() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    // A manifest carrying the deprecated `mirror:` key loads and the value is
    // parsed into the field (honored for backward compatibility, below
    // --mirror and the user config in precedence).
    let yaml_content = r#"mirror: /tmp/mirror
gits:
  - name: test
    url: https://example.com/test.git
    commit: main
"#;

    fixture.write_file("sdk.yml", yaml_content);

    let loaded = load_config(&config_path).expect("Should load config");
    assert_eq!(loaded.gits.len(), 1);
    assert_eq!(loaded.gits[0].name, "test");
    assert_eq!(
        loaded.mirror.as_deref(),
        Some(std::path::Path::new("/tmp/mirror"))
    );
}

#[test]
fn test_user_config_list_all_empty() {
    use dsdk_cli::config::UserConfig;

    let config = UserConfig::default();
    let list = config.list_all();
    assert!(list.is_empty(), "Default config should have no values");
}

#[test]
fn test_user_config_list_all_simple_fields() {
    use dsdk_cli::config::UserConfig;
    use std::path::PathBuf;

    let config = UserConfig {
        mirror: Some(PathBuf::from("/custom/mirror")),
        default_workspace: Some(PathBuf::from("/home/user/workspace")),
        workspace_prefix: Some("myprefix-".to_string()),
        default_source: Some("https://example.com/manifests".to_string()),
        alternate_sources: None,
        no_mirror: Some(true),
        copy_files: None,
        shell: Some("/bin/zsh".to_string()),
        shell_arg: Some("-c".to_string()),
        documentation_dirs: Some("docs,manuals".to_string()),
        cert_validation: None,
        no_dividers: None,
    };

    let list = config.list_all();
    assert_eq!(list.len(), 8);
    assert!(list.contains(&"mirror=/custom/mirror".to_string()));
    assert!(list.contains(&"default_workspace=/home/user/workspace".to_string()));
    assert!(list.contains(&"workspace_prefix=myprefix-".to_string()));
    assert!(list.contains(&"default_source=https://example.com/manifests".to_string()));
    assert!(list.contains(&"no_mirror=true".to_string()));
    assert!(list.contains(&"shell=/bin/zsh".to_string()));
    assert!(list.contains(&"shell_arg=-c".to_string()));
    assert!(list.contains(&"documentation_dirs=docs,manuals".to_string()));
}

#[test]
fn test_user_config_list_all_with_copy_files() {
    use dsdk_cli::config::{CopyFileConfig, UserConfig};
    use std::path::PathBuf;

    let config = UserConfig {
        mirror: Some(PathBuf::from("/mirror")),
        copy_files: Some(vec![
            CopyFileConfig {
                source: "file1.txt".to_string(),
                dest: "dest1.txt".to_string(),
                cache: None,
                sha256: None,
                post_data: None,
                symlink: None,
            },
            CopyFileConfig {
                source: "file2.txt".to_string(),
                dest: "dest2.txt".to_string(),
                cache: None,
                sha256: None,
                post_data: None,
                symlink: None,
            },
        ]),
        ..Default::default()
    };

    let list = config.list_all();
    assert!(list.contains(&"mirror=/mirror".to_string()));
    assert!(list.contains(&"copy_files.0.source=file1.txt".to_string()));
    assert!(list.contains(&"copy_files.0.dest=dest1.txt".to_string()));
    assert!(list.contains(&"copy_files.1.source=file2.txt".to_string()));
    assert!(list.contains(&"copy_files.1.dest=dest2.txt".to_string()));
    assert_eq!(list.len(), 5); // mirror + 4 copy_files entries
}

#[test]
fn test_user_config_get_value_simple() {
    use dsdk_cli::config::UserConfig;
    use std::path::PathBuf;

    let config = UserConfig {
        mirror: Some(PathBuf::from("/custom/mirror")),
        default_source: Some("https://example.com".to_string()),
        no_mirror: Some(false),
        ..Default::default()
    };

    assert_eq!(
        config.get_value("mirror"),
        Some("/custom/mirror".to_string())
    );
    assert_eq!(
        config.get_value("default_source"),
        Some("https://example.com".to_string())
    );
    assert_eq!(config.get_value("no_mirror"), Some("false".to_string()));
    assert_eq!(config.get_value("nonexistent"), None);
}

#[test]
fn test_user_config_get_value_copy_files() {
    use dsdk_cli::config::{CopyFileConfig, UserConfig};

    let config = UserConfig {
        copy_files: Some(vec![CopyFileConfig {
            source: "source.txt".to_string(),
            dest: "destination.txt".to_string(),
            cache: None,
            sha256: None,
            post_data: None,
            symlink: None,
        }]),
        ..Default::default()
    };

    assert_eq!(
        config.get_value("copy_files.0.source"),
        Some("source.txt".to_string())
    );
    assert_eq!(
        config.get_value("copy_files.0.dest"),
        Some("destination.txt".to_string())
    );
    assert_eq!(config.get_value("copy_files.1.source"), None);
    assert_eq!(config.get_value("copy_files.0.invalid"), None);
}

#[test]
fn test_user_config_load_and_list() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("config.toml");

    let config_content = r#"
mirror = "/custom/mirror"
default_source = "https://example.com/manifests"
workspace_prefix = "myprefix-"
no_mirror = true
"#;

    fixture.write_file("config.toml", config_content);

    let config = dsdk_cli::config::UserConfig::load_from(&config_path)
        .expect("Should load config")
        .expect("Config should exist");

    let list = config.list_all();
    assert_eq!(list.len(), 4);
    assert!(list.contains(&"mirror=/custom/mirror".to_string()));
    assert!(list.contains(&"default_source=https://example.com/manifests".to_string()));
    assert!(list.contains(&"workspace_prefix=myprefix-".to_string()));
    assert!(list.contains(&"no_mirror=true".to_string()));
}

#[test]
fn test_user_config_partial_config() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("config.toml");

    let config_content = r#"
mirror = "/some/mirror"
"#;

    fixture.write_file("config.toml", config_content);

    let config = dsdk_cli::config::UserConfig::load_from(&config_path)
        .expect("Should load config")
        .expect("Config should exist");

    let list = config.list_all();
    assert_eq!(list.len(), 1);
    assert!(list.contains(&"mirror=/some/mirror".to_string()));
}

#[test]
fn test_user_config_missing_file() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("nonexistent.toml");

    let config = dsdk_cli::config::UserConfig::load_from(&config_path)
        .expect("Should not error on missing file");

    assert!(config.is_none(), "Missing config should return None");
}

#[test]
fn test_config_template_generation() {
    use dsdk_cli::config::UserConfig;

    let template = UserConfig::generate_template();

    // Template should not be empty
    assert!(!template.is_empty());

    // Template should contain key sections
    assert!(template.contains("Code in Motion Configuration"));
    assert!(template.contains("mirror"));
    assert!(template.contains("default_source"));
    assert!(template.contains("documentation_dirs"));

    // Template should be valid TOML
    let parsed: Result<UserConfig, _> = toml::from_str(&template);
    assert!(parsed.is_ok(), "Template should be valid TOML");
}

#[test]
fn test_config_validate_valid_file() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("config.toml");

    let config_content = r#"
mirror = "/test/mirror"
default_source = "https://example.com"
"#;

    fixture.write_file("config.toml", config_content);

    // Should load without error
    let result = dsdk_cli::config::UserConfig::load_from(&config_path);
    assert!(result.is_ok());
    assert!(result.unwrap().is_some());
}

#[test]
fn test_config_validate_invalid_toml() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("config.toml");

    let invalid_content = r#"
mirror = /missing/quotes
this is not valid toml
"#;

    fixture.write_file("config.toml", invalid_content);

    // Should fail to parse
    let result = dsdk_cli::config::UserConfig::load_from(&config_path);
    assert!(result.is_err());
}

#[test]
fn test_cert_validation_mode_default() {
    // Test default behavior (no CLI flag, no config)
    let (mode, show_warning) = dsdk_cli::config::get_cert_validation_mode(None);
    assert_eq!(mode, "strict");
    assert!(!show_warning);
}

#[test]
fn test_cert_validation_mode_cli_override() {
    // CLI flag should override everything
    let (mode, show_warning) = dsdk_cli::config::get_cert_validation_mode(Some("relaxed"));
    assert_eq!(mode, "relaxed");
    assert!(!show_warning); // No warning for explicit CLI choice

    let (mode, show_warning) = dsdk_cli::config::get_cert_validation_mode(Some("auto"));
    assert_eq!(mode, "auto");
    assert!(!show_warning);

    let (mode, show_warning) = dsdk_cli::config::get_cert_validation_mode(Some("strict"));
    assert_eq!(mode, "strict");
    assert!(!show_warning);
}

#[test]
fn test_cert_validation_in_user_config() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("config.toml");

    // Test strict mode
    let config_content = r#"
cert_validation = "strict"
"#;
    fixture.write_file("config.toml", config_content);
    let config = dsdk_cli::config::UserConfig::load_from(&config_path)
        .expect("Should load config")
        .expect("Should have config");
    assert_eq!(config.cert_validation, Some("strict".to_string()));

    // Test relaxed mode
    let config_content = r#"
cert_validation = "relaxed"
"#;
    fixture.write_file("config.toml", config_content);
    let config = dsdk_cli::config::UserConfig::load_from(&config_path)
        .expect("Should load config")
        .expect("Should have config");
    assert_eq!(config.cert_validation, Some("relaxed".to_string()));

    // Test auto mode
    let config_content = r#"
cert_validation = "auto"
"#;
    fixture.write_file("config.toml", config_content);
    let config = dsdk_cli::config::UserConfig::load_from(&config_path)
        .expect("Should load config")
        .expect("Should have config");
    assert_eq!(config.cert_validation, Some("auto".to_string()));
}

#[test]
fn test_cert_validation_config_omitted() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("config.toml");

    // Config without cert_validation should use default
    let config_content = r#"
default_source = "/some/path"
"#;
    fixture.write_file("config.toml", config_content);
    let config = dsdk_cli::config::UserConfig::load_from(&config_path)
        .expect("Should load config")
        .expect("Should have config");
    assert_eq!(config.cert_validation, None);
}

// Tests for os-dependencies multi-version support

#[test]
fn test_parse_distro_key_new_format() {
    use dsdk_cli::config::OsDependencies;

    // Test standard Ubuntu format
    assert_eq!(
        OsDependencies::parse_distro_key("ubuntu-22.04"),
        Some(("ubuntu".to_string(), "22.04".to_string()))
    );

    // Test Ubuntu 24.04
    assert_eq!(
        OsDependencies::parse_distro_key("ubuntu-24.04"),
        Some(("ubuntu".to_string(), "24.04".to_string()))
    );

    // Test distro with dashes in name
    assert_eq!(
        OsDependencies::parse_distro_key("rocky-linux-9.0"),
        Some(("rocky-linux".to_string(), "9.0".to_string()))
    );

    // Test Debian
    assert_eq!(
        OsDependencies::parse_distro_key("debian-12"),
        Some(("debian".to_string(), "12".to_string()))
    );

    // Test Fedora with version
    assert_eq!(
        OsDependencies::parse_distro_key("fedora-39"),
        Some(("fedora".to_string(), "39".to_string()))
    );
}

#[test]
fn test_parse_distro_key_legacy_format() {
    use dsdk_cli::config::OsDependencies;

    // Legacy format without version should return None
    assert_eq!(OsDependencies::parse_distro_key("ubuntu"), None);
    assert_eq!(OsDependencies::parse_distro_key("debian"), None);
    assert_eq!(OsDependencies::parse_distro_key("fedora"), None);
    assert_eq!(OsDependencies::parse_distro_key("macos"), None);
}

#[test]
fn test_parse_distro_key_edge_cases() {
    use dsdk_cli::config::OsDependencies;

    // Invalid: no version after dash
    assert_eq!(OsDependencies::parse_distro_key("ubuntu-"), None);

    // Invalid: version not in X.Y format
    assert_eq!(OsDependencies::parse_distro_key("ubuntu-lts"), None);

    // Invalid: empty string
    assert_eq!(OsDependencies::parse_distro_key(""), None);

    // Valid: single digit version
    assert_eq!(
        OsDependencies::parse_distro_key("alpine-3.18"),
        Some(("alpine".to_string(), "3.18".to_string()))
    );
}

#[test]
fn test_os_dependencies_new_format_multi_version() {
    use dsdk_cli::config::OsDependencies;

    // Test YAML with multiple Ubuntu versions using new format
    let yaml_content = r#"
linux-x86_64:
  ubuntu-22.04:
    command: "apt-get install"
    packages:
      - git
      - build-essential

  ubuntu-24.04:
    command: "apt-get install"
    packages:
      - git
      - build-essential
      - cmake
"#;

    let os_deps: OsDependencies =
        serde_yaml::from_str(yaml_content).expect("Should parse new format with multiple versions");

    let os_config = os_deps
        .os_configs
        .get("linux-x86_64")
        .expect("Should have linux-x86_64 config");

    // Should have both versions as separate keys
    assert!(os_config.distros.contains_key("ubuntu-22.04"));
    assert!(os_config.distros.contains_key("ubuntu-24.04"));

    // Verify package lists
    let ubuntu_22 = &os_config.distros["ubuntu-22.04"];
    assert_eq!(ubuntu_22.package_manager.resolved_packages().len(), 2);

    let ubuntu_24 = &os_config.distros["ubuntu-24.04"];
    assert_eq!(ubuntu_24.package_manager.resolved_packages().len(), 3);
}

#[test]
fn test_os_dependencies_legacy_format_backward_compat() {
    use dsdk_cli::config::OsDependencies;

    // Test old YAML format with version field
    let yaml_content = r#"
linux:
  ubuntu:
    version: "22.04"
    command: "apt-get install"
    packages:
      - git
      - build-essential

  debian:
    version: "12"
    command: "apt-get install"
    packages:
      - git
"#;

    let os_deps: OsDependencies = serde_yaml::from_str(yaml_content)
        .expect("Should parse legacy format for backward compatibility");

    let os_config = os_deps
        .os_configs
        .get("linux")
        .expect("Should have linux config");

    // Should have distros with bare names
    assert!(os_config.distros.contains_key("ubuntu"));
    assert!(os_config.distros.contains_key("debian"));

    // Verify version field exists
    let ubuntu = &os_config.distros["ubuntu"];
    assert_eq!(ubuntu.version, Some("22.04".to_string()));

    let debian = &os_config.distros["debian"];
    assert_eq!(debian.version, Some("12".to_string()));
}

#[test]
fn test_os_dependencies_mixed_format() {
    use dsdk_cli::config::OsDependencies;

    // Test mixing new and legacy formats (new format should take precedence)
    let yaml_content = r#"
linux-x86_64:
  ubuntu-22.04:
    command: "apt-get install"
    packages:
      - git

  ubuntu-24.04:
    command: "apt-get install"
    packages:
      - git
      - cmake

  debian:
    version: "12"
    command: "apt-get install"
    packages:
      - git
"#;

    let os_deps: OsDependencies =
        serde_yaml::from_str(yaml_content).expect("Should parse mixed format");

    let os_config = os_deps
        .os_configs
        .get("linux-x86_64")
        .expect("Should have linux-x86_64 config");

    // Should have both new format keys
    assert!(os_config.distros.contains_key("ubuntu-22.04"));
    assert!(os_config.distros.contains_key("ubuntu-24.04"));

    // Should have legacy format key
    assert!(os_config.distros.contains_key("debian"));
    assert_eq!(os_config.distros["debian"].version, Some("12".to_string()));
}

#[test]
fn test_os_dependencies_distro_with_dashes() {
    use dsdk_cli::config::OsDependencies;

    // Test distro names with dashes (e.g., rocky-linux)
    let yaml_content = r#"
linux-x86_64:
  rocky-linux-9.0:
    command: "dnf install"
    packages:
      - git
      - gcc

  alma-linux-8.8:
    command: "dnf install"
    packages:
      - git
"#;

    let os_deps: OsDependencies =
        serde_yaml::from_str(yaml_content).expect("Should parse distro names with dashes");

    let os_config = os_deps
        .os_configs
        .get("linux-x86_64")
        .expect("Should have linux-x86_64 config");

    assert!(os_config.distros.contains_key("rocky-linux-9.0"));
    assert!(os_config.distros.contains_key("alma-linux-8.8"));

    // Verify parsing extracts correct names and versions
    assert_eq!(
        OsDependencies::parse_distro_key("rocky-linux-9.0"),
        Some(("rocky-linux".to_string(), "9.0".to_string()))
    );
    assert_eq!(
        OsDependencies::parse_distro_key("alma-linux-8.8"),
        Some(("alma-linux".to_string(), "8.8".to_string()))
    );
}

#[test]
fn test_os_dependencies_nested_packages_flat_compat() {
    use dsdk_cli::config::OsDependencies;

    let yaml_content = r#"
linux-x86_64:
  ubuntu-24.04:
    command: "apt install"
    packages:
      - git
      - cmake
      - build-essential
"#;

    let os_deps: OsDependencies =
        serde_yaml::from_str(yaml_content).expect("Should parse flat packages list");

    let distro = &os_deps.os_configs["linux-x86_64"].distros["ubuntu-24.04"];
    let packages = distro.package_manager.resolved_packages();
    assert_eq!(packages.len(), 3);
    assert!(packages.contains(&"git".to_string()));
    assert!(packages.contains(&"cmake".to_string()));
    assert!(packages.contains(&"build-essential".to_string()));
}

#[test]
fn test_os_dependencies_nested_packages_composed() {
    use dsdk_cli::config::OsDependencies;

    // Simulate what YAML anchors expand to: a list of lists
    let yaml_content = r#"
linux-x86_64:
  ubuntu-24.04:
    command: "apt install"
    packages:
      - - git
        - cmake
      - - gcc-multilib
        - g++-multilib
"#;

    let os_deps: OsDependencies =
        serde_yaml::from_str(yaml_content).expect("Should parse nested (composed) packages list");

    let distro = &os_deps.os_configs["linux-x86_64"].distros["ubuntu-24.04"];
    let packages = distro.package_manager.resolved_packages();
    assert_eq!(packages.len(), 4);
    assert!(packages.contains(&"git".to_string()));
    assert!(packages.contains(&"cmake".to_string()));
    assert!(packages.contains(&"gcc-multilib".to_string()));
    assert!(packages.contains(&"g++-multilib".to_string()));
}

#[test]
fn test_os_dependencies_nested_packages_dedup() {
    use dsdk_cli::config::OsDependencies;

    // Both groups contain "git" — should appear only once in the result
    let yaml_content = r#"
linux-x86_64:
  ubuntu-24.04:
    command: "apt install"
    packages:
      - - git
        - cmake
      - - git
        - gcc-multilib
"#;

    let os_deps: OsDependencies =
        serde_yaml::from_str(yaml_content).expect("Should parse nested packages with duplicates");

    let distro = &os_deps.os_configs["linux-x86_64"].distros["ubuntu-24.04"];
    let packages = distro.package_manager.resolved_packages();
    assert_eq!(packages.len(), 3, "duplicate 'git' should be removed");
    assert!(packages.contains(&"git".to_string()));
    assert!(packages.contains(&"cmake".to_string()));
    assert!(packages.contains(&"gcc-multilib".to_string()));
}

#[test]
fn test_os_dependencies_mixed_flat_and_nested_same_file() {
    use dsdk_cli::config::OsDependencies;

    // x86_64 uses nested (composed), aarch64 uses flat — both must parse correctly
    let yaml_content = r#"
linux-x86_64:
  ubuntu-24.04:
    command: "apt install"
    packages:
      - - git
        - cmake
      - - gcc-multilib

linux-aarch64:
  ubuntu-24.04:
    command: "apt install"
    packages:
      - git
      - cmake
"#;

    let os_deps: OsDependencies =
        serde_yaml::from_str(yaml_content).expect("Should parse file with mixed flat and nested");

    let x86_distro = &os_deps.os_configs["linux-x86_64"].distros["ubuntu-24.04"];
    let x86_pkgs = x86_distro.package_manager.resolved_packages();
    assert_eq!(x86_pkgs.len(), 3);
    assert!(x86_pkgs.contains(&"gcc-multilib".to_string()));

    let arm_distro = &os_deps.os_configs["linux-aarch64"].distros["ubuntu-24.04"];
    let arm_pkgs = arm_distro.package_manager.resolved_packages();
    assert_eq!(arm_pkgs.len(), 2);
    assert!(!arm_pkgs.contains(&"gcc-multilib".to_string()));
}

#[test]
fn test_resolve_mirror_cli_override_wins() {
    use dsdk_cli::workspace::resolve_mirror;
    use std::path::{Path, PathBuf};

    // The --mirror CLI override takes precedence over user config and the
    // built-in default, regardless of any manifest contents.
    let resolved = resolve_mirror(Some(Path::new("/explicit/cli/mirror")), None);
    assert_eq!(resolved, PathBuf::from("/explicit/cli/mirror"));
}

#[test]
fn test_apply_to_sdk_config_no_longer_overrides_mirror() {
    use dsdk_cli::config::{CopyFileConfig, UserConfig};

    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    let config = create_minimal_sdk_config();
    write_sdk_config(&config, &config_path);
    let mut loaded_sdk_config = load_config(&config_path).expect("Should load SDK config");

    // A user config that sets only `mirror` produces no SdkConfig overrides:
    // the mirror is resolved separately (via resolve_mirror), not applied here.
    let mirror_only = UserConfig {
        mirror: Some(fixture.path().join("user-mirror")),
        default_source: None,
        alternate_sources: None,
        default_workspace: None,
        workspace_prefix: None,
        no_mirror: None,
        copy_files: None,
        shell: None,
        shell_arg: None,
        documentation_dirs: None,
        cert_validation: None,
        no_dividers: None,
    };
    assert_eq!(
        mirror_only.apply_to_sdk_config(&mut loaded_sdk_config, false),
        0,
        "mirror-only user config should apply no SdkConfig overrides"
    );

    // copy_files is still applied by apply_to_sdk_config.
    let with_copy_files = UserConfig {
        copy_files: Some(vec![CopyFileConfig {
            source: "test.txt".to_string(),
            dest: "test-dest.txt".to_string(),
            cache: None,
            sha256: None,
            post_data: None,
            symlink: None,
        }]),
        ..Default::default()
    };
    assert_eq!(
        with_copy_files.apply_to_sdk_config(&mut loaded_sdk_config, false),
        1,
        "copy_files user config should still apply one override"
    );
    assert!(loaded_sdk_config.copy_files.is_some());
}

#[test]
fn test_toolchain_config_parses_sha256() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    let yaml_content = format!(
        r#"mirror: {}
gits: []
toolchains:
  - name: test-toolchain.tar.xz
    url: https://example.com/downloads/
    destination: toolchains/test
    sha256: abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890
    os: linux
    arch: x86_64
"#,
        fixture.path().display()
    );
    std::fs::write(&config_path, yaml_content).expect("Failed to write config");

    let loaded = load_config(&config_path).expect("Should load config with toolchain sha256");
    let toolchains = loaded.toolchains.expect("Should have toolchains section");
    assert_eq!(toolchains.len(), 1);
    assert_eq!(
        toolchains[0].sha256.as_deref(),
        Some("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890")
    );
}

#[test]
fn test_toolchain_config_parses_without_sha256() {
    let fixture = TestFixture::new();
    let config_path = fixture.path().join("sdk.yml");

    let yaml_content = format!(
        r#"mirror: {}
gits: []
toolchains:
  - name: test-toolchain.tar.xz
    url: https://example.com/downloads/
    destination: toolchains/test
    os: linux
    arch: x86_64
"#,
        fixture.path().display()
    );
    std::fs::write(&config_path, yaml_content).expect("Failed to write config");

    let loaded = load_config(&config_path).expect("Should load config without toolchain sha256");
    let toolchains = loaded.toolchains.expect("Should have toolchains section");
    assert_eq!(toolchains.len(), 1);
    assert!(
        toolchains[0].sha256.is_none(),
        "sha256 should be None when not specified"
    );
}
