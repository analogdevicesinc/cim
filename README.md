# Code in Motion

Code in Motion, also known as `cim`, manages multi-repository SDK workspaces and makes it possible to bundle everything needed to setup, build and work with software projects of any size. With the concept of "manifests", it also allows to create dedicated manifest gits for all sorts of projects and purpose. A company can for example, put all their manifests at an internal only git, which allows them to have a single source and entrance point for all their SDKs and software projects. At the same time, they might have external facing manifests for customers and partners, which can be public or private.

Setting up and building an entire project takes just a handful of commands, typically around 5 lines in a shell. The tool standardizes build targets across projects (sdk-xyz commands), so teams don't need to learn different conventions for each software project. Still, advanced users can continue working with the underlying build systems directly if and when needed. The defaults just make the common case easy. `cim` minimizes duplication by letting you share toolchains, workspace components, and git mirrors across multiple projects. Since it is built as a CLI, it works with CI/CD systems like GitHub Actions out of the box. The goal is to deliver production-ready software projects either as standalone projects or in the form of SDKs, both that avoid the struggle with tooling.

## What Problems Does This Solve?

- **Automated setup**: Replace manual README instructions and copy-paste command workflows with a single init command
- **Repository synchronization**: Clone and update multiple git repos to exact commits specified in a manifest
- **Toolchain management**: Download and extract cross-platform toolchains with OS/architecture filtering
- **Reproducible builds**: Makes it possible to lock all dependencies to specific versions for consistent environments
- **Offline operation**: Mirror repos and artifacts locally, to save bandwidth, disc space and setup time
- **Workspace isolation**: Everything lives in the workspace folder except host OS dependencies and the shared mirror - no scattered files across your system. Deleting the workspace, deletes it all.

---

## Table of Contents

