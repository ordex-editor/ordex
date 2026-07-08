//! Shared picker state, popup models, and fuzzy matching helpers.

use crate::syntax::HighlightSpan;

/// One rendered picker row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerPopupEntry {
    /// Full display label shown in the picker row.
    pub(crate) label: String,
    /// Optional structured search-result segments used for width-aware display formatting.
    pub(crate) search_result_parts: Option<PickerPopupSearchResultParts>,
    /// Whether this row is currently selected.
    pub(crate) selected: bool,
    /// Whether this row uses the primary marker and accent styling.
    pub(crate) primary_marker: bool,
    /// Whether this row uses the secondary marker slot.
    pub(crate) secondary_marker: bool,
}

/// Structured segments for one Search Results row shown by the generic picker popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerPopupSearchResultParts {
    /// `path:line:column` text shown before preview content.
    pub(crate) location_label: String,
    /// Search-line preview text shown after the location label.
    pub(crate) preview_label: String,
}

/// One syntax-highlighted source line rendered inside the preview pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerPreviewLine {
    /// One-based line number shown beside the preview text.
    pub(crate) line_number: usize,
    /// Visible source text for the logical line.
    pub(crate) text: String,
    /// Syntax-highlight spans for the line content.
    pub(crate) spans: Vec<HighlightSpan>,
    /// Whether this line is the primary target highlighted by the picker.
    pub(crate) highlighted: bool,
}

/// Render-facing snapshot for the optional picker preview pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerPreviewPopup {
    /// Popup title shown in the top border.
    pub(crate) title: String,
    /// User-facing path label shown above the preview content.
    pub(crate) path_label: String,
    /// Optional status message shown when no preview lines are available.
    pub(crate) status_message: Option<String>,
    /// Prepared preview lines in display order.
    pub(crate) lines: Vec<PickerPreviewLine>,
}

impl PickerPreviewPopup {
    /// Build one ready-to-render preview popup with source lines.
    pub(crate) fn ready(path_label: String, lines: Vec<PickerPreviewLine>) -> Self {
        Self {
            title: "Preview".to_string(),
            path_label,
            status_message: None,
            lines,
        }
    }

    /// Build one placeholder popup shown while preview work is in flight.
    pub(crate) fn loading(path_label: String) -> Self {
        Self {
            title: "Preview".to_string(),
            path_label,
            status_message: Some("Loading preview...".to_string()),
            lines: Vec::new(),
        }
    }

    /// Build one error popup that explains why no preview is available.
    pub(crate) fn error(message: String) -> Self {
        Self {
            title: "Preview".to_string(),
            path_label: String::new(),
            status_message: Some(message),
            lines: Vec::new(),
        }
    }

    /// Build one blank popup that keeps the preview pane visible with no content.
    pub(crate) fn empty() -> Self {
        Self {
            title: String::new(),
            path_label: String::new(),
            status_message: None,
            lines: Vec::new(),
        }
    }

    /// Return whether the popup has no title, path, status, or preview lines.
    ///
    /// Returns `true` when the popup is fully blank, and `false` when any
    /// content field is populated.
    pub(crate) fn is_empty(&self) -> bool {
        self.title.is_empty()
            && self.path_label.is_empty()
            && self.lines.is_empty()
            && self.status_message.is_none()
    }
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
    /// Optional right-side preview pane shown beside the picker.
    pub(crate) preview: Option<PickerPreviewPopup>,
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
    /// Reused buffer holding the currently filtered pinned indices during one merge pass.
    merge_existing_pinned: Vec<usize>,
    /// Reused buffer holding the currently filtered non-pinned match indices during one merge pass.
    merge_existing_matches: Vec<usize>,
}

/// Count summary for fuzzy-matchable picker rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PickerMatchCounts {
    /// Number of currently filtered fuzzy rows.
    pub(crate) filtered: usize,
    /// Number of total fuzzy-matchable rows.
    pub(crate) total: usize,
}

impl<T: PickerItem> PickerState<T> {
    /// Create picker state from the current ordered item list.
    pub(crate) fn new(items: Vec<T>) -> Self {
        let mut picker = Self {
            items,
            match_scores: Vec::new(),
            filtered_indices: Vec::new(),
            selected_index: 0,
            merge_existing_pinned: Vec::new(),
            merge_existing_matches: Vec::new(),
        };
        picker.sync_query("");
        picker
    }

