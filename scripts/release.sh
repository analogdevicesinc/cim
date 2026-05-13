#!/usr/bin/env bash
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
# Release script for cim
# Creates a new version by updating Cargo.toml files, regenerating Cargo.lock,
# and creating a git commit and tag.
#
# Usage: ./scripts/release.sh [--summary] <version>
# Example: ./scripts/release.sh 0.8.0-rc.1
#          ./scripts/release.sh --summary 0.8.0-rc.1
#
# Options:
#   --summary  Use Claude Code (claude CLI) to generate a detailed,
#              categorized commit message from the git log since the
#              last release tag. Opens $EDITOR for review before
#              committing. Requires the 'claude' CLI to be installed.

set -e

# --- Argument parsing ---
SUMMARY=false
VERSION=""

for arg in "$@"; do
  case "$arg" in
    --summary)
      SUMMARY=true
      ;;
    -h|--help)
      echo "Usage: $0 [--summary] <version>"
      echo ""
      echo "Options:"
      echo "  --summary  Generate a detailed commit message using Claude Code"
      echo ""
      echo "Example: $0 0.8.0-rc.1"
      echo "         $0 --summary 0.8.0-rc.1"
      exit 0
      ;;
    -*)
      echo "✗ Unknown option: $arg"
      echo "Usage: $0 [--summary] <version>"
      exit 1
      ;;
    *)
      if [[ -n "$VERSION" ]]; then
        echo "✗ Unexpected argument: $arg (version already set to $VERSION)"
        exit 1
      fi
      VERSION="$arg"
      ;;
  esac
done

if [[ -z "$VERSION" ]]; then
  echo "Usage: $0 [--summary] <version>"
  echo "Example: $0 0.8.0-rc.1"
  exit 1
fi

# Validate version format (semver-ish: X.Y.Z or X.Y.Z-prerelease)
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
  echo "✗ Invalid version format: $VERSION"
  echo "Expected format: X.Y.Z or X.Y.Z-prerelease"
  exit 1
fi

# Check that we're on the main branch
CURRENT_BRANCH=$(git branch --show-current)
if [[ "$CURRENT_BRANCH" != "main" ]]; then
  echo "✗ Error: Not on main branch (currently on: $CURRENT_BRANCH)"
  echo "All release tags must be created on the main branch."
  echo "Please switch to main before running this script:"
  echo "  git checkout main"
  exit 1
fi

# --- Helper functions for --summary ---

# Find the most recent release tag matching v*
get_last_release_tag() {
  git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || \
    git rev-list --max-parents=0 HEAD
}

# Generate an AI-powered release summary using the claude CLI
generate_ai_summary() {
  local last_tag="$1"
  local version="$2"
  local commits diffstat

  commits=$(git log "${last_tag}..HEAD" --oneline)
  diffstat=$(git diff --stat "${last_tag}..HEAD" | tail -1)

  if [[ -z "$commits" ]]; then
    echo "No commits found since ${last_tag}."
    return
  fi

  local prompt
  prompt="Below are the git commits and diffstat for the cim release v${version} \
(since ${last_tag}). Write a commit message BODY only (no subject line). \
Categorize changes under these headings (omit empty categories): \
Features, Bug Fixes, Improvements, Breaking Changes, Documentation, Internal. \
Use '- ' bullet points. Wrap lines at 72 characters. Be concise. \
Do not include any markdown formatting like \`\`\` or **bold**.

Commits:
${commits}

Diffstat: ${diffstat}"

  claude -p "$prompt" 2>/dev/null
}

echo "Updating version to $VERSION..."

# Update Cargo.toml version
sed -i '' "s/^version = \".*\"/version = \"$VERSION\"/" dsdk-cli/Cargo.toml

# Regenerate Cargo.lock with new versions
echo "Updating Cargo.lock..."
cargo check --quiet

# Stage changes
git add dsdk-cli/Cargo.toml Cargo.lock

# --- Commit ---
if [[ "$SUMMARY" == true ]]; then
  # Check that claude CLI is available
  if ! command -v claude &>/dev/null; then
    echo "✗ Error: 'claude' CLI not found in PATH."
    echo "Install Claude Code or run without --summary."
    exit 1
  fi

  LAST_TAG=$(get_last_release_tag)
  echo "Generating release summary (${LAST_TAG}..HEAD)..."

  AI_BODY=$(generate_ai_summary "$LAST_TAG" "$VERSION") || true

  if [[ -z "$AI_BODY" ]]; then
    echo "⚠ AI summary generation failed, falling back to simple message."
    git commit -s -m "chore: release $VERSION"
  else
    # Assemble commit message in a temp file for editor review
    TMPFILE=$(mktemp "${TMPDIR:-/tmp}/cim-release-msg.XXXXXX")
    trap 'rm -f "$TMPFILE"' EXIT

    {
      echo "chore: release $VERSION"
      echo ""
      echo "$AI_BODY"
    } > "$TMPFILE"

    # Open editor for review
    "${EDITOR:-vi}" "$TMPFILE"

    # Check if the user emptied the file (abort)
    if [[ ! -s "$TMPFILE" ]] || ! grep -q '[^[:space:]]' "$TMPFILE"; then
      echo "✗ Commit message is empty, aborting."
      exit 1
    fi

    git commit -s -F "$TMPFILE"
  fi
else
  git commit -s -m "chore: release $VERSION"
fi

# Create tag
git tag "v$VERSION"

echo ""
echo "✓ Version v$VERSION created successfully!"
echo ""
echo "Next steps:"
echo "  git push && git push --tags"
