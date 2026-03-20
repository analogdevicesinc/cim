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

//! Theme system for the TUI application.
//!
//! Provides a centralized color palette and styling system inspired by
//! modern terminal themes (Tokyo Night variant).

use ratatui::{
    style::{Color, Modifier, Style},
    widgets::BorderType,
};

/// Tokyo Night inspired color palette
pub struct Palette {
    /// Background color - dark blue-gray
    pub bg: Color,
    /// Surface color - slightly lighter than background
    pub surface: Color,
    /// Foreground color - soft white
    pub fg: Color,
    /// Muted/secondary text color
    pub muted: Color,
    /// Primary accent color - blue
    pub primary: Color,
    // Secondary accent color - reserved for future use
    // pub secondary: Color,
    /// Success color - green
    pub success: Color,
    /// Warning color - yellow/orange
    pub warning: Color,
    /// Error color - red
    pub error: Color,
    /// Info color - cyan
    pub info: Color,
    /// Highlight background color
    pub highlight_bg: Color,
    /// Border color for focused elements
    pub focus_border: Color,
    /// Border color for unfocused elements
    pub unfocus_border: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            bg: Color::Rgb(26, 27, 38),              // #1a1b26
            surface: Color::Rgb(36, 40, 59),         // #24283b
            fg: Color::Rgb(169, 177, 214),           // #a9b1d6
            muted: Color::Rgb(86, 95, 137),          // #565f89
            primary: Color::Rgb(122, 162, 247),      // #7aa2f7
            success: Color::Rgb(158, 206, 106),      // #9ece6a
            warning: Color::Rgb(224, 175, 104),      // #e0af68
            error: Color::Rgb(247, 118, 142),        // #f7768e
            info: Color::Rgb(42, 195, 222),          // #2ac3de
            highlight_bg: Color::Rgb(52, 59, 88),    // #343b58
            focus_border: Color::Rgb(122, 162, 247), // #7aa2f7
            unfocus_border: Color::Rgb(56, 62, 90),  // #383e5a
        }
    }
}

/// Theme provides styling functions for consistent UI appearance
#[derive(Default)]
pub struct Theme {
    pub palette: Palette,
}

impl Theme {
    /// Create a new theme with default palette
    pub fn new() -> Self {
        Self::default()
    }

    // Border types
    pub const BORDER_TYPE: BorderType = BorderType::Rounded;

    // Base styles

    /// Default text style
    pub fn text(&self) -> Style {
        Style::default().fg(self.palette.fg)
    }

    /// Muted/secondary text style
    pub fn muted(&self) -> Style {
        Style::default().fg(self.palette.muted)
    }

    /// Success style
    pub fn success(&self) -> Style {
        Style::default().fg(self.palette.success)
    }

    /// Warning style
    pub fn warning(&self) -> Style {
        Style::default().fg(self.palette.warning)
    }

    /// Error style
    pub fn error(&self) -> Style {
        Style::default().fg(self.palette.error)
    }

    /// Info style
    pub fn info(&self) -> Style {
        Style::default().fg(self.palette.info)
    }

    // Component styles

