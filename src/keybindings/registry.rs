//! Runtime keybinding storage, lookup, and sequence matching.

use super::{Action, ActionBinding, KeyInput, ModeContext, SequenceContinuation, SequenceMatch};
use crate::mode::Mode;
use std::collections::HashMap;
use termion::event::Key;

/// Internal storage for a configured multi-key binding and its action payload.
#[derive(Debug, Clone)]
struct SequenceBinding {
    mode: ModeContext,
    keys: Vec<KeyInput>,
    actions: ActionBinding,
}

/// Key bindings configuration.
pub(crate) struct KeyBindings {
    /// Bindings for each mode: `(ModeContext, KeyInput) -> actions`.
    bindings: HashMap<(ModeContext, KeyInput), ActionBinding>,
    /// Sequence bindings for each mode, such as `gg`.
    sequence_bindings: Vec<SequenceBinding>,
}

impl KeyBindings {
    /// Create an empty runtime registry before defaults or config are applied.
    pub(super) fn empty() -> Self {
        Self {
            bindings: HashMap::new(),
            sequence_bindings: Vec::new(),
        }
    }

    /// Insert one single-action binding into the runtime registry.
    pub(super) fn insert_action(&mut self, mode: ModeContext, key: KeyInput, action: Action) {
        self.bindings
            .insert((mode, key), ActionBinding::single(action));
    }

    /// Insert one single-action sequence binding into the runtime registry.
    pub(super) fn insert_sequence_action(
        &mut self,
        mode: ModeContext,
        keys: Vec<KeyInput>,
        action: Action,
    ) {
        self.sequence_bindings.push(SequenceBinding {
            mode,
            keys,
            actions: ActionBinding::single(action),
        });
    }

    /// Get the single action bound to one key press in the given mode.
    ///
    /// Returns `None` when the binding executes multiple actions or when the
    /// key is unbound.
    pub(crate) fn get_action(&self, key: Key, mode: &Mode) -> Option<Action> {
        match self.get_binding(key, mode) {
            Some(ActionBinding::Single(action)) => Some(*action),
            _ => None,
        }
    }

    /// Get the configured action binding for a key press in the given mode.
    pub(crate) fn get_binding(&self, key: Key, mode: &Mode) -> Option<&ActionBinding> {
        let context = ModeContext::from(mode);
        let key_input = KeyInput::from(key);
        self.bindings.get(&(context, key_input))
    }

    /// Check if a key can begin a known multi-key sequence in the given mode.
    pub(crate) fn starts_sequence_prefix(&self, mode: &Mode, key: &KeyInput) -> bool {
        let context = ModeContext::from(mode);
        self.sequence_bindings.iter().any(|binding| {
            binding.mode == context && binding.keys.len() > 1 && binding.keys.first() == Some(key)
        })
    }

    /// Match a sequence of keys against configured multi-key bindings.
    pub(crate) fn match_sequence(&self, mode: &Mode, keys: &[KeyInput]) -> SequenceMatch {
        let context = ModeContext::from(mode);
        let mut has_prefix = false;

        for binding in self
            .sequence_bindings
            .iter()
            .filter(|binding| binding.mode == context)
        {
            if binding.keys == keys {
                return SequenceMatch::Exact(binding.actions.clone());
            }

            // Track whether the typed keys still match a longer configured sequence.
            if binding.keys.starts_with(keys) {
                has_prefix = true;
            }
        }

        if has_prefix {
            SequenceMatch::Prefix
        } else {
            SequenceMatch::NoMatch
        }
    }

    /// Return every configured continuation that remains valid for `keys`.
    pub(crate) fn continuations_for_prefix(
        &self,
        mode: &Mode,
        keys: &[KeyInput],
    ) -> Vec<SequenceContinuation> {
        let context = ModeContext::from(mode);

        // Discovery only lists bindings that need at least one more key.
        self.sequence_bindings
            .iter()
            .filter(|binding| {
                binding.mode == context
                    && binding.keys.len() > keys.len()
                    && binding.keys.starts_with(keys)
            })
            .map(|binding| SequenceContinuation {
                remaining_keys: binding.keys[keys.len()..].to_vec(),
                actions: binding.actions.clone(),
            })
            .collect()
    }

    /// Return every single-key binding in `mode` that resolves directly to `action`.
    pub(crate) fn keys_for_action(&self, mode: &Mode, action: Action) -> Vec<KeyInput> {
        let context = ModeContext::from(mode);
        let mut keys = self
            .bindings
            .iter()
            .filter_map(|((binding_mode, key), binding)| match binding {
                ActionBinding::Single(bound_action)
                    if *binding_mode == context && *bound_action == action =>
                {
                    Some(key.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        // Discovery popups should stay deterministic even though the registry
        // stores bindings in a hash map with non-stable iteration order.
        keys.sort_by_key(KeyInput::label);
        keys
    }

    /// Check if a key is a character that should be inserted or appended.
    ///
    /// This handles the case where typed characters aren't in the bindings map.
    pub(crate) fn is_insertable_char(key: Key) -> Option<char> {
        if let Key::Char(c) = key {
            // Newline is handled by dedicated insert-mode bindings.
            if c != '\n' {
                return Some(c);
            }
        }

        None
    }

    /// Override or add a key binding with one or more actions at runtime.
    #[cfg(test)]
    pub(crate) fn set_binding_actions(
        &mut self,
        mode: ModeContext,
        key: KeyInput,
        actions: Vec<Action>,
    ) {
        let binding =
            ActionBinding::from_actions(actions).expect("binding actions must not be empty");
        self.set_binding_action_binding(mode, key, binding);
    }

    /// Override or add a key binding using a pre-built action binding.
    pub(crate) fn set_binding_action_binding(
        &mut self,
        mode: ModeContext,
        key: KeyInput,
        binding: ActionBinding,
    ) {
        self.bindings.insert((mode, key), binding);
    }

    /// Override or add a multi-key sequence binding with one or more actions.
    #[cfg(test)]
    pub(crate) fn set_sequence_binding_actions(
        &mut self,
        mode: ModeContext,
        keys: Vec<KeyInput>,
        actions: Vec<Action>,
    ) {
        let binding =
            ActionBinding::from_actions(actions).expect("sequence actions must not be empty");
        self.set_sequence_binding_action_binding(mode, keys, binding);
    }

    /// Override or add a multi-key sequence binding using a pre-built action binding.
    pub(crate) fn set_sequence_binding_action_binding(
        &mut self,
        mode: ModeContext,
        mut keys: Vec<KeyInput>,
        binding: ActionBinding,
    ) {
        if keys.len() == 1 {
            let key = keys.pop().expect("single-key path checked length");
            self.bindings.insert((mode, key), binding);
            return;
        }

        // Replace any existing sequence with the same mode and key path.
        self.sequence_bindings
            .retain(|binding| !(binding.mode == mode && binding.keys == keys));
        self.sequence_bindings.push(SequenceBinding {
            mode,
            keys,
            actions: binding,
        });
    }
}
