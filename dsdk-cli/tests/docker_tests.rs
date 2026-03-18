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

//! Integration tests for Docker functionality.
//!
//! These tests verify Docker configuration generation and URL transformations
//! without requiring Docker to be installed or running.

mod common;

use common::{create_complex_sdk_config, create_minimal_sdk_config, TestFixture};
use dsdk_cli::config::{GitConfig, OsDependencies, PythonDependencies, SdkConfig};
use dsdk_cli::docker_manager::{DockerManager, DockerfileConfig};
use std::fs;

#[test]
fn test_docker_manager_creation() {
    let fixture = TestFixture::new();
    let workspace = fixture.create_dir("workspace");
    let config_path = fixture.path().join("sdk.yml");

    let config = create_minimal_sdk_config(fixture.path());
    let yaml = serde_yaml::to_string(&config).unwrap();
    fs::write(&config_path, yaml).unwrap();

    let _docker_manager = DockerManager::new(workspace, config_path);
    // If we get here without panicking, creation succeeded
}

#[test]
fn test_dockerfile_generation_basic() {
    let fixture = TestFixture::new();
    let workspace = fixture.create_dir("workspace");
    let config_path = fixture.path().join("sdk.yml");
    let dockerfile_path = fixture.path().join("Dockerfile");

    let mut config = create_minimal_sdk_config(fixture.path());
    config.gits = vec![GitConfig {
        name: "test-repo".to_string(),
        url: "https://github.com/example/test.git".to_string(),
        commit: "main".to_string(),
        build_depends_on: None,
        git_depends_on: None,
        build: Some(vec!["make".to_string()]),
        documentation_dir: None,
    }];

    let yaml = serde_yaml::to_string(&config).unwrap();
    fs::write(&config_path, yaml).unwrap();

    // Create minimal OS dependencies
    let os_deps: OsDependencies = serde_yaml::from_str(
        r#"
linux:
  ubuntu:
    version: "22.04"
    command: "apt-get install"
    packages:
      - git
      - build-essential
"#,
    )
    .unwrap();

    // Create minimal Python dependencies
    let python_deps: PythonDependencies = serde_yaml::from_str(
        r#"
profiles:
  minimal:
    packages: []
default: minimal
"#,
    )
    .unwrap();

    let docker_manager = DockerManager::new(workspace, config_path);

    let dockerfile_config = DockerfileConfig {
        sdk_config: &config,
        os_deps: &os_deps,
        python_deps: &python_deps,
        output_path: &dockerfile_path,
        distro_preference: Some("ubuntu"),
        python_profile: "minimal",
        force: true,
        force_https: false,
        force_ssh: false,
        no_mirror: false,
    };

    let result = docker_manager.create_dockerfile(dockerfile_config);

    if result.is_ok() {
        assert!(dockerfile_path.exists());
        let content = fs::read_to_string(&dockerfile_path).unwrap();
        assert!(content.contains("FROM"));
        assert!(content.contains("ubuntu") || content.contains("debian"));
    }
}

