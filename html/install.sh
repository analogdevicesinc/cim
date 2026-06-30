#!/bin/sh
# Code in Motion (cim) installer
#
# Installs the latest cim release into ~/.local/bin.
#
# Usage:
#   curl -fsSL https://analogdevicesinc.github.io/cim/install.sh | sh
#
# Install a specific version instead of the latest:
#   curl -fsSL https://analogdevicesinc.github.io/cim/install.sh | CIM_VERSION=v1.1.12 sh
#   # or, when running a downloaded copy:
#   ./install.sh v1.1.12

set -eu

REPO="analogdevicesinc/cim"
BIN_DIR="$HOME/.local/bin"

# --- helpers ----------------------------------------------------------------

err() {
	printf 'error: %s\n' "$1" >&2
	exit 1
}

need() {
	command -v "$1" >/dev/null 2>&1 || err "'$1' is required but was not found in PATH."
}

# --- preflight --------------------------------------------------------------

need curl
need tar

# --- detect platform --------------------------------------------------------

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
	Linux)
		case "$arch" in
			x86_64 | amd64) target="x86_64-unknown-linux-gnu" ;;
			aarch64 | arm64) target="aarch64-unknown-linux-gnu" ;;
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

# --- resolve version --------------------------------------------------------

# Precedence: first argument, then CIM_VERSION, then the latest release.
version="${1:-${CIM_VERSION:-}}"

# Release tags carry a 'v' prefix (e.g. v1.1.12); tolerate either form when a
# version is supplied explicitly.
case "$version" in
	"" | v*) ;;
	*) version="v$version" ;;
esac

if [ -z "$version" ]; then
	printf 'Resolving latest release...\n'
	# Follow the /releases/latest redirect and read the resolved tag from the
	# final URL. Avoids a jq dependency.
	latest_url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
		"https://github.com/$REPO/releases/latest")" \
		|| err "could not reach GitHub to determine the latest release."
	version="${latest_url##*/}"
	[ -n "$version" ] && [ "$version" != "latest" ] \
		|| err "could not determine the latest release version."
fi

printf 'Installing cim %s (%s)...\n' "$version" "$target"

# --- download & extract -----------------------------------------------------

asset="cim-suite-$version-$target.tar.gz"
url="https://github.com/$REPO/releases/download/$version/$asset"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$url" -o "$tmp/$asset" \
	|| err "failed to download $url"

tar -xzf "$tmp/$asset" -C "$tmp" \
	|| err "failed to extract $asset"

src="$tmp/cim-suite-$version-$target/cim"
[ -f "$src" ] || err "cim binary not found in archive (expected $src)."

# --- install ----------------------------------------------------------------

mkdir -p "$BIN_DIR"
mv "$src" "$BIN_DIR/cim"
chmod 755 "$BIN_DIR/cim"

printf 'Installed cim to %s\n' "$BIN_DIR/cim"

# --- PATH check -------------------------------------------------------------

case ":$PATH:" in
	*":$BIN_DIR:"*)
		printf '\n\342\234\223 Installation complete. Run: cim --help\n'
		;;
	*)
		printf '\nInstallation complete, but %s is not in your PATH.\n' "$BIN_DIR"
		printf 'Add it by appending this line to your shell profile (e.g. ~/.bashrc or ~/.zshrc):\n\n'
		printf '    export PATH="$HOME/.local/bin:$PATH"\n\n'
		printf 'Then restart your shell or run:\n\n'
		printf '    export PATH="$HOME/.local/bin:$PATH"\n\n'
		printf 'After that, run: cim --help\n'
		;;
esac
