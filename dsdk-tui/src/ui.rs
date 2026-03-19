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

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Focus, InitPopupState, PopupFocus};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Main layout
    // - Source: 5 rows (with padding)
    // - Target list: exactly 12 rows (10 items + borders + padding)
    // - Output: fills remaining space
    // - Status: 1 row
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),     // Source input (with padding)
            Constraint::Length(12),    // Target list (10 items max)
            Constraint::Min(5),        // Output pane (fills remaining)
            Constraint::Length(1),     // Status bar
        ])
        .split(area);

    // Source input section
    draw_source_input(frame, chunks[0], app);

    // Target list section
    draw_target_list(frame, chunks[1], app);

    // Output pane
    draw_output_pane(frame, chunks[2], app);

    // Status bar
    draw_status_bar(frame, chunks[3], app);

    // Draw popup if open
    if let Some(popup) = &app.init_popup {
        draw_init_popup(frame, area, popup);
    }
}

fn draw_source_input(frame: &mut Frame, area: Rect, app: &App) {
    let input_style = if app.focus == Focus::SourceInput {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let block = Block::default()
        .title("Source")
        .borders(Borders::ALL)
        .border_style(input_style)
        .padding(ratatui::widgets::Padding::new(1, 1, 1, 1));

    let input_text = if app.focus == Focus::SourceInput {
        format!("{}_", app.source_input)
    } else {
        app.source_input.clone()
    };

    let paragraph = Paragraph::new(input_text).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_target_list(frame: &mut Frame, area: Rect, app: &App) {
    let list_style = if app.focus == Focus::TargetList {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let block = Block::default()
        .title("Available Targets")
        .borders(Borders::ALL)
        .border_style(list_style)
        .padding(ratatui::widgets::Padding::new(1, 1, 1, 1)); // left, right, top, bottom padding

    // Show error message if there is one
    if let Some(ref error) = app.error_message {
        let error_text = format!("Error: {}\n\nPress 'r' to retry.", error);
        let error_para = Paragraph::new(error_text)
            .block(block)
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Center);
        frame.render_widget(error_para, area);
        return;
    }

    if app.is_loading {
        let loading = Paragraph::new("Loading targets...")
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(loading, area);
        return;
    }

    if app.targets.is_empty() {
        let empty = Paragraph::new("No targets found. Press 'r' to refresh.")
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    // Calculate visible range based on scroll offset
    let total_targets = app.targets.len();
    let visible_count = 10.min(total_targets);
    let scroll_offset = app.target_scroll_offset.min(total_targets.saturating_sub(visible_count));
    
    // Create visible items with scrollbar indicators
    let items: Vec<ListItem> = app
        .targets
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_count)
        .map(|(i, target)| {
            let style = if i == app.selected_target {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            
            // Add scroll indicators if needed
            let display_text = if total_targets > visible_count {
                if i == scroll_offset && scroll_offset > 0 {
                    format!("↑ {}", target)
                } else if i == scroll_offset + visible_count - 1 && scroll_offset + visible_count < total_targets {
                    format!("{} ↓", target)
                } else {
                    target.clone()
                }
            } else {
                target.clone()
            };
            
            ListItem::new(display_text).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_spacing(HighlightSpacing::Always);

    frame.render_widget(list, area);
}

fn draw_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let help_text = if let Some(ref msg) = app.status_message {
        msg.clone()
    } else {
        match app.focus {
            Focus::Output => "↑/↓: scroll | PgUp/PgDown: page | Home/End: top/bottom | Tab: next | q: quit".to_string(),
            _ => "Tab: switch focus | Enter: initialize | r: refresh | q: quit".to_string(),
        }
    };

    let status = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(status, area);
}

fn draw_output_pane(frame: &mut Frame, area: Rect, app: &App) {
    let output_style = if app.focus == Focus::Output {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let block = Block::default()
        .title("Output")
        .borders(Borders::ALL)
        .border_style(output_style)
        .padding(ratatui::widgets::Padding::new(1, 1, 0, 0));

    let text = if app.output_text.is_empty() {
        Text::from("Output from cim commands will appear here...")
    } else {
        // Split into lines and apply color coding based on content (case-insensitive)
        let lines: Vec<Line> = app.output_text
            .lines()
            .map(|line| {
                let line_lower = line.to_lowercase();
                let style = if line_lower.contains("error") {
                    Style::default().fg(Color::Red)
                } else if line_lower.contains("warning") {
                    Style::default().fg(Color::Yellow)
                } else if line_lower.contains("success") {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };
                Line::from(line.to_string()).style(style)
            })
            .collect();
        Text::from(lines)
    };

    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: true })
        .scroll((app.output_scroll, 0));

    frame.render_widget(paragraph, area);
}

fn draw_init_popup(frame: &mut Frame, area: Rect, popup: &InitPopupState) {
    // Calculate popup area (centered, 70% width, 80% height)
    let popup_width = (area.width as f32 * 0.7) as u16;
    let popup_height = (area.height as f32 * 0.85) as u16;
    let popup_x = (area.width - popup_width) / 2;
    let popup_y = (area.height - popup_height) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear background
    frame.render_widget(Clear, popup_area);

    // Popup block
    let target_name = if popup.targets.is_empty() {
        "Unknown".to_string()
    } else {
        popup.targets[popup.selected_target].clone()
    };

    let block = Block::default()
        .title(format!(" Initialize Workspace - {} ", target_name))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    frame.render_widget(block.clone(), popup_area);

    // Inner area
    let inner = popup_area.inner(Margin::new(2, 1));

    // Layout for form fields
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Version
            Constraint::Length(3), // Workspace
            Constraint::Length(3), // Match
            Constraint::Length(2), // Checkboxes row 1
            Constraint::Length(2), // Checkboxes row 2
            Constraint::Length(2), // Checkboxes row 3
            Constraint::Length(2), // Checkboxes row 4
            Constraint::Length(3), // Cert validation
            Constraint::Length(3), // Buttons
        ])
        .split(inner);

    // Version dropdown (collect area for later overlay drawing)
    let version_dropdown_area = draw_version_dropdown(frame, chunks[0], popup);

    // Workspace input
    draw_popup_input(
        frame,
        chunks[1],
        "Workspace",
        &popup.workspace_input,
        popup.focus == PopupFocus::WorkspaceInput,
    );

    // Match input
    draw_popup_input(
        frame,
        chunks[2],
        "Match pattern",
        &popup.match_input,
        popup.focus == PopupFocus::MatchInput,
    );

    // Checkboxes - row 1
    let row1 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(25), Constraint::Percentage(25), Constraint::Percentage(25)])
        .split(chunks[3]);
    draw_checkbox(frame, row1[0], "No mirror", popup.no_mirror, popup.focus == PopupFocus::NoMirror);
    draw_checkbox(frame, row1[1], "Force", popup.force, popup.focus == PopupFocus::Force);
    draw_checkbox(frame, row1[2], "Verbose", popup.verbose, popup.focus == PopupFocus::Verbose);
    draw_checkbox(frame, row1[3], "Install", popup.install, popup.focus == PopupFocus::Install);

    // Checkboxes - row 2
    let row2 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(25), Constraint::Percentage(25), Constraint::Percentage(25)])
        .split(chunks[4]);
    draw_checkbox(frame, row2[0], "Full", popup.full, popup.focus == PopupFocus::Full);
    draw_checkbox(frame, row2[1], "No sudo", popup.no_sudo, popup.focus == PopupFocus::NoSudo);
    draw_checkbox(frame, row2[2], "Symlink", popup.symlink, popup.focus == PopupFocus::Symlink);
    draw_checkbox(frame, row2[3], "Yes (skip confirm)", popup.yes, popup.focus == PopupFocus::Yes);

    // Cert validation dropdown (collect area for later overlay drawing)
    let cert_dropdown_area = draw_cert_dropdown(frame, chunks[7], popup);

    // Buttons
    let button_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[8]);

    let cancel_style = if popup.focus == PopupFocus::CancelButton {
        Style::default().bg(Color::Red).fg(Color::White)
    } else {
        Style::default()
    };
    let cancel_btn = Paragraph::new(" Cancel (ESC) ")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL))
        .style(cancel_style);
    frame.render_widget(cancel_btn, button_row[0]);

    let create_style = if popup.focus == PopupFocus::CreateButton {
        Style::default().bg(Color::Green).fg(Color::White)
    } else {
        Style::default()
    };
    let create_btn = Paragraph::new(" Create (c) ")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL))
        .style(create_style);
    frame.render_widget(create_btn, button_row[1]);

    // Draw dropdowns LAST so they appear on top of everything else
    if let Some(area) = version_dropdown_area {
        draw_version_dropdown_overlay(frame, area, popup);
    }
    if let Some(area) = cert_dropdown_area {
        draw_cert_dropdown_overlay(frame, area, popup);
    }
}

