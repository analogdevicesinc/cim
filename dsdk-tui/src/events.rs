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

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

use crate::app::{App, Focus, PopupFocus};

/// Handle events and return true if should quit
pub fn handle_event(app: &mut App) -> anyhow::Result<bool> {
    if event::poll(Duration::from_millis(50))? {
        if let Event::Key(key) = event::read()? {
            return handle_key_event(app, key);
        }
    }
    Ok(false)
}

fn handle_key_event(app: &mut App, key: KeyEvent) -> anyhow::Result<bool> {
    // Handle popup events first if popup is open
    if app.init_popup.is_some() {
        return handle_popup_event(app, key);
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(true),
        KeyCode::Char('r') | KeyCode::Char('R') => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                app.refresh_targets();
            } else {
                app.refresh_targets();
            }
        }
        KeyCode::Tab => {
            app.focus = match app.focus {
                Focus::SourceInput => Focus::TargetList,
                Focus::TargetList => Focus::Output,
                Focus::Output => Focus::SourceInput,
            };
        }
        KeyCode::Enter => {
            if app.focus == Focus::TargetList && !app.targets.is_empty() {
                app.open_init_popup();
            }
        }
        KeyCode::Up => {
            match app.focus {
                Focus::TargetList => app.select_previous(),
                Focus::Output => app.scroll_output_up(1),
                _ => {}
            }
        }
        KeyCode::Down => {
            match app.focus {
                Focus::TargetList => app.select_next(),
                Focus::Output => app.scroll_output_down(1),
                _ => {}
            }
        }
        KeyCode::PageUp => {
            if app.focus == Focus::Output {
                app.scroll_output_up(10);
            }
        }
        KeyCode::PageDown => {
            if app.focus == Focus::Output {
                app.scroll_output_down(10);
            }
        }
        KeyCode::Home => {
            if app.focus == Focus::Output {
                app.scroll_output_top();
            }
        }
        KeyCode::End => {
            if app.focus == Focus::Output {
                app.scroll_output_bottom();
            }
        }
        KeyCode::Char(c) => {
            if app.focus == Focus::SourceInput {
                app.source_input.push(c);
                app.targets.clear();
                app.selected_target = 0;
            }
        }
        KeyCode::Backspace => {
            if app.focus == Focus::SourceInput {
                app.source_input.pop();
                app.targets.clear();
                app.selected_target = 0;
            }
        }
        KeyCode::Delete => {
            if app.focus == Focus::SourceInput {
                // Delete at cursor position would need cursor tracking
            }
        }
        KeyCode::Left => {
            // Move cursor left
        }
        KeyCode::Right => {
            // Move cursor right
        }
        _ => {}
    }

    Ok(false)
}

fn handle_popup_event(app: &mut App, key: KeyEvent) -> anyhow::Result<bool> {
    let popup = app.init_popup.as_mut().unwrap();

    match key.code {
        KeyCode::Esc => {
            app.close_init_popup();
            return Ok(false);
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            // 'c' or 'C' to Create
            app.execute_init();
        }
        KeyCode::Tab => {
            popup.next_focus();
        }
        KeyCode::BackTab => {
            popup.previous_focus();
        }
        KeyCode::Enter => {
            if popup.focus == PopupFocus::CancelButton {
                app.close_init_popup();
            } else if popup.focus == PopupFocus::CreateButton {
                app.execute_init();
            } else if popup.focus == PopupFocus::VersionDropdown && !popup.versions.is_empty() {
                popup.version_dropdown_open = !popup.version_dropdown_open;
            } else if popup.focus == PopupFocus::CertValidation {
                popup.cert_dropdown_open = !popup.cert_dropdown_open;
            }
        }
        KeyCode::Up => {
            match popup.focus {
                PopupFocus::VersionDropdown if popup.version_dropdown_open => {
                    popup.selected_version = popup.selected_version.saturating_sub(1);
                }
                PopupFocus::CertValidation if popup.cert_dropdown_open => {
                    popup.selected_cert = popup.selected_cert.saturating_sub(1);
                }
                _ => {}
            }
        }
        KeyCode::Down => {
            match popup.focus {
                PopupFocus::VersionDropdown if popup.version_dropdown_open => {
                    if popup.selected_version + 1 < popup.versions.len() + 1 {
                        popup.selected_version += 1;
                    }
                }
                PopupFocus::CertValidation if popup.cert_dropdown_open => {
                    if popup.selected_cert + 1 < 3 {
                        popup.selected_cert += 1;
                    }
                }
                _ => {}
            }
        }
        KeyCode::Char(' ') => {
            match popup.focus {
                PopupFocus::NoMirror => popup.no_mirror = !popup.no_mirror,
                PopupFocus::Force => popup.force = !popup.force,
                PopupFocus::Verbose => popup.verbose = !popup.verbose,
                PopupFocus::Install => popup.install = !popup.install,
                PopupFocus::Full => popup.full = !popup.full,
                PopupFocus::NoSudo => popup.no_sudo = !popup.no_sudo,
                PopupFocus::Symlink => popup.symlink = !popup.symlink,
                PopupFocus::Yes => popup.yes = !popup.yes,
                _ => {}
            }
        }
        KeyCode::Char(c) => {
            match popup.focus {
                PopupFocus::WorkspaceInput => popup.workspace_input.push(c),
                PopupFocus::MatchInput => popup.match_input.push(c),
                _ => {}
            }
        }
        KeyCode::Backspace => {
            match popup.focus {
                PopupFocus::WorkspaceInput => { popup.workspace_input.pop(); }
                PopupFocus::MatchInput => { popup.match_input.pop(); }
                _ => {}
            }
        }
        _ => {}
    }

    Ok(false)
}
