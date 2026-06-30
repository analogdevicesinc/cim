#!/bin/sh
# Code in Motion (cim) installer
#
# Installs or updates the cim CLI on Linux and macOS.
#
# Usage:
#   curl -fsSL https://analogdevicesinc.github.io/cim/install.sh | sh
#
# Re-run the same command to upgrade: the script looks for an existing cim on
# your PATH and updates that binary in place. With nothing installed yet it
# installs into ~/.local/bin.
#
# The latest release is always the one installed, configured only by the
# variables below.
#
# Environment:
#   CIM_BIN_DIR   install directory; overrides PATH detection entirely
#   CIM_FORCE     set to 1 to reinstall even when already up to date
#
# To install some other version, download it from the releases page:
#   https://github.com/analogdevicesinc/cim/releases

set -eu

REPO="analogdevicesinc/cim"
DEFAULT_BIN_DIR="$HOME/.local/bin"

# --- helpers ----------------------------------------------------------------

err() {
	printf 'error: %s\n' "$1" >&2
	exit 1
}

need() {
	command -v "$1" >/dev/null 2>&1 || err "'$1' is required but was not found in PATH."
}

# Follow a symlink chain to the real file. readlink -f is unavailable on older
# macOS, so walk the chain by hand.
resolve_link() {
	_link="$1"
	_hops=0
	while [ -L "$_link" ]; do
		_hops=$((_hops + 1))
		[ "$_hops" -le 16 ] || err "too many symlinks while resolving $1"
		_target="$(readlink "$_link")"
		case "$_target" in
			/*) _link="$_target" ;;
			*) _link="$(dirname "$_link")/$_target" ;;
		esac
	done
	printf '%s\n' "$_link"
}

# Version of an installed cim, or empty if it does not run. `cim --version`
# prints "cim: v1.2.1" on its first line, then SHA256 and commit lines.
binary_version() {
	"$1" --version 2>/dev/null | head -n 1 | tr -d '\r' | awk 'NF { print $NF }'
}

# --- preflight --------------------------------------------------------------

need curl
need tar

# --- detect platform --------------------------------------------------------

os="$(uname -s)"
arch="$(uname -m)"

# Linux releases ship both glibc and musl x86_64 builds. A glibc binary will
# not start on a musl system (Alpine and friends), so pick by libc.
is_musl() {
	[ -f /etc/alpine-release ] && return 0
	command -v ldd >/dev/null 2>&1 || return 1
	ldd --version 2>&1 | grep -qiE 'gnu|glibc' && return 1
	return 0
}

case "$os" in
	Linux)
		case "$arch" in
			x86_64 | amd64)
				if is_musl; then
					target="x86_64-unknown-linux-musl"
				else
					target="x86_64-unknown-linux-gnu"
				fi
				;;
			aarch64 | arm64)
				# Only a glibc aarch64 build is published.
				if is_musl; then
					err "no musl build is published for aarch64 Linux. See https://github.com/$REPO/releases"
				fi
				target="aarch64-unknown-linux-gnu"
				;;
			*) err "unsupported architecture '$arch' on Linux. See https://github.com/$REPO/releases" ;;
		esac
		;;
	Darwin)
		case "$arch" in
			x86_64) target="x86_64-apple-darwin" ;;
			arm64 | aarch64) target="aarch64-apple-darwin" ;;
			*) err "unsupported architecture '$arch' on macOS. See https://github.com/$REPO/releases" ;;
		esac
		;;
	*)
		err "unsupported operating system '$os'. See https://github.com/$REPO/releases"
		;;
esac

# --- resolve install target -------------------------------------------------

# Precedence:
#   1. CIM_BIN_DIR      an explicit choice, never second-guessed
#   2. a cim already on PATH, updated where it lives
#   3. ~/.local/bin
dest=""
existing=""
found_via_path=no

if [ -n "${CIM_BIN_DIR:-}" ]; then
	dest="$CIM_BIN_DIR/cim"
	if [ -f "$dest" ]; then
		existing="$dest"
	fi
else
	path_cim="$(command -v cim 2>/dev/null || true)"
	if [ -n "$path_cim" ]; then
		# Write to the real file rather than clobbering a symlink that a
		# package manager may own.
		existing="$(resolve_link "$path_cim")"
		dest="$existing"
		found_via_path=yes
		if [ "$existing" != "$path_cim" ]; then
			printf 'Found cim at %s -> %s\n' "$path_cim" "$existing"
		fi
	else
		dest="$DEFAULT_BIN_DIR/cim"
	fi
fi

bin_dir="$(dirname "$dest")"

current=""
if [ -n "$existing" ]; then
	current="$(binary_version "$existing")"
fi

# --- resolve version --------------------------------------------------------

printf 'Resolving latest release...\n'
# Follow the /releases/latest redirect and read the resolved tag from the final
# URL. Avoids a jq dependency.
latest_url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
	"https://github.com/$REPO/releases/latest")" \
	|| err "could not reach GitHub to determine the latest release."
version="${latest_url##*/}"
[ -n "$version" ] && [ "$version" != "latest" ] \
	|| err "could not determine the latest release version."

