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

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// Flags shared verbatim by `init` and `bootstrap` for selecting/locating a
/// target and controlling how its workspace is cloned. Flattened into both
/// subcommands via `#[command(flatten)]` so the two can never drift apart --
/// adding or changing a flag here automatically applies to both.
#[derive(Args, Debug)]
pub struct WorkspaceSelectionArgs {
    /// Manifest source: git URL or local path
    ///
    /// Defaults to $HOME/devel/cim-manifests.
    #[arg(short, long, value_name = "URL|PATH")]
    pub source: Option<String>,
    /// Target version (branch or tag)
    ///
    /// When omitted, the target's default version is used.
    #[arg(short, long, value_name = "VERSION")]
    pub version: Option<String>,
    /// Workspace directory
    ///
    /// Defaults to $HOME/dsdk-<target>.
    #[arg(short, long, value_name = "DIR")]
    pub workspace: Option<PathBuf>,
    /// Skip the mirror and use remote URLs
    #[arg(short = 'N', long)]
    pub no_mirror: bool,
    /// Mirror cache directory
    ///
    /// Overrides the config file and the default location.
    #[arg(short, long, value_name = "DIR")]
    pub mirror: Option<PathBuf>,
    /// Only repositories matching this regex
    #[arg(short = 'p', long, value_name = "REGEX")]
    pub r#match: Option<String>,
    /// Only repositories in these groups
    ///
    /// Comma-separated group names.
    #[arg(short = 'g', long, value_name = "NAMES")]
    pub include_group: Option<String>,
    /// Skip repositories in these groups
    ///
    /// Comma-separated group names.
    #[arg(short = 'G', long, value_name = "NAMES")]
    pub exclude_group: Option<String>,
    /// Show detailed progress information
    #[arg(long)]
    pub verbose: bool,
    /// Skip all confirmation prompts
    #[arg(short, long)]
    pub yes: bool,
    /// Certificate validation mode
    ///
    /// One of: strict (default), relaxed (insecure), auto.
    #[arg(short = 'c', long, value_name = "MODE")]
    pub cert_validation: Option<String>,
}

