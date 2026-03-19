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
use ratatui::prelude::*;

use tokio::sync::mpsc;

use crate::cim;
use crate::events::handle_event;
use crate::ui::draw;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    SourceInput,
    #[allow(dead_code)]
    TargetList,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PopupFocus {
    VersionDropdown,
    WorkspaceInput,
    MatchInput,
    NoMirror,
    Force,
    Verbose,
    Install,
    Full,
    NoSudo,
    Symlink,
    Yes,
    CertValidation,
    CancelButton,
    CreateButton,
}

pub struct InitPopupState {
    pub source: String,
    pub targets: Vec<String>,
    pub selected_target: usize,
    pub versions: Vec<String>,
    pub selected_version: usize,
    pub version_dropdown_open: bool,

    pub workspace_input: String,
    pub match_input: String,

    pub no_mirror: bool,
    pub force: bool,
    pub verbose: bool,
    pub install: bool,
    pub full: bool,
    pub no_sudo: bool,
    pub symlink: bool,
    pub yes: bool,

    pub selected_cert: usize,
    pub cert_dropdown_open: bool,

    pub focus: PopupFocus,
    pub is_loading_versions: bool,
}

impl InitPopupState {
    pub fn new(source: String, targets: Vec<String>, selected_target: usize) -> Self {
        Self {
            source,
            targets,
            selected_target,
            versions: Vec::new(),
            selected_version: 0,
            version_dropdown_open: false,
            workspace_input: String::new(),
            match_input: String::new(),
            no_mirror: false,
            force: false,
            verbose: false,
            install: true,
            full: false,
            no_sudo: false,
            symlink: false,
            yes: false,
            selected_cert: 0,
            cert_dropdown_open: false,
            focus: PopupFocus::VersionDropdown,
            is_loading_versions: true,
        }
    }

    pub fn next_focus(&mut self) {
        self.focus = match self.focus {
            PopupFocus::VersionDropdown => PopupFocus::WorkspaceInput,
            PopupFocus::WorkspaceInput => PopupFocus::MatchInput,
            PopupFocus::MatchInput => PopupFocus::NoMirror,
            PopupFocus::NoMirror => PopupFocus::Force,
            PopupFocus::Force => PopupFocus::Verbose,
            PopupFocus::Verbose => PopupFocus::Install,
            PopupFocus::Install => PopupFocus::Full,
            PopupFocus::Full => PopupFocus::NoSudo,
            PopupFocus::NoSudo => PopupFocus::Symlink,
            PopupFocus::Symlink => PopupFocus::Yes,
            PopupFocus::Yes => PopupFocus::CertValidation,
            PopupFocus::CertValidation => PopupFocus::CancelButton,
            PopupFocus::CancelButton => PopupFocus::CreateButton,
            PopupFocus::CreateButton => PopupFocus::VersionDropdown,
        };
    }

    pub fn previous_focus(&mut self) {
        self.focus = match self.focus {
            PopupFocus::VersionDropdown => PopupFocus::CreateButton,
            PopupFocus::WorkspaceInput => PopupFocus::VersionDropdown,
            PopupFocus::MatchInput => PopupFocus::WorkspaceInput,
            PopupFocus::NoMirror => PopupFocus::MatchInput,
            PopupFocus::Force => PopupFocus::NoMirror,
            PopupFocus::Verbose => PopupFocus::Force,
            PopupFocus::Install => PopupFocus::Verbose,
            PopupFocus::Full => PopupFocus::Install,
            PopupFocus::NoSudo => PopupFocus::Full,
            PopupFocus::Symlink => PopupFocus::NoSudo,
            PopupFocus::Yes => PopupFocus::Symlink,
            PopupFocus::CertValidation => PopupFocus::Yes,
            PopupFocus::CancelButton => PopupFocus::CertValidation,
            PopupFocus::CreateButton => PopupFocus::CancelButton,
        };
    }

    pub fn get_selected_version(&self) -> Option<String> {
        if self.selected_version == 0 || self.versions.is_empty() {
            None
        } else {
            Some(self.versions[self.selected_version - 1].clone())
        }
    }

    pub fn get_cert_validation(&self) -> &'static str {
        match self.selected_cert {
            0 => "strict",
            1 => "relaxed",
            2 => "auto",
            _ => "strict",
        }
    }
}

