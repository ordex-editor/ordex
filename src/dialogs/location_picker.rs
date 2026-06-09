//! Navigation-target picker state built on the shared picker foundation.

use super::picker::{PickerItem, PickerPopup, PickerPopupEntry, PickerPopupSpec, PickerState};
use crate::lsp::{NavigationKind, NavigationTarget};

/// One navigation target listed by the picker dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocationPickerItem {
    /// Target opened when this row is confirmed.
    pub(crate) target: NavigationTarget,
    /// Stable display order used as a tie-breaker.
    pub(crate) order: usize,
}

/// Mutable state for the navigation-target picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocationPickerState {
    kind: NavigationKind,
    picker: PickerState<LocationPickerItem>,
}

impl LocationPickerState {
    /// Create picker state from the current ordered definition list.
    pub(crate) fn new(kind: NavigationKind, items: Vec<LocationPickerItem>) -> Self {
        Self {
            kind,
            picker: PickerState::new(items),
        }
    }

    /// Borrow the shared picker state mutably.
    pub(crate) fn picker_mut(&mut self) -> &mut PickerState<LocationPickerItem> {
        &mut self.picker
    }

    /// Recompute matches for `query` while preserving the selected target when possible.
    pub(crate) fn sync_query(&mut self, query: &str) {
        self.picker.sync_query(query);
    }

    /// Return the selected navigation target, if the current filter still has matches.
    pub(crate) fn selected_target(&self) -> Option<&NavigationTarget> {
        self.picker.selected().map(|item| &item.target)
    }

    /// Build the render-facing popup snapshot for the current query and selection.
    pub(crate) fn popup(
        &self,
        query: &str,
        cursor_column: usize,
        visible_entry_capacity: usize,
    ) -> PickerPopup {
        self.picker.popup(
            PickerPopupSpec {
                title: self.kind.picker_title(),
                query_label: " Filter: ",
                empty_message: self.kind.picker_empty_message(),
            },
            query,
            cursor_column,
            visible_entry_capacity,
        )
    }
}

impl PickerItem for LocationPickerItem {
    fn label(&self) -> &str {
        &self.target.display_label
    }

    fn order(&self) -> usize {
        self.order
    }

    fn popup_entry(&self, selected: bool) -> PickerPopupEntry {
        PickerPopupEntry {
            label: self.target.display_label.clone(),
            search_result_parts: None,
            selected,
            primary_marker: false,
            secondary_marker: false,
        }
    }
}
