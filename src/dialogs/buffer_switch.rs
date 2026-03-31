//! Buffer-switch picker state and lightweight fuzzy matching.

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

/// One rendered picker row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BufferSwitchPopupEntry {
    /// Full display label shown in the picker row.
    pub(crate) label: String,
    /// Whether this row is currently selected.
    pub(crate) selected: bool,
    /// Whether this row represents the active buffer.
    pub(crate) active: bool,
    /// Whether this row represents a modified buffer.
    pub(crate) modified: bool,
}

/// Render-facing snapshot for the buffer-switch picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BufferSwitchPopup {
    /// Current filter query.
    pub(crate) query: String,
    /// Zero-based cursor column inside the query.
    pub(crate) cursor_column: usize,
    /// Filtered picker rows in display order.
    pub(crate) entries: Vec<BufferSwitchPopupEntry>,
}

/// Mutable state for the buffer-switch picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BufferSwitchState {
    items: Vec<BufferSwitchItem>,
    filtered_indices: Vec<usize>,
    selected_index: usize,
}

impl BufferSwitchState {
    /// Create picker state from the current ordered buffer list.
    pub(crate) fn new(items: Vec<BufferSwitchItem>) -> Self {
        let filtered_indices = (0..items.len()).collect::<Vec<_>>();
        let selected_index =
            Self::first_selectable_position_in(&items, &filtered_indices).unwrap_or(0);
        Self {
            items,
            filtered_indices,
            selected_index,
        }
    }

    /// Recompute matches for `query` while preserving the selected buffer when possible.
    pub(crate) fn sync_query(&mut self, query: &str) {
        let selected_buffer_id = self.selected_buffer_id();
        // There is only one active buffer at a time, so keep its index separate
        // from the fuzzy-matched rows and pin it to the top of the popup.
        let active_index = self.items.iter().position(|item| item.active);
        let mut matches = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| !item.active)
            .filter_map(|(index, item)| {
                fuzzy_match_score(&item.label, query).map(|score| (index, score, item.order))
            })
            .collect::<Vec<_>>();

        // Match quality should dominate, while open-buffer order stays a stable tie-breaker.
        matches.sort_by_key(|(index, score, order)| (*score, *order, *index));
        self.filtered_indices = active_index.into_iter().collect();
        // The active row stays visible even when the query does not match it, so
        // the user always sees which buffer is current while browsing candidates.
        self.filtered_indices
            .extend(matches.into_iter().map(|(index, _, _)| index));

        if self.filtered_indices.is_empty() {
            self.selected_index = 0;
            return;
        }

        let selected_position = selected_buffer_id.and_then(|buffer_id| {
            self.filtered_indices.iter().position(|&index| {
                self.items
                    .get(index)
                    .is_some_and(|item| item.buffer_id == buffer_id)
            })
        });

        // Keep the previously selected buffer highlighted when it still survives
        // the new filter result set instead of jumping back to the first row.
        if let Some(position) = selected_position {
            self.selected_index = position;
            return;
        }

