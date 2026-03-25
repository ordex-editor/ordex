#![allow(clippy::question_mark)]

//! Ordex - A TUI text editor
//!
//! Binary entry point for Ordex.
//!
//! Runtime orchestration is implemented in [`app`].

mod app;
mod config;
mod cursor;
mod editor_state;
mod keybindings;
mod mode;
mod navigation;
mod render;
mod signal;
mod soft_wrap;
mod syntax;
mod text_buffer;
mod themes;
mod tui;
mod viewport;

/// Launch the application runtime.
fn main() {
    app::launch();
}