// Returns the area where the dropdown overlay should be drawn (if open)
fn draw_version_dropdown(frame: &mut Frame, area: Rect, popup: &InitPopupState) -> Option<Rect> {
    let focus = popup.focus == PopupFocus::VersionDropdown;
    let style = if focus {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let version_text = if popup.is_loading_versions {
        "Loading...".to_string()
    } else if popup.versions.is_empty() {
        "Latest (no versions available)".to_string()
    } else if popup.selected_version == 0 {
        "Latest".to_string()
    } else {
        popup.versions[popup.selected_version - 1].clone()
    };

    let block = Block::default()
        .title("Version")
        .borders(Borders::ALL)
        .border_style(style);

    let text = if focus {
        format!("{} ▼", version_text)
    } else {
        version_text
    };

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);

    // Return the overlay area if dropdown should be open
    if popup.version_dropdown_open && focus && !popup.versions.is_empty() {
        let dropdown_height = (popup.versions.len() + 1).min(10) as u16 + 2;
        Some(Rect::new(
            area.x,
            area.y + area.height,
            area.width,
            dropdown_height,
        ))
    } else {
        None
    }
}

fn draw_version_dropdown_overlay(frame: &mut Frame, area: Rect, popup: &InitPopupState) {
    frame.render_widget(Clear, area);

    let items: Vec<ListItem> = std::iter::once("Latest")
        .chain(popup.versions.iter().map(|v| v.as_str()))
        .enumerate()
        .map(|(i, v)| {
            let style = if i == popup.selected_version {
                Style::default().bg(Color::Blue).fg(Color::White)
            } else {
                Style::default()
            };
            ListItem::new(v).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).bg(Color::Black));
    frame.render_widget(list, area);
}

