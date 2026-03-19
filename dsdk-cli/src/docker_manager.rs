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

use crate::config::{OsDependencies, PythonDependencies, SdkConfig};
use crate::messages;
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// URL type for git repository URLs
#[derive(Debug, PartialEq)]
enum UrlType {
    Ssh,
    Https,
    Git,
    Other,
}

/// Docker image configuration for different distributions
#[derive(Debug, Clone)]
pub struct DockerImage {
    pub name: String,
    pub tag: String,
    pub package_manager: String,
    pub update_command: String,
}

/// Configuration for Dockerfile creation
#[derive(Debug)]
pub struct DockerfileConfig<'a> {
    pub sdk_config: &'a SdkConfig,
    pub os_deps: &'a OsDependencies,
    pub python_deps: &'a PythonDependencies,
    pub output_path: &'a Path,
    pub distro_preference: Option<&'a str>,
    pub python_profile: &'a str,
    pub force: bool,
    pub force_https: bool,
    pub force_ssh: bool,
    pub no_mirror: bool,
}

/// Configuration for Dockerfile generation
#[derive(Debug)]
pub struct DockerfileGenerationConfig<'a> {
    pub sdk_config: &'a SdkConfig,
    pub os_deps: &'a OsDependencies,
    pub python_deps: &'a PythonDependencies,
    pub docker_image: &'a DockerImage,
    pub python_profile: &'a str,
    pub force_https: bool,
    pub force_ssh: bool,
    pub no_mirror: bool,
}

/// Docker manager for generating Dockerfiles and managing containerized SDK development
pub struct DockerManager {
    pub workspace_path: PathBuf,
    /// Directory containing config files (sdk.yml, os-dependencies.yml, patches/, etc.)
    /// Used to generate absolute COPY paths in Dockerfile
    pub config_dir: PathBuf,
}

impl DockerManager {
    /// Create a new Docker manager
    ///
    /// # Arguments
    /// * `workspace_path` - Path to cim source directory (for finding binary)
    /// * `config_dir` - Directory containing extracted config files (for COPY commands)
    pub fn new(workspace_path: PathBuf, config_dir: PathBuf) -> Self {
        Self {
            workspace_path,
            config_dir,
        }
    }

    /// Get available Docker images from OS dependencies
    pub fn get_available_images(&self, os_deps: &OsDependencies) -> Vec<DockerImage> {
        let mut images = Vec::new();

        // Determine the preferred OS key based on architecture
        let os_key = if cfg!(target_arch = "aarch64") {
            "linux-aarch64"
        } else {
            "linux-x86_64"
        };

        // Try architecture-specific config first
        if let Some(os_config) = os_deps.os_configs.get(os_key) {
            for (distro_key, distro_config) in &os_config.distros {
                // Parse distro key to extract name and version
                // New format: "ubuntu-22.04" -> name="ubuntu", tag="22.04"
                // Legacy format: "ubuntu" -> name="ubuntu", tag from version field
                let (name, tag) = if let Some((distro_name, version)) =
                    OsDependencies::parse_distro_key(distro_key)
                {
                    (distro_name, version)
                } else {
                    // TODO: Remove backward compatibility after migration period
                    // Legacy format: use key as name, version from field
                    (
                        distro_key.clone(),
                        distro_config
                            .version
                            .clone()
                            .unwrap_or_else(|| "latest".to_string()),
                    )
                };

                let image = DockerImage {
                    name,
                    tag,
                    package_manager: distro_config.package_manager.command.clone(),
                    update_command: Self::get_update_command(distro_key),
                };
                images.push(image);
            }
        }

        // Fall back to generic "linux" if architecture-specific config not found
        if images.is_empty() {
            if let Some(os_config) = os_deps.os_configs.get("linux") {
                for (distro_key, distro_config) in &os_config.distros {
                    // Parse distro key to extract name and version
                    let (name, tag) = if let Some((distro_name, version)) =
                        OsDependencies::parse_distro_key(distro_key)
                    {
                        (distro_name, version)
                    } else {
                        // TODO: Remove backward compatibility after migration period
                        // Legacy format: use key as name, version from field
                        (
                            distro_key.clone(),
                            distro_config
                                .version
                                .clone()
                                .unwrap_or_else(|| "latest".to_string()),
                        )
                    };

                    let image = DockerImage {
                        name,
                        tag,
                        package_manager: distro_config.package_manager.command.clone(),
                        update_command: Self::get_update_command(distro_key),
                    };
                    images.push(image);
                }
            }
        }

        images
    }

