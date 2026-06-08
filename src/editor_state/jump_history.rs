//! Jump-history helpers for meaningful location navigation in `EditorState`.

use super::*;
use std::collections::VecDeque;

/// One stored jump target inside the current editor session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct JumpLocation {
    /// Buffer id used when the destination still exists as an open buffer.
    buffer_id: usize,
    /// File path used to reopen named buffers when needed.
    file_path: PathBuf,
    /// Zero-based target line inside the destination buffer.
    line: usize,
    /// Zero-based target column inside the destination buffer.
    column: usize,
}

/// Session-wide stacks for moving backward and forward through jump targets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct JumpHistory {
    /// Older locations available through backward traversal.
    older: VecDeque<JumpLocation>,
    /// Newer locations available through forward traversal.
    newer: VecDeque<JumpLocation>,
}

impl JumpHistory {
    /// Maximum number of stored jump-history entries kept for one editor session.
    const MAX_OLDER_LEN: usize = 999_999;

    /// Create one empty jump-history state.
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Push one older jump unless it duplicates the current stack top.
    pub(super) fn push_older(&mut self, location: JumpLocation) {
        if self.older.back() == Some(&location) {
            return;
        }

        // Fresh jumps only grow the older stack, while backward/forward replay
        // merely redistributes the same entries between the two stacks. Capping
        // the accumulated older stack is therefore enough to bound total memory.
        if self.older.len() == Self::MAX_OLDER_LEN {
            self.older.pop_front();
        }
        self.older.push_back(location);
    }

    /// Push one newer jump unless it duplicates the current stack top.
    pub(super) fn push_newer(&mut self, location: JumpLocation) {
        if self.newer.back() != Some(&location) {
            self.newer.push_back(location);
        }
    }

    /// Drop every forward jump after a fresh non-history navigation.
    pub(super) fn clear_newer(&mut self) {
        self.newer.clear();
    }

    /// Remove and return the next older jump target, if any.
    pub(super) fn pop_older(&mut self) -> Option<JumpLocation> {
        self.older.pop_back()
    }

    /// Remove and return the next newer jump target, if any.
    pub(super) fn pop_newer(&mut self) -> Option<JumpLocation> {
        self.newer.pop_back()
    }
}

/// Direction used when traversing jump history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JumpHistoryDirection {
    Backward,
    Forward,
}

impl JumpHistoryDirection {
    /// Return the status line message shown when this traversal cannot move.
    fn empty_message(self) -> &'static str {
        match self {
            Self::Backward => "Already at oldest jump",
            Self::Forward => "Already at newest jump",
        }
    }
}

impl EditorState {
    /// Capture the current editor location as one jump-history entry.
    pub(super) fn current_jump_location(&self) -> JumpLocation {
        JumpLocation {
            buffer_id: self.active_buffer_id,
            file_path: self.file_path.clone(),
            line: self.cursor.line(),
            column: self.cursor.column(),
        }
    }

    /// Report whether the supplied destination is exactly the current location.
    ///
    /// Returns `true` when the file path, line, and column already match the
    /// current cursor location, and `false` when moving there would change the
    /// active editor position.
    pub(super) fn current_location_matches_destination(
        &self,
        file_path: &Path,
        line: usize,
        column: usize,
    ) -> bool {
        paths_match(&self.file_path, file_path)
            && self.cursor.line() == line
            && self.cursor.column() == column
    }

    /// Record the current location before a fresh jump to `file_path:line:column`.
    ///
    /// Returns `true` when the destination differs and the caller should perform
    /// the jump, and `false` when the destination already matches the current
    /// location so no history entry or cursor move is needed.
    pub(super) fn record_jump_origin_for_destination(
        &mut self,
        file_path: &Path,
        line: usize,
        column: usize,
    ) -> bool {
        if self.current_location_matches_destination(file_path, line, column) {
            return false;
        }
        self.jump_history.push_older(self.current_jump_location());
        self.jump_history.clear_newer();
        true
    }

