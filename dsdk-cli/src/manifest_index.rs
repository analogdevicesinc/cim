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

//! Locating the `sdk.yml` manifests a source repository exposes.
//!
//! A source repository describes its manifests with an optional root
//! `.cim.yml` (see [`PROJECT_CONFIG_FILE`]); without one, manifests follow the
//! `targets/<name>/sdk.yml` layout. Both `list-targets` and `init` resolve
//! manifests through this module so they can never disagree.

use crate::workspace::SDK_CONFIG_FILE;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Optional project-root file that lists the manifests a repository exposes.
pub const PROJECT_CONFIG_FILE: &str = ".cim.yml";

/// Raw contents of the optional `.cim.yml` at a source repository root.
/// At least one of `targets:` and `members:` is set.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectConfig {
    targets: Option<BTreeMap<String, PathBuf>>,
    members: Option<Vec<String>>,
}

/// Validated `.cim.yml`: named manifests, selected by `--target`. They are the
/// explicit `targets:` entries plus every directory matched by a `members:`
/// glob that holds an `sdk.yml`.
#[derive(Debug)]
struct ProjectIndex {
    targets: BTreeMap<String, PathBuf>,
    members: Vec<String>,
}

impl ProjectConfig {
    fn into_index(self) -> Result<ProjectIndex, anyhow::Error> {
        let targets = self.targets.unwrap_or_default();
        let members = self.members.unwrap_or_default();
        if targets.is_empty() && members.is_empty() {
            return Err(anyhow::anyhow!("one of 'targets' or 'members' must be set"));
        }
        for (name, path) in &targets {
            validate_target_name(name)?;
            validate_manifest_path(path)?;
        }
        for pattern in &members {
            validate_member_pattern(pattern)?;
        }
        Ok(ProjectIndex { targets, members })
    }
}

/// Target names end up in version refs (`<name>-*`) and workspace paths.
fn validate_target_name(name: &str) -> Result<(), anyhow::Error> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(anyhow::anyhow!("Invalid target name '{}'", name));
    }
    Ok(())
}

/// Whether `path` is non-empty and stays inside the repository it is relative
/// to: no `..`, root or drive prefix components.
fn is_repo_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
}

/// A `members:` pattern must be a valid glob that stays inside the repository.
fn validate_member_pattern(pattern: &str) -> Result<(), anyhow::Error> {
    if !is_repo_relative(Path::new(pattern)) {
        return Err(anyhow::anyhow!(
            "Member pattern must be relative to the source repository and must not contain '..': '{}'",
            pattern
        ));
    }
    glob::Pattern::new(pattern)
        .map_err(|e| anyhow::anyhow!("Invalid member pattern '{}': {}", pattern, e))?;
    Ok(())
}

/// How to locate the `sdk.yml` inside a source repository.
#[derive(Debug, Clone, Copy)]
pub enum ManifestSelection<'a> {
    /// A repository-relative path, from `--manifest-path` or the workspace marker.
    Explicit(&'a Path),
    /// The root `.cim.yml` when present, else `targets/<target>/sdk.yml`.
    Discover,
    /// Always `targets/<target>/sdk.yml`, ignoring any `.cim.yml`.
    TargetsLayout,
}

impl<'a> ManifestSelection<'a> {
    /// Selection for a user-supplied `--manifest-path` override, if any.
    pub fn from_override(manifest_path: Option<&'a Path>) -> Self {
        manifest_path.map_or(Self::Discover, Self::Explicit)
    }
}

/// Read and validate the repository's `.cim.yml`, if present.
fn read_project_index(repo_path: &Path) -> Result<Option<ProjectIndex>, anyhow::Error> {
    let config_path = repo_path.join(PROJECT_CONFIG_FILE);
    if !config_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&config_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read project config {}: {}",
            config_path.display(),
            e
        )
    })?;
    let project_config: ProjectConfig = noyalib::from_str(&content).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse project config {}: {}",
            config_path.display(),
            e
        )
    })?;
    project_config
        .into_index()
        .map(Some)
        .map_err(|e| anyhow::anyhow!("Invalid project config {}: {}", config_path.display(), e))
}

