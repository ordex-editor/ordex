#![allow(clippy::question_mark)]

//! Ordex - A TUI text editor
//!
//! Binary entry point for Ordex.
//!
//! Runtime orchestration is implemented in [`app`].

mod app;
mod cache_dirs;
mod completion;
mod config;
mod cursor;
mod dialogs;
mod editor_state;
mod keybindings;
mod lsp;
mod mode;
mod navigation;
mod path_utils;
mod render;
mod search;
mod session;
mod signal;
mod soft_wrap;
mod spinner;
mod swap;
mod syntax;
mod temp_paths;
mod text_buffer;
mod themes;
mod toml_like_parser;
mod tui;
mod unsafe_io;
mod viewport;

/// Launch the application runtime.
fn main() {
    app::launch();
}
