//! Transient `:s` preview lifecycle for `EditorState`.

use super::*;
use crate::substitute::{
    PreviewSubstituteCommand, SubstituteCommand, SubstitutePlan, build_substitute_plan,
    parse_preview_substitute_command,
};

/// One active substitute preview rendered from a transient cloned buffer.
#[derive(Debug, Clone)]
pub(super) struct SubstitutePreviewState {
    command: SubstituteCommand,
    plan: SubstitutePlan,
    buffer: TextBuffer,
    original_viewport: Viewport,
    first_affected_line: usize,
}

impl SubstitutePreviewState {
    /// Build one preview state from a committed-buffer snapshot and plan.
    pub(super) fn new(
        command: SubstituteCommand,
        plan: SubstitutePlan,
        mut buffer: TextBuffer,
        original_viewport: Viewport,
    ) -> Self {
        // Apply edits from the end so the plan's original coordinates stay valid
        // while the transient preview buffer is assembled.
        for edit in plan.edits().iter().rev() {
            buffer.remove(edit.start_char, edit.end_char);
            if !edit.replacement.is_empty() {
                buffer.insert(edit.start_char, &edit.replacement);
            }
        }
        let first_affected_line = plan
            .edits()
            .first()
            .map_or(0, |edit| buffer.char_to_line(edit.start_char));

        Self {
            command,
            plan,
            buffer,
            original_viewport,
            first_affected_line,
        }
    }

    /// Borrow the transient preview buffer used for rendering.
    pub(super) fn buffer(&self) -> &TextBuffer {
        &self.buffer
    }

    /// Return whether plain preview rendering should cover `line_idx`.
    ///
    /// Returns `true` when preview edits may have changed this line or later
    /// lines in the transient buffer, and `false` when the committed-buffer
    /// syntax and highlight caches still match the rendered content.
    pub(super) fn affects_line(&self, line_idx: usize) -> bool {
        line_idx >= self.first_affected_line
    }
}

impl EditorState {
    /// Refresh substitute preview state from the active command prompt.
    pub(super) fn refresh_substitute_preview(&mut self) {
        let Some(input) = self.mode.command_string() else {
            self.clear_substitute_preview(true);
            return;
        };
        match parse_preview_substitute_command(input) {
            PreviewSubstituteCommand::NotSubstitute | PreviewSubstituteCommand::Incomplete => {
                // Only restore the saved viewport while the user stays in the
                // command prompt; committed execution handles its own cleanup.
                self.clear_substitute_preview(true);
                self.status_message = None;
            }
            PreviewSubstituteCommand::Invalid(error) => {
                self.clear_substitute_preview(true);
                self.show_status_message(error);
            }
            PreviewSubstituteCommand::Ready(command) => {
                let plan = match build_substitute_plan(&command, &self.buffer, self.cursor.line()) {
                    Ok(plan) => plan,
                    Err(error) => {
                        self.clear_substitute_preview(true);
                        self.show_status_message(error);
                        return;
                    }
                };
                if plan.substitution_count() == 0 {
                    self.clear_substitute_preview(true);
                    self.status_message = None;
                    return;
                }

                self.store_substitute_preview(command, plan);
                self.status_message = None;
            }
        }
    }

    /// Clear the active substitute preview and optionally restore the old viewport.
    pub(super) fn clear_substitute_preview(&mut self, restore_viewport: bool) {
        let Some(preview) = self.substitute_preview.take() else {
            return;
        };
        if restore_viewport {
            self.viewport = preview.original_viewport;
        }
        self.substitute_preview_revision = self.substitute_preview_revision.saturating_add(1);
    }

    /// Consume one active substitute preview for Enter-driven command execution.
    pub(super) fn take_substitute_preview_for_commit(
        &mut self,
        command: &SubstituteCommand,
    ) -> Option<SubstitutePlan> {
        let preview = self.substitute_preview.take()?;
        self.substitute_preview_revision = self.substitute_preview_revision.saturating_add(1);
        (preview.command == *command).then_some(preview.plan)
    }