/// Reject manifest paths that are empty, escape the repository, or do not
/// name an `sdk.yml`.
fn validate_manifest_path(manifest_path: &Path) -> Result<(), anyhow::Error> {
    if !is_repo_relative(manifest_path) {
        return Err(anyhow::anyhow!(
            "Manifest path must be relative to the source repository and must not contain '..': {}",
            manifest_path.display()
        ));
    }
    if manifest_path.file_name().and_then(|name| name.to_str()) != Some(SDK_CONFIG_FILE) {
        return Err(anyhow::anyhow!(
            "Manifest path must point to {}: {}",
            SDK_CONFIG_FILE,
            manifest_path.display()
        ));
    }
    Ok(())
}

/// Resolve the repository-relative path of the `sdk.yml` selected by
/// `selection`, validating that it stays inside the repository and exists.
pub fn resolve_manifest_path(
    repo_path: &Path,
    target: &str,
    selection: ManifestSelection<'_>,
) -> Result<PathBuf, anyhow::Error> {
    let pointed = match selection {
        ManifestSelection::Explicit(path) => Some(path.to_path_buf()),
        ManifestSelection::Discover => match read_project_index(repo_path)? {
            Some(index) => {
                let entries = index_entries(repo_path, index)?;
                let path = entries.get(target).cloned().ok_or_else(|| {
                    let available: Vec<&str> = entries.keys().map(String::as_str).collect();
                    anyhow::anyhow!(
                        "Target '{}' is not defined in {} (available targets: {})",
                        target,
                        PROJECT_CONFIG_FILE,
                        available.join(", ")
                    )
                })?;
                Some(path)
            }
            None => None,
        },
        ManifestSelection::TargetsLayout => None,
    };
    let manifest_path =
        pointed.unwrap_or_else(|| PathBuf::from("targets").join(target).join(SDK_CONFIG_FILE));

    validate_manifest_path(&manifest_path)?;

    if !repo_path.join(&manifest_path).is_file() {
        return Err(anyhow::anyhow!(
            "Target '{}' not found: {} does not exist in the source repository",
            target,
            manifest_path.display()
        ));
    }

    Ok(manifest_path)
}

/// A target exposed by a source repository and where its `sdk.yml` lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredTarget {
    pub name: String,
    /// Repository-relative path to the target's `sdk.yml`.
    pub manifest_path: PathBuf,
}

/// List the targets a source repository exposes: the entries of its
/// `.cim.yml` when present, otherwise every `targets/<name>/sdk.yml`.
pub fn discover_targets(repo_path: &Path) -> Result<Vec<DiscoveredTarget>, anyhow::Error> {
    let entries = match read_project_index(repo_path)? {
        Some(index) => index_entries(repo_path, index)?,
        None => scan_targets_dir(repo_path)?,
    };
    Ok(entries
        .into_iter()
        .map(|(name, manifest_path)| DiscoveredTarget {
            name,
            manifest_path,
        })
        .collect())
}

/// Every target a `.cim.yml` exposes, keyed by name.
fn index_entries(
    repo_path: &Path,
    index: ProjectIndex,
) -> Result<BTreeMap<String, PathBuf>, anyhow::Error> {
    let ProjectIndex {
        mut targets,
        members,
    } = index;
    for (name, manifest) in &targets {
        ensure_manifest_exists(repo_path, name, manifest)?;
    }
    expand_members(repo_path, &members, &mut targets)?;
    Ok(targets)
}

/// An index entry must not advertise a target that cannot be initialized.
fn ensure_manifest_exists(
    repo_path: &Path,
    name: &str,
    manifest_path: &Path,
) -> Result<(), anyhow::Error> {
    if !repo_path.join(manifest_path).is_file() {
        return Err(anyhow::anyhow!(
            "Target '{}' points to {}, which does not exist in the source repository",
            name,
            manifest_path.display()
        ));
    }
    Ok(())
}

/// Add a target, rejecting a name that already maps to a different manifest.
/// Reaching the same manifest twice (e.g. two overlapping globs) is fine.
fn insert_target(
    entries: &mut BTreeMap<String, PathBuf>,
    name: String,
    manifest_path: PathBuf,
) -> Result<(), anyhow::Error> {
    match entries.get(&name) {
        Some(existing) if *existing != manifest_path => Err(anyhow::anyhow!(
            "Target '{}' is defined more than once: {} and {}",
            name,
            existing.display(),
            manifest_path.display()
        )),
        Some(_) => Ok(()),
        None => {
            entries.insert(name, manifest_path);
            Ok(())
        }
    }
}