    /// Append new items and refresh matches for the active query.
    pub(crate) fn extend_items<I>(&mut self, items: I, query: &str)
    where
        I: IntoIterator<Item = T>,
    {
        let was_at_top = self.selected_index == 0;
        let selected_position = self.selected_index;
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
        self.restore_selection(was_at_top, selected_position);
    }

    /// Recompute matches for `query` while preserving the selected item when possible.
    pub(crate) fn sync_query(&mut self, query: &str) {
        let was_at_top = self.selected_index == 0;
        let selected_position = self.selected_index;
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

        self.restore_selection(was_at_top, selected_position);
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
            return;
        }

        // If the whole destination span is disabled, keep scanning below it so
        // page-down still reaches the next confirmable row when one exists.
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

    /// Return fuzzy row counts for the active filter, excluding pinned rows.
    pub(crate) fn fuzzy_match_counts(&self) -> PickerMatchCounts {
        // Pinned rows stay visible as context entries and are intentionally excluded
        // from both the filtered and total fuzzy-match counts.
        let total = self.items.iter().filter(|item| !item.is_pinned()).count();
        let filtered = self
            .filtered_indices
            .iter()
            .filter(|&&index| {
                self.items
                    .get(index)
                    .is_some_and(|item| !item.is_pinned() && self.match_scores[index].is_some())
            })
            .count();
        PickerMatchCounts { filtered, total }
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
            preview: None,
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

        let existing_pinned_len = self
            .filtered_indices
            .iter()
            .take_while(|&&index| self.items[index].is_pinned())
            .count();
        // For empty-query streaming with no pinned rows, appended rows already follow
        // the stable order, so appending avoids the merge work entirely.
        if self.can_append_empty_query_matches(existing_pinned_len, &new_pinned, &new_matches) {
            self.filtered_indices.extend(new_matches);
            return;
        }

        // Pre-sort only the appended slices so each side of the merge is sorted.
        new_pinned.sort_by_key(|&index| self.pinned_sort_key(index));
        new_matches.sort_by_key(|&index| self.match_sort_key(index));

        // Copy current filtered partitions into reusable scratch buffers so the
        // destination vector can be rebuilt without allocating fresh side buffers.
        self.merge_existing_pinned.clear();
        self.merge_existing_pinned
            .extend_from_slice(&self.filtered_indices[..existing_pinned_len]);
        self.merge_existing_matches.clear();
        self.merge_existing_matches
            .extend_from_slice(&self.filtered_indices[existing_pinned_len..]);

        // Reuse the filtered vector allocation as the merge output buffer.
        let mut merged_indices = std::mem::take(&mut self.filtered_indices);
        merged_indices.clear();
        merged_indices.reserve(
            self.merge_existing_pinned
                .len()
                .saturating_add(new_pinned.len())
                .saturating_add(self.merge_existing_matches.len())
                .saturating_add(new_matches.len()),
        );

        // Merge pinned and matched slices separately because pinned rows always stay first,
        // and each slice has its own sort key.
        self.merge_sorted_indices_into(
            &self.merge_existing_pinned,
            &new_pinned,
            |left, right| self.pinned_sort_key(left) <= self.pinned_sort_key(right),
            &mut merged_indices,
        );
        self.merge_sorted_indices_into(
            &self.merge_existing_matches,
            &new_matches,
            |left, right| self.match_sort_key(left) <= self.match_sort_key(right),
            &mut merged_indices,
        );
        self.filtered_indices = merged_indices;
    }

    /// Return whether one empty-query append can skip full merge and stay ordered.
    ///
    /// Returns `true` when the appended rows can be added at the tail without
    /// violating picker ordering, and returns `false` when full merge is still required.
    fn can_append_empty_query_matches(
        &self,
        existing_pinned_len: usize,
        new_pinned: &[usize],
        new_matches: &[usize],
    ) -> bool {
        if new_matches.is_empty() {
            return true;
        }
        if existing_pinned_len > 0 || !new_pinned.is_empty() {
            return false;
        }
        // Empty-query fast path only applies when all appended rows are matched.
        if !new_matches
            .iter()
            .all(|&index| self.match_scores[index].is_some())
        {
            return false;
        }

        // Preserve sorted invariants by confirming new rows are already monotonic
        // against the current tail according to the stable match sort key.
        let mut previous = self
            .filtered_indices
            .last()
            .copied()
            .map(|index| self.match_sort_key(index));
        for &index in new_matches {
            let key = self.match_sort_key(index);
            if previous.is_some_and(|previous| key < previous) {
                return false;
            }
            previous = Some(key);
        }
        true
    }