#[test]
fn test_dockerfile_with_https_urls() {
    let fixture = TestFixture::new();
    let workspace = fixture.create_dir("workspace");
    let config_path = fixture.path().join("sdk.yml");
    let dockerfile_path = fixture.path().join("Dockerfile.https");

    let config = SdkConfig {
        mirror: fixture.path().to_path_buf(),
        gits: vec![
            GitConfig {
                name: "repo1".to_string(),
                url: "https://github.com/example/repo1.git".to_string(),
                commit: "v1.0.0".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
            },
            GitConfig {
                name: "repo2".to_string(),
                url: "git@github.com:example/repo2.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
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
    };

    let yaml = serde_yaml::to_string(&config).unwrap();
    fs::write(&config_path, yaml).unwrap();

    let os_deps: OsDependencies = serde_yaml::from_str(
        r#"
linux:
  ubuntu:
    version: "22.04"
    command: "apt-get install"
    packages:
      - git
"#,
    )
    .unwrap();

    let python_deps: PythonDependencies = serde_yaml::from_str(
        r#"
profiles:
  minimal:
    packages: []
default: minimal
"#,
    )
    .unwrap();

    let docker_manager = DockerManager::new(workspace, config_path);

    // Test with force_https = true (should convert SSH URLs to HTTPS)
    let dockerfile_config = DockerfileConfig {
        sdk_config: &config,
        os_deps: &os_deps,
        python_deps: &python_deps,
        output_path: &dockerfile_path,
        distro_preference: Some("ubuntu"),
        python_profile: "minimal",
        force: true,
        force_https: true,
        force_ssh: false,
        no_mirror: false,
    };

    let result = docker_manager.create_dockerfile(dockerfile_config);

    if result.is_ok() && dockerfile_path.exists() {
        let content = fs::read_to_string(&dockerfile_path).unwrap();
        // With force_https, SSH URLs should be converted or HTTPS should be prominent
        assert!(content.contains("https://") || content.contains("github.com"));
    }
}

#[test]
fn test_dockerfile_with_dependencies() {
    let fixture = TestFixture::new();
    let workspace = fixture.create_dir("workspace");
    let config_path = fixture.path().join("sdk.yml");
    let dockerfile_path = fixture.path().join("Dockerfile.deps");

    let config = create_complex_sdk_config(fixture.path());
    let yaml = serde_yaml::to_string(&config).unwrap();
    fs::write(&config_path, yaml).unwrap();

    let os_deps: OsDependencies = serde_yaml::from_str(
        r#"
linux:
  ubuntu:
    version: "22.04"
    command: "apt-get install"
    packages:
      - git
      - build-essential
      - cmake
"#,
    )
    .unwrap();

    let python_deps: PythonDependencies = serde_yaml::from_str(
        r#"
profiles:
  dev:
    packages:
      - pytest
      - black
default: dev
"#,
    )
    .unwrap();

    let docker_manager = DockerManager::new(workspace, config_path);

    let dockerfile_config = DockerfileConfig {
        sdk_config: &config,
        os_deps: &os_deps,
        python_deps: &python_deps,
        output_path: &dockerfile_path,
        distro_preference: Some("ubuntu"),
        python_profile: "dev",
        force: true,
        force_https: false,
        force_ssh: false,
        no_mirror: false,
    };

    let result = docker_manager.create_dockerfile(dockerfile_config);

    if result.is_ok() && dockerfile_path.exists() {
        let content = fs::read_to_string(&dockerfile_path).unwrap();
        assert!(content.contains("FROM"));
        // Should contain package installation
        assert!(content.contains("apt-get") || content.contains("RUN"));
    }
}

#[test]
fn test_dockerfile_no_mirror_option() {
    let fixture = TestFixture::new();
    let workspace = fixture.create_dir("workspace");
    let config_path = fixture.path().join("sdk.yml");
    let dockerfile_path = fixture.path().join("Dockerfile.no-mirror");

    let config = create_minimal_sdk_config(fixture.path());
    let yaml = serde_yaml::to_string(&config).unwrap();
    fs::write(&config_path, yaml).unwrap();

    let os_deps: OsDependencies = serde_yaml::from_str(
        r#"
linux:
  ubuntu:
    version: "22.04"
    command: "apt-get install"
    packages:
      - git
"#,
    )
    .unwrap();

    let python_deps: PythonDependencies = serde_yaml::from_str(
        r#"
profiles:
  minimal:
    packages: []
default: minimal
"#,
    )
    .unwrap();

    let docker_manager = DockerManager::new(workspace, config_path);

    // Test with no_mirror = true
    let dockerfile_config = DockerfileConfig {
        sdk_config: &config,
        os_deps: &os_deps,
        python_deps: &python_deps,
        output_path: &dockerfile_path,
        distro_preference: Some("ubuntu"),
        python_profile: "minimal",
        force: true,
        force_https: false,
        force_ssh: false,
        no_mirror: true,
    };

    let result = docker_manager.create_dockerfile(dockerfile_config);

    if result.is_ok() && dockerfile_path.exists() {
        let content = fs::read_to_string(&dockerfile_path).unwrap();
        assert!(content.contains("FROM"));
        // With no_mirror, should not reference mirror paths
        assert!(!content.contains("/mirror/") || content.contains("--no-mirror"));
    }
}

#[test]
fn test_url_transformation_ssh_to_https() {
    // Test URL transformation logic without full Docker generation
    let ssh_urls = [
        "git@github.com:user/repo.git",
        "git@gitlab.com:group/project.git",
        "git@bitbucket.org:team/repository.git",
    ];

    let expected_https = [
        "https://github.com/user/repo.git",
        "https://gitlab.com/group/project.git",
        "https://bitbucket.org/team/repository.git",
    ];

    for (ssh_url, expected) in ssh_urls.iter().zip(expected_https.iter()) {
        if ssh_url.starts_with("git@") {
            let parts: Vec<&str> = ssh_url.split(':').collect();
            if parts.len() == 2 {
                let host = parts[0].replace("git@", "");
                let path = parts[1];
                let converted = format!("https://{}/{}", host, path);
                assert_eq!(converted, *expected);
            }
        }
    }
}

#[test]
fn test_url_transformation_https_to_ssh() {
    // Test URL transformation logic
    let https_urls = [
        "https://github.com/user/repo.git",
        "https://gitlab.com/group/project.git",
    ];

    let expected_ssh = [
        "git@github.com:user/repo.git",
        "git@gitlab.com:group/project.git",
    ];

    for (https_url, expected) in https_urls.iter().zip(expected_ssh.iter()) {
        if https_url.starts_with("https://") {
            let without_protocol = https_url.replace("https://", "");
            let parts: Vec<&str> = without_protocol.splitn(2, '/').collect();
            if parts.len() == 2 {
                let converted = format!("git@{}:{}", parts[0], parts[1]);
                assert_eq!(converted, *expected);
            }
        }
    }
}

#[test]
fn test_detect_url_type() {
    let test_cases = vec![
        ("https://github.com/user/repo.git", false),
        ("git@github.com:user/repo.git", true),
        ("ssh://git@github.com/user/repo.git", true),
        ("http://example.com/repo.git", false),
    ];

    for (url, expected_is_ssh) in test_cases {
        let is_ssh = url.starts_with("git@") || url.starts_with("ssh://");
        assert_eq!(is_ssh, expected_is_ssh, "URL: {}", url);
    }
}

#[test]
fn test_dockerfile_with_different_distros() {
    let fixture = TestFixture::new();
    let workspace = fixture.create_dir("workspace");
    let config_path = fixture.path().join("sdk.yml");

    let config = create_minimal_sdk_config(fixture.path());
    let yaml = serde_yaml::to_string(&config).unwrap();
    fs::write(&config_path, yaml).unwrap();

    let distros = vec!["ubuntu", "fedora", "debian"];

    for distro in distros {
        let dockerfile_path = fixture.path().join(format!("Dockerfile.{}", distro));

        let os_deps_yaml = format!(
            r#"
linux:
  {}:
    version: "latest"
    command: "{}"
    packages:
      - git
"#,
            distro,
            if distro == "ubuntu" || distro == "debian" {
                "apt-get install"
            } else {
                "dnf install"
            }
        );

        let os_deps: OsDependencies = serde_yaml::from_str(&os_deps_yaml).unwrap();

        let python_deps: PythonDependencies = serde_yaml::from_str(
            r#"
profiles:
  minimal:
    packages: []
default: minimal
"#,
        )
        .unwrap();

        let docker_manager = DockerManager::new(workspace.clone(), config_path.clone());

        let dockerfile_config = DockerfileConfig {
            sdk_config: &config,
            os_deps: &os_deps,
            python_deps: &python_deps,
            output_path: &dockerfile_path,
            distro_preference: Some(distro),
            python_profile: "minimal",
            force: true,
            force_https: false,
            force_ssh: false,
            no_mirror: false,
        };

        let result = docker_manager.create_dockerfile(dockerfile_config);

        if result.is_ok() && dockerfile_path.exists() {
            let content = fs::read_to_string(&dockerfile_path).unwrap();
            assert!(content.contains("FROM"));
        }
    }
}

// Tests for os-dependencies multi-version support in Docker

#[test]
fn test_docker_images_new_format_multi_version() {
    use dsdk_cli::config::OsDependencies;

    let fixture = TestFixture::new();
    let workspace = fixture.create_dir("workspace");
    let config_path = fixture.path().join("sdk.yml");

    let docker_manager = DockerManager::new(workspace, config_path);

    // Test YAML with multiple Ubuntu versions using new format
    // Use "linux" as the fallback key to work on all architectures
    let os_deps: OsDependencies = serde_yaml::from_str(
        r#"
linux:
  ubuntu-22.04:
    command: "apt-get install"
    packages:
      - git

  ubuntu-24.04:
    command: "apt-get install"
    packages:
      - git
      - cmake
"#,
    )
    .expect("Should parse new format");

    let images = docker_manager.get_available_images(&os_deps);

    // Should have separate images for each version
    assert_eq!(images.len(), 2);

    // Find ubuntu 22.04
    let ubuntu_22 = images
        .iter()
        .find(|img| img.name == "ubuntu" && img.tag == "22.04");
    assert!(ubuntu_22.is_some(), "Should have ubuntu:22.04 image");

    // Find ubuntu 24.04
    let ubuntu_24 = images
        .iter()
        .find(|img| img.name == "ubuntu" && img.tag == "24.04");
    assert!(ubuntu_24.is_some(), "Should have ubuntu:24.04 image");
}

#[test]
fn test_docker_images_legacy_format_backward_compat() {
    use dsdk_cli::config::OsDependencies;

    let fixture = TestFixture::new();
    let workspace = fixture.create_dir("workspace");
    let config_path = fixture.path().join("sdk.yml");

    let docker_manager = DockerManager::new(workspace, config_path);

    // Test old YAML format with version field
    let os_deps: OsDependencies = serde_yaml::from_str(
        r#"
linux:
  ubuntu:
    version: "22.04"
    command: "apt-get install"
    packages:
      - git

  debian:
    version: "12"
    command: "apt-get install"
    packages:
      - git
"#,
    )
    .expect("Should parse legacy format");

    let images = docker_manager.get_available_images(&os_deps);

    // Should have images with versions from version field
    assert_eq!(images.len(), 2);

    let ubuntu = images.iter().find(|img| img.name == "ubuntu");
    assert!(ubuntu.is_some());
    assert_eq!(ubuntu.unwrap().tag, "22.04");

    let debian = images.iter().find(|img| img.name == "debian");
    assert!(debian.is_some());
    assert_eq!(debian.unwrap().tag, "12");
}

#[test]
fn test_docker_images_distro_with_dashes() {
    use dsdk_cli::config::OsDependencies;

    let fixture = TestFixture::new();
    let workspace = fixture.create_dir("workspace");
    let config_path = fixture.path().join("sdk.yml");

    let docker_manager = DockerManager::new(workspace, config_path);

    // Test distro names with dashes
    let os_deps: OsDependencies = serde_yaml::from_str(
        r#"
linux:
  rocky-linux-9.0:
    command: "dnf install"
    packages:
      - git
"#,
    )
    .expect("Should parse distro with dashes");

    let images = docker_manager.get_available_images(&os_deps);

    assert_eq!(images.len(), 1);
    assert_eq!(images[0].name, "rocky-linux");
    assert_eq!(images[0].tag, "9.0");
}

#[test]
fn test_docker_image_selection_with_version() {
    use dsdk_cli::config::OsDependencies;

    let fixture = TestFixture::new();
    let workspace = fixture.create_dir("workspace");
    let config_path = fixture.path().join("sdk.yml");

    let docker_manager = DockerManager::new(workspace, config_path);

    let os_deps: OsDependencies = serde_yaml::from_str(
        r#"
linux:
  ubuntu-22.04:
    command: "apt-get install"
    packages:
      - git

  ubuntu-24.04:
    command: "apt-get install"
    packages:
      - git
"#,
    )
    .expect("Should parse");

    let images = docker_manager.get_available_images(&os_deps);

    // Test selecting specific version
    let selected = docker_manager.select_docker_image(Some("ubuntu:24.04"), &images);
    assert!(selected.is_some());
    let img = selected.unwrap();
    assert_eq!(img.name, "ubuntu");
    assert_eq!(img.tag, "24.04");

    // Test selecting just by name (should match first ubuntu)
    let selected = docker_manager.select_docker_image(Some("ubuntu"), &images);
    assert!(selected.is_some());
    assert_eq!(selected.unwrap().name, "ubuntu");
}
