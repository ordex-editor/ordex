//! Clipboard-register prefix helpers for `EditorState`.

use super::*;
use crate::clipboard::{
    ClipboardPastePosition, ClipboardPasteRequest, ClipboardPayload, ClipboardPayloadKind,
    ClipboardRegister, ClipboardWriteRequest,
};

/// Pending clipboard-register prefix state for Vim-style `"` commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingRegister {
    AwaitTarget,
    Selected(ClipboardRegister),
}

impl PendingRegister {
    /// Return the typed prefix label shown in the status line while this prefix is pending.
    pub(super) fn prefix_label(self, count: Option<usize>) -> String {
        let mut label = String::new();
        if let Some(count) = count {
            label.push_str(&count.to_string());
        }
        label.push('"');
        if let Self::Selected(register) = self {
            label.push(register.key_char());
        }
        label
    }
}

impl EditorState {
    /// Start waiting for one Vim-style clipboard register after a typed `"`.
    pub(super) fn begin_register_prefix(&mut self) {
        self.pending_sequence.clear();
        self.pending_sequence_count = None;
        self.pending_sequence_motion_count = None;
        self.pending_register = Some(PendingRegister::AwaitTarget);
    }

    /// Consume one key while a clipboard register prefix is pending.
    ///
    /// Returns `true` when the register prefix consumed the key and `false` when
    /// no register prefix is currently active.
    pub(super) fn handle_pending_register_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_register else {
            return false;
        };
        if matches!(key, Key::Esc) {
            self.pending_register = None;
            return true;
        }

