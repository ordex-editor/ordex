//! Render-facing and syntax-view helpers for `EditorState`.

use super::*;
use crate::completion::CompletionPopup;
use crate::dialogs::{HoverPopup, PickerPopup, SignatureHelpPopup};
use crate::editor_state::buffers::display_file_name;
use crate::render::picker_popup_visible_entries;

impl EditorState {
    /// Borrow the current text buffer for render-side reads.
    pub(crate) fn buffer(&self) -> &TextBuffer {
        &self.buffer
    }

    /// Borrow the transient render buffer when substitute preview is active.
    pub(crate) fn render_buffer(&self) -> &TextBuffer {
        self.substitute_preview.as_ref().map_or(
            &self.buffer,
            substitute_preview::SubstitutePreviewState::buffer,
        )
    }

    /// Borrow the current text buffer mutably for crate-local test setup.
    #[cfg(test)]
    pub(crate) fn buffer_mut(&mut self) -> &mut TextBuffer {
        &mut self.buffer
    }

    /// Return whether relative line numbers are enabled for rendering.
    ///
    /// Returns `true` when the gutter should show relative distances away from
    /// the cursor line, and `false` when every line uses its absolute number.
    pub(crate) fn relative_line_numbers_enabled(&self) -> bool {
        self.settings.relative_line_numbers
    }

    /// Return whether soft wrapping is currently enabled.
    ///
    /// Returns `true` when long logical lines wrap across screen rows, and
    /// `false` when they stay on one row and use horizontal scrolling instead.
    pub(crate) fn soft_wrap_enabled(&self) -> bool {
        self.settings.soft_wrap
    }

    /// Return whether the sequence-discovery popup is currently enabled.
    ///
    /// Returns `true` when pending multi-key prefixes may open the popup, and
    /// `false` when that overlay is disabled in the current settings.
    pub(crate) fn sequence_discovery_popup_enabled(&self) -> bool {
        self.settings.sequence_discovery_popup
    }

    /// Set the terminal color capability used for themed rendering.
    pub(crate) fn set_color_capability(&mut self, capability: themes::ColorCapability) {
        self.settings.color_capability = capability;
    }

