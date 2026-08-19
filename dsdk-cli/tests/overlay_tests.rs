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

//! Integration tests for the extends:/overlay: merge engine (dsdk_cli::overlay).
//!
//! Merge model: a derived target's own sdk.yml may freely define brand-new
//! gits/toolchains/copy_files/install entries of its own, merged with the
//! base's list by concatenation. sdk.yml's own `overlay:` key only ever
//! contains remove:/modify: (no add: -- new entries always live directly in
//! sdk.yml). Merge order per section is: remove (from the base) -> combine
//! (base-after-removal + this target's own entries) -> modify (on the
//! combined result).

mod common;

use common::{create_complex_sdk_config, create_minimal_sdk_config};
use dsdk_cli::config::{
    CopyFileConfig, GitConfig, InstallConfig, MakefileInclude, MakefileIncludeConfig,
    ToolchainConfig,
};
use dsdk_cli::overlay::{
    apply_overlay, compute_owned_entries, merge_copy_files, merge_gits, merge_install,
    merge_toolchains, merge_variables, validate_dependencies, CopyFilePatch, CopyFilesOverlay,
    GitPatch, GitsOverlay, InstallOverlay, InstallPatch, OverlayConfig, ToolchainPatch,
    ToolchainsOverlay, VariablesOverlay,
};
use std::collections::HashMap;

fn new_git(name: &str, url: &str, commit: &str) -> GitConfig {
    GitConfig {
        name: name.to_string(),
        url: url.to_string(),
        commit: commit.to_string(),
        ..Default::default()
    }
}

fn new_toolchain(name: &str) -> ToolchainConfig {
    ToolchainConfig {
        name: Some(name.to_string()),
        url: "https://example.com/downloads".to_string(),
        destination: "toolchains/test".to_string(),
        strip_components: None,
        os: None,
        arch: None,
        sha256: None,
        mirror_destination: None,
        environment: None,
        post_install_commands: None,
    }
}

fn new_install(name: &str, depends_on: Option<Vec<&str>>) -> InstallConfig {
    new_install_with_gits(name, depends_on, None)
}

fn new_install_with_gits(
    name: &str,
    depends_on: Option<Vec<&str>>,
    depends_on_gits: Option<Vec<&str>>,
) -> InstallConfig {
    InstallConfig {
        name: name.to_string(),
        depends_on: depends_on.map(|v| v.into_iter().map(String::from).collect()),
        sentinel: None,
        commands: None,
        depends_on_gits: depends_on_gits.map(|v| v.into_iter().map(String::from).collect()),
    }
}

fn new_copy_file(dest: &str) -> CopyFileConfig {
    CopyFileConfig {
        source: format!("https://example.com/{}", dest),
        dest: dest.to_string(),
        cache: None,
        sha256: None,
        post_data: None,
        symlink: None,
    }
}

// ---------------------------------------------------------------------
// gits: merge tests
// ---------------------------------------------------------------------

#[test]
fn test_merge_gits_own_entries_are_concatenated_with_base() {
    let base = create_complex_sdk_config().gits;
    let own = vec![new_git(
        "drone-camera",
        "https://example.com/camera.git",
        "main",
    )];

    let merged = merge_gits(base, own, None).expect("merge should succeed");
    assert_eq!(merged.len(), 4);
    assert!(merged.iter().any(|g| g.name == "drone-camera"));
}

#[test]
fn test_merge_gits_own_entry_colliding_with_base_errors() {
    let base = create_complex_sdk_config().gits;
    let own = vec![new_git("middleware", "https://example.com/dup.git", "main")];

    let err = merge_gits(base, own, None).unwrap_err();
    assert!(err.contains("middleware"));
    assert!(err.contains("already exists"));
}

#[test]
fn test_merge_gits_overlay_remove() {
    let base = create_complex_sdk_config().gits;
    let overlay = GitsOverlay {
        remove: vec!["application".to_string()],
        modify: vec![],
    };

    let merged = merge_gits(base, vec![], Some(&overlay)).expect("merge should succeed");
    assert_eq!(merged.len(), 2);
    assert!(!merged.iter().any(|g| g.name == "application"));
}

