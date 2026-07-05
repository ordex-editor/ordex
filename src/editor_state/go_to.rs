//! Go-to motion helpers for file, alternate-buffer, and last-change navigation.

use super::*;
use crate::corresponding_file::find_corresponding_file_path;
use crate::file_targets::{
    FileTargetPathResolution, find_file_target, resolve_file_target_path_detailed,
};
use crate::path_utils::current_dir_relative_path;

/// One open-buffer target chosen for a go-to motion.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BufferTarget {
    /// Stable buffer identifier that should become active.
    buffer_id: usize,
    /// Target buffer-local cursor position in character coordinates.
    char_idx: usize,
    /// Monotonic generation used to rank competing change targets.
    generation: u64,
}

impl EditorState {
    /// Record the active buffer at the front of the recent-buffer history.
    pub(super) fn record_active_buffer(&mut self) {
        // Keep only one copy of each buffer so alternate-file traversal
        // reflects recency rather than duplicate visits to the same buffer id.
        self.recent_buffers
            .retain(|buffer_id| *buffer_id != self.active_buffer_id);
        self.recent_buffers.push_front(self.active_buffer_id);
    }

    /// Jump to the file-like token under the cursor.
    pub(super) fn goto_file_under_cursor(&mut self) {
        self.goto_file_target(false);
    }

    /// Jump to the file-like token under the cursor and honor `:line[:column]`.
    pub(super) fn goto_file_under_cursor_at_position(&mut self) {
        self.goto_file_target(true);
    }

    /// Jump to the most recently active buffer that is still open.
    pub(super) fn goto_alternate_file(&mut self) {
        let Some(target) = self.next_alternate_buffer_target() else {
            self.show_status_message("No alternate file");
            return;
        };
        self.goto_buffer_target(target);
    }

    /// Jump to the corresponding source/header interface file for the active buffer.
    pub(super) fn goto_corresponding_file(&mut self) {
        let Some(active_path) = self.active_named_file_path() else {
            self.show_status_message("No file name");
            return;
        };
        let target_path = match find_corresponding_file_path(active_path) {
            Ok(path) => path,
            Err(error) => {
                self.show_status_message(error.status_message());
                return;
            }
        };
        if !self.record_jump_origin_for_destination(&target_path, 0, 0) {
            return;
        }

        // Open through cwd-relative display logic so status/path rendering stays
        // consistent with other cross-file navigation flows.
        let open_path = current_dir_relative_path(&target_path);
        if let Err(error) = self.open_buffer(open_path.as_ref()) {
            self.show_error_message(format!(
                "Failed to open corresponding file \"{}\": {error}",
                open_path.display()
            ));
            return;
        }

        self.finish_nonlocal_navigation();
        self.clear_status_message();
    }

    /// Jump to the cursor position after the most recently committed change.
    pub(super) fn goto_last_modification(&mut self) {
        let Some(target) = self.last_modification_target() else {
            self.show_status_message("No committed change");
            return;
        };
        self.goto_buffer_target(target);
    }

    /// Resolve and open one file target under the cursor.
    fn goto_file_target(&mut self, allow_position_suffix: bool) {
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        let Some(target) = find_file_target(&self.buffer, cursor_char_idx, allow_position_suffix)
        else {
            self.show_status_message("No file target under cursor");
            return;
        };
        let path = match resolve_file_target_path_detailed(
            self.active_named_file_path(),
            &target.path_text,
        ) {
            FileTargetPathResolution::Resolved(path) => path,
            FileTargetPathResolution::MissingWorkingDirectory => {
                self.show_error_message(
                    "Cannot resolve file target because the working directory is unavailable",
                );
                return;
            }
            FileTargetPathResolution::MissingHomeDirectory => {
                self.show_error_message(
                    "Cannot resolve file target because the home directory is unavailable",
                );
                return;
            }
            FileTargetPathResolution::MissingPath => {
                self.show_status_message("No file target under cursor");
                return;
            }
        };

        let target_line = target.line.unwrap_or(1).saturating_sub(1);
        let target_column = target.column.unwrap_or(1).saturating_sub(1);
        if !self.record_jump_origin_for_destination(&path, target_line, target_column) {
            return;
        }

        let open_path = current_dir_relative_path(&path);
        if let Err(error) = self.open_buffer(open_path.as_ref()) {
            self.show_error_message(format!(
                "Failed to open file target \"{}\": {error}",
                open_path.display()
            ));
            return;
        }

        // Clamp the parsed destination after opening so nonexistent files and
        // short lines still land at the nearest valid cursor position.
        self.cursor = self.clamped_normal_cursor(target_line, target_column);
        self.finish_nonlocal_navigation();
        self.clear_status_message();
    }

