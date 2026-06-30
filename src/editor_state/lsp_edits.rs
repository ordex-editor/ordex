//! Editor-side application of LSP-provided workspace edits.

use super::*;
use crate::lsp::protocol::{LspDocumentEdit, LspTextEdit};
use crate::path_utils::display_path_for_ui;
use std::cmp::Reverse;

/// Small summary of one applied workspace edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkspaceEditSummary {
    /// Number of files touched by the applied workspace edit.
    changed_files: usize,
    /// Total number of individual text edits applied across all changed files.
    changed_edits: usize,
}

impl EditorState {
    /// Apply one completed code-action lookup result and report whether UI state changed.
    ///
    /// Returns `true` when the result matched the active in-flight code-action
    /// request for its source buffer, and `false` when the response was stale or
    /// the originating buffer no longer exists.
    pub(crate) fn apply_code_action_lookup_result(
        &mut self,
        result: CodeActionLookupResult,
    ) -> bool {
        if self.active_buffer_id != result.buffer_id {
            self.switch_to_buffer_id(result.buffer_id);
        }
        if self.active_buffer_id != result.buffer_id {
            return false;
        }
        let Some(lookup) = self
            .code_action_lookup_for_buffer(result.buffer_id)
            .copied()
        else {
            return false;
        };
        // Token/version mismatches mean a newer lookup replaced this request or
        // the source buffer moved on, so opening a picker would show stale edits.
        if lookup.token != result.lookup_token || lookup.document_version != result.document_version
        {
            return false;
        }
        self.finish_document_sync(result.buffer_id, result.document_version, true);
        self.clear_code_action_lookup(result.buffer_id);
        match result.outcome {
            CodeActionLookupOutcome::Found(actions) => self.open_code_action_picker(
                result.buffer_id,
                lookup.request_edit_generation,
                actions,
            ),
            CodeActionLookupOutcome::NotFound => {
                self.show_error_message("No supported code actions available")
            }
            CodeActionLookupOutcome::UnsupportedFile(message)
            | CodeActionLookupOutcome::UnsupportedProject(message)
            | CodeActionLookupOutcome::Unavailable(message)
            | CodeActionLookupOutcome::Error(message) => self.show_error_message(message),
        }
        true
    }

    /// Apply one completed rename lookup result and report whether UI state changed.
    ///
    /// Returns `true` when the result matched the active in-flight rename request
    /// for its source buffer, and `false` when the response was stale or the
    /// originating buffer no longer exists.
    pub(crate) fn apply_rename_lookup_result(&mut self, result: RenameLookupResult) -> bool {
        let Some(lookup) = self.rename_lookup_for_buffer(result.buffer_id).cloned() else {
            return false;
        };
        // Token/version mismatches mean a newer rename replaced this request or
        // the source buffer moved on, so applying the returned edit would mix
        // two unrelated document snapshots.
        if lookup.token != result.lookup_token || lookup.document_version != result.document_version
        {
            return false;
        }
        self.finish_document_sync(result.buffer_id, result.document_version, true);
        self.clear_rename_lookup(result.buffer_id);
        match result.outcome {
            RenameLookupOutcome::Applied(edit) => {
                match self.apply_workspace_edit(
                    &edit,
                    result.buffer_id,
                    lookup.request_edit_generation,
                    "Rename",
                ) {
                    Ok(summary) if summary.changed_edits == 0 => {
                        self.show_error_message("Rename produced no changes");
                    }
                    Ok(summary) => {
                        self.show_status_message(format!(
                            "Renamed symbol across {} file(s)",
                            summary.changed_files
                        ));
                    }
                    Err(error) => self.show_error_message(error),
                }
            }
            RenameLookupOutcome::NotFound => self.show_error_message("No rename changes found"),
            RenameLookupOutcome::UnsupportedFile(message)
            | RenameLookupOutcome::UnsupportedProject(message)
            | RenameLookupOutcome::Unavailable(message)
            | RenameLookupOutcome::Error(message) => self.show_error_message(message),
        }
        true
    }