#[test]
fn test_merge_gits_overlay_modify_overrides_only_patched_fields() {
    let base = create_complex_sdk_config().gits;
    let overlay = GitsOverlay {
        remove: vec![],
        modify: vec![GitPatch {
            name: "middleware".to_string(),
            url: None,
            commit: Some("v2.0.0".to_string()),
            build_depends_on: None,
            git_depends_on: None,
            build: None,
            documentation_dir: None,
            python_deps: None,
            group: None,
        }],
    };

    let merged = merge_gits(base, vec![], Some(&overlay)).expect("merge should succeed");
    let middleware = merged.iter().find(|g| g.name == "middleware").unwrap();
    assert_eq!(middleware.commit, "v2.0.0");
    // Unpatched fields preserved
    assert_eq!(middleware.url, "https://github.com/example/middleware.git");
    assert_eq!(
        middleware.build_depends_on,
        Some(vec!["base-lib".to_string()])
    );
}

#[test]
fn test_merge_gits_overlay_modify_can_target_own_new_entry() {
    let base = create_complex_sdk_config().gits;
    let own = vec![new_git(
        "drone-camera",
        "https://example.com/camera.git",
        "main",
    )];
    let overlay = GitsOverlay {
        remove: vec![],
        modify: vec![GitPatch {
            name: "drone-camera".to_string(),
            url: None,
            commit: Some("v1.2.3".to_string()),
            build_depends_on: None,
            git_depends_on: None,
            build: None,
            documentation_dir: None,
            python_deps: None,
            group: None,
        }],
    };

    let merged = merge_gits(base, own, Some(&overlay)).expect("merge should succeed");
    let camera = merged.iter().find(|g| g.name == "drone-camera").unwrap();
    assert_eq!(camera.commit, "v1.2.3");
}

#[test]
fn test_merge_gits_overlay_remove_missing_errors() {
    let base = create_complex_sdk_config().gits;
    let overlay = GitsOverlay {
        remove: vec!["does-not-exist".to_string()],
        modify: vec![],
    };

    let err = merge_gits(base, vec![], Some(&overlay)).unwrap_err();
    assert!(err.contains("does-not-exist"));
    assert!(err.contains("not found in base"));
}

#[test]
fn test_merge_gits_overlay_modify_missing_errors() {
    let base = create_complex_sdk_config().gits;
    let overlay = GitsOverlay {
        remove: vec![],
        modify: vec![GitPatch {
            name: "does-not-exist".to_string(),
            url: None,
            commit: Some("v2.0.0".to_string()),
            build_depends_on: None,
            git_depends_on: None,
            build: None,
            documentation_dir: None,
            python_deps: None,
            group: None,
        }],
    };

    let err = merge_gits(base, vec![], Some(&overlay)).unwrap_err();
    assert!(err.contains("does-not-exist"));
}

#[test]
fn test_merge_gits_remove_then_own_redefine_allows_replacement() {
    // Removing a base entry and then defining a new sdk.yml entry with the
    // same name (but different data) should succeed, since remove is
    // applied to the base before combining with this target's own entries.
    let base = create_complex_sdk_config().gits;
    let own = vec![new_git(
        "middleware",
        "https://example.com/replaced.git",
        "main",
    )];
    let overlay = GitsOverlay {
        remove: vec!["middleware".to_string()],
        modify: vec![],
    };

    let merged = merge_gits(base, own, Some(&overlay)).expect("merge should succeed");
    let middleware = merged.iter().find(|g| g.name == "middleware").unwrap();
    assert_eq!(middleware.url, "https://example.com/replaced.git");
}

#[test]
fn test_merge_gits_none_overlay_and_no_own_is_noop() {
    let base = create_complex_sdk_config().gits;
    let base_len = base.len();
    let merged = merge_gits(base, vec![], None).expect("merge should succeed");
    assert_eq!(merged.len(), base_len);
}

// ---------------------------------------------------------------------
// toolchains: merge tests
// ---------------------------------------------------------------------