    /// Return the active bundled theme.
    pub(crate) fn theme(&self) -> &'static themes::Theme {
        themes::find(self.settings.theme_name).unwrap_or_else(themes::default_theme)
    }

    /// Return the active theme name.
    pub(crate) fn theme_name(&self) -> &'static str {
        self.settings.theme_name
    }

    /// Return the active terminal color capability.
    pub(crate) fn color_capability(&self) -> themes::ColorCapability {
        self.settings.color_capability
    }

    /// Return the cursor's current logical line index.
    pub(crate) fn cursor_line(&self) -> usize {
        self.cursor.line()
    }

    /// Replace the current cursor position without adjusting viewport state.
    #[cfg(test)]
    pub(crate) fn set_cursor(&mut self, cursor: Cursor) {
        self.cursor = cursor;
    }

    /// Return the cursor's current logical column index.
    pub(crate) fn cursor_column(&self) -> usize {
        self.cursor.column()
    }

    /// Return the first visible logical line in the viewport.
    pub(crate) fn first_visible_line(&self) -> usize {
        self.viewport.first_visible_line()
    }

    /// Return the first visible wrapped-row offset within the first visible line.
    pub(crate) fn first_visible_row(&self) -> usize {
        self.viewport.first_visible_row()
    }

    /// Return the first visible logical column for horizontal scrolling.
    pub(crate) fn first_visible_column(&self) -> usize {
        self.viewport.first_visible_column()
    }

    /// Return the visible file name for the status line and prompts.
    pub(crate) fn file_name(&self) -> &str {
        display_file_name(&self.file_path)
    }

    /// Return whether the current buffer has unsaved modifications.
    ///
    /// Returns `true` when the in-memory buffer differs from the last clean
    /// on-disk state, and `false` when the buffer is currently clean.
    pub(crate) fn is_modified(&self) -> bool {
        self.buffer.is_modified()
    }

    /// Return whether the active named file is read-only on disk.
    ///
    /// Returns `true` when the active path exists and its permissions are
    /// read-only, and `false` for writable files, unnamed buffers, or paths
    /// whose metadata is unavailable.
    pub(crate) fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Return ordered summaries for all open buffers for render-only UI surfaces.
    pub(crate) fn buffer_summaries(&self) -> Vec<BufferSummary> {
        self.buffer_manager.summaries(
            self.active_buffer_id,
            self.file_name(),
            &self.file_path,
            self.buffer.is_modified(),
        )
    }

    /// Return the total number of logical lines currently rendered on screen.
    pub(crate) fn render_buffer_line_count(&self) -> usize {
        self.render_buffer().lines_count()
    }

    /// Return the total number of characters currently rendered on screen.
    pub(crate) fn render_buffer_char_count(&self) -> usize {
        self.render_buffer().chars_count()
    }

    /// Return the transient status message scheduled for the next render, if any.
    pub(crate) fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }

    /// Return whether the terminal message row still needs one clearing redraw.
    ///
    /// Returns `true` when a previously rendered one-shot status message is still
    /// visible on the terminal, and `false` when the rendered message row already
    /// matches the current editor state.
    pub(crate) fn message_line_needs_clear(&self) -> bool {
        self.message_line_needs_clear
    }

    /// Return whether a stale multi-line status overlay still needs one full redraw.
    ///
    /// Returns `true` when the buffer area still contains a rendered multi-line
    /// status overlay, and `false` when the visible buffer already matches the
    /// current editor state.
    pub(crate) fn status_overlay_needs_clear(&self) -> bool {
        self.status_overlay_needs_clear
    }

    /// Return whether the next frame should force a full repaint.
    ///
    /// Returns `true` when a command explicitly requested a full redraw, and
    /// `false` when snapshot diffing may choose a smaller repaint.
    pub(crate) fn redraw_requested(&self) -> bool {
        self.redraw_requested
    }

    /// Return the current multi-line status overlay text, if any.
    pub(crate) fn status_overlay_message(&self) -> Option<&str> {
        self.status_message
            .as_deref()
            .filter(|message| message.contains('\n'))
    }

    /// Replace the transient status message shown on the message line.
    pub(crate) fn show_status_message<S: Into<String>>(&mut self, message: S) {
        self.status_message = Some(message.into());
    }

    /// Set or clear a synthetic swap prompt for render-focused tests.
    #[cfg(test)]
    pub(crate) fn set_swap_recovery_prompt_for_test(&mut self, active: bool) {
        self.pending_swap_recovery = active.then(|| PendingSwapPrompt {
            prompt: "swap prompt".to_string(),
            recovered_buffer: TextBuffer::new(),
            swap_path: PathBuf::from("/tmp/render-test.swp"),
            kind: PendingSwapPromptKind::Recovery,
            cancel_action: PendingSwapCancelAction::Quit,
            recreate_handle_on_discard: false,
        });
    }

    /// Clear any previously rendered status overlay before processing a new key.
    pub(crate) fn clear_status_overlay(&mut self) {
        self.status_overlay_needs_clear = false;
    }

    /// Record that a full render pass has completed and advance transient message state.
    pub(crate) fn finish_full_render(&mut self) {
        self.advance_status_render_state(true);
        self.redraw_requested = false;
    }

    /// Record that the current message row has been rendered and advance one-shot state.
    pub(crate) fn finish_message_render(&mut self) {
        self.advance_status_render_state(false);
    }

    /// Advance transient status state after one render pass.
    fn advance_status_render_state(&mut self, full_frame: bool) {
        self.message_line_needs_clear = false;
        if full_frame {
            self.status_overlay_needs_clear = false;
        }
        if let Some(message) = self.status_message.take() {
            // Single-line statuses remain message-row flashes, while multi-line
            // messages are cleared by the next full repaint of the buffer area.
            if message.contains('\n') {
                self.status_overlay_needs_clear = true;
            } else {
                self.message_line_needs_clear = true;
            }
        }
    }

    /// Borrow the bounded LSP progress lines currently visible in the overlay.
    pub(crate) fn lsp_progress_lines(&self) -> &[String] {
        &self.lsp_progress_lines
    }

    /// Replace the currently visible LSP progress lines.
    ///
    /// Returns `true` when the visible overlay lines changed and a redraw is
    /// needed, and `false` when the supplied lines match the current state.
    pub(crate) fn set_lsp_progress_lines(&mut self, lines: Vec<String>) -> bool {
        if self.lsp_progress_lines == lines {
            return false;
        }
        self.lsp_progress_lines = lines;
        true
    }

    /// Return the gutter number to show for one buffer line.
    ///
    /// When relative numbering is enabled, the cursor line stays absolute and all
    /// other buffer lines show their distance from the cursor.
    pub(crate) fn display_line_number(&self, line_idx: usize) -> usize {
        if !self.settings.relative_line_numbers || line_idx == self.cursor.line() {
            return line_idx + 1;
        }

        line_idx.abs_diff(self.cursor.line())
    }

    /// Re-detect the active language and rebuild syntax state for the current buffer.
    pub(crate) fn refresh_syntax(&mut self) {
        let path = self.active_named_file_path().map(PathBuf::from);
        self.syntax.open_document(path.as_deref(), &self.buffer);
        self.clear_match_state();
    }

    /// Return the current syntax-generation counter.
    pub(crate) fn syntax_generation(&self) -> u64 {
        self.syntax.generation()
    }

    /// Drop cached `%` pairs and any visible passive match state.
    pub(super) fn clear_match_state(&mut self) {
        self.matching.reset(self.syntax.generation());
    }

    /// Return the visible match role covering `char_idx`, if any.
    pub(crate) fn visible_match_role(&self, char_idx: usize) -> Option<VisibleMatchRole> {
        self.matching.visible_match_role(char_idx)
    }

    /// Return whether one visible passive match endpoint intersects `line_idx`.
    ///
    /// Returns `true` when the current visible `%`-match highlight touches that
    /// line, and `false` when the line has no visible passive match endpoint.
    pub(crate) fn line_has_visible_match(&self, line_idx: usize) -> bool {
        self.matching.line_has_visible_match(&self.buffer, line_idx)
    }

    /// Return a stable snapshot of the current visible passive match spans.
    pub(crate) fn visible_match_snapshot(&self) -> Option<(usize, usize, usize, usize)> {
        self.matching.visible_match_snapshot()
    }

    /// Return whether one visible search-result highlight intersects `line_idx`.
    ///
    /// Returns `true` when one cached visible search-result span overlaps that
    /// logical line, and `false` when the line has no cached visible result.
    pub(crate) fn line_has_visible_search_match(&self, line_idx: usize) -> bool {
        self.search_highlighting.line_has_visible_match(line_idx)
    }

    /// Return the visible search-result spans for `line_idx`.
    pub(crate) fn visible_search_match_spans(
        &self,
        line_idx: usize,
    ) -> &[search_highlighting::SearchHighlightSpan] {
        self.search_highlighting.line_spans(line_idx).unwrap_or(&[])
    }

    /// Return whether the active buffer cursor sits on one visible search-result span.
    ///
    /// Returns `true` when the cursor's current line/column lands inside one
    /// cached visible search-result span, and `false` otherwise.
    pub(crate) fn cursor_on_visible_search_match(&self) -> bool {
        self.visible_search_match_spans(self.cursor.line())
            .iter()
            .any(|span| span.covers(self.cursor.column()))
    }

    /// Return a stable snapshot of the current visible search-result spans.
    pub(crate) fn search_highlight_snapshot(&self) -> Vec<(usize, usize)> {
        self.search_highlighting.snapshot()
    }

    /// Return the redraw token for transient substitute preview content.
    pub(crate) fn substitute_preview_revision(&self) -> u64 {
        self.substitute_preview_revision
    }

    /// Return whether substitute preview is currently active.
    ///
    /// Returns `true` when render should source visible text from the preview
    /// buffer, and `false` when only the committed buffer should be shown.
    pub(crate) fn substitute_preview_active(&self) -> bool {
        self.substitute_preview.is_some()
    }

    /// Return whether `line_idx` should bypass committed-buffer syntax caches.
    ///
    /// Returns `true` when substitute preview may have changed the rendered line
    /// content, and `false` when committed-buffer rendering data still applies.
    pub(crate) fn substitute_preview_affects_line(&self, line_idx: usize) -> bool {
        self.substitute_preview
            .as_ref()
            .is_some_and(|preview| preview.affects_line(line_idx))
    }

    /// Prepare syntax spans for the current viewport and a small surrounding margin.
    pub(crate) fn prepare_syntax_view(&mut self, content_height: usize) {
        let first_line = self.viewport.first_visible_line();
        let last_line = first_line.saturating_add(content_height.saturating_sub(1));
        self.syntax
            .prepare_visible_lines(&self.buffer, first_line, last_line);
        self.refresh_visible_match(content_height);
        self.refresh_visible_search_matches(content_height);
    }

    /// Borrow the syntax spans for one logical line.
    pub(crate) fn syntax_spans_for_line(&self, line_index: usize) -> &[HighlightSpan] {
        self.syntax.spans_for_line(line_index)
    }

    /// Compute exact syntax spans for one logical line from the nearest checkpoint.
    #[cfg(test)]
    pub(crate) fn compute_syntax_spans_for_line(&self, line_index: usize) -> Vec<HighlightSpan> {
        self.syntax.compute_spans_for_line(&self.buffer, line_index)
    }

    /// Return the currently cached syntax spans for one logical line as an owned vector.
    #[cfg(test)]
    pub(crate) fn cached_syntax_spans_for_line(&self, line_index: usize) -> Vec<HighlightSpan> {
        self.syntax_spans_for_line(line_index).to_vec()
    }

    /// Update viewport dimensions after a terminal resize.
    pub(crate) fn handle_resize(&mut self, terminal_width: usize, terminal_height: usize) {
        self.viewport.set_width(terminal_width);
        self.viewport
            .set_height(terminal_height.saturating_sub(Self::RESERVED_SCREEN_ROWS));
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.buffer_manager.apply_shared_view_settings(
            self.viewport.height(),
            self.settings.scroll_margin,
            self.settings.horizontal_scroll_margin,
            self.settings.soft_wrap,
        );
    }

    /// Synchronize the viewport width used by rendering with the current gutter.
    pub(crate) fn sync_viewport_width_for_render(&mut self, content_width: usize) {
        let width_changed = self.viewport.width() != content_width;
        // Gutter-width changes alter the effective content width, which can change
        // wrapped rows or horizontal scrolling even when the cursor itself is stable.
        self.viewport.set_width(content_width);
        if width_changed {
            self.viewport
                .ensure_cursor_visible(&self.cursor, &self.buffer);
        }
    }

    /// Prepare visible syntax, then refresh passive match state for the viewport.
    pub(super) fn sync_visible_match_for_viewport(&mut self) {
        matching::sync_visible_match_for_viewport(self);
    }

    /// Recompute visible-only passive match spans from the current cursor position.
    pub(super) fn refresh_visible_match(&mut self, content_height: usize) {
        matching::refresh_visible_match(self, content_height);
    }

    /// Rebuild preview-driven search highlights for the current viewport.
    pub(super) fn sync_search_highlights_for_viewport(&mut self) {
        search_highlighting::sync_for_viewport(self);
    }

    /// Suppress committed search highlights until the next search action reveals them.
    pub(super) fn hide_search_highlighting(&mut self) {
        search_highlighting::hide_committed(self);
    }

    /// Recompute cached visible search-result spans for the current viewport.
    pub(super) fn refresh_visible_search_matches(&mut self, content_height: usize) {
        search_highlighting::refresh_visible_matches(self, content_height);
    }

    /// Jump from the current or next-on-line delimiter to its matching endpoint.
    pub(super) fn jump_to_matching_delimiter(&mut self) {
        matching::jump_to_matching_delimiter(self);
    }

    /// Get the current mode name for display
    pub(crate) fn mode_name(&self) -> &'static str {
        self.mode.mode_label()
    }

    /// Borrow the current editor mode for render-side comparisons.
    pub(crate) fn mode(&self) -> &Mode {
        &self.mode
    }

    /// Replace the current editor mode.
    #[cfg(test)]
    pub(crate) fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Return the process exit status requested by the active quit command.
    pub(crate) fn quit_exit_code(&self) -> i32 {
        self.quit_exit_code
    }

    /// Return whether the editor has requested that the app loop exit.
    ///
    /// Returns `true` when the app loop should stop and return the stored exit
    /// code, and `false` while interactive editing should continue.
    pub(crate) fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Cancel one pending quit request and keep the editor running.
    pub(crate) fn cancel_quit(&mut self) {
        self.should_quit = false;
        self.quit_exit_code = 0;
    }

    /// Return the terminal cursor shape for the active editor mode.
    pub(crate) fn cursor_shape(&self) -> tui::CursorShape {
        if self.mode.uses_beam_cursor() {
            return tui::CursorShape::Beam;
        }

        tui::CursorShape::Block
    }

    /// Get the command/search input string for display
    pub(crate) fn input_line(&self) -> Option<&str> {
        self.mode
            .command_string()
            .or_else(|| self.mode.search_string())
    }

    /// Get the prompt character for command/search mode
    pub(crate) fn input_prompt(&self) -> Option<char> {
        match &self.mode {
            Mode::Command(_) => Some(':'),
            Mode::Search(_) => Some('/'),
            _ => None,
        }
    }

    /// Return the 1-based cursor column for the active input prompt.
    pub(crate) fn input_cursor_column(&self) -> Option<usize> {
        self.mode.input_cursor().map(|cursor| cursor + 1)
    }

    /// Return the overwrite-confirmation prompt, when saving needs confirmation.
    pub(crate) fn overwrite_prompt(&self) -> Option<String> {
        self.pending_overwrite
            .as_ref()
            .map(|pending| format!("Overwrite \"{}\"? [y/N]", pending.target_path.display()))
    }

    /// Return the soft read-only save prompt, when saving needs confirmation.
    pub(crate) fn soft_read_only_save_prompt(&self) -> Option<String> {
        self.pending_soft_read_only_save.as_ref().map(|pending| {
            format!(
                "Write read-only file \"{}\" anyway? [y/N]",
                pending.target_path.display()
            )
        })
    }

    /// Return the swap-recovery prompt, when stale recovery data exists.
    pub(crate) fn swap_recovery_prompt(&self) -> Option<&str> {
        self.pending_swap_recovery
            .as_ref()
            .map(|pending| pending.prompt.as_str())
    }

    /// Return the quit-confirmation prompt, when quitting needs confirmation.
    pub(crate) fn quit_prompt(&self) -> Option<String> {
        if self.pending_quit_confirmation.is_none() {
            return None;
        }

        Some(format!(
            "Save changes to \"{}\"? [y]es/[n]o/[c]ancel",
            self.file_name()
        ))
    }

    /// Return the session-open confirmation prompt, when replacing dirty buffers.
    pub(crate) fn session_open_prompt(&self) -> Option<String> {
        let pending = self.pending_session_open_confirmation.as_ref()?;
        Some(format!(
            "Save changes to \"{}\" before opening session \"{}\"? [y]es/[n]o/[c]ancel",
            self.file_name(),
            pending.session_name
        ))
    }

    /// Return the close-confirmation prompt, when deleting a dirty buffer.
    pub(crate) fn buffer_close_prompt(&self) -> Option<String> {
        if !self.pending_buffer_close_confirmation {
            return None;
        }

        Some(format!(
            "Save changes to \"{}\" before closing? [y]es/[n]o/[c]ancel",
            self.file_name()
        ))
    }

    /// Get a short pending multi-key prefix label for UI display.
    pub(crate) fn pending_prefix_label(&self) -> Option<String> {
        if !self.mode_uses_modal_bindings() {
            return None;
        }

        if let Some(motion) = self.pending_find {
            let mut label = String::new();
            if motion.count > 1 {
                label.push_str(&motion.count.to_string());
            }
            let suffix = match (motion.kind, motion.direction) {
                (FindMotionKind::Find, FindDirection::Forward) => "f",
                (FindMotionKind::Find, FindDirection::Backward) => "F",
                (FindMotionKind::Till, FindDirection::Forward) => "t",
                (FindMotionKind::Till, FindDirection::Backward) => "T",
            };
            label.push_str(suffix);
            return Some(label);
        }

        if let Some(pending) = self.pending_operator.as_ref() {
            return Some(pending.prefix_label());
        }

        if let Some(pending) = self.pending_macro {
            return Some(pending.prefix_label());
        }

        if !self.pending_sequence.is_empty() {
            let mut label = String::new();
            if let Some(count) = self.pending_sequence_count {
                label.push_str(&count.to_string());
            }
            for key in &self.pending_sequence {
                label.push_str(&key.label());
            }
            if let Some(motion_count) = self.pending_sequence_motion_count {
                label.push_str(&motion_count.to_string());
            }
            return Some(label);
        }

        if let Some(count) = self.pending_count {
            return Some(count.to_string());
        }
        None
    }

    /// Return the active macro-recording label rendered in the message-row status slot.
    pub(crate) fn macro_recording_label(&self) -> Option<String> {
        self.macro_state
            .recording_register()
            .map(|register| format!("recording @{register}"))
    }

    /// Build the discovery-popup model for the current pending multi-key sequence.
    pub(crate) fn sequence_discovery_popup(&self) -> Option<SequenceDiscoveryPopup> {
        if !self.sequence_discovery_popup_enabled() || !self.mode_uses_modal_bindings() {
            return None;
        }

        if let Some(popup) = self.operator_discovery_popup() {
            return Some(popup);
        }

        if self.pending_sequence.is_empty() {
            return None;
        }

        let prefix = self.pending_prefix_label()?;
        let entries = self
            .keybindings
            .continuations_for_prefix(&self.mode, &self.pending_sequence)
            .into_iter()
            .map(|continuation| SequenceDiscoveryEntry {
                keys: continuation.keys_label(),
                action: continuation.action_label(),
            })
            .collect::<Vec<_>>();

        if entries.is_empty() {
            return None;
        }

        Some(SequenceDiscoveryPopup { prefix, entries })
    }

    /// Build the active picker popup model, if any overlay picker is open.
    pub(crate) fn picker_popup(&self) -> Option<PickerPopup> {
        // Compute the visible picker window once so every picker model stays aligned
        // with the current viewport height.
        let visible_entry_capacity = picker_popup_visible_entries(self.viewport.height());
        let picker = self.active_picker_kind()?;
        let query = self.mode.picker_string()?;
        let cursor_column = self.mode.input_cursor().unwrap_or(0);
        match picker {
            PickerKind::BufferSwitch => self
                .buffer_switch
                .as_ref()
                .map(|picker| picker.popup(query, cursor_column, visible_entry_capacity)),
            PickerKind::FilePicker => self
                .file_picker
                .as_ref()
                .map(|picker| picker.popup(query, cursor_column, visible_entry_capacity)),
            PickerKind::LocationPicker => self
                .location_picker
                .as_ref()
                .map(|picker| picker.popup(query, cursor_column, visible_entry_capacity)),
            PickerKind::DiagnosticPicker => self
                .diagnostic_picker
                .as_ref()
                .map(|picker| picker.popup(query, cursor_column, visible_entry_capacity)),
            PickerKind::CodeActionPicker => self
                .code_action_picker
                .as_ref()
                .map(|picker| picker.popup(query, cursor_column, visible_entry_capacity)),
        }
    }

    /// Build the active completion popup model, if insert-mode completion is visible.
    pub(crate) fn completion_popup(&self) -> Option<CompletionPopup> {
        self.completion_session
            .as_ref()
            .filter(|session| session.state == crate::completion::CompletionState::Active)
            .map(|session| session.popup())
    }

    /// Borrow the active hover popup model, if a hover response is visible.
    pub(crate) fn hover_popup(&self) -> Option<&HoverPopup> {
        self.hover_popup.as_ref()
    }

    /// Borrow the currently visible signature-help popup, if any.
    pub(crate) fn signature_help_popup(&self) -> Option<&SignatureHelpPopup> {
        self.signature_help_popup.as_ref()
    }
}
