//! Macro recording and playback helpers for `EditorState`.

use super::*;
use crate::keybindings::ReplayBinding;

/// Pending macro command waiting for a register key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingMacro {
    /// Start recording into the typed register.
    Record,
    /// Replay the typed register `count` times.
    Playback { count: usize },
}

impl PendingMacro {
    /// Return the pending-prefix label shown while waiting for a register key.
    pub(super) fn prefix_label(self) -> String {
        match self {
            Self::Record => "q".to_string(),
            Self::Playback { count } if count > 1 => format!("{count}@"),
            Self::Playback { .. } => "@".to_string(),
        }
    }
}

/// One in-progress macro recording.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MacroRecording {
    register: char,
    keys: Vec<Key>,
}

/// Session-local macro registers plus recording/playback guards.
#[derive(Debug, Default)]
pub(super) struct MacroState {
    registers: HashMap<char, Vec<Key>>,
    active_recording: Option<MacroRecording>,
    last_played_register: Option<char>,
    replaying: bool,
}

impl MacroState {
    /// Return whether one recording is currently active.
    ///
    /// Returns `true` when new eligible keys should be appended to the active
    /// register capture, and `false` when no recording session is open.
    pub(super) fn is_recording(&self) -> bool {
        self.active_recording.is_some()
    }

    /// Return whether macro playback is currently re-entering `handle_key`.
    ///
    /// Returns `true` while a stored macro is being replayed, and `false`
    /// during ordinary live input handling.
    pub(super) fn is_replaying(&self) -> bool {
        self.replaying
    }

    /// Return the register currently being recorded, if any.
    pub(super) fn recording_register(&self) -> Option<char> {
        self.active_recording
            .as_ref()
            .map(|recording| recording.register)
    }

    /// Start recording into `register`, replacing any earlier contents on stop.
    pub(super) fn begin_recording(&mut self, register: char) {
        self.active_recording = Some(MacroRecording {
            register,
            keys: Vec::new(),
        });
    }

    /// Stop recording and store the captured key stream.
    pub(super) fn stop_recording(&mut self) -> Option<char> {
        let recording = self.active_recording.take()?;
        self.registers.insert(recording.register, recording.keys);
        Some(recording.register)
    }

    /// Append one key to the active recording, if a recording exists.
    pub(super) fn push_recorded_key(&mut self, key: Key) {
        if let Some(recording) = self.active_recording.as_mut() {
            recording.keys.push(key);
        }
    }

    /// Return the stored key stream for `register`, if one exists.
    pub(super) fn register_keys(&self, register: char) -> Option<&[Key]> {
        self.registers.get(&register).map(Vec::as_slice)
    }

    /// Return the register used by the most recent successful playback.
    pub(super) fn last_played_register(&self) -> Option<char> {
        self.last_played_register
    }

    /// Store the register used by the most recent successful playback.
    pub(super) fn set_last_played_register(&mut self, register: char) {
        self.last_played_register = Some(register);
    }

    /// Mark whether macro playback is currently active.
    pub(super) fn set_replaying(&mut self, replaying: bool) {
        self.replaying = replaying;
    }
}

impl EditorState {
    /// Begin or stop recording, matching the current recording state.
    pub(super) fn begin_macro_recording_action(&mut self) {
        if self.macro_state.is_recording() {
            self.stop_macro_recording();
            return;
        }
        self.pending_macro = Some(PendingMacro::Record);
    }

    /// Start waiting for one macro register to replay.
    pub(super) fn begin_macro_playback_action(&mut self, count: usize) {
        self.pending_macro = Some(PendingMacro::Playback { count });
    }

    /// Consume one key while a macro command is waiting for a register.
    ///
    /// Returns `true` when the pending macro command consumed the key, and
    /// `false` when no macro command is currently waiting for a register.
    pub(super) fn handle_pending_macro_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_macro.take() else {
            return false;
        };
        if matches!(key, Key::Esc) {
            return true;
        }

