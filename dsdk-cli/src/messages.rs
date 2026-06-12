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

// Message utility functions for consistent output formatting
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// Global verbose flag
static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Cached check: is stdout an interactive terminal?
///
/// When false (output piped, redirected to a file, running in CI), we skip
/// the spinner machinery entirely and fall back to plain println!/eprintln!
/// so consumers like `tee` see every line. When true, draws and routed
/// log lines go through MultiProgress so spinners don't get garbled.
fn stdout_is_tty() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| std::io::stdout().is_terminal())
}

/// Lazily-initialized MultiProgress drawn to stdout. Only created when
/// stdout is a real terminal — otherwise we use plain println!/eprintln!
/// directly so piped/redirected output is not silently swallowed.
static MULTI: OnceLock<MultiProgress> = OnceLock::new();

fn multi() -> &'static MultiProgress {
    MULTI.get_or_init(|| MultiProgress::with_draw_target(ProgressDrawTarget::stdout()))
}

/// Standard success icon
pub const SUCCESS: &str = "✓";
/// Standard error icon
pub const ERROR: &str = "✗";
/// Standard warning/info icon
pub const INFO: &str = "!";

/// Set verbose mode
pub fn set_verbose(enabled: bool) {
    VERBOSE.store(enabled, Ordering::SeqCst);
}

/// Check if verbose mode is enabled
pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::SeqCst)
}

/// Print a stdout line. When a spinner is active in a TTY, route through
/// MultiProgress so the line lands above any live spinner; otherwise use
/// plain println! so piped/redirected output is captured.
fn emit_stdout(line: &str) {
    match MULTI.get() {
        Some(m) if stdout_is_tty() => {
            let _ = m.println(line);
        }
        _ => println!("{}", line),
    }
}

/// Print a stderr line. When a spinner is active in a TTY, suspend it
/// briefly so the stderr write doesn't garble the in-place spinner draw.
fn emit_stderr(line: &str) {
    match MULTI.get() {
        Some(m) if stdout_is_tty() => m.suspend(|| eprintln!("{}", line)),
        _ => eprintln!("{}", line),
    }
}

/// Print a success message with standard formatting
pub fn success(msg: &str) {
    emit_stdout(&format!("{} {}", SUCCESS, msg));
}

/// Print an error message with standard formatting
pub fn error(msg: &str) {
    emit_stderr(&format!("{} {}", ERROR, msg));
}

/// Print an info/warning message with standard formatting
pub fn info(msg: &str) {
    emit_stdout(&format!("{} {}", INFO, msg));
}

/// Print a simple status message without icon
pub fn status(msg: &str) {
    emit_stdout(msg);
}

/// Print verbose message (only shown when verbose mode is enabled)
pub fn verbose(msg: &str) {
    if is_verbose() {
        println!("  {}", msg);
    }
}

/// Print detailed progress with repository name and action
pub fn progress(repo: &str, action: &str) {
    if is_verbose() {
        println!("  {} - {}", repo, action);
    }
}

/// Print workspace status line
pub fn workspace(path: &std::path::Path) {
    status(&format!("Workspace: {}", path.display()));
}

/// A live spinner showing elapsed time for a long-running operation
/// (currently used for git clones).
///
/// In a TTY, animates a Braille spinner with elapsed time and current phase
/// next to the repository name. When stdout is not a TTY (CI, redirected
/// output), `indicatif` auto-detects this and renders nothing — callers
/// still get start/finish log lines via `success`/`error`.
///
/// In verbose mode the spinner is a no-op: verbose already prints each git
/// phase on its own line, and an animated spinner would garble that trace.
pub struct Spinner {
    bar: Option<ProgressBar>,
}

impl Spinner {
    /// Start a spinner labeled `<name>  <action>`. Tick interval is 100ms.
    pub fn start(name: &str, action: &str) -> Self {
        if is_verbose() || !stdout_is_tty() {
            return Spinner { bar: None };
        }
        let style = ProgressStyle::with_template("{spinner:.cyan} {msg} {elapsed:.dim}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ");
        let bar = multi().add(ProgressBar::new_spinner());
        bar.set_style(style);
        bar.set_message(format!("{}  {}", name, action));
        bar.enable_steady_tick(Duration::from_millis(100));
        Spinner { bar: Some(bar) }
    }

    /// Update the trailing action text (e.g. switch from "cloning" to "checking out").
    pub fn set_action(&self, name: &str, action: &str) {
        if let Some(bar) = &self.bar {
            bar.set_message(format!("{}  {}", name, action));
        }
    }

    /// Stop and remove the spinner without printing anything. The caller is
    /// expected to follow up with `success` / `error` / `info` for the final
    /// status line.
    pub fn finish(self) {
        if let Some(bar) = self.bar {
            bar.finish_and_clear();
        }
    }
}