    /// Get the package update command for a distribution
    fn get_update_command(distro: &str) -> String {
        match distro {
            "ubuntu" | "debian" => "apt-get update".to_string(),
            "fedora" => "dnf update -y".to_string(),
            "centos" | "rhel" => "yum update -y".to_string(),
            "alpine" => "apk update".to_string(),
            _ => "echo 'Unknown package manager'".to_string(),
        }
    }

    /// Select the best Docker image based on user preference and availability
    pub fn select_docker_image(
        &self,
        user_preference: Option<&str>,
        available_images: &[DockerImage],
    ) -> Option<DockerImage> {
        // If user specified a preference, try to find it
        if let Some(pref) = user_preference {
            // Check if it's a full image name (name:tag)
            if let Some((name, tag)) = pref.split_once(':') {
                for image in available_images {
                    if image.name == name && image.tag == tag {
                        return Some(image.clone());
                    }
                }
            }

            // Check if it's just a distribution name
            for image in available_images {
                if image.name == pref {
                    return Some(image.clone());
                }
            }
        }

        // Default fallbacks in order of preference
        let fallbacks = ["ubuntu", "fedora", "debian"];

        for fallback in &fallbacks {
            for image in available_images {
                if image.name == *fallback {
                    return Some(image.clone());
                }
            }
        }

        // Return the first available image if no fallbacks match
        available_images.first().cloned()
    }

    /// Detect the type of git URL
    fn detect_url_type(url: &str) -> UrlType {
        if url.starts_with("git@") || url.starts_with("ssh://") {
            UrlType::Ssh
        } else if url.starts_with("https://") {
            UrlType::Https
        } else if url.starts_with("git://") {
            UrlType::Git
        } else {
            UrlType::Other
        }
    }

    /// Transform git URL based on protocol preference
    fn transform_git_url(url: &str, force_https: bool, force_ssh: bool) -> String {
        // If both flags are set, prefer HTTPS as it's more corporate-friendly
        if force_https && force_ssh {
            return Self::convert_to_https(url);
        }

        if force_https {
            return Self::convert_to_https(url);
        }

        if force_ssh {
            return Self::convert_to_ssh(url);
        }

        // No transformation needed
        url.to_string()
    }

    /// Convert any git URL to HTTPS format
    pub fn convert_to_https(url: &str) -> String {
        match Self::detect_url_type(url) {
            UrlType::Ssh => {
                // git@github.com:owner/repo.git -> https://github.com/owner/repo.git
                if url.starts_with("git@") {
                    if let Some(colon_pos) = url.find(':') {
                        let host = &url[4..colon_pos]; // Skip "git@"
                        let path = &url[colon_pos + 1..];
                        format!("https://{}/{}", host, path)
                    } else {
                        url.to_string()
                    }
                } else {
                    url.to_string()
                }
            }
            UrlType::Git => {
                // git://example.org/repo -> https://example.org/repo.git
                let https_url = url.replace("git://", "https://");
                if https_url.ends_with(".git") {
                    https_url
                } else {
                    format!("{}.git", https_url)
                }
            }
            _ => url.to_string(), // Already HTTPS or other format
        }
    }