pub struct App {
    pub source_input: String,
    pub targets: Vec<String>,
    pub selected_target: usize,
    pub target_scroll_offset: usize,  // For scrolling target list
    pub focus: Focus,
    pub is_loading: bool,
    pub error_message: Option<String>,
    pub init_popup: Option<InitPopupState>,
    pub status_message: Option<String>,
    pub output_text: String,
    pub output_scroll: u16,           // Vertical scroll offset for output pane

    // Async channels
    target_rx: Option<mpsc::Receiver<Result<Vec<String>>>>,
    version_rx: Option<mpsc::Receiver<Result<Vec<String>>>>,
    output_rx: Option<mpsc::Receiver<String>>,
}

impl App {
    pub fn new() -> Self {
        // Use the remote manifest repo that has versions
        let default_source = "https://github.com/joabech/cim-manifests.git".to_string();
        
        let mut app = Self {
            source_input: default_source,
            targets: Vec::new(),
            selected_target: 0,
            target_scroll_offset: 0,
            focus: Focus::TargetList,
            is_loading: false,
            error_message: None,
            init_popup: None,
            status_message: None,
            target_rx: None,
            version_rx: None,
            output_rx: None,
            output_text: String::new(),
            output_scroll: 0,
        };
        app.refresh_targets();
        app
    }

    pub async fn run(&mut self, terminal: &mut Terminal<impl Backend>) -> Result<()> {
        loop {
            // Draw UI
            terminal.draw(|f| draw(f, self))?;

            // Handle events
            if handle_event(self)? {
                return Ok(());
            }

            // Check async results
            self.check_async_results().await;
            
            // Check for command output
            self.check_output();
        }
    }

