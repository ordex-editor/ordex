//! Shared picker state, popup models, and fuzzy matching helpers.

/// One rendered picker row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerPopupEntry {
    /// Full display label shown in the picker row.
    pub(crate) label: String,
    /// Whether this row is currently selected.
    pub(crate) selected: bool,
    /// Whether this row represents the active item.
    pub(crate) active: bool,
    /// Whether this row represents a modified item.
    pub(crate) modified: bool,
}

/// Render-facing snapshot for one picker popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerPopup {
    /// Popup title shown in the top border.
    pub(crate) title: String,
    /// Label shown before the editable query text.
    pub(crate) query_label: String,
    /// Message shown when no rows match the current query.
    pub(crate) empty_message: String,
    /// Current filter query.
    pub(crate) query: String,
    /// Zero-based cursor column inside the query.
    pub(crate) cursor_column: usize,
    /// Filtered picker rows in display order.
    pub(crate) entries: Vec<PickerPopupEntry>,
}

/// Static strings that define one picker popup presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PickerPopupSpec {
    /// Popup title shown in the top border.
    pub(crate) title: &'static str,
    /// Label shown before the editable query text.
    pub(crate) query_label: &'static str,
    /// Message shown when no rows match the current query.
    pub(crate) empty_message: &'static str,
}

/// One sortable fuzzy-match score where lower values represent a better match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct MatchScore {
    boundary_rank: usize,
    gap_count: usize,
    start_index: usize,
    span_len: usize,
    candidate_len: usize,
}

/// Data required by the shared picker selection and filtering state.
pub(crate) trait PickerItem {
    /// Stable key used to preserve selection across query and item updates.
    type Key: Clone + Eq;

    /// Return the stable key for this item.
    fn key(&self) -> Self::Key;

    /// Return the main display label for this item.
    fn label(&self) -> &str;

    /// Return the stable order used as a tie-breaker for equal matches.
    fn order(&self) -> usize;

    /// Return whether the item can be confirmed.
    fn is_selectable(&self) -> bool {
        true
    }

    /// Return whether the item stays visible at the top of the popup.
    fn is_pinned(&self) -> bool {
        false
    }

    /// Return the fuzzy-match score for the current query, if any.
    fn match_score(&self, query: &str) -> Option<MatchScore> {
        fuzzy_match_score(self.label(), query)
    }

    /// Build the render-facing row for this item.
    fn popup_entry(&self, selected: bool) -> PickerPopupEntry;
}

/// Mutable state for one picker backed by typed item data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerState<T> {
    items: Vec<T>,
    filtered_indices: Vec<usize>,
    selected_index: usize,
}

impl<T: PickerItem> PickerState<T> {
    /// Create picker state from the current ordered item list.
    pub(crate) fn new(items: Vec<T>) -> Self {
        let mut picker = Self {
            items,
            filtered_indices: Vec::new(),
            selected_index: 0,
        };
        picker.sync_query("");
        picker
    }

    /// Append new items and refresh matches for the active query.
    pub(crate) fn extend_items<I>(&mut self, items: I, query: &str)
    where
        I: IntoIterator<Item = T>,
    {
        self.items.extend(items);
        self.sync_query(query);
    }

