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

pub(crate) use loader::{ConfigLoadOutcome, load_config};
pub(crate) use validator::ConfigSettings;
#[cfg(test)]
pub(crate) use validator::{ConfiguredBinding, ConfiguredSequenceBinding};
pub(crate) use warnings::emit_startup_warnings;