    /// Convert any git URL to SSH format
    pub fn convert_to_ssh(url: &str) -> String {
        match Self::detect_url_type(url) {
            UrlType::Https => {
                // https://github.com/owner/repo.git -> git@github.com:owner/repo.git
                if let Some(without_protocol) = url.strip_prefix("https://") {
                    if let Some(slash_pos) = without_protocol.find('/') {
                        let host = &without_protocol[..slash_pos];
                        let path = &without_protocol[slash_pos + 1..];
                        format!("git@{}:{}", host, path)
                    } else {
                        url.to_string()
                    }
                } else {
                    url.to_string()
                }
            }
            UrlType::Git => {
                // git://example.org/repo -> git@example.org:repo.git
                if let Some(without_protocol) = url.strip_prefix("git://") {
                    if let Some(slash_pos) = without_protocol.find('/') {
                        let host = &without_protocol[..slash_pos];
                        let path = &without_protocol[slash_pos + 1..];
                        let path_with_git = if path.ends_with(".git") {
                            path.to_string()
                        } else {
                            format!("{}.git", path)
                        };
                        format!("git@{}:{}", host, path_with_git)
                    } else {
                        url.to_string()
                    }
                } else {
                    url.to_string()
                }
            }
            _ => url.to_string(), // Already SSH or other format
        }
    }

    /// Check if any repositories use SSH protocol
    pub fn has_ssh_repositories(sdk_config: &SdkConfig) -> bool {
        sdk_config
            .gits
            .iter()
            .any(|git| Self::detect_url_type(&git.url) == UrlType::Ssh)
    }

