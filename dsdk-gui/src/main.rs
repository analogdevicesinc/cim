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

use anyhow::Result;
use eframe::egui;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

/// Load workspace prefix from user config file
/// Returns the configured prefix or "dsdk-" as default
fn load_workspace_prefix() -> String {
    // Try to load from ~/.config/cim/config.toml
    let config_path = dirs::config_dir()
        .map(|d| d.join("cim").join("config.toml"))
        .filter(|p| p.exists());

    if let Some(path) = config_path {
        if let Ok(content) = std::fs::read_to_string(path) {
            // Simple TOML parsing for workspace_prefix line
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with("workspace_prefix") && line.contains('=') {
                    if let Some(value_part) = line.split('=').nth(1) {
                        let value = value_part.trim().trim_matches('"').trim_matches('\'');
                        return value.to_string();
                    }
                }
            }
        }
    }

    // Default prefix
    "dsdk-".to_string()
}

#[derive(Default)]
struct CimInstallerApp {
    // Configuration
    target_url: String, // URL or local path for fetching targets
    workspace_path: String,
    force_wipe: bool,
    use_mirror: bool,

    // UI State
    available_targets: Vec<String>,
    selected_target_index: usize,
    available_versions: Vec<String>, // New: available versions for selected target
    selected_version_index: usize,   // New: selected version index
    is_fetching: bool,
    is_fetching_versions: bool, // New: fetching versions state
    is_installing: bool,

    // Logging
    log_output: String,

    // Background operations
    fetch_receiver: Option<mpsc::Receiver<Result<Vec<String>>>>,
    version_receiver: Option<mpsc::Receiver<Result<Vec<String>>>>, // New: version fetching
    install_receiver: Option<mpsc::Receiver<String>>,
}

impl CimInstallerApp {
    fn new() -> Self {
        let home_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("dsdk-workspace")
            .to_string_lossy()
            .to_string();

        let mut app = Self {
            target_url: "https://github.com/joabech/cim-manifests".to_string(),
            workspace_path: home_dir,
            force_wipe: false,
            use_mirror: true, // Default to using mirror
            available_targets: Vec::new(),
            selected_target_index: 0,
            available_versions: Vec::new(),
            selected_version_index: 0,
            is_fetching: false,
            is_fetching_versions: false,
            is_installing: false,
            log_output: "Starting up...\n".to_string(),
            fetch_receiver: None,
            version_receiver: None,
            install_receiver: None,
        };

        // Auto-fetch targets on startup
        app.fetch_targets();
        app
    }