    /// Apply one selected code action through the shared workspace-edit path.
    pub(crate) fn apply_selected_code_action(
        &mut self,
        action: &LspCodeAction,
        source_buffer_id: usize,
        request_edit_generation: u64,
    ) {
        // Reuse the same multi-buffer edit path as rename so code actions keep
        // undo/history behavior and stale-target safety checks in one place.
        match self.apply_workspace_edit(
            &action.edit,
            source_buffer_id,
            request_edit_generation,
            "Code action",
        ) {
            Ok(summary) if summary.changed_edits == 0 => {
                self.show_error_message(format!(
                    "Code action \"{}\" made no changes",
                    action.title
                ));
            }
            Ok(summary) => {
                self.show_status_message(format!(
                    "Applied code action \"{}\" across {} file(s)",
                    action.title, summary.changed_files
                ));
            }
            Err(error) => self.show_error_message(error),
        }
    }

    /// Return the stored rename lookup metadata for `buffer_id`, if any.
    fn rename_lookup_for_buffer(&self, buffer_id: usize) -> Option<&ActiveRenameLookup> {
        if self.active_buffer_id == buffer_id {
            return self.active_rename_lookup.as_ref();
        }
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .find(|buffer| buffer.id == buffer_id)
            .and_then(|buffer| buffer.active_rename_lookup.as_ref())
    }

    /// Clear the stored rename lookup metadata for `buffer_id`.
    fn clear_rename_lookup(&mut self, buffer_id: usize) {
        if self.active_buffer_id == buffer_id {
            self.active_rename_lookup = None;
            return;
        }
        if let Some(buffer) = self
            .buffer_manager
            .inactive_buffers_mut()
            .iter_mut()
            .find(|buffer| buffer.id == buffer_id)
        {
            buffer.active_rename_lookup = None;
        }
    }

    /// Return the stored code-action lookup metadata for `buffer_id`, if any.
    fn code_action_lookup_for_buffer(&self, buffer_id: usize) -> Option<&ActiveCodeActionLookup> {
        if self.active_buffer_id == buffer_id {
            return self.active_code_action_lookup.as_ref();
        }
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .find(|buffer| buffer.id == buffer_id)
            .and_then(|buffer| buffer.active_code_action_lookup.as_ref())
    }

    /// Clear the stored code-action lookup metadata for `buffer_id`.
    fn clear_code_action_lookup(&mut self, buffer_id: usize) {
        if self.active_buffer_id == buffer_id {
            self.active_code_action_lookup = None;
            return;
        }
        // Responses are keyed by the originating buffer id, so clear inactive
        // lookups in place instead of switching buffers just for bookkeeping.
        if let Some(buffer) = self
            .buffer_manager
            .inactive_buffers_mut()
            .iter_mut()
            .find(|buffer| buffer.id == buffer_id)
        {
            buffer.active_code_action_lookup = None;
        }
    }

    /// Apply one workspace edit while preserving the visible active buffer.
    fn apply_workspace_edit(
        &mut self,
        edit: &LspWorkspaceEdit,
        source_buffer_id: usize,
        request_edit_generation: u64,
        operation_name: &str,
    ) -> Result<WorkspaceEditSummary, String> {
        self.ensure_workspace_edit_targets_are_current(
            edit,
            source_buffer_id,
            request_edit_generation,
            operation_name,
        )?;
        let original_active_buffer_id = self.active_buffer_id;
        let mut open_edits = Vec::with_capacity(edit.document_edits.len());

        // Open every touched file before mutating text so any buffer-creation
        // failure is surfaced before the first edit is applied.
        for document_edit in &edit.document_edits {
            let buffer_id =
                self.ensure_workspace_edit_buffer_open(&document_edit.path, operation_name)?;
            open_edits.push((buffer_id, document_edit.clone()));
        }

        for (buffer_id, document_edit) in &open_edits {
            self.apply_open_buffer_edit(*buffer_id, document_edit)?;
        }
        if self.active_buffer_id != original_active_buffer_id {
            self.switch_to_buffer_id(original_active_buffer_id);
        }
        Ok(WorkspaceEditSummary {
            changed_files: open_edits.len(),
            changed_edits: edit
                .document_edits
                .iter()
                .map(|entry| entry.edits.len())
                .sum(),
        })
    }

