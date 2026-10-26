use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn init_and_bootstrap_return_failure_for_missing_targets() {
    let temp_dir = tempdir().expect("Failed to create temp directory");
    let source = temp_dir.path().join("manifests");
    fs::create_dir_all(source.join("targets")).expect("Failed to create targets directory");

    for command in ["init", "bootstrap"] {
        let workspace = temp_dir.path().join(format!("workspace-{command}"));
        let output = Command::new(env!("CARGO_BIN_EXE_cim"))
            .args([
                command,
                "-t",
                "missing-target",
                "-s",
                source.to_str().expect("Source path is not valid UTF-8"),
                "-w",
                workspace
                    .to_str()
                    .expect("Workspace path is not valid UTF-8"),
                "--no-mirror",
                "--yes",
            ])
            .output()
            .expect("Failed to run cim");

        assert_eq!(
            output.status.code(),
            Some(1),
            "{command} should fail for a missing target; stdout: {}; stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