    fn fetch_targets(&mut self) {
        if self.is_fetching {
            return;
        }

        self.is_fetching = true;
        self.log_output
            .push_str("Discovering available targets...\n");

        let target_url = self.target_url.clone();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = Self::call_cli_list_targets(&target_url);
            let _ = tx.send(result);
        });

        self.fetch_receiver = Some(rx);
    }

    fn call_cli_list_targets(config_source: &str) -> Result<Vec<String>> {
        let mut cmd = Command::new("cim");
        cmd.arg("list-targets");

        if !config_source.is_empty() {
            cmd.args(["--source", config_source]);
        }

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("CLI command failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut targets = Vec::new();

        // Parse the CLI output to extract target names
        for line in stdout.lines() {
            if let Some(target) = line.strip_prefix("  - ") {
                targets.push(target.to_string());
            }
        }

        Ok(targets)
    }

    fn fetch_versions_for_target(&mut self, target: &str) {
        if self.is_fetching_versions {
            return;
        }

        self.is_fetching_versions = true;
        self.log_output
            .push_str(&format!("Fetching versions for target: {}...\n", target));

        // Get workspace prefix from config, default to "dsdk-"
        let prefix = load_workspace_prefix();

        // Update workspace path to use {prefix}{target-name} pattern
        let workspace_name = format!("{}{}", prefix, target);
        self.workspace_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(&workspace_name)
            .to_string_lossy()
            .to_string();

        let target_name = target.to_string();
        let target_url = self.target_url.clone();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = Self::call_cli_list_target_versions(&target_url, &target_name);
            let _ = tx.send(result);
        });

        self.version_receiver = Some(rx);
    }

    fn call_cli_list_target_versions(config_source: &str, target: &str) -> Result<Vec<String>> {
        let mut cmd = Command::new("cim");
        cmd.args(["list-targets", "--target", target]);

        if !config_source.is_empty() {
            cmd.args(["--source", config_source]);
        }

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("CLI command failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut versions = Vec::new();

        // Parse the CLI output to extract version names
        for line in stdout.lines() {
            if let Some(version) = line.strip_prefix("  - ") {
                versions.push(version.to_string());
            }
        }

        Ok(versions)
    }

    fn install_project(&mut self) {
        if self.is_installing {
            return;
        }

        if self.available_targets.is_empty() {
            return;
        }

        let selected_target = &self.available_targets[self.selected_target_index];
        let selected_version = if !self.available_versions.is_empty() {
            let version = &self.available_versions[self.selected_version_index];
            // Don't use version if "Latest" is selected
            if version == "Latest" {
                None
            } else {
                Some(version)
            }
        } else {
            None
        };

        self.is_installing = true;

        if let Some(version) = selected_version {
            self.log_output.push_str(&format!(
                "Installing target: {} version: {}\n",
                selected_target, version
            ));
        } else {
            self.log_output.push_str(&format!(
                "Installing target: {} (latest)\n",
                selected_target
            ));
        }

        let target_name = selected_target.clone();
        let target_url = self.target_url.clone();
        let workspace_path = self.workspace_path.clone();
        let version_name = selected_version.cloned();
        let force_wipe = self.force_wipe;
        let use_mirror = self.use_mirror;

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            // Call cim init with new command structure
            let mut args = vec![
                "init",
                "--target",
                &target_name,
                "--workspace",
                &workspace_path,
            ];

            // Add --source
            if !target_url.is_empty() {
                args.extend_from_slice(&["--source", &target_url]);
            }

            // Add --version if specified
            if let Some(version) = &version_name {
                args.extend_from_slice(&["--version", version]);
            }

            if force_wipe {
                args.push("--force");
            }

            if !use_mirror {
                args.push("--no-mirror");
            }

            Self::run_cim_command(&args, tx);
        });

        self.install_receiver = Some(rx);
    }

    fn run_cim_command(args: &[&str], tx: mpsc::Sender<String>) {
        match Command::new("cim")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                // Stream stdout
                if let Some(stdout) = child.stdout.take() {
                    let tx_stdout = tx.clone();
                    thread::spawn(move || {
                        let reader = BufReader::new(stdout);
                        for line in reader.lines().map_while(Result::ok) {
                            let _ = tx_stdout.send(format!("{}\n", line));
                        }
                    });
                }

                // Stream stderr
                if let Some(stderr) = child.stderr.take() {
                    let tx_stderr = tx.clone();
                    thread::spawn(move || {
                        let reader = BufReader::new(stderr);
                        for line in reader.lines().map_while(Result::ok) {
                            let _ = tx_stderr.send(format!("ERROR: {}\n", line));
                        }
                    });
                }

                // Wait for process to complete
                match child.wait() {
                    Ok(status) => {
                        if status.success() {
                            let _ = tx.send("Installation completed successfully!\n".to_string());
                        } else {
                            let _ = tx.send(format!(
                                "Installation failed with exit code: {:?}\n",
                                status.code()
                            ));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(format!("Failed to wait for process: {}\n", e));
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(format!("Failed to execute cim: {}\n", e));
            }
        }
    }

    fn check_background_tasks(&mut self) {
        // Check fetch results
        if let Some(receiver) = &self.fetch_receiver {
            if let Ok(result) = receiver.try_recv() {
                self.is_fetching = false;
                self.fetch_receiver = None;

                match result {
                    Ok(items) => {
                        self.available_targets = items;
                        self.log_output
                            .push_str(&format!("Found {} targets\n", self.available_targets.len()));

                        // Auto-fetch versions for the first target if available
                        if !self.available_targets.is_empty() {
                            let first_target = self.available_targets[0].clone();
                            self.fetch_versions_for_target(&first_target);
                        }
                    }
                    Err(e) => {
                        self.log_output
                            .push_str(&format!("Error discovering targets: {}\n", e));
                    }
                }
            }
        }

        // Check version fetch results
        if let Some(receiver) = &self.version_receiver {
            if let Ok(result) = receiver.try_recv() {
                self.is_fetching_versions = false;
                self.version_receiver = None;

                match result {
                    Ok(versions) => {
                        if versions.is_empty() {
                            self.available_versions = versions;
                            self.log_output.push_str("No versions found for target\n");
                        } else {
                            // Add "Latest" as first option when versions are available
                            let mut versions_with_latest = vec!["Latest".to_string()];
                            versions_with_latest.extend(versions);
                            self.available_versions = versions_with_latest;
                            self.log_output.push_str(&format!(
                                "Found {} versions (plus Latest option)\n",
                                self.available_versions.len() - 1
                            ));
                        }
                        self.selected_version_index = 0; // Reset selection to "Latest" or first item
                    }
                    Err(e) => {
                        self.log_output
                            .push_str(&format!("Error fetching versions: {}\n", e));
                        self.available_versions.clear();
                    }
                }
            }
        }

        // Check install results
        if let Some(receiver) = self.install_receiver.take() {
            let mut should_keep_receiver = true;
            while let Ok(message) = receiver.try_recv() {
                self.log_output.push_str(&message);
                if message.contains("Installation completed")
                    || message.contains("Installation failed")
                {
                    self.is_installing = false;
                    should_keep_receiver = false;
                }
            }
            if should_keep_receiver {
                self.install_receiver = Some(receiver);
            }
        }
    }
}

