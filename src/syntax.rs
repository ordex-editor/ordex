//! Syntax-highlighting subsystem entry points.
//!
//! This module wires together the shared profile metadata, helper predicates,
//! incremental highlighting engine, and built-in language profiles.

pub(crate) mod engine;
pub(crate) mod helpers;
pub(crate) mod profile;
pub(crate) mod profiles;

#[cfg(test)]
mod profile_tests;

pub(crate) use engine::{BufferEdit, HighlightSpan, SyntaxEngine};
pub(crate) use profile::{SyntaxClass, SyntaxModifier};