    /// Reject workspace-edit targets whose open-buffer state no longer matches the request snapshot.
    fn ensure_workspace_edit_targets_are_current(
        &self,
        edit: &LspWorkspaceEdit,
        source_buffer_id: usize,
        request_edit_generation: u64,
        operation_name: &str,
    ) -> Result<(), String> {
        for document_edit in &edit.document_edits {
            let Some(buffer_id) = self.open_buffer_id_for_path(&document_edit.path) else {
                continue;
            };
            // The source buffer produced the rename request, so its own edit path
            // is validated by the token/version checks before this helper runs.
            if buffer_id == source_buffer_id {
                continue;
            }

            // Workspace edits are only safe to merge into another open buffer when
            // the language server has already seen that buffer's current text.
            // Otherwise the returned ranges may have been computed against older
            // content and could land on the wrong spans.
            if self
                .buffer_has_unsynced_lsp_state(buffer_id)
                .unwrap_or(false)
            {
                return Err(format!(
                    "{operation_name} aborted because open target buffer \"{}\" has unsynced changes",
                    display_path_for_ui(&document_edit.path)
                ));
            }
            // Buffers that were already dirty when the rename started are fine as
            // long as they have not changed since then: the request generation
            // tracks later local edits that would make the server response stale.
            if self.buffer_is_modified(buffer_id).unwrap_or(false)
                && self
                    .buffer_last_edit_generation(buffer_id)
                    .is_some_and(|generation| generation > request_edit_generation)
            {
                return Err(format!(
                    "{operation_name} aborted because open target buffer \"{}\" changed after the {operation_name} started",
                    display_path_for_ui(&document_edit.path)
                ));
            }
        }
        Ok(())
    }

    /// Return the open buffer id for `path`, when that file is already open.
    fn open_buffer_id_for_path(&self, path: &Path) -> Option<usize> {
        if paths_match(&self.file_path, path) {
            return Some(self.active_buffer_id);
        }
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .find(|buffer| paths_match(&buffer.file_path, path))
            .map(|buffer| buffer.id)
    }

    /// Return whether `buffer_id` has unsaved changes.
    fn buffer_is_modified(&self, buffer_id: usize) -> Option<bool> {
        if self.active_buffer_id == buffer_id {
            return Some(self.buffer.is_modified());
        }
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .find(|buffer| buffer.id == buffer_id)
            .map(|buffer| buffer.buffer.is_modified())
    }

    /// Return whether `buffer_id` still has unsynced local edits.
    ///
    /// Returns `true` when the buffer is dirty and still has queued LSP sync
    /// work, and `false` when the server has already seen the current text.
    fn buffer_has_unsynced_lsp_state(&self, buffer_id: usize) -> Option<bool> {
        if self.active_buffer_id == buffer_id {
            return Some(
                self.buffer.is_modified()
                    && (!self.pending_lsp_changes.is_empty() || self.pending_lsp_sync_at.is_some()),
            );
        }
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .find(|buffer| buffer.id == buffer_id)
            .map(|buffer| {
                buffer.buffer.is_modified()
                    && (!buffer.pending_lsp_changes.is_empty()
                        || buffer.pending_lsp_sync_at.is_some())
            })
    }

    /// Return the last edit generation recorded for `buffer_id`.
    fn buffer_last_edit_generation(&self, buffer_id: usize) -> Option<u64> {
        if self.active_buffer_id == buffer_id {
            return Some(self.last_edit_generation);
        }
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .find(|buffer| buffer.id == buffer_id)
            .map(|buffer| buffer.last_edit_generation)
    }

    /// Ensure one workspace-edit target is open as a live buffer and return its id.
    fn ensure_workspace_edit_buffer_open(
        &mut self,
        path: &Path,
        operation_name: &str,
    ) -> Result<usize, String> {
        if let Some(buffer_id) = self.open_buffer_id_for_path(path) {
            return Ok(buffer_id);
        }
        if !path.exists() {
            return Err(format!(
                "{operation_name} target \"{}\" does not exist",
                display_path_for_ui(path)
            ));
        }
        self.open_buffer(path).map_err(|error| {
            format!(
                "Failed to open {operation_name} target \"{}\": {error}",
                display_path_for_ui(path)
            )
        })?;
        Ok(self.active_buffer_id)
    }