    /// Move backward through up to `count` stored jump locations.
    pub(super) fn jump_backward_count(&mut self, count: usize) {
        self.step_jump_history(count, JumpHistoryDirection::Backward);
    }

    /// Move forward through up to `count` stored jump locations.
    pub(super) fn jump_forward_count(&mut self, count: usize) {
        self.step_jump_history(count, JumpHistoryDirection::Forward);
    }

    /// Move backward once through jump history.
    pub(super) fn jump_backward(&mut self) {
        self.jump_backward_count(1);
    }

    /// Move forward once through jump history.
    pub(super) fn jump_forward(&mut self) {
        self.jump_forward_count(1);
    }

    /// Move through jump history repeatedly until `count` moves succeed or stop.
    fn step_jump_history(&mut self, count: usize, direction: JumpHistoryDirection) {
        for _ in 0..count {
            if !self.move_through_jump_history(direction) {
                break;
            }
        }
    }

    /// Apply the next jump-history move in `direction`.
    ///
    /// Returns `true` when the cursor moved to a stored jump location, and
    /// `false` when the requested side of the jump history was empty or every
    /// remaining entry was unusable.
    fn move_through_jump_history(&mut self, direction: JumpHistoryDirection) -> bool {
        let current = self.current_jump_location();

        loop {
            let candidate = match direction {
                JumpHistoryDirection::Backward => self.jump_history.pop_older(),
                JumpHistoryDirection::Forward => self.jump_history.pop_newer(),
            };
            let Some(candidate) = candidate else {
                self.show_status_message(direction.empty_message());
                return false;
            };

            // Replayed stacks can contain duplicates or stale unnamed-buffer ids,
            // so skip unusable entries until one valid destination is found.
            if candidate == current || !self.apply_jump_location(&candidate) {
                continue;
            }

            match direction {
                JumpHistoryDirection::Backward => self.jump_history.push_newer(current),
                JumpHistoryDirection::Forward => self.jump_history.push_older(current),
            }
            self.clear_status_message();
            return true;
        }
    }

    /// Open and apply one stored jump location without re-recording history.
    ///
    /// Returns `true` when the destination buffer and cursor were restored, and
    /// `false` when the entry points at a missing unnamed buffer or a named file
    /// that can no longer be opened.
    fn apply_jump_location(&mut self, location: &JumpLocation) -> bool {
        // Prefer switching by buffer id so inactive named buffers keep unsaved
        // contents, then fall back to reopening the stored path when needed.
        if location.buffer_id != self.active_buffer_id {
            self.switch_to_buffer_id(location.buffer_id);
            if self.active_buffer_id != location.buffer_id {
                if location.file_path.as_os_str().is_empty() {
                    return false;
                }
                if self.open_buffer(&location.file_path).is_err() {
                    return false;
                }
            }
        }

        if !location.file_path.as_os_str().is_empty()
            && !paths_match(&self.file_path, &location.file_path)
        {
            return false;
        }

        // Clamp the stored location so history survives file edits that shrink
        // the destination line or line length after the jump was recorded.
        self.cursor = self.clamped_normal_cursor(location.line, location.column);
        self.finish_nonlocal_navigation();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Jump history should discard the oldest stored jump once the session cap is reached.
    fn test_jump_history_caps_older_entries() {
        let mut history = JumpHistory::new();

        for line in 0..=JumpHistory::MAX_OLDER_LEN {
            history.push_older(JumpLocation {
                buffer_id: 1,
                file_path: PathBuf::from("sample.rs"),
                line,
                column: 0,
            });
        }

        assert_eq!(history.older.len(), JumpHistory::MAX_OLDER_LEN);
        assert_eq!(history.older.front().map(|location| location.line), Some(1));
        assert_eq!(
            history.older.back().map(|location| location.line),
            Some(JumpHistory::MAX_OLDER_LEN)
        );
    }
}
