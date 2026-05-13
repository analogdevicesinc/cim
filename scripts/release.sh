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
# Usage: ./scripts/release.sh <version>
# Example: ./scripts/release.sh 0.8.0-rc.1

set -e

VERSION="$1"

if [[ -z "$VERSION" ]]; then
  echo "Usage: $0 <version>"
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

echo "Updating version to $VERSION..."

# Update Cargo.toml version
sed -i '' "s/^version = \".*\"/version = \"$VERSION\"/" dsdk-cli/Cargo.toml

# Regenerate Cargo.lock with new versions
echo "Updating Cargo.lock..."
cargo check --quiet

# Stage changes
git add dsdk-cli/Cargo.toml Cargo.lock

# Commit with sign-off
git commit -s -m "chore: release $VERSION"

# Create tag
git tag "v$VERSION"

echo ""
echo "✓ Version v$VERSION created successfully!"
echo ""
echo "Next steps:"
echo "  git push && git push --tags"
