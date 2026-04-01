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
    /// Optional right-aligned suffix shown on the query row.
    pub(crate) query_suffix: String,
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

impl MatchScore {
    /// Combine two token scores into one score for a multi-term query.
    fn merge(self, other: Self) -> Self {
        Self {
            boundary_rank: self.boundary_rank.saturating_add(other.boundary_rank),
            gap_count: self.gap_count.saturating_add(other.gap_count),
            start_index: self.start_index.min(other.start_index),
            span_len: self.span_len.saturating_add(other.span_len),
            candidate_len: self.candidate_len.max(other.candidate_len),
        }
    }
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
    match_scores: Vec<Option<MatchScore>>,
    filtered_indices: Vec<usize>,
    selected_index: usize,
    /// Track whether empty-query streaming should keep following the top-ranked row.
    follow_top_match_on_empty_query: bool,
}

impl<T: PickerItem> PickerState<T> {
    /// Create picker state from the current ordered item list.
    pub(crate) fn new(items: Vec<T>) -> Self {
        let mut picker = Self {
            items,
            match_scores: Vec::new(),
            filtered_indices: Vec::new(),
            selected_index: 0,
            follow_top_match_on_empty_query: true,
        };
        picker.sync_query("");
        picker
    }

    /// Append new items and refresh matches for the active query.
    pub(crate) fn extend_items<I>(&mut self, items: I, query: &str)
    where
        I: IntoIterator<Item = T>,
    {
        let selected_key = self.selected_key();
        let start_index = self.items.len();
        self.items.extend(items);
        if start_index == self.items.len() {
            return;
        }

        // Cache scores for only the newly appended items so streaming updates do
        // not need to rescore the full picker contents on every batch.
        for index in start_index..self.items.len() {
            let item = &self.items[index];
            self.match_scores.push(if item.is_pinned() {
                None
            } else {
                item.match_score(query)
            });
        }

        self.merge_filtered_indices_for_appended_items(start_index);
        self.restore_selection(query, selected_key);
    }

    /// Recompute matches for `query` while preserving the selected item when possible.
    pub(crate) fn sync_query(&mut self, query: &str) {
        let selected_key = self.selected_key();
        self.match_scores = self
            .items
            .iter()
            .map(|item| {
                if item.is_pinned() {
                    None
                } else {
                    item.match_score(query)
                }
            })
            .collect();
        let mut pinned = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_pinned())
            .collect::<Vec<_>>();
        let mut matches = self
            .match_scores
            .iter()
            .enumerate()
            .filter_map(|(index, score)| score.map(|score| (index, score)))
            .collect::<Vec<_>>();

        // Pinned rows stay in their stable order at the top of the popup, while
        // fuzzy-matched rows sort by match quality and then by their source order.
        pinned.sort_by_key(|(index, item)| (item.order(), *index));
        matches.sort_by_key(|(index, score)| (*score, self.items[*index].order(), *index));

        self.filtered_indices = pinned.into_iter().map(|(index, _)| index).collect();
        self.filtered_indices
            .extend(matches.into_iter().map(|(index, _)| index));

