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

//! Host OS/architecture detection and the shared `os:`/`arch:` filter used
//! by every sdk.yml section that can be restricted to a given host.

use crate::config::ToolchainConfig;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::Command;

/// Architecture detection result
#[derive(Debug, Clone, PartialEq)]
pub struct HostInfo {
    pub os: String,
    pub arch: String,
}

/// Detect host OS and architecture
pub fn detect_host_info() -> HostInfo {
    let os = if cfg!(target_os = "macos") {
        "darwin".to_string()
    } else if cfg!(target_os = "linux") {
        "linux".to_string()
    } else if cfg!(target_os = "windows") {
        "windows".to_string()
    } else {
        "unknown".to_string()
    };

    // Detect architecture at runtime using system calls
    let arch = detect_runtime_arch();

    HostInfo { os, arch }
}

/// An sdk.yml entry that can be restricted to a host OS (and optionally
/// architecture). An unset filter matches every host.
pub trait HostFilter {
    fn os(&self) -> Option<&str>;

    fn arch(&self) -> Option<&str> {
        None
    }
}

/// Check whether an entry applies to the given host OS and architecture
pub fn is_applicable<T: HostFilter + ?Sized>(entry: &T, host_info: &HostInfo) -> bool {
    entry.os().is_none_or(|os| os == host_info.os)
        && entry.arch().is_none_or(|arch| arch == host_info.arch)
}

impl HostFilter for ToolchainConfig {
    fn os(&self) -> Option<&str> {
        self.os.as_deref()
    }

    fn arch(&self) -> Option<&str> {
        self.arch.as_deref()
    }
}

/// Detect the actual runtime architecture using platform-specific methods
fn detect_runtime_arch() -> String {
    // Platform-specific runtime architecture detection
    #[cfg(target_os = "macos")]
    {
        detect_macos_runtime_arch()
    }

    #[cfg(target_os = "linux")]
    {
        detect_linux_runtime_arch()
    }

    #[cfg(target_os = "windows")]
    {
        detect_windows_runtime_arch()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        // Fallback to compile-time detection for unsupported platforms
        detect_compile_time_arch()
    }
}

/// Detect runtime architecture on macOS
#[cfg(target_os = "macos")]
fn detect_macos_runtime_arch() -> String {
    // On macOS, check if we're running under Rosetta emulation
    // If ARCHPREFERENCE is set to "i386,x86_64", we're running x86_64 code on ARM64
    if let Ok(arch_pref) = std::env::var("ARCHPREFERENCE") {
        if arch_pref.contains("i386") || arch_pref.contains("x86_64") {
            // We're likely running under Rosetta, but we want the actual hardware arch
            // Try to get the actual hardware architecture via sysctl
            if let Some(hw_arch) = get_macos_hw_arch() {
                return normalize_arch(&hw_arch);
            }
        }
    }

    // Try to read hardware architecture directly
    if let Some(hw_arch) = get_macos_hw_arch() {
        return normalize_arch(&hw_arch);
    }

    // Fallback to compile-time detection
    detect_compile_time_arch()
}

/// Get macOS hardware architecture using sysctl (if available)
#[cfg(target_os = "macos")]
fn get_macos_hw_arch() -> Option<String> {
    // Try sysctl hw.optional.arm64 to detect Apple Silicon
    if let Ok(output) = Command::new("sysctl")
        .arg("-n")
        .arg("hw.optional.arm64")
        .output()
    {
        if output.status.success() {
            let result_raw = String::from_utf8_lossy(&output.stdout);
            let result = result_raw.trim();
            if result == "1" {
                return Some("arm64".to_string());
            }
        }
    }

    // Try sysctl hw.machine to get machine type
    if let Ok(output) = Command::new("sysctl").arg("-n").arg("hw.machine").output() {
        if output.status.success() {
            let machine_raw = String::from_utf8_lossy(&output.stdout);
            let machine = machine_raw.trim();
            return Some(machine.to_string());
        }
    }

    None
}

/// Detect runtime architecture on Linux
#[cfg(target_os = "linux")]
fn detect_linux_runtime_arch() -> String {
    // Try reading from /proc/cpuinfo
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        // Look for architecture indicators in cpuinfo
        if cpuinfo.contains("aarch64") || cpuinfo.contains("arm64") {
            return "arm64".to_string();
        }
        if cpuinfo.contains("x86_64") || cpuinfo.contains("AMD64") {
            return "x86_64".to_string();
        }
        // Look for ARM indicators
        if cpuinfo.contains("ARM") || cpuinfo.contains("arm") {
            return "arm64".to_string(); // Modern ARM is typically arm64
        }
    }

    // Try uname -m as fallback (only on Unix-like systems)
    if let Ok(output) = Command::new("uname").arg("-m").output() {
        if output.status.success() {
            let arch_raw = String::from_utf8_lossy(&output.stdout);
            let arch_str = arch_raw.trim();
            return normalize_arch(arch_str);
        }
    }

    // Fallback to compile-time detection
    detect_compile_time_arch()
}