/// Expand `members:` globs into `entries`: each matched directory that holds
/// an `sdk.yml` is a target named after the directory. Like Cargo workspace
/// members, patterns are relative to the repository root.
fn expand_members(
    repo_path: &Path,
    patterns: &[String],
    entries: &mut BTreeMap<String, PathBuf>,
) -> Result<(), anyhow::Error> {
    let repo_root = repo_path.canonicalize()?;
    let escaped_root = glob::Pattern::escape(&repo_path.to_string_lossy());

    for pattern in patterns {
        let full_pattern = format!("{}/{}", escaped_root.trim_end_matches('/'), pattern);
        let matches = glob::glob(&full_pattern)
            .map_err(|e| anyhow::anyhow!("Invalid member pattern '{}': {}", pattern, e))?;
        for dir in matches {
            let dir = dir?;
            if !dir.is_dir() || !dir.join(SDK_CONFIG_FILE).is_file() {
                continue;
            }
            // A symlinked member must not lead outside the repository.
            if !dir.canonicalize()?.starts_with(&repo_root) {
                return Err(anyhow::anyhow!(
                    "Member '{}' (from pattern '{}') resolves outside the source repository",
                    dir.display(),
                    pattern
                ));
            }

            let relative_dir = dir.strip_prefix(repo_path).unwrap_or(&dir);
            let name = relative_dir
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Member pattern '{}' matches the repository root; members must be \
                         subdirectories (use 'targets:' for a root {})",
                        pattern,
                        SDK_CONFIG_FILE
                    )
                })?;
            validate_target_name(name)?;
            let manifest_path = relative_dir
                .join(SDK_CONFIG_FILE)
                .components()
                .filter(|component| *component != std::path::Component::CurDir)
                .collect();
            insert_target(entries, name.to_string(), manifest_path)?;
        }
    }
    Ok(())
}

