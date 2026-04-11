//! Active-buffer diagnostics picker state built on the shared picker foundation.

use super::picker::{PickerItem, PickerPopup, PickerPopupEntry, PickerPopupSpec, PickerState};
use crate::lsp::{LspDiagnostic, LspDiagnosticSeverity};

/// One active-buffer diagnostic listed by the picker dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiagnosticPickerItem {
    /// Stable zero-based index into the active-buffer diagnostics list.
    pub(crate) diagnostic_index: usize,
    /// Cached diagnostic payload used for display and navigation.
    pub(crate) diagnostic: LspDiagnostic,
}

/// Mutable state for the active-buffer diagnostics picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiagnosticPickerState {
    picker: PickerState<DiagnosticPickerItem>,
}

impl DiagnosticPickerState {
    /// Create picker state from the current ordered diagnostics list.
    pub(crate) fn new(items: Vec<DiagnosticPickerItem>) -> Self {
        Self {
            picker: PickerState::new(items),
        }
    }

    /// Borrow the shared picker state mutably.
    pub(crate) fn picker_mut(&mut self) -> &mut PickerState<DiagnosticPickerItem> {
        &mut self.picker
    }

    /// Recompute matches for `query` while preserving the selected diagnostic when possible.
    pub(crate) fn sync_query(&mut self, query: &str) {
        self.picker.sync_query(query);
    }

    /// Return the selected active-buffer diagnostic index, if any.
    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.picker.selected().map(|item| item.diagnostic_index)
    }

    /// Build the render-facing popup snapshot for the current query and selection.
    pub(crate) fn popup(
        &self,
        query: &str,
        cursor_column: usize,
        visible_entry_capacity: usize,
    ) -> PickerPopup {
        let mut popup = self.picker.popup(
            PickerPopupSpec {
                title: "Diagnostics",
                query_label: " Filter: ",
                empty_message: "No matching diagnostics",
            },
            query,
            cursor_column,
            visible_entry_capacity,
        );
        popup.query_suffix = format!("{} ", self.picker.item_count());
        popup
    }
}

impl PickerItem for DiagnosticPickerItem {
    type Key = usize;

    fn key(&self) -> Self::Key {
        self.diagnostic_index
    }

    fn label(&self) -> &str {
        &self.diagnostic.message
    }

    fn order(&self) -> usize {
        self.diagnostic_index
    }

    fn popup_entry(&self, selected: bool) -> PickerPopupEntry {
        PickerPopupEntry {
            label: self.diagnostic.display_label(),
            selected,
            active: matches!(self.diagnostic.severity, LspDiagnosticSeverity::Error),
            modified: matches!(self.diagnostic.severity, LspDiagnosticSeverity::Warning),
        }
    }
}
