#!/bin/bash
# Copyright (c) 2026 Analog Devices, Inc.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# Bash completion for cim command
#
# Supports all commands and options:
#   - list-targets: --source (-s), --target (-t)
#   - init: --target (-t), --source (-s), --version (-v), --workspace (-w),
#           --no-mirror, --force, --match, --verbose, --install, --full, --symlink,
#           --yes (-y), --cert-validation
#   - update: --no-mirror, --match, --verbose (-v), --cert-validation
#   - foreach: command, --match
#   - makefile: --no-dividers
#   - add: --name (-n), --url (-u), --commit
#   - install os-deps: --yes (-y), --no-sudo
#   - install pip: --profile (-p), --force (-f), --symlink, --list-profiles
#   - install toolchains: --force (-f), --symlink, --verbose (-v), --cert-validation
#   - install tools: name, --list, --all, --force (-f)
#   - docs create: --force (-f), --theme, --symlink, --verbose, --cert-validation
#   - docs build: --format (-f)
#   - docs serve: --port (-p), --host
#   - docker create: --target (-t), --source (-s), --version (-v), --distro (-d),
#                    --profile (-p), --arch (-a), --output (-o), --force (-f),
#                    --force-https, --force-ssh, --no-mirror, --match (-m)
#   - release: --tag (-t), --genconfig, --include, --exclude, --dry-run
#   - config: --list (-l), --get (-g), --path (-p), --template (-t), --create (-c),
#             --force (-f), --edit (-e), --validate (-v)
#   - utils hash-copy-files: --dry-run, --verbose (-v), --add-missing
#   - utils hash-toolchains: --dry-run, --verbose (-v), --add-missing
#   - utils sync-copy-files: --dry-run, --verbose (-v), --force (-f)
#   - utils update: (no options)