/// Scan the conventional `targets/<name>/sdk.yml` layout, i.e. `members: ["targets/*"]`.
fn scan_targets_dir(repo_path: &Path) -> Result<BTreeMap<String, PathBuf>, anyhow::Error> {
    let targets_dir = repo_path.join("targets");
    if !targets_dir.exists() {
        return Err(anyhow::anyhow!(
            "Targets directory not found: {}",
            targets_dir.display()
        ));
    }
    let mut entries = BTreeMap::new();
    expand_members(repo_path, &["targets/*".to_string()], &mut entries)?;
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_workspace() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let path = temp_dir.path().to_path_buf();
        (temp_dir, path)
    }

    #[test]
    fn test_resolve_manifest_path_precedence_and_validation() {
        let (_temp_dir, repo_path) = create_test_workspace();
        let pointer_manifest = repo_path.join("cim/sdk.yml");
        let explicit_manifest = repo_path.join("custom/sdk.yml");
        let legacy_manifest = repo_path.join("targets/project/sdk.yml");
        for manifest in [&pointer_manifest, &explicit_manifest, &legacy_manifest] {
            fs::create_dir_all(manifest.parent().unwrap()).unwrap();
            fs::write(manifest, "gits: []\n").unwrap();
        }
        fs::write(
            repo_path.join(PROJECT_CONFIG_FILE),
            "targets:\n  project: cim/sdk.yml\n",
        )
        .unwrap();

        assert_eq!(
            resolve_manifest_path(&repo_path, "project", ManifestSelection::Discover).unwrap(),
            PathBuf::from("cim/sdk.yml")
        );
        assert_eq!(
            resolve_manifest_path(
                &repo_path,
                "project",
                ManifestSelection::Explicit(Path::new("custom/sdk.yml")),
            )
            .unwrap(),
            PathBuf::from("custom/sdk.yml")
        );
        assert_eq!(
            resolve_manifest_path(&repo_path, "project", ManifestSelection::TargetsLayout).unwrap(),
            PathBuf::from("targets/project/sdk.yml")
        );

        fs::write(repo_path.join(PROJECT_CONFIG_FILE), "invalid: [\n").unwrap();
        assert_eq!(
            resolve_manifest_path(
                &repo_path,
                "project",
                ManifestSelection::Explicit(Path::new("custom/sdk.yml")),
            )
            .unwrap(),
            PathBuf::from("custom/sdk.yml")
        );
        assert!(resolve_manifest_path(&repo_path, "project", ManifestSelection::Discover).is_err());

        let traversal = resolve_manifest_path(
            &repo_path,
            "project",
            ManifestSelection::Explicit(Path::new("../sdk.yml")),
        )
        .unwrap_err();
        assert!(traversal
            .to_string()
            .contains("relative to the source repository"));
        let wrong_file = resolve_manifest_path(
            &repo_path,
            "project",
            ManifestSelection::Explicit(Path::new("custom/other.yml")),
        )
        .unwrap_err();
        assert!(wrong_file.to_string().contains("must point to sdk.yml"));
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn target_names(targets: &[DiscoveredTarget]) -> Vec<&str> {
        targets.iter().map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn test_discover_targets_from_targets_map() {
        let (_temp_dir, repo) = create_test_workspace();
        write_file(&repo, "boards/h7/sdk.yml", "gits: []\n");
        write_file(&repo, "tools/docs/sdk.yml", "gits: []\n");
        write_file(
            &repo,
            PROJECT_CONFIG_FILE,
            "targets:\n  h7: boards/h7/sdk.yml\n  docs: tools/docs/sdk.yml\n",
        );
        // The legacy layout is ignored once .cim.yml exists
        write_file(&repo, "targets/legacy/sdk.yml", "gits: []\n");

        let targets = discover_targets(&repo).unwrap();
        assert_eq!(target_names(&targets), ["docs", "h7"]);
        assert_eq!(targets[1].manifest_path, PathBuf::from("boards/h7/sdk.yml"));

        let selected = resolve_manifest_path(&repo, "docs", ManifestSelection::Discover).unwrap();
        assert_eq!(selected, PathBuf::from("tools/docs/sdk.yml"));
        let err = resolve_manifest_path(&repo, "nope", ManifestSelection::Discover)
            .unwrap_err()
            .to_string();
        assert!(err.contains("available targets: docs, h7"), "{err}");
    }

    #[test]
    fn test_discover_targets_legacy_layout() {
        let (_temp_dir, repo) = create_test_workspace();
        write_file(&repo, "targets/b/sdk.yml", "gits: []\n");
        write_file(&repo, "targets/a/sdk.yml", "gits: []\n");
        fs::create_dir_all(repo.join("targets/empty")).unwrap();

        let targets = discover_targets(&repo).unwrap();
        assert_eq!(target_names(&targets), ["a", "b"]);
        assert_eq!(targets[0].manifest_path, PathBuf::from("targets/a/sdk.yml"));

        let (_temp_dir2, no_targets) = create_test_workspace();
        assert!(discover_targets(&no_targets).is_err());
    }

    #[test]
    fn test_project_config_validation() {
        let (_temp_dir, repo) = create_test_workspace();
        for (content, expected) in [
            ("{}\n", "must be set"),
            ("targets: {}\n", "must be set"),
            ("members: []\n", "must be set"),
            ("manifest: a/sdk.yml\n", "unknown field"),
            ("targets:\n  x: b/sdk.yml\nname: n\n", "unknown field"),
            ("members: [\"../x\"]\n", "relative to the source repository"),
            (
                "members: [\"/abs/*\"]\n",
                "relative to the source repository",
            ),
            ("members: [\"a[\"]\n", "Invalid member pattern"),
            (
                "targets:\n  x: ../sdk.yml\n",
                "relative to the source repository",
            ),
            ("targets:\n  x: b/other.yml\n", "must point to sdk.yml"),
            ("targets:\n  \"a/b\": b/sdk.yml\n", "Invalid target name"),
        ] {
            write_file(&repo, PROJECT_CONFIG_FILE, content);
            let err = discover_targets(&repo).unwrap_err().to_string();
            assert!(err.contains(expected), "{content:?} -> {err}");
        }
    }

    #[test]
    fn test_discover_targets_from_members_globs() {
        let (_temp_dir, repo) = create_test_workspace();
        write_file(&repo, "boards/h7/sdk.yml", "gits: []\n");
        write_file(&repo, "boards/f4/sdk.yml", "gits: []\n");
        write_file(&repo, "tools/ci/deep/sdk.yml", "gits: []\n");
        // A directory without sdk.yml is not a target
        fs::create_dir_all(repo.join("boards/empty")).unwrap();
        write_file(&repo, "tools/lint/sdk.yml", "gits: []\n");
        write_file(
            &repo,
            PROJECT_CONFIG_FILE,
            "members: [\"boards/*\", \"tools/**\"]\ntargets:\n  linter: tools/lint/sdk.yml\n",
        );

        let targets = discover_targets(&repo).unwrap();
        // tools/lint is both matched and listed explicitly under another name
        assert_eq!(
            target_names(&targets),
            ["deep", "f4", "h7", "lint", "linter"]
        );
        let deep = targets.iter().find(|t| t.name == "deep").unwrap();
        assert_eq!(deep.manifest_path, PathBuf::from("tools/ci/deep/sdk.yml"));

        assert_eq!(
            resolve_manifest_path(&repo, "h7", ManifestSelection::Discover).unwrap(),
            PathBuf::from("boards/h7/sdk.yml")
        );
        let err = resolve_manifest_path(&repo, "nope", ManifestSelection::Discover)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("available targets: deep, f4, h7, lint, linter"),
            "{err}"
        );
    }

    #[test]
    fn test_members_root_match_is_rejected() {
        let (_temp_dir, repo) = create_test_workspace();
        write_file(&repo, "sdk.yml", "gits: []\n");
        write_file(&repo, PROJECT_CONFIG_FILE, "members: [\".\"]\n");
        let err = discover_targets(&repo).unwrap_err().to_string();
        assert!(err.contains("matches the repository root"), "{err}");
    }

    #[test]
    fn test_index_entry_with_missing_manifest_is_rejected() {
        let (_temp_dir, repo) = create_test_workspace();
        write_file(&repo, "real/sdk.yml", "gits: []\n");

        write_file(
            &repo,
            PROJECT_CONFIG_FILE,
            "targets:\n  real: real/sdk.yml\n  ghost: nowhere/sdk.yml\n",
        );
        let err = discover_targets(&repo).unwrap_err().to_string();
        assert!(err.contains("'ghost' points to nowhere/sdk.yml"), "{err}");
        // Initializing another target of the same broken index fails the same way
        assert!(resolve_manifest_path(&repo, "real", ManifestSelection::Discover).is_err());
    }

    #[test]
    fn test_members_duplicate_names_are_rejected() {
        let (_temp_dir, repo) = create_test_workspace();
        write_file(&repo, "boards/x/sdk.yml", "gits: []\n");
        write_file(&repo, "tools/x/sdk.yml", "gits: []\n");
        write_file(
            &repo,
            PROJECT_CONFIG_FILE,
            "members: [\"boards/*\", \"tools/*\"]\n",
        );
        let err = discover_targets(&repo).unwrap_err().to_string();
        assert!(err.contains("defined more than once"), "{err}");

        // An explicit entry colliding with a different matched manifest is also an error
        write_file(
            &repo,
            PROJECT_CONFIG_FILE,
            "members: [\"boards/*\"]\ntargets:\n  x: tools/x/sdk.yml\n",
        );
        let err = discover_targets(&repo).unwrap_err().to_string();
        assert!(err.contains("defined more than once"), "{err}");

        // The same manifest reached twice is fine
        write_file(
            &repo,
            PROJECT_CONFIG_FILE,
            "members: [\"boards/*\", \"boards/x\"]\ntargets:\n  x: boards/x/sdk.yml\n",
        );
        assert_eq!(target_names(&discover_targets(&repo).unwrap()), ["x"]);
    }

    #[cfg(unix)]
    #[test]
    fn test_members_symlink_outside_repository_is_rejected() {
        let (_temp_dir, repo) = create_test_workspace();
        let (_outside_temp, outside) = create_test_workspace();
        write_file(&outside, "evil/sdk.yml", "gits: []\n");
        fs::create_dir_all(repo.join("boards")).unwrap();
        std::os::unix::fs::symlink(outside.join("evil"), repo.join("boards/evil")).unwrap();
        write_file(&repo, PROJECT_CONFIG_FILE, "members: [\"boards/*\"]\n");
        let err = discover_targets(&repo).unwrap_err().to_string();
        assert!(err.contains("resolves outside"), "{err}");
    }
}
