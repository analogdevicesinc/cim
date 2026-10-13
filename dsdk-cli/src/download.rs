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

use crate::workspace::{expand_env_vars, is_url, OS_DEPS_FILE, PYTHON_DEPS_FILE, SDK_CONFIG_FILE};
use crate::{config, messages};
use glob::glob;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;
use threadpool::ThreadPool;

/// Copy YAML configuration files to workspace root
pub fn copy_yaml_files_to_workspace(
    workspace_path: &Path,
    original_config_path: &Path,
    base_url: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create workspace directory if it doesn't exist
    if !workspace_path.exists() {
        fs::create_dir_all(workspace_path)?;
        messages::success(&format!(
            "Created workspace directory: {}",
            workspace_path.display()
        ));
    }

    if let Some(url) = base_url {
        // Handle URL-based configuration - download dependency files from URLs
        copy_yaml_files_from_url(workspace_path, original_config_path, url)
    } else {
        // Handle local file-based configuration
        copy_yaml_files_from_local(workspace_path, original_config_path)
    }
}

/// Copy YAML files from local directory
pub fn copy_yaml_files_from_local(
    workspace_path: &Path,
    original_config_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get the directory containing the original config file
    let config_dir = original_config_path.parent().unwrap_or(Path::new("."));

    let files_to_copy = [
        (SDK_CONFIG_FILE, original_config_path),
        (OS_DEPS_FILE, &config_dir.join(OS_DEPS_FILE)),
        (PYTHON_DEPS_FILE, &config_dir.join(PYTHON_DEPS_FILE)),
    ];

    for (filename, source_path) in files_to_copy {
        // Skip if source doesn't exist
        if !source_path.exists() {
            messages::verbose(&format!(
                "{} not found, skipping copy",
                source_path.display()
            ));
            continue;
        }

        let dest_path = workspace_path.join(filename);

        // Skip if source and destination are the same to avoid file corruption
        if source_path.canonicalize().ok() == dest_path.canonicalize().ok() && dest_path.exists() {
            messages::verbose(&format!("{} already in workspace, skipping copy", filename));
            continue;
        }

        fs::copy(source_path, &dest_path)?;
        messages::verbose(&format!("Copied {} to workspace", filename));
    }

    Ok(())
}

/// Copy YAML files from URL-based configuration
pub fn copy_yaml_files_from_url(
    workspace_path: &Path,
    original_config_path: &Path,
    base_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Extract base URL directory (remove the filename)
    let base_dir_url = if base_url.ends_with("/sdk.yml") {
        base_url.strip_suffix("/sdk.yml").unwrap()
    } else if base_url.ends_with(SDK_CONFIG_FILE) {
        base_url
            .rsplit_once('/')
            .map(|(base, _)| base)
            .unwrap_or(base_url)
    } else {
        base_url
    };

    let files_to_download = [
        (OS_DEPS_FILE, format!("{}/{}", base_dir_url, OS_DEPS_FILE)),
        (
            PYTHON_DEPS_FILE,
            format!("{}/{}", base_dir_url, PYTHON_DEPS_FILE),
        ),
    ];

    // Copy the already downloaded sdk.yml file
    let dest_config_path = workspace_path.join(SDK_CONFIG_FILE);
    fs::copy(original_config_path, &dest_config_path)?;
    messages::verbose("Copied sdk.yml to workspace");

    // Download dependency files
    for (filename, url) in files_to_download {
        let dest_path = workspace_path.join(filename);

        match download_dependency_file(&url) {
            Ok(temp_path) => {
                fs::copy(temp_path, &dest_path)?;
                messages::verbose(&format!("Downloaded and copied {} to workspace", filename));
            }
            Err(e) => {
                messages::info(&format!("Failed to download {}: {}", filename, e));
            }
        }
    }

    Ok(())
}

/// Download a dependency file from URL
pub fn download_dependency_file(url: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let response = reqwest::blocking::get(url)?;

    if !response.status().is_success() {
        return Err(format!(
            "HTTP error {}: {}",
            response.status(),
            response.status().canonical_reason().unwrap_or("Unknown")
        )
        .into());
    }

    let content = response.text()?;

    // Create a named temporary file
    let temp_file = tempfile::NamedTempFile::new()?;
    let temp_file_path = temp_file.path().to_path_buf();

    fs::write(&temp_file_path, content)?;

    // Keep the tempfile alive by forgetting it
    std::mem::forget(temp_file);

    Ok(temp_file_path)
}

/// Check if a path pattern contains wildcard characters
pub fn has_wildcards(path: &str) -> bool {
    path.contains('*') || path.contains('?') || path.contains('[')
}

