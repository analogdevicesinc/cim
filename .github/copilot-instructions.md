# Copilot Instructions for Code in Motion Tool Suite

## Project Context
This project is a Rust-based SDK management tool designed to manage multiple git repositories that make up a dynamic SDK. The tool uses a config file, `sdk.yml` to define the repositories, their URLs, commits/tags, makefile targets, toolchains and dependencies. `sdk.yml` can be found in target specific folder in the git called cim-manifests.git by default, however manifests can live in any git, the name doesn't matter. `cim` supports local mirroring, delta updates, repository management (add/remove), documentation generation, release work, listing target and Docker integration. The project creates a CLI tool named `cim`. Overall the tool shares similarities with repo tool from Google and west from the Zephyr project.

## Project and directory Structure
.
├── README.md     : cim README file
├── completions   : Bash completions
├── dsdk-cli      : The main CLI tool
│   └── src       : Source code for the CLI tool
├── dsdk-vscode   : VSCode extension for the SDK tool
│   ├── README.md : Readme for the VSCode extension
│   ├── dist
│   ├── media
│   ├── node_modules
│   └── src      : Source code for the VSCode extension

## cim-manifests structure
.
├── shared             : Shared yml-files and templates
│   └── templates      : templates for documentation generation
└── targets            : cim targets (initialized with the 'init' command)
    ├── dummy1         : small and simple target for testing
    └── optee-qemu-v8  : target for OP-TEE testing (somewhat large, fully open source)

- Each `targets` folder contains:
  - `sdk.yml`: Main manifest file in YAML that defines a project and the workspace it will create.
  - `os-dependencies.yml`: Lists required HOST OS/system dependencies.
  - `python-dependencies.yml`: Lists required Python dependencies
  - All `*.yml` files can be symlinked to files in the shared folder and other locations if needed.
- Default location on disk is `$HOME/devel/cim-manifests`
- Legacy location `$HOME/devel/sdk-manager-manifests` is also checked automatically for backward compatibility
- Our remote location is at: `https://github.com/analogdevicesinc/cim-manifests`
- cim can point to any other location via the `cim init --source <path-or-url>` option.

## Workspace structure
- `.workspace`: Workspace marker file created by init command for automatic workspace detection.
- `Makefile`: Makefile created by `cim makefile` command for easy access to common targets.
- `.vscode`: VCcode `tasks.json` also created when running `cim makefile`.

## WORKSPACE Variable and ${{ VAR }} Syntax

Every generated `Makefile` opens with:

```make
WORKSPACE := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
```

This makes `$(WORKSPACE)` a relocatable Make variable pointing to the
workspace root at build time. Never hard-code absolute workspace paths
in manifest fields or Makefile fragments; always use `${{ WORKSPACE }}`
(in sdk.yml) or `$(WORKSPACE)` (in `.mk` files).

The `${{ VAR }}` template syntax is used throughout sdk.yml. Its
behaviour differs by field type:

- **Recipe commands** (`build`, `test`, `clean`, `flash`, `envsetup`,
  per-git `build`): `${{ VAR }}` → `$(VAR)` via
  `render_command_for_makefile()`. Make expands the reference at build
  time, so the value remains overridable from the command line.
- **Path fields** (`build_folder`, `makefile_include` entries):
  at `cim makefile` generation time `${{ WORKSPACE }}` is expanded to
  the real workspace path for file-system probing
  (`resolve_build_folder_for_check()` injects `WORKSPACE` into the
  variable map and calls `expand_manifest_vars()`). The `-include`
  directive written to the Makefile uses `$(WORKSPACE)/…` so it
  remains portable across machines.
- **`variables:` values**: host env-var syntax (`$VAR`, `$HOME`, `~/`)
  is expanded at manifest load time via `expand_env_vars()`. Any
  surviving `${{ VAR }}` becomes `$(VAR)` in the generated `?=`
  assignment via `render_command_for_makefile()`.

Key functions in `dsdk-cli/src/`:
- `makefile.rs` — `render_command_for_makefile()`: `${{ VAR }}` →
  `$(VAR)` for use in Makefile recipes and `-include` directives.
- `makefile.rs` — `resolve_build_folder_for_check()`: resolves a
  `build_folder` or similar path field for FS probing; handles
  relative, absolute, and `${{ WORKSPACE }}` forms.
- `workspace.rs` — `expand_manifest_vars()`: expands `${{ VAR }}`
  tokens against a caller-supplied variable map.
- `workspace.rs` — `expand_env_vars()`: expands `$VAR`, `${VAR}`,
  `~/` from the host environment.

## Cim Development Workflow
- Use `make` or `make all` to build, test, lint, format, and install cim in one command.
- Use `make build` for quick builds during development.
- Always use mirrors to save bandwidth and speed up cloning. Mirrors will be located at `$HOME/tmp/mirror` by default. The location is defined in `sdk.yml` under `mirror`.
- Workspace will be created at `$HOME/dsdk-{target-name}` by default if no `--workspace` option is given during `init` (e.g., `dsdk-adi-sdk` for the `adi-sdk` target).
- For testing, you can always use `-w $HOME/dsdk-test`.
- When Python is needed, use a virtual environment to avoid dependency conflicts. Use `python -m venv .venv` to create a virtual environment and `source .venv/bin/activate` to activate it. Note that cim can also create virtual environments in workspace by running `cim install pip`. To save time, you can use `cim install pip --symlink` that will install and reference a shared virtual environment located in the mirror folder.
- Use `cargo run -- <command>` to run the CLI tool during development.
- If not all gits are needed when implementing a new feature and testing, use `init --match` to filter which repos to clone to save time and bandwidth.

## Git commits
- Before git commit, run: `make all` (or individually: `cargo fmt`, `cargo clippy`, `cargo test`).
- Always fix all errors before committing.
- Always use `git commit -s` to sign off your commits.
- Consider making small, incremential and logical commits.
- Use Linux kernel style commit messages, with a short (50 char) summary, a blank line, and a more detailed explanatory text wrapped at 72 characters.

## Makefile Targets
The repository includes a Makefile to streamline common development tasks:
- `make` or `make all` - Run the complete workflow: build, test, clippy, fmt, and install (default target)
- `make build` - Build cim in release mode
- `make test` - Run all tests
- `make clippy` - Run clippy linter
- `make fmt` - Format code
- `make install` - Install cim CLI to `$HOME/bin` (creates directory if needed)
- `make clean` - Clean build artifacts
- `make help` - Display all available targets

## Docker Usage
- Can only be used from with/from the source code folder.
- Build and run the SDK tool in a container using the provided `Dockerfile`.
- See `dsdk-cli/src/docker_manager.rs` for Docker integration details.

## Formatting Standards
- **Line Endings**: All files must use Unix line endings (LF, `\n`) for cross-platform compatibility. Never use Windows line endings (CRLF, `\r\n`).
