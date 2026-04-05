#![allow(clippy::question_mark)]

//! Ordex - A TUI text editor
//!
//! Binary entry point for Ordex.
//!
//! Runtime orchestration is implemented in [`app`].

mod app;
mod completion;
mod config;
mod cursor;
mod dialogs;
mod editor_state;
mod keybindings;
mod mode;
mod navigation;
mod render;
mod session;
mod signal;
mod soft_wrap;
mod swap;
mod syntax;
mod text_buffer;
mod themes;
mod toml_like_parser;
mod tui;
mod viewport;

/// Launch the application runtime.
fn main() {
    app::launch();
}
