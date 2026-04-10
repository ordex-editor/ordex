//! Editor-side application of LSP-provided workspace edits.

use super::*;
use crate::lsp::protocol::{LspDocumentEdit, LspTextEdit};
use crate::temp_paths;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;

/// Small summary of one applied workspace edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkspaceEditSummary {
    changed_files: usize,
    changed_edits: usize,
}

impl EditorState {
    /// Apply one completed rename lookup result and report whether UI state changed.
    ///
    /// Returns `true` when the result matched the active in-flight rename request
    /// for its source buffer, and `false` when the response was stale or the
    /// originating buffer no longer exists.
    pub(crate) fn apply_rename_lookup_result(&mut self, result: RenameLookupResult) -> bool {
        let Some(lookup) = self.rename_lookup_for_buffer(result.buffer_id).cloned() else {
            return false;
        };
        if lookup.token != result.lookup_token || lookup.document_version != result.document_version {
            return false;
        }
        self.finish_document_sync(result.buffer_id, result.document_version, true);
        self.clear_rename_lookup(result.buffer_id);
        match result.outcome {
            RenameLookupOutcome::Applied(edit) => match self.apply_workspace_edit(&edit, result.buffer_id)
            {
                Ok(summary) if summary.changed_edits == 0 => {
                    self.show_status_message("Rename produced no changes");
                }
                Ok(summary) => {
                    self.show_status_message(format!(
                        "Renamed symbol across {} file(s)",
                        summary.changed_files
                    ));
                }
                Err(error) => self.show_status_message(error),
            },
            RenameLookupOutcome::NotFound => self.show_status_message("No rename changes found"),
            RenameLookupOutcome::UnsupportedFile(message)
            | RenameLookupOutcome::UnsupportedProject(message)
            | RenameLookupOutcome::Unavailable(message)
            | RenameLookupOutcome::Error(message) => self.show_status_message(message),
        }
        true
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

    /// Apply one workspace edit while preserving the visible active buffer.
    fn apply_workspace_edit(
        &mut self,
        edit: &LspWorkspaceEdit,
        source_buffer_id: usize,
    ) -> Result<WorkspaceEditSummary, String> {
        self.ensure_workspace_edit_targets_are_safe(edit, source_buffer_id)?;
        let original_active_buffer_id = self.active_buffer_id;
        let mut disk_edits = Vec::new();
        let mut open_edits = Vec::new();

        // Split on-disk and open-buffer targets first so filesystem failures are
        // discovered before any in-memory buffer state is mutated.
        for document_edit in &edit.document_edits {
            if let Some(buffer_id) = self.open_buffer_id_for_path(&document_edit.path) {
                open_edits.push((buffer_id, document_edit.clone()));
            } else {
                disk_edits.push(document_edit.clone());
            }
        }

        for document_edit in &disk_edits {
            self.apply_disk_document_edit(document_edit)?;
        }
        for (buffer_id, document_edit) in &open_edits {
            self.apply_open_buffer_edit(*buffer_id, document_edit)?;
        }
        if self.active_buffer_id != original_active_buffer_id {
            self.switch_to_buffer_id(original_active_buffer_id);
        }
        Ok(WorkspaceEditSummary {
            changed_files: edit.document_edits.len(),
            changed_edits: edit.document_edits.iter().map(|entry| entry.edits.len()).sum(),
        })
    }

    /// Reject rename targets that would overwrite unsaved edits in other open buffers.
    fn ensure_workspace_edit_targets_are_safe(
        &self,
        edit: &LspWorkspaceEdit,
        source_buffer_id: usize,
    ) -> Result<(), String> {
        for document_edit in &edit.document_edits {
            let Some(buffer_id) = self.open_buffer_id_for_path(&document_edit.path) else {
                continue;
            };
            if buffer_id == source_buffer_id {
                continue;
            }
            if self.buffer_is_modified(buffer_id).unwrap_or(false) {
                return Err(format!(
                    "Rename aborted because open target buffer \"{}\" has unsaved changes",
                    current_dir_relative_path(&document_edit.path).display()
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
                current_dir_relative_path(&document_edit.path).display()
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
        self.finish_history_transaction();
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        Ok(())
    }

    /// Apply one document edit directly to a file that is not currently open.
    fn apply_disk_document_edit(&mut self, document_edit: &LspDocumentEdit) -> Result<(), String> {
        let file = File::open(&document_edit.path).map_err(|error| {
            format!(
                "Failed to open rename target \"{}\": {error}",
                current_dir_relative_path(&document_edit.path).display()
            )
        })?;
        let mut buffer = TextBuffer::from_reader(file).map_err(|error| {
            format!(
                "Failed to read rename target \"{}\": {error}",
                current_dir_relative_path(&document_edit.path).display()
            )
        })?;
        apply_text_edits_to_text_buffer(&mut buffer, &document_edit.edits)?;
        write_text_buffer_atomically(&buffer, &document_edit.path).map_err(|error| {
            format!(
                "Failed to write rename target \"{}\": {error}",
                current_dir_relative_path(&document_edit.path).display()
            )
        })
    }
}

/// Apply one list of descending LSP edits to a detached text buffer.
fn apply_text_edits_to_text_buffer(buffer: &mut TextBuffer, edits: &[LspTextEdit]) -> Result<(), String> {
    let operations = compile_text_edit_operations(buffer, edits)?;
    for (start_char, end_char, new_text) in &operations {
        buffer.remove(*start_char, *end_char);
        buffer.insert(*start_char, new_text);
    }
    Ok(())
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

/// Convert one LSP position into a character index within `buffer`.
fn lsp_position_to_char_idx(buffer: &TextBuffer, position: LspPosition) -> Result<usize, String> {
    let max_line = buffer.lines_count().saturating_sub(1);
    if position.line > max_line {
        return Err(format!("rename target references missing line {}", position.line + 1));
    }
    let line_start = buffer.line_to_char(position.line);
    let line_text = buffer
        .line(position.line)
        .ok_or_else(|| format!("rename target references missing line {}", position.line + 1))?
        .to_string();
    let mut utf16_units = 0usize;
    let mut char_offset = 0usize;

    // LSP columns count UTF-16 code units, so scalar values beyond BMP advance
    // by two units while ordinary ASCII still advances by one.
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
    sorted.sort_by(|left, right| {
        right
            .range
            .start
            .line
            .cmp(&left.range.start.line)
            .then_with(|| right.range.start.character.cmp(&left.range.start.character))
            .then_with(|| right.range.end.line.cmp(&left.range.end.line))
            .then_with(|| right.range.end.character.cmp(&left.range.end.character))
    });
    sorted
}

/// Write one detached text buffer through a sibling temp file and atomic rename.
fn write_text_buffer_atomically(buffer: &TextBuffer, target_path: &Path) -> io::Result<()> {
    let temp_path = temp_write_path(target_path)?;
    let write_result = (|| {
        // The temp file stays beside the destination so the final rename remains
        // atomic on the same filesystem instead of falling back to copy semantics.
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)?;
        buffer.write_to(&mut file)?;
        file.sync_all()?;
        fs::rename(&temp_path, target_path)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

/// Build one temp write path beside `target_path`.
fn temp_write_path(target_path: &Path) -> io::Result<PathBuf> {
    temp_paths::unique_sibling_temp_path(target_path, "ordex")
}