# --- already up to date? ----------------------------------------------------

if [ -n "$current" ] && [ "${CIM_FORCE:-}" != "1" ] && [ "${current#v}" = "${version#v}" ]; then
	printf 'cim %s is already installed at %s and up to date.\n' "$current" "$existing"
	printf 'Set CIM_FORCE=1 to reinstall it anyway.\n'
	exit 0
fi

# --- permission check -------------------------------------------------------

# Creating the directory now doubles as the permission check: both it and the
# file already there, if any, have to be writable.
mkdir -p "$bin_dir" 2>/dev/null || true

if [ ! -w "$bin_dir" ] || { [ -e "$dest" ] && [ ! -w "$dest" ]; }; then
	printf 'error: cannot write to %s\n' "$dest" >&2
	printf '\nRe-run under sudo so the shell writing the file has permission:\n\n' >&2
	if [ -n "${CIM_BIN_DIR:-}" ]; then
		printf '    curl -fsSL https://analogdevicesinc.github.io/cim/install.sh | sudo CIM_BIN_DIR=%s sh\n\n' \
			"$CIM_BIN_DIR" >&2
	else
		printf '    curl -fsSL https://analogdevicesinc.github.io/cim/install.sh | sudo sh\n\n' >&2
	fi
	printf 'Keep any CIM_* variables after sudo so they reach the elevated shell.\n' >&2
	exit 1
fi

if [ -n "$existing" ]; then
	if [ -n "$current" ]; then
		printf 'Found existing cim %s at %s - updating in place to %s (%s)...\n' \
			"$current" "$existing" "$version" "$target"
	else
		printf 'Replacing existing install at %s with %s (%s)...\n' \
			"$existing" "$version" "$target"
	fi
else
	printf 'Installing cim %s (%s) to %s...\n' "$version" "$target" "$bin_dir"
fi

# --- download & extract -----------------------------------------------------

asset="cim-suite-$version-$target.tar.gz"
url="https://github.com/$REPO/releases/download/$version/$asset"

tmp="$(mktemp -d)"
# Staged inside the destination directory so the final step is a rename(2)
# within one filesystem, which is atomic: the name always resolves to either the
# old binary or the complete new one. Moving straight from $tmp would instead be
# a cross-filesystem copy, which unlinks the destination and rewrites it in
# place - leaving a window where cim is missing or half-written, and risking
# ETXTBSY where mv does not unlink first.
staged="$bin_dir/.cim.install.$$"
trap 'rm -rf "$tmp" "$staged"' EXIT

curl -fsSL "$url" -o "$tmp/$asset" \
	|| err "failed to download $url"

tar -xzf "$tmp/$asset" -C "$tmp" \
	|| err "failed to extract $asset"

src="$tmp/cim-suite-$version-$target/cim"
[ -f "$src" ] || err "cim binary not found in archive (expected $src)."

# --- install ----------------------------------------------------------------

mv "$src" "$staged"
chmod 755 "$staged"

# Confirm the downloaded binary runs before it replaces a working install.
staged_version="$(binary_version "$staged")"
[ -n "$staged_version" ] || err "the downloaded cim did not run on this machine ($os/$arch); $dest was left untouched."

mv "$staged" "$dest"

printf 'Installed cim %s to %s\n' "$staged_version" "$dest"

# --- PATH and shadowing checks ----------------------------------------------

# A cim found through `command -v` is already reachable, even when its real
# file lives outside PATH behind a symlink, so only an install into a directory
# the script picked itself needs the PATH advice.
if [ "$found_via_path" = no ]; then
	case ":$PATH:" in
		*":$bin_dir:"*) ;;
		*)
			printf '\nInstallation complete, but %s is not in your PATH.\n' "$bin_dir"
			printf 'Add it by appending this line to your shell profile (e.g. ~/.bashrc or ~/.zshrc):\n\n'
			printf '    export PATH="%s:$PATH"\n\n' "$bin_dir"
			printf 'Then restart your shell or run:\n\n'
			printf '    export PATH="%s:$PATH"\n\n' "$bin_dir"
			printf 'After that, run: cim --help\n'
			exit 0
			;;
	esac
fi

# On PATH, but an earlier entry may hold a different cim that would still win.
resolved="$(command -v cim 2>/dev/null || true)"
if [ -n "$resolved" ]; then
	resolved="$(resolve_link "$resolved")"
fi

if [ -n "$resolved" ] && [ "$resolved" != "$dest" ]; then
	shadow_version="$(binary_version "$resolved")"
	printf '\nWarning: another cim earlier in your PATH will still be used:\n\n'
	printf '    %s' "$resolved"
	[ -n "$shadow_version" ] && printf ' (%s)' "$shadow_version"
	printf '\n\nRemove it, or put %s earlier in your PATH, so that\n' "$bin_dir"
	printf 'the cim you just installed is the one that runs.\n'
	exit 0
fi

printf '\n\342\234\223 Installation complete. Run: cim --help\n'