/// Expand glob pattern and return list of matching files with their relative paths
///
/// # Arguments
///
/// * `pattern` - The glob pattern to expand (e.g., "patches/qemu/*.patch")
/// * `base_dir` - The base directory to resolve the pattern from
///
/// # Returns
///
/// A vector of tuples containing (absolute_path, relative_path_from_pattern_base)
pub fn expand_glob_pattern(
    pattern: &str,
    base_dir: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, Box<dyn std::error::Error>> {
    let mut results = Vec::new();

    // Expand environment variables and tilde in the pattern first
    let expanded_pattern = expand_env_vars(pattern);

    // Build the full glob pattern
    let full_pattern = if Path::new(&expanded_pattern).is_absolute() {
        expanded_pattern.clone()
    } else {
        base_dir
            .join(&expanded_pattern)
            .to_string_lossy()
            .to_string()
    };

    // Find the base path (the part before any wildcards)
    let pattern_base = if let Some(wildcard_pos) = expanded_pattern.find(['*', '?', '[']) {
        let base_pattern = &expanded_pattern[..wildcard_pos];
        // Find the last path separator before the wildcard
        if let Some(sep_pos) = base_pattern.rfind(['/', '\\']) {
            &expanded_pattern[..sep_pos]
        } else {
            "."
        }
    } else {
        expanded_pattern.as_str()
    };

    let pattern_base_path = if Path::new(pattern_base).is_absolute() {
        PathBuf::from(pattern_base)
    } else {
        base_dir.join(pattern_base)
    };

    // Expand the glob pattern
    for entry in glob(&full_pattern)? {
        match entry {
            Ok(path) => {
                // Only include files, skip directories
                if path.is_file() {
                    // Calculate the relative path from the pattern base
                    let relative = if let Ok(rel) = path.strip_prefix(&pattern_base_path) {
                        rel.to_path_buf()
                    } else {
                        // Fallback: use the file name
                        path.file_name()
                            .map(PathBuf::from)
                            .unwrap_or_else(|| path.clone())
                    };
                    results.push((path, relative));
                }
            }
            Err(e) => {
                messages::verbose(&format!("Glob pattern error: {}", e));
            }
        }
    }

    Ok(results)
}

/// Copy a single file to destination, creating parent directories as needed
pub fn copy_single_file(
    source_path: &Path,
    dest_path: &Path,
    source_display: &str,
    dest_display: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create destination directory if it doesn't exist
    if let Some(parent) = dest_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
            messages::verbose(&format!("Created directory: {}", parent.display()));
        }
    }

    // Read and write file content
    let file_content = fs::read(source_path)?;
    fs::write(dest_path, &file_content)?;
    messages::verbose(&format!("Copied {} -> {}", source_display, dest_display));

    Ok(())
}

/// Extract filename from URL
///
/// # Arguments
///
/// * `url` - The URL to extract filename from
///
/// # Returns
///
/// Filename as a string, or "downloaded_file" if extraction fails
pub fn extract_filename_from_url(url: &str) -> String {
    url.split('/')
        .next_back()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "downloaded_file".to_string())
}

