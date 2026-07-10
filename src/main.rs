#![allow(clippy::question_mark)]

//! Ordex - A TUI text editor
//!
//! Binary entry point for Ordex.
//!
//! Runtime orchestration is implemented in [`app`].

mod app;
mod cache_dirs;
mod cli;
mod clipboard;
mod command_completion;
mod completion;
mod config;
mod corresponding_file;
mod cursor;
mod dialogs;
mod display_columns;
mod editor_state;
mod file_targets;
mod indent;
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
mod substitute;
mod swap;
mod syntax;
mod temp_paths;
mod text_buffer;
mod themes;
mod toml_like_parser;
mod tui;
mod unsafe_io;
mod viewport;
mod visible_whitespace;

/// Launch the application runtime.
fn main() {
    match cli::parse_env_args() {
        Ok(cli::CliCommand::Launch(args)) => app::launch(args),
        Ok(cli::CliCommand::PrintVersion) => {
            println!("{}", cli::version_string());
        }
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
    }
}