// Returns the area where the dropdown overlay should be drawn (if open)
fn draw_cert_dropdown(frame: &mut Frame, area: Rect, popup: &InitPopupState) -> Option<Rect> {
    let focus = popup.focus == PopupFocus::CertValidation;
    let style = if focus {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let cert_values = ["strict", "relaxed", "auto"];
    let cert_text = cert_values[popup.selected_cert.min(2)];

    let block = Block::default()
        .title("Cert Validation")
        .borders(Borders::ALL)
        .border_style(style);

    let text = if focus {
        format!("{} ▼", cert_text)
    } else {
        cert_text.to_string()
    };

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);

    // Return the overlay area if dropdown should be open
    if popup.cert_dropdown_open && focus {
        let dropdown_height = 5;
        Some(Rect::new(
            area.x,
            area.y + area.height,
            area.width,
            dropdown_height,
        ))
    } else {
        None
    }
}

fn draw_cert_dropdown_overlay(frame: &mut Frame, area: Rect, popup: &InitPopupState) {
    frame.render_widget(Clear, area);

    let cert_values = ["strict", "relaxed", "auto"];
    
    let items: Vec<ListItem> = cert_values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let style = if i == popup.selected_cert {
                Style::default().bg(Color::Blue).fg(Color::White)
            } else {
                Style::default()
            };
            ListItem::new(v).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).bg(Color::Black));
    frame.render_widget(list, area);
}

fn draw_popup_input(frame: &mut Frame, area: Rect, title: &str, value: &str, focus: bool) {
    let style = if focus {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(style);

    let text = if focus {
        format!("{}_", value)
    } else {
        value.to_string()
    };

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_checkbox(frame: &mut Frame, area: Rect, label: &str, checked: bool, focus: bool) {
    let checkbox = if checked { "[✓]" } else { "[ ]" };
    let text = format!("{} {}", checkbox, label);

    let style = if focus {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if checked {
        Style::default().fg(Color::Green)
    } else {
        Style::default()
    };

    let paragraph = Paragraph::new(text).style(style);
    frame.render_widget(paragraph, area);
}
