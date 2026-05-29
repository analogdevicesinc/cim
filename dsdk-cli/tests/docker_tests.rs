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

//! Integration tests for the Docker Dockerfile generation feature.

use dsdk_cli::docker_manager::{create_dockerfile, generate_dockerfile, DockerfileConfig};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_generate_dockerfile_contains_from() {
    let config = DockerfileConfig {
        target: "my-target",
        version: None,
        source: None,
        distro: "ubuntu:22.04",
        output_path: std::path::Path::new("Dockerfile"),
        force: false,
    };
    let out = generate_dockerfile(&config);
    assert!(
        out.starts_with("FROM ubuntu:22.04\n"),
        "Dockerfile must start with FROM"
    );
}

#[test]
fn test_generate_dockerfile_cim_init_command() {
    let config = DockerfileConfig {
        target: "optee-qemu-v8",
        version: Some("main"),
        source: Some("https://github.com/analogdevicesinc/cim-manifests.git"),
        distro: "fedora:40",
        output_path: std::path::Path::new("Dockerfile"),
        force: false,
    };
    let out = generate_dockerfile(&config);
    assert!(out.contains("cim init --target optee-qemu-v8 --version main --source https://github.com/analogdevicesinc/cim-manifests.git --full --no-sudo --yes --cert-validation=auto --symlink"));
}

#[test]
fn test_generate_dockerfile_no_version_no_source() {
    let config = DockerfileConfig {
        target: "dummy1",
        version: None,
        source: None,
        distro: "ubuntu:22.04",
        output_path: std::path::Path::new("Dockerfile"),
        force: false,
    };
    let out = generate_dockerfile(&config);
    assert!(out.contains(
        "cim init --target dummy1 --full --no-sudo --yes --cert-validation=auto --symlink"
    ));
    assert!(
        !out.contains("--version"),
        "no --version when not specified"
    );
    assert!(!out.contains("--source"), "no --source when not specified");
}

#[test]
fn test_generate_dockerfile_downloads_from_github() {
    let config = DockerfileConfig {
        target: "t",
        version: None,
        source: None,
        distro: "ubuntu:22.04",
        output_path: std::path::Path::new("Dockerfile"),
        force: false,
    };
    let out = generate_dockerfile(&config);
    assert!(out.contains("api.github.com/repos/analogdevicesinc/cim/releases/latest"));
    assert!(out.contains("cim-suite-${LATEST}-${CIM_TARGET}.tar.gz"));
    assert!(out.contains("x86_64-unknown-linux-musl"));
    assert!(out.contains("aarch64-unknown-linux-gnu"));
    assert!(out.contains("/root/bin/cim"));
}

#[test]
fn test_generate_dockerfile_path_setup() {
    let config = DockerfileConfig {
        target: "t",
        version: None,
        source: None,
        distro: "ubuntu:22.04",
        output_path: std::path::Path::new("Dockerfile"),
        force: false,
    };
    let out = generate_dockerfile(&config);
    assert!(out.contains("ENV HOME=/root"));
    assert!(out.contains("/root/bin"));
    assert!(out.contains("/root/tmp/mirror"));
    assert!(out.contains("ENV PATH=\"/root/bin:${PATH}\""));
}

#[test]
fn test_generate_dockerfile_default_cmd() {
    let config = DockerfileConfig {
        target: "t",
        version: None,
        source: None,
        distro: "ubuntu:22.04",
        output_path: std::path::Path::new("Dockerfile"),
        force: false,
    };
    let out = generate_dockerfile(&config);
    assert!(out.contains("CMD [\"/bin/bash\"]"));
    assert!(!out.contains("ENTRYPOINT"));
}

#[test]
fn test_create_dockerfile_writes_file() {
    let dir = tempdir().unwrap();
    let out = dir.path().join("Dockerfile");
    let config = DockerfileConfig {
        target: "my-target",
        version: None,
        source: None,
        distro: "ubuntu:22.04",
        output_path: &out,
        force: false,
    };
    create_dockerfile(&config).unwrap();
    assert!(out.exists());
    let content = fs::read_to_string(&out).unwrap();
    assert!(content.starts_with("FROM ubuntu:22.04\n"));
    assert!(content.contains(
        "cim init --target my-target --full --no-sudo --yes --cert-validation=auto --symlink"
    ));
}

#[test]
fn test_create_dockerfile_no_overwrite_without_force() {
    let dir = tempdir().unwrap();
    let out = dir.path().join("Dockerfile");
    fs::write(&out, "original content").unwrap();

    let config = DockerfileConfig {
        target: "t",
        version: None,
        source: None,
        distro: "ubuntu:22.04",
        output_path: &out,
        force: false,
    };
    let result = create_dockerfile(&config);
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    // Original must be untouched
    assert_eq!(fs::read_to_string(&out).unwrap(), "original content");
}

#[test]
fn test_create_dockerfile_force_overwrites() {
    let dir = tempdir().unwrap();
    let out = dir.path().join("Dockerfile");
    fs::write(&out, "old content").unwrap();

    let config = DockerfileConfig {
        target: "t",
        version: None,
        source: None,
        distro: "alpine:3.19",
        output_path: &out,
        force: true,
    };
    create_dockerfile(&config).unwrap();
    let content = fs::read_to_string(&out).unwrap();
    assert!(content.starts_with("FROM alpine:3.19\n"));
    assert!(!content.contains("old content"));
}

#[test]
fn test_create_dockerfile_creates_parent_dirs() {
    let dir = tempdir().unwrap();
    let out = dir.path().join("nested").join("dir").join("Dockerfile");

    let config = DockerfileConfig {
        target: "t",
        version: None,
        source: None,
        distro: "ubuntu:22.04",
        output_path: &out,
        force: false,
    };
    create_dockerfile(&config).unwrap();
    assert!(out.exists());
}

#[test]
fn test_generate_dockerfile_package_manager_detection() {
    // The generated Dockerfile must handle multiple package managers via shell
    let config = DockerfileConfig {
        target: "t",
        version: None,
        source: None,
        distro: "ubuntu:22.04",
        output_path: std::path::Path::new("Dockerfile"),
        force: false,
    };
    let out = generate_dockerfile(&config);
    assert!(out.contains("apt-get"));
    assert!(out.contains("dnf"));
    assert!(out.contains("yum"));
    assert!(out.contains("apk"));
}