    /// Generate Dockerfile content
    pub fn generate_dockerfile(&self, config: &DockerfileGenerationConfig) -> io::Result<String> {
        let mut dockerfile = String::new();

        // FROM instruction
        dockerfile.push_str(&format!(
            "FROM {}:{}\n\n",
            config.docker_image.name, config.docker_image.tag
        ));

        // Metadata
        dockerfile.push_str("# Generated by cim\n");
        dockerfile
            .push_str("# This Dockerfile creates a containerized SDK development environment\n\n");

        // Update package manager
        dockerfile.push_str("# Update package manager\n");
        dockerfile.push_str(&format!("RUN {}\n\n", config.docker_image.update_command));

        // Install system packages
        dockerfile.push_str("# Install system dependencies\n");
        if let Some(packages) = self.get_system_packages(config.os_deps, &config.docker_image.name)
        {
            let install_cmd = format!("{} -y", config.docker_image.package_manager);
            dockerfile.push_str(&format!("RUN {} \\\n", install_cmd));

            for (i, package) in packages.iter().enumerate() {
                if i == packages.len() - 1 {
                    dockerfile.push_str(&format!("    {} && \\\n", package));
                } else {
                    dockerfile.push_str(&format!("    {} \\\n", package));
                }
            }

            // Add SSL certificate update for better corporate environment support
            dockerfile.push_str("    update-ca-certificates\n\n");
        }

        // Install Python packages
        dockerfile.push_str("# Install Python dependencies\n");
        if let Some(packages) = self.get_python_packages(config.python_deps, config.python_profile)
        {
            if !packages.is_empty() {
                // dockerfile.push_str("RUN pip3 install \\\n");
                dockerfile.push_str("RUN pip3 install --trusted-host pypi.org --trusted-host files.pythonhosted.org \\\n");
                for (i, package) in packages.iter().enumerate() {
                    if i == packages.len() - 1 {
                        dockerfile.push_str(&format!("    {}\n\n", package));
                    } else {
                        dockerfile.push_str(&format!("    {} \\\n", package));
                    }
                }
            }
        }

        // Set up workspace
        // Use workspace directory name from host configuration (e.g., dsdk-workspace)
        let workspace_dirname = self
            .workspace_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace");
        let container_workspace = format!("/root/{}", workspace_dirname);

        dockerfile.push_str("# Set up workspace\n");
        dockerfile.push_str(&format!("WORKDIR {}\n", container_workspace));
        dockerfile.push_str(&format!("ENV WORKSPACE={}\n\n", container_workspace));

        // Clone SDK repositories at build time
        dockerfile.push_str("# Clone SDK repositories\n");

        // Check if we have GitHub HTTPS repos that might need authentication
        let has_github_https = config.sdk_config.gits.iter().any(|git| {
            let url = Self::transform_git_url(&git.url, config.force_https, config.force_ssh);
            url.contains("github.com") && Self::detect_url_type(&url) == UrlType::Https
        });

        if has_github_https {
            dockerfile
                .push_str("# Note: For private GitHub HTTPS repos, use a personal access token:\n");
            dockerfile.push_str("# Method 1: Pass token as environment variable\n");
            dockerfile.push_str(
                "#   docker build --secret id=GIT_AUTH_TOKEN,env=GITHUB_TOKEN -t sdk-image .\n",
            );
            dockerfile.push_str("#\n");
            dockerfile.push_str("# Method 2: Export environment variable first\n");
            dockerfile.push_str("#   export GITHUB_TOKEN=<token>\n");
            dockerfile.push_str("#   docker build --secret id=GIT_AUTH_TOKEN -t sdk-image .\n");
            dockerfile.push_str("#\n");
        }

        // Add GitHub token support for HTTPS repos using BuildKit secrets
        if has_github_https {
            dockerfile.push_str(
                "# Configure git to use token for GitHub HTTPS URLs (using BuildKit secret)\n",
            );
            dockerfile.push_str("RUN --mount=type=secret,id=GIT_AUTH_TOKEN \\\n");
            dockerfile.push_str("    if [ -f /run/secrets/GIT_AUTH_TOKEN ]; then \\\n");
            dockerfile
                .push_str("        GIT_AUTH_TOKEN=$(cat /run/secrets/GIT_AUTH_TOKEN) && \\\n");
            dockerfile.push_str("        git config --global url.\"https://${GIT_AUTH_TOKEN}@github.com/\".insteadOf \"https://github.com/\"; \\\n");
            dockerfile.push_str("    fi\n\n");
        }

        for git_config in &config.sdk_config.gits {
            let transformed_url =
                Self::transform_git_url(&git_config.url, config.force_https, config.force_ssh);

            dockerfile.push_str(&format!(
                "RUN git clone {} {}/{}\n",
                transformed_url, container_workspace, git_config.name
            ));
        }

        // Clean up git config after cloning
        if has_github_https {
            dockerfile.push_str("\n# Clean up git credentials\n");
            dockerfile.push_str("RUN git config --global --unset url.\"https://.*@github.com/\".insteadOf || true\n");
        }

        dockerfile.push('\n');

        // Copy cross-compiled cim binary
        // Binary has been copied to build context, so use relative path
        dockerfile.push_str("# Copy cross-compiled cim binary\n");
        dockerfile.push_str("COPY cim /usr/local/bin/cim\n");
        dockerfile.push_str("RUN chmod +x /usr/local/bin/cim\n\n");

        // Create workspace marker file and generate Makefile
        dockerfile.push_str("# Set up workspace configuration\n");

        // Use relative path for sdk.yml since it's in the build context (temp directory)
        dockerfile.push_str(&format!("COPY sdk.yml {}/sdk.yml\n", container_workspace));

        // Create workspace marker file for workspace detection
        dockerfile.push_str("# Create workspace marker file\n");
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Calculate SHA256 of the config file
        let config_file_path = self.config_dir.join("sdk.yml");
        let config_sha256 = match fs::read(&config_file_path) {
            Ok(config_content) => {
                let mut hasher = Sha256::new();
                hasher.update(&config_content);
                format!("{:x}", hasher.finalize())
            }
            Err(_) => {
                // If config file doesn't exist (e.g., in tests), use a placeholder
                "test-generated".to_string()
            }
        };

        // Get SDK manager version information
        let version = env!("CARGO_PKG_VERSION");
        let commit = env!("GIT_HASH");
        let sha256 = match std::env::current_exe() {
            Ok(exe_path) => match std::fs::read(&exe_path) {
                Ok(binary_data) => {
                    let mut hasher_bin = Sha256::new();
                    hasher_bin.update(&binary_data);
                    format!("{:x}", hasher_bin.finalize())
                }
                Err(_) => "unknown".to_string(),
            },
            Err(_) => "unknown".to_string(),
        };

        dockerfile.push_str(&format!(
            "RUN echo 'workspace_version: \"1\"' > {}/.workspace && \\\n",
            container_workspace
        ));
        dockerfile.push_str(&format!(
            "    echo 'created_at: \"{}\"' >> {}/.workspace && \\\n",
            current_time, container_workspace
        ));
        dockerfile.push_str(&format!(
            "    echo 'config_file: \"sdk.yml\"' >> {}/.workspace && \\\n",
            container_workspace
        ));
        dockerfile.push_str(&format!(
            "    echo 'target: \"sdk.yml\"' >> {}/.workspace && \\\n",
            container_workspace
        ));
        dockerfile.push_str(&format!(
            "    echo 'target_version: \"latest\"' >> {}/.workspace && \\\n",
            container_workspace
        ));
        dockerfile.push_str(&format!(
            "    echo 'config_sha256: \"{}\"' >> {}/.workspace && \\\n",
            config_sha256, container_workspace
        ));
        dockerfile.push_str(&format!(
            "    echo 'mirror_path: \"/tmp/mirror\"' >> {}/.workspace && \\\n",
            container_workspace
        ));
        dockerfile.push_str(&format!(
            "    echo 'cim_version: \"{}\"' >> {}/.workspace && \\\n",
            version, container_workspace
        ));
        dockerfile.push_str(&format!(
            "    echo 'cim_sha256: \"{}\"' >> {}/.workspace && \\\n",
            sha256, container_workspace
        ));
        dockerfile.push_str(&format!(
            "    echo 'cim_commit: \"{}\"' >> {}/.workspace\n\n",
            commit, container_workspace
        ));

        // Copy additional files from copy_files directive
        if let Some(copy_files) = &config.sdk_config.copy_files {
            if !copy_files.is_empty() {
                dockerfile.push_str("# Copy additional files from copy_files directive\n");
                for copy_file in copy_files {
                    // Skip URL sources - they were downloaded during init
                    if copy_file.source.starts_with("http://")
                        || copy_file.source.starts_with("https://")
                    {
                        continue;
                    }

                    // For glob patterns, we need to copy the entire directory
                    // since Docker COPY doesn't expand shell globs the same way
                    let source_path: String =
                        if copy_file.source.contains('*') || copy_file.source.contains('?') {
                            // Extract the directory path from the glob pattern
                            // e.g., "patches/qemu/*.patch" -> "patches/qemu/"
                            let path_parts: Vec<&str> = copy_file.source.split('/').collect();
                            let dir_parts: Vec<&str> = path_parts
                                .iter()
                                .take_while(|&&part| !part.contains('*') && !part.contains('?'))
                                .copied()
                                .collect();

                            if dir_parts.is_empty() {
                                copy_file.source.clone()
                            } else {
                                // For patterns like "patches/qemu/*.patch", copy the whole directory
                                // This ensures all files that matched the glob are copied
                                dir_parts.join("/") + "/"
                            }
                        } else {
                            copy_file.source.clone()
                        };

                    let dest_path = format!("{}/{}", container_workspace, copy_file.dest);

                    dockerfile.push_str(&format!("COPY {} {}\n", source_path, dest_path));
                }
                dockerfile.push('\n');
            }
        } // Generate Makefile for SDK
        dockerfile.push_str("# Generate Makefile for SDK\n");
        dockerfile.push_str("RUN cim makefile\n\n");

        // Default command
        dockerfile.push_str("# Default command\n");
        dockerfile.push_str("CMD [\"/bin/bash\"]\n");

        Ok(dockerfile)
    }

