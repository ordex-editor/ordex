//! Buffer-switch picker state built on the shared picker foundation.

use super::picker::{PickerItem, PickerPopup, PickerPopupEntry, PickerPopupSpec, PickerState};

/// One buffer listed by the switcher dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BufferSwitchItem {
    /// Stable buffer identifier used when confirming a selection.
    pub(crate) buffer_id: usize,
    /// Full display label shown in the picker.
    pub(crate) label: String,
    /// Whether this item represents the active buffer.
    pub(crate) active: bool,
    /// Whether this buffer has unsaved changes.
    pub(crate) modified: bool,
    /// Stable open-buffer order used as a tiebreaker during sorting.
    pub(crate) order: usize,
}

/// Mutable state for the buffer-switch picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BufferSwitchState {
    picker: PickerState<BufferSwitchItem>,
}

impl BufferSwitchState {
    const POPUP_SPEC: PickerPopupSpec = PickerPopupSpec {
        title: "Buffers",
        query_label: " Filter: ",
        empty_message: "No matching buffers",
    };

    /// Create picker state from the current ordered buffer list.
    pub(crate) fn new(items: Vec<BufferSwitchItem>) -> Self {
        Self {
            picker: PickerState::new(items),
        }
    }

    /// Borrow the shared picker state mutably.
    pub(crate) fn picker_mut(&mut self) -> &mut PickerState<BufferSwitchItem> {
        &mut self.picker
    }

    /// Recompute matches for `query` while preserving the selected buffer when possible.
    pub(crate) fn sync_query(&mut self, query: &str) {
        self.picker.sync_query(query);
    }

    /// Return the selected buffer id, if the current filter still has matches.
    pub(crate) fn selected_buffer_id(&self) -> Option<usize> {
        self.picker.selected().map(|item| item.buffer_id)
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

impl PickerItem for BufferSwitchItem {
    fn label(&self) -> &str {
        &self.label
    }

    fn order(&self) -> usize {
        self.order
    }

    fn is_selectable(&self) -> bool {
        !self.active
    }

    fn is_pinned(&self) -> bool {
        self.active
    }

    fn popup_entry(&self, selected: bool) -> PickerPopupEntry {
        PickerPopupEntry {
            label: self.label.clone(),
            search_result_parts: None,
            selected: selected && !self.active,
            primary_marker: self.active,
            secondary_marker: self.modified,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one test picker item with the requested label and order.
    fn item(buffer_id: usize, order: usize, label: &str) -> BufferSwitchItem {
        BufferSwitchItem {
            buffer_id,
            label: label.to_string(),
            active: false,
            modified: false,
            order,
        }
    }

    /// Build one active test picker item with the requested label and order.
    fn active_item(buffer_id: usize, order: usize, label: &str) -> BufferSwitchItem {
        BufferSwitchItem {
            buffer_id,
            label: label.to_string(),
            active: true,
            modified: false,
            order,
        }
    }

    #[test]
    fn test_picker_prefers_tighter_fuzzy_matches() {
        let mut picker = BufferSwitchState::new(vec![
            item(1, 0, "/tmp/src_buffer.rs"),
            item(2, 1, "/tmp/scratch/base.rs"),
        ]);

        picker.sync_query("sbr");

        let popup = picker.popup("sbr", 3, 10);
        assert_eq!(popup.entries.len(), 2);
        assert_eq!(popup.entries[0].label, "/tmp/src_buffer.rs");
    }

    #[test]
    fn test_picker_preserves_selected_buffer_across_query_updates() {
        let mut picker = BufferSwitchState::new(vec![
            item(1, 0, "/tmp/alpha.rs"),
            item(2, 1, "/tmp/beta.rs"),
            item(3, 2, "/tmp/beta_test.rs"),
        ]);

        picker.picker_mut().move_down();
        picker.sync_query("beta");

        assert_eq!(picker.selected_buffer_id(), Some(2));
    }

    #[test]
    fn test_picker_shows_active_buffer_but_selects_first_inactive_entry() {
        let picker = BufferSwitchState::new(vec![
            active_item(1, 0, "/tmp/current.rs"),
            item(2, 1, "/tmp/other.rs"),
        ]);

        let popup = picker.popup("", 0, 10);

        assert_eq!(popup.entries[0].label, "/tmp/current.rs");
        assert!(!popup.entries[0].selected);
        assert!(popup.entries[1].selected);
        assert_eq!(picker.selected_buffer_id(), Some(2));
    }

    #[test]
    fn test_picker_keeps_active_buffer_as_first_visible_entry_during_filtering() {
        let mut picker = BufferSwitchState::new(vec![
            active_item(1, 0, "/tmp/current.rs"),
            item(2, 1, "/tmp/alpha.rs"),
            item(3, 2, "/tmp/beta.rs"),
        ]);

        picker.sync_query("beta");

        let popup = picker.popup("beta", 4, 10);
        assert_eq!(popup.entries[0].label, "/tmp/current.rs");
        assert_eq!(popup.entries[1].label, "/tmp/beta.rs");
        assert!(!popup.entries[0].selected);
        assert!(popup.entries[1].selected);
    }

    #[test]
    fn test_picker_handles_empty_match_set() {
        let mut picker = BufferSwitchState::new(vec![item(1, 0, "/tmp/alpha.rs")]);

        picker.sync_query("zzz");

        assert_eq!(picker.selected_buffer_id(), None);
        assert!(picker.popup("zzz", 3, 10).entries.is_empty());
    }

    #[test]
    fn test_picker_prefers_contiguous_cpp_match_over_scattered_match() {
        let mut picker = BufferSwitchState::new(vec![
            item(1, 0, "src/app.rs"),
            item(2, 1, "src/syntax/profiles/cpp.rs"),
        ]);

        picker.sync_query("cpp");

        let popup = picker.popup("cpp", 3, 10);
        assert_eq!(popup.entries[0].label, "src/syntax/profiles/cpp.rs");
        assert_eq!(popup.entries[1].label, "src/app.rs");
    }
}