    /// Block title style - bold primary color
    pub fn title(&self) -> Style {
        Style::default()
            .fg(self.palette.primary)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for focused block borders
    pub fn focus_border(&self) -> Style {
        Style::default()
            .fg(self.palette.focus_border)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for unfocused block borders
    pub fn unfocus_border(&self) -> Style {
        Style::default().fg(self.palette.unfocus_border)
    }

    /// Selected item style (for lists)
    pub fn selected(&self) -> Style {
        Style::default()
            .bg(self.palette.highlight_bg)
            .fg(self.palette.primary)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for the status bar
    pub fn status_bar(&self) -> Style {
        Style::default().fg(self.palette.muted)
    }

    /// Style for key badges in status bar
    pub fn key_badge(&self) -> Style {
        Style::default()
            .fg(self.palette.primary)
            .add_modifier(Modifier::BOLD)
    }

    /// Primary action button style (green/success)
    pub fn primary_button(&self, focused: bool) -> Style {
        if focused {
            Style::default()
                .bg(self.palette.success)
                .fg(self.palette.bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.palette.success)
        }
    }

    /// Cancel button style (red/error)
    pub fn cancel_button(&self, focused: bool) -> Style {
        if focused {
            Style::default()
                .bg(self.palette.error)
                .fg(self.palette.bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.palette.muted)
        }
    }

    /// Checkbox style
    pub fn checkbox(&self, checked: bool, focused: bool) -> Style {
        if focused {
            Style::default()
                .fg(self.palette.primary)
                .add_modifier(Modifier::BOLD)
        } else if checked {
            Style::default().fg(self.palette.success)
        } else {
            Style::default().fg(self.palette.muted)
        }
    }

    /// Input field style
    pub fn input(&self, focused: bool) -> Style {
        if focused {
            Style::default()
                .fg(self.palette.fg)
                .bg(self.palette.surface)
        } else {
            Style::default().fg(self.palette.muted)
        }
    }

    /// Placeholder text style
    pub fn placeholder(&self) -> Style {
        Style::default()
            .fg(self.palette.muted)
            .add_modifier(Modifier::ITALIC)
    }

    /// Dropdown style
    pub fn dropdown(&self, focused: bool) -> Style {
        if focused {
            Style::default()
                .fg(self.palette.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.palette.fg)
        }
    }

    /// Dropdown item style
    pub fn dropdown_item(&self, selected: bool) -> Style {
        if selected {
            Style::default()
                .bg(self.palette.primary)
                .fg(self.palette.bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.palette.fg)
        }
    }

    /// Loading/spinner style
    pub fn loading(&self) -> Style {
        Style::default()
            .fg(self.palette.primary)
            .add_modifier(Modifier::BOLD)
    }

    /// Popup overlay background style
    pub fn popup_overlay(&self) -> Style {
        Style::default().bg(self.palette.bg)
    }

    /// Popup block style
    pub fn popup_block(&self) -> Style {
        Style::default()
            .fg(self.palette.primary)
            .add_modifier(Modifier::BOLD)
    }

    /// Help text style
    pub fn help_text(&self) -> Style {
        Style::default().fg(self.palette.muted)
    }
}

/// Global theme instance - can be accessed from anywhere
use std::sync::OnceLock;

static THEME: OnceLock<Theme> = OnceLock::new();

/// Initialize the global theme
pub fn init_theme() -> &'static Theme {
    THEME.get_or_init(Theme::new)
}

/// Get the global theme instance
pub fn theme() -> &'static Theme {
    THEME.get().expect("Theme not initialized")
}

/// Symbols used throughout the UI
pub mod symbols {
    /// Checkbox checked
    pub const CHECKBOX_CHECKED: &str = "✓";
    /// Checkbox unchecked
    pub const CHECKBOX_UNCHECKED: &str = " ";
    /// Selection indicator
    pub const SELECTION: &str = "▸";
    // Dropdown open indicator (reserved for future use)
    // pub const DROPDOWN_OPEN: &str = "▴";
    /// Dropdown closed indicator
    pub const DROPDOWN_CLOSED: &str = "▾";
    /// Scroll up indicator
    pub const SCROLL_UP: &str = "▲";
    /// Scroll down indicator
    pub const SCROLL_DOWN: &str = "▼";
    /// Bullet point
    pub const BULLET: &str = "•";
    /// Arrow right
    pub const ARROW_RIGHT: &str = "→";
    /// Section icons
    pub mod icons {
        /// Source/link icon
        pub const SOURCE: &str = "🔗";
        /// Target/list icon
        pub const TARGET: &str = "📋";
        /// Output/log icon
        pub const OUTPUT: &str = "📜";
        // Info icon (reserved for future use)
        // pub const INFO: &str = "ℹ";
        // Warning icon (reserved for future use)
        // pub const WARNING: &str = "⚠";
        /// Error icon
        pub const ERROR: &str = "✖";
        // Success icon (reserved for future use)
        // pub const SUCCESS: &str = "✓";
        /// Loading/spinner frame 1
        pub const SPINNER_1: &str = "⠋";
        /// Loading/spinner frame 2
        pub const SPINNER_2: &str = "⠙";
        /// Loading/spinner frame 3
        pub const SPINNER_3: &str = "⠹";
        /// Loading/spinner frame 4
        pub const SPINNER_4: &str = "⠸";
        /// Loading/spinner frame 5
        pub const SPINNER_5: &str = "⠼";
        /// Loading/spinner frame 6
        pub const SPINNER_6: &str = "⠴";
        /// Loading/spinner frame 7
        pub const SPINNER_7: &str = "⠦";
        /// Loading/spinner frame 8
        pub const SPINNER_8: &str = "⠧";
    }
}

/// Get a spinner character based on frame number
pub fn spinner(frame: usize) -> &'static str {
    use symbols::icons::*;
    match frame % 8 {
        0 => SPINNER_1,
        1 => SPINNER_2,
        2 => SPINNER_3,
        3 => SPINNER_4,
        4 => SPINNER_5,
        5 => SPINNER_6,
        6 => SPINNER_7,
        _ => SPINNER_8,
    }
}
