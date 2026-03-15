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

// Build script for dsdk-cli (cim)
//
// PURPOSE:
// This build.rs script runs at compile time (before the main compilation) to capture
// the git commit hash and embed it as a compile-time environment variable. This enables
// the binary to know which exact source code commit it was built from.
//
// WHY IT'S NEEDED:
// The embedded git commit hash provides complete version traceability:
// - Users can run `cim --version` to see the exact commit the binary was built from
// - Bug reports can be correlated to specific source code versions
// - The .workspace marker file stores this commit for reproducibility
// - Docker operations include commit info for build tracking
//
// HOW IT WORKS:
// 1. Executes `git rev-parse --short=8 HEAD` to get the 8-character commit hash
// 2. Checks for uncommitted changes using `git diff-index --quiet HEAD`
// 3. Appends "-dirty" suffix if there are uncommitted/unstaged changes
// 4. Sets GIT_HASH as a compile-time environment variable via cargo:rustc-env
// 5. GIT_HASH is then accessible in the source code via env!("GIT_HASH")
// 6. Sets up rebuild triggers when git HEAD or index changes
//
// DIRTY STATE DETECTION:
// If you build with uncommitted changes, the hash will include "-dirty" suffix
// (e.g., "a1b2c3d4-dirty"). This prevents confusion when trying to trace a binary
// back to source code - you'll know the binary doesn't exactly match any commit.
// Release builds should always be from a clean git state (no "-dirty" suffix).
//
// USAGE IN SOURCE CODE:
// - main.rs: get_cim_version() includes commit in version info
// - main.rs: print_version_info() displays it with `cim --version`
// - main.rs: Stores commit in .workspace marker file
// - docker_manager.rs: Includes commit in Docker context version tracking
//
// RELATED:
// This pattern complements CARGO_PKG_VERSION (from Cargo.toml). Together with the
// binary's SHA256 hash, this provides a complete triple (version, commit, binary hash)
// for full version traceability.

use std::process::Command;

fn main() {
    // Get git commit hash at build time
    let output = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output();

    let mut git_hash = match output {
        Ok(output) => {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .unwrap_or_else(|_| "unknown".to_string())
                    .trim()
                    .to_string()
            } else {
                "unknown".to_string()
            }
        }
        Err(_) => "unknown".to_string(),
    };

    // Check if working directory has uncommitted changes
    // git diff-index --quiet HEAD returns non-zero if there are changes
    if git_hash != "unknown" {
        let dirty_check = Command::new("git")
            .args(["diff-index", "--quiet", "HEAD", "--"])
            .status();

        if let Ok(status) = dirty_check {
            if !status.success() {
                git_hash.push_str("-dirty");
            }
        }
    }

    // Make the git hash available as an environment variable during compilation
    println!("cargo:rustc-env=GIT_HASH={}", git_hash);

    // Embed the target triple so the binary knows which platform archive to download for self-update
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_TARGET={}", target);

    // Rebuild if the git HEAD or index changes (to detect dirty state)
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs/heads");
    println!("cargo:rerun-if-changed=../.git/index");
}
