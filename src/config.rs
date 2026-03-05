//! Configuration loading entry points.
//!
//! The config subsystem parses a TOML-like format with resilient, section-scoped
//! recovery behavior.

mod include_loader;
mod keymap_merge;
mod loader;
mod parser;
mod validator;
mod warnings;

pub(crate) use loader::load_config;
pub(crate) use validator::ConfigSettings;
pub(crate) use warnings::emit_startup_warnings;
