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

#[cfg(test)]
use crate::config::SdkConfig;
use crate::config::{SdkConfigCore, UserConfig};
use crate::messages;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Represents a documentation source found in a repository
#[derive(Debug, Clone)]
pub struct DocSource {
    pub repo_name: String,
    pub docs_path: PathBuf,
    pub index_rst_path: PathBuf,
    pub title: String,
}

/// Documentation manager for discovering and aggregating repository documentation
pub struct DocManager {
    pub workspace_path: PathBuf,
    pub docs_output_path: PathBuf,
}

impl DocManager {
    /// Create a new documentation manager
    pub fn new(workspace_path: PathBuf) -> Self {
        let docs_output_path = workspace_path.join("documentation");
        Self {
            workspace_path,
            docs_output_path,
        }
    }

    /// Check if virtual environment exists in workspace
    fn venv_exists(&self) -> bool {
        let venv_path = self.workspace_path.join(".venv");
        venv_path.exists() && venv_path.join("bin").join("python3").exists()
    }

    /// Get the command and environment for executing Python tools with virtual environment support
    fn get_python_command(&self, tool: &str) -> (String, Vec<(String, String)>) {
        if self.venv_exists() {
            let venv_bin = self.workspace_path.join(".venv").join("bin");
            let mut path = std::env::var("PATH").unwrap_or_default();

            // Prepend venv bin directory to PATH so tools like sphinx-build are found
            path = format!("{}:{}", venv_bin.display(), path);

            let env_vars = vec![
                ("PATH".to_string(), path),
                (
                    "VIRTUAL_ENV".to_string(),
                    self.workspace_path
                        .join(".venv")
                        .to_string_lossy()
                        .to_string(),
                ),
            ];

            (tool.to_string(), env_vars)
        } else {
            (tool.to_string(), vec![])
        }
    }

    /// Discover all documentation sources in the workspace repositories
    ///
    /// Searches for index.rst files in multiple directories per repository:
    /// 1. Built-in defaults: docs, doc, documentation, documents, . (root)
    /// 2. User config directories (if provided via config.toml)
    /// 3. Per-repository custom directory (if specified in sdk.yml)
    ///
    /// The search list is deduplicated and first match wins.
    pub fn discover_doc_sources<T: SdkConfigCore>(
        &self,
        sdk_config: &T,
        user_config: Option<&UserConfig>,
        verbose: bool,
    ) -> io::Result<Vec<DocSource>> {
        let mut doc_sources = Vec::new();

        // Show default search directories once at the beginning
        let default_dirs = Self::get_default_search_dirs(user_config);
        if !default_dirs.is_empty() {
            messages::status(&format!("Default folders: {}", default_dirs.join(", ")));
        }

        for git_cfg in sdk_config.gits() {
            let repo_path = self.workspace_path.join(&git_cfg.name);

            if !repo_path.exists() {
                messages::info(&format!(
                    "Warning: Repository {} does not exist in workspace",
                    git_cfg.name
                ));
                continue;
            }

            // Build combined search list
            let search_dirs = self.build_search_dirs(git_cfg, user_config);

            if verbose {
                messages::verbose(&format!(
                    "Searching for documentation in {} using directories: {:?}",
                    git_cfg.name, search_dirs
                ));
            }

            // Search for index.rst in the combined list
            let mut found_docs_path = None;
            let mut found_index_rst = None;

            for dir_name in &search_dirs {
                let (check_path, check_index) = if dir_name == "." {
                    // Root directory
                    (repo_path.clone(), repo_path.join("index.rst"))
                } else {
                    // Subdirectory
                    let subdir = repo_path.join(dir_name);
                    (subdir.clone(), subdir.join("index.rst"))
                };

                if check_index.exists() {
                    found_docs_path = Some(check_path);
                    found_index_rst = Some(check_index);
                    break; // First match wins
                }
            }

            if let (Some(docs_path), Some(index_rst)) = (found_docs_path, found_index_rst) {
                let title = self
                    .extract_title_from_index_rst(&index_rst)
                    .unwrap_or_else(|| git_cfg.name.clone());

                doc_sources.push(DocSource {
                    repo_name: git_cfg.name.clone(),
                    docs_path,
                    index_rst_path: index_rst,
                    title,
                });

                messages::success(&format!("Found documentation in {}", git_cfg.name));
            } else {
                messages::status(&format!("- No index.rst found in {}", git_cfg.name));
            }
        }

        Ok(doc_sources)
    }

