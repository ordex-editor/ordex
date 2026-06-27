//! Buffer storage helpers for multi-buffer editor sessions.

use super::*;
use crate::editor_state::matching::MatchingState;
use crate::lsp::protocol::LspTextChange;
use crate::path_utils::display_path_for_ui;
use crate::swap::SwapHandle;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Return the display name shown for one optional file path.
pub(super) fn display_file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("[No Name]")
}

/// Return the full picker label shown for one buffer path and stable id.
pub(super) fn display_buffer_path(path: &Path, buffer_id: usize) -> String {
    if path.as_os_str().is_empty() {
        return format!("[No Name] #{buffer_id}");
    }

    display_path_for_ui(path)
}

/// Normalize one path for buffer-identity comparisons.
pub(super) fn normalize_lookup_path(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        return None;
    }

    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    Some(joined.canonicalize().unwrap_or(joined))
}

/// Return whether two buffer paths refer to the same on-disk location.
///
/// Returns `true` when both paths resolve to the same file, and `false` when
/// they point somewhere else or one side cannot be normalized.
pub(super) fn paths_match(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (normalize_lookup_path(left), normalize_lookup_path(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// Return whether `path` currently resolves to a read-only file.
///
/// Returns `true` when the current process cannot open the existing file for
/// writing because the platform reports permission denied, and `false` for
/// unnamed paths, missing files, or all other outcomes.
pub(super) fn path_is_read_only(path: &Path) -> bool {
    if path.as_os_str().is_empty() {
        return false;
    }
    match OpenOptions::new().write(true).open(path) {
        Ok(_) => false,
        Err(error) => error.kind() == io::ErrorKind::PermissionDenied,
    }
}

/// One inactive buffer snapshot parked by the buffer manager.
#[derive(Debug)]
pub(super) struct BufferState {
    /// Stable identifier used while switching, listing, and prompting.
    pub(super) id: usize,
    /// Text content and dirty flag for this buffer.
    pub(super) buffer: TextBuffer,
    /// Cursor position local to this buffer.
    pub(super) cursor: Cursor,
    /// Scroll position local to this buffer.
    pub(super) viewport: Viewport,
    /// File path associated with this buffer, if any.
    pub(super) file_path: PathBuf,
    /// Whether the current on-disk file is reported as read-only.
    pub(super) read_only: bool,
    /// Whether the buffer was intentionally opened in soft read-only mode.
    pub(super) soft_read_only: bool,
    /// Last synced disk fingerprint plus any unresolved external-change state.
    pub(super) external_file: ExternalFileState,
    /// Syntax-highlighting cache for this buffer.
    pub(super) syntax: SyntaxEngine,
    /// Preferred wrapped-row column preserved across wrapped motions.
    pub(super) desired_visual_column: Option<usize>,
    /// `%`-matching cache and visible passive highlight state.
    pub(super) matching: MatchingState,
    /// Undoable changes committed in this buffer.
    pub(super) undo_stack: Vec<UndoTransaction>,
    /// Changes undone in this buffer that may still be replayed.
    pub(super) redo_stack: Vec<UndoTransaction>,
    /// In-progress undo transaction for this buffer.
    pub(super) active_undo: Option<ActiveUndoTransaction>,
    /// Undo-stack depth that matches the last clean on-disk state.
    pub(super) saved_undo_depth: usize,
    /// Suppress history capture while replaying existing edits.
    pub(super) replaying_history: bool,
    /// Swap file handle associated with this buffer, when recovery is active.
    pub(super) swap: Option<SwapHandle>,
    /// Deadline for the next debounced swap refresh after an edit.
    pub(super) pending_swap_refresh_at: Option<Instant>,
    /// Whether this buffer must not create or refresh a swap file right now.
    ///
    /// Inactive buffers carry this so switching away from a conflict-opened
    /// buffer does not accidentally resume swap ownership later.
    pub(super) suppress_swap_creation: bool,
    /// Pending swap prompt that should appear when this buffer becomes active.
    pub(super) pending_swap_recovery: Option<PendingSwapPrompt>,
    /// Whether swap state has been initialized for this buffer.
    ///
    /// When `false`, the next buffer activation must run
    /// `load_swap_state_for_active_buffer()` to establish swap ownership
    /// and surface any pending recovery prompts.
    pub(super) swap_loaded: bool,
    /// Monotonic document version sent to the language server for this buffer.
    pub(super) lsp_document_version: i32,
    /// Ordered edits queued for the next successful LSP sync of this buffer.
    pub(super) pending_lsp_changes: Vec<LspTextChange>,
    /// Deadline when the next proactive LSP sync may be dispatched for this buffer.
    pub(super) pending_lsp_sync_at: Option<Instant>,
    /// Most recent global edit generation applied to this buffer.
    pub(super) last_edit_generation: u64,
    /// Cursor position after the latest committed change in this buffer.
    pub(super) last_committed_change_char_idx: Option<usize>,
    /// Last active navigation lookup request for this buffer, if any.
    pub(super) active_navigation_lookup: Option<ActiveNavigationLookup>,
    /// Last active rename lookup request for this buffer, if any.
    pub(super) active_rename_lookup: Option<ActiveRenameLookup>,
    /// Last active code-action lookup request for this buffer, if any.
    pub(super) active_code_action_lookup: Option<ActiveCodeActionLookup>,
}

impl BufferState {
    /// Return the associated path when this buffer is named.
    pub(super) fn named_file_path(&self) -> Option<&Path> {
        (!self.file_path.as_os_str().is_empty()).then_some(self.file_path.as_path())
    }

    /// Create one empty unnamed buffer state with the requested viewport height.
    pub(super) fn new_empty(id: usize, terminal_height: usize) -> Self {
        let viewport =
            Viewport::new(terminal_height.saturating_sub(EditorState::RESERVED_SCREEN_ROWS));
        Self {
            id,
            buffer: TextBuffer::new(),
            cursor: Cursor::new(0, 0),
            viewport,
            file_path: PathBuf::new(),
            read_only: false,
            soft_read_only: false,
            external_file: ExternalFileState::default(),
            syntax: SyntaxEngine::new(),
            desired_visual_column: None,
            matching: MatchingState::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            active_undo: None,
            saved_undo_depth: 0,
            replaying_history: false,
            swap: None,
            pending_swap_refresh_at: None,
            suppress_swap_creation: false,
            pending_swap_recovery: None,
            swap_loaded: false,
            lsp_document_version: 0,
            pending_lsp_changes: Vec::new(),
            pending_lsp_sync_at: None,
            last_edit_generation: 0,
            last_committed_change_char_idx: None,
            active_navigation_lookup: None,
            active_rename_lookup: None,
            active_code_action_lookup: None,
        }
    }

    /// Create one named empty buffer so unsaved startup paths keep their filename.
    pub(super) fn new_named_empty(id: usize, terminal_height: usize, path: &Path) -> Self {
        let mut state = Self::new_empty(id, terminal_height);
        state.file_path = path.to_path_buf();
        state.read_only = path_is_read_only(path);
        state.soft_read_only = false;
        state.external_file.sync_to_missing_file();
        state.pending_lsp_sync_at = (!state.file_path.as_os_str().is_empty()).then(Instant::now);
        state.refresh_syntax();
        state
    }

    /// Load a buffer snapshot from disk and reset per-buffer history state.
    pub(super) fn from_file(
        id: usize,
        terminal_height: usize,
        path: &Path,
    ) -> std::io::Result<Self> {
        let mut state = Self::new_empty(id, terminal_height);
        state.buffer = Self::read_named_buffer_from_disk(path, &mut state.external_file)?;
        state.file_path = path.to_path_buf();
        state.read_only = path_is_read_only(path);
        state.soft_read_only = false;
        state.pending_lsp_sync_at = (!state.file_path.as_os_str().is_empty()).then(Instant::now);
        state.refresh_syntax();
        state
            .viewport
            .ensure_cursor_visible(&state.cursor, &state.buffer);
        Ok(state)
    }

    /// Reload this named buffer from disk while preserving its local cursor focus as much as possible.
    ///
    /// Returns `Ok(Some(message))` when the reload succeeded but follow-up work
    /// such as swap refresh still needs one warning, `Ok(None)` when the reload
    /// completed without any extra status message, and `Err(error)` when reading
    /// the backing file failed.
    pub(super) fn reload_from_disk(&mut self) -> io::Result<Option<String>> {
        let first_visible_line = self.viewport.first_visible_line();
        let previous_cursor = self.cursor.clone();
        let previous_buffer = self.buffer.clone();
        let reloaded_buffer =
            Self::read_named_buffer_from_disk(&self.file_path, &mut self.external_file)?;
        let reloaded_cursor = Self::clamped_buffer_cursor(
            &reloaded_buffer,
            previous_cursor.line(),
            previous_cursor.column(),
        );

        self.buffer = reloaded_buffer;
        self.read_only = path_is_read_only(&self.file_path);
        self.refresh_syntax();
        self.record_reload_history(&previous_buffer, previous_cursor, reloaded_cursor.clone());
        self.cursor = reloaded_cursor;
        self.viewport.set_first_visible_line(first_visible_line);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.lsp_document_version = 0;
        self.pending_lsp_changes.clear();
        self.pending_lsp_sync_at = Some(Instant::now());
        self.active_navigation_lookup = None;
        self.active_rename_lookup = None;
        self.active_code_action_lookup = None;

        // Hidden buffers still own swap handles, so auto-reloads must refresh the
        // parked recovery payload before the buffer becomes visible again.
        if let Some(swap) = self.swap.as_mut()
            && let Err(error) = swap.refresh(&self.buffer)
        {
            return Ok(Some(format!(
                "\"{}\" reloaded, but swap protection is unavailable: {error}",
                display_path_for_ui(&self.file_path)
            )));
        }
        Ok(None)
    }

    /// Rebuild syntax detection and clear visible match state for this buffer.
    pub(super) fn refresh_syntax(&mut self) {
        let path = self.named_file_path().map(PathBuf::from);
        self.syntax.open_document(path.as_deref(), &self.buffer);
        self.matching.reset(self.syntax.generation());
    }

    /// Read one named buffer from disk and refresh its external-file baseline.
    pub(super) fn read_named_buffer_from_disk(
        path: &Path,
        external_file: &mut ExternalFileState,
    ) -> io::Result<TextBuffer> {
        // Missing files reopen as named empty buffers so the buffer still points
        // at the same path after an external delete followed by a manual reload.
        match File::open(path) {
            Ok(file) => {
                let buffer = TextBuffer::from_reader(file)?;
                external_file.sync_to_loaded_buffer(&buffer);
                Ok(buffer)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                external_file.sync_to_missing_file();
                Ok(TextBuffer::new())
            }
            Err(error) => Err(error),
        }
    }

    /// Clamp one line and column to a valid cursor position in `buffer`.
    pub(super) fn clamped_buffer_cursor(buffer: &TextBuffer, line: usize, column: usize) -> Cursor {
        let line = line.min(buffer.lines_count().saturating_sub(1));
        let column = column.min(buffer.line_len(line).saturating_sub(1));
        Cursor::new(line, column)
    }

    /// Record one undoable history step that transforms `previous_buffer` into `reloaded_buffer`.
    fn record_reload_history(
        &mut self,
        previous_buffer: &TextBuffer,
        previous_cursor: Cursor,
        reloaded_cursor: Cursor,
    ) {
        let previous_text = previous_buffer.slice_string(0, previous_buffer.chars_count());
        let reloaded_text = self.buffer.slice_string(0, self.buffer.chars_count());
        let mut edits = Vec::new();

        // Reload behaves like one whole-buffer replace so undo restores the exact
        // pre-reload contents instead of dropping all earlier history.
        if !previous_text.is_empty() {
            edits.push(HistoryEdit::Remove {
                char_idx: 0,
                text: previous_text,
            });
        }
        if !reloaded_text.is_empty() {
            edits.push(HistoryEdit::Insert {
                char_idx: 0,
                text: reloaded_text,
            });
        }

        self.active_undo = None;
        self.replaying_history = false;
        if !edits.is_empty() {
            self.undo_stack.push(UndoTransaction {
                before_cursor_char_idx: previous_cursor.to_char_index(previous_buffer),
                after_cursor_char_idx: reloaded_cursor.to_char_index(&self.buffer),
                edits,
            });
            self.redo_stack.clear();
            self.last_committed_change_char_idx = Some(reloaded_cursor.to_char_index(&self.buffer));
        } else {
            self.last_committed_change_char_idx = None;
        }

        self.saved_undo_depth = self.undo_stack.len();
    }

    /// Return the display name used by buffer listings and prompts.
    pub(super) fn file_name(&self) -> &str {
        display_file_name(&self.file_path)
    }

    /// Return the full path label used by picker-style dialogs.
    pub(super) fn display_path(&self) -> String {
        display_buffer_path(&self.file_path, self.id)
    }
}

/// Small summary of one buffer for list and prompt surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BufferSummary {
    /// Stable identifier of the buffer.
    pub(crate) id: usize,
    /// Whether this buffer is the active one.
    pub(crate) active: bool,
    /// Whether this buffer has unsaved modifications.
    pub(crate) modified: bool,
    /// Display name for the buffer.
    pub(crate) file_name: String,
    /// Full path label for picker surfaces.
    pub(crate) display_path: String,
}

/// Ordered read-only snapshot of one open buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrderedBufferState {
    /// Stable identifier of the buffer.
    pub(crate) id: usize,
    /// Whether this buffer is the active one.
    pub(crate) active: bool,
    /// Path associated with this buffer.
    pub(crate) file_path: PathBuf,
    /// Cursor local to this buffer.
    pub(crate) cursor: Cursor,
}

/// Ordered collection of inactive buffers plus stable buffer ordering.
#[derive(Debug, Default)]
pub(super) struct BufferManager {
    /// Inactive buffers parked while another buffer is active in `EditorState`.
    inactive_buffers: Vec<BufferState>,
    /// Display and navigation order of all buffer ids, including the active one.
    order: Vec<usize>,
    /// Next stable buffer identifier.
    next_id: usize,
}

impl BufferManager {
    /// Create a buffer manager with one initial active buffer id.
    pub(super) fn new(active_id: usize) -> Self {
        Self {
            inactive_buffers: Vec::new(),
            order: vec![active_id],
            next_id: active_id + 1,
        }
    }

    /// Return the number of open buffers.
    pub(super) fn len(&self) -> usize {
        self.order.len()
    }

    /// Return whether exactly one buffer is open.
    ///
    /// Returns `true` when no other buffers are available for switching, and
    /// `false` when at least one additional buffer is open.
    pub(super) fn has_single_buffer(&self) -> bool {
        self.len() == 1
    }

    /// Create and reserve the next stable buffer identifier.
    pub(super) fn allocate_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Append a newly opened buffer id at the end of the navigation order.
    pub(super) fn push_new_id(&mut self, id: usize) {
        self.order.push(id);
    }

    /// Park one inactive buffer until it becomes active again.
    pub(super) fn store_inactive(&mut self, buffer: BufferState) {
        self.inactive_buffers.push(buffer);
    }

    /// Remove and return one inactive buffer by id.
    pub(super) fn take_inactive_by_id(&mut self, id: usize) -> Option<BufferState> {
        let index = self
            .inactive_buffers
            .iter()
            .position(|buffer| buffer.id == id)?;
        Some(self.inactive_buffers.swap_remove(index))
    }

    /// Remove and return one inactive buffer matching `path`, if any.
    pub(super) fn take_inactive_by_path(&mut self, path: &Path) -> Option<BufferState> {
        let index = self
            .inactive_buffers
            .iter()
            .position(|buffer| paths_match(&buffer.file_path, path))?;
        Some(self.inactive_buffers.swap_remove(index))
    }

    /// Return the id of the next buffer in order, wrapping at the end.
    pub(super) fn next_buffer_id(&self, active_id: usize) -> usize {
        let index = self
            .order
            .iter()
            .position(|&buffer_id| buffer_id == active_id)
            .expect("active buffer id should be present in order");
        self.order[(index + 1) % self.order.len()]
    }

    /// Return the id of the previous buffer in order, wrapping at the front.
    pub(super) fn prev_buffer_id(&self, active_id: usize) -> usize {
        let index = self
            .order
            .iter()
            .position(|&buffer_id| buffer_id == active_id)
            .expect("active buffer id should be present in order");
        self.order[if index == 0 {
            self.order.len() - 1
        } else {
            index - 1
        }]
    }

    /// Remove `active_id` from the buffer order and return the replacement active id.
    pub(super) fn remove_active_id(&mut self, active_id: usize) -> Option<usize> {
        let index = self
            .order
            .iter()
            .position(|&buffer_id| buffer_id == active_id)?;
        // Closing a buffer is an uncommon administrative action and the open-buffer
        // list is expected to stay small, so preserving order with `remove()` is
        // simpler and more valuable here than optimizing this rare O(n) path.
        self.order.remove(index);
        if self.order.is_empty() {
            return None;
        }

        let replacement_index = index.min(self.order.len() - 1);
        Some(self.order[replacement_index])
    }

    /// Return summaries in navigation order for every open buffer.
    pub(super) fn summaries(
        &self,
        active_id: usize,
        active_file_name: &str,
        active_file_path: &Path,
        active_modified: bool,
    ) -> Vec<BufferSummary> {
        self.order
            .iter()
            .map(|&buffer_id| {
                if buffer_id == active_id {
                    return BufferSummary {
                        id: buffer_id,
                        active: true,
                        modified: active_modified,
                        file_name: active_file_name.to_string(),
                        display_path: display_buffer_path(active_file_path, buffer_id),
                    };
                }

                let buffer = self
                    .inactive_buffers
                    .iter()
                    .find(|buffer| buffer.id == buffer_id)
                    .expect("inactive buffer id should resolve");
                BufferSummary {
                    id: buffer_id,
                    active: false,
                    modified: buffer.buffer.is_modified(),
                    file_name: buffer.file_name().to_string(),
                    display_path: buffer.display_path(),
                }
            })
            .collect()
    }

    /// Return ordered path and cursor snapshots for every open buffer.
    pub(super) fn ordered_states(
        &self,
        active_id: usize,
        active_file_path: &Path,
        active_cursor: &Cursor,
    ) -> Vec<OrderedBufferState> {
        self.order
            .iter()
            .map(|&buffer_id| {
                if buffer_id == active_id {
                    return OrderedBufferState {
                        id: buffer_id,
                        active: true,
                        file_path: active_file_path.to_path_buf(),
                        cursor: active_cursor.clone(),
                    };
                }

                let buffer = self
                    .inactive_buffers
                    .iter()
                    .find(|buffer| buffer.id == buffer_id)
                    .expect("inactive buffer id should resolve");
                OrderedBufferState {
                    id: buffer_id,
                    active: false,
                    file_path: buffer.file_path.clone(),
                    cursor: buffer.cursor.clone(),
                }
            })
            .collect()
    }

    /// Return every dirty buffer id in navigation order except the active buffer.
    pub(super) fn inactive_dirty_ids(&self) -> Vec<usize> {
        self.order
            .iter()
            .filter(|&&buffer_id| {
                self.inactive_buffers
                    .iter()
                    .find(|buffer| buffer.id == buffer_id)
                    .is_some_and(|buffer| buffer.buffer.is_modified())
            })
            .copied()
            .collect()
    }

    /// Apply shared viewport settings that must stay consistent across parked buffers.
    pub(super) fn apply_shared_view_settings(
        &mut self,
        viewport_height: usize,
        scroll_margin: usize,
        horizontal_scroll_margin: usize,
        soft_wrap: bool,
    ) {
        for buffer in &mut self.inactive_buffers {
            // Buffer-local cursor and scroll offsets stay intact, while terminal-
            // or config-derived viewport settings stay synchronized globally.
            buffer.viewport.set_height(viewport_height);
            buffer.viewport.set_scroll_margin(scroll_margin);
            buffer
                .viewport
                .set_horizontal_scroll_margin(horizontal_scroll_margin);
            buffer.viewport.set_soft_wrap(soft_wrap);
        }
    }

    /// Return shared access to every inactive buffer snapshot.
    pub(super) fn inactive_buffers(&self) -> &[BufferState] {
        &self.inactive_buffers
    }

    /// Return mutable access to every inactive buffer snapshot.
    pub(super) fn inactive_buffers_mut(&mut self) -> &mut [BufferState] {
        &mut self.inactive_buffers
    }
}

#[cfg(test)]
mod readonly_tests {
    use super::*;
    use test_utils::TempFile;

    /// Verify that writable temp files do not report as read-only.
    #[test]
    fn test_path_is_read_only_false_for_writable_temp_file() {
        let file = TempFile::new().expect("create temp file");
        std::fs::write(file.path(), "status\n").expect("seed temp file");

        assert!(!path_is_read_only(file.path()));
    }

    /// Verify that non-writable temp files report as read-only.
    #[test]
    fn test_path_is_read_only_true_for_nonwritable_temp_file() {
        let file = TempFile::new().expect("create temp file");
        std::fs::write(file.path(), "status\n").expect("seed temp file");
        let mut permissions = std::fs::metadata(file.path())
            .expect("stat temp file")
            .permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(file.path(), permissions).expect("mark temp file non-writable");

        assert!(path_is_read_only(file.path()));
    }

    /// Verify that root-owned config-style files report as read-only for non-root users.
    #[test]
    fn test_path_is_read_only_true_for_user_unwritable_system_file() {
        let Some(system_file) = ["/etc/pacman.conf", "/etc/passwd"]
            .into_iter()
            .map(Path::new)
            .find(|path| {
                path.exists()
                    && File::open(path).is_ok()
                    && OpenOptions::new().write(true).open(path).is_err()
            })
        else {
            return;
        };
        assert!(path_is_read_only(system_file));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::{EnvVarGuard, TempTree, lock_process_environment};

    #[test]
    /// Verify unnamed buffers show their stable synthetic label.
    fn test_display_buffer_path_uses_actual_buffer_id_for_unnamed_buffers() {
        assert_eq!(display_buffer_path(Path::new(""), 7), "[No Name] #7");
    }

    #[test]
    /// Verify named buffers under the home directory use a compact display label.
    fn test_display_buffer_path_compacts_home_relative_named_buffer() {
        let lock = lock_process_environment();
        let tree = TempTree::new().expect("create temp tree");
        let home = tree.path().join("home");
        std::fs::create_dir_all(home.join("project")).expect("create home project");
        let _home_guard = EnvVarGuard::set(&lock, "HOME", home.clone().into_os_string());

        assert_eq!(
            display_buffer_path(&home.join("project/main.rs"), 3),
            "~/project/main.rs"
        );
    }
}