        // Recording and playback share the same one-key register prompt, so
        // this branch only decides which concrete macro operation to run.
        match pending {
            PendingMacro::Record => {
                let Some(register) = Self::macro_register_from_key(key) else {
                    self.show_error_message("Macro registers must be lowercase letters");
                    return true;
                };
                self.start_macro_recording(register);
            }
            PendingMacro::Playback { count } => {
                let Some(register) = self.resolve_macro_playback_register(key) else {
                    return true;
                };
                self.replay_macro_register(register, count);
            }
        }
        true
    }

    /// Return whether `key` should be appended to the active recording.
    ///
    /// Returns `true` when the key belongs to the supported recording surface,
    /// and `false` when it is a macro-control key or an unsupported picker key.
    pub(super) fn should_capture_macro_key(&self, key: Key) -> bool {
        if !self.macro_state.is_recording() || self.macro_state.is_replaying() {
            return false;
        }
        if self.pending_macro.is_some() || self.active_picker_kind().is_some() {
            return false;
        }
        !self.is_macro_control_key(key)
    }

    /// Append one normalized key to the active recording when eligible.
    pub(super) fn capture_macro_key(&mut self, key: Key) {
        if self.should_capture_macro_key(key) {
            self.macro_state.push_recorded_key(key);
        }
    }

    /// Return whether `key` triggers one of the macro control actions.
    ///
    /// Returns `true` when the active mode binds the key to start/stop recording
    /// or to begin playback, and `false` for ordinary editable content.
    fn is_macro_control_key(&self, key: Key) -> bool {
        matches!(
            self.keybindings
                .get_binding(key, &self.mode)
                .and_then(Binding::as_action_binding),
            Some(ActionBinding::Single(
                Action::BeginMacroRecord | Action::BeginMacroPlayback
            ))
        )
    }

    /// Resolve one typed register key for playback, handling `@@`.
    fn resolve_macro_playback_register(&mut self, key: Key) -> Option<char> {
        if key == Key::Char('@') {
            // `@@` reuses the most recently replayed register instead of asking
            // the caller to remember which register was last executed.
            let Some(register) = self.macro_state.last_played_register() else {
                self.show_error_message("No macro to replay");
                return None;
            };
            return Some(register);
        }
        let Some(register) = Self::macro_register_from_key(key) else {
            self.show_error_message("Macro registers must be lowercase letters");
            return None;
        };
        Some(register)
    }

    /// Convert one typed key into a supported lowercase macro register.
    fn macro_register_from_key(key: Key) -> Option<char> {
        match key {
            Key::Char(register) if register.is_ascii_lowercase() => Some(register),
            _ => None,
        }
    }

    /// Start recording into `register` unless playback is already active.
    fn start_macro_recording(&mut self, register: char) {
        if self.macro_state.is_replaying() {
            self.show_error_message("Cannot record a macro during playback");
            return;
        }
        self.macro_state.begin_recording(register);
    }

    /// Stop the current recording and keep the stored register available.
    fn stop_macro_recording(&mut self) {
        let Some(register) = self.macro_state.stop_recording() else {
            return;
        };
        self.show_status_message(format!("Recorded @{register}"));
    }

    /// Replay `register` through the ordinary key-handling pipeline.
    fn replay_macro_register(&mut self, register: char, count: usize) {
        if self.macro_state.is_recording() {
            self.show_error_message("Cannot replay a macro while recording");
            return;
        }
        if self.macro_state.is_replaying() {
            self.show_error_message("Cannot replay a macro during playback");
            return;
        }

        let Some(keys) = self
            .macro_state
            .register_keys(register)
            .map(|keys| keys.to_vec())
        else {
            self.show_error_message(format!("Macro @{register} is empty"));
            return;
        };
        if keys.is_empty() {
            self.show_error_message(format!("Macro @{register} is empty"));
            return;
        }

        let repeats = count.clamp(1, Self::MAX_COUNT);
        self.macro_state.set_last_played_register(register);
        self.macro_state.set_replaying(true);
        self.replay_keys_through_input(&keys, repeats);
        self.macro_state.set_replaying(false);
    }

    /// Execute one config-defined replay binding through the ordinary key pipeline.
    pub(super) fn execute_config_replay_binding(
        &mut self,
        binding: &ReplayBinding,
        count: Option<usize>,
    ) {
        if self
            .active_config_replays
            .iter()
            .any(|active| active == &binding.recursion_id)
        {
            self.show_error_message(format!(
                "Config replay binding `{}` would recurse",
                binding.trigger
            ));
            return;
        }

        // Track active replay ids so mutually recursive bindings fail cleanly
        // while non-recursive nested replays can still execute normally.
        self.active_config_replays
            .push(binding.recursion_id.clone());
        let repeats = count.map_or(1, |value| value.clamp(1, Self::MAX_COUNT));
        let keys = binding
            .keys
            .iter()
            .map(|key| key.to_key().expect("validated replay keys must convert"))
            .collect::<Vec<_>>();
        self.replay_keys_through_input(&keys, repeats);
        self.active_config_replays.pop();
    }

    /// Feed a stored key stream back through `handle_key()` in left-to-right order.
    fn replay_keys_through_input(&mut self, keys: &[Key], repeats: usize) {
        // Re-enter `handle_key()` so replay follows the same routing, prompts,
        // and side effects as live input instead of inventing a second path.
        'replay: for _ in 0..repeats {
            for key in keys.iter().copied() {
                self.handle_key(key);
                if self.should_quit() {
                    break 'replay;
                }
            }
        }
    }
}