    /// Apply one document edit to an already open buffer.
    fn apply_open_buffer_edit(
        &mut self,
        buffer_id: usize,
        document_edit: &LspDocumentEdit,
    ) -> Result<(), String> {
        let original_active_buffer_id = self.active_buffer_id;
        if self.active_buffer_id != buffer_id {
            self.switch_to_buffer_id(buffer_id);
        }
        if self.active_buffer_id != buffer_id {
            return Err(format!(
                "Failed to activate rename target \"{}\"",
                display_path_for_ui(&document_edit.path)
            ));
        }

        // The active-buffer editing path maintains undo history, syntax state,
        // swap refresh, and queued LSP sync changes in one place.
        let result = self.apply_text_edits_to_active_buffer(&document_edit.edits);
        if self.active_buffer_id != original_active_buffer_id {
            self.switch_to_buffer_id(original_active_buffer_id);
        }
        result
    }

    /// Apply one ordered text-edit list to the active buffer.
    fn apply_text_edits_to_active_buffer(&mut self, edits: &[LspTextEdit]) -> Result<(), String> {
        let operations = compile_text_edit_operations(&self.buffer, edits)?;
        self.begin_history_transaction();
        for (start_char, end_char, new_text) in &operations {
            self.remove_buffer_range(*start_char, *end_char);
            if !new_text.is_empty() {
                self.insert_buffer_text(*start_char, new_text);
            }
        }
        self.clamp_active_cursor_after_workspace_edit();
        self.finish_history_transaction();
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        Ok(())
    }

    /// Clamp the active cursor after an external workspace edit mutates the buffer.
    fn clamp_active_cursor_after_workspace_edit(&mut self) {
        // Workspace edits can delete the cursor's line or trim it shorter, so
        // clamp against the post-edit buffer before the viewport reads it.
        if self.mode == Mode::Insert {
            self.cursor.clamp_to_buffer(&self.buffer);
        } else {
            self.cursor.clamp_to_buffer_normal(&self.buffer);
        }
    }
}

/// Convert text edits into stable character-index operations before mutating text.
fn compile_text_edit_operations(
    buffer: &TextBuffer,
    edits: &[LspTextEdit],
) -> Result<Vec<(usize, usize, String)>, String> {
    let sorted_edits = sort_text_edits_descending(edits);
    let mut scratch = buffer.clone();
    let mut operations = Vec::with_capacity(sorted_edits.len());

    // Validate positions against a scratch copy first so malformed edit batches
    // cannot leave either an open buffer or a detached file half-updated.
    for edit in &sorted_edits {
        let start_char = lsp_position_to_char_idx(&scratch, edit.range.start)?;
        let end_char = lsp_position_to_char_idx(&scratch, edit.range.end)?;
        scratch.remove(start_char, end_char);
        scratch.insert(start_char, &edit.new_text);
        operations.push((start_char, end_char, edit.new_text.clone()));
    }
    Ok(operations)
}

/// Convert one LSP position into a character index while validating the buffer text.
fn lsp_position_to_char_idx(buffer: &TextBuffer, position: LspPosition) -> Result<usize, String> {
    let max_line = buffer.lines_count().saturating_sub(1);
    if position.line > max_line {
        return Err(format!(
            "rename target references missing line {}",
            position.line + 1
        ));
    }
    let line_start = buffer.line_to_char(position.line);
    let line_text = buffer
        .line(position.line)
        .ok_or_else(|| {
            format!(
                "rename target references missing line {}",
                position.line + 1
            )
        })?
        .to_string();
    let mut utf16_units = 0usize;
    let mut char_offset = 0usize;

    // LSP columns count UTF-16 code units, so scalar values beyond the Basic
    // Multilingual Plane (the first 65,536 Unicode code points) advance by two
    // units while ordinary ASCII still advances by one.
    for ch in line_text.chars() {
        if utf16_units >= position.character {
            break;
        }
        utf16_units += ch.len_utf16();
        char_offset += 1;
        if utf16_units > position.character {
            return Err(format!(
                "rename target splits a UTF-16 code point on line {}",
                position.line + 1
            ));
        }
    }
    if utf16_units != position.character {
        return Err(format!(
            "rename target column {} is outside line {}",
            position.character + 1,
            position.line + 1
        ));
    }
    Ok(line_start + char_offset)
}

/// Return a descending copy of `edits` so later offsets stay stable.
fn sort_text_edits_descending(edits: &[LspTextEdit]) -> Vec<LspTextEdit> {
    let mut sorted = edits.to_vec();
    sorted.sort_by_key(|edit| {
        (
            Reverse(edit.range.start.line),
            Reverse(edit.range.start.character),
            Reverse(edit.range.end.line),
            Reverse(edit.range.end.character),
        )
    });
    sorted
}