_cim_completions() {
    local cur prev opts subcommands
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"

    # Main commands
    local main_commands="list-targets init update foreach makefile add install docs docker release config utils help"

    # Global options
    local global_opts="--help --version -v"

    # Function to complete file paths
    _complete_file_path() {
        COMPREPLY=( $(compgen -f "${cur}") )
    }

    # Function to complete directory paths
    _complete_dir_path() {
        COMPREPLY=( $(compgen -d "${cur}") )
    }

    # Function to complete config files (*.yml, *.yaml)
    _complete_config_files() {
        COMPREPLY=( $(compgen -G "${cur}*.yml" "${cur}*.yaml" 2>/dev/null) )
    }

    # Function to complete target names using CLI
    _complete_targets() {
        local source=""
        
        # Check if we have a custom --source
        for ((i=1; i<COMP_CWORD; i++)); do
            if [[ "${COMP_WORDS[i]}" == "--source" ]] && [ $((i+1)) -lt $COMP_CWORD ]; then
                source="${COMP_WORDS[i+1]}"
                break
            fi
        done
        
        # Use cim list-targets to get available targets
        local targets_cmd="cim list-targets"
        if [ -n "$source" ]; then
            targets_cmd="$targets_cmd --source \"$source\""
        fi
        
        local targets=$(eval "$targets_cmd" 2>/dev/null | grep "  - " | sed 's/  - //')
        if [ -n "$targets" ]; then
            COMPREPLY=( $(compgen -W "$targets" -- "${cur}") )
        fi
    }

    # Function to complete version names using CLI
    _complete_versions() {
        local source=""
        local target=""
        
        # Find source and target from command line
        for ((i=1; i<COMP_CWORD; i++)); do
            if [[ "${COMP_WORDS[i]}" == "--source" ]] && [ $((i+1)) -lt $COMP_CWORD ]; then
                source="${COMP_WORDS[i+1]}"
            elif [[ "${COMP_WORDS[i]}" == "--target" ]] && [ $((i+1)) -lt $COMP_CWORD ]; then
                target="${COMP_WORDS[i+1]}"
            fi
        done
        
        if [ -n "$target" ]; then
            local versions_cmd="cim list-targets --target \"$target\""
            if [ -n "$source" ]; then
                versions_cmd="$versions_cmd --source \"$source\""
            fi
            
            local versions=$(eval "$versions_cmd" 2>/dev/null | grep "  - " | sed 's/  - //')
            if [ -n "$versions" ]; then
                COMPREPLY=( $(compgen -W "$versions" -- "${cur}") )
            fi
        fi
    }

    # If we're completing the first argument (command)
    if [ $COMP_CWORD -eq 1 ]; then
        COMPREPLY=( $(compgen -W "${main_commands} ${global_opts}" -- "${cur}") )
        return 0
    fi

    # Handle completion based on the main command
    case "${COMP_WORDS[1]}" in
        list-targets)
            case "${prev}" in
                -s|--source)
                    # Complete directories and common URL prefixes
                    COMPREPLY=( $(compgen -d "${cur}") $(compgen -W "https://github.com/ http://localhost:" -- "${cur}") )
                    return 0
                    ;;
                -t|--target)
                    _complete_targets
                    return 0
                    ;;
                *)
                    COMPREPLY=( $(compgen -W "--source -s --target -t --help" -- "${cur}") )
                    return 0
                    ;;
            esac
            ;;
        
        init)
            case "${prev}" in
                -t|--target)
                    _complete_targets
                    return 0
                    ;;
                -s|--source)
                    # Complete directories and common URL prefixes
                    COMPREPLY=( $(compgen -d "${cur}") $(compgen -W "https://github.com/ http://localhost:" -- "${cur}") )
                    return 0
                    ;;
                -v|--version)
                    _complete_versions
                    return 0
                    ;;
                -w|--workspace)
                    _complete_dir_path
                    return 0
                    ;;
                --match)
                    # Common regex patterns
                    COMPREPLY=( $(compgen -W "\"optee.*\" \".*test.*\" \"build.*\"" -- "${cur}") )
                    return 0
                    ;;
                --cert-validation)
                    COMPREPLY=( $(compgen -W "strict relaxed auto" -- "${cur}") )
                    return 0
                    ;;
                *)
                    COMPREPLY=( $(compgen -W "--target -t --source -s --version -v --workspace -w --no-mirror --force --match --install --full --symlink --yes -y --verbose --cert-validation --help" -- "${cur}") )
                    return 0
                    ;;
            esac
            ;;

        update)
            case "${prev}" in
                --match)
                    # Common regex patterns
                    COMPREPLY=( $(compgen -W "\"optee.*\" \".*test.*\" \"build.*\"" -- "${cur}") )
                    return 0
                    ;;
                --cert-validation)
                    COMPREPLY=( $(compgen -W "strict relaxed auto" -- "${cur}") )
                    return 0
                    ;;
                *)
                    COMPREPLY=( $(compgen -W "--no-mirror --match --verbose -v --cert-validation --help" -- "${cur}") )
                    return 0
                    ;;
            esac
            ;;

        foreach)
            case "${prev}" in
                --match)
                    # Common regex patterns
                    COMPREPLY=( $(compgen -W "\"optee.*\" \".*test.*\" \"build.*\"" -- "${cur}") )
                    return 0
                    ;;
                *)
                    # Complete common commands for foreach
                    if [ $COMP_CWORD -eq 2 ]; then
                        local common_commands="\"git status\" \"git pull\" \"git log\" \"git diff\" \"make clean\" ls pwd"
                        COMPREPLY=( $(compgen -W "${common_commands} --match --help" -- "${cur}") )
                    else
                        COMPREPLY=( $(compgen -W "--match --help" -- "${cur}") )
                    fi
                    return 0
                    ;;
            esac
            ;;

        makefile)
            COMPREPLY=( $(compgen -W "--no-dividers --help" -- "${cur}") )
            return 0
            ;;

        add)
            case "${prev}" in
                -n|--name|-u|--url|--commit)
                    # These expect user input, no completion
                    return 0
                    ;;
                *)
                    COMPREPLY=( $(compgen -W "--name -n --url -u --commit --help" -- "${cur}") )
                    return 0
                    ;;
            esac
            ;;

        install)
            if [ $COMP_CWORD -eq 2 ]; then
                COMPREPLY=( $(compgen -W "os-deps pip toolchains tools --help" -- "${cur}") )
            elif [ "${COMP_WORDS[2]}" = "os-deps" ]; then
                case "${prev}" in
                    os-deps)
                        COMPREPLY=( $(compgen -W "--yes -y --no-sudo --help" -- "${cur}") )
                        ;;
                    *)
                        COMPREPLY=( $(compgen -W "--yes -y --no-sudo --help" -- "${cur}") )
                        ;;
                esac
            elif [ "${COMP_WORDS[2]}" = "pip" ]; then
                case "${prev}" in
                    -p|--profile)
                        # Complete with profile names
                        COMPREPLY=( $(compgen -W "minimal docs dev full" -- "${cur}") )
                        return 0
                        ;;
                    pip)
                        COMPREPLY=( $(compgen -W "--profile -p --force -f --symlink --list-profiles --help" -- "${cur}") )
                        ;;
                    *)
                        COMPREPLY=( $(compgen -W "--profile -p --force -f --symlink --list-profiles --help" -- "${cur}") )
                        ;;
                esac
            elif [ "${COMP_WORDS[2]}" = "toolchains" ]; then
                case "${prev}" in
                    toolchains)
                        COMPREPLY=( $(compgen -W "--force -f --symlink --verbose -v --cert-validation --help" -- "${cur}") )
                        ;;
                    --cert-validation)
                        COMPREPLY=( $(compgen -W "strict relaxed auto" -- "${cur}") )
                        ;;
                    *)
                        COMPREPLY=( $(compgen -W "--force -f --symlink --verbose -v --cert-validation --help" -- "${cur}") )
                        ;;
                esac
            elif [ "${COMP_WORDS[2]}" = "tools" ]; then
                case "${prev}" in
                    tools)
                        # Complete with flags and check for install targets in workspace
                        local install_targets=""

                        # Try to get install targets from current workspace if available
                        if [ -f "sdk.yml" ]; then
                            install_targets=$(grep -A1 "^  - name:" sdk.yml 2>/dev/null | grep "name:" | sed 's/.*name: //' | tr '\n' ' ')
                        fi

                        COMPREPLY=( $(compgen -W "--list --all ${install_targets}" -- "${cur}") )
                        ;;
                    *)
                        COMPREPLY=( $(compgen -W "--force -f --list --all --help" -- "${cur}") )
                        ;;
                esac
            else
                COMPREPLY=( $(compgen -W "--help" -- "${cur}") )
            fi
            return 0
            ;;

        docs)
            if [ $COMP_CWORD -eq 2 ]; then
                # Docs subcommands
                COMPREPLY=( $(compgen -W "create build serve --help" -- "${cur}") )
                return 0
            elif [ $COMP_CWORD -gt 2 ]; then
                # Handle docs subcommand completion
                case "${COMP_WORDS[2]}" in
                    create)
                        case "${prev}" in
                            --theme)
                                COMPREPLY=( $(compgen -W "sphinx_rtd_theme alabaster classic nature pyramid bootstrap" -- "${cur}") )
                                return 0
                                ;;
                            --cert-validation)
                                COMPREPLY=( $(compgen -W "strict relaxed auto" -- "${cur}") )
                                return 0
                                ;;
                            *)
                                COMPREPLY=( $(compgen -W "--force -f --theme --symlink --verbose --cert-validation --help" -- "${cur}") )
                                return 0
                                ;;
                        esac
                        ;;
                    build)
                        case "${prev}" in
                            -f|--format)
                                COMPREPLY=( $(compgen -W "html pdf epub" -- "${cur}") )
                                return 0
                                ;;
                            *)
                                COMPREPLY=( $(compgen -W "--format -f --help" -- "${cur}") )
                                return 0
                                ;;
                        esac
                        ;;
                    serve)
                        case "${prev}" in
                            -p|--port|--host)
                                # No completion for port/host values
                                return 0
                                ;;
                            *)
                                COMPREPLY=( $(compgen -W "--port -p --host --help" -- "${cur}") )
                                return 0
                                ;;
                        esac
                        ;;
                esac
            fi
            ;;

        docker)
            if [ $COMP_CWORD -eq 2 ]; then
                # Docker subcommands
                COMPREPLY=( $(compgen -W "create --help" -- "${cur}") )
                return 0
            elif [ $COMP_CWORD -gt 2 ]; then
                # Handle docker subcommand completion
                case "${COMP_WORDS[2]}" in
                    create)
                        case "${prev}" in
                            -t|--target)
                                _complete_targets
                                return 0
                                ;;
                            -s|--source)
                                # Complete directories and common URL prefixes
                                COMPREPLY=( $(compgen -d "${cur}") $(compgen -W "https://github.com/ http://localhost:" -- "${cur}") )
                                return 0
                                ;;
                            -v|--version)
                                _complete_versions
                                return 0
                                ;;
                            -d|--distro)
                                COMPREPLY=( $(compgen -W "ubuntu:20.04 ubuntu:22.04 ubuntu:24.04 fedora:39 fedora:40 fedora:41 fedora:42 debian:11 debian:12 centos:7 centos:8" -- "${cur}") )
                                return 0
                                ;;
                            -p|--profile)
                                COMPREPLY=( $(compgen -W "minimal docs dev full" -- "${cur}") )
                                return 0
                                ;;
                            -a|--arch)
                                COMPREPLY=( $(compgen -W "aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu x86_64-pc-windows-gnu" -- "${cur}") )
                                return 0
                                ;;
                            -o|--output)
                                _complete_file_path
                                return 0
                                ;;
                            -m|--match)
                                # Common regex patterns
                                COMPREPLY=( $(compgen -W "\"optee.*\" \".*test.*\" \"build.*\"" -- "${cur}") )
                                return 0
                                ;;
                            *)
                                COMPREPLY=( $(compgen -W "--target -t --source -s --version -v --distro -d --profile -p --arch -a --output -o --force -f --force-https --force-ssh --no-mirror --match -m --help" -- "${cur}") )
                                return 0
                                ;;
                        esac
                        ;;
                esac
            fi
            ;;

        release)
            case "${prev}" in
                -t|--tag)
                    # Tag values are user-defined, no completion
                    return 0
                    ;;
                --include|--exclude)
                    # Common regex patterns for include/exclude
                    COMPREPLY=( $(compgen -W "\"optee.*\" \"test.*\" \".*-test\" \".*_test\" \"adi.*\" \"core.*\"" -- "${cur}") )
                    return 0
                    ;;
                *)
                    COMPREPLY=( $(compgen -W "--tag -t --genconfig --include --exclude --dry-run --help" -- "${cur}") )
                    return 0
                    ;;
            esac
            ;;

        config)
            case "${prev}" in
                --get|-g)
                    # Complete config key names
                    COMPREPLY=( $(compgen -W "default_source docker_temp_dir cert_validation default_workspace workspace_prefix mirror_path documentation_dirs" -- "${cur}") )
                    return 0
                    ;;
                *)
                    COMPREPLY=( $(compgen -W "--list -l --get -g --path -p --template -t --create -c --force -f --edit -e --validate -v --help" -- "${cur}") )
                    return 0
                    ;;
            esac
            ;;

        utils)
            if [ $COMP_CWORD -eq 2 ]; then
                # Utils subcommands
                COMPREPLY=( $(compgen -W "hash-copy-files hash-toolchains sync-copy-files update --help" -- "${cur}") )
                return 0
            elif [ $COMP_CWORD -gt 2 ]; then
                # Handle utils subcommand completion
                case "${COMP_WORDS[2]}" in
                    hash-copy-files)
                        case "${prev}" in
                            *)
                                COMPREPLY=( $(compgen -W "--dry-run --verbose -v --add-missing --help" -- "${cur}") )
                                return 0
                                ;;
                        esac
                        ;;
                    hash-toolchains)
                        case "${prev}" in
                            *)
                                COMPREPLY=( $(compgen -W "--dry-run --verbose -v --add-missing --help" -- "${cur}") )
                                return 0
                                ;;
                        esac
                        ;;
                    sync-copy-files)
                        case "${prev}" in
                            *)
                                COMPREPLY=( $(compgen -W "--dry-run --verbose -v --force -f --help" -- "${cur}") )
                                return 0
                                ;;
                        esac
                        ;;
                    update)
                        COMPREPLY=( $(compgen -W "--help" -- "${cur}") )
                        return 0
                        ;;
                esac
            fi
            ;;

        help)
            if [ $COMP_CWORD -eq 2 ]; then
                COMPREPLY=( $(compgen -W "${main_commands}" -- "${cur}") )
            fi
            return 0
            ;;

        *)
            # Fallback to global options
            COMPREPLY=( $(compgen -W "${global_opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

# Register the completion function
complete -F _cim_completions cim