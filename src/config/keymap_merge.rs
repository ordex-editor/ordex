//! Keymap conflict resolution for loaded configuration bindings.

use crate::config::validator::{ConfiguredBinding, ConfiguredSequenceBinding};
use crate::config::warnings::{WarningCode, WarningEvent};
use std::collections::HashMap;
use std::path::Path;

/// Deduplicate key bindings using last-definition-wins semantics.
pub(crate) fn dedupe_bindings(
    bindings: &[ConfiguredBinding],
    source_path: &Path,
) -> (Vec<ConfiguredBinding>, Vec<WarningEvent>) {
    let mut latest: HashMap<
        (
            crate::keybindings::ModeContext,
            crate::keybindings::KeyInput,
        ),
        usize,
    > = HashMap::new();
    let mut deduped: Vec<ConfiguredBinding> = Vec::new();
    let mut warnings = Vec::new();

    // Track the latest assignment for each (mode, key) pair while preserving
    // stable iteration order in the resulting vector.
    for binding in bindings {
        let key = (binding.mode, binding.key.clone());
        if let Some(existing_idx) = latest.get(&key).copied() {
            let previous = deduped[existing_idx].source.clone();
            deduped[existing_idx] = binding.clone();
            warnings.push(WarningEvent::new(
                WarningCode::DuplicateKeymap,
                format!(
                    "Duplicate key mapping replaced by last definition (previous: {})",
                    previous
                ),
                source_path,
                Some("keymap".to_string()),
                None,
            ));
            continue;
        }

        latest.insert(key, deduped.len());
        deduped.push(binding.clone());
    }

    (deduped, warnings)
}

/// Deduplicate sequence bindings using last-definition-wins semantics.
pub(crate) fn dedupe_sequence_bindings(
    bindings: &[ConfiguredSequenceBinding],
    source_path: &Path,
) -> (Vec<ConfiguredSequenceBinding>, Vec<WarningEvent>) {
    let mut latest: HashMap<
        (
            crate::keybindings::ModeContext,
            Vec<crate::keybindings::KeyInput>,
        ),
        usize,
    > = HashMap::new();
    let mut deduped: Vec<ConfiguredSequenceBinding> = Vec::new();
    let mut warnings = Vec::new();

    for binding in bindings {
        let key = (binding.mode, binding.keys.clone());
        if let Some(existing_idx) = latest.get(&key).copied() {
            let previous = deduped[existing_idx].source.clone();
            deduped[existing_idx] = binding.clone();
            warnings.push(WarningEvent::new(
                WarningCode::DuplicateKeymap,
                format!(
                    "Duplicate key mapping replaced by last definition (previous: {})",
                    previous
                ),
                source_path,
                Some("keymap".to_string()),
                None,
            ));
            continue;
        }

        latest.insert(key, deduped.len());
        deduped.push(binding.clone());
    }

    (deduped, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybindings::{Action, KeyInput, ModeContext};

    #[test]
    fn duplicate_binding_last_definition_wins() {
        let bindings = vec![
            ConfiguredBinding {
                mode: ModeContext::Normal,
                key: KeyInput::Char('z'),
                action: Action::MoveLeft,
                source: "a".to_string(),
            },
            ConfiguredBinding {
                mode: ModeContext::Normal,
                key: KeyInput::Char('z'),
                action: Action::MoveRight,
                source: "b".to_string(),
            },
        ];
        let (deduped, warnings) = dedupe_bindings(&bindings, Path::new("config"));
        assert_eq!(deduped.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert_eq!(deduped[0].action, Action::MoveRight);
    }

    #[test]
    fn duplicate_sequence_binding_last_definition_wins() {
        let bindings = vec![
            ConfiguredSequenceBinding {
                mode: ModeContext::Normal,
                keys: vec![KeyInput::Char('z'), KeyInput::Char('u')],
                action: Action::MoveLeft,
                source: "a".to_string(),
            },
            ConfiguredSequenceBinding {
                mode: ModeContext::Normal,
                keys: vec![KeyInput::Char('z'), KeyInput::Char('u')],
                action: Action::MoveDown,
                source: "b".to_string(),
            },
        ];
        let (deduped, warnings) = dedupe_sequence_bindings(&bindings, Path::new("config"));
        assert_eq!(deduped.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert_eq!(deduped[0].action, Action::MoveDown);
    }
}