        self.selected_index =
            Self::first_selectable_position_in(&self.items, &self.filtered_indices).unwrap_or(0);
    }

    /// Move the picker selection one row up, stopping at the first row.
    pub(crate) fn move_up(&mut self) {
        // Disabled rows such as the active buffer stay visible, but navigation
        // should land only on entries that can actually be confirmed.
        // Scan backward to the closest selectable row instead of landing on the
        // disabled active entry that can be rendered above the current choice.
        if let Some(position) = (0..self.selected_index)
            .rev()
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
        }
    }

    /// Move the picker selection one row down, stopping at the last row.
    pub(crate) fn move_down(&mut self) {
        // Scan forward until the next confirmable row so the disabled active
        // buffer stays visible without intercepting cursor movement.
        if let Some(position) = ((self.selected_index + 1)..self.filtered_indices.len())
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
        }
    }

    /// Move the picker selection one page up, stopping at the first row.
    pub(crate) fn move_page_up(&mut self, page_len: usize) {
        let target = self.selected_index.saturating_sub(page_len.max(1));
        // Page-up should land as close as possible to the page boundary while
        // still skipping any non-selectable rows inside that destination span.
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

        let target = (self.selected_index + page_len.max(1)).min(self.filtered_indices.len() - 1);
        // Search backward from the destination edge so page-down lands near the
        // next page boundary instead of stopping early on the first selectable row.
        if let Some(position) = ((self.selected_index + 1)..=target)
            .rev()
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
            return;
        }
        // If the whole destination span is disabled, keep scanning below it
        // until we find the next row the user can actually confirm.
        if let Some(position) = (target..self.filtered_indices.len())
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
        }
    }

    /// Return the selected buffer id, if the current filter still has matches.
    pub(crate) fn selected_buffer_id(&self) -> Option<usize> {
        let item_index = *self.filtered_indices.get(self.selected_index)?;
        self.items
            .get(item_index)
            .filter(|item| !item.active)
            .map(|item| item.buffer_id)
    }

    /// Build the render-facing popup snapshot for the current query and selection.
    pub(crate) fn popup(&self, query: &str, cursor_column: usize) -> BufferSwitchPopup {
        let entries = self
            .filtered_indices
            .iter()
            .enumerate()
            .filter_map(|(position, &item_index)| {
                self.items
                    .get(item_index)
                    .map(|item| BufferSwitchPopupEntry {
                        label: item.label.clone(),
                        selected: position == self.selected_index && !item.active,
                        active: item.active,
                        modified: item.modified,
                    })
            })
            .collect();
        BufferSwitchPopup {
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
            .is_some_and(|item| !item.active)
    }

    /// Return the first selectable filtered row, if any.
    fn first_selectable_position_in(
        items: &[BufferSwitchItem],
        filtered_indices: &[usize],
    ) -> Option<usize> {
        filtered_indices
            .iter()
            .position(|&item_index| items.get(item_index).is_some_and(|item| !item.active))
    }
}

/// One sortable fuzzy-match score where lower values represent a better match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MatchScore {
    boundary_rank: usize,
    gap_count: usize,
    start_index: usize,
    span_len: usize,
    candidate_len: usize,
}

/// Score one candidate label against `query` using subsequence matching.
fn fuzzy_match_score(candidate: &str, query: &str) -> Option<MatchScore> {
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

    // Try every possible starting match for the query's first character so later
    // contiguous runs like `cpp` in `.../cpp.rs` can outrank earlier scattered matches.
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
        // query so the score measures how tightly the full subsequence packs together.
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

        // Keep the best complete subsequence across all viable start positions.
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

        let popup = picker.popup("sbr", 3);
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

        picker.move_down();
        picker.sync_query("beta");

        assert_eq!(picker.selected_buffer_id(), Some(2));
    }

    #[test]
    fn test_picker_shows_active_buffer_but_selects_first_inactive_entry() {
        let picker = BufferSwitchState::new(vec![
            active_item(1, 0, "/tmp/current.rs"),
            item(2, 1, "/tmp/other.rs"),
        ]);

        let popup = picker.popup("", 0);

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

        let popup = picker.popup("beta", 4);
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
        assert!(picker.popup("zzz", 3).entries.is_empty());
    }

    #[test]
    fn test_picker_prefers_contiguous_cpp_match_over_scattered_match() {
        let mut picker = BufferSwitchState::new(vec![
            item(1, 0, "src/app.rs"),
            item(2, 1, "src/syntax/profiles/cpp.rs"),
        ]);

        picker.sync_query("cpp");

        let popup = picker.popup("cpp", 3);
        assert_eq!(popup.entries[0].label, "src/syntax/profiles/cpp.rs");
        assert_eq!(popup.entries[1].label, "src/app.rs");
    }
}