#[test]
fn test_merge_toolchains_own_remove_modify() {
    let base = Some(vec![new_toolchain("a"), new_toolchain("b")]);
    let own = Some(vec![new_toolchain("c")]);
    let overlay = ToolchainsOverlay {
        remove: vec!["a".to_string()],
        modify: vec![ToolchainPatch {
            name: "b".to_string(),
            url: None,
            destination: Some("toolchains/patched".to_string()),
            strip_components: None,
            os: None,
            arch: None,
            sha256: None,
            mirror_destination: None,
            environment: None,
            post_install_commands: None,
        }],
    };

    let merged = merge_toolchains(base, own, Some(&overlay))
        .expect("merge should succeed")
        .unwrap();
    assert_eq!(merged.len(), 2);
    assert!(!merged.iter().any(|t| t.get_name() == "a"));
    assert!(merged.iter().any(|t| t.get_name() == "c"));
    let patched = merged.iter().find(|t| t.get_name() == "b").unwrap();
    assert_eq!(patched.destination, "toolchains/patched");
}

#[test]
fn test_merge_toolchains_own_collision_with_base_errors() {
    let base = Some(vec![new_toolchain("a")]);
    let own = Some(vec![new_toolchain("a")]);

    let err = merge_toolchains(base, own, None).unwrap_err();
    assert!(err.contains('a'));
    assert!(err.contains("already exists"));
}

// ---------------------------------------------------------------------
// install: merge tests
// ---------------------------------------------------------------------

#[test]
fn test_merge_install_own_remove_modify() {
    let base = Some(vec![new_install("a", None), new_install("b", None)]);
    let own = Some(vec![new_install("c", Some(vec!["b"]))]);
    let overlay = InstallOverlay {
        remove: vec!["a".to_string()],
        modify: vec![InstallPatch {
            name: "b".to_string(),
            depends_on: None,
            sentinel: Some(true),
            commands: None,
        }],
    };

    let merged = merge_install(base, own, Some(&overlay))
        .expect("merge should succeed")
        .unwrap();
    assert_eq!(merged.len(), 2);
    let b = merged.iter().find(|i| i.name == "b").unwrap();
    assert_eq!(b.sentinel, Some(true));
    assert_eq!(b.sentinel_path().as_deref(), Some(".cim/b-installed"));
    let c = merged.iter().find(|i| i.name == "c").unwrap();
    assert_eq!(c.depends_on, Some(vec!["b".to_string()]));
}

// ---------------------------------------------------------------------
// copy_files: merge tests (keyed by dest)
// ---------------------------------------------------------------------

#[test]
fn test_merge_copy_files_own_remove_modify() {
    let base = Some(vec![
        new_copy_file("patches/a.patch"),
        new_copy_file("patches/b.patch"),
    ]);
    let own = Some(vec![new_copy_file("patches/c.patch")]);
    let overlay = CopyFilesOverlay {
        remove: vec!["patches/a.patch".to_string()],
        modify: vec![CopyFilePatch {
            dest: "patches/b.patch".to_string(),
            source: Some("https://example.com/new-b".to_string()),
            cache: Some(true),
            sha256: None,
            post_data: None,
            symlink: None,
        }],
    };

    let merged = merge_copy_files(base, own, Some(&overlay))
        .expect("merge should succeed")
        .unwrap();
    assert_eq!(merged.len(), 2);
    assert!(!merged.iter().any(|c| c.dest == "patches/a.patch"));
    let b = merged.iter().find(|c| c.dest == "patches/b.patch").unwrap();
    assert_eq!(b.source, "https://example.com/new-b");
    assert_eq!(b.cache, Some(true));
}

#[test]
fn test_merge_copy_files_remove_missing_errors() {
    let base = Some(vec![new_copy_file("patches/a.patch")]);
    let overlay = CopyFilesOverlay {
        remove: vec!["patches/missing.patch".to_string()],
        modify: vec![],
    };

    let err = merge_copy_files(base, None, Some(&overlay)).unwrap_err();
    assert!(err.contains("patches/missing.patch"));
}

// ---------------------------------------------------------------------
// variables: merge tests
// ---------------------------------------------------------------------