/// Detect runtime architecture on Windows
#[cfg(target_os = "windows")]
fn detect_windows_runtime_arch() -> String {
    // Check PROCESSOR_ARCHITEW6432 first (indicates running 32-bit on 64-bit)
    if let Ok(arch) = std::env::var("PROCESSOR_ARCHITEW6432") {
        return normalize_arch(&arch);
    }

    // Check PROCESSOR_ARCHITECTURE
    if let Ok(arch) = std::env::var("PROCESSOR_ARCHITECTURE") {
        return normalize_arch(&arch);
    }

    // Fallback to compile-time detection
    detect_compile_time_arch()
}

/// Normalize architecture name to standard format
fn normalize_arch(arch_str: &str) -> String {
    match arch_str.to_lowercase().as_str() {
        "x86_64" | "amd64" | "x64" => "x86_64".to_string(),
        "aarch64" | "arm64" | "arm64ec" => "arm64".to_string(),
        "i386" | "i686" | "x86" => "i386".to_string(),
        "arm14t2" | "arm64t2" => "arm64".to_string(), // Apple Silicon variants
        other => other.to_string(),
    }
}

/// Fallback compile-time architecture detection
fn detect_compile_time_arch() -> String {
    if cfg!(target_arch = "x86_64") {
        "x86_64".to_string()
    } else if cfg!(target_arch = "aarch64") {
        "arm64".to_string()
    } else if cfg!(target_arch = "x86") {
        "i386".to_string()
    } else {
        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Entry {
        os: Option<&'static str>,
        arch: Option<&'static str>,
    }

    impl HostFilter for Entry {
        fn os(&self) -> Option<&str> {
            self.os
        }

        fn arch(&self) -> Option<&str> {
            self.arch
        }
    }

    fn linux_x86_64() -> HostInfo {
        HostInfo {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        }
    }

    #[test]
    fn test_is_applicable_no_filters() {
        let entry = Entry {
            os: None,
            arch: None,
        };
        assert!(is_applicable(&entry, &linux_x86_64()));
    }

    #[test]
    fn test_is_applicable_matching_filters() {
        let entry = Entry {
            os: Some("linux"),
            arch: Some("x86_64"),
        };
        assert!(is_applicable(&entry, &linux_x86_64()));
    }

    #[test]
    fn test_is_applicable_mismatched_os() {
        let entry = Entry {
            os: Some("windows"),
            arch: None,
        };
        assert!(!is_applicable(&entry, &linux_x86_64()));
    }

    #[test]
    fn test_is_applicable_mismatched_arch() {
        let entry = Entry {
            os: None,
            arch: Some("arm64"),
        };
        assert!(!is_applicable(&entry, &linux_x86_64()));
    }

    #[test]
    fn test_host_info_detection() {
        let host_info = detect_host_info();

        // The actual values will depend on the platform running the test
        assert!(!host_info.os.is_empty(), "OS should be detected");
        assert!(
            !host_info.arch.is_empty(),
            "Architecture should be detected"
        );

        // Check that the values are expected ones
        assert!(
            matches!(
                host_info.os.as_str(),
                "darwin" | "linux" | "windows" | "unknown"
            ),
            "OS should be one of the expected values"
        );
        assert!(
            matches!(
                host_info.arch.as_str(),
                "x86_64" | "arm64" | "i386" | "unknown"
            ),
            "Architecture should be one of the expected values"
        );
    }

    #[test]
    fn test_runtime_arch_detection() {
        let runtime_arch = detect_runtime_arch();

        // Architecture should be detected (either runtime or compile-time fallback)
        assert!(
            !runtime_arch.is_empty(),
            "Runtime architecture should be detected"
        );

        // Should be one of the standard architecture names
        assert!(
            matches!(
                runtime_arch.as_str(),
                "x86_64" | "arm64" | "i386" | "unknown"
            ),
            "Runtime architecture should be normalized to standard format, got: {}",
            runtime_arch
        );

        // Verify that runtime detection produces consistent results
        let second_detection = detect_runtime_arch();
        assert_eq!(
            runtime_arch, second_detection,
            "Runtime detection should be consistent across calls"
        );

        // Verify that normalize_arch works correctly
        assert_eq!(normalize_arch("x86_64"), "x86_64");
        assert_eq!(normalize_arch("amd64"), "x86_64");
        assert_eq!(normalize_arch("aarch64"), "arm64");
        assert_eq!(normalize_arch("arm64"), "arm64");
        assert_eq!(normalize_arch("i386"), "i386");
        assert_eq!(normalize_arch("i686"), "i386");
    }

    #[test]
    fn test_normalize_arch() {
        // Test x86_64 variants
        assert_eq!(normalize_arch("x86_64"), "x86_64");
        assert_eq!(normalize_arch("amd64"), "x86_64");
        assert_eq!(normalize_arch("x64"), "x86_64");
        assert_eq!(normalize_arch("AMD64"), "x86_64");

        // Test ARM64 variants
        assert_eq!(normalize_arch("aarch64"), "arm64");
        assert_eq!(normalize_arch("arm64"), "arm64");
        assert_eq!(normalize_arch("ARM64"), "arm64");
        assert_eq!(normalize_arch("arm64ec"), "arm64");

        // Test x86 variants
        assert_eq!(normalize_arch("i386"), "i386");
        assert_eq!(normalize_arch("i686"), "i386");
        assert_eq!(normalize_arch("x86"), "i386");

        // Test unknown architecture
        assert_eq!(normalize_arch("riscv64"), "riscv64");
        assert_eq!(normalize_arch("unknown"), "unknown");
    }
}