    /// Replace the active substitute preview with one newly planned state.
    fn store_substitute_preview(&mut self, command: SubstituteCommand, plan: SubstitutePlan) {
        let original_viewport = self
            .substitute_preview
            .as_ref()
            .map_or(self.viewport, |preview| preview.original_viewport);
        let preview =
            SubstitutePreviewState::new(command, plan, self.buffer.clone(), original_viewport);

        // The prompt keeps owning the beam cursor, so preview recenters the
        // viewport with a synthetic buffer cursor instead of moving editor state.
        if let Some(first_edit) = preview.plan.edits().first() {
            let preview_cursor = Cursor::from_char_index(preview.buffer(), first_edit.start_char);
            self.viewport
                .align_cursor_center(&preview_cursor, preview.buffer());
        }
        self.substitute_preview = Some(preview);
        self.substitute_preview_revision = self.substitute_preview_revision.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refreshing a valid substitute command should activate preview and recenter the viewport.
    #[test]
    fn test_refresh_substitute_preview_activates_preview() {
        let mut editor = EditorState::new(8);
        editor.viewport.set_soft_wrap(false);
        editor
            .buffer_mut()
            .insert(0, "zero\none\nfoo line\nthree\n");

        editor.enter_command_prompt("%s/foo/bar");

        assert!(editor.substitute_preview.is_some());
        assert_eq!(
            editor
                .substitute_preview
                .as_ref()
                .expect("preview should exist")
                .buffer()
                .to_string(),
            "zero\none\nbar line\nthree\n"
        );
    }

    /// Invalid substitute preview should clear the active preview and restore the viewport.
    #[test]
    fn test_refresh_substitute_preview_invalid_regex_restores_viewport() {
        let mut editor = EditorState::new(8);
        editor.viewport.set_soft_wrap(false);
        editor.buffer_mut().insert(0, "alpha\nbeta\n");
        editor.enter_command_prompt("%s/alpha/bravo");
        let previewed_line = editor.viewport.first_visible_line();

        editor.replace_active_prompt_text("%s/(?=beta)/x/".to_string());

        assert!(editor.substitute_preview.is_none());
        assert_eq!(editor.viewport.first_visible_line(), 0);
        assert!(previewed_line >= editor.viewport.first_visible_line());
        assert!(
            editor
                .status_message
                .as_deref()
                .is_some_and(|message| message.starts_with("Invalid regex:"))
        );
    }

    /// Canceling command input should drop substitute preview and restore the saved viewport.
    #[test]
    fn test_cancel_prompt_input_restores_viewport_after_preview() {
        let mut editor = EditorState::new(8);
        editor.viewport.set_soft_wrap(false);
        editor
            .buffer_mut()
            .insert(0, "zero\none\nfoo line\nthree\n");
        editor.enter_command_prompt("%s/foo/bar");
        let previewed_line = editor.viewport.first_visible_line();

        editor.cancel_prompt_input();

        assert!(editor.substitute_preview.is_none());
        assert!(editor.mode.is_normal());
        assert_eq!(editor.viewport.first_visible_line(), 0);
        assert!(previewed_line >= editor.viewport.first_visible_line());
    }

    /// Executing a previewed substitute should commit edits and keep the preview viewport.
    #[test]
    fn test_execute_command_commits_preview_without_restoring_viewport() {
        let mut editor = EditorState::new(8);
        editor.viewport.set_soft_wrap(false);
        editor
            .buffer_mut()
            .insert(0, "zero\none\nfoo line\nthree\n");
        editor.enter_command_prompt("%s/foo/bar");
        let previewed_line = editor.viewport.first_visible_line();

        editor.execute_command();

        assert!(editor.substitute_preview.is_none());
        assert_eq!(editor.buffer.to_string(), "zero\none\nbar line\nthree\n");
        assert_eq!(editor.viewport.first_visible_line(), previewed_line);
    }
}