    /// Return one open-buffer target for the latest committed change, if any.
    fn last_modification_target(&self) -> Option<BufferTarget> {
        let active = self
            .last_committed_change_char_idx
            .map(|char_idx| BufferTarget {
                buffer_id: self.active_buffer_id,
                char_idx,
                generation: self.last_edit_generation,
            });
        active
            .into_iter()
            .chain(
                self.buffer_manager
                    .inactive_buffers()
                    .iter()
                    .filter_map(|buffer| {
                        buffer
                            .last_committed_change_char_idx
                            .map(|char_idx| BufferTarget {
                                buffer_id: buffer.id,
                                char_idx,
                                generation: buffer.last_edit_generation,
                            })
                    }),
            )
            .max_by_key(|target| target.generation)
    }

    /// Return the open-buffer target associated with `buffer_id`, if any.
    fn buffer_target_for_id(&self, buffer_id: usize) -> Option<BufferTarget> {
        if buffer_id == self.active_buffer_id {
            return Some(BufferTarget {
                buffer_id: self.active_buffer_id,
                char_idx: self.cursor.to_char_index(&self.buffer),
                generation: self.last_edit_generation,
            });
        }
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .find(|buffer| buffer.id == buffer_id)
            .map(|buffer| BufferTarget {
                buffer_id: buffer.id,
                char_idx: buffer.cursor.to_char_index(&buffer.buffer),
                generation: buffer.last_edit_generation,
            })
    }

    /// Return the next alternate buffer target, lazily dropping stale ids.
    fn next_alternate_buffer_target(&mut self) -> Option<BufferTarget> {
        let mut idx = 0;
        while idx < self.recent_buffers.len() {
            let buffer_id = self.recent_buffers[idx];
            // Skip the active buffer so `ga` toggles to a different recent file
            // instead of immediately selecting the buffer already on screen.
            if buffer_id == self.active_buffer_id {
                idx += 1;
                continue;
            }

            let Some(target) = self.buffer_target_for_id(buffer_id) else {
                // Remove stale ids for buffers that were closed so later scans do
                // not pay the same lookup cost or consider invalid alternates.
                self.recent_buffers.remove(idx);
                continue;
            };
            return Some(target);
        }
        None
    }

    /// Return the file path for `buffer_id` when that buffer is named.
    pub(super) fn named_file_path_for_buffer_id(&self, buffer_id: usize) -> Option<&Path> {
        if buffer_id == self.active_buffer_id {
            return self.active_named_file_path();
        }
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .find(|buffer| buffer.id == buffer_id)
            .and_then(BufferState::named_file_path)
    }

    /// Apply one buffer target while recording one jump-history origin.
    fn goto_buffer_target(&mut self, target: BufferTarget) {
        let current_char_idx = self.cursor.to_char_index(&self.buffer);
        if target.buffer_id == self.active_buffer_id && target.char_idx == current_char_idx {
            return;
        }

        self.jump_history.push_older(self.current_jump_location());
        self.jump_history.clear_newer();
        if target.buffer_id != self.active_buffer_id {
            self.switch_to_buffer_id(target.buffer_id);
            if target.buffer_id != self.active_buffer_id {
                self.show_error_message("Target buffer is no longer open");
                return;
            }
        }

        // Clamp after the buffer switch so the stored change location survives
        // file edits that shortened the target since the motion was recorded.
        let clamped = target.char_idx.min(self.buffer.chars_count());
        self.cursor = Cursor::from_char_index(&self.buffer, clamped);
        self.finish_nonlocal_navigation();
        self.clear_status_message();
    }
}
