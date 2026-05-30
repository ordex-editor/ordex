//! Code-action picker state built on the shared picker foundation.

use super::picker::{PickerItem, PickerPopup, PickerPopupEntry, PickerPopupSpec, PickerState};
use crate::lsp::protocol::LspCodeAction;

/// One code action listed by the picker dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeActionPickerItem {
    /// Action applied when this row is confirmed.
    pub(crate) action: LspCodeAction,
    /// Stable server-order tie-breaker for repeated lookups.
    pub(crate) order: usize,
}

/// Mutable state for the code-action picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeActionPickerState {
    source_buffer_id: usize,
    request_edit_generation: u64,
    picker: PickerState<CodeActionPickerItem>,
}

impl CodeActionPickerState {
    const POPUP_SPEC: PickerPopupSpec = PickerPopupSpec {
        title: "Code Actions",
        query_label: " Filter: ",
        empty_message: "No matching code actions",
    };

    /// Create picker state from the current ordered code-action list.
    pub(crate) fn new(
        source_buffer_id: usize,
        request_edit_generation: u64,
        items: Vec<CodeActionPickerItem>,
    ) -> Self {
        Self {
            source_buffer_id,
            request_edit_generation,
            picker: PickerState::new(items),
        }
    }

    /// Borrow the shared picker state mutably.
    pub(crate) fn picker_mut(&mut self) -> &mut PickerState<CodeActionPickerItem> {
        &mut self.picker
    }

    /// Recompute matches for `query` while preserving the selected action when possible.
    pub(crate) fn sync_query(&mut self, query: &str) {
        self.picker.sync_query(query);
    }

    /// Return the selected code action, if the current filter still has matches.
    pub(crate) fn selected_action(&self) -> Option<&LspCodeAction> {
        self.picker.selected().map(|item| &item.action)
    }

    /// Return the source buffer id that requested this picker.
    pub(crate) fn source_buffer_id(&self) -> usize {
        self.source_buffer_id
    }

    /// Return the edit generation captured when the lookup started.
    pub(crate) fn request_edit_generation(&self) -> u64 {
        self.request_edit_generation
    }

    /// Build the render-facing popup snapshot for the current query and selection.
    pub(crate) fn popup(
        &self,
        query: &str,
        cursor_column: usize,
        visible_entry_capacity: usize,
    ) -> PickerPopup {
        self.picker.popup(
            Self::POPUP_SPEC,
            query,
            cursor_column,
            visible_entry_capacity,
        )
    }
}

impl PickerItem for CodeActionPickerItem {
    type Key = usize;

    fn key(&self) -> Self::Key {
        self.order
    }

    fn label(&self) -> &str {
        &self.action.title
    }

    fn order(&self) -> usize {
        self.order
    }

    fn popup_entry(&self, selected: bool) -> PickerPopupEntry {
        PickerPopupEntry {
            label: self.action.title.clone(),
            search_result_parts: None,
            selected,
            primary_marker: false,
            secondary_marker: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::protocol::LspWorkspaceEdit;

    /// Build one test code action item with the requested title and order.
    fn item(title: &str, order: usize) -> CodeActionPickerItem {
        CodeActionPickerItem {
            action: LspCodeAction {
                title: title.to_string(),
                edit: LspWorkspaceEdit {
                    document_edits: Vec::new(),
                },
            },
            order,
        }
    }

    #[test]
    /// The picker should keep the selected action when the narrowed query still matches it.
    fn test_picker_preserves_selected_action_across_query_updates() {
        let mut picker = CodeActionPickerState::new(
            7,
            12,
            vec![
                item("Apply alpha fix", 0),
                item("Apply beta fix", 1),
                item("Apply beta cleanup", 2),
            ],
        );

        picker.picker_mut().move_down();
        picker.sync_query("beta");

        assert_eq!(
            picker.selected_action().map(|action| action.title.as_str()),
            Some("Apply beta fix")
        );
        assert_eq!(picker.source_buffer_id(), 7);
        assert_eq!(picker.request_edit_generation(), 12);
    }
}
