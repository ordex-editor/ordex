//! Prompt history helpers for command and search mode.

use std::collections::VecDeque;

const PROMPT_HISTORY_LIMIT: usize = 999_999;

/// Identify which prompt owns one history entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PromptHistoryKind {
    Command,
    Search,
}

/// Describe whether recall should filter by the typed prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PromptHistoryScope {
    MatchingPrefix,
    Full,
}

/// Session-local history storage for both editable prompts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct PromptHistory {
    command: PromptHistoryState,
    search: PromptHistoryState,
}

/// History entries plus one active traversal session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PromptHistoryState {
    entries: VecDeque<String>,
    traversal: Option<PromptHistoryTraversal>,
}

/// State that keeps one recall session anchored to its original typed text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptHistoryTraversal {
    original_input: String,
    scope: PromptHistoryScope,
    index: Option<usize>,
}

impl PromptHistory {
    /// Create one empty prompt-history store.
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Record one submitted prompt entry for the selected prompt kind.
    pub(super) fn record(&mut self, kind: PromptHistoryKind, input: &str) {
        self.state_mut(kind).record_entry(input);
    }

    /// Clear one prompt's active traversal session.
    pub(super) fn reset_traversal(&mut self, kind: PromptHistoryKind) {
        self.state_mut(kind).traversal = None;
    }

    /// Recall one older entry for the selected prompt kind.
    pub(super) fn previous(
        &mut self,
        kind: PromptHistoryKind,
        current_input: &str,
        scope: PromptHistoryScope,
    ) -> Option<String> {
        self.state_mut(kind).previous_entry(current_input, scope)
    }

    /// Recall one newer entry for the selected prompt kind.
    pub(super) fn next(
        &mut self,
        kind: PromptHistoryKind,
        current_input: &str,
        scope: PromptHistoryScope,
    ) -> Option<String> {
        self.state_mut(kind).next_entry(current_input, scope)
    }

    /// Return the mutable history bucket for one prompt kind.
    fn state_mut(&mut self, kind: PromptHistoryKind) -> &mut PromptHistoryState {
        match kind {
            PromptHistoryKind::Command => &mut self.command,
            PromptHistoryKind::Search => &mut self.search,
        }
    }
}

impl PromptHistoryState {
    /// Record one submitted entry while enforcing deduplication and the history cap.
    fn record_entry(&mut self, input: &str) {
        if input.is_empty() || self.entries.back().is_some_and(|entry| entry == input) {
            self.traversal = None;
            return;
        }
        if self.entries.len() == PROMPT_HISTORY_LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back(input.to_string());
        self.traversal = None;
    }

    /// Recall one older entry according to the requested traversal scope.
    fn previous_entry(&mut self, current_input: &str, scope: PromptHistoryScope) -> Option<String> {
        self.ensure_traversal(current_input, scope);
        let (start, original_input, traversal_scope) = {
            let traversal = self.traversal.as_ref().expect("traversal initialized");
            (
                traversal.index.unwrap_or(self.entries.len()),
                traversal.original_input.clone(),
                traversal.scope,
            )
        };

        // Walk backward from the current position so repeated Up/Ctrl+P visits older entries first.
        let next_index = (0..start).rev().find(|&index| {
            Self::matches_scope(&self.entries[index], &original_input, traversal_scope)
        })?;
        self.traversal
            .as_mut()
            .expect("traversal initialized")
            .index = Some(next_index);
        Some(self.entries[next_index].clone())
    }

    /// Recall one newer entry according to the requested traversal scope.
    fn next_entry(&mut self, current_input: &str, scope: PromptHistoryScope) -> Option<String> {
        self.ensure_traversal(current_input, scope);
        let (current_index, original_input, traversal_scope) = {
            let traversal = self.traversal.as_ref().expect("traversal initialized");
            (
                traversal.index,
                traversal.original_input.clone(),
                traversal.scope,
            )
        };
        let current_index = current_index?;

        // Walk forward from the current match and fall back to the original typed text at the end.
        if let Some(next_index) = ((current_index + 1)..self.entries.len()).find(|&index| {
            Self::matches_scope(&self.entries[index], &original_input, traversal_scope)
        }) {
            self.traversal
                .as_mut()
                .expect("traversal initialized")
                .index = Some(next_index);
            return Some(self.entries[next_index].clone());
        }

        self.traversal
            .as_mut()
            .expect("traversal initialized")
            .index = None;
        Some(original_input)
    }

