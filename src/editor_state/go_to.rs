//! Go-to motion helpers for file, alternate-buffer, and last-change navigation.

use super::*;
use crate::file_targets::{find_file_target, resolve_file_target_path};
use crate::path_utils::current_dir_relative_path;

/// One open-buffer target chosen for a go-to motion.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BufferTarget {
    /// Stable buffer identifier that should become active.
    buffer_id: usize,
    /// File path associated with that buffer.
    file_path: PathBuf,
    /// Target buffer-local cursor position in character coordinates.
    char_idx: usize,
    /// Monotonic generation used to rank competing change targets.
    generation: u64,
}

impl EditorState {
    /// Record the active named file at the front of the recent-file history.
    pub(super) fn record_active_named_file(&mut self) {
        if self.file_path.as_os_str().is_empty() {
            return;
        }

        // Keep only one copy of each named file so alternate-file traversal
        // reflects recency rather than path duplication across buffer switches.
        self.recent_named_files
            .retain(|path| !paths_match(path, &self.file_path));
        self.recent_named_files.push_front(self.file_path.clone());
    }

    /// Jump to the file-like token under the cursor.
    pub(super) fn goto_file_under_cursor(&mut self) {
        self.goto_file_target(false);
    }

    /// Jump to the file-like token under the cursor and honor `:line[:column]`.
    pub(super) fn goto_file_under_cursor_at_position(&mut self) {
        self.goto_file_target(true);
    }

    /// Jump to the most recently active named buffer that is still open.
    pub(super) fn goto_alternate_file(&mut self) {
        let current_path =
            (!self.file_path.as_os_str().is_empty()).then_some(self.file_path.clone());
        let Some(target_path) = self.recent_named_files.iter().find(|path| {
            current_path
                .as_ref()
                .is_none_or(|current| !paths_match(current, path))
                && self.find_open_buffer_id_for_path(path).is_some()
        }) else {
            self.show_status_message("No alternate file");
            return;
        };

        let Some(target) = self.buffer_target_for_path(target_path) else {
            self.show_status_message("No alternate file");
            return;
        };
        self.goto_buffer_target(target);
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
        let Some(path) = resolve_file_target_path(
            (!self.file_path.as_os_str().is_empty()).then_some(self.file_path.as_path()),
            &target.path_text,
        ) else {
            self.show_status_message("No file target under cursor");
            return;
        };

        let target_line = target.line.unwrap_or(1).saturating_sub(1);
        let target_column = target.column.unwrap_or(1).saturating_sub(1);
        if !self.record_jump_origin_for_destination(&path, target_line, target_column) {
            return;
        }

        let open_path = current_dir_relative_path(&path);
        if let Err(error) = self.open_buffer(open_path.as_ref()) {
            self.show_status_message(format!(
                "Failed to open file target \"{}\": {error}",
                open_path.display()
            ));
            return;
        }

        // Clamp the parsed destination after opening so nonexistent files and
        // short lines still land at the nearest valid cursor position.
        let max_line = self.buffer.lines_count().saturating_sub(1);
        let line = target_line.min(max_line);
        let column = target_column.min(self.buffer.line_len(line).saturating_sub(1));
        self.cursor = Cursor::new(line, column);
        self.visual_anchor = None;
        self.mode = Mode::Normal;
        self.desired_visual_column = None;
        self.clear_pending_modal_state();
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.sync_visible_match_for_viewport();
        self.status_message = None;
    }

    /// Return one open-buffer target for the latest committed change, if any.
    fn last_modification_target(&self) -> Option<BufferTarget> {
        let active = self
            .last_committed_change_char_idx
            .map(|char_idx| BufferTarget {
                buffer_id: self.active_buffer_id,
                file_path: self.file_path.clone(),
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
                                file_path: buffer.file_path.clone(),
                                char_idx,
                                generation: buffer.last_edit_generation,
                            })
                    }),
            )
            .max_by_key(|target| target.generation)
    }

    /// Return the open-buffer target associated with `path`, if any.
    fn buffer_target_for_path(&self, path: &Path) -> Option<BufferTarget> {
        if paths_match(&self.file_path, path) {
            return Some(BufferTarget {
                buffer_id: self.active_buffer_id,
                file_path: self.file_path.clone(),
                char_idx: self.cursor.to_char_index(&self.buffer),
                generation: self.last_edit_generation,
            });
        }
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .find(|buffer| paths_match(&buffer.file_path, path))
            .map(|buffer| BufferTarget {
                buffer_id: buffer.id,
                file_path: buffer.file_path.clone(),
                char_idx: buffer.cursor.to_char_index(&buffer.buffer),
                generation: buffer.last_edit_generation,
            })
    }

    /// Return the open buffer id for `path` when that path is still open.
    fn find_open_buffer_id_for_path(&self, path: &Path) -> Option<usize> {
        self.buffer_target_for_path(path)
            .map(|target| target.buffer_id)
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
                self.show_status_message("Target buffer is no longer open");
                return;
            }
        }

        // Clamp after the buffer switch so the stored change location survives
        // file edits that shortened the target since the motion was recorded.
        let clamped = target.char_idx.min(self.buffer.chars_count());
        self.cursor = Cursor::from_char_index(&self.buffer, clamped);
        self.visual_anchor = None;
        self.mode = Mode::Normal;
        self.desired_visual_column = None;
        self.clear_pending_modal_state();
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.sync_visible_match_for_viewport();
        self.status_message = None;
    }
}