    /// Recompute matches for `query` while preserving the selected item when possible.
    pub(crate) fn sync_query(&mut self, query: &str) {
        let selected_key = self.selected_key();
        let mut pinned = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_pinned())
            .collect::<Vec<_>>();
        let mut matches = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| !item.is_pinned())
            .filter_map(|(index, item)| item.match_score(query).map(|score| (index, score)))
            .collect::<Vec<_>>();

        // Pinned rows stay in their stable order at the top of the popup, while
        // fuzzy-matched rows sort by match quality and then by their source order.
        pinned.sort_by_key(|(index, item)| (item.order(), *index));
        matches.sort_by_key(|(index, score)| (*score, self.items[*index].order(), *index));

        self.filtered_indices = pinned.into_iter().map(|(index, _)| index).collect();
        self.filtered_indices
            .extend(matches.into_iter().map(|(index, _)| index));

        if self.filtered_indices.is_empty() {
            self.selected_index = 0;
            return;
        }

        // Selection stays on the same logical item whenever that item remains in
        // the filtered list after the query or item set changes.
        if let Some(selected_key) = selected_key
            && let Some(position) = self.filtered_indices.iter().position(|&index| {
                self.items
                    .get(index)
                    .is_some_and(|item| item.key() == selected_key)
            })
        {
            self.selected_index = position;
            return;
        }

        self.selected_index = self.first_selectable_position().unwrap_or(0);
    }

    /// Move the picker selection one row up, stopping at the first row.
    pub(crate) fn move_up(&mut self) {
        // Disabled rows can remain visible for context, but keyboard navigation
        // should stop only on entries that can actually be confirmed.
        if let Some(position) = (0..self.selected_index)
            .rev()
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
        }
    }

    /// Move the picker selection one row down, stopping at the last row.
    pub(crate) fn move_down(&mut self) {
        if let Some(position) = ((self.selected_index + 1)..self.filtered_indices.len())
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
        }
    }

    /// Move the picker selection one page up, stopping at the first row.
    pub(crate) fn move_page_up(&mut self, page_len: usize) {
        let target = self.selected_index.saturating_sub(page_len.max(1));
        if let Some(position) =
            (target..self.selected_index).find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
        }
    }

    /// Move the picker selection one page down, stopping at the last row.
    pub(crate) fn move_page_down(&mut self, page_len: usize) {
        if self.filtered_indices.is_empty() {
            return;
        }

        // Page-down should land near the next page boundary, but it still needs
        // to skip any disabled rows inside the destination window.
        let target = (self.selected_index + page_len.max(1)).min(self.filtered_indices.len() - 1);
        if let Some(position) = ((self.selected_index + 1)..=target)
            .rev()
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
            return;
        }

        if let Some(position) = (target..self.filtered_indices.len())
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
        }
    }

    /// Return the selected item, if the current filter still has a selectable row.
    pub(crate) fn selected(&self) -> Option<&T> {
        let item_index = *self.filtered_indices.get(self.selected_index)?;
        self.items
            .get(item_index)
            .filter(|item| item.is_selectable())
    }

    /// Return the number of tracked items, including currently filtered-out rows.
    pub(crate) fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Build the render-facing popup snapshot for the current query and selection.
    pub(crate) fn popup(
        &self,
        spec: PickerPopupSpec,
        query: &str,
        cursor_column: usize,
    ) -> PickerPopup {
        let entries = self
            .filtered_indices
            .iter()
            .enumerate()
            .filter_map(|(position, &item_index)| {
                self.items
                    .get(item_index)
                    .map(|item| item.popup_entry(position == self.selected_index))
            })
            .collect();
        PickerPopup {
            title: spec.title.to_string(),
            query_label: spec.query_label.to_string(),
            empty_message: spec.empty_message.to_string(),
            query: query.to_string(),
            cursor_column,
            entries,
        }
    }

    /// Return whether the filtered row at `position` can be confirmed.
    fn position_is_selectable(&self, position: usize) -> bool {
        self.filtered_indices
            .get(position)
            .and_then(|&item_index| self.items.get(item_index))
            .is_some_and(PickerItem::is_selectable)
    }

    /// Return the first selectable filtered row, if any.
    fn first_selectable_position(&self) -> Option<usize> {
        self.filtered_indices.iter().position(|&item_index| {
            self.items
                .get(item_index)
                .is_some_and(PickerItem::is_selectable)
        })
    }

    /// Return the selected logical key, if the current selection is selectable.
    fn selected_key(&self) -> Option<T::Key> {
        self.selected().map(PickerItem::key)
    }
}