    /// Get the combined default and user config directories (excludes per-git custom dirs)
    ///
    /// Returns directories that apply to all repositories:
    /// 1. Built-in defaults: docs, doc, documentation, documents, . (root)
    /// 2. User config directories (from config.toml, comma-separated)
    fn get_default_search_dirs(user_config: Option<&UserConfig>) -> Vec<String> {
        let mut dirs = Vec::new();
        let mut seen = HashSet::new();

        // Built-in defaults
        let defaults = vec!["docs", "doc", "documentation", "documents", "."];
        for dir in defaults {
            if seen.insert(dir.to_string()) {
                dirs.push(dir.to_string());
            }
        }

        // User config directories (parse comma-separated string)
        if let Some(user_cfg) = user_config {
            if let Some(user_dirs_str) = &user_cfg.documentation_dirs {
                for dir in user_dirs_str.split(',') {
                    let trimmed = dir.trim();
                    if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
                        dirs.push(trimmed.to_string());
                    }
                }
            }
        }

        dirs
    }

    /// Build the combined, deduplicated search directory list for a git repository
    ///
    /// Combines:
    /// 1. Built-in defaults: docs, doc, documentation, documents, . (root)
    /// 2. User config directories (from config.toml, comma-separated)
    /// 3. Per-repository custom directory (from sdk.yml)
    ///
    /// Empty strings are filtered out and duplicates are removed while preserving order.
    fn build_search_dirs(
        &self,
        git_cfg: &crate::config::GitConfig,
        user_config: Option<&UserConfig>,
    ) -> Vec<String> {
        let mut dirs = Vec::new();
        let mut seen = HashSet::new();

        // Built-in defaults
        let defaults = vec!["docs", "doc", "documentation", "documents", "."];
        for dir in defaults {
            if seen.insert(dir.to_string()) {
                dirs.push(dir.to_string());
            }
        }

        // User config directories (parse comma-separated string)
        if let Some(user_cfg) = user_config {
            if let Some(user_dirs_str) = &user_cfg.documentation_dirs {
                for dir in user_dirs_str.split(',') {
                    let trimmed = dir.trim();
                    if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
                        dirs.push(trimmed.to_string());
                    }
                }
            }
        }

        // Per-repository custom directory
        if let Some(custom_dir) = &git_cfg.documentation_dir {
            let trimmed = custom_dir.trim();
            if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
                dirs.push(trimmed.to_string());
            }
        }

        dirs
    }

    /// Extract title from index.rst file
    fn extract_title_from_index_rst(&self, index_rst_path: &Path) -> Option<String> {
        let content = fs::read_to_string(index_rst_path).ok()?;

        // Look for the first non-empty line that looks like a title
        // Sphinx titles are typically followed by underline characters
        let lines: Vec<&str> = content.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            let line = line.trim();
            if !line.is_empty() && i + 1 < lines.len() {
                let next_line = lines[i + 1].trim();
                // Check if next line is an underline (====, ----, etc.)
                if !next_line.is_empty()
                    && next_line.chars().all(|c| "=-*^~#".contains(c))
                    && next_line.len() >= line.len()
                {
                    return Some(line.to_string());
                }
            }
        }

        None
    }

    /// Create the unified documentation structure
    pub fn create_unified_docs(
        &self,
        doc_sources: &[DocSource],
        theme: &str,
        force: bool,
        use_symlinks: bool,
    ) -> io::Result<()> {
        // Check if documentation directory exists
        if self.docs_output_path.exists() && !force {
            messages::info("Documentation directory already exists. Use --force to recreate.");
            return Ok(());
        }

        // Remove existing documentation if force is enabled
        if self.docs_output_path.exists() && force {
            fs::remove_dir_all(&self.docs_output_path)?;
        }

        // Create documentation directory structure
        fs::create_dir_all(&self.docs_output_path)?;
        fs::create_dir_all(self.docs_output_path.join("_static"))?;
        fs::create_dir_all(self.docs_output_path.join("_templates"))?;

        // Create conf.py
        self.create_conf_py(theme, doc_sources)?;

        // Create master index.rst
        self.create_master_index_rst(doc_sources)?;

        // Copy and organize repository documentation
        self.copy_repository_docs(doc_sources, use_symlinks)?;

        messages::success(&format!(
            "Created unified documentation structure in {}",
            self.docs_output_path.display()
        ));

        Ok(())
    }

    /// Create Sphinx configuration file
    fn create_conf_py(&self, theme: &str, _doc_sources: &[DocSource]) -> io::Result<()> {
        let conf_py_path = self.docs_output_path.join("conf.py");
        let mut file = fs::File::create(&conf_py_path)?;

        let project_name = self
            .workspace_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("SDK Documentation");

        writeln!(
            file,
            "# Configuration file for the Sphinx documentation builder."
        )?;
        writeln!(file, "# Auto-generated by cim")?;
        writeln!(file)?;
        writeln!(file, "import os")?;
        writeln!(file, "import sys")?;
        writeln!(file)?;
        writeln!(file, "# Project information")?;
        writeln!(file, "project = '{}'", project_name)?;
        writeln!(file, "copyright = '2025, Analog Devices'")?;
        writeln!(file, "author = 'SDK Team'")?;
        writeln!(file)?;
        writeln!(file, "# General configuration")?;
        writeln!(file, "extensions = [")?;
        writeln!(file, "    'sphinx.ext.autodoc',")?;
        writeln!(file, "    'sphinx.ext.viewcode',")?;
        writeln!(file, "    'sphinx.ext.napoleon',")?;
        writeln!(file, "    'sphinx.ext.intersphinx',")?;
        writeln!(file, "]")?;
        writeln!(file)?;
        writeln!(file, "templates_path = ['_templates']")?;
        writeln!(
            file,
            "exclude_patterns = ['_build', 'Thumbs.db', '.DS_Store']"
        )?;
        writeln!(file)?;
        writeln!(file, "# HTML output")?;
        writeln!(file, "html_theme = '{}'", theme)?;
        writeln!(file, "html_static_path = ['_static']")?;
        writeln!(file)?;
        writeln!(file, "# Intersphinx mapping")?;
        writeln!(file, "intersphinx_mapping = {{")?;
        writeln!(file, "    'python': ('https://docs.python.org/3', None),")?;
        writeln!(file, "}}")?;

        Ok(())
    }

    /// Create master index.rst file
    fn create_master_index_rst(&self, doc_sources: &[DocSource]) -> io::Result<()> {
        let index_rst_path = self.docs_output_path.join("index.rst");
        let mut file = fs::File::create(&index_rst_path)?;

        let project_name = self
            .workspace_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("SDK Documentation");

        // Write title
        writeln!(file, "{}", project_name)?;
        writeln!(file, "{}", "=".repeat(project_name.len()))?;
        writeln!(file)?;
        writeln!(
            file,
            "Welcome to the unified SDK documentation. This documentation is"
        )?;
        writeln!(
            file,
            "automatically generated from all repositories in the SDK workspace."
        )?;
        writeln!(file)?;

        // Write table of contents
        writeln!(file, ".. toctree::")?;
        writeln!(file, "   :maxdepth: 2")?;
        writeln!(file, "   :caption: Repository Documentation:")?;
        writeln!(file)?;

        for doc_source in doc_sources {
            writeln!(file, "   {}/index", doc_source.repo_name)?;
        }

        writeln!(file)?;
        writeln!(file, "Indices and tables")?;
        writeln!(file, "==================")?;
        writeln!(file)?;
        writeln!(file, "* :ref:`genindex`")?;
        writeln!(file, "* :ref:`modindex`")?;
        writeln!(file, "* :ref:`search`")?;

        Ok(())
    }

    /// Copy repository documentation to unified structure
    fn copy_repository_docs(
        &self,
        doc_sources: &[DocSource],
        use_symlinks: bool,
    ) -> io::Result<()> {
        for doc_source in doc_sources {
            let target_dir = self.docs_output_path.join(&doc_source.repo_name);

            if use_symlinks {
                // Create symlink to the entire docs directory
                if target_dir.exists() {
                    fs::remove_dir_all(&target_dir)?;
                }

                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(&doc_source.docs_path, &target_dir)?;
                    messages::success(&format!(
                        "Created symlink to documentation from {}",
                        doc_source.repo_name
                    ));
                }

                #[cfg(not(unix))]
                {
                    // Fallback to copying on non-Unix systems
                    fs::create_dir_all(&target_dir)?;
                    Self::copy_dir_recursive(&doc_source.docs_path, &target_dir)?;
                    messages::success(&format!(
                        "Copied documentation from {} (symlinks not supported on this platform)",
                        doc_source.repo_name
                    ));
                }
            } else {
                // Create target directory
                fs::create_dir_all(&target_dir)?;

                // Copy documentation
                Self::copy_dir_recursive(&doc_source.docs_path, &target_dir)?;
                messages::success(&format!(
                    "Copied documentation from {}",
                    doc_source.repo_name
                ));
            }
        }

        Ok(())
    }

    /// Recursively copy directory contents
    fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
        if !src.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Source is not a directory",
            ));
        }

        fs::create_dir_all(dst)?;

        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if src_path.is_dir() {
                Self::copy_dir_recursive(&src_path, &dst_path)?;
            } else {
                fs::copy(&src_path, &dst_path)?;
            }
        }

        Ok(())
    }

    /// Build documentation using Sphinx
    pub fn build_docs(&self, format: &str) -> io::Result<()> {
        match format {
            "pdf" => self.build_pdf_docs(),
            _ => self.build_standard_docs(format),
        }
    }

    fn build_standard_docs(&self, format: &str) -> io::Result<()> {
        let build_dir = self.docs_output_path.join("_build").join(format);
        fs::create_dir_all(&build_dir)?;

        let project_name = self
            .workspace_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("SDK");

        messages::status(&format!(
            "Building unified documentation for {} in {} format...",
            project_name, format
        ));

        // Get command and environment setup for virtual environment support
        let (sphinx_cmd, env_vars) = self.get_python_command("sphinx-build");

        let mut command = std::process::Command::new(&sphinx_cmd);
        command
            .args(["-b", format])
            .arg(&self.docs_output_path)
            .arg(&build_dir);

        // Apply environment variables for virtual environment
        for (key, value) in env_vars {
            command.env(&key, &value);
        }

        let output = match command.output() {
            Ok(output) => output,
            Err(e) => {
                let venv_hint = if self.venv_exists() {
                    "\nVirtual environment detected but sphinx-build not found in venv."
                } else {
                    "\nConsider running 'cim install pip' to set up Python packages in a virtual environment."
                };
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Failed to execute 'sphinx-build' command:\n{}.{}\nMake sure Sphinx is installed and in your PATH.", e, venv_hint)
                ));
            }
        };

        if output.status.success() {
            messages::success("Documentation built successfully!");
            messages::status(&format!("Output: {}", build_dir.display()));
        } else {
            messages::error("Documentation build failed:");
            messages::error(&String::from_utf8_lossy(&output.stderr));
            return Err(io::Error::other("Sphinx build failed"));
        }

        Ok(())
    }

    fn build_pdf_docs(&self) -> io::Result<()> {
        let project_name = self
            .workspace_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("SDK");

        messages::status(&format!(
            "Building unified documentation for {} in PDF format...",
            project_name
        ));

        // Step 1: Build LaTeX files using Sphinx
        let latex_build_dir = self.docs_output_path.join("_build").join("latex");
        fs::create_dir_all(&latex_build_dir)?;

        messages::status("Step 1/2: Generating LaTeX files...");

        let (sphinx_cmd, env_vars) = self.get_python_command("sphinx-build");

        let mut command = std::process::Command::new(&sphinx_cmd);
        command
            .args(["-b", "latex"])
            .arg(&self.docs_output_path)
            .arg(&latex_build_dir);

        // Apply environment variables for virtual environment
        for (key, value) in env_vars {
            command.env(&key, &value);
        }

        let output = match command.output() {
            Ok(output) => output,
            Err(e) => {
                let venv_hint = if self.venv_exists() {
                    "\nVirtual environment detected but sphinx-build not found in venv."
                } else {
                    "\nConsider running 'cim install pip' to set up Python packages in a virtual environment."
                };
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Failed to execute 'sphinx-build' command:\n{}.{}\nMake sure Sphinx is installed and in your PATH.", e, venv_hint)
                ));
            }
        };

        if !output.status.success() {
            messages::error("LaTeX generation failed:");
            messages::error(&String::from_utf8_lossy(&output.stderr));
            return Err(io::Error::other("LaTeX generation failed"));
        }

        // Step 2: Convert LaTeX to PDF using pdflatex
        messages::status("Step 2/2: Converting LaTeX to PDF...");

        // Find the main LaTeX file (usually named after the project)
        let tex_file = latex_build_dir.join(format!("{}.tex", project_name.to_lowercase()));
        if !tex_file.exists() {
            // Try some common names if the project-based name doesn't exist
            let alternatives = ["index.tex", "main.tex", "document.tex"];
            let found_file = alternatives
                .iter()
                .map(|name| latex_build_dir.join(name))
                .find(|path| path.exists());

            if found_file.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Could not find LaTeX file. Expected {} or common alternatives (index.tex, main.tex, document.tex) in {}", 
                           tex_file.display(), latex_build_dir.display())
                ));
            }
        }

        // Run pdflatex
        let mut pdf_command = std::process::Command::new("pdflatex");
        pdf_command
            .arg("-interaction=nonstopmode")
            .arg(&tex_file)
            .current_dir(&latex_build_dir);

        let pdf_output = match pdf_command.output() {
            Ok(output) => output,
            Err(e) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Failed to execute 'pdflatex' command: {}.\nMake sure LaTeX is installed (try: brew install --cask mactex on macOS)", e)
                ));
            }
        };

        if !pdf_output.status.success() {
            messages::error("PDF generation failed:");
            messages::error(&String::from_utf8_lossy(&pdf_output.stderr));
            messages::error("LaTeX output:");
            messages::error(&String::from_utf8_lossy(&pdf_output.stdout));
            return Err(io::Error::other("PDF generation failed"));
        }

        // Create pdf output directory and move the PDF file
        let pdf_build_dir = self.docs_output_path.join("_build").join("pdf");
        fs::create_dir_all(&pdf_build_dir)?;

        let pdf_source = latex_build_dir.join(format!("{}.pdf", project_name.to_lowercase()));
        let pdf_dest = pdf_build_dir.join(format!("{}.pdf", project_name.to_lowercase()));

        if pdf_source.exists() {
            fs::copy(&pdf_source, &pdf_dest)?;
            messages::success("PDF documentation built successfully!");
            messages::status(&format!("Output: {}", pdf_dest.display()));
        } else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("PDF file was not generated: {}", pdf_source.display()),
            ));
        }

        Ok(())
    }

    /// Serve documentation locally
    pub fn serve_docs(&self, host: &str, port: u16) -> io::Result<()> {
        let html_dir = self.docs_output_path.join("_build").join("html");

        if !html_dir.exists() {
            messages::status("HTML documentation not found. Building first...");
            self.build_docs("html")?;
        }

        messages::status(&format!(
            "Serving documentation at http://{}:{}",
            host, port
        ));
        messages::status("Press Ctrl+C to stop the server");

        // Get command and environment setup for virtual environment support
        let (python_cmd, env_vars) = self.get_python_command("python3");

        let mut command = std::process::Command::new(&python_cmd);
        command
            .args(["-m", "http.server", &port.to_string()])
            .args(["--bind", host])
            .current_dir(&html_dir);

        // Apply environment variables for virtual environment
        for (key, value) in env_vars {
            command.env(&key, &value);
        }

        let mut child = command.spawn()?;

        // Wait for the child process
        let _ = child.wait()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_doc_manager_creation() {
        let temp_dir = tempdir().unwrap();
        let doc_manager = DocManager::new(temp_dir.path().to_path_buf());

        assert_eq!(doc_manager.workspace_path, temp_dir.path());
        assert_eq!(
            doc_manager.docs_output_path,
            temp_dir.path().join("documentation")
        );
    }

    #[test]
    fn test_discover_doc_sources() {
        let temp_dir = tempdir().unwrap();
        let workspace_path = temp_dir.path();

        // Create a test repository with docs
        let repo_path = workspace_path.join("test-repo");
        let docs_path = repo_path.join("docs");
        fs::create_dir_all(&docs_path).unwrap();

        let index_content = "Test Documentation\n==================\n\nThis is test documentation.";
        fs::write(docs_path.join("index.rst"), index_content).unwrap();

        let sdk_config = SdkConfig {
            mirror: PathBuf::from("/tmp/mirror"),
            gits: vec![crate::config::GitConfig {
                name: "test-repo".to_string(),
                url: "https://example.com/test-repo.git".to_string(),
                commit: "main".to_string(),
                build_depends_on: None,
                git_depends_on: None,
                build: None,
                documentation_dir: None,
            }],
            toolchains: None,
            copy_files: None,
            install: None,
            makefile_include: None,
            envsetup: None,
            test: None,
            clean: None,
            build: None,
            flash: None,
            variables: None,
        };

        let doc_manager = DocManager::new(workspace_path.to_path_buf());
        let doc_sources = doc_manager
            .discover_doc_sources(&sdk_config, None, false)
            .unwrap();

        assert_eq!(doc_sources.len(), 1);
        assert_eq!(doc_sources[0].repo_name, "test-repo");
        assert_eq!(doc_sources[0].title, "Test Documentation");
    }

    #[test]
    fn test_extract_title_from_index_rst() {
        let temp_dir = tempdir().unwrap();
        let index_path = temp_dir.path().join("index.rst");

        let content = "My Project Title\n================\n\nThis is the content.";
        fs::write(&index_path, content).unwrap();

        let doc_manager = DocManager::new(temp_dir.path().to_path_buf());
        let title = doc_manager.extract_title_from_index_rst(&index_path);

        assert_eq!(title, Some("My Project Title".to_string()));
    }

    #[test]
    fn test_copy_dir_recursive() {
        let temp_dir = tempdir().unwrap();
        let src_dir = temp_dir.path().join("src");
        let dst_dir = temp_dir.path().join("dst");

        // Create source structure
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("file1.txt"), "content1").unwrap();

        let subdir = src_dir.join("subdir");
        fs::create_dir_all(&subdir).unwrap();
        fs::write(subdir.join("file2.txt"), "content2").unwrap();

        let _doc_manager = DocManager::new(temp_dir.path().to_path_buf());
        DocManager::copy_dir_recursive(&src_dir, &dst_dir).unwrap();

        // Verify copy
        assert!(dst_dir.join("file1.txt").exists());
        assert!(dst_dir.join("subdir").join("file2.txt").exists());

        let content = fs::read_to_string(dst_dir.join("file1.txt")).unwrap();
        assert_eq!(content, "content1");
    }
}