#[derive(Parser)]
#[command(name = "cim")]
#[command(disable_version_flag = true)]
#[command(
    about = "Code in Motion - Multi-repository workspace manager",
    after_long_help = "WORKFLOW:\n  \
        1. List targets:         cim list-targets [-s DIR|URL]\n  \
        2. Initialize workspace: cim init -t <name> [-w DIR]\n  \
        3. Use from workspace:   cd DIR && cim <command>\n\n\
        CONFIG FILES:\n  \
        sdk.yml                  - Main configuration\n  \
        .workspace               - Workspace marker (auto-created)\n  \
        os-dependencies.yml      - System packages\n  \
        python-dependencies.yml  - Python packages"
)]
pub struct Cli {
    /// Show version
    #[arg(short = 'v', long = "version")]
    pub version: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List available targets and versions
    ListTargets {
        /// Manifest source: git URL or local path
        ///
        /// Defaults to $HOME/devel/cim-manifests.
        #[arg(short, long, value_name = "URL|PATH")]
        source: Option<String>,
        /// List versions for this target
        #[arg(short, long, value_name = "NAME")]
        target: Option<String>,
        /// Show credential attempts and git operations
        #[arg(long)]
        verbose: bool,
        /// Output format: text or json
        #[arg(short, long, value_name = "FORMAT", default_value = "text")]
        format: String,
    },
    /// Initialize a new workspace from a manifest
    Init {
        /// Target name or URL
        ///
        /// Looks in targets/<name>/sdk.yml or fetches from the URL.
        #[arg(short, long, value_name = "TARGET")]
        target: Option<String>,
        #[command(flatten)]
        common: WorkspaceSelectionArgs,
        /// Replace an existing workspace directory
        #[arg(short, long)]
        force: bool,
        /// Also install toolchains, pip packages and tools
        #[arg(short, long)]
        install: bool,
        /// Also install OS deps, toolchains, pip and tools
        #[arg(short = 'F', long)]
        full: bool,
        /// Do not use sudo for package installs
        ///
        /// For root users or special configurations.
        #[arg(short = 'S', long = "no-sudo")]
        no_sudo: bool,
        /// Symlink toolchains and pip venv from the mirror
        ///
        /// Requires a mirror.
        #[arg(short = 'l', long)]
        symlink: bool,
    },
    /// Pick a target, init a workspace and build it
    Bootstrap {
        /// Target name or URL
        ///
        /// Omit to pick interactively. Required when not running in a
        /// terminal, e.g. in CI.
        #[arg(short, long, value_name = "TARGET")]
        target: Option<String>,
        #[command(flatten)]
        common: WorkspaceSelectionArgs,
    },
    /// Update all git repositories
    Update {
        /// Skip the mirror and use remote URLs
        #[arg(short = 'N', long)]
        no_mirror: bool,
        /// Mirror cache directory
        ///
        /// Overrides the config file and the default location.
        #[arg(short, long, value_name = "DIR")]
        mirror: Option<PathBuf>,
        /// Only repositories matching this regex
        #[arg(short = 'p', long, value_name = "REGEX")]
        r#match: Option<String>,
        /// Only repositories in these groups
        ///
        /// Comma-separated group names.
        #[arg(short = 'g', long, value_name = "NAMES")]
        include_group: Option<String>,
        /// Skip repositories in these groups
        ///
        /// Comma-separated group names.
        #[arg(short = 'G', long, value_name = "NAMES")]
        exclude_group: Option<String>,
        /// Update all repositories
        ///
        /// Ignores the match filter stored by init.
        #[arg(short, long)]
        all: bool,
        /// Show detailed progress information
        #[arg(long)]
        verbose: bool,
        /// Certificate validation mode
        ///
        /// One of: strict (default), relaxed (insecure), auto.
        #[arg(short = 'c', long, value_name = "MODE")]
        cert_validation: Option<String>,
    },
    /// Execute a command in each repository
    Foreach {
        /// The command to execute in each repository
        command: String,
        /// Only execute command in repositories matching the given regex pattern
        #[arg(long, help = "Only run command in repositories matching this regex")]
        r#match: Option<String>,
        /// Only execute command in repositories belonging to the given group(s)
        #[arg(
            long,
            value_name = "NAMES",
            help = "Only run command in repositories in these comma-separated group(s)"
        )]
        include_group: Option<String>,
        /// Exclude repositories belonging to the given group(s)
        #[arg(
            long,
            value_name = "NAMES",
            help = "Exclude repositories in these comma-separated group(s)"
        )]
        exclude_group: Option<String>,
    },
    /// Generate a Makefile from configuration
    Makefile {
        /// Skip section dividers in the generated Makefile
        #[arg(long, help = "Do not add section dividers to the generated Makefile")]
        no_dividers: bool,
        /// Only include repositories belonging to the given group(s)
        #[arg(
            long,
            value_name = "NAMES",
            help = "Only include repositories in these comma-separated group(s)"
        )]
        include_group: Option<String>,
        /// Exclude repositories belonging to the given group(s)
        #[arg(
            long,
            value_name = "NAMES",
            help = "Exclude repositories in these comma-separated group(s)"
        )]
        exclude_group: Option<String>,
    },
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
        /// Target name (looks in targets/<name>/sdk.yml)
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
        /// Base Docker image (e.g. ubuntu:22.04, fedora:40) [default: ubuntu:22.04]
        #[arg(
            short,
            long,
            help = "Linux distribution for Docker image (e.g. ubuntu:22.04)"
        )]
        distro: Option<String>,
        /// Output Dockerfile path
        #[arg(
            short,
            long,
            default_value = "Dockerfile",
            help = "Output path for Dockerfile"
        )]
        output: PathBuf,
        /// Force overwrite existing Dockerfile
        #[arg(short, long, help = "Force overwrite existing Dockerfile")]
        force: bool,
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
        #[arg(short, long, help = "Reinstall by removing the existing venv")]
        force: bool,
        /// Install Python packages to mirror and create symlinks in workspace
        #[arg(long, help = "Use a shared venv in the mirror via a symlink")]
        symlink: bool,
        /// Python dependency profile to use
        #[arg(
            short,
            long,
            value_name = "PROFILE",
            help = "Profile to use (e.g. minimal, docs, dev, full)",
            long_help = "Profile to use (e.g. minimal, docs, dev, full). \
                    Supports comma-separated: --profile dev,docs"
        )]
        profile: Option<String>,
        /// List available dependency profiles
        #[arg(long, help = "List available dependency profiles")]
        list_profiles: bool,
        /// Only install per-repo Python deps for repositories in the given group(s)
        #[arg(
            long,
            value_name = "NAMES",
            help = "Only install deps for these group(s)",
            long_help = "Only install per-repo Python deps for repositories in \
                    these comma-separated group(s)"
        )]
        include_group: Option<String>,
        /// Exclude per-repo Python deps for repositories in the given group(s)
        #[arg(
            long,
            value_name = "NAMES",
            help = "Exclude deps for these group(s)",
            long_help = "Exclude per-repo Python deps for repositories in these \
                    comma-separated group(s)"
        )]
        exclude_group: Option<String>,
        /// Certificate validation mode for package downloads (strict, relaxed, auto)
        #[arg(
            long,
            value_name = "MODE",
            help = "Certificate validation mode",
            long_help = "Certificate validation: strict (default), relaxed \
                    (insecure), auto"
        )]
        cert_validation: Option<String>,
        /// Repair a broken shared mirror virtual environment in place
        #[arg(
            long,
            conflicts_with_all = [
                "force",
                "symlink",
                "profile",
                "list_profiles",
                "include_group",
                "exclude_group"
            ],
            help = "Repair the shared mirror venv in place",
            long_help = "Repair the shared mirror virtual environment (used by \
                    --symlink workspaces) instead of installing packages: tries \
                    a non-destructive fix first (e.g. a stale interpreter path \
                    after a system Python upgrade) and only rebuilds it from \
                    scratch as a last resort"
        )]
        repair: bool,
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
        /// Automatically add missing sha256 fields instead of skipping entries
        #[arg(
            long,
            help = "Add sha256 field to entries that are missing it, then compute and set the hash"
        )]
        add_missing: bool,
    },
    /// Compute and update SHA256 hashes for toolchain entries
    HashToolchains {
        /// Only compute hash for specific toolchain (by name or URL)
        #[arg(
            value_name = "TOOLCHAIN",
            help = "Specific toolchain to update hash for (optional)"
        )]
        file: Option<String>,
        /// Show what would be changed without modifying sdk.yml
        #[arg(long, help = "Show proposed changes without updating files")]
        dry_run: bool,
        /// Enable verbose output
        #[arg(short, long, help = "Show detailed progress information")]
        verbose: bool,
        /// Automatically add missing sha256 fields instead of skipping entries
        #[arg(
            long,
            help = "Add sha256 field to entries that are missing it, then compute and set the hash"
        )]
        add_missing: bool,
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
            Some(Commands::ListTargets {
                source,
                target,
                verbose,
                format,
            }) => {
                assert!(source.is_none());
                assert!(target.is_none());
                assert!(!verbose);
                assert_eq!(format, "text");
            }
            _ => panic!("Expected ListTargets command"),
        }

        // Test list-targets with source
        let list_args_with_source = vec!["cim", "list-targets", "--source", "/path/to/source"];
        let cli_result = Cli::try_parse_from(list_args_with_source);
        assert!(cli_result.is_ok());

        let cli = cli_result.unwrap();
        match &cli.command {
            Some(Commands::ListTargets {
                source,
                target,
                verbose,
                format,
            }) => {
                assert_eq!(source, &Some("/path/to/source".to_string()));
                assert!(target.is_none());
                assert!(!verbose);
                assert_eq!(format, "text");
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
                common,
                force: _,
                install: _,
                full: _,
                no_sudo: _,
                symlink: _,
            }) => {
                assert_eq!(target, &Some("my-target".to_string()));
                assert_eq!(
                    common.source,
                    Some("https://github.com/user/repo.git".to_string())
                );
                assert_eq!(common.version, Some("v1.0.0".to_string()));
            }
            _ => panic!("Expected Init command"),
        }

        // Test that --list-targets option is no longer available in init command
        let invalid_args = vec!["cim", "init", "--list-targets"];
        let cli_result = Cli::try_parse_from(invalid_args);
        assert!(cli_result.is_err()); // Should fail because --list-targets is removed from init
    }

    #[test]
    fn test_bootstrap_cli_structure() {
        use clap::Parser;

        // No flags: interactive picker path (target/version/source all None)
        let cli_result = Cli::try_parse_from(vec!["cim", "bootstrap"]);
        assert!(cli_result.is_ok());
        let cli = cli_result.unwrap();
        match &cli.command {
            Some(Commands::Bootstrap { target, common }) => {
                assert!(target.is_none());
                assert!(common.source.is_none());
                assert!(common.version.is_none());
            }
            _ => panic!("Expected Bootstrap command"),
        }

        // Explicit target/version/source: fully non-interactive/CI-friendly path
        let cli_result = Cli::try_parse_from(vec![
            "cim",
            "bootstrap",
            "--target",
            "my-target",
            "--source",
            "https://github.com/user/repo.git",
            "--version",
            "v1.0.0",
            "--yes",
        ]);
        assert!(cli_result.is_ok());
        let cli = cli_result.unwrap();
        match &cli.command {
            Some(Commands::Bootstrap { target, common }) => {
                assert_eq!(target, &Some("my-target".to_string()));
                assert_eq!(
                    common.source,
                    Some("https://github.com/user/repo.git".to_string())
                );
                assert_eq!(common.version, Some("v1.0.0".to_string()));
                assert!(common.yes);
            }
            _ => panic!("Expected Bootstrap command"),
        }
    }
}