    /// Restore the selected row after the query or item set changes.
    fn restore_selection(&mut self, was_at_top: bool, selected_position: usize) {
        if self.filtered_indices.is_empty() {
            self.selected_index = 0;
            return;
        }

        // Stay on the top match when the user never moved the selection.
        if was_at_top {
            self.selected_index = self.first_selectable_position().unwrap_or(0);
            return;
        }

        // Keep the same UI row when the user explicitly moved the selection.
        let last = self.filtered_indices.len().saturating_sub(1);
        self.selected_index = selected_position.min(last);
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
    fn merge_sorted_indices_into<F>(
        &self,
        left: &[usize],
        right: &[usize],
        mut prefer_left: F,
        merged: &mut Vec<usize>,
    ) where
        F: FnMut(usize, usize) -> bool,
    {
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
    }
}

/// Score one candidate label against `query` using subsequence matching.
pub(crate) fn fuzzy_match_score(candidate: &str, query: &str) -> Option<MatchScore> {
    if query.trim().is_empty() {
        return Some(empty_query_match_score());
    }

    if query_excludes_candidate(candidate, query) {
        return None;
    }

    let mut combined_score: Option<MatchScore> = None;
    for term in query.split_whitespace() {
        if let QueryTerm::Include(term) = parse_query_term(term) {
            // Whitespace-separated positive terms still act as independent fuzzy
            // filters so users can match multiple path segments in any order.
            let score = fuzzy_match_term_score(candidate, term)?;
            combined_score = Some(match combined_score {
                Some(existing) => existing.merge(score),
                None => score,
            });
        }
    }

    combined_score.or(Some(empty_query_match_score()))
}

/// Return the neutral score used when no positive query terms are present.
fn empty_query_match_score() -> MatchScore {
    MatchScore {
        boundary_rank: 0,
        gap_count: 0,
        start_index: 0,
        span_len: 0,
        candidate_len: 0,
    }
}

/// Return whether any negated token in `query` excludes `candidate`.
pub(crate) fn query_excludes_candidate(candidate: &str, query: &str) -> bool {
    query.split_whitespace().any(|term| {
        if let QueryTerm::Exclude(term) = parse_query_term(term) {
            return contains_excluded_term(candidate, term);
        }
        false
    })
}

/// One parsed query token with inclusion or exclusion semantics.
enum QueryTerm<'a> {
    Include(&'a str),
    Exclude(&'a str),
    Ignore,
}

/// Parse one query token into its picker filtering behavior.
fn parse_query_term(term: &str) -> QueryTerm<'_> {
    if let Some(excluded) = term.strip_prefix('!') {
        if excluded.is_empty() {
            return QueryTerm::Ignore;
        }
        return QueryTerm::Exclude(excluded);
    }
    QueryTerm::Include(term)
}

/// Return whether one exclusion token appears literally inside `candidate`.
fn contains_excluded_term(candidate: &str, query: &str) -> bool {
    candidate.contains(query)
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

#[cfg(all(test, feature = "unstable_bench"))]
#[path = "picker_bench.rs"]
mod picker_bench;

#[cfg(test)]
mod tests {
    use super::*;

    /// One lightweight test item for shared picker behavior.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestItem {
        label: String,
        order: usize,
        pinned: bool,
        selectable: bool,
    }

    impl PickerItem for TestItem {
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
                search_result_parts: None,
                selected,
                primary_marker: self.pinned,
                secondary_marker: false,
            }
        }
    }

    /// Build one test picker item with the requested flags.
    fn item(order: usize, label: &str, pinned: bool, selectable: bool) -> TestItem {
        TestItem {
            label: label.to_string(),
            order,
            pinned,
            selectable,
        }
    }

    #[test]
    fn test_picker_prefers_tighter_fuzzy_matches() {
        let mut picker = PickerState::new(vec![
            item(0, "/tmp/src_buffer.rs", false, true),
            item(1, "/tmp/scratch/base.rs", false, true),
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
    fn test_negated_term_filters_literal_substrings() {
        assert!(fuzzy_match_score("src", "!src").is_none());
        assert!(fuzzy_match_score("src/main.rs", "!src").is_none());
        assert!(fuzzy_match_score("src", "!Src").is_some());
    }

    #[test]
    fn test_negated_term_combines_with_positive_fuzzy_terms() {
        assert!(fuzzy_match_score("src/main.rs", "main !src/").is_none());
        assert!(fuzzy_match_score("tests/main.rs", "main !src/").is_some());
    }

    #[test]
    fn test_negated_only_query_matches_non_excluded_items() {
        assert!(fuzzy_match_score("src/main.rs", "!src/lib.rs").is_some());
    }

    #[test]
    fn test_empty_negated_term_does_not_filter_anything() {
        assert_eq!(
            fuzzy_match_score("src/main.rs", "!"),
            fuzzy_match_score("src/main.rs", "")
        );
        assert!(fuzzy_match_score("src/main.rs", "main !").is_some());
    }

    #[test]
    fn test_picker_preserves_selected_item_across_query_updates() {
        let mut picker = PickerState::new(vec![
            item(0, "/tmp/alpha.rs", false, true),
            item(1, "/tmp/beta.rs", false, true),
            item(2, "/tmp/beta_test.rs", false, true),
        ]);

        picker.move_down();
        picker.sync_query("beta");

        assert_eq!(picker.selected().map(PickerItem::order), Some(2));
    }

    #[test]
    fn test_picker_keeps_pinned_items_visible_above_matches() {
        let mut picker = PickerState::new(vec![
            item(0, "/tmp/current.rs", true, false),
            item(1, "/tmp/alpha.rs", false, true),
            item(2, "/tmp/beta.rs", false, true),
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

    /// Empty queries should preserve source order instead of ranking shorter labels first.
    #[test]
    fn test_empty_query_keeps_first_item_selected_after_streaming_update() {
        let mut picker = PickerState::new(vec![item(
            0,
            "src/very_long_component_name.rs",
            false,
            true,
        )]);

        picker.extend_items(
            [item(1, "a.rs", false, true), item(2, "b.rs", false, true)],
            "",
        );

        assert_eq!(picker.selected().map(PickerItem::order), Some(0));
    }

    /// Empty queries should keep differing label lengths in stable source order.
    #[test]
    fn test_empty_query_popup_preserves_source_order_for_different_label_lengths() {
        let picker = PickerState::new(vec![
            item(0, "src/syntax/profiles/go.rs", false, true),
            item(1, "src/render.rs", false, true),
            item(2, "src/syntax/profiles/r.rs", false, true),
        ]);

        let popup = picker.popup(
            PickerPopupSpec {
                title: "Test",
                query_label: "Filter: ",
                empty_message: "No matches",
            },
            "",
            0,
            10,
        );

        assert_eq!(
            popup
                .entries
                .into_iter()
                .map(|entry| entry.label)
                .collect::<Vec<_>>(),
            vec![
                "src/syntax/profiles/go.rs".to_string(),
                "src/render.rs".to_string(),
                "src/syntax/profiles/r.rs".to_string(),
            ]
        );
    }

    #[test]
    fn test_empty_query_preserves_user_selection_during_streaming_update() {
        let mut picker = PickerState::new(vec![
            item(0, "a.rs", false, true),
            item(1, "alphabet.rs", false, true),
        ]);

        assert_eq!(picker.selected().map(PickerItem::order), Some(0));
        picker.move_down();
        assert_eq!(picker.selected().map(PickerItem::order), Some(1));
        picker.extend_items([item(2, "b.rs", false, true)], "");

        assert_eq!(picker.selected().map(PickerItem::order), Some(1));
    }

    #[test]
    fn test_streaming_update_preserves_ranked_order_for_non_empty_query() {
        let initial_items = vec![
            item(0, "src/alpha_notes.rs", false, true),
            item(1, "src/app.rs", false, true),
        ];
        let appended_items = vec![
            item(2, "src/api.rs", false, true),
            item(3, "src/shape.rs", false, true),
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
                .map(|index| item(index, "item", false, true))
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
            fn label(&self) -> &str {
                "entry"
            }

            fn order(&self) -> usize {
                self.key
            }

            fn popup_entry(&self, selected: bool) -> PickerPopupEntry {
                PickerPopupEntry {
                    label: "entry".to_string(),
                    search_result_parts: None,
                    selected,
                    primary_marker: false,
                    secondary_marker: false,
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

    #[test]
    fn test_typing_query_without_moving_follows_top_match() {
        let mut picker = PickerState::new(vec![
            item(0, "alpha", false, true),
            item(1, "beta", false, true),
        ]);

        assert_eq!(picker.selected().map(PickerItem::order), Some(0));

        picker.sync_query("beta");

        assert_eq!(picker.selected().map(PickerItem::order), Some(1));
    }

    #[test]
    fn test_typing_query_after_explicit_move_preserves_by_position() {
        let mut picker = PickerState::new(vec![
            item(0, "alpha", false, true),
            item(1, "beta", false, true),
            item(2, "gamma", false, true),
        ]);

        picker.move_down();
        assert_eq!(picker.selected().map(PickerItem::order), Some(1));

        // Query "a" matches alpha, gamma, and beta. Alpha ranks first.
        // Position 1 is now gamma (order=2) in the filtered list.
        picker.sync_query("a");

        assert_eq!(picker.selected().map(PickerItem::order), Some(2));
    }

    #[test]
    fn test_backspace_to_empty_after_filter_collapse_follows_top_match() {
        let mut picker = PickerState::new(vec![
            item(0, "alpha", false, true),
            item(1, "beta", false, true),
        ]);

        picker.move_down();
        assert_eq!(picker.selected().map(PickerItem::order), Some(1));

        // Filtering to only beta collapses the selection to position 0,
        // which resets the "user moved" signal.
        picker.sync_query("beta");
        assert_eq!(picker.selected().map(PickerItem::order), Some(1));

        picker.sync_query("");

        assert_eq!(picker.selected().map(PickerItem::order), Some(0));
    }

    #[test]
    fn test_explicit_move_then_query_filters_out_selected_item() {
        let mut picker = PickerState::new(vec![
            item(0, "alpha", false, true),
            item(1, "beta", false, true),
            item(2, "gamma", false, true),
        ]);

        picker.move_down();
        assert_eq!(picker.selected().map(PickerItem::order), Some(1));

        picker.sync_query("gamma");

        assert_eq!(picker.selected().map(PickerItem::order), Some(2));
    }

    #[test]
    fn test_streaming_update_without_moving_follows_top_match() {
        let mut picker = PickerState::new(vec![item(0, "alpha", false, true)]);

        // Simulate the user typing "beta" before streaming completes,
        // which filters out "alpha" from the existing items.
        picker.sync_query("beta");

        picker.extend_items([item(1, "beta", false, true)], "beta");

        assert_eq!(picker.selected().map(PickerItem::order), Some(1));
    }

    #[test]
    /// Fuzzy count summaries should track both filtered and total non-pinned rows.
    fn test_fuzzy_match_counts_track_filtered_and_total_rows() {
        let mut picker = PickerState::new(vec![
            item(0, "alpha", false, true),
            item(1, "beta", false, true),
            item(2, "gamma", false, true),
        ]);

        // Empty queries include all fuzzy-matchable rows.
        assert_eq!(
            picker.fuzzy_match_counts(),
            PickerMatchCounts {
                filtered: 3,
                total: 3
            }
        );

        // Narrowing the query should affect only the filtered side of the ratio.
        picker.sync_query("gm");

        assert_eq!(
            picker.fuzzy_match_counts(),
            PickerMatchCounts {
                filtered: 1,
                total: 3
            }
        );
    }

    #[test]
    /// Pinned rows should stay visible without contributing to fuzzy count summaries.
    fn test_fuzzy_match_counts_exclude_pinned_rows() {
        let mut picker = PickerState::new(vec![
            item(0, "active.rs", true, false),
            item(1, "alpha.rs", false, true),
            item(2, "beta.rs", false, true),
        ]);

        // The pinned active row stays visible but does not affect fuzzy counts.
        assert_eq!(
            picker.fuzzy_match_counts(),
            PickerMatchCounts {
                filtered: 2,
                total: 2
            }
        );

        // Query filtering still excludes pinned rows from both numerator and denominator.
        picker.sync_query("alp");

        assert_eq!(
            picker.fuzzy_match_counts(),
            PickerMatchCounts {
                filtered: 1,
                total: 2
            }
        );
    }

    /// Verify that `empty()` builds a popup with no title, path, status, or lines.
    #[test]
    fn test_picker_preview_popup_empty_has_no_content() {
        let popup = PickerPreviewPopup::empty();
        assert!(popup.title.is_empty());
        assert!(popup.path_label.is_empty());
        assert!(popup.status_message.is_none());
        assert!(popup.lines.is_empty());
    }

    /// Verify that `is_empty()` returns `true` only for the empty constructor.
    #[test]
    fn test_picker_preview_popup_is_empty_distinguishes_empty_from_populated() {
        assert!(PickerPreviewPopup::empty().is_empty());
        assert!(!PickerPreviewPopup::ready("path".to_string(), Vec::new()).is_empty());
        assert!(!PickerPreviewPopup::loading("path".to_string()).is_empty());
        assert!(!PickerPreviewPopup::error("oops".to_string()).is_empty());
    }
}
