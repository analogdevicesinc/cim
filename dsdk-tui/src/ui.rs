// Copyright (c) 2026 Analog Devices, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph, Scrollbar,
        ScrollbarOrientation, Wrap,
    },
    Frame,
};

use crate::app::{App, Focus, InitPopupState, PopupFocus};
use crate::theme::{spinner, symbols, theme, Theme};
use ratatui::prelude::Stylize;
use ratatui::style::Modifier;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let _theme = theme();

    // Main layout
    // - Source: 5 rows (with padding)
    // - Target list: exactly 12 rows (10 items + borders + padding)
    // - Output: fills remaining space
    // - Status: 1 row
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // Source input (with padding)
            Constraint::Length(12), // Target list (10 items max)
            Constraint::Min(5),     // Output pane (fills remaining)
            Constraint::Length(1),  // Status bar
        ])
        .split(area);

    // Source input section
    draw_source_input(frame, chunks[0], app);

    // Target list section
    draw_target_list(frame, chunks[1], app);

    // Output pane (needs mutable reference for height tracking)
    draw_output_pane(frame, chunks[2], app);

    // Status bar
    draw_status_bar(frame, chunks[3], app);

    // Draw popup if open (borrow popup separately to avoid borrow issues)
    let popup_ref = app.init_popup.as_ref();
    if let Some(popup) = popup_ref {
        // Draw overlay to create depth
        draw_popup_overlay(frame, area);
        draw_init_popup(frame, area, popup);
    }
}

fn draw_source_input(frame: &mut Frame, area: Rect, app: &App) {
    let theme = theme();
    let is_focused = app.focus == Focus::SourceInput;

    let border_style = if is_focused {
        theme.focus_border()
    } else {
        theme.unfocus_border()
    };

    let title = format!(" {} Source ", symbols::icons::SOURCE);

    let block = Block::default()
        .title(title)
        .title_style(theme.title())
        .borders(Borders::ALL)
        .border_type(Theme::BORDER_TYPE)
        .border_style(border_style)
        .padding(ratatui::widgets::Padding::new(1, 1, 1, 1));

    let input_text = if is_focused {
        format!("{}▌", app.source_input)
    } else {
        app.source_input.clone()
    };

    let style = if is_focused {
        theme.input(true)
    } else {
        theme.input(false)
    };

    let paragraph = Paragraph::new(input_text).block(block).style(style);
    frame.render_widget(paragraph, area);
}

