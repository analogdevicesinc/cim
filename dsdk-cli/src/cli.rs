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

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cim")]
#[command(disable_version_flag = true)]
#[command(
    about = "Code in Motion - Multi-repository workspace manager\n\nWORKFLOW:\n  1. List targets:         cim list-targets [--source DIR|URL]\n  2. Initialize workspace: cim init --target <name> [--source DIR|URL] [-w /path/to/workspace]\n  3. Use from workspace:   cd /path/to/workspace && cim <command>\n\nCONFIG FILES:\n  sdk.yml                   - Main configuration\n  .workspace                - Workspace marker (auto-created)\n  os-dependencies.yml       - System packages\n  python-dependencies.yml   - Python packages"
)]
pub struct Cli {
    /// Show version information
    #[arg(short = 'v', long = "version")]
    pub version: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List available targets and versions
    ListTargets {
        /// Source location (git repository URL or local path, default: $HOME/devel/cim-manifests)
        #[arg(
            short,
            long,
            value_name = "URL|PATH",
            help = "Git repository URL or local path to manifests"
        )]
        source: Option<String>,
        /// List versions for specific target
        #[arg(
            short,
            long,
            value_name = "TARGET_NAME",
            help = "Show available versions for this target"
        )]
        target: Option<String>,
    },
    /// Initialize a new workspace from a configuration file
    Init {
        /// Target name or URL (looks in targets/<name>/sdk.yml or fetches from URL)
        #[arg(
            short,
            long,
            value_name = "TARGET",
            help = "Target name of the project"
        )]
        target: Option<String>,
        /// Source location (git repository URL or local path, default: $HOME/devel/cim-manifests)
        #[arg(
            short,
            long,
            value_name = "URL|PATH",
            help = "Git repository URL or local path to manifests"
        )]
        source: Option<String>,
        /// Target version (branch or tag name)
        #[arg(
            short,
            long,
            value_name = "VERSION",
            help = "Target version (branch/tag name)"
        )]
        version: Option<String>,
        /// Workspace directory path (default: $HOME/dsdk-workspace)
        #[arg(
            short,
            long,
            value_name = "DIR",
            help = "Directory where to create the workspace"
        )]
        workspace: Option<PathBuf>,
        /// Skip mirror operations and clone directly from remote URLs
        #[arg(long, help = "Skip mirror, clone directly from remote repos")]
        no_mirror: bool,
        /// Force initialization by removing existing workspace directory
        #[arg(long, help = "Force workspace creation (removes existing")]
        force: bool,
        /// Only initialize repositories matching the given regex pattern
        #[arg(long, help = "Only clone repositories matching this regex pattern")]
        r#match: Option<String>,
        /// Enable verbose output
        #[arg(long, help = "Show detailed progress information")]
        verbose: bool,
        /// Install toolchains, pip packages, and install targets after workspace initialization
        #[arg(long, help = "Install toolchains, pip packages, and install targets")]
        install: bool,
        /// Complete SDK setup including OS dependencies, toolchains, pip, and install targets
        #[arg(
            long,
            help = "Install OS dependencies, toolchains, pip packages, and install targets"
        )]
        full: bool,
        /// Skip using sudo for package installation (for root users or special configurations)
        #[arg(
            long = "no-sudo",
            help = "Skip using sudo when running package manager commands"
        )]
        no_sudo: bool,
        /// Use symlinks for toolchains and pip packages (requires mirror)
        #[arg(
            long,
            help = "Install toolchains and pip to mirror with symlinks in workspace"
        )]
        symlink: bool,
        /// Skip all confirmation prompts
        #[arg(short = 'y', long = "yes", help = "Skip all confirmation prompts")]
        yes: bool,
        /// Certificate validation mode for downloads (strict, relaxed, auto)
        #[arg(
            long,
            value_name = "MODE",
            help = "Certificate validation: strict (default), relaxed (insecure), auto"
        )]
        cert_validation: Option<String>,
    },
    /// Update all git repositories
    Update {
        /// Skip mirror operations and clone directly from remote URLs
        #[arg(long, help = "Skip mirror, only update workspace from remote URLs")]
        no_mirror: bool,
        /// Only update repositories matching the given regex pattern
        #[arg(long, help = "Only update repositories matching this regex pattern")]
        r#match: Option<String>,
        /// Enable verbose output
        #[arg(short, long, help = "Show detailed progress information")]
        verbose: bool,
        /// Certificate validation mode for downloads (strict, relaxed, auto)
        #[arg(
            long,
            value_name = "MODE",
            help = "Certificate validation: strict (default), relaxed (insecure), auto"
        )]
        cert_validation: Option<String>,
    },
    /// Execute a command in each repository
    Foreach {
        /// The command to execute in each repository
        command: String,
        /// Only execute command in repositories matching the given regex pattern
        #[arg(long, help = "Only run command in repositories matching this regex")]
        r#match: Option<String>,
    },
    /// Generate a Makefile from configuration
    Makefile,
    /// Add a new git repository to configuration
    Add {
        /// Name of the repository
        #[arg(short, long, help = "Name of the repository")]
        name: String,
        /// URL of the repository
        #[arg(short, long, help = "URL of the repository")]
        url: String,
        /// Commit or tag to checkout
        #[arg(long, help = "Commit or tag to checkout")]
        commit: String,
    },
    /// Install system dependencies, Python packages, or toolchains
    Install {
        #[command(subcommand)]
        install_command: InstallCommand,
    },
    /// Create and manage unified documentation
    Docs {
        #[command(subcommand)]
        docs_command: DocsCommand,
    },
    /// Generate Docker configurations for development
    Docker {
        #[command(subcommand)]
        docker_command: DockerCommand,
    },
    /// Create release tags and configuration
    Release {
        /// Release tag to apply (e.g., v1.0.0)
        #[arg(short, long, help = "Release tag to apply to repositories")]
        tag: Option<String>,
        /// Generate a release configuration file
        #[arg(long, help = "Generate release configuration file")]
        genconfig: bool,
        /// Include only repositories matching the given regex patterns
        #[arg(
            long,
            help = "Include repositories matching regex patterns.\n\
                           Supports comma-separated values: --include 'adi.*,core.*'\n\
                           Or multiple flags: --include adi.* --include core.*"
        )]
        include: Vec<String>,
        /// Exclude repositories matching the given regex patterns
        #[arg(
            long,
            help = "Exclude repositories matching regex patterns.\n\
                           Supports comma-separated values: --exclude 'drivers,CMSIS_6'\n\
                           Or multiple flags: --exclude drivers --exclude CMSIS_6"
        )]
        exclude: Vec<String>,
        /// Show what would be done without executing
        #[arg(long, help = "Show what would be done without making any changes")]
        dry_run: bool,
    },

    /// Manage user configuration
    Config {
        /// List all configuration values
        #[arg(short = 'l', long = "list")]
        list: bool,

        /// Get specific configuration value
        #[arg(short = 'g', long = "get", value_name = "KEY")]
        get: Option<String>,

        /// Show config file location
        #[arg(short = 'p', long = "path")]
        show_path: bool,

        /// Print config template to stdout
        #[arg(short = 't', long = "template")]
        template: bool,

        /// Create config file from template
        #[arg(short = 'c', long = "create")]
        create: bool,

        /// Force overwrite when creating config
        #[arg(short = 'f', long = "force")]
        force: bool,

        /// Open config in editor (creates from template if missing)
        #[arg(short = 'e', long = "edit")]
        edit: bool,

        /// Validate config file syntax
        #[arg(short = 'v', long = "validate")]
        validate: bool,
    },

    /// Utility commands for workspace maintenance
    Utils {
        #[command(subcommand)]
        utils_command: UtilsCommand,
    },
}

