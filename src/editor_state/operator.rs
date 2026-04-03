//! Generic Normal-mode operator handling for `EditorState`.

use super::*;
use crate::navigation::{
    WordStyle, find_next_word_start_with_style, find_prev_word_start_with_style,
    find_word_end_with_style,
};

/// Distinguish the supported Normal-mode operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperatorKind {
    Delete,
    Change,
    Yank,
}

impl OperatorKind {
    /// Return the typed operator character used in pending-prefix labels.
    pub(super) fn key_char(self) -> char {
        match self {
            Self::Delete => 'd',
            Self::Change => 'c',
            Self::Yank => 'y',
        }
    }

    /// Return the verb used in discovery popups and tests.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Delete => "Delete",
            Self::Change => "Change",
            Self::Yank => "Yank",
        }
    }

    /// Return whether this operator should become the source for `.` replay.
    pub(super) fn is_repeatable_change(self) -> bool {
        !matches!(self, Self::Yank)
    }
}

/// Track `i`/`a` text-object prefixes while an operator is pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextObjectPrefix {
    Inner,
    Around,
}

impl TextObjectPrefix {
    /// Return the typed prefix character for pending-prefix labels.
    fn key_char(self) -> char {
        match self {
            Self::Inner => 'i',
            Self::Around => 'a',
        }
    }
}

/// Track a pending `f/F/t/T` style operator target request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingFindTarget {
    FindForward,
    FindBackward,
    TillForward,
    TillBackward,
}

impl PendingFindTarget {
    /// Return the typed motion prefix character for pending-prefix labels.
    fn key_char(self) -> char {
        match self {
            Self::FindForward => 'f',
            Self::FindBackward => 'F',
            Self::TillForward => 't',
            Self::TillBackward => 'T',
        }
    }

    /// Convert the pending marker into the matching motion metadata.
    fn resolve(self, count: usize) -> FindMotion {
        match self {
            Self::FindForward => FindMotion {
                kind: FindMotionKind::Find,
                direction: FindDirection::Forward,
                count,
            },
            Self::FindBackward => FindMotion {
                kind: FindMotionKind::Find,
                direction: FindDirection::Backward,
                count,
            },
            Self::TillForward => FindMotion {
                kind: FindMotionKind::Till,
                direction: FindDirection::Forward,
                count,
            },
            Self::TillBackward => FindMotion {
                kind: FindMotionKind::Till,
                direction: FindDirection::Backward,
                count,
            },
        }
    }
}

/// Describe one resolved motion or text object consumed by an operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OperatorMotion {
    Line,
    WordForward(WordStyle),
    WordBackward(WordStyle),
    WordEnd(WordStyle),
    Find(PendingFindTarget, char),
    MatchDelimiter,
    InnerWord,
    AroundParen,
}

/// Store the keys already typed while waiting for an operator target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingOperator {
    kind: OperatorKind,
    count: Option<usize>,
    motion_count: Option<usize>,
    text_object_prefix: Option<TextObjectPrefix>,
    find_target: Option<PendingFindTarget>,
}

impl PendingOperator {
    /// Build a pending operator from the typed operator key and outer count.
    pub(super) fn new(kind: OperatorKind, count: Option<usize>) -> Self {
        Self {
            kind,
            count,
            motion_count: None,
            text_object_prefix: None,
            find_target: None,
        }
    }

    /// Return the effective operator count after combining outer and motion counts.
    pub(super) fn effective_count(self) -> usize {
        let outer = self.count.unwrap_or(1);
        let inner = self.motion_count.unwrap_or(1);
        outer.saturating_mul(inner).max(1)
    }

    /// Build the currently typed prefix label for the status line.
    pub(super) fn prefix_label(self) -> String {
        let mut label = String::new();
        if let Some(count) = self.count {
            label.push_str(&count.to_string());
        }
        label.push(self.kind.key_char());
        if let Some(motion_count) = self.motion_count {
            label.push_str(&motion_count.to_string());
        }
        if let Some(prefix) = self.text_object_prefix {
            label.push(prefix.key_char());
        }
        if let Some(find) = self.find_target {
            label.push(find.key_char());
        }
        label
    }
}