#[test]
fn test_merge_variables_set_and_remove() {
    let mut base = HashMap::new();
    base.insert("ZEPHYR_BOARD".to_string(), "board-a".to_string());
    base.insert("KEEP_ME".to_string(), "value".to_string());

    let mut set = HashMap::new();
    set.insert("ZEPHYR_BOARD".to_string(), "board-b".to_string());
    set.insert("NEW_VAR".to_string(), "new-value".to_string());

    let overlay = VariablesOverlay {
        set,
        remove: vec!["KEEP_ME".to_string()],
    };

    let merged = merge_variables(Some(base), None, Some(&overlay))
        .expect("merge should succeed")
        .unwrap();
    assert_eq!(merged.get("ZEPHYR_BOARD"), Some(&"board-b".to_string()));
    assert_eq!(merged.get("NEW_VAR"), Some(&"new-value".to_string()));
    assert!(!merged.contains_key("KEEP_ME"));
}

#[test]
fn test_merge_variables_own_upserts_base() {
    let mut base = HashMap::new();
    base.insert("ZEPHYR_BOARD".to_string(), "board-a".to_string());

    let mut own = HashMap::new();
    own.insert("ZEPHYR_BOARD".to_string(), "board-own".to_string());
    own.insert("OWN_VAR".to_string(), "own-value".to_string());

    let merged = merge_variables(Some(base), Some(own), None)
        .expect("merge should succeed")
        .unwrap();
    assert_eq!(merged.get("ZEPHYR_BOARD"), Some(&"board-own".to_string()));
    assert_eq!(merged.get("OWN_VAR"), Some(&"own-value".to_string()));
}

#[test]
fn test_merge_variables_remove_missing_errors() {
    let overlay = VariablesOverlay {
        set: HashMap::new(),
        remove: vec!["DOES_NOT_EXIST".to_string()],
    };

    let err = merge_variables(None, None, Some(&overlay)).unwrap_err();
    assert!(err.contains("DOES_NOT_EXIST"));
}

// ---------------------------------------------------------------------
// apply_overlay: full SdkConfig merge tests
// ---------------------------------------------------------------------

#[test]
fn test_apply_overlay_merges_own_entries_and_overrides_scalars() {
    let base = create_complex_sdk_config();
    let mut derived = create_minimal_sdk_config();
    derived.build_folder = Some("custom-build".to_string());
    derived.gits = vec![new_git(
        "drone-camera",
        "https://example.com/camera.git",
        "main",
    )];

    let overlay = OverlayConfig {
        gits: Some(GitsOverlay {
            remove: vec!["application".to_string()],
            modify: vec![],
        }),
        toolchains: None,
        install: None,
        copy_files: None,
        variables: None,
    };

    let merged = apply_overlay(base, derived, &overlay).expect("apply_overlay should succeed");
    assert_eq!(merged.gits.len(), 3);
    assert!(merged.gits.iter().any(|g| g.name == "drone-camera"));
    assert!(!merged.gits.iter().any(|g| g.name == "application"));
    assert_eq!(merged.build_folder.as_deref(), Some("custom-build"));
    assert!(merged.extends.is_none());
}

#[test]
fn test_apply_overlay_own_collision_with_base_errors() {
    let base = create_complex_sdk_config();
    let mut derived = create_minimal_sdk_config();
    derived.gits = vec![new_git("middleware", "https://example.com/x.git", "main")];

    let err = apply_overlay(base, derived, &OverlayConfig::default()).unwrap_err();
    assert!(err.contains("middleware"));
    assert!(err.contains("already exists"));
}

#[test]
fn test_apply_overlay_missing_overlay_is_noop_on_lists() {
    let base = create_complex_sdk_config();
    let derived = create_minimal_sdk_config();
    let base_len = base.gits.len();

    let merged = apply_overlay(base, derived, &OverlayConfig::default())
        .expect("apply_overlay should succeed");
    assert_eq!(merged.gits.len(), base_len);
}

// ---------------------------------------------------------------------
// apply_overlay: makefile_include is merged additively, not overridden
// ---------------------------------------------------------------------