- [Installation](#installation)
- [Quick Start](#quick-start)
- [Advanced Usage](#advanced-usage)
- [Manifest](#manifest)
- [Examples](#examples)
- [Experimental Features](#experimental-features)

## Installation

### Requirements

**Required:**
- git 2.0+
- make
- tar and unzip
- python3 3.8+, venv and pip
- curl or wget

On Ubuntu for example, you can install those dependencies with:

```bash
sudo apt install -y git make tar unzip python3 python3-pip python3-venv curl wget
```

**Optional:**
- [Rust 1.56+](https://rust-lang.org/tools/install/) (to build from source)
- Docker (for containerized development)

### Download Precompiled Binary

`cim` is distributed as a single binary (no installer needed). Download the [latest release](https://github.com/analogdevicesinc/cim/releases) for your platform from GitHub and place it in a directory on your PATH.

```bash
tar -xzf cim-*.tar.gz
chmod 711 cim
cp cim $HOME/.local/bin/
# or
cp cim $HOME/bin/
# or
sudo cp cim /usr/local/bin/
# or to a PATH similar in Windows.
```

### Build from Source

```bash
git clone https://github.com/analogdevicesinc/cim.git
cd cim
cargo build --release
```

Binary location: `target/release/cim`

#### Install

```bash
cargo install --path dsdk-cli
```

Or copy to a directory in your PATH:

```bash
cp target/release/cim ~/.local/bin/
```

---

## Quick Start

Manifests define SDK targets and are stored locally (e.g., `~/devel/cim-manifests`) or fetched from git repositories.

### List Available Targets

```bash
cim list-targets
cim list-targets --source https://github.com/<path-to-a>/cim-manifests
cim list-targets --t optee-qemu-v8  # show versions
```

### Initialize a Workspace

```bash
cim init -t optee-qemu-v8
```

Creates workspace at `$HOME/dsdk-optee-qemu-v8`. Use `-w` or `--workspace` to specify a different location.

### Build and Test

```bash
cd ~/dsdk-optee-qemu-v8
cim makefile         # generate Makefile
make sdk-build       # build
make sdk-test        # test
```

---

## Advanced Usage

### Concepts

**Workspace**: A directory with sdk.yml, os-dependencies.yml, python-dependencies.yml, cloned git repos, and a .workspace marker file. Workspaces are isolated.

**Mirror**: Local cache at `$HOME/tmp/mirror` (configurable) for offline operation. Stores downloaded toolchains, files and repo mirrors. Possible to opt out of mirroring with `--no-mirror` or disable in user config.

**Target**: Predefined SDK configuration in a manifest repository under `targets/<name>/sdk.yml`.

### Common Commands

#### list-targets

Show available SDK targets from manifest repo.

```bash
cim list-targets [--source URL|PATH] [--target NAME]
```

#### init

Initialize workspace from target.

```bash
cim init --target NAME [--workspace PATH] [--version VERSION]
          [--match REGEX] [--install] [--full] [--symlink] [--no-mirror]
```

- `--install`: Install toolchains and pip packages after init
- `--full`: Complete setup including OS dependencies (requires sudo)
- `--symlink`: Install to mirror with symlinks in workspace
- `--match REGEX`: Only clone repos matching pattern
- `--no-mirror`: Disable mirroring for this workspace

#### update

Update git repos in workspace.

```bash
cim update [--match REGEX] [--no-mirror]
```

#### makefile

Generate Makefile from sdk.yml (run from workspace).

```bash
cim makefile
```

#### foreach

Run command in each repo.

```bash
cim foreach "COMMAND" [--match REGEX]
# Example: cim foreach "git status"
```

#### add

Add git repo to workspace config.

```bash
cim add --name NAME --url URL --commit COMMIT
```

#### install

**os-deps** - Install system packages from os-dependencies.yml

```bash
cim install os-deps [--yes] [--no-sudo] [--yes]
```

**pip** - Install Python packages from python-dependencies.yml

```bash
cim install pip [--profile PROFILE] [--symlink] [--force]
# Example: cim install pip --profile dev,docs
```

**toolchains** - Download and extract toolchains from sdk.yml

```bash
cim install toolchains [--symlink] [--force]
```

**tools** - Install SDK components via install section

```bash
cim install tools [NAME] [--all] [--list]
```

#### docs

**create** - Aggregate documentation from repos

```bash
cim docs create [--force] [--theme THEME] [--symlink]
```

**build** - Build documentation

```bash
cim docs build [--format html|pdf|epub]
```

**serve** - Serve docs locally

```bash
cim docs serve [--port PORT]
```

#### release

Create release tags.

```bash
cim release --tag TAG [--include PATTERNS] [--exclude PATTERNS] [--dry-run]
```

#### config

Manage user configuration.

```bash
cim config [--list] [--get KEY] [--create] [--edit] [--validate]
```

Config location: `~/.config/cim/config.toml` (Unix/Linux/macOS) or `%LOCALAPPDATA%\cim\config.toml` (Windows)

### Configuration File

`cim config -c` will create it for you. Here are a few example settings you can customize, for a complete list, generate the file and check the comments.

```toml
# Override default manifest source
default_source = "https://github.com/<a-path-to>/cim-manifests"

# Override mirror location
mirror_path = "/custom/mirror"

# Workspace naming prefix (default: "dsdk-")
workspace_prefix = "sdk-"

# Additional documentation directories
documentation_dirs = "wiki, manual, reference"

# Certificate validation: "strict" (default), "relaxed" (insecure), "auto"
cert_validation = "strict"
```

### Certificate Validation

By default, cim validates TLS certificates when downloading files. By default we use strict checking. If errors are seen related to certificates, then the less secure "relaxed" mode can be used as a workaround."

```bash
# Per-command override
cim install toolchains --cert-validation=relaxed  # INSECURE

# Or set in config.toml
cert_validation = "auto"  # try strict, fallback to relaxed with warning
```

Use `relaxed` mode only in trusted networks. It disables certificate validation and is vulnerable to MITM attacks.

---

## Manifest

The `sdk.yml` manifest defines repositories, build targets, toolchains, and
variables for an SDK target. The `os-dependencies.yml` defines the host OS
packages and the python-dependencies.yml defines the Python packages needed
(if any).

### Example

Note that this example, isn't a complete manifest, but rather a demonstration of the different sections and features.

#### sdk.yml
```yaml
################################################################################
# Mirror location serving as a local cache for downloads, git etc.
# Possible to opt-out of mirroring with --no-mirror or disable in user config.
################################################################################
mirror: $HOME/tmp/mirror


################################################################################
# Manifest variables — optional key-value pairs that can be reused across
# the manifest with ${{ VAR }} syntax.
#
# Values may reference host environment variables using the standard $VAR
# syntax; these are resolved when cim loads the manifest.
#
# Affected fields: git URLs, toolchain name/url/destination, and
# copy_files source/destination.
#
# Variables are also emitted in the generated Makefile as "VAR ?= value"
# (weak assignments), so host environment variables or command-line
# overrides ("make VAR=other") always take precedence.
#
# In build/test/clean and per-repo build commands, ${{ VAR }} is rewritten
# to $(VAR), so Make expands the variable at recipe time rather than at
# manifest load time.
#
# Unresolved ${{ VAR }} references are left unchanged and a warning is
# printed, making mistakes visible rather than silently ignored.
################################################################################
variables:
  SDK_BASE_URL: https://artifacts.example.com/sdk  # literal base URL
  SDK_ARCH: $HOST_ARCH                             # resolved from host env var


################################################################################
# Toolchains used in the workspace. Initiated by the "install toolchains"
# command. Uses "os" and "arch" to find the correct toolchain for the host
# system.
#
# post_install_commands: optional commands to run after toolchain installation
# environment: optional environment variables to set during post-install commands
#   - Supports variable expansion: $PWD (install dir), $WORKSPACE (workspace root), $HOME
#   - Useful for isolating toolchain installations from system-wide installations
################################################################################
toolchains:
  # Example: ARM GNU Toolchain for aarch32 (macOS ARM64)
  - name: arm-gnu-toolchain-14.3.rel1-darwin-arm64-arm-none-eabi.tar.xz
    url: https://developer.arm.com/-/media/Files/downloads/gnu/14.3.rel1/binrel/
    destination: toolchains/aarch32
    strip_components: 1
    os: darwin
    arch: arm64

  # Example: Rust toolchain with environment isolation
  # Downloads rustup installer script and runs it with isolated environment
  # The environment variables ensure Rust installs only in the workspace
  - name:  # Empty name - will be derived from URL as "sh.rustup.rs"
    url: https://sh.rustup.rs
    destination: toolchains/rust
    environment:
      # $PWD expands to the toolchain installation directory (toolchains/rust)
      CARGO_HOME: "$PWD/cargo"
      RUSTUP_HOME: "$PWD/rustup"
      # Prepend cargo bin to PATH (uses existing $PATH from environment)
      PATH: "$PWD/cargo/bin:$PATH"
    post_install_commands:
      - "mkdir -p cargo rustup"
      - "bash ./sh.rustup.rs -y --no-modify-path"
      - "rustup toolchain install nightly-2025-01-01"
      - "rustup default nightly-2025-01-01"
      - "echo 'Rust installation complete in $PWD'"


################################################################################
# Build related commands, these will end up in the Makefile generated
# by "cim makefile". Although there is no limit on the amount of commands here,
# it's recommended to keep it minimal and simple, and put more complex logic in
# a dedicated "build.git" or similar. This is mostly meant to initiate and
# redirect build commands to the different build systems used in the workspace.
#
# In the example below, we envision that a there exist a "build" folder, either
# as a directory or as a "build.git" repository, which contains the actual
# build logic and Makefiles for the current project.
# Note that there is no requirement on using "make" and tradtional Makefiles
# there. You can use any build system you want, as long as you redirect to it
# from the high level makefile targets defined here.
################################################################################
# Environment setup commands, these will typically run before the build and
# test commands.
# ${{ VAR }} references in commands become $(VAR) in the generated Makefile,
# so Make expands them at recipe time and they remain overridable.
envsetup:
  - ln -sf qemu_v8.mk build/Makefile
  - echo "Building for arch: ${{ SDK_ARCH }}"

# Build commands
build:
  - $(MAKE) -C build all $(MAKEFLAGS)

# Test commands
test:
  - $(MAKE) -C build check $(MAKEFLAGS)

# Clean commands
clean:
  - $(MAKE) -C build clean $(MAKEFLAGS)

# Flash/deploy commands
flash:
  - @echo "This is a QEMU setup, no flashing needed"


################################################################################
# Copy files section - download files to be used during installation
# Support for local files and remote URLs. Support wildcards and directories
# for local sources.
#
# cache: true - will store in mirrors
# symlink: true - will create symlink in workspace instead of copying
# sha256: optional checksum for integrity verification. If the checksum does not
#         match, the file will be re-downloaded (threshold of 3 attempts before
#         an error is reporterd)
################################################################################
copy_files:
  - source: extra.mk
    dest: extra.mk

  - source: https://my-remote-server.com/foobar.zip
    dest: downloads/foobar.zip
    cache: true
    symlink: true
    sha256: 65d1191f755c92d6b7792b1d054cbd3aa6762bb2b0788dedbcaa929497927c98


################################################################################
# Triggered by the "install tools" command. Can be used to setup and install
# arbitrary tools, files and binaries.
#
# sentinel: is an optional file that is created after successful installation.
#           If the sentinel file exists, the installation step will be skipped
#           on subsequent runs.
################################################################################
install:
  - name: protoc
    commands: |
      @mkdir -p opt/protoc bin
      @cd opt/protoc && unzip -q -o ../../downloads/protoc-21.7-linux-x86_64.zip
      @ln -sf ../opt/protoc/bin/protoc bin/protoc
    sentinel: .sdk/protoc.installed


################################################################################
# Git repositories to clone.
#
# name: encourge to use a single name to keep the git in the workspace root, but
# it can be nested if needed.
# build: will generate a Makefile target for this git repository.
# build_depends_on: ensures the dependent repository is built before this one
#             in the generated Makefile (also accepts legacy name "depends_on").
#             It's possible to depend on multiple targets.
# git_depends_on: controls clone ordering. Repositories listed here will be
#             cloned before this one. Useful for nested repos where a child
#             path lives inside a parent repo's directory tree.
# commit: can be a branch, tag, or specific commit hash.
################################################################################
gits:
  - name: build
    url: https://github.com/OP-TEE/build.git
    commit: master
    build:
      - make -C build -j`nproc` $(MAKEFLAGS)

  - name: optee_os
    url: https://github.com/OP-TEE/optee_os.git
    commit: master
    build_depends_on:
      - build

  - name: optee_client
    url: https://github.com/OP-TEE/optee_client.git
    commit: master
    build_depends_on:
      - optee_os

  # Example: nested repositories - the parent must be cloned first so that
  # children can be placed inside its directory tree.
  # The URLs here use ${{ SDK_BASE_URL }} defined in the variables section
  # above. The generated Makefile will expose SDK_BASE_URL as a ?= variable,
  # so callers can override it without editing the manifest.
  - name: platform
    url: ${{ SDK_BASE_URL }}/platform.git
    commit: main

  - name: platform/drivers
    url: ${{ SDK_BASE_URL }}/drivers.git
    commit: main
    git_depends_on:
      - platform

  - name: platform/libs
    url: ${{ SDK_BASE_URL }}/libs.git
    commit: main
    git_depends_on:
      - platform
```

#### os-dependencies.yml
Here we define the host OS dependencies for different OS'es. If the distros use the same package names, we can use YAML anchors to avoid duplication (as seen in the example below). This is the source when running `cim install os-deps`. Here `cim` will detect the host OS and distro and then run the corresponding command with the corresponding packages. Note that `cim` will ask for sudo permissions to install the packages on Linux systems. There are also commands to opt out from the sudo requirement (`--no-sudo`) and to run the installation with `--yes` to avoid the interactive confirmation, something that can be useful in CI environments.

`command:` is the command to install packages for that particular OS/distro. For example, `apt install` for Ubuntu and `dnf install` for Fedora, `brew install` for macOS, `winget` for Windows etc.

`packages:` is the list of packages to install for that particular OS/distro.

```yaml
# Common package lists using YAML anchors for DRY
ubuntu_packages: &ubuntu_pkgs
  - build-essential
  - curl
  - git
  - ninja-build
  - python3
  - vim

fedora_packages: &fedora_pkgs
  - ccache
  - cmake
  - curl
  - gcc
  - gcc-c++
  - git
  - boost-devel
  - python3
  - vim

# Linux x86_64 (Intel/AMD) dependencies
linux-x86_64:
  ubuntu-22.04:
    command: "apt-get install"
    packages: *ubuntu_pkgs

  fedora-42:
    command: "dnf install"
    packages: *fedora_pkgs

# Linux ARM64 (Apple Silicon, ARM servers) dependencies
linux-aarch64:
  ubuntu-22.04:
    command: "apt-get install"
    packages: *ubuntu_pkgs

  fedora-42:
    command: "dnf install"
    packages: *fedora_pkgs

# Backward compatibility - generic linux (defaults to x86_64 behavior)
linux:
  ubuntu-22.04:
    command: "apt-get install"
    packages: *ubuntu_pkgs

  fedora-42:
    command: "dnf install"
    packages: *fedora_pkgs

macos:
  macos-any:
    command: "brew install"
    packages:
      - autoconf
      - automake
      - ccache
      - cmake
      - gcc
      - git
      - python@3.14
```

#### python-dependencies.yml
Here we define the Python dependencies and packages needed for the project. Everything defined in here will end up in the `<workspace>/.venv` folder after running `cim install pip`. As can be seen in the example below, we can define different profiles for different purposes. If you don't specify a profile when running the install command, the `default` profile will be used. You can also specify multiple profiles at the same time using the `-p` or `--profile` option.

```yaml
profiles:
  # Minimal profile - no additional packages
  minimal:
    packages: []
    
  # Documentation profile - packages for building and serving docs
  docs:
    packages:
      - sphinx
      - sphinx-rtd-theme
      - myst-parser
      - sphinx-autobuild
      
  # Development profile - docs + development tools
  dev:
    packages:
      - sphinx
      - sphinx-rtd-theme
      - myst-parser
      - sphinx-autobuild
      - pytest
      - black
      - flake8
      
  # Full profile - everything
  full:
    packages:
      - sphinx
      - sphinx-rtd-theme
      - myst-parser
      - sphinx-autobuild
      - pytest
      - black
      - flake8
      - mypy
      - pre-commit

# Default profile to use when none specified
default: docs
```

---

## Examples

### Basic Workflow

```bash
# List available targets
cim list-targets

# Initialize workspace
cim init --target optee-qemu-v8 --workspace ~/optee

# Install dependencies and toolchains
cd ~/optee
cim install os-deps --yes
cim install toolchains
cim install pip

# Generate Makefile and build
cim makefile
make sdk-build
make sdk-test
```

The same example, but using `--install` which runs the individual install automatically after setting up the workspace.

```bash
# List available targets
cim list-targets

# Initialize workspace
cim init --target optee-qemu-v8 --workspace ~/optee --install

# Install dependencies and toolchains
cd ~/optee
make sdk-test
```

### Custom Manifest

```bash
# Create manifest structure
mkdir -p my-manifests/targets/my-sdk

# Create sdk.yml
cat > my-manifests/targets/my-sdk/sdk.yml << 'EOF'
mirror: $HOME/tmp/mirror

build:
  - make -j$(nproc)

gits:
  - name: myproject
    url: https://github.com/myorg/myproject.git
    commit: main
EOF

# Initialize from custom manifest
cim init --target my-sdk --source ./my-manifests
```

---

## Experimental Features

### Docker

The `docker create` command generates Dockerfiles for containerized SDK development. The feature is experimental and command options may change in future versions. Since it need cross compiled `cim` binaries for the target, this command can and should only be used from the source code folder of `cim` itself.

```bash
# Generate Dockerfile
cim docker create --target optee-qemu-v8 --distro ubuntu:22.04

# Build and run
docker build -t sdk-dev .
docker run -it sdk-dev bash
```

Options:
- `--distro`: Linux distribution (e.g., ubuntu:22.04, fedora:42)
- `--profile`: Python profile for documentation tools
- `--force-https`: Convert git URLs to HTTPS (useful for corporate proxies)
- `--match`: Filter repositories by regex pattern

---

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md)
before submitting a pull request. All commits must be signed off
(`git commit -s`) per the
[Developer Certificate of Origin](DCO).

## License

This project is licensed under the Apache License 2.0. See the
[LICENSE](LICENSE) file for details.