/// Capture one fully resolved operator command so repeat replay can rerun it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExecutedOperatorCommand {
    pub(super) kind: OperatorKind,
    pub(super) motion: OperatorMotion,
    pub(super) count: usize,
}

/// Return the buffer range and register shape produced by one operator target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedOperatorRange {
    selection: SelectionRange,
    yank_kind: YankKind,
}

impl EditorState {
    /// Enter operator-pending mode for one Normal-mode delete/change/yank command.
    pub(super) fn begin_operator(&mut self, kind: OperatorKind, count: Option<usize>) {
        self.pending_sequence.clear();
        self.pending_sequence_count = None;
        self.pending_sequence_motion_count = None;
        self.pending_find = None;
        self.pending_operator = Some(PendingOperator::new(kind, count));
    }

    /// Return the operator-pending discovery popup, if an operator is active.
    pub(super) fn operator_discovery_popup(&self) -> Option<SequenceDiscoveryPopup> {
        let pending = self.pending_operator?;
        if pending.find_target.is_some() {
            return None;
        }

        let entries = match pending.text_object_prefix {
            Some(TextObjectPrefix::Inner) => vec![SequenceDiscoveryEntry {
                keys: "w".to_string(),
                action: format!("{} inner word", pending.kind.label()),
            }],
            Some(TextObjectPrefix::Around) => vec![SequenceDiscoveryEntry {
                keys: "(".to_string(),
                action: format!("{} around paren", pending.kind.label()),
            }],
            None => vec![
                SequenceDiscoveryEntry {
                    keys: pending.kind.key_char().to_string(),
                    action: format!("{} current line", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "w".to_string(),
                    action: format!("{} word forward", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "e".to_string(),
                    action: format!("{} word end", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "b".to_string(),
                    action: format!("{} word backward", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "W".to_string(),
                    action: format!("{} WORD forward", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "E".to_string(),
                    action: format!("{} WORD end", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "B".to_string(),
                    action: format!("{} WORD backward", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "f".to_string(),
                    action: format!("{} find forward", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "F".to_string(),
                    action: format!("{} find backward", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "t".to_string(),
                    action: format!("{} till forward", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "T".to_string(),
                    action: format!("{} till backward", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "%".to_string(),
                    action: format!("{} matching delimiter", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "iw".to_string(),
                    action: format!("{} inner word", pending.kind.label()),
                },
                SequenceDiscoveryEntry {
                    keys: "a(".to_string(),
                    action: format!("{} around paren", pending.kind.label()),
                },
            ],
        };

        Some(SequenceDiscoveryPopup {
            prefix: pending.prefix_label(),
            entries,
        })
    }

    /// Consume one key while an operator target is pending.
    pub(super) fn handle_pending_operator_key(&mut self, key: Key) -> bool {
        let Some(mut pending) = self.pending_operator else {
            return false;
        };
        if !self.mode.is_normal() {
            self.pending_operator = None;
            return false;
        }
        if matches!(key, Key::Esc) {
            self.pending_operator = None;
            return true;
        }

        if let Some(find) = pending.find_target {
            if let Some(target) = KeyBindings::is_insertable_char(key) {
                self.pending_operator = None;
                self.execute_operator_command(ExecutedOperatorCommand {
                    kind: pending.kind,
                    motion: OperatorMotion::Find(find, target),
                    count: pending.effective_count().min(Self::MAX_COUNT),
                });
                return true;
            }
            self.pending_operator = None;
            return true;
        }

        if pending.text_object_prefix.is_none()
            && let Some(digit) = Self::key_count_digit(key)
            && let Some(next) = Self::append_count_digit(pending.motion_count, digit)
        {
            pending.motion_count = Some(next);
            self.pending_operator = Some(pending);
            return true;
        }

        let reprocess = pending.kind == OperatorKind::Yank;
        let motion = if let Some(prefix) = pending.text_object_prefix {
            self.resolve_text_object_motion(prefix, key)
        } else {
            self.resolve_pending_operator_motion(&mut pending, key)
        };

        match motion {
            Some(Some(motion)) => {
                self.pending_operator = None;
                self.execute_operator_command(ExecutedOperatorCommand {
                    kind: pending.kind,
                    motion,
                    count: pending.effective_count().min(Self::MAX_COUNT),
                });
                true
            }
            Some(None) => {
                self.pending_operator = Some(pending);
                true
            }
            None => {
                self.pending_operator = None;
                !reprocess
            }
        }
    }

    /// Resolve one direct motion key while an operator is pending.
    fn resolve_pending_operator_motion(
        &self,
        pending: &mut PendingOperator,
        key: Key,
    ) -> Option<Option<OperatorMotion>> {
        match key {
            Key::Char('i') => {
                pending.text_object_prefix = Some(TextObjectPrefix::Inner);
                Some(None)
            }
            Key::Char('a') => {
                pending.text_object_prefix = Some(TextObjectPrefix::Around);
                Some(None)
            }
            Key::Char(c) if c == pending.kind.key_char() => Some(Some(OperatorMotion::Line)),
            Key::Char('w') => Some(Some(OperatorMotion::WordForward(WordStyle::Small))),
            Key::Char('e') => Some(Some(OperatorMotion::WordEnd(WordStyle::Small))),
            Key::Char('b') => Some(Some(OperatorMotion::WordBackward(WordStyle::Small))),
            Key::Char('W') => Some(Some(OperatorMotion::WordForward(WordStyle::Big))),
            Key::Char('E') => Some(Some(OperatorMotion::WordEnd(WordStyle::Big))),
            Key::Char('B') => Some(Some(OperatorMotion::WordBackward(WordStyle::Big))),
            Key::Char('f') => {
                pending.find_target = Some(PendingFindTarget::FindForward);
                Some(None)
            }
            Key::Char('F') => {
                pending.find_target = Some(PendingFindTarget::FindBackward);
                Some(None)
            }
            Key::Char('t') => {
                pending.find_target = Some(PendingFindTarget::TillForward);
                Some(None)
            }
            Key::Char('T') => {
                pending.find_target = Some(PendingFindTarget::TillBackward);
                Some(None)
            }
            Key::Char('%') => Some(Some(OperatorMotion::MatchDelimiter)),
            _ => None,
        }
    }

    /// Resolve one trailing text-object key while an operator is pending.
    fn resolve_text_object_motion(
        &self,
        prefix: TextObjectPrefix,
        key: Key,
    ) -> Option<Option<OperatorMotion>> {
        match (prefix, key) {
            (TextObjectPrefix::Inner, Key::Char('w')) => Some(Some(OperatorMotion::InnerWord)),
            (TextObjectPrefix::Around, Key::Char('(')) => Some(Some(OperatorMotion::AroundParen)),
            _ => None,
        }
    }

    /// Execute one resolved operator command and update repeat/history state.
    pub(super) fn execute_operator_command(&mut self, command: ExecutedOperatorCommand) {
        let undo_depth_before = self.undo_stack.len();
        match command.kind {
            OperatorKind::Delete => self.apply_delete_operator(&command),
            OperatorKind::Change => self.apply_change_operator(&command),
            OperatorKind::Yank => self.apply_yank_operator(&command),
        }

        self.capture_repeat_after_operator(command, undo_depth_before);
        if self.mode.is_normal() {
            self.finish_counted_normal_action();
        } else {
            // Change operators may enter Insert mode with the cursor positioned at
            // an insertion site past the current line end, so avoid normal-mode
            // clamping and only refresh the visible viewport state here.
            self.viewport
                .ensure_cursor_visible(&self.cursor, &self.buffer);
            self.sync_visible_match_for_viewport();
        }
    }

    /// Apply one delete operator motion inside a single undoable transaction.
    fn apply_delete_operator(&mut self, command: &ExecutedOperatorCommand) {
        if matches!(command.motion, OperatorMotion::InnerWord) {
            self.delete_inner_word_count(command.count);
            return;
        }
        if matches!(command.motion, OperatorMotion::AroundParen) {
            self.delete_around_paren_count(command.count);
            return;
        }

        self.with_history_transaction(|editor| {
            let Some(range) = editor.resolve_operator_range(command) else {
                return;
            };

            // Delete operators copy into the unnamed register before removing text
            // so later paste commands can reuse the deleted payload.
            editor.delete_range_into_yank_buffer(range.selection, range.yank_kind);
            editor.cursor = Cursor::from_char_index(&editor.buffer, range.selection.start);
        });
    }

    /// Apply one change operator by deleting text and entering Insert mode.
    fn apply_change_operator(&mut self, command: &ExecutedOperatorCommand) {
        if matches!(command.motion, OperatorMotion::InnerWord) {
            self.change_inner_word_count(command.count);
            return;
        }

        let Some(range) = self.resolve_operator_range(command) else {
            return;
        };

        // Change commands keep the delete and following insert session inside one
        // undo transaction so `.` and undo replay the full edit coherently.
        self.begin_history_transaction();
        self.delete_range_into_yank_buffer(range.selection, range.yank_kind);
        self.cursor = Cursor::from_char_index(&self.buffer, range.selection.start);
        if self
            .active_undo
            .as_ref()
            .is_some_and(|active| !active.edits.is_empty())
        {
            self.enter_insert_mode();
        } else {
            self.finish_history_transaction();
        }
    }

    /// Apply one yank operator without changing the current buffer contents.
    fn apply_yank_operator(&mut self, command: &ExecutedOperatorCommand) {
        let Some(range) = self.resolve_operator_range(command) else {
            return;
        };
        self.store_yank_range(range.selection, range.yank_kind);
    }

    /// Resolve one operator command into the corresponding buffer selection.
    fn resolve_operator_range(
        &mut self,
        command: &ExecutedOperatorCommand,
    ) -> Option<ResolvedOperatorRange> {
        match &command.motion {
            OperatorMotion::Line => Some(ResolvedOperatorRange {
                selection: self.current_line_range(command.count),
                yank_kind: YankKind::Line,
            }),
            OperatorMotion::WordForward(style) => {
                self.resolve_forward_word_range(*style, command.count)
            }
            OperatorMotion::WordBackward(style) => {
                self.resolve_backward_word_range(*style, command.count)
            }
            OperatorMotion::WordEnd(style) => self.resolve_word_end_range(*style, command.count),
            OperatorMotion::Find(find, target) => {
                self.resolve_find_range(*find, *target, command.count)
            }
            OperatorMotion::MatchDelimiter => self.resolve_match_delimiter_range(),
            OperatorMotion::InnerWord => self.resolve_inner_word_range(),
            OperatorMotion::AroundParen => self.resolve_around_paren_range(),
        }
    }

    /// Resolve a forward word motion into a characterwise operator range.
    fn resolve_forward_word_range(
        &self,
        style: WordStyle,
        count: usize,
    ) -> Option<ResolvedOperatorRange> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let mut target = cursor_idx;

        // Walk the requested number of word boundaries using the same helpers that
        // ordinary motions use so operator ranges stay consistent with navigation.
        for _ in 0..count.max(1) {
            let next = find_next_word_start_with_style(&self.buffer, target, style);
            if next == target {
                break;
            }
            target = next;
        }
        if target == cursor_idx {
            return None;
        }

        Some(ResolvedOperatorRange {
            selection: SelectionRange {
                start: cursor_idx,
                end: target,
            },
            yank_kind: YankKind::Character,
        })
    }

    /// Resolve a backward word motion into a characterwise operator range.
    fn resolve_backward_word_range(
        &self,
        style: WordStyle,
        count: usize,
    ) -> Option<ResolvedOperatorRange> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let mut target = cursor_idx;

        // Backward operators act over the text traversed by the motion, which is
        // the span from the resolved target up to the original cursor position.
        for _ in 0..count.max(1) {
            let next = find_prev_word_start_with_style(&self.buffer, target, style);
            if next == target {
                break;
            }
            target = next;
        }
        if target == cursor_idx {
            return None;
        }

        Some(ResolvedOperatorRange {
            selection: SelectionRange {
                start: target,
                end: cursor_idx,
            },
            yank_kind: YankKind::Character,
        })
    }

    /// Resolve an end-of-word motion into an inclusive characterwise operator range.
    fn resolve_word_end_range(
        &self,
        style: WordStyle,
        count: usize,
    ) -> Option<ResolvedOperatorRange> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let mut target = cursor_idx;

        // `e`/`E` are inclusive motions, so the resulting operator range extends
        // one character beyond the resolved end position.
        for _ in 0..count.max(1) {
            let next = find_word_end_with_style(&self.buffer, target, style);
            if next == target && self.buffer.char_at(next).is_none() {
                break;
            }
            target = next;
        }

        let end = target.saturating_add(1).min(self.buffer.chars_count());
        if end <= cursor_idx {
            return None;
        }
        Some(ResolvedOperatorRange {
            selection: SelectionRange {
                start: cursor_idx,
                end,
            },
            yank_kind: YankKind::Character,
        })
    }

    /// Resolve an `f/F/t/T` operator target into a characterwise range.
    fn resolve_find_range(
        &self,
        pending: PendingFindTarget,
        target: char,
        count: usize,
    ) -> Option<ResolvedOperatorRange> {
        let motion = pending.resolve(count.max(1));
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let mut search_from = cursor_idx;
        let mut target_idx = None;

        // Counted search motions reuse the exact same single-line scan helper as
        // ordinary find/till navigation so repeated targets resolve identically.
        for _ in 0..motion.count {
            let next = self.find_char_on_current_line(search_from, motion.direction, target)?;
            target_idx = Some(next);
            search_from = next;
        }

        let target_idx = target_idx?;
        let selection = match pending {
            PendingFindTarget::FindForward => SelectionRange {
                start: cursor_idx,
                end: target_idx.saturating_add(1),
            },
            PendingFindTarget::TillForward => SelectionRange {
                start: cursor_idx,
                end: target_idx,
            },
            PendingFindTarget::FindBackward => SelectionRange {
                start: target_idx,
                end: cursor_idx.saturating_add(1),
            },
            PendingFindTarget::TillBackward => SelectionRange {
                start: target_idx.saturating_add(1),
                end: cursor_idx.saturating_add(1),
            },
        };
        if selection.end <= selection.start {
            return None;
        }

        Some(ResolvedOperatorRange {
            selection,
            yank_kind: YankKind::Character,
        })
    }

    /// Resolve `%` into the inclusive span between matching delimiters.
    fn resolve_match_delimiter_range(&mut self) -> Option<ResolvedOperatorRange> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let target_idx = matching::matching_target_start(self)?;
        let start = cursor_idx.min(target_idx);
        let end = cursor_idx.max(target_idx).saturating_add(1);
        Some(ResolvedOperatorRange {
            selection: SelectionRange { start, end },
            yank_kind: YankKind::Character,
        })
    }

    /// Resolve `iw` into the nearest inner-word characterwise span.
    fn resolve_inner_word_range(&self) -> Option<ResolvedOperatorRange> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let (start, end) = find_inner_word_span(&self.buffer, cursor_idx)?;
        Some(ResolvedOperatorRange {
            selection: SelectionRange { start, end },
            yank_kind: YankKind::Character,
        })
    }

    /// Resolve `a(` into the smallest enclosing parenthesized span.
    fn resolve_around_paren_range(&self) -> Option<ResolvedOperatorRange> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let (start, end) = find_around_paren_span(&self.buffer, cursor_idx)?;
        Some(ResolvedOperatorRange {
            selection: SelectionRange { start, end },
            yank_kind: YankKind::Character,
        })
    }
}