        self.restore_selection(query, selected_key);
    }

    /// Move the picker selection one row up, stopping at the first row.
    pub(crate) fn move_up(&mut self) {
        // Scan backward to the closest selectable row instead of landing on the
        // disabled context rows that stay visible in some pickers.
        if let Some(position) = (0..self.selected_index)
            .rev()
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
            self.follow_top_match_on_empty_query = false;
        }
    }

    /// Move the picker selection one row down, stopping at the last row.
    pub(crate) fn move_down(&mut self) {
        // Scan forward until the next confirmable row so the disabled active row
        // or other informational entries never take keyboard focus.
        if let Some(position) = ((self.selected_index + 1)..self.filtered_indices.len())
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
            self.follow_top_match_on_empty_query = false;
        }
    }

    /// Move the picker selection one page up, stopping at the first row.
    pub(crate) fn move_page_up(&mut self, page_len: usize) {
        let target = self.selected_index.saturating_sub(page_len.max(1));
        // Page-up should land as close as possible to the page boundary while
        // still honoring non-selectable rows inside that span.
        if let Some(position) =
            (target..self.selected_index).find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
            self.follow_top_match_on_empty_query = false;
        }
    }

    /// Move the picker selection one page down, stopping at the last row.
    pub(crate) fn move_page_down(&mut self, page_len: usize) {
        if self.filtered_indices.is_empty() {
            return;
        }

        // Search backward from the destination edge so page-down lands near the
        // next page boundary instead of stopping at the first selectable row in
        // that window.
        let target = (self.selected_index + page_len.max(1)).min(self.filtered_indices.len() - 1);
        if let Some(position) = ((self.selected_index + 1)..=target)
            .rev()
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
            self.follow_top_match_on_empty_query = false;
            return;
        }

        // If the whole destination span is disabled, keep scanning below it so
        // page-down still reaches the next confirmable row when one exists.
        if let Some(position) = (target..self.filtered_indices.len())
            .find(|&position| self.position_is_selectable(position))
        {
            self.selected_index = position;
            self.follow_top_match_on_empty_query = false;
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
        visible_entry_capacity: usize,
    ) -> PickerPopup {
        let entries = if self.filtered_indices.is_empty() || visible_entry_capacity == 0 {
            Vec::new()
        } else {
            // Materialize only the rows the renderer can actually display so very
            // large pickers stay responsive while the user moves the selection.
            let start_index = self
                .selected_index
                .saturating_sub(visible_entry_capacity.saturating_sub(1) / 2);
            self.filtered_indices
                .iter()
                .skip(start_index)
                .take(visible_entry_capacity)
                .enumerate()
                .filter_map(|(offset, &item_index)| {
                    self.items
                        .get(item_index)
                        .map(|item| item.popup_entry(start_index + offset == self.selected_index))
                })
                .collect()
        };
        PickerPopup {
            title: spec.title.to_string(),
            query_label: spec.query_label.to_string(),
            query_suffix: String::new(),
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

    /// Merge newly appended items into the already-ranked filtered list.
    fn merge_filtered_indices_for_appended_items(&mut self, start_index: usize) {
        let mut new_pinned = Vec::new();
        let mut new_matches = Vec::new();

        // Classify appended rows once so the merge can reuse the existing ranked slices.
        for index in start_index..self.items.len() {
            if self.items[index].is_pinned() {
                new_pinned.push(index);
            } else if self.match_scores[index].is_some() {
                new_matches.push(index);
            }
        }

        new_pinned.sort_by_key(|&index| self.pinned_sort_key(index));
        new_matches.sort_by_key(|&index| self.match_sort_key(index));

        let existing_pinned_len = self
            .filtered_indices
            .iter()
            .take_while(|&&index| self.items[index].is_pinned())
            .count();
        let existing_pinned = self.filtered_indices[..existing_pinned_len].to_vec();
        let existing_matches = self.filtered_indices[existing_pinned_len..].to_vec();

        // Merge pinned and matched slices separately because pinned rows always stay first.
        self.filtered_indices =
            self.merge_sorted_indices(&existing_pinned, &new_pinned, |left, right| {
                self.pinned_sort_key(left) <= self.pinned_sort_key(right)
            });
        self.filtered_indices.extend(self.merge_sorted_indices(
            &existing_matches,
            &new_matches,
            |left, right| self.match_sort_key(left) <= self.match_sort_key(right),
        ));
    }

    /// Restore the selected row after the query or item set changes.
    fn restore_selection(&mut self, query: &str, selected_key: Option<T::Key>) {
        if self.filtered_indices.is_empty() {
            self.selected_index = 0;
            return;
        }

        // Empty-query streaming follows the top result until the user manually moves away.
        if query.is_empty() && self.follow_top_match_on_empty_query {
            self.selected_index = self.first_selectable_position().unwrap_or(0);
            return;
        }

        // Preserve the logical selection whenever that item still matches.
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

    /// Return the stable sort key for one pinned row.
    fn pinned_sort_key(&self, index: usize) -> (usize, usize) {
        (self.items[index].order(), index)
    }

    /// Return the stable sort key for one matched row.
    fn match_sort_key(&self, index: usize) -> (MatchScore, usize, usize) {
        (
            self.match_scores[index].expect("matched rows should have a cached score"),
            self.items[index].order(),
            index,
        )
    }

    /// Merge two already-sorted index slices while preserving their ordering key.
    fn merge_sorted_indices<F>(
        &self,
        left: &[usize],
        right: &[usize],
        mut prefer_left: F,
    ) -> Vec<usize>
    where
        F: FnMut(usize, usize) -> bool,
    {
        let mut merged = Vec::with_capacity(left.len() + right.len());
        let mut left_index = 0usize;
        let mut right_index = 0usize;

        // Walk both sorted inputs once so streaming batches stay linear in the
        // number of new rows instead of rebuilding the full ranked list.
        while left_index < left.len() && right_index < right.len() {
            if prefer_left(left[left_index], right[right_index]) {
                merged.push(left[left_index]);
                left_index += 1;
            } else {
                merged.push(right[right_index]);
                right_index += 1;
            }
        }

        merged.extend_from_slice(&left[left_index..]);
        merged.extend_from_slice(&right[right_index..]);
        merged
    }
}

/// Score one candidate label against `query` using subsequence matching.
pub(crate) fn fuzzy_match_score(candidate: &str, query: &str) -> Option<MatchScore> {
    if query.trim().is_empty() {
        return fuzzy_match_term_score(candidate, "");
    }

    let mut terms = query.split_whitespace();
    let first_term = terms.next()?;
    let mut combined_score = fuzzy_match_term_score(candidate, first_term)?;

    // Whitespace-separated terms act as independent filters so users can match
    // multiple path segments without forcing one global subsequence order.
    for term in terms {
        combined_score = combined_score.merge(fuzzy_match_term_score(candidate, term)?);
    }

    Some(combined_score)
}

/// Score one candidate label against a single query term using subsequence matching.
fn fuzzy_match_term_score(candidate: &str, query: &str) -> Option<MatchScore> {
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

    // Keep the best complete subsequence across all viable start positions so
    // tighter runs later in the string can outrank earlier scattered matches.
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
            10,
        );
        assert_eq!(popup.entries.len(), 2);
        assert_eq!(popup.entries[0].label, "/tmp/src_buffer.rs");
    }

    #[test]
    fn test_single_term_requires_one_subsequence_order() {
        assert!(fuzzy_match_score("test/one", "testone").is_some());
        assert!(fuzzy_match_score("one/test", "testone").is_none());
    }

    #[test]
    fn test_whitespace_only_query_matches_like_empty_query() {
        assert_eq!(
            fuzzy_match_score("test/one", "   "),
            fuzzy_match_score("test/one", "")
        );
    }

    #[test]
    fn test_space_separated_terms_match_across_path_segments_in_any_order() {
        assert!(fuzzy_match_score("test/one", "test one").is_some());
        assert!(fuzzy_match_score("one/test", "test one").is_some());
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
            10,
        );
        assert_eq!(popup.entries[0].label, "/tmp/current.rs");
        assert_eq!(popup.entries[1].label, "/tmp/beta.rs");
        assert!(!popup.entries[0].selected);
        assert!(popup.entries[1].selected);
    }

    #[test]
    fn test_empty_query_selects_top_ranked_match_after_streaming_update() {
        let mut picker = PickerState::new(vec![item(
            1,
            0,
            "src/very_long_component_name.rs",
            false,
            true,
        )]);

        picker.extend_items(
            [
                item(2, 1, "a.rs", false, true),
                item(3, 2, "b.rs", false, true),
            ],
            "",
        );

        assert_eq!(picker.selected().map(PickerItem::key), Some(2));
    }

    #[test]
    fn test_empty_query_preserves_user_selection_during_streaming_update() {
        let mut picker = PickerState::new(vec![
            item(1, 0, "a.rs", false, true),
            item(2, 1, "alphabet.rs", false, true),
        ]);

        assert_eq!(picker.selected().map(PickerItem::key), Some(1));
        picker.move_down();
        assert_eq!(picker.selected().map(PickerItem::key), Some(2));
        assert!(!picker.follow_top_match_on_empty_query);
        picker.extend_items([item(3, 2, "b.rs", false, true)], "");

        assert_eq!(picker.selected().map(PickerItem::key), Some(2));
    }

    #[test]
    fn test_streaming_update_preserves_ranked_order_for_non_empty_query() {
        let initial_items = vec![
            item(1, 0, "src/alpha_notes.rs", false, true),
            item(2, 1, "src/app.rs", false, true),
        ];
        let appended_items = vec![
            item(3, 2, "src/api.rs", false, true),
            item(4, 3, "src/shape.rs", false, true),
        ];
        let mut picker = PickerState::new(initial_items.clone());

        picker.sync_query("ap");
        picker.extend_items(appended_items.clone(), "ap");

        let mut rebuilt = PickerState::new(
            initial_items
                .into_iter()
                .chain(appended_items)
                .collect::<Vec<_>>(),
        );
        rebuilt.sync_query("ap");

        let popup = picker.popup(
            PickerPopupSpec {
                title: "Test",
                query_label: "Filter: ",
                empty_message: "No matches",
            },
            "ap",
            2,
            10,
        );
        let rebuilt_popup = rebuilt.popup(
            PickerPopupSpec {
                title: "Test",
                query_label: "Filter: ",
                empty_message: "No matches",
            },
            "ap",
            2,
            10,
        );
        assert_eq!(
            popup
                .entries
                .into_iter()
                .map(|entry| entry.label)
                .collect::<Vec<_>>(),
            rebuilt_popup
                .entries
                .into_iter()
                .map(|entry| entry.label)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_picker_popup_limits_entries_to_visible_window() {
        let mut picker = PickerState::new(
            (0..100)
                .map(|index| item(index, index, "item", false, true))
                .collect(),
        );
        // Move into the middle of the list so the popup has to choose a window.
        for _ in 0..40 {
            picker.move_down();
        }

        let popup = picker.popup(
            PickerPopupSpec {
                title: "Test",
                query_label: "Filter: ",
                empty_message: "No matches",
            },
            "",
            0,
            7,
        );

        assert_eq!(popup.entries.len(), 7);
        assert!(popup.entries.iter().any(|entry| entry.selected));
    }

    #[test]
    fn test_picker_popup_stays_fast_with_large_item_set() {
        #[derive(Clone, Copy)]
        struct PerfItem {
            key: usize,
        }

        impl PickerItem for PerfItem {
            type Key = usize;

            fn key(&self) -> Self::Key {
                self.key
            }

            fn label(&self) -> &str {
                "entry"
            }

            fn order(&self) -> usize {
                self.key
            }

            fn popup_entry(&self, selected: bool) -> PickerPopupEntry {
                PickerPopupEntry {
                    label: "entry".to_string(),
                    selected,
                    active: false,
                    modified: false,
                }
            }
        }

        let mut picker = PickerState::new((0..1_000_000).map(|key| PerfItem { key }).collect());
        let started = std::time::Instant::now();
        // Use a non-zero offset so popup construction cannot special-case the first row.
        for _ in 0..1000 {
            picker.move_down();
        }

        // The popup should stay bounded by visible rows instead of visiting every item.
        let popup = picker.popup(
            PickerPopupSpec {
                title: "Test",
                query_label: "Filter: ",
                empty_message: "No matches",
            },
            "",
            0,
            9,
        );

        assert_eq!(popup.entries.len(), 9);
        assert!(started.elapsed() <= std::time::Duration::from_millis(100));
    }
}