    /// Get system packages for a specific distribution
    fn get_system_packages(&self, os_deps: &OsDependencies, distro: &str) -> Option<Vec<String>> {
        // Try architecture-specific configs first, then fall back to generic linux
        let os_key = if cfg!(target_arch = "aarch64") {
            "linux-aarch64"
        } else {
            "linux-x86_64"
        };

        // Try architecture-specific config first
        let result = os_deps
            .os_configs
            .get(os_key)
            .and_then(|linux_config| linux_config.distros.get(distro))
            .map(|distro_config| distro_config.package_manager.packages.clone());

        // Fall back to generic "linux" for backward compatibility
        if result.is_some() {
            result
        } else {
            os_deps
                .os_configs
                .get("linux")
                .and_then(|linux_config| linux_config.distros.get(distro))
                .map(|distro_config| distro_config.package_manager.packages.clone())
        }
    }

    /// Get Python packages for a specific profile
    fn get_python_packages(
        &self,
        python_deps: &PythonDependencies,
        profile: &str,
    ) -> Option<Vec<String>> {
        python_deps
            .profiles
            .get(profile)
            .map(|profile_config| profile_config.packages.clone())
    }

    /// Check if cross tool is available
    fn validate_cross_tool(&self) -> io::Result<()> {
        if std::process::Command::new("cross")
            .arg("--version")
            .output()
            .is_err()
        {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Cross compilation tool 'cross' not found.\n\
                Please install it with: cargo install cross",
            ));
        }
        Ok(())
    }

    /// Create Dockerfile and write to file
    pub fn create_dockerfile(&self, config: DockerfileConfig) -> io::Result<()> {
        // Check if file exists and force is not set
        if config.output_path.exists() && !config.force {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "Dockerfile already exists at {}. Use --force to overwrite.",
                    config.output_path.display()
                ),
            ));
        }

        // Validate cross compilation setup
        self.validate_cross_tool()?;
        // Note: Binary validation now happens in main.rs with correct architecture

        // Get available Docker images
        let available_images = self.get_available_images(config.os_deps);
        if available_images.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "No supported Linux distributions found in os-dependencies.yml",
            ));
        }

        // Select Docker image
        let docker_image = self
            .select_docker_image(config.distro_preference, &available_images)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "Could not select a suitable Docker image",
                )
            })?;

        messages::status(&format!(
            "Selected Docker image: {}:{}",
            docker_image.name, docker_image.tag
        ));

        // No additional validation needed - cim will be installed from git

        messages::status("Building cim from source inside container");

        // Generate Dockerfile content
        let gen_config = DockerfileGenerationConfig {
            sdk_config: config.sdk_config,
            os_deps: config.os_deps,
            python_deps: config.python_deps,
            docker_image: &docker_image,
            python_profile: config.python_profile,
            force_https: config.force_https,
            force_ssh: config.force_ssh,
            no_mirror: config.no_mirror,
        };
        let dockerfile_content = self.generate_dockerfile(&gen_config)?;

        // Create parent directory if it doesn't exist
        if let Some(parent) = config.output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write Dockerfile
        fs::write(config.output_path, dockerfile_content)?;

        messages::success(&format!(
            "Dockerfile created at {}",
            config.output_path.display()
        ));
        messages::status(&format!(
            "  Base image: {}:{}",
            docker_image.name, docker_image.tag
        ));
        messages::status(&format!("  Python profile: {}", config.python_profile));
        messages::status(&format!("  Repositories: {}", config.sdk_config.gits.len()));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GitConfig;
    use tempfile::tempdir;

    #[test]
    fn test_docker_manager_creation() {
        let temp_dir = tempdir().unwrap();
        let workspace_path = temp_dir.path().to_path_buf();
        let config_dir = temp_dir.path().to_path_buf();

        let docker_manager = DockerManager::new(workspace_path.clone(), config_dir.clone());

        assert_eq!(docker_manager.workspace_path, workspace_path);
        assert_eq!(docker_manager.config_dir, config_dir);
    }

    #[test]
    fn test_detect_url_type() {
        assert_eq!(
            DockerManager::detect_url_type("git@github.com:owner/repo.git"),
            UrlType::Ssh
        );
        assert_eq!(
            DockerManager::detect_url_type("ssh://git@example.com/repo.git"),
            UrlType::Ssh
        );
        assert_eq!(
            DockerManager::detect_url_type("https://github.com/owner/repo.git"),
            UrlType::Https
        );
        assert_eq!(
            DockerManager::detect_url_type("git://example.org/repo"),
            UrlType::Git
        );
        assert_eq!(
            DockerManager::detect_url_type("ftp://example.com/repo"),
            UrlType::Other
        );
    }

    #[test]
    fn test_convert_to_https() {
        // SSH to HTTPS
        assert_eq!(
            DockerManager::convert_to_https("git@github.com:owner/repo.git"),
            "https://github.com/owner/repo.git"
        );

        // Git protocol to HTTPS
        assert_eq!(
            DockerManager::convert_to_https("git://example.org/repo"),
            "https://example.org/repo.git"
        );

        // Already HTTPS - no change
        assert_eq!(
            DockerManager::convert_to_https("https://github.com/owner/repo.git"),
            "https://github.com/owner/repo.git"
        );
    }

    #[test]
    fn test_convert_to_ssh() {
        // HTTPS to SSH
        assert_eq!(
            DockerManager::convert_to_ssh("https://github.com/owner/repo.git"),
            "git@github.com:owner/repo.git"
        );

        // Git protocol to SSH
        assert_eq!(
            DockerManager::convert_to_ssh("git://example.org/repo"),
            "git@example.org:repo.git"
        );

        // Already SSH - no change
        assert_eq!(
            DockerManager::convert_to_ssh("git@github.com:owner/repo.git"),
            "git@github.com:owner/repo.git"
        );
    }

    #[test]
    fn test_transform_git_url() {
        let url = "git@github.com:owner/repo.git";

        // Force HTTPS
        assert_eq!(
            DockerManager::transform_git_url(url, true, false),
            "https://github.com/owner/repo.git"
        );

        // Force SSH (no change since already SSH)
        assert_eq!(
            DockerManager::transform_git_url(url, false, true),
            "git@github.com:owner/repo.git"
        );

        // Both flags - HTTPS wins
        assert_eq!(
            DockerManager::transform_git_url(url, true, true),
            "https://github.com/owner/repo.git"
        );

        // No flags - no change
        assert_eq!(
            DockerManager::transform_git_url(url, false, false),
            "git@github.com:owner/repo.git"
        );
    }

    #[test]
    fn test_has_ssh_repositories() {
        use std::path::PathBuf;

        let config_with_ssh = SdkConfig {
            mirror: PathBuf::from("/tmp"),
            gits: vec![
                GitConfig {
                    name: "repo1".to_string(),
                    url: "https://github.com/owner/repo1.git".to_string(),
                    commit: "main".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                },
                GitConfig {
                    name: "repo2".to_string(),
                    url: "git@github.com:owner/repo2.git".to_string(),
                    commit: "main".to_string(),
                    build_depends_on: None,
                    git_depends_on: None,
                    build: None,
                    documentation_dir: None,
                },
            ],
            toolchains: None,
            copy_files: None,
            install: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            build: None,
            clean: None,
            flash: None,
            variables: None,
        };

        assert!(DockerManager::has_ssh_repositories(&config_with_ssh));

        let config_no_ssh = SdkConfig {
            mirror: PathBuf::from("/tmp"),
            toolchains: None,
            gits: vec![GitConfig {
                name: "repo1".to_string(),
                url: "https://github.com/owner/repo1.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
            }],
            copy_files: None,
            install: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            build: None,
            clean: None,
            flash: None,
            variables: None,
        };

        assert!(!DockerManager::has_ssh_repositories(&config_no_ssh));
    }

    #[test]
    fn test_url_transformation_edge_cases() {
        // Test URLs without .git suffix
        assert_eq!(
            DockerManager::convert_to_https("git://example.org/repo"),
            "https://example.org/repo.git"
        );

        // Test complex paths
        assert_eq!(
            DockerManager::convert_to_https("git@gitlab.com:group/subgroup/project.git"),
            "https://gitlab.com/group/subgroup/project.git"
        );

        assert_eq!(
            DockerManager::convert_to_ssh("https://gitlab.com/group/subgroup/project.git"),
            "git@gitlab.com:group/subgroup/project.git"
        );
    }

    #[test]
    fn test_docker_image_selection() {
        let temp_dir = tempdir().unwrap();
        let docker_manager = DockerManager::new(
            temp_dir.path().to_path_buf(),
            temp_dir.path().join("sdk.yml"),
        );

        let images = vec![
            DockerImage {
                name: "ubuntu".to_string(),
                tag: "22.04".to_string(),
                package_manager: "apt-get install".to_string(),
                update_command: "apt-get update".to_string(),
            },
            DockerImage {
                name: "fedora".to_string(),
                tag: "42".to_string(),
                package_manager: "dnf install".to_string(),
                update_command: "dnf update -y".to_string(),
            },
        ];

        // Test user preference
        let selected = docker_manager.select_docker_image(Some("fedora"), &images);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().name, "fedora");

        // Test default fallback
        let selected = docker_manager.select_docker_image(None, &images);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().name, "ubuntu");
    }

    #[test]
    fn test_update_command_generation() {
        assert_eq!(
            DockerManager::get_update_command("ubuntu"),
            "apt-get update"
        );
        assert_eq!(DockerManager::get_update_command("fedora"), "dnf update -y");
        assert_eq!(DockerManager::get_update_command("centos"), "yum update -y");
        assert_eq!(
            DockerManager::get_update_command("unknown"),
            "echo 'Unknown package manager'"
        );
    }

    #[test]
    fn test_https_with_github_token_support() {
        let temp_dir = tempdir().unwrap();
        let docker_manager =
            DockerManager::new(temp_dir.path().to_path_buf(), temp_dir.path().to_path_buf());

        let sdk_config = SdkConfig {
            mirror: PathBuf::from("/tmp/mirror"),
            toolchains: None,
            gits: vec![GitConfig {
                name: "test-repo".to_string(),
                url: "git@github.com:user/repo.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
            }],
            copy_files: None,
            install: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            build: None,
            clean: None,
            flash: None,
            variables: None,
        };

        let os_deps = OsDependencies {
            os_configs: std::collections::HashMap::new(),
        };
        let python_deps = PythonDependencies {
            profiles: std::collections::HashMap::new(),
            default: "default".to_string(),
        };
        let docker_image = DockerImage {
            name: "ubuntu".to_string(),
            tag: "22.04".to_string(),
            package_manager: "apt-get install".to_string(),
            update_command: "apt-get update".to_string(),
        };

        let gen_config = DockerfileGenerationConfig {
            sdk_config: &sdk_config,
            os_deps: &os_deps,
            python_deps: &python_deps,
            docker_image: &docker_image,
            python_profile: "default",
            force_https: true,
            force_ssh: false,
            no_mirror: false,
        };
        let dockerfile = docker_manager.generate_dockerfile(&gen_config).unwrap();

        // Should contain build-time git clone commands (converted to HTTPS)
        assert!(dockerfile.contains("RUN git clone https://github.com/user/repo.git"));

        // Should have GitHub token support for private repos using BuildKit secrets
        assert!(dockerfile.contains("--mount=type=secret,id=GIT_AUTH_TOKEN"));
        assert!(dockerfile.contains("git config --global url"));

        // Should use cross-compiled binary approach
        assert!(dockerfile.contains("COPY cim /usr/local/bin/cim"));
        assert!(dockerfile.contains("RUN chmod +x /usr/local/bin/cim"));

        // Should NOT have entrypoint
        assert!(!dockerfile.contains("ENTRYPOINT"));

        // Should have default bash CMD
        assert!(dockerfile.contains("CMD [\"/bin/bash\"]"));
    }
}
