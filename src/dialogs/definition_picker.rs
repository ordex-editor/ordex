//! Definition-target picker state built on the shared picker foundation.

use super::picker::{PickerItem, PickerPopup, PickerPopupEntry, PickerPopupSpec, PickerState};
use crate::lsp::DefinitionTarget;

/// One definition target listed by the picker dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionPickerItem {
    /// Target opened when this row is confirmed.
    pub(crate) target: DefinitionTarget,
    /// Stable display order used as a tie-breaker.
    pub(crate) order: usize,
}

/// Mutable state for the definition-target picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionPickerState {
    picker: PickerState<DefinitionPickerItem>,
}

impl DefinitionPickerState {
    const POPUP_SPEC: PickerPopupSpec = PickerPopupSpec {
        title: "Definitions",
        query_label: " Filter: ",
        empty_message: "No matching definitions",
    };

    /// Create picker state from the current ordered definition list.
    pub(crate) fn new(items: Vec<DefinitionPickerItem>) -> Self {
        Self {
            picker: PickerState::new(items),
        }
    }

    /// Borrow the shared picker state mutably.
    pub(crate) fn picker_mut(&mut self) -> &mut PickerState<DefinitionPickerItem> {
        &mut self.picker
    }

    /// Recompute matches for `query` while preserving the selected definition when possible.
    pub(crate) fn sync_query(&mut self, query: &str) {
        self.picker.sync_query(query);
    }

    /// Return the selected definition target, if the current filter still has matches.
    pub(crate) fn selected_target(&self) -> Option<DefinitionTarget> {
        self.picker.selected().map(|item| item.target.clone())
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

impl PickerItem for DefinitionPickerItem {
    type Key = String;

    fn key(&self) -> Self::Key {
        self.target.display_label.clone()
    }

    fn label(&self) -> &str {
        &self.target.display_label
    }

    fn order(&self) -> usize {
        self.order
    }

    fn popup_entry(&self, selected: bool) -> PickerPopupEntry {
        PickerPopupEntry {
            label: self.target.display_label.clone(),
            selected,
            active: false,
            modified: false,
        }
    }
}
