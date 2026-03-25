//! Render-facing and syntax-view helpers for `EditorState`.

use super::*;

impl EditorState {
    /// Return whether relative line numbers are enabled for rendering.
    pub(crate) fn relative_line_numbers_enabled(&self) -> bool {
        self.settings.relative_line_numbers
    }

    /// Return whether soft wrapping is currently enabled.
    pub(crate) fn soft_wrap_enabled(&self) -> bool {
        self.settings.soft_wrap
    }

    /// Return whether the sequence-discovery popup is currently enabled.
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
        let path = (!self.file_path.as_os_str().is_empty()).then_some(self.file_path.as_path());
        self.syntax.open_document(path, &self.buffer);
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
    pub(crate) fn line_has_visible_match(&self, line_idx: usize) -> bool {
        self.matching.line_has_visible_match(&self.buffer, line_idx)
    }

    /// Return a stable snapshot of the current visible passive match spans.
    pub(crate) fn visible_match_snapshot(&self) -> Option<(usize, usize, usize, usize)> {
        self.matching.visible_match_snapshot()
    }

    /// Prepare syntax spans for the current viewport and a small surrounding margin.
    pub(crate) fn prepare_syntax_view(&mut self, content_height: usize) {
        let first_line = self.viewport.first_visible_line();
        let last_line = first_line.saturating_add(content_height.saturating_sub(1));
        self.syntax
            .prepare_visible_lines(&self.buffer, first_line, last_line);
        self.refresh_visible_match(content_height);
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
            .set_height(terminal_height.saturating_sub(Self::RESERVED_BOTTOM_ROWS));
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Prepare visible syntax, then refresh passive match state for the viewport.
    pub(super) fn sync_visible_match_for_viewport(&mut self) {
        matching::sync_visible_match_for_viewport(self);
    }

    /// Recompute visible-only passive match spans from the current cursor position.
    pub(super) fn refresh_visible_match(&mut self, content_height: usize) {
        matching::refresh_visible_match(self, content_height);
    }

    /// Jump from the current or next-on-line delimiter to its matching endpoint.
    pub(super) fn jump_to_matching_delimiter(&mut self) {
        matching::jump_to_matching_delimiter(self);
    }

    /// Get the current mode name for display
    pub(crate) fn mode_name(&self) -> &str {
        self.mode.mode_label()
    }

    /// Return the process exit status requested by the active quit command.
    pub(crate) fn quit_exit_code(&self) -> i32 {
        self.quit_exit_code
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

    pub(crate) fn input_cursor_column(&self) -> Option<usize> {
        self.mode.input_cursor().map(|cursor| cursor + 1)
    }

    pub(crate) fn overwrite_prompt(&self) -> Option<String> {
        self.pending_overwrite
            .as_ref()
            .map(|pending| format!("Overwrite \"{}\"? [y/N]", pending.target_path.display()))
    }

    pub(crate) fn quit_prompt(&self) -> Option<String> {
        if self.pending_quit_confirmation.is_none() {
            return None;
        }

        let file_name = self
            .file_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("[No Name]");
        Some(format!(
            "Save changes to \"{}\"? [y]es/[n]o/[c]ancel",
            file_name
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

    /// Build the discovery-popup model for the current pending multi-key sequence.
    pub(crate) fn sequence_discovery_popup(&self) -> Option<SequenceDiscoveryPopup> {
        if !self.sequence_discovery_popup_enabled()
            || !self.mode_uses_modal_bindings()
            || self.pending_sequence.is_empty()
        {
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
}