#[test]
fn test_apply_overlay_merges_makefile_include_files_and_exclude() {
    let mut base = create_complex_sdk_config();
    base.makefile_include = Some(MakefileInclude::Structured(MakefileIncludeConfig {
        files: vec!["include base.mk".to_string()],
        exclude: vec!["qemu".to_string()],
    }));
    let mut derived = create_minimal_sdk_config();
    derived.makefile_include = Some(MakefileInclude::Structured(MakefileIncludeConfig {
        files: vec!["include team-a.mk".to_string()],
        exclude: vec!["trusted-services".to_string()],
    }));

    let merged = apply_overlay(base, derived, &OverlayConfig::default())
        .expect("apply_overlay should succeed");
    let mi = merged.makefile_include.expect("makefile_include missing");
    assert_eq!(
        mi.files(),
        &[
            "include base.mk".to_string(),
            "include team-a.mk".to_string()
        ]
    );
    assert_eq!(
        mi.exclude(),
        &["qemu".to_string(), "trusted-services".to_string()]
    );
}

#[test]
fn test_apply_overlay_makefile_include_dedups_repeated_entries() {
    let mut base = create_complex_sdk_config();
    base.makefile_include = Some(MakefileInclude::Structured(MakefileIncludeConfig {
        files: vec!["include shared.mk".to_string()],
        exclude: vec!["qemu".to_string()],
    }));
    let mut derived = create_minimal_sdk_config();
    derived.makefile_include = Some(MakefileInclude::Structured(MakefileIncludeConfig {
        files: vec!["include shared.mk".to_string()],
        exclude: vec!["qemu".to_string()],
    }));

    let merged = apply_overlay(base, derived, &OverlayConfig::default())
        .expect("apply_overlay should succeed");
    let mi = merged.makefile_include.expect("makefile_include missing");
    assert_eq!(mi.files(), &["include shared.mk".to_string()]);
    assert_eq!(mi.exclude(), &["qemu".to_string()]);
}

#[test]
fn test_apply_overlay_makefile_include_one_sided_is_preserved() {
    let mut base = create_complex_sdk_config();
    base.makefile_include = Some(MakefileInclude::Legacy(vec!["include base.mk".to_string()]));
    let derived = create_minimal_sdk_config();

    let merged = apply_overlay(base, derived, &OverlayConfig::default())
        .expect("apply_overlay should succeed");
    let mi = merged.makefile_include.expect("makefile_include missing");
    assert_eq!(mi.files(), &["include base.mk".to_string()]);
    assert!(mi.exclude().is_empty());
}

// ---------------------------------------------------------------------
// validate_dependencies tests
// ---------------------------------------------------------------------

#[test]
fn test_validate_dependencies_ok() {
    let config = create_complex_sdk_config();
    assert!(validate_dependencies(&config).is_ok());
}

#[test]
fn test_validate_dependencies_allows_phase_target_in_build_depends_on() {
    // build_depends_on is emitted verbatim as a Makefile prerequisite, so
    // besides other git names it may legitimately reference a phase target
    // like "sdk-envsetup" (a real, documented, pre-existing manifest
    // pattern -- see the "example" target in cim-manifests).
    let mut config = create_minimal_sdk_config();
    config.gits = vec![GitConfig {
        name: "git-sandbox".to_string(),
        url: "https://example.com/git-sandbox.git".to_string(),
        commit: "master".to_string(),
        build_depends_on: Some(vec!["sdk-envsetup".to_string()]),
        ..Default::default()
    }];

    assert!(validate_dependencies(&config).is_ok());
}

#[test]
fn test_validate_dependencies_allows_install_target_in_build_depends_on() {
    let mut config = create_minimal_sdk_config();
    config.install = Some(vec![new_install("protoc", None)]);
    config.gits = vec![GitConfig {
        name: "app".to_string(),
        url: "https://example.com/app.git".to_string(),
        commit: "main".to_string(),
        build_depends_on: Some(vec!["install-protoc".to_string()]),
        ..Default::default()
    }];

    assert!(validate_dependencies(&config).is_ok());
}

#[test]
fn test_validate_dependencies_still_rejects_truly_unknown_target() {
    let mut config = create_minimal_sdk_config();
    config.gits = vec![GitConfig {
        name: "app".to_string(),
        url: "https://example.com/app.git".to_string(),
        commit: "main".to_string(),
        build_depends_on: Some(vec!["totally-made-up-target".to_string()]),
        ..Default::default()
    }];

    let err = validate_dependencies(&config).unwrap_err();
    assert!(err.contains("totally-made-up-target"));
}