fn draw_target_list(frame: &mut Frame, area: Rect, app: &App) {
    let theme = theme();
    let is_focused = app.focus == Focus::TargetList;

    let border_style = if is_focused {
        theme.focus_border()
    } else {
        theme.unfocus_border()
    };

    let target_count = app.targets.len();
    let title = if target_count > 0 {
        format!(
            " {} Available Targets [{}] ",
            symbols::icons::TARGET,
            target_count
        )
    } else {
        format!(" {} Available Targets ", symbols::icons::TARGET)
    };

    let block = Block::default()
        .title(title)
        .title_style(theme.title())
        .borders(Borders::ALL)
        .border_type(Theme::BORDER_TYPE)
        .border_style(border_style)
        .padding(ratatui::widgets::Padding::new(1, 1, 1, 1));

    // Show error message if there is one
    if let Some(ref error) = app.error_message {
        let error_text = format!(
            "{} Error:\n\n{}\n\nPress [{}] to retry.",
            symbols::icons::ERROR,
            error,
            "r"
        );
        let error_para = Paragraph::new(error_text)
            .block(block)
            .style(theme.error())
            .alignment(Alignment::Center);
        frame.render_widget(error_para, area);
        return;
    }

    if app.is_loading {
        let spinner = spinner(app.status_message.as_ref().map(|s| s.len()).unwrap_or(0));
        let loading_text = format!("{} Loading targets...", spinner);
        let loading = Paragraph::new(loading_text)
            .block(block)
            .style(theme.loading())
            .alignment(Alignment::Center);
        frame.render_widget(loading, area);
        return;
    }

    if app.targets.is_empty() {
        let empty_text = format!("No targets found.\n\nPress [{}] to refresh.", "r");
        let empty = Paragraph::new(empty_text)
            .block(block)
            .style(theme.muted())
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    // Calculate visible range based on scroll offset
    let total_targets = app.targets.len();
    let visible_count = 10.min(total_targets);
    let scroll_offset = app
        .target_scroll_offset
        .min(total_targets.saturating_sub(visible_count));

    // Create visible items with modern styling
    let items: Vec<ListItem> = app
        .targets
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_count)
        .map(|(i, target)| {
            let is_selected = i == app.selected_target;
            let style = if is_selected {
                theme.selected()
            } else {
                theme.text()
            };

            // Add scroll indicators if needed
            let display_text = if total_targets > visible_count {
                if i == scroll_offset && scroll_offset > 0 {
                    format!("{} {} {}", symbols::SCROLL_UP, symbols::SELECTION, target)
                } else if i == scroll_offset + visible_count - 1
                    && scroll_offset + visible_count < total_targets
                {
                    format!("{} {} {}", symbols::SELECTION, target, symbols::SCROLL_DOWN)
                } else if is_selected {
                    format!("{} {}", symbols::SELECTION, target)
                } else {
                    format!("  {}", target)
                }
            } else if is_selected {
                format!("{} {}", symbols::SELECTION, target)
            } else {
                format!("  {}", target)
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
    let theme = theme();

    // Split status bar into left (keys) and right (status)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(30)])
        .split(area);

    // Status message takes priority
    let left_content: Line = if let Some(ref msg) = app.status_message {
        Line::from(vec![Span::styled(msg.clone(), theme.status_bar())])
    } else {
        // Create key badges based on current focus
        let badges = match app.focus {
            Focus::Output => vec![
                Span::styled("[↑/↓]", theme.key_badge()),
                Span::styled(" scroll ", theme.muted()),
                Span::styled("[PgUp/PgDn]", theme.key_badge()),
                Span::styled(" page ", theme.muted()),
                Span::styled("[tab]", theme.key_badge()),
                Span::styled(" focus ", theme.muted()),
                Span::styled("[q]", theme.key_badge()),
                Span::styled(" quit", theme.muted()),
            ],
            _ => vec![
                Span::styled("[tab]", theme.key_badge()),
                Span::styled(" focus ", theme.muted()),
                Span::styled("[enter]", theme.key_badge()),
                Span::styled(" init ", theme.muted()),
                Span::styled("[r]", theme.key_badge()),
                Span::styled(" refresh ", theme.muted()),
                Span::styled("[q]", theme.key_badge()),
                Span::styled(" quit", theme.muted()),
            ],
        };
        Line::from(badges)
    };

    let left = Paragraph::new(left_content);
    frame.render_widget(left, chunks[0]);

    // Right side: target count or hint
    let right_text = if !app.targets.is_empty() {
        format!("{} targets ", app.targets.len())
    } else {
        String::new()
    };
    let right = Paragraph::new(right_text)
        .style(theme.muted())
        .alignment(Alignment::Right);
    frame.render_widget(right, chunks[1]);
}

fn draw_output_pane(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = theme();

    // Update the pane height and width for auto-scroll calculations
    app.output_pane_height = area.height;
    app.output_pane_width = area.width.saturating_sub(4); // 2 borders + 2 padding

    let is_focused = app.focus == Focus::Output;

    let border_style = if is_focused {
        theme.focus_border()
    } else {
        theme.unfocus_border()
    };

    let title = format!(" {} Output ", symbols::icons::OUTPUT);

    let block = Block::default()
        .title(title)
        .title_style(theme.title())
        .borders(Borders::ALL)
        .border_type(Theme::BORDER_TYPE)
        .border_style(border_style)
        .padding(ratatui::widgets::Padding::new(1, 1, 0, 0));

    let text = if app.output_text.is_empty() {
        Text::styled(
            "Output from cim commands will appear here...",
            theme.placeholder(),
        )
    } else {
        // Split into lines and apply color coding based on content (case-insensitive)
        let lines: Vec<Line> = app
            .output_text
            .lines()
            .map(|line| {
                let line_lower = line.to_lowercase();
                let style = if line_lower.contains("error") || line_lower.starts_with("error:") {
                    theme.error()
                } else if line_lower.contains("warning") || line_lower.starts_with("warning:") {
                    theme.warning()
                } else if line_lower.contains("success")
                    || line_lower.contains("completed")
                    || line_lower.contains("done")
                {
                    theme.success()
                } else if line_lower.contains("info:") || line_lower.contains("[info]") {
                    theme.info()
                } else {
                    theme.text()
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

    // Render scrollbar over the right border (inside top/bottom corners)
    let scrollbar_area = area.inner(Margin {
        horizontal: 0,
        vertical: 1,
    });
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some(symbols::SCROLL_UP))
            .end_symbol(Some(symbols::SCROLL_DOWN))
            .track_symbol(Some("│"))
            .thumb_symbol("█"),
        scrollbar_area,
        &mut app.output_scrollbar_state,
    );
}

fn draw_popup_overlay(frame: &mut Frame, area: Rect) {
    let theme = theme();
    // Create a dimmed overlay effect
    let overlay = Paragraph::new("").style(theme.popup_overlay());
    frame.render_widget(overlay, area);
}

fn draw_init_popup(frame: &mut Frame, area: Rect, popup: &InitPopupState) {
    let theme = theme();

    // Calculate popup area (centered, 70% width, 85% height)
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

    let title = format!(
        " {} Initialize Workspace - {} ",
        symbols::icons::TARGET,
        target_name
    );

    let block = Block::default()
        .title(title)
        .title_style(theme.popup_block())
        .borders(Borders::ALL)
        .border_type(Theme::BORDER_TYPE)
        .border_style(Style::default().fg(theme.palette.primary));

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
            Constraint::Length(2), // Checkboxes row 1 (3 items)
            Constraint::Length(2), // Checkboxes row 2 (3 items)
            Constraint::Length(2), // Checkboxes row 3 (2 items)
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

    // Checkboxes - row 1: No mirror | Force | Verbose
    let row1 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),   // checkbox 1
            Constraint::Length(3), // separator " | "
            Constraint::Fill(1),   // checkbox 2
            Constraint::Length(3), // separator " | "
            Constraint::Fill(1),   // checkbox 3
        ])
        .split(chunks[3]);
    draw_checkbox(
        frame,
        row1[0],
        "[N]o mirror",
        popup.no_mirror,
        popup.focus == PopupFocus::NoMirror,
    );
    draw_separator(frame, row1[1]);
    draw_checkbox(
        frame,
        row1[2],
        "[F]orce",
        popup.force,
        popup.focus == PopupFocus::Force,
    );
    draw_separator(frame, row1[3]);
    draw_checkbox(
        frame,
        row1[4],
        "Ver[b]ose",
        popup.verbose,
        popup.focus == PopupFocus::Verbose,
    );

    // Checkboxes - row 2: Install | Full | No sudo
    let row2 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),   // checkbox 1
            Constraint::Length(3), // separator " | "
            Constraint::Fill(1),   // checkbox 2
            Constraint::Length(3), // separator " | "
            Constraint::Fill(1),   // checkbox 3
        ])
        .split(chunks[4]);
    draw_checkbox(
        frame,
        row2[0],
        "[I]nstall",
        popup.install,
        popup.focus == PopupFocus::Install,
    );
    draw_separator(frame, row2[1]);
    draw_checkbox(
        frame,
        row2[2],
        "F[u]ll",
        popup.full,
        popup.focus == PopupFocus::Full,
    );
    draw_separator(frame, row2[3]);
    draw_checkbox(
        frame,
        row2[4],
        "No [s]udo",
        popup.no_sudo,
        popup.focus == PopupFocus::NoSudo,
    );

    // Checkboxes - row 3: Symlink | Yes (centered, 2 items)
    let row3 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),   // spacer
            Constraint::Fill(1),   // checkbox 1
            Constraint::Length(3), // separator " | "
            Constraint::Fill(1),   // checkbox 2
            Constraint::Fill(1),   // spacer
        ])
        .split(chunks[5]);
    draw_checkbox(
        frame,
        row3[1],
        "Sym[l]ink",
        popup.symlink,
        popup.focus == PopupFocus::Symlink,
    );
    draw_separator(frame, row3[2]);
    draw_checkbox(
        frame,
        row3[3],
        "[Y]es (skip confirm)",
        popup.yes,
        popup.focus == PopupFocus::Yes,
    );

    // Cert validation dropdown (collect area for later overlay drawing)
    let cert_dropdown_area = draw_cert_dropdown(frame, chunks[6], popup);

    // Help footer with keyboard shortcuts
    let help_text = "Shortcuts: n,f,b,i,u,s,l,y,v,a,c,esc | Press key to toggle or focus";
    let help = Paragraph::new(help_text)
        .style(theme.help_text())
        .alignment(Alignment::Center);
    let help_area = Rect::new(
        popup_area.x + 2,
        popup_area.y + popup_area.height - 2,
        popup_area.width - 4,
        1,
    );
    frame.render_widget(help, help_area);

    // Buttons
    let button_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[7]);

    // Cancel button
    let cancel_focused = popup.focus == PopupFocus::CancelButton;
    let cancel_style = theme.cancel_button(cancel_focused);
    let cancel_text = if cancel_focused {
        format!(
            " {} Cancel (ESC) {} ",
            symbols::ARROW_RIGHT,
            symbols::ARROW_RIGHT
        )
    } else {
        " Cancel (ESC) ".to_string()
    };
    let cancel_btn = Paragraph::new(cancel_text)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::BORDER_TYPE)
                .border_style(cancel_style),
        )
        .style(cancel_style);
    frame.render_widget(cancel_btn, button_row[0]);

    // Create button
    let create_focused = popup.focus == PopupFocus::CreateButton;
    let create_style = theme.primary_button(create_focused);
    let create_text = if create_focused {
        format!(
            " {} Create (c) {} ",
            symbols::ARROW_RIGHT,
            symbols::ARROW_RIGHT
        )
    } else {
        " Create (c) ".to_string()
    };
    let create_btn = Paragraph::new(create_text)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::BORDER_TYPE)
                .border_style(create_style),
        )
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
    let theme = theme();
    let is_focused = popup.focus == PopupFocus::VersionDropdown;
    let style = theme.dropdown(is_focused);

    let version_text = if popup.is_loading_versions {
        format!("{} Loading...", spinner(0))
    } else if popup.versions.is_empty() {
        "Latest (no versions available)".to_string()
    } else if popup.selected_version == 0 {
        "Latest".to_string()
    } else {
        popup.versions[popup.selected_version - 1].clone()
    };

    let display_text = if is_focused {
        format!("{} {}", version_text, symbols::DROPDOWN_CLOSED)
    } else {
        version_text
    };

    let title = "[V]ersion".to_string();
    let block = Block::default()
        .title(title)
        .title_style(if is_focused {
            theme.title()
        } else {
            theme.muted()
        })
        .borders(Borders::ALL)
        .border_type(Theme::BORDER_TYPE)
        .border_style(style);

    let paragraph = Paragraph::new(display_text).block(block).style(style);
    frame.render_widget(paragraph, area);

    // Return the overlay area if dropdown should be open
    if popup.version_dropdown_open && is_focused && !popup.versions.is_empty() {
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
    let theme = theme();
    frame.render_widget(Clear, area);

    let items: Vec<ListItem> = std::iter::once("Latest")
        .chain(popup.versions.iter().map(|v| v.as_str()))
        .enumerate()
        .map(|(i, v)| {
            let is_selected = i == popup.selected_version;
            let style = theme.dropdown_item(is_selected);
            ListItem::new(v).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(Theme::BORDER_TYPE)
            .bg(theme.palette.surface),
    );
    frame.render_widget(list, area);
}

// Returns the area where the dropdown overlay should be drawn (if open)
fn draw_cert_dropdown(frame: &mut Frame, area: Rect, popup: &InitPopupState) -> Option<Rect> {
    let theme = theme();
    let is_focused = popup.focus == PopupFocus::CertValidation;
    let style = theme.dropdown(is_focused);

    let cert_values = ["strict", "relaxed", "auto"];
    let cert_text = cert_values[popup.selected_cert.min(2)];

    let display_text = if is_focused {
        format!("{} {}", cert_text, symbols::DROPDOWN_CLOSED)
    } else {
        cert_text.to_string()
    };

    let title = "Cert V[a]lidation".to_string();
    let block = Block::default()
        .title(title)
        .title_style(if is_focused {
            theme.title()
        } else {
            theme.muted()
        })
        .borders(Borders::ALL)
        .border_type(Theme::BORDER_TYPE)
        .border_style(style);

    let paragraph = Paragraph::new(display_text).block(block).style(style);
    frame.render_widget(paragraph, area);

    // Return the overlay area if dropdown should be open
    if popup.cert_dropdown_open && is_focused {
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
    let theme = theme();
    frame.render_widget(Clear, area);

    let cert_values = ["strict", "relaxed", "auto"];

    let items: Vec<ListItem> = cert_values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let is_selected = i == popup.selected_cert;
            let style = theme.dropdown_item(is_selected);
            ListItem::new(v).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(Theme::BORDER_TYPE)
            .bg(theme.palette.surface),
    );
    frame.render_widget(list, area);
}

fn draw_popup_input(frame: &mut Frame, area: Rect, title: &str, value: &str, is_focused: bool) {
    let theme = theme();
    let style = theme.input(is_focused);

    let title_style = if is_focused {
        theme.title()
    } else {
        theme.muted()
    };

    let block = Block::default()
        .title(title)
        .title_style(title_style)
        .borders(Borders::ALL)
        .border_type(Theme::BORDER_TYPE)
        .border_style(style);

    let text = if is_focused {
        format!("{}▌", value)
    } else {
        value.to_string()
    };

    let paragraph = Paragraph::new(text).block(block).style(style);
    frame.render_widget(paragraph, area);
}

fn draw_checkbox(frame: &mut Frame, area: Rect, label: &str, checked: bool, is_focused: bool) {
    let theme = theme();
    let checkbox = if checked {
        format!("[{}]", symbols::CHECKBOX_CHECKED)
    } else {
        format!("[{}]", symbols::CHECKBOX_UNCHECKED)
    };

    let checkbox_style = theme.checkbox(checked, is_focused);
    let label_style = if is_focused {
        theme.text().add_modifier(Modifier::BOLD)
    } else if checked {
        theme.text()
    } else {
        theme.muted()
    };

    let spans = vec![
        Span::styled(checkbox, checkbox_style),
        Span::styled(" ", Style::default()),
        Span::styled(label.to_string(), label_style),
    ];

    let paragraph = Paragraph::new(Line::from(spans));
    frame.render_widget(paragraph, area);
}

fn draw_separator(frame: &mut Frame, area: Rect) {
    let theme = theme();
    let separator = Paragraph::new(symbols::BULLET)
        .style(theme.muted())
        .alignment(Alignment::Center);
    frame.render_widget(separator, area);
}