/// Score one candidate label against `query` using subsequence matching.
pub(crate) fn fuzzy_match_score(candidate: &str, query: &str) -> Option<MatchScore> {
    let candidate_len = candidate.chars().count();
    if query.is_empty() {
        return Some(MatchScore {
            boundary_rank: 0,
            gap_count: 0,
            start_index: 0,
            span_len: 0,
            candidate_len,
        });
    }

    let mut query_chars = query.chars();
    let first_query = query_chars.next().map(|ch| ch.to_ascii_lowercase())?;
    let remaining_query = query_chars;
    let mut best_score = None;
    let mut previous_candidate = None;

    // Try every possible starting match so contiguous runs later in the string
    // can outrank earlier scattered matches in long paths or labels.
    for (start_index, (start_byte_idx, candidate_char)) in candidate.char_indices().enumerate() {
        if candidate_char.to_ascii_lowercase() != first_query {
            previous_candidate = Some(candidate_char);
            continue;
        }

        let boundary_rank = usize::from(!is_boundary(previous_candidate));
        let mut remaining_query_chars = remaining_query.clone();
        let mut next_query = remaining_query_chars
            .next()
            .map(|ch| ch.to_ascii_lowercase());
        let mut previous_match_index = start_index;
        let mut gap_count = 0;
        let mut end_index = start_index;
        let remaining = &candidate[start_byte_idx + candidate_char.len_utf8()..];

        // From one chosen start, keep the leftmost completion for the rest of the
        // query so the score reflects how tightly the full subsequence packs.
        for (offset, remaining_char) in remaining.chars().enumerate() {
            let Some(expected_query) = next_query else {
                break;
            };
            if remaining_char.to_ascii_lowercase() != expected_query {
                continue;
            }

            let candidate_index = start_index + 1 + offset;
            gap_count += candidate_index - previous_match_index - 1;
            previous_match_index = candidate_index;
            end_index = candidate_index;
            next_query = remaining_query_chars
                .next()
                .map(|ch| ch.to_ascii_lowercase());
        }

        if next_query.is_some() {
            previous_candidate = Some(candidate_char);
            continue;
        }

        let score = MatchScore {
            boundary_rank,
            gap_count,
            start_index,
            span_len: end_index - start_index + 1,
            candidate_len,
        };
        best_score = Some(best_score.map_or(score, |current: MatchScore| current.min(score)));
        previous_candidate = Some(candidate_char);
    }

    best_score
}

/// Return whether a match after `previous` starts at a word or path boundary.
fn is_boundary(previous: Option<char>) -> bool {
    previous.is_none_or(|ch| matches!(ch, '/' | '\\' | '-' | '_' | ' ' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One lightweight test item for shared picker behavior.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestItem {
        key: usize,
        label: String,
        order: usize,
        pinned: bool,
        selectable: bool,
    }

    impl PickerItem for TestItem {
        type Key = usize;

        fn key(&self) -> Self::Key {
            self.key
        }

        fn label(&self) -> &str {
            &self.label
        }

        fn order(&self) -> usize {
            self.order
        }

        fn is_selectable(&self) -> bool {
            self.selectable
        }

        fn is_pinned(&self) -> bool {
            self.pinned
        }

        fn popup_entry(&self, selected: bool) -> PickerPopupEntry {
            PickerPopupEntry {
                label: self.label.clone(),
                selected,
                active: self.pinned,
                modified: false,
            }
        }
    }

    /// Build one test picker item with the requested flags.
    fn item(key: usize, order: usize, label: &str, pinned: bool, selectable: bool) -> TestItem {
        TestItem {
            key,
            label: label.to_string(),
            order,
            pinned,
            selectable,
        }
    }

    #[test]
    fn test_picker_prefers_tighter_fuzzy_matches() {
        let mut picker = PickerState::new(vec![
            item(1, 0, "/tmp/src_buffer.rs", false, true),
            item(2, 1, "/tmp/scratch/base.rs", false, true),
        ]);

        picker.sync_query("sbr");

        let popup = picker.popup(
            PickerPopupSpec {
                title: "Test",
                query_label: "Filter: ",
                empty_message: "No matches",
            },
            "sbr",
            3,
        );
        assert_eq!(popup.entries.len(), 2);
        assert_eq!(popup.entries[0].label, "/tmp/src_buffer.rs");
    }

    #[test]
    fn test_picker_preserves_selected_item_across_query_updates() {
        let mut picker = PickerState::new(vec![
            item(1, 0, "/tmp/alpha.rs", false, true),
            item(2, 1, "/tmp/beta.rs", false, true),
            item(3, 2, "/tmp/beta_test.rs", false, true),
        ]);

        picker.move_down();
        picker.sync_query("beta");

        assert_eq!(picker.selected().map(PickerItem::key), Some(2));
    }

    #[test]
    fn test_picker_keeps_pinned_items_visible_above_matches() {
        let mut picker = PickerState::new(vec![
            item(1, 0, "/tmp/current.rs", true, false),
            item(2, 1, "/tmp/alpha.rs", false, true),
            item(3, 2, "/tmp/beta.rs", false, true),
        ]);

        picker.sync_query("beta");

        let popup = picker.popup(
            PickerPopupSpec {
                title: "Test",
                query_label: "Filter: ",
                empty_message: "No matches",
            },
            "beta",
            4,
        );
        assert_eq!(popup.entries[0].label, "/tmp/current.rs");
        assert_eq!(popup.entries[1].label, "/tmp/beta.rs");
        assert!(!popup.entries[0].selected);
        assert!(popup.entries[1].selected);
    }
}