impl eframe::App for CimInstallerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for background task completion
        self.check_background_tasks();

        // Request repaint for smooth updates
        ctx.request_repaint();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("🚀 Code in Motion Installer");
            ui.add_space(10.0);

            // Top section with fixed height for configuration and controls
            ui.vertical(|ui| {
                // Configuration section
                ui.group(|ui| {
                    ui.label("Configuration");
                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.label("Config Source:");
                        ui.text_edit_singleline(&mut self.target_url);

                        let refresh_button = if self.is_fetching {
                            egui::Button::new("Discovering...")
                        } else {
                            egui::Button::new("Refresh Targets")
                        };

                        if ui
                            .add(
                                refresh_button.stroke(egui::Stroke::new(1.0, egui::Color32::BLACK)),
                            )
                            .clicked()
                            && !self.is_fetching
                        {
                            self.fetch_targets();
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Workspace Path:");
                        ui.text_edit_singleline(&mut self.workspace_path);

                        if ui
                            .add(
                                egui::Button::new("Browse...")
                                    .stroke(egui::Stroke::new(1.0, egui::Color32::BLACK)),
                            )
                            .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                self.workspace_path = path.display().to_string();
                            }
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.force_wipe, "Force wipe existing workspace");
                    });

                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.use_mirror, "Use mirror");
                    });
                });

                ui.add_space(10.0);

                // Target selection section
                ui.group(|ui| {
                    ui.label("Target Selection");
                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.label("Target:");

                        if self.available_targets.is_empty() {
                            ui.label("No targets available. Click Refresh to discover.");
                        } else {
                            let old_target_index = self.selected_target_index;

                            // Ensure selected_target_index is within bounds
                            if self.selected_target_index >= self.available_targets.len() {
                                self.selected_target_index = 0;
                            }

                            egui::ComboBox::from_id_salt("sdk_target_selection_combo")
                                .selected_text(&self.available_targets[self.selected_target_index])
                                .width(200.0)
                                .show_ui(ui, |ui| {
                                    for (i, target) in self.available_targets.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut self.selected_target_index,
                                            i,
                                            target,
                                        );
                                    }
                                });

                            // If target selection changed, fetch versions for new target
                            if old_target_index != self.selected_target_index {
                                // Clear existing versions and reset index while fetching new ones
                                self.available_versions.clear();
                                self.selected_version_index = 0;

                                let new_target =
                                    self.available_targets[self.selected_target_index].clone();
                                self.log_output
                                    .push_str(&format!("Target changed to: {}\n", new_target));
                                self.fetch_versions_for_target(&new_target);
                            }
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Version:");

                        if self.available_versions.is_empty() {
                            if self.is_fetching_versions {
                                ui.label("Fetching versions...");
                            } else {
                                ui.label("Latest (no versions available)");
                            }
                        } else {
                            // Ensure selected_version_index is within bounds
                            if self.selected_version_index >= self.available_versions.len() {
                                self.selected_version_index = 0;
                            }

                            let old_version_index = self.selected_version_index;

                            egui::ComboBox::from_id_salt("sdk_version_selection_combo")
                                .selected_text(
                                    &self.available_versions[self.selected_version_index],
                                )
                                .width(200.0)
                                .show_ui(ui, |ui| {
                                    for (i, version) in self.available_versions.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut self.selected_version_index,
                                            i,
                                            version,
                                        );
                                    }
                                });

                            // Log version changes for debugging
                            if old_version_index != self.selected_version_index {
                                self.log_output.push_str(&format!(
                                    "Version changed to: {}\n",
                                    self.available_versions[self.selected_version_index]
                                ));
                            }
                        }
                    });
                });

                ui.add_space(10.0);

                // Action buttons
                ui.horizontal(|ui| {
                    let install_enabled = !self.available_targets.is_empty()
                        && !self.is_installing
                        && !self.workspace_path.trim().is_empty();

                    let install_button = if self.is_installing {
                        egui::Button::new("Installing...")
                    } else {
                        egui::Button::new("🔧 Install Target")
                    };

                    if ui
                        .add_enabled(
                            install_enabled,
                            install_button.stroke(egui::Stroke::new(1.0, egui::Color32::BLACK)),
                        )
                        .clicked()
                    {
                        self.install_project();
                    }

                    // Placeholder build button
                    ui.add_enabled(
                        false,
                        egui::Button::new("🏗️ Build (Coming Soon)")
                            .stroke(egui::Stroke::new(1.0, egui::Color32::BLACK)),
                    );
                });

                ui.add_space(10.0);
            });

            // Log output section - takes all remaining space
            ui.group(|ui| {
                ui.label("Log Output");
                ui.separator();

                // Calculate available space for the text area
                let available_rect = ui.available_rect_before_wrap();
                let button_height = 40.0; // Approximate height for all buttons
                let text_area_height = available_rect.height() - button_height - 15.0; // Extra padding

                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .min_scrolled_height(text_area_height)
                    .max_height(text_area_height)
                    .show(ui, |ui| {
                        let desired_size = egui::vec2(ui.available_width(), text_area_height);
                        ui.add_sized(
                            desired_size,
                            egui::TextEdit::multiline(&mut self.log_output)
                                .desired_width(ui.available_width()),
                        );
                    });

                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new("Clear Log")
                                .stroke(egui::Stroke::new(1.0, egui::Color32::BLACK)),
                        )
                        .clicked()
                    {
                        self.log_output.clear();
                    }

                    if ui
                        .add(
                            egui::Button::new("📋 Copy to Clipboard")
                                .stroke(egui::Stroke::new(1.0, egui::Color32::BLACK)),
                        )
                        .clicked()
                    {
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            if let Err(e) = clipboard.set_text(&self.log_output) {
                                eprintln!("Failed to copy to clipboard: {}", e);
                            }
                        }
                    }

                    if ui
                        .add(
                            egui::Button::new("💾 Save Log")
                                .stroke(egui::Stroke::new(1.0, egui::Color32::BLACK)),
                        )
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("cim-log.txt")
                            .add_filter("Text files", &["txt"])
                            .add_filter("All files", &["*"])
                            .save_file()
                        {
                            if let Err(e) = std::fs::write(&path, &self.log_output) {
                                eprintln!("Failed to save log file: {}", e);
                            }
                        }
                    }
                });
            });
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([600.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Code in Motion Installer",
        options,
        Box::new(|_cc| Ok(Box::new(CimInstallerApp::new()))),
    )
}
