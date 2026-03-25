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

mod cli;
mod init_cmd;
mod install_cmd;
mod makefile;
mod release_cmd;
mod update_cmd;
mod utils_cmd;
mod version;

use clap::{CommandFactory, Parser};
use cli::{Cli, Commands};
use dsdk_cli::messages;
use init_cmd::{
    handle_add_command, handle_docs_command, handle_foreach_command, handle_init_command,
    InitConfig,
};
use install_cmd::handle_install_command;
use makefile::handle_makefile_command;
use release_cmd::handle_release_command;
use update_cmd::{
    handle_config_command, handle_docker_command, handle_list_targets_command,
    handle_update_command, ConfigOptions,
};
use utils_cmd::handle_utils_command;
use version::{print_update_notice, print_version_info, spawn_version_check};

fn main() {
    let cli = Cli::parse();

    // Handle version flag first
    if cli.version {
        print_version_info();
        return;
    }

    // Command is now optional, so we need to handle the case where it's None
    let command = match &cli.command {
        Some(cmd) => cmd,
        None => {
            // Show help and check for updates when no command is provided
            Cli::command().print_help().unwrap();
            print_update_notice(spawn_version_check());
            return;
        }
    };

    match command {
        Commands::ListTargets { source, target } => {
            handle_list_targets_command(source.as_deref(), target.as_deref());
        }
        Commands::Init {
            target,
            source,
            version,
            workspace,
            no_mirror,
            force,
            r#match,
            verbose,
            install,
            full,
            no_sudo,
            symlink,
            yes,
            cert_validation,
        } => {
            // Validate that target is provided
            let target_name = match target {
                Some(t) => t.clone(),
                None => {
                    messages::error("--target is required");
                    messages::status("Use 'cim list-targets' to see available targets");
                    std::process::exit(1);
                }
            };

            handle_init_command(InitConfig {
                target: target_name,
                source: source.clone(),
                version: version.clone(),
                workspace: workspace.clone(),
                no_mirror: *no_mirror,
                force: *force,
                match_pattern: r#match.as_deref(),
                verbose: *verbose,
                install: *install,
                full: *full,
                no_sudo: *no_sudo,
                symlink: *symlink,
                yes: *yes,
                _cert_validation: cert_validation.as_deref(),
            });
        }
        Commands::Foreach { command, r#match } => {
            handle_foreach_command(command, r#match.as_deref());
        }
        Commands::Update {
            no_mirror,
            r#match,
            verbose,
            cert_validation,
        } => {
            handle_update_command(
                *no_mirror,
                r#match.as_deref(),
                *verbose,
                cert_validation.as_deref(),
            );
        }
        Commands::Makefile { no_dividers } => {
            handle_makefile_command(*no_dividers);
        }
        Commands::Add { name, url, commit } => {
            handle_add_command(name, url, commit);
        }
        Commands::Install { install_command } => {
            handle_install_command(install_command);
        }
        Commands::Docs { docs_command } => {
            handle_docs_command(docs_command);
        }
        Commands::Docker { docker_command } => {
            handle_docker_command(docker_command);
        }
        Commands::Release {
            tag,
            genconfig,
            include,
            exclude,
            dry_run,
        } => {
            handle_release_command(tag.as_deref(), *genconfig, include, exclude, *dry_run);
        }
        Commands::Config {
            list,
            get,
            show_path,
            template,
            create,
            force,
            edit,
            validate,
        } => {
            handle_config_command(ConfigOptions {
                list: *list,
                get: get.as_deref(),
                show_path: *show_path,
                template: *template,
                create: *create,
                force: *force,
                edit: *edit,
                validate: *validate,
            });
        }
        Commands::Utils { utils_command } => {
            handle_utils_command(utils_command);
        }
    }
}