/// Truncate filename for display in progress bar
///
/// # Arguments
///
/// * `filename` - The filename to truncate
/// * `max_len` - Maximum length (default: 16)
///
/// # Returns
///
/// Truncated filename with "..." in the middle if longer than max_len
///
/// # Examples
///
/// * "short.txt" -> "short.txt"
/// * "XtensaTools_RJ_2024_4_linux.tgz" -> "XtensaT...x.tgz"
pub fn truncate_filename(filename: &str, max_len: usize) -> String {
    if filename.len() <= max_len {
        return filename.to_string();
    }

    // Reserve 3 chars for "..."
    let available = max_len.saturating_sub(3);
    if available < 2 {
        // Too short to truncate meaningfully
        return filename.chars().take(max_len).collect();
    }

    // Split available space: more chars at start, fewer at end
    let start_chars = (available * 2) / 3;
    let end_chars = available - start_chars;

    let start: String = filename.chars().take(start_chars).collect();
    let end: String = filename
        .chars()
        .rev()
        .take(end_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    format!("{}...{}", start, end)
}

/// Generate cache path for a URL download
///
/// Creates a path like: $MIRROR/downloads/<url-hash>-<filename>
/// The URL hash (first 16 chars of SHA256) prevents collisions when different URLs
/// have the same filename.
///
/// # Arguments
///
/// * `url` - The source URL
/// * `mirror_path` - The mirror directory path
///
/// # Returns
///
/// PathBuf to the cache location
pub fn generate_cache_path(url: &str, mirror_path: &Path) -> PathBuf {
    // Compute SHA256 of URL and take first 16 characters
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let url_hash = format!("{:x}", hasher.finalize());
    let url_hash_short = &url_hash[..16];

    // Extract filename from URL
    let filename = extract_filename_from_url(url);

    // Build cache path: $MIRROR/downloads/<hash>-<filename>
    mirror_path
        .join("downloads")
        .join(format!("{}-{}", url_hash_short, filename))
}

/// Download a file from a URL directly to a destination path
///
/// # Arguments
///
/// * `target` - What to download and where to save it
/// * `options` - POST data, headers, auth, and progress reporting
///
/// # Returns
///
/// Ok(()) on success, or an error if download or writing fails
pub fn download_file_to_destination(
    target: DownloadTarget,
    options: DownloadOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let clients = build_ssl_fallback_clients()?;
    let mut last_error = None;

    for (i, client) in clients.iter().enumerate() {
        let result = build_request(client, &target, &options).send();

        match result {
            Ok(response) if response.status().is_success() => {
                // Check content-type to detect HTML responses (bot protection pages)
                if response_is_html(&response) {
                    last_error = Some(format!(
                        "Received HTML content instead of file (likely bot protection): {}",
                        target.url
                    ));
                    continue; // Try next client
                }

                if i > 0 {
                    messages::verbose(&format!(
                        "Download successful using fallback SSL configuration (client {})",
                        i + 1
                    ));
                }
                return download_with_progress(
                    response,
                    target.dest_path,
                    options.multi_progress,
                    target.display_name,
                );
            }
            Ok(response) => {
                last_error = Some(format!(
                    "HTTP {} {} from {}",
                    response.status().as_u16(),
                    response.status().canonical_reason().unwrap_or("error"),
                    target.url
                ));
            }
            Err(e) => {
                if i == 0 {
                    messages::verbose(
                        "Standard SSL verification failed, trying with relaxed SSL settings...",
                    );
                }
                last_error = Some(format!("Failed to send request to {}: {}", target.url, e));
            }
        }
    }

    // If all HTTP clients failed, return the last error
    Err(last_error
        .unwrap_or_else(|| "Unknown download error".to_string())
        .into())
}

/// Build the two SSL fallback client configurations: strict verification
/// first (webpki roots, Wget User-Agent since it's known to work with the
/// ARM developer site), then one that accepts invalid certificates (for
/// problematic sites like SEGGER).
fn build_ssl_fallback_clients() -> Result<[reqwest::blocking::Client; 2], Box<dyn std::error::Error>>
{
    Ok([
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .user_agent("Wget/1.21.3")
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?,
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .user_agent("Wget/1.21.3")
            .redirect(reqwest::redirect::Policy::limited(10))
            .danger_accept_invalid_certs(true)
            .build()?,
    ])
}

/// Build one client attempt's request, applying POST data, custom headers,
/// and HTTP Basic auth from `options`.
fn build_request(
    client: &reqwest::blocking::Client,
    target: &DownloadTarget,
    options: &DownloadOptions,
) -> reqwest::blocking::RequestBuilder {
    let mut builder = if let Some(data) = options.post_data {
        client
            .post(target.url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(data.to_string())
    } else {
        client.get(target.url)
    };
    for (name, value) in options.headers {
        builder = builder.header(name, value);
    }
    if let Some((user, pass)) = options.basic_auth {
        builder = builder.basic_auth(user, Some(pass));
    }
    builder
}

/// Detect an HTML response body, which usually indicates a bot-protection
/// page rather than the requested file.
fn response_is_html(response: &reqwest::blocking::Response) -> bool {
    if let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) {
        if let Ok(ct_str) = content_type.to_str() {
            return ct_str.contains("text/html");
        }
    }
    false
}

/// Download a file with retry logic for transient network failures
///
/// Attempts up to 3 times with exponential backoff (1s, 2s, 4s delays).
/// This helps handle temporary network issues, server hiccups, or connection resets.
///
/// # Arguments
///
/// * `target` - What to download and where to save it
/// * `options` - POST data, headers, auth, and progress reporting
///
/// # Returns
///
/// Ok(()) on success after any attempt, or an error if all retries fail
pub fn download_file_with_retry(
    target: DownloadTarget,
    options: DownloadOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    const MAX_RETRIES: u32 = 3;
    let mut last_error: Option<Box<dyn std::error::Error>> = None;

    for attempt in 1..=MAX_RETRIES {
        // Try HTTP client first
        match download_file_to_destination(target, options) {
            Ok(()) => {
                if attempt > 1 {
                    messages::verbose(&format!("✓ Download succeeded on attempt {}", attempt));
                }
                return Ok(());
            }
            Err(e) => {
                messages::verbose(&format!("HTTP client failed: {}", e));
                last_error = Some(e);
            }
        }

        if try_wget_fallback(&target, &options) {
            return Ok(());
        }

        if try_curl_fallback(&target, &options) {
            return Ok(());
        }

        // All methods failed for this attempt
        if attempt < MAX_RETRIES {
            wait_and_clear_before_retry(attempt, MAX_RETRIES, target.dest_path);
        }
    }

    // All methods and retries failed
    Err(last_error.unwrap_or_else(|| {
        "Download failed after all retries and fallback methods"
            .to_string()
            .into()
    }))
}

/// What to download and where to save it.
#[derive(Clone, Copy)]
pub struct DownloadTarget<'a> {
    pub url: &'a str,
    pub dest_path: &'a Path,
    pub display_name: &'a str,
}

/// Optional extras controlling how a download request is sent.
#[derive(Clone, Copy)]
pub struct DownloadOptions<'a> {
    pub post_data: Option<&'a str>,
    pub headers: &'a [(String, String)],
    pub basic_auth: Option<(&'a str, &'a str)>,
    pub multi_progress: Option<&'a MultiProgress>,
}