        match pending {
            PendingRegister::AwaitTarget => self.handle_register_target_key(key),
            PendingRegister::Selected(register) => {
                self.handle_registered_command_key(key, register)
            }
        }
    }

    /// Resolve the typed `+` or `*` register after one pending `"`.
    fn handle_register_target_key(&mut self, key: Key) -> bool {
        let Some(register) = Self::clipboard_register_from_key(key) else {
            self.pending_register = None;
            self.show_status_message("Clipboard registers must be + or *");
            return true;
        };
        self.pending_register = Some(PendingRegister::Selected(register));
        true
    }

    /// Resolve the command typed after one selected clipboard register.
    fn handle_registered_command_key(&mut self, key: Key, register: ClipboardRegister) -> bool {
        if let Some(digit) = Self::key_count_digit(key)
            && let Some(next) = Self::append_count_digit(self.pending_count, digit)
        {
            self.pending_count = Some(next);
            return true;
        }

        let binding = self.keybindings.get_binding(key, &self.mode).cloned();
        self.pending_register = None;
        let Some(binding) = binding else {
            self.show_status_message(
                "Clipboard registers only support yank, delete, change, and paste",
            );
            return true;
        };

        let count = self.pending_count.take();
        let key_input = KeyInput::from(key);
        self.execute_registered_binding(&binding, count, register, key_input);
        true
    }

    /// Execute one bound action with an explicit clipboard register target.
    pub(super) fn execute_registered_binding(
        &mut self,
        binding: &ActionBinding,
        count: Option<usize>,
        register: ClipboardRegister,
        trigger: KeyInput,
    ) {
        let ActionBinding::Single(action) = binding else {
            self.show_status_message(
                "Clipboard register prefixes do not support multi-action bindings",
            );
            return;
        };
        let mode_before = self.mode.clone();
        let undo_depth_before = self.undo_stack.len();
        let executed = self.execute_registered_action(*action, count, register, trigger);
        if !executed {
            return;
        }
        self.capture_repeat_after_binding(
            binding,
            count,
            Some(register),
            &mode_before,
            undo_depth_before,
        );
    }

    /// Execute one supported action against an explicit clipboard register.
    ///
    /// Returns `true` when the action was accepted for the explicit register and
    /// `false` when the action is unsupported for clipboard-register prefixes.
    fn execute_registered_action(
        &mut self,
        action: Action,
        count: Option<usize>,
        register: ClipboardRegister,
        trigger: KeyInput,
    ) -> bool {
        match action {
            Action::BeginDeleteOperator => {
                self.begin_operator(
                    OperatorKind::Delete,
                    Some(trigger),
                    count.map(|value| value.clamp(1, Self::MAX_COUNT)),
                    Some(register),
                );
                true
            }
            Action::BeginChangeOperator => {
                self.begin_operator(
                    OperatorKind::Change,
                    Some(trigger),
                    count.map(|value| value.clamp(1, Self::MAX_COUNT)),
                    Some(register),
                );
                true
            }
            Action::BeginYankOperator => {
                self.begin_operator(
                    OperatorKind::Yank,
                    Some(trigger),
                    count.map(|value| value.clamp(1, Self::MAX_COUNT)),
                    Some(register),
                );
                true
            }
            Action::YankSelection => {
                self.yank_visual_selection_into_register(register);
                true
            }
            Action::DeleteSelection => {
                self.delete_visual_selection_into_register(false, register);
                true
            }
            Action::ChangeSelection => {
                self.delete_visual_selection_into_register(true, register);
                true
            }
            Action::YankCurrentLine => {
                self.yank_current_line_count_into_register(count.unwrap_or(1), register);
                true
            }
            Action::PasteAfterCursor => {
                self.request_clipboard_paste(
                    register,
                    ClipboardPastePosition::After,
                    count.unwrap_or(1).clamp(1, Self::MAX_COUNT),
                );
                true
            }
            Action::PasteBeforeCursor => {
                self.request_clipboard_paste(
                    register,
                    ClipboardPastePosition::Before,
                    count.unwrap_or(1).clamp(1, Self::MAX_COUNT),
                );
                true
            }
            _ => {
                self.show_status_message(
                    "Clipboard registers only support yank, delete, change, and paste",
                );
                false
            }
        }
    }

    /// Return the clipboard register selected by `key`, if it is supported.
    fn clipboard_register_from_key(key: Key) -> Option<ClipboardRegister> {
        match key {
            Key::Char('+') => Some(ClipboardRegister::Clipboard),
            Key::Char('*') => Some(ClipboardRegister::Primary),
            _ => None,
        }
    }

    /// Queue one clipboard write request for the provided payload.
    pub(super) fn request_clipboard_write(
        &mut self,
        register: ClipboardRegister,
        payload: ClipboardPayload,
    ) {
        self.pending_request = Some(EditorRequest::WriteClipboard(ClipboardWriteRequest {
            register,
            payload,
        }));
    }

    /// Queue a clipboard write from the current unnamed register when requested.
    pub(super) fn queue_clipboard_write_from_yank_buffer(
        &mut self,
        register: Option<ClipboardRegister>,
    ) {
        let Some(register) = register else {
            return;
        };
        let Some(payload) = self.clipboard_payload_from_yank_buffer() else {
            return;
        };
        self.request_clipboard_write(register, payload);
    }

    /// Queue one clipboard paste request for the selected register and placement.
    pub(super) fn request_clipboard_paste(
        &mut self,
        register: ClipboardRegister,
        position: ClipboardPastePosition,
        count: usize,
    ) {
        self.pending_request = Some(EditorRequest::PasteClipboard(ClipboardPasteRequest {
            register,
            position,
            count,
        }));
    }

    /// Apply one clipboard payload through the existing paste helpers.
    pub(crate) fn apply_clipboard_paste(
        &mut self,
        payload: ClipboardPayload,
        position: ClipboardPastePosition,
        count: usize,
    ) {
        let kind = Self::yank_kind_from_clipboard(payload.kind);
        let yank_payload = YankBuffer {
            text: payload.text,
            kind,
        };

        self.with_history_transaction(|editor| {
            // Clipboard pastes deliberately avoid mutating the unnamed register so
            // explicit `\"+` and `\"*` paste stays scoped to the current command.
            for _ in 0..count.max(1) {
                let before = editor.buffer.chars_count();
                editor.paste_payload(
                    &yank_payload,
                    match position {
                        ClipboardPastePosition::Before => PastePosition::Before,
                        ClipboardPastePosition::After => PastePosition::After,
                    },
                );
                if editor.buffer.chars_count() == before {
                    break;
                }
            }
        });
    }

    /// Yank the current visual selection and queue a matching clipboard write.
    fn yank_visual_selection_into_register(&mut self, register: ClipboardRegister) {
        let Some(selection) = self.visual_selection() else {
            return;
        };
        let payload = self.clipboard_payload_from_visual_selection(selection);
        self.yank_visual_selection();
        self.request_clipboard_write(register, payload);
    }

    /// Delete or change the current visual selection and queue a clipboard write.
    fn delete_visual_selection_into_register(
        &mut self,
        enter_insert: bool,
        register: ClipboardRegister,
    ) {
        let Some(selection) = self.visual_selection() else {
            return;
        };
        let payload = self.clipboard_payload_from_visual_selection(selection);
        self.delete_visual_selection(enter_insert);
        self.request_clipboard_write(register, payload.clone());
        self.set_pending_visual_register(register);
    }

    /// Yank the current linewise selection and queue a clipboard write.
    fn yank_current_line_count_into_register(&mut self, count: usize, register: ClipboardRegister) {
        let selection = self.current_line_range(count);
        let payload = self.clipboard_payload_from_range(selection, YankKind::Line);
        self.yank_current_line_count(count);
        self.request_clipboard_write(register, payload);
    }

    /// Build one clipboard payload from the active visual selection.
    fn clipboard_payload_from_visual_selection(
        &self,
        selection: VisualSelection,
    ) -> ClipboardPayload {
        match selection {
            VisualSelection::Character(range) => {
                self.clipboard_payload_from_range(range, YankKind::Character)
            }
            VisualSelection::Line(range) => {
                self.clipboard_payload_from_range(range, YankKind::Line)
            }
            VisualSelection::Block(selection) => ClipboardPayload {
                text: selection.yank_lines(&self.buffer).join("\n"),
                kind: ClipboardPayloadKind::Block,
            },
        }
    }

    /// Build one clipboard payload from a contiguous selection range.
    fn clipboard_payload_from_range(
        &self,
        selection: SelectionRange,
        kind: YankKind,
    ) -> ClipboardPayload {
        ClipboardPayload {
            text: self.buffer.slice_string(selection.start, selection.end),
            kind: Self::clipboard_kind_from_yank(kind),
        }
    }

    /// Build one clipboard payload from the current unnamed register, if any.
    fn clipboard_payload_from_yank_buffer(&self) -> Option<ClipboardPayload> {
        self.yank_buffer.as_ref().map(|buffer| ClipboardPayload {
            text: buffer.text.clone(),
            kind: Self::clipboard_kind_from_yank(buffer.kind),
        })
    }

    /// Convert one internal yank shape into the clipboard payload kind.
    pub(super) fn clipboard_kind_from_yank(kind: YankKind) -> ClipboardPayloadKind {
        match kind {
            YankKind::Character => ClipboardPayloadKind::Character,
            YankKind::Line => ClipboardPayloadKind::Line,
            YankKind::Block => ClipboardPayloadKind::Block,
        }
    }

    /// Convert one clipboard payload kind into the internal unnamed-register shape.
    fn yank_kind_from_clipboard(kind: ClipboardPayloadKind) -> YankKind {
        match kind {
            ClipboardPayloadKind::Character => YankKind::Character,
            ClipboardPayloadKind::Line => YankKind::Line,
            ClipboardPayloadKind::Block => YankKind::Block,
        }
    }
}