#[test]
fn test_validate_dependencies_dangling_build_depends_on() {
    let mut config = create_minimal_sdk_config();
    config.gits = vec![GitConfig {
        name: "app".to_string(),
        url: "https://example.com/app.git".to_string(),
        commit: "main".to_string(),
        build_depends_on: Some(vec!["removed-git".to_string()]),
        ..Default::default()
    }];

    let err = validate_dependencies(&config).unwrap_err();
    assert!(err.contains("app"));
    assert!(err.contains("removed-git"));
}

#[test]
fn test_validate_dependencies_dangling_install_depends_on() {
    let mut config = create_minimal_sdk_config();
    config.install = Some(vec![new_install("c", Some(vec!["missing-install"]))]);

    let err = validate_dependencies(&config).unwrap_err();
    assert!(err.contains("missing-install"));
}

#[test]
fn test_validate_dependencies_allows_known_git_in_depends_on_gits() {
    let mut config = create_minimal_sdk_config();
    config.gits = vec![new_git("app", "https://example.com/app.git", "main")];
    config.install = Some(vec![new_install_with_gits(
        "app-python-deps",
        None,
        Some(vec!["app"]),
    )]);

    assert!(validate_dependencies(&config).is_ok());
}

#[test]
fn test_validate_dependencies_dangling_depends_on_gits() {
    let mut config = create_minimal_sdk_config();
    config.install = Some(vec![new_install_with_gits(
        "app-python-deps",
        None,
        Some(vec!["missing-git"]),
    )]);

    let err = validate_dependencies(&config).unwrap_err();
    assert!(err.contains("app-python-deps"));
    assert!(err.contains("missing-git"));
}

// ---------------------------------------------------------------------
// compute_owned_entries tests
// ---------------------------------------------------------------------

#[test]
fn test_compute_owned_entries_across_all_sections() {
    let mut derived = create_minimal_sdk_config();
    derived.gits = vec![new_git(
        "drone-camera",
        "https://example.com/camera.git",
        "main",
    )];
    derived.toolchains = Some(vec![new_toolchain("gcc-drone")]);
    derived.install = Some(vec![new_install("overlay-greeting", None)]);
    derived.copy_files = Some(vec![new_copy_file("patches/drone.patch")]);

    let overlay = OverlayConfig {
        gits: Some(GitsOverlay {
            remove: vec!["mcuboot".to_string()],
            modify: vec![GitPatch {
                name: "zephyr".to_string(),
                url: None,
                commit: Some("v4.5.0".to_string()),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
                python_deps: None,
                group: None,
            }],
        }),
        toolchains: None,
        install: Some(InstallOverlay {
            remove: vec![],
            modify: vec![InstallPatch {
                name: "protoc".to_string(),
                depends_on: None,
                sentinel: None,
                commands: None,
            }],
        }),
        copy_files: None,
        variables: None,
    };

    let owned = compute_owned_entries(&derived, &overlay);

    // Own new gits + overlay-modified gits are both "owned".
    assert_eq!(owned.gits.len(), 2);
    assert!(owned.gits.contains("drone-camera"));
    assert!(owned.gits.contains("zephyr"));
    assert!(!owned.gits.contains("mcuboot")); // remove: doesn't count as "owned"

    assert_eq!(owned.toolchains.len(), 1);
    assert!(owned.toolchains.contains("gcc-drone"));

    // Own new install target + overlay-modified install target.
    assert_eq!(owned.install.len(), 2);
    assert!(owned.install.contains("overlay-greeting"));
    assert!(owned.install.contains("protoc"));

    assert_eq!(owned.copy_files.len(), 1);
    assert!(owned.copy_files.contains("patches/drone.patch"));
}

#[test]
fn test_compute_owned_entries_default_overlay_and_minimal_derived_is_empty() {
    let derived = create_minimal_sdk_config();
    let owned = compute_owned_entries(&derived, &OverlayConfig::default());
    assert!(owned.gits.is_empty());
    assert!(owned.toolchains.is_empty());
    assert!(owned.install.is_empty());
    assert!(owned.copy_files.is_empty());
}