    fn spawn_fetch_targets(&mut self, tx: mpsc::Sender<Result<Vec<String>>>) {
        if self.is_loading {
            return;
        }

        self.is_loading = true;
        self.error_message = None;
        let source = self.source_input.clone();

        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || cim::fetch_targets(&source)).await;
            let _ = tx.send(result.unwrap_or_else(|e| Err(anyhow::anyhow!("Task panicked: {}", e)))).await;
        });
    }

    pub fn refresh_targets(&mut self) {
        let (tx, rx) = mpsc::channel(10);
        self.target_rx = Some(rx);
        self.spawn_fetch_targets(tx);
    }

    async fn check_async_results(&mut self) {
        // Check target fetch results
        if let Some(rx) = &mut self.target_rx {

            if let Ok(result) = rx.try_recv() {

                self.is_loading = false;
                self.target_rx = None;

                match result {
                    Ok(targets) => {
                        self.targets = targets;
                        if self.selected_target >= self.targets.len() && !self.targets.is_empty() {
                            self.selected_target = 0;
                        }
                        self.status_message = Some(format!("Loaded {} targets", self.targets.len()));
                    }
                    Err(e) => {
                        self.error_message = Some(e.to_string());
                        self.status_message = Some(format!("Error: {}", e));
                    }
                }
            }
        }

        // Check version fetch results for popup
        if let Some(rx) = &mut self.version_rx {
            if let Ok(result) = rx.try_recv() {
                self.version_rx = None;
                if let Some(popup) = &mut self.init_popup {
                    popup.is_loading_versions = false;
                    match result {
                        Ok(versions) => {
                            popup.versions = versions;
                        }
                        Err(e) => {
                            popup.versions = Vec::new();
                            self.status_message = Some(format!("Failed to fetch versions: {}", e));
                        }
                    }
                }
            }
        }
    }

    pub fn select_next(&mut self) {
        if !self.targets.is_empty() && self.selected_target + 1 < self.targets.len() {
            self.selected_target += 1;
            // Adjust scroll offset if selection goes below visible area
            let visible_count = 10.min(self.targets.len());
            if self.selected_target >= self.target_scroll_offset + visible_count {
                self.target_scroll_offset = self.selected_target.saturating_sub(visible_count - 1);
            }
        }
    }

    pub fn select_previous(&mut self) {
        self.selected_target = self.selected_target.saturating_sub(1);
        // Adjust scroll offset if selection goes above visible area
        if self.selected_target < self.target_scroll_offset {
            self.target_scroll_offset = self.selected_target;
        }
    }

    pub fn open_init_popup(&mut self) {
        if self.targets.is_empty() {
            return;
        }

        let popup = InitPopupState::new(
            self.source_input.clone(),
            self.targets.clone(),
            self.selected_target,
        );

        // Fetch versions for the selected target
        let target = self.targets[self.selected_target].clone();
        let source = self.source_input.clone();
        let (tx, rx) = mpsc::channel(10);
        self.version_rx = Some(rx);

        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || cim::fetch_versions(&source, &target)).await;
            let _ = tx.send(result.unwrap_or_else(|e| Err(anyhow::anyhow!("Task panicked: {}", e)))).await;
        });

        self.init_popup = Some(popup);
    }

    pub fn close_init_popup(&mut self) {
        self.init_popup = None;
        self.version_rx = None;
    }

    pub fn execute_init(&mut self) {
        if let Some(popup) = &self.init_popup {
            let target = popup.targets[popup.selected_target].clone();
            let source = popup.source.clone();
            let version = popup.get_selected_version();
            let workspace = if popup.workspace_input.is_empty() {
                None
            } else {
                Some(popup.workspace_input.clone())
            };
            let match_pattern = if popup.match_input.is_empty() {
                None
            } else {
                Some(popup.match_input.clone())
            };

            match cim::run_init(
                &target,
                &source,
                version.as_deref(),
                workspace.as_deref(),
                popup.no_mirror,
                popup.force,
                match_pattern.as_deref(),
                popup.verbose,
                popup.install,
                popup.full,
                popup.no_sudo,
                popup.symlink,
                popup.yes,
                Some(popup.get_cert_validation()),
            ) {
                Ok(mut child) => {
                    self.status_message = Some(format!("Initializing workspace for '{}'...", target));
                    self.close_init_popup();

                    // Clear previous output
                    self.output_text.clear();
                    self.output_text.push_str(&format!("=== Initializing workspace for '{}' ===\n\n", target));

                    // Create channel for output lines
                    let (tx, rx) = mpsc::channel::<String>(100);
                    self.output_rx = Some(rx);

                    // Spawn stdout reader
                    if let Some(stdout) = child.stdout.take() {
                        let tx_stdout = tx.clone();
                        tokio::spawn(async move {
                            use tokio::io::{AsyncBufReadExt, BufReader};
                            let reader = BufReader::new(stdout);
                            let mut lines = reader.lines();
                            while let Ok(Some(line)) = lines.next_line().await {
                                let _ = tx_stdout.send(line).await;
                            }
                        });
                    }

                    // Spawn stderr reader
                    if let Some(stderr) = child.stderr.take() {
                        let tx_stderr = tx.clone();
                        tokio::spawn(async move {
                            use tokio::io::{AsyncBufReadExt, BufReader};
                            let reader = BufReader::new(stderr);
                            let mut lines = reader.lines();
                            while let Ok(Some(line)) = lines.next_line().await {
                                let _ = tx_stderr.send(format!("ERROR: {}", line)).await;
                            }
                        });
                    }

                    // Spawn completion monitor
                    tokio::spawn(async move {
                        match child.wait().await {
                            Ok(status) => {
                                let msg = if status.success() {
                                    "\n=== Workspace initialized successfully ===".to_string()
                                } else {
                                    format!("\n=== Initialization failed with exit code: {:?} ===", status.code())
                                };
                                let _ = tx.send(msg).await;
                            }
                            Err(e) => {
                                let _ = tx.send(format!("\n=== Failed to wait for process: {} ===", e)).await;
                            }
                        }
                    });
                }
                Err(e) => {
                    self.status_message = Some(format!("Failed to start init: {}", e));
                }
            }
        }
    }

    fn check_output(&mut self) {
        if let Some(rx) = &mut self.output_rx {
            // Drain all available lines
            while let Ok(line) = rx.try_recv() {
                self.output_text.push_str(&line);
                self.output_text.push('\n');
            }
        }
    }

    // Output scrolling methods
    pub fn scroll_output_up(&mut self, lines: u16) {
        self.output_scroll = self.output_scroll.saturating_sub(lines);
    }

    pub fn scroll_output_down(&mut self, lines: u16) {
        let total_lines = self.output_text.lines().count() as u16;
        // Allow scrolling to show last line at top
        let max_scroll = total_lines.saturating_sub(1);
        self.output_scroll = (self.output_scroll + lines).min(max_scroll);
    }

    pub fn scroll_output_top(&mut self) {
        self.output_scroll = 0;
    }

    pub fn scroll_output_bottom(&mut self) {
        let total_lines = self.output_text.lines().count() as u16;
        self.output_scroll = total_lines.saturating_sub(1);
    }
}