#[derive(Subcommand)]
pub enum DocsCommand {
    /// Create unified documentation by aggregating all repository docs
    Create {
        /// Force recreation of documentation even if it exists
        #[arg(short, long, help = "Force recreation of existing documentation")]
        force: bool,
        /// Theme to use for documentation
        #[arg(long, default_value = "sphinx_rtd_theme", help = "Sphinx theme to use")]
        theme: String,
        /// Use symbolic links instead of copying files (Linux/macOS only)
        #[arg(
            long,
            help = "Create symbolic links instead of copying documentation files"
        )]
        symlink: bool,
        /// Enable verbose output
        #[arg(long, help = "Show detailed search information")]
        verbose: bool,
        /// Certificate validation mode for downloads (strict, relaxed, auto)
        #[arg(
            long,
            value_name = "MODE",
            help = "Certificate validation: strict (default), relaxed (insecure), auto"
        )]
        cert_validation: Option<String>,
    },
    /// Build the unified documentation
    Build {
        /// Output format (html, pdf, epub)
        #[arg(short, long, default_value = "html", help = "Output format")]
        format: String,
    },
    /// Serve documentation locally with live reload
    Serve {
        /// Port to serve on
        #[arg(
            short,
            long,
            default_value = "8000",
            help = "Port to serve documentation"
        )]
        port: u16,
        /// Host to bind to
        #[arg(long, default_value = "localhost", help = "Host to bind to")]
        host: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum DockerCommand {
    /// Create a Dockerfile for SDK development
    Create {
        /// Target name (looks in configs/targets/<name>/sdk.yml)
        #[arg(
            short,
            long,
            value_name = "TARGET",
            help = "Target name of the project"
        )]
        target: String,
        /// Source location (git repository URL or local path, default: $HOME/devel/cim-manifests)
        #[arg(
            short,
            long,
            value_name = "URL|PATH",
            help = "Git repository URL or local path to manifests"
        )]
        source: Option<String>,
        /// Target version (branch or tag name)
        #[arg(
            short,
            long,
            value_name = "VERSION",
            help = "Target version (branch/tag name)"
        )]
        version: Option<String>,
        /// Target Linux distribution (e.g., ubuntu:22.04, fedora:42)
        #[arg(short, long, help = "Linux distribution for Docker image")]
        distro: Option<String>,
        /// Python dependency profile to use
        #[arg(
            short,
            long,
            default_value = "docs",
            help = "Python profile from python-dependencies.yml"
        )]
        profile: String,
        /// Target architecture for cross-compilation
        #[arg(
            short,
            long,
            default_value = "aarch64-unknown-linux-gnu",
            help = "Target architecture"
        )]
        arch: String,
        /// Output Dockerfile path
        #[arg(
            short,
            long,
            default_value = "Dockerfile",
            help = "Output path for Dockerfile"
        )]
        output: PathBuf,
        /// Force overwrite existing Dockerfile
        #[arg(short, long, help = "Force overwrite existing files")]
        force: bool,
        /// Force all git URLs to use HTTPS protocol for corporate proxy compatibility
        #[arg(long, help = "Convert git URLs to HTTPS")]
        force_https: bool,
        /// Force all git URLs to use SSH protocol for key-based authentication
        #[arg(long, help = "Convert git URLs to SSH")]
        force_ssh: bool,
        /// Skip mirror operations in the generated Docker environment
        #[arg(long, help = "Skip mirror operations in Docker")]
        no_mirror: bool,
        /// Only include repositories matching this regex pattern in the Dockerfile
        #[arg(short, long, help = "Filter repositories by regex pattern")]
        r#match: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum InstallCommand {
    /// Install OS system packages
    OsDeps {
        /// Skip confirmation prompt and install automatically
        #[arg(short = 'y', long = "yes", help = "Skip confirmation prompt")]
        yes: bool,
        /// Skip using sudo for package installation (for root users or special configurations)
        #[arg(
            long = "no-sudo",
            help = "Skip using sudo when running package manager commands"
        )]
        no_sudo: bool,
    },
    /// Install Python packages for documentation
    Pip {
        /// Force reinstallation by removing existing virtual environment
        #[arg(
            short,
            long,
            help = "Force reinstallation by removing existing virtual environment"
        )]
        force: bool,
        /// Install Python packages to mirror and create symlinks in workspace
        #[arg(
            long,
            help = "Install Python packages to mirror directory and create symlinks in workspace"
        )]
        symlink: bool,
        /// Python dependency profile to use
        #[arg(
            short,
            long,
            value_name = "PROFILE",
            help = "Profile to use (e.g., minimal, docs, dev, full)\n\
                    Supports comma-separated: --profile dev,docs"
        )]
        profile: Option<String>,
        /// List available dependency profiles
        #[arg(long, help = "List available dependency profiles")]
        list_profiles: bool,
    },
    /// Install and extract toolchains
    Toolchains {
        /// Force reinstallation by removing existing destination directories
        #[arg(
            short,
            long,
            help = "Force reinstallation by removing existing destination directories"
        )]
        force: bool,
        /// Install toolchains to mirror and create symlinks in workspace
        #[arg(
            long,
            help = "Install toolchains to mirror directory and create symlinks in workspace"
        )]
        symlink: bool,
        /// Enable verbose output
        #[arg(short, long, help = "Show detailed progress information")]
        verbose: bool,
        /// Certificate validation mode for downloads (strict, relaxed, auto)
        #[arg(
            long,
            value_name = "MODE",
            help = "Certificate validation: strict (default), relaxed (insecure), auto"
        )]
        cert_validation: Option<String>,
    },
    /// Install SDK components via install section targets (wraps Makefile)
    Tools {
        /// Component name to install
        #[arg(help = "Component name (e.g., ninja, ccache, zephyr-sdk)\n\
                    Note: This wraps 'make install-<name>' for convenience. You can also use make directly.")]
        name: Option<String>,
        /// List available install targets
        #[arg(long, help = "List available install targets")]
        list: bool,
        /// Install all components
        #[arg(long, help = "Install all components")]
        all: bool,
        /// Force reinstallation by removing sentinel file
        #[arg(
            short,
            long,
            help = "Force reinstallation by removing sentinel file before running"
        )]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum UtilsCommand {
    /// Compute and update SHA256 hashes for copy_files entries
    HashCopyFiles {
        /// Only compute hash for specific file (by dest path or source URL)
        #[arg(
            value_name = "FILE",
            help = "Specific file to update hash for (optional)"
        )]
        file: Option<String>,
        /// Show what would be changed without modifying sdk.yml
        #[arg(long, help = "Show proposed changes without updating files")]
        dry_run: bool,
        /// Enable verbose output
        #[arg(short, long, help = "Show detailed progress information")]
        verbose: bool,
    },
    /// Re-run copy_files operation to sync files to workspace
    SyncCopyFiles {
        /// Only sync specific file (by dest path or source URL)
        #[arg(value_name = "FILE", help = "Specific file to sync (optional)")]
        file: Option<String>,
        /// Show what would be copied without modifying workspace
        #[arg(long, help = "Show proposed changes without copying files")]
        dry_run: bool,
        /// Enable verbose output
        #[arg(short, long, help = "Show detailed progress information")]
        verbose: bool,
        /// Force overwrite existing files
        #[arg(short, long, help = "Overwrite existing files")]
        force: bool,
    },
    /// Update cim to the latest release from GitHub
    Update,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_new_cli_structure_compatibility() {
        // Test that the new CLI structure maintains expected functionality

        // Test list-targets command parsing
        let list_args = vec!["cim", "list-targets"];
        let cli_result = Cli::try_parse_from(list_args);
        assert!(cli_result.is_ok());

        let cli = cli_result.unwrap();
        match &cli.command {
            Some(Commands::ListTargets { source, target }) => {
                assert!(source.is_none());
                assert!(target.is_none());
            }
            _ => panic!("Expected ListTargets command"),
        }

        // Test list-targets with source
        let list_args_with_source = vec!["cim", "list-targets", "--source", "/path/to/source"];
        let cli_result = Cli::try_parse_from(list_args_with_source);
        assert!(cli_result.is_ok());

        let cli = cli_result.unwrap();
        match &cli.command {
            Some(Commands::ListTargets { source, target }) => {
                assert_eq!(source, &Some("/path/to/source".to_string()));
                assert!(target.is_none());
            }
            _ => panic!("Expected ListTargets command"),
        }
    }

    #[test]
    fn test_updated_init_cli_structure() {
        use clap::Parser;

        // Test init command with new structure
        let init_args = vec![
            "cim",
            "init",
            "--target",
            "my-target",
            "--source",
            "https://github.com/user/repo.git",
            "--version",
            "v1.0.0",
        ];
        let cli_result = Cli::try_parse_from(init_args);
        assert!(cli_result.is_ok());

        let cli = cli_result.unwrap();
        match &cli.command {
            Some(Commands::Init {
                target,
                source,
                version,
                workspace: _,
                no_mirror: _,
                force: _,
                r#match: _,
                verbose: _,
                install: _,
                full: _,
                no_sudo: _,
                symlink: _,
                yes: _,
                cert_validation: _,
            }) => {
                assert_eq!(target, &Some("my-target".to_string()));
                assert_eq!(
                    source,
                    &Some("https://github.com/user/repo.git".to_string())
                );
                assert_eq!(version, &Some("v1.0.0".to_string()));
            }
            _ => panic!("Expected Init command"),
        }

        // Test that --list-targets option is no longer available in init command
        let invalid_args = vec!["cim", "init", "--list-targets"];
        let cli_result = Cli::try_parse_from(invalid_args);
        assert!(cli_result.is_err()); // Should fail because --list-targets is removed from init
    }
}
