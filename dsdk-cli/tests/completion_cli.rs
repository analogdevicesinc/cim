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

use std::process::{Command, Output};

fn cim(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cim"))
        .args(args)
        .output()
        .expect("Failed to run cim")
}

#[test]
fn completion_without_shell_prints_help_and_succeeds() {
    let output = cim(&["utils", "completion"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "stdout: {stdout}");
    for word in [
        "Usage: cim utils completion",
        "Shells",
        "bash",
        "zsh",
        "powershell",
    ] {
        assert!(stdout.contains(word), "help lacks {word}: {stdout}");
    }
    assert!(!stdout.contains("fish"), "help lists fish: {stdout}");
}

#[test]
fn completion_prints_only_the_script() {
    for shell in ["bash", "zsh", "powershell"] {
        let output = cim(&["utils", "completion", shell]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.code(), Some(0), "{shell}");
        assert!(
            output.stderr.is_empty(),
            "{shell} wrote to stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!stdout.trim().is_empty(), "{shell} printed nothing");
        if shell == "zsh" {
            assert!(stdout.starts_with("#compdef cim"));
        }
    }
}

#[test]
fn completion_rejects_fish() {
    let output = cim(&["utils", "completion", "fish"]);
    assert_ne!(output.status.code(), Some(0));
}

// On Windows `bash` may be the WSL launcher, which cannot see $CIM.
#[cfg(unix)]
#[test]
fn bash_completion_registers_when_evaled() {
    if Command::new("bash").arg("--version").output().is_err() {
        return; // bash not available on this host
    }
    let output = Command::new("bash")
        .args([
            "-c",
            r#"eval "$("$CIM" utils completion bash)" && complete -p cim"#,
        ])
        .env("CIM", env!("CARGO_BIN_EXE_cim"))
        .output()
        .expect("Failed to run bash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {stdout}; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("complete"), "{stdout}");
    assert!(stdout.trim_end().ends_with(" cim"), "{stdout}");
}

#[cfg(target_os = "linux")]
#[test]
fn completion_write_error_reports_cleanly() {
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("Failed to open /dev/full");
    let output = Command::new(env!("CARGO_BIN_EXE_cim"))
        .args(["utils", "completion", "bash"])
        .stdout(full)
        .output()
        .expect("Failed to run cim");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
    assert!(!stderr.trim().is_empty(), "no error message");
}

#[cfg(unix)]
#[test]
fn completion_broken_pipe_exits_quietly() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cim"))
        .args(["utils", "completion", "bash"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to run cim");
    drop(child.stdout.take()); // reader goes away before the script is written
    let output = child.wait_with_output().expect("Failed to wait for cim");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}