    /// Ensure one traversal session exists for the current recall scope.
    fn ensure_traversal(&mut self, current_input: &str, scope: PromptHistoryScope) {
        if self
            .traversal
            .as_ref()
            .is_some_and(|traversal| traversal.scope == scope)
        {
            return;
        }

        // Scope switches keep the first typed prompt text so arrows can still match that prefix.
        let original_input = self.traversal.as_ref().map_or_else(
            || current_input.to_string(),
            |traversal| traversal.original_input.clone(),
        );
        self.traversal = Some(PromptHistoryTraversal {
            original_input,
            scope,
            index: None,
        });
    }

    /// Return whether one stored entry matches the active traversal scope.
    fn matches_scope(entry: &str, original_input: &str, scope: PromptHistoryScope) -> bool {
        match scope {
            PromptHistoryScope::MatchingPrefix => entry.starts_with(original_input),
            PromptHistoryScope::Full => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one history store containing the provided command entries.
    fn command_history(entries: &[&str]) -> PromptHistory {
        let mut history = PromptHistory::new();
        for entry in entries {
            history.record(PromptHistoryKind::Command, entry);
        }
        history
    }

    #[test]
    fn record_ignores_empty_and_adjacent_duplicates() {
        let history = command_history(&["", "open", "open", "write", "open"]);

        assert_eq!(
            history.command.entries.iter().collect::<Vec<_>>(),
            vec![
                &"open".to_string(),
                &"write".to_string(),
                &"open".to_string()
            ]
        );
    }

    #[test]
    fn full_history_restores_original_input() {
        let mut history = command_history(&["alpha", "beta", "gamma"]);

        assert_eq!(
            history.previous(PromptHistoryKind::Command, "pref", PromptHistoryScope::Full),
            Some("gamma".to_string())
        );
        assert_eq!(
            history.previous(
                PromptHistoryKind::Command,
                "ignored",
                PromptHistoryScope::Full
            ),
            Some("beta".to_string())
        );
        assert_eq!(
            history.next(
                PromptHistoryKind::Command,
                "ignored",
                PromptHistoryScope::Full
            ),
            Some("gamma".to_string())
        );
        assert_eq!(
            history.next(
                PromptHistoryKind::Command,
                "ignored",
                PromptHistoryScope::Full
            ),
            Some("pref".to_string())
        );
    }

    #[test]
    fn prefix_history_filters_entries() {
        let mut history = command_history(&["quit", "rename", "reload", "write"]);

        assert_eq!(
            history.previous(
                PromptHistoryKind::Command,
                "re",
                PromptHistoryScope::MatchingPrefix
            ),
            Some("reload".to_string())
        );
        assert_eq!(
            history.previous(
                PromptHistoryKind::Command,
                "ignored",
                PromptHistoryScope::MatchingPrefix
            ),
            Some("rename".to_string())
        );
        assert_eq!(
            history.next(
                PromptHistoryKind::Command,
                "ignored",
                PromptHistoryScope::MatchingPrefix
            ),
            Some("reload".to_string())
        );
        assert_eq!(
            history.next(
                PromptHistoryKind::Command,
                "ignored",
                PromptHistoryScope::MatchingPrefix
            ),
            Some("re".to_string())
        );
    }

    #[test]
    fn scope_switch_preserves_original_prefix() {
        let mut history = command_history(&["alpha", "rename", "reload"]);

        assert_eq!(
            history.previous(
                PromptHistoryKind::Command,
                "re",
                PromptHistoryScope::MatchingPrefix
            ),
            Some("reload".to_string())
        );
        assert_eq!(
            history.previous(
                PromptHistoryKind::Command,
                "ignored",
                PromptHistoryScope::Full
            ),
            Some("reload".to_string())
        );
        assert_eq!(
            history.previous(
                PromptHistoryKind::Command,
                "ignored",
                PromptHistoryScope::MatchingPrefix
            ),
            Some("reload".to_string())
        );
    }

    #[test]
    fn history_limit_discards_oldest_entries() {
        let mut history = PromptHistory::new();
        for index in 0..=PROMPT_HISTORY_LIMIT {
            history.record(PromptHistoryKind::Search, &format!("entry-{index}"));
        }

        assert_eq!(history.search.entries.len(), PROMPT_HISTORY_LIMIT);
        assert_eq!(history.search.entries.front(), Some(&"entry-1".to_string()));
        assert_eq!(
            history.search.entries.back(),
            Some(&format!("entry-{PROMPT_HISTORY_LIMIT}"))
        );
    }
}
