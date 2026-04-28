# Bash Completion for Code in Motion (cim)

This directory contains bash completion scripts for the SDK manager tool.

## Installation

### Option 1: System-wide installation (Recommended)

For system-wide installation, copy the completion script to the bash completion directory:

#### On Ubuntu/Debian:
```bash
sudo cp completions/cim.bash /usr/share/bash-completion/completions/cim
```

#### On CentOS/RHEL/Fedora:
```bash
sudo cp completions/cim.bash /etc/bash_completion.d/cim
```

#### On macOS with Homebrew bash-completion:
```bash
# First install bash-completion if not already installed
brew install bash-completion

# Then copy the completion file
cp completions/cim.bash $(brew --prefix)/etc/bash_completion.d/cim
```

### Option 2: User-specific installation

For user-specific installation, add the completion to your personal bash configuration:

#### Create local completion directory (if it doesn't exist):
```bash
mkdir -p ~/.local/share/bash-completion/completions
```

#### Copy the completion script:
```bash
cp completions/cim.bash ~/.local/share/bash-completion/completions/cim
```

#### Add to your ~/.bashrc:
```bash
echo 'source ~/.local/share/bash-completion/completions/cim' >> ~/.bashrc
```

### Option 3: Direct sourcing

You can also source the completion script directly in your shell session or add it to your `~/.bashrc`:

```bash
# For current session only
source completions/cim.bash

# Or add to ~/.bashrc for permanent effect
echo 'source /path/to/dsdk/completions/cim.bash' >> ~/.bashrc
```

## Reload Bash

After installation, either:
- Start a new terminal session, or
- Reload your bash configuration: `source ~/.bashrc`

## Features

The completion script provides intelligent completion for:

### Commands:
- `cim list-targets` - List available SDK targets with source and target filtering
- `cim init` - Initialize workspace with target, source, version, and match filtering
- `cim update` - Update repositories with mirror, match, all, and verbose options
- `cim foreach` - Execute commands in each repository with match filtering
- `cim makefile` - Generate Makefile from SDK configuration
- `cim add` - Add new repositories with name, URL, and commit parameters
- `cim install` - Install dependencies with subcommands (os-deps, pip, toolchains, tools)
- `cim docs` - Documentation management with subcommands (create, build, serve)
- `cim docker` - Docker configuration with create subcommand
- `cim release` - Create release tags with include/exclude filters and dry-run mode
- `cim config` - Manage user configuration with list, get, create, edit, and validate options

### Options and Arguments:
- **Target completion** for `--target` options using `cim list-targets`
- **Version completion** for `--version` options based on selected target
- **Source completion** for `--source` with directory paths and common URL prefixes
- **Directory completion** for workspace paths (`--workspace`)
- **Match pattern completion** with common regex examples for filtering repositories
- **Profile completion** for Python dependency profiles (minimal, docs, dev, full)
- **Distribution completion** for Docker with common Linux distributions
- **Architecture completion** for cross-compilation targets
- **Theme completion** for documentation with Sphinx theme options
- **Format completion** for documentation output (html, pdf, epub)
- **Config key completion** for configuration management
- **Context-aware completion** based on current command and subcommand

### Examples:

```bash
# Complete main commands
cim <TAB>
# Shows: list-targets init update foreach makefile add install docs docker release config help

# Complete list-targets options
cim list-targets --<TAB>
# Shows: --source --target --help

# Complete init options
cim init --<TAB>
# Shows: --target --source --version --workspace --no-mirror --force --match --install --verbose --help

# Complete available targets (dynamically from list-targets)
cim init --target <TAB>
# Shows: adi-sdk dummy1 dummy2 optee-qemu-v8 etc.

# Complete versions for a specific target
cim init --target adi-sdk --version <TAB>
# Shows available versions/branches for adi-sdk

# Complete match patterns for filtering
cim init --match <TAB>
# Shows: "optee.*" ".*test.*" "build.*"

# Complete update options
cim update --<TAB>
# Shows: --no-mirror --match --all --verbose --help

# Complete foreach commands
cim foreach <TAB>
# Shows: "git status" "git pull" "git log" "git diff" "make clean" ls pwd --match --help

# Complete install subcommands
cim install <TAB>
# Shows: os-deps pip toolchains tools help

# Complete install pip options
cim install pip --<TAB>
# Shows: --profile --force --symlink --list-profiles --help

# Complete Python profiles
cim install pip --profile <TAB>
# Shows: minimal docs dev full

# Complete docs subcommands
cim docs <TAB>
# Shows: create build serve help

# Complete format options
cim docs build --format <TAB>
# Shows: html pdf epub

# Complete Docker distributions
cim docker create --distro <TAB>
# Shows: ubuntu:20.04 ubuntu:22.04 ubuntu:24.04 fedora:39 fedora:40 fedora:41 etc.

# Complete release options
cim release --<TAB>
# Shows: --tag --genconfig --include --exclude --dry-run --help

# Complete config options
cim config --<TAB>
# Shows: --list --get --path --template --create --force --edit --validate --help

# Complete config keys
cim config --get <TAB>
# Shows: default_source cert_validation
```

## Testing

To test if completion is working:

1. Type `cim ` and press `<TAB><TAB>` - you should see all available commands
2. Type `cim init --` and press `<TAB><TAB>` - you should see all init options
3. Try completing file paths with `cim init --config-file ` and `<TAB>`

## Troubleshooting

### Completion not working?

1. **Check if bash-completion is installed:**
   ```bash
   # Ubuntu/Debian
   dpkg -l bash-completion
   
   # CentOS/RHEL/Fedora
   rpm -q bash-completion
   
   # macOS with Homebrew
   brew list bash-completion
   ```

2. **Verify the completion is loaded:**
   ```bash
   complete -p cim
   ```
   Should show: `complete -F _sdk_manager_completions cim`

3. **Check your bash version:**
   ```bash
   bash --version
   ```
   Bash completion requires bash 4.0 or later.

4. **Ensure the script is executable:**
   ```bash
   chmod +x completions/cim.bash
   ```

### macOS Specific Issues:

On macOS, the default bash is often outdated. Install a newer version:
```bash
brew install bash
```

And ensure your terminal is using the Homebrew bash:
```bash
which bash
# Should show /usr/local/bin/bash or /opt/homebrew/bin/bash
```

Add the new bash to `/etc/shells` and set it as your default shell:
```bash
echo $(brew --prefix)/bin/bash | sudo tee -a /etc/shells
chsh -s $(brew --prefix)/bin/bash
```