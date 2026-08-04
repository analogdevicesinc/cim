# Copilot Instructions for Code in Motion Tool Suite

## Project Context
This project is a Rust-based SDK management tool designed to manage multiple git repositories that make up a dynamic SDK. The tool uses a config file, `sdk.yml` to define the repositories, their URLs, commits/tags, makefile targets, toolchains and dependencies. `sdk.yml` can be found in target specific folder in the git called cim-manifests.git by default, however manifests can live in any git, the name doesn't matter. `cim` supports local mirroring, delta updates, repository management (add/remove), documentation generation, release work, listing target and Docker integration. The project creates a CLI tool named `cim`. Overall the tool shares similarities with repo tool from Google and west from the Zephyr project.

Two composition features let manifests avoid duplication:
- **Repository groups**: each `gits:` entry can set `group: <name>` (or a
  list of names); `init`/`update`/`foreach` accept `--include-group`/
  `--exclude-group` (comma-separated) to filter which repos are acted on.
  An entry with no `group:` implicitly belongs to `default`.
- **`extends:`/`overlay:`**: see "Composing Manifests" below.

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
    ├── example         : small, fully open source target used in docs/demos
    ├── overlay-example : demonstrates extends:/overlay: on top of 'example'
    └── optee-qemu-v8   : target for OP-TEE testing (somewhat large, fully open source)

- Each `targets` folder contains:
  - `sdk.yml`: Main manifest file in YAML that defines a project and the workspace it will create. When `sdk.yml` declares `extends: <base-target>`, its own `overlay:` key holds the `remove:`/`modify:` diff against the base target's content (see "Composing Manifests" below).
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
- `sdk.yml`/`os-dependencies.yml`/`python-dependencies.yml`: the originally-requested (primary) target always keeps these bare names at the workspace root.
- `.cim/target-overlays/`: only present for an `extends:` target; holds each ancestor level's own files, copied in with a `<target>-` prefix (e.g. `.cim/target-overlays/example-sdk.yml`, `.cim/target-overlays/example-os-dependencies.yml`) -- see "Composing Manifests" below.

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

## Composing Manifests: extends:/overlay:

A target's `sdk.yml` can declare `extends: <base-target>` to build on
top of another target instead of duplicating its whole manifest. All
merge logic lives in `dsdk-cli/src/overlay.rs`; `cim init` never
flattens the chain to disk -- every level's original sdk.yml is copied
into the workspace verbatim: the primary target's own file stays
bare-named at the workspace root, every ancestor's file goes into the
`.cim/target-overlays/` subfolder (`OVERLAYS_DIR` in `workspace.rs`,
nested under the existing `.cim/` directory also used for per-git
venvs) under its `<target>-` prefixed name (see `TargetFilePair`/
`discover_sibling_dep_files()`/`discover_dependency_files()`/
`resolve_local_extends_chain()` in `init_cmd.rs`/`workspace.rs`).

- **New entries** (any level, including the derived target itself):
  unique to that level, go directly in the normal
  `gits:`/`toolchains:`/`install:`/`copy_files:`/`variables:`
  lists/maps, same as a target with no `extends:` at all.
- **`overlay:`** (a key on that same level's sdk.yml, derived levels
  only): ONLY `remove:` and `modify:` operations against content
  *inherited* from the base -- there is no `add:`. `config::SdkConfig`'s
  `overlay: Option<overlay::OverlayConfig>` field names this key.

`overlay::apply_overlay()` merges a base `SdkConfig` with a derived
`SdkConfig` and its `OverlayConfig` per list section in a fixed order:
**remove** (against base only) → **combine** (base-after-removal plus
the derived target's own new entries, via `combine_with_own()`; a
name/dest collision is a hard error) → **modify** (against the
combined result). `merge_variables()` follows the same idea for the
`variables:` map (own upserts base, then overlay `remove:`/`set:`).

`overlay::compute_owned_entries()` determines, per section, which
entry names belong to the derived target (either added directly in
its own `sdk.yml` or referenced in its `overlay:` key's `modify:`).
This powers scoping for `cim release` (only tags/freezes gits owned by
the target) and `cim utils hash-copy-files`/`hash-toolchains` (both
always write the computed hash back into sdk.yml, since `overlay:` now
lives inside it -- see `load_extends_owned_entries()` in
`release_cmd.rs`).

`os-dependencies.yml`/`python-dependencies.yml` are per-level too, but
simpler: never merged, just copied and processed independently at
every level in the chain.

See `targets/overlay-example` (extends `targets/example`) in
cim-manifests for a full worked example.

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
- `cim docker create` generates a Dockerfile that downloads `cim` from
  GitHub Releases and runs `cim init` inside the container.
- Can be run from anywhere — no source tree or cross-compilation needed.
- The generated Dockerfile auto-detects the package manager and CPU
  architecture at build time.
- See `dsdk-cli/src/docker_manager.rs` for the `generate_dockerfile()`
  and `create_dockerfile()` functions.

## Formatting Standards
- **Line Endings**: All files must use Unix line endings (LF, `\n`) for cross-platform compatibility. Never use Windows line endings (CRLF, `\r\n`).