/// Try the download via `wget` as an HTTP-client fallback.
///
/// Uses wget's default User-Agent (not browser-like) to avoid bot protection.
/// Returns true on success.
fn try_wget_fallback(target: &DownloadTarget, options: &DownloadOptions) -> bool {
    messages::verbose("Trying wget as fallback...");
    let mut wget_cmd = Command::new("wget");
    wget_cmd.arg("--no-check-certificate").arg("--timeout=300");
    for (name, value) in options.headers {
        wget_cmd.arg(format!("--header={}: {}", name, value));
    }
    if let Some((user, pass)) = options.basic_auth {
        wget_cmd
            .arg(format!("--http-user={}", user))
            .arg(format!("--http-password={}", pass));
    }
    match wget_cmd
        .arg("-O")
        .arg(target.dest_path)
        .arg(target.url)
        .output()
    {
        Ok(output) if output.status.success() => {
            messages::verbose("✓ Download successful using wget");
            true
        }
        Ok(output) => {
            messages::verbose(&format!(
                "wget failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
            false
        }
        Err(_) => false,
    }
}

/// Try the download via `curl` (with its default User-Agent) as a further fallback.
///
/// Returns true on success.
fn try_curl_fallback(target: &DownloadTarget, options: &DownloadOptions) -> bool {
    messages::verbose("Trying curl as fallback...");
    let mut curl_cmd = Command::new("curl");
    curl_cmd
        .arg("--insecure")
        .arg("--max-time")
        .arg("300")
        .arg("-L"); // follow redirects
    for (name, value) in options.headers {
        curl_cmd.arg("-H").arg(format!("{}: {}", name, value));
    }
    if let Some((user, pass)) = options.basic_auth {
        curl_cmd.arg("-u").arg(format!("{}:{}", user, pass));
    }
    match curl_cmd
        .arg("-o")
        .arg(target.dest_path)
        .arg(target.url)
        .output()
    {
        Ok(output) if output.status.success() => {
            messages::verbose("✓ Download successful using curl");
            true
        }
        Ok(output) => {
            messages::verbose(&format!(
                "curl failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
            false
        }
        Err(_) => false,
    }
}

/// Sleep with exponential backoff (3s, 6s, 12s), then remove any partial
/// file left behind before the next retry attempt.
fn wait_and_clear_before_retry(attempt: u32, max_retries: u32, dest_path: &Path) {
    let delay_secs = 3u64 << (attempt - 1);
    messages::info(&format!(
        "Download failed (attempt {}/{}), retrying in {}s...",
        attempt, max_retries, delay_secs
    ));
    std::thread::sleep(Duration::from_secs(delay_secs));

    if dest_path.exists() {
        if let Err(remove_err) = fs::remove_file(dest_path) {
            messages::verbose(&format!(
                "Warning: Could not remove partial file: {}",
                remove_err
            ));
        }
    }
}

/// Helper function to download response with progress bar
pub fn download_with_progress(
    mut response: reqwest::blocking::Response,
    dest_path: &Path,
    multi_progress: Option<&MultiProgress>,
    display_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create destination directory if needed
    if let Some(parent) = dest_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    // Get total size for progress bar
    let total_size = response.content_length();

    // Create progress bar - bitbake-style with filename prefix
    // Truncate filename to 16 chars for uniform display
    let truncated_name = truncate_filename(display_name, 16);

    let pb = if let Some(size) = total_size {
        let pb = if let Some(mp) = multi_progress {
            mp.add(ProgressBar::new(size))
        } else {
            ProgressBar::new(size)
        };
        pb.set_style(
            ProgressStyle::default_bar()
                .template("  {msg}: [{bar:27}] {bytes}/{total_bytes} ({eta})")?
                .progress_chars("=>-"),
        );
        pb.set_message(truncated_name.clone());
        Some(pb)
    } else {
        // Unknown size - show spinner
        let pb = if let Some(mp) = multi_progress {
            mp.add(ProgressBar::new_spinner())
        } else {
            ProgressBar::new_spinner()
        };
        pb.set_style(ProgressStyle::default_spinner().template("  {spinner} {msg}")?);
        pb.set_message(format!("Downloading {}", truncated_name));
        Some(pb)
    };

    // Stream download with 8KB buffer (memory efficient)
    let mut file = fs::File::create(dest_path)?;
    let mut buffer = [0; 8192];
    let mut downloaded = 0u64;

    loop {
        let bytes_read = response.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        file.write_all(&buffer[..bytes_read])?;
        downloaded += bytes_read as u64;

        if let Some(pb) = &pb {
            if total_size.is_some() {
                pb.set_position(downloaded);
            } else {
                pb.tick();
            }
        }
    }

    // Finish progress bar and clear it (bitbake-style: remove completed downloads)
    if let Some(pb) = pb {
        pb.finish_and_clear();
        // Only show completion in verbose mode to keep output clean
        messages::verbose(&format!("Downloaded: {}", display_name));
    }

    messages::verbose(&format!("Downloaded to: {}", dest_path.display()));
    Ok(())
}

/// Compute SHA256 hash of a file
///
/// # Arguments
///
/// * `file_path` - Path to the file to hash
///
/// # Returns
///
/// SHA256 hash as a lowercase hex string, or an error if file cannot be read
pub fn compute_file_sha256(file_path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let content = fs::read(file_path)?;
    let mut hasher = Sha256::new();
    hasher.update(&content);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Verify SHA256 checksum of a file
///
/// # Arguments
///
/// * `file_path` - Path to the file to verify
/// * `expected_sha256` - Expected SHA256 hash (lowercase hex string)
///
/// # Returns
///
/// Ok(()) if hash matches, or an error with mismatch details
pub fn verify_file_sha256(
    file_path: &Path,
    expected_sha256: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let actual_sha256 = compute_file_sha256(file_path)?;
    let expected_lower = expected_sha256.to_lowercase();

    if actual_sha256 == expected_lower {
        Ok(())
    } else {
        Err(format!(
            "SHA256 mismatch:\n  Expected: {}\n  Actual:   {}",
            expected_lower, actual_sha256
        )
        .into())
    }
}

/// Configuration for downloading a file with optional caching
pub struct DownloadConfig<'a> {
    pub url: &'a str,
    pub dest_path: &'a Path,
    pub mirror_path: &'a Path,
    pub use_cache: bool,
    pub expected_sha256: Option<&'a str>,
    pub post_data: Option<&'a str>,
    /// Already-resolved (env-var-expanded) headers -- see
    /// `CopyFileConfig::resolved_headers`.
    pub headers: &'a [(String, String)],
    /// Already-resolved (env-var-expanded) HTTP Basic auth credentials --
    /// see `CopyFileConfig::resolved_basic_auth`.
    pub basic_auth: Option<(&'a str, &'a str)>,
    pub multi_progress: Option<&'a MultiProgress>,
    pub use_symlink: bool,
}

/// Download a file from URL with mirror caching support
///
/// If cache is enabled:
/// - Checks if file exists in mirror cache
/// - If cached: copies from cache to destination
/// - If not cached: downloads to cache, then copies to destination
///
/// If cache is disabled:
/// - Downloads directly to destination (existing behavior)
///
/// # Arguments
///
/// * `config` - Download configuration containing URL, paths, and options
///
/// # Returns
///
/// Ok(()) on success, or an error if download, copy, or verification fails
pub fn download_file_with_cache(config: DownloadConfig) -> Result<(), Box<dyn std::error::Error>> {
    let DownloadConfig {
        url,
        dest_path,
        mirror_path,
        use_cache,
        expected_sha256,
        post_data,
        headers,
        basic_auth,
        multi_progress,
        use_symlink,
    } = config;

    if !use_cache {
        let filename = extract_filename_from_url(url);
        messages::verbose(&format!("Downloading: {}", filename));
        download_file_with_retry(
            DownloadTarget {
                url,
                dest_path,
                display_name: &filename,
            },
            DownloadOptions {
                post_data,
                headers,
                basic_auth,
                multi_progress,
            },
        )?;
        return verify_destination_checksum(dest_path, expected_sha256);
    }

    let cache_path = generate_cache_path(url, mirror_path);
    let filename = extract_filename_from_url(url);

    if !cache_path.exists() {
        messages::verbose(&format!(
            "Downloading: {} (first time, will cache)",
            filename
        ));
        ensure_parent_dir(&cache_path)?;
        download_file_with_retry(
            DownloadTarget {
                url,
                dest_path: &cache_path,
                display_name: &filename,
            },
            DownloadOptions {
                post_data,
                headers,
                basic_auth,
                multi_progress,
            },
        )?;
        place_file_from_cache(&cache_path, dest_path, use_symlink, true)?;
        return verify_destination_checksum(dest_path, expected_sha256);
    }

    // Cache hit: verify integrity before trusting it, re-downloading in
    // place if it turns out to be corrupt or partial.
    if matches!(
        validate_cached_file(&cache_path, &filename, expected_sha256)?,
        CachedFileStatus::Removed
    ) {
        download_file_with_retry(
            DownloadTarget {
                url,
                dest_path: &cache_path,
                display_name: &filename,
            },
            DownloadOptions {
                post_data,
                headers,
                basic_auth,
                multi_progress,
            },
        )?;
    }
    place_file_from_cache(&cache_path, dest_path, use_symlink, false)?;

    verify_destination_checksum(dest_path, expected_sha256)
}

/// Create `path`'s parent directory if it doesn't already exist.
fn ensure_parent_dir(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

/// Result of checking a cached download against its expected checksum.
enum CachedFileStatus {
    /// Cache is trustworthy (verified, or no checksum was requested) and can be reused as-is.
    Valid,
    /// Cache was corrupt/partial and has been deleted; caller must re-download.
    Removed,
}

/// Verify a cached file's SHA256 (if one was requested), deleting it on mismatch.
fn validate_cached_file(
    cache_path: &Path,
    filename: &str,
    expected_sha256: Option<&str>,
) -> Result<CachedFileStatus, Box<dyn std::error::Error>> {
    let Some(sha256) = expected_sha256 else {
        // No SHA256 provided, trust the cached file
        messages::verbose(&format!("Using cached {} from mirror", filename));
        return Ok(CachedFileStatus::Valid);
    };

    messages::verbose(&format!("Verifying integrity of cached {} ...", filename));

    match verify_file_sha256(cache_path, sha256) {
        Ok(()) => {
            messages::verbose("✓ Cached file integrity verified");
            Ok(CachedFileStatus::Valid)
        }
        Err(e) => {
            // Cached file is corrupt or partial - delete and re-download
            messages::info(&format!(
                "Cached file is corrupt ({}), re-downloading...",
                e
            ));

            if let Err(remove_err) = fs::remove_file(cache_path) {
                messages::verbose(&format!(
                    "Warning: Failed to remove corrupt cache file: {}",
                    remove_err
                ));
            }

            ensure_parent_dir(cache_path)?;
            Ok(CachedFileStatus::Removed)
        }
    }
}

/// Copy or symlink `cache_path` to `dest_path`, skipping the copy entirely if
/// the destination already resolves to the same file. `freshly_downloaded`
/// only selects the verbose-log wording ("Cached and ..." vs "... from cache").
fn place_file_from_cache(
    cache_path: &Path,
    dest_path: &Path,
    use_symlink: bool,
    freshly_downloaded: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    ensure_parent_dir(dest_path)?;

    // Check if destination already resolves to the same file as cache
    // (e.g., dest_path == cache_path, or dest is a symlink to cache).
    // Copying a file onto itself truncates it to 0 bytes.
    let is_same_file = if dest_path.exists() {
        match (fs::canonicalize(cache_path), fs::canonicalize(dest_path)) {
            (Ok(src), Ok(dst)) => src == dst,
            _ => false,
        }
    } else {
        false
    };

    if is_same_file {
        messages::verbose("Destination already points to cached file, skipping copy");
        return Ok(());
    }

    let symlink_verb = if freshly_downloaded {
        "Cached and symlinked"
    } else {
        "Symlinked from cache"
    };
    let copy_verb = if freshly_downloaded {
        "Cached and copied"
    } else {
        "Copied from cache"
    };

    if use_symlink {
        // Remove destination if it exists (needed for symlink creation)
        if dest_path.exists() {
            fs::remove_file(dest_path)?;
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(cache_path, dest_path)?;
            messages::verbose(&format!("{} to: {}", symlink_verb, dest_path.display()));
        }
        #[cfg(not(unix))]
        {
            fs::copy(cache_path, dest_path)?;
            messages::verbose(&format!("{} to: {}", copy_verb, dest_path.display()));
        }
    } else {
        fs::copy(cache_path, dest_path)?;
        messages::verbose(&format!("{} to: {}", copy_verb, dest_path.display()));
    }

    Ok(())
}

/// Verify the final destination's SHA256 if one was requested by the manifest.
fn verify_destination_checksum(
    dest_path: &Path,
    expected_sha256: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(sha256) = expected_sha256 else {
        messages::verbose("SHA256 checksum not provided, skipping verification");
        return Ok(());
    };

    messages::verbose("Verifying SHA256 checksum...");
    verify_file_sha256(dest_path, sha256)?;
    messages::verbose("✓ SHA256 checksum verified");
    Ok(())
}

/// Process copy_files configuration to copy files from source to workspace
///
/// Supports wildcard patterns in source paths:
/// - `patches/qemu/*.patch` - matches all .patch files in patches/qemu/
/// - `patches/qemu/000*.patch` - matches files starting with 000
/// - `patches/**/*.patch` - recursively matches all .patch files under patches/
/// - `patches` or `patches/` - copies entire directory recursively
///
/// Also supports downloading files from URLs:
/// - `https://example.com/file.tar.gz` - downloads file directly to destination
/// - `cache: true` - optional caching in mirror for reuse
pub fn process_copy_files(
    workspace_path: &Path,
    config_source_dir: &Path,
    copy_files: &[config::CopyFileConfig],
    mirror_path: &Path,
    is_remote_git_source: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Separate URL downloads from local file operations
    let (url_files, local_files): (Vec<_>, Vec<_>) =
        copy_files.iter().partition(|cf| is_url(&cf.source));

    if !url_files.is_empty() {
        download_url_copy_files(workspace_path, mirror_path, &url_files)?;
    }

    // Process local files sequentially (fast, no need for parallelization)
    for copy_file in local_files {
        process_local_copy_file(
            workspace_path,
            config_source_dir,
            is_remote_git_source,
            copy_file,
        );
    }

    Ok(())
}

type ResolvedUrlCopyFile<'a> = (
    &'a config::CopyFileConfig,
    String,
    Vec<(String, String)>,
    Option<(String, String)>,
);

/// Resolve headers/basic_auth/source URL for every `copy_files:` URL entry
/// up front, so a missing env var aborts the whole batch immediately rather
/// than failing mid-flight after other downloads have already started.
fn resolve_url_copy_files<'a>(
    url_files: &[&'a config::CopyFileConfig],
) -> Result<Vec<ResolvedUrlCopyFile<'a>>, Box<dyn std::error::Error>> {
    let mut resolved = Vec::with_capacity(url_files.len());
    for copy_file in url_files {
        let headers = copy_file
            .resolved_headers()
            .map_err(|e| format!("copy_files entry '{}': {}", copy_file.dest, e))?;
        let basic_auth = copy_file
            .resolved_basic_auth()
            .map_err(|e| format!("copy_files entry '{}': {}", copy_file.dest, e))?;
        // Expand $VAR/${VAR} in the source URL itself, e.g. for a
        // query-string API key -- same as toolchains: url is expanded.
        let url = expand_env_vars(&copy_file.source);
        resolved.push((*copy_file, url, headers, basic_auth));
    }
    Ok(resolved)
}

/// A `copy_files:` URL entry with all values resolved and owned, ready to be
/// moved into a thread-pool download job closure.
struct QueuedDownload {
    dest: String,
    dest_path: PathBuf,
    mirror_path: PathBuf,
    use_cache: bool,
    use_symlink: bool,
    expected_sha256: Option<String>,
    post_data: Option<String>,
    headers: Vec<(String, String)>,
    basic_auth: Option<(String, String)>,
    url: String,
}

fn prepare_queued_download(
    workspace_path: &Path,
    mirror_path: &Path,
    copy_file: &config::CopyFileConfig,
    url: String,
    headers: Vec<(String, String)>,
    basic_auth: Option<(String, String)>,
) -> QueuedDownload {
    let dest = expand_env_vars(&copy_file.dest);
    let dest_path = workspace_path.join(&dest);

    messages::verbose(&format!("Processing URL: {} -> {}", url, dest));
    if !headers.is_empty() {
        let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_str()).collect();
        messages::verbose(&format!("  Custom header(s): {}", names.join(", ")));
    }
    if basic_auth.is_some() {
        messages::verbose("  Using HTTP Basic auth (credentials redacted)");
    }

    let use_cache = copy_file.cache.unwrap_or(false);
    QueuedDownload {
        dest,
        dest_path,
        mirror_path: mirror_path.to_path_buf(),
        use_cache,
        use_symlink: copy_file.symlink.unwrap_or(false) && use_cache,
        expected_sha256: copy_file.sha256.clone(),
        post_data: copy_file.post_data.clone(),
        headers,
        basic_auth,
        url,
    }
}

fn collect_download_results(
    rx: std::sync::mpsc::Receiver<(String, Result<(), String>)>,
) -> Vec<String> {
    let mut failed_downloads = Vec::new();
    for (dest, result) in rx {
        if let Err(e) = result {
            messages::error(&format!("Failed to download {}: {}", dest, e));
            failed_downloads.push(dest);
        }
    }
    failed_downloads
}

fn report_failed_downloads(failed_downloads: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if failed_downloads.is_empty() {
        return Ok(());
    }
    let error_msg = if failed_downloads.len() == 1 {
        format!("Download failed: {}", failed_downloads[0])
    } else {
        format!(
            "{} downloads failed: {}",
            failed_downloads.len(),
            failed_downloads.join(", ")
        )
    };
    Err(error_msg.into())
}

/// Download every `copy_files:` URL entry in parallel (up to 4 concurrent).
fn download_url_copy_files(
    workspace_path: &Path,
    mirror_path: &Path,
    url_files: &[&config::CopyFileConfig],
) -> Result<(), Box<dyn std::error::Error>> {
    let resolved = resolve_url_copy_files(url_files)?;

    let multi_progress = MultiProgress::new();
    let pool = ThreadPool::new(4); // Max 4 concurrent downloads
    let (tx, rx): (Sender<(String, Result<(), String>)>, _) = channel();

    messages::status("Downloading and checking file integrity...");

    for (copy_file, url, headers, basic_auth) in resolved {
        let queued = prepare_queued_download(
            workspace_path,
            mirror_path,
            copy_file,
            url,
            headers,
            basic_auth,
        );
        let tx = tx.clone();
        let mp = multi_progress.clone();

        pool.execute(move || {
            let basic_auth_ref = queued
                .basic_auth
                .as_ref()
                .map(|(u, p)| (u.as_str(), p.as_str()));
            let result = download_file_with_cache(DownloadConfig {
                url: &queued.url,
                dest_path: &queued.dest_path,
                mirror_path: &queued.mirror_path,
                use_cache: queued.use_cache,
                expected_sha256: queued.expected_sha256.as_deref(),
                post_data: queued.post_data.as_deref(),
                headers: &queued.headers,
                basic_auth: basic_auth_ref,
                multi_progress: Some(&mp),
                use_symlink: queued.use_symlink,
            })
            .map_err(|e| e.to_string());

            tx.send((queued.dest.clone(), result)).unwrap();
        });
    }

    // Drop sender so receiver knows when all tasks are done
    drop(tx);

    let failed_downloads = collect_download_results(rx);

    // Ensure MultiProgress is properly cleared before dropping
    drop(multi_progress);

    report_failed_downloads(&failed_downloads)
}

/// Process one local (non-URL) `copy_files:` entry, dispatching to wildcard
/// expansion or a direct file/directory copy.
fn process_local_copy_file(
    workspace_path: &Path,
    config_source_dir: &Path,
    is_remote_git_source: bool,
    copy_file: &config::CopyFileConfig,
) {
    let expanded_dest = expand_env_vars(&copy_file.dest);

    if has_wildcards(&copy_file.source) {
        copy_wildcard_matches(workspace_path, config_source_dir, copy_file, &expanded_dest);
    } else {
        copy_local_source(
            workspace_path,
            config_source_dir,
            is_remote_git_source,
            copy_file,
            &expanded_dest,
        );
    }
}

/// Expand a wildcard `copy_files:` source pattern and copy every match.
fn copy_wildcard_matches(
    workspace_path: &Path,
    config_source_dir: &Path,
    copy_file: &config::CopyFileConfig,
    expanded_dest: &str,
) {
    messages::verbose(&format!("Expanding pattern: {}", copy_file.source));

    let matches = match expand_glob_pattern(&copy_file.source, config_source_dir) {
        Ok(matches) => matches,
        Err(e) => {
            messages::info(&format!(
                "Failed to expand pattern {}: {}",
                copy_file.source, e
            ));
            return;
        }
    };

    if matches.is_empty() {
        messages::info(&format!("No files matched pattern: {}", copy_file.source));
        return;
    }

    messages::verbose(&format!(
        "Found {} file(s) matching pattern: {}",
        matches.len(),
        copy_file.source
    ));

    // Ensure destination is treated as a directory
    let dest_base = workspace_path.join(expanded_dest);

    for (source_path, relative_path) in matches {
        let dest_path = dest_base.join(&relative_path);

        if let Err(e) = copy_single_file(
            &source_path,
            &dest_path,
            &format!("{}/{}", copy_file.source, relative_path.display()),
            &format!("{}/{}", expanded_dest, relative_path.display()),
        ) {
            messages::info(&format!("Failed to copy {}: {}", source_path.display(), e));
        }
    }
}

/// Copy a non-wildcard `copy_files:` source path, handling both a single
/// file and a whole directory (copied recursively).
fn copy_local_source(
    workspace_path: &Path,
    config_source_dir: &Path,
    is_remote_git_source: bool,
    copy_file: &config::CopyFileConfig,
    expanded_dest: &str,
) {
    // Expand environment variables and tilde in the source path
    let expanded_source = expand_env_vars(&copy_file.source);
    let source_path = if Path::new(&expanded_source).is_absolute() {
        PathBuf::from(&expanded_source)
    } else {
        config_source_dir.join(&expanded_source)
    };

    if !source_path.exists() {
        report_missing_copy_source(copy_file, is_remote_git_source);
        return;
    }

    if source_path.is_dir() {
        copy_directory_source(workspace_path, copy_file, expanded_dest, &source_path);
        return;
    }

    // Source is a file - copy single file as before
    let dest_path = workspace_path.join(expanded_dest);
    if let Err(e) = copy_single_file(&source_path, &dest_path, &copy_file.source, expanded_dest) {
        messages::info(&format!("Failed to copy {}: {}", copy_file.source, e));
    }
}

fn report_missing_copy_source(copy_file: &config::CopyFileConfig, is_remote_git_source: bool) {
    if is_remote_git_source {
        messages::info(&format!(
            "File {} not found in extracted manifest (check copy_files paths), skipping copy",
            copy_file.source
        ));
    } else {
        messages::info(&format!(
            "Source file {} does not exist, skipping copy",
            copy_file.source
        ));
    }
}

/// Copy a whole source directory recursively into the workspace destination.
fn copy_directory_source(
    workspace_path: &Path,
    copy_file: &config::CopyFileConfig,
    expanded_dest: &str,
    source_path: &Path,
) {
    messages::verbose(&format!(
        "Copying directory: {} -> {}",
        copy_file.source, expanded_dest
    ));

    let dest_base = workspace_path.join(expanded_dest);

    if let Err(e) = copy_dir_contents(source_path, &dest_base, source_path, &copy_file.source) {
        messages::info(&format!(
            "Failed to copy directory {}: {}",
            copy_file.source, e
        ));
    }
}

/// Recursively walk `src`, copying every file into the matching path under
/// `dst`; `verbose_prefix` is the original (un-expanded) source used for
/// per-file logging.
fn copy_dir_contents(
    src: &Path,
    dst: &Path,
    src_base: &Path,
    verbose_prefix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dst.join(&file_name);

        if path.is_dir() {
            copy_dir_contents(&path, &dest_path, src_base, verbose_prefix)?;
        } else {
            // Calculate relative path for verbose output
            let rel_path = path
                .strip_prefix(src_base)
                .unwrap_or(&path)
                .to_string_lossy();

            if let Err(e) = copy_single_file(
                &path,
                &dest_path,
                &format!("{}/{}", verbose_prefix, rel_path),
                &format!("{}/{}", verbose_prefix, rel_path),
            ) {
                messages::info(&format!("Failed to copy {}: {}", path.display(), e));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_filename() {
        // Test short filename (no truncation needed)
        assert_eq!(truncate_filename("short.txt", 16), "short.txt");

        // Test filename at max length
        assert_eq!(
            truncate_filename("exactly16chars!!", 16),
            "exactly16chars!!"
        );

        // Test long filename truncation
        let long_name = "XtensaTools_RJ_2024_4_linux.tgz";
        let truncated = truncate_filename(long_name, 16);
        assert_eq!(truncated.len(), 16);
        assert!(truncated.starts_with("XtensaTo"));
        assert!(truncated.ends_with(".tgz"));
        assert!(truncated.contains("..."));

        // Test very long filename
        let very_long = "cim-suite-0.5.5-beta.1-aarch64-unknown-linux-gnu.tar.gz";
        let truncated = truncate_filename(very_long, 16);
        assert_eq!(truncated.len(), 16);
        assert!(truncated.contains("..."));

        // Test edge case with max_len smaller than filename
        let result = truncate_filename("test.txt", 5);
        assert_eq!(result.len(), 5);

        // Test empty string
        assert_eq!(truncate_filename("", 16), "");

        // Test single character
        assert_eq!(truncate_filename("a", 16), "a");
    }

    #[test]
    fn test_download_file_to_destination_rejects_bad_header_value() {
        // A header value with an embedded newline is invalid and must be
        // rejected by reqwest's own validation, surfacing as an Err rather
        // than a panic -- no real network access needed, since the bad
        // header is rejected before any connection is attempted.
        let dir = tempfile::tempdir().unwrap();
        let dest_path = dir.path().join("out.bin");
        let headers = vec![("X-Bad".to_string(), "value\nwith\nnewlines".to_string())];

        let result = download_file_to_destination(
            DownloadTarget {
                url: "http://127.0.0.1:0",
                dest_path: &dest_path,
                display_name: "test",
            },
            DownloadOptions {
                post_data: None,
                headers: &headers,
                basic_auth: None,
                multi_progress: None,
            },
        );

        assert!(result.is_err());
    }
}
