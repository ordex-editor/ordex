//! Generic Normal-mode operator handling for `EditorState`.

use super::auto_insert::IndentDirection;
use super::*;
use crate::clipboard::ClipboardRegister;
use crate::keybindings::OperatorBinding;
use crate::navigation::{
    WordStyle, find_around_delimiter_span, find_around_quote_span, find_around_word_span,
    find_inner_delimiter_span, find_inner_quote_span, find_inner_word_span_with_style,
    find_next_paragraph_line, find_next_word_start_with_style, find_prev_paragraph_line,
    find_prev_word_start_with_style, find_word_end_with_style,
};

/// Distinguish the supported Normal-mode operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperatorKind {
    Delete,
    Change,
    Yank,
    Reindent,
    Indent,
    Dedent,
}

impl OperatorKind {
    /// Return the typed operator character used in pending-prefix labels.
    pub(super) fn key_char(self) -> char {
        match self {
            Self::Delete => 'd',
            Self::Change => 'c',
            Self::Yank => 'y',
            Self::Reindent => '=',
            Self::Indent => '>',
            Self::Dedent => '<',
        }
    }

    /// Return the verb used in discovery popups and tests.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Delete => "Delete",
            Self::Change => "Change",
            Self::Yank => "Yank",
            Self::Reindent => "Reindent",
            Self::Indent => "Indent",
            Self::Dedent => "Dedent",
        }
    }

    /// Return whether this operator should become the source for `.` replay.
    pub(super) fn is_repeatable_change(self) -> bool {
        !matches!(self, Self::Yank)
    }
}

/// Track the intermediate operator prefix key already typed while awaiting a
/// second key to complete the operator motion.
///
/// `Inner` and `Around` await a text-object key; `GotoLine` awaits the second
/// key of a two-key goto-line motion (e.g. `gg`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperatorPrefix {
    /// `i` prefix: awaiting a text-object kind key.
    Inner,
    /// `a` prefix: awaiting a text-object kind key.
    Around,
    /// `g` prefix (or its remap): awaiting the second key of a goto-line motion.
    GotoLine,
}

impl OperatorPrefix {
    /// Return the typed prefix character for pending-prefix labels.
    fn key_char(self) -> char {
        match self {
            Self::Inner => 'i',
            Self::Around => 'a',
            Self::GotoLine => 'g',
        }
    }
}

/// The text-object flavour carried by `OperatorPrefix::Inner` and `OperatorPrefix::Around`.
///
/// Extracted as a separate type so that `TextObjectSpec` and the text-object
/// range resolver can work with an already-narrowed prefix value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextObjectPrefix {
    Inner,
    Around,
}

impl TextObjectPrefix {
    /// Return the label fragment used in discovery popups.
    fn label(self) -> &'static str {
        match self {
            Self::Inner => "inner",
            Self::Around => "around",
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

/// Distinguish the delimiter families supported by generic text objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DelimiterTextObject {
    Paren,
    Bracket,
    Brace,
    /// Angle bracket pair `<` / `>` (balanced, depth-tracked).
    AngleBracket,
    /// A symmetric quote delimiter: `"`, `'`, or `` ` ``.
    ///
    /// The stored char is the quote character used for both the open and close
    /// sides. Because quotes are symmetric they require a separate scan algorithm
    /// that does not track nesting depth.
    Quote(char),
}

impl DelimiterTextObject {
    /// Return the pair of delimiters that define this text object.
    ///
    /// For `Quote`, both sides of the pair carry the same character.
    fn delimiters(self) -> (char, char) {
        match self {
            Self::Paren => ('(', ')'),
            Self::Bracket => ('[', ']'),
            Self::Brace => ('{', '}'),
            Self::AngleBracket => ('<', '>'),
            Self::Quote(quote_char) => (quote_char, quote_char),
        }
    }

    /// Return the label fragment used in discovery popups.
    fn label(self) -> &'static str {
        match self {
            Self::Paren => "paren",
            Self::Bracket => "bracket",
            Self::Brace => "brace",
            Self::AngleBracket => "angle bracket",
            Self::Quote('"') => "double quote",
            Self::Quote('\'') => "single quote",
            Self::Quote('`') => "backtick",
            // Unreachable for any other quote char, but provide a safe fallback.
            Self::Quote(_) => "quote",
        }
    }

    /// Return the delimiter object selected by `key`, if any.
    fn from_key(key: Key) -> Option<Self> {
        match key {
            // Opening and closing delimiters point at the same surrounding object
            // so users can keep their usual Vim muscle memory for either spelling.
            Key::Char('(') | Key::Char(')') => Some(Self::Paren),
            // `b` is the Vim alias for `)` in text-object context.
            Key::Char('b') => Some(Self::Paren),
            Key::Char('[') | Key::Char(']') => Some(Self::Bracket),
            Key::Char('{') | Key::Char('}') => Some(Self::Brace),
            // `B` is the Vim alias for `}` in text-object context.
            Key::Char('B') => Some(Self::Brace),
            Key::Char('<') | Key::Char('>') => Some(Self::AngleBracket),
            Key::Char('"') => Some(Self::Quote('"')),
            Key::Char('\'') => Some(Self::Quote('\'')),
            Key::Char('`') => Some(Self::Quote('`')),
            _ => None,
        }
    }

    /// Return whether this delimiter uses symmetric (quote-style) matching.
    ///
    /// Returns `true` for `Quote` variants, where open and close are the same
    /// character and depth-tracking is not applicable. Returns `false` for all
    /// bracket-style delimiters that use distinct open/close characters.
    pub(super) fn is_quote(self) -> bool {
        matches!(self, Self::Quote(_))
    }
}

/// Describe one generic text object selected after an `i`/`a` prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextObjectKind {
    Word(WordStyle),
    Delimiter(DelimiterTextObject),
}

impl TextObjectKind {
    /// Return the label fragment used in discovery popups.
    fn label(self) -> &'static str {
        match self {
            Self::Word(WordStyle::Small) => "word",
            Self::Word(WordStyle::Big) => "WORD",
            Self::Delimiter(delimiter) => delimiter.label(),
        }
    }
}

/// One fully specified text object consumed by an operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TextObjectSpec {
    prefix: TextObjectPrefix,
    kind: TextObjectKind,
}

impl TextObjectSpec {
    /// Return the full human-readable description for discovery popups.
    fn label(self) -> String {
        format!("{} {}", self.prefix.label(), self.kind.label())
    }
}

/// Describe one resolved motion or text object consumed by an operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OperatorMotion {
    Line,
    WordForward(WordStyle),
    WordBackward(WordStyle),
    WordEnd(WordStyle),
    ParagraphForward,
    ParagraphBackward,
    Find(PendingFindTarget, char),
    MatchDelimiter,
    TextObject(TextObjectSpec),
    /// Move to a target line and apply operator from the current line through it.
    ///
    /// The `Option<usize>` carries an explicit 1-indexed line number when the user
    /// provided a count (e.g. `d5gg`); `None` means "go to the very first line".
    LineToFirst(Option<usize>),
    /// Move to a target line and apply operator from the current line through it.
    ///
    /// The `Option<usize>` carries an explicit 1-indexed line number when the user
    /// provided a count (e.g. `d5G`); `None` means "go to the very last line".
    LineToLast(Option<usize>),
    /// Apply operator from the cursor through the end of the current line.
    LineEnd,
    /// Apply operator from the start of the current line through the cursor.
    LineStart,
    /// Apply operator from the first non-blank character of the current line through the cursor.
    FirstNonBlank,
}

/// Describe how one pending operator key should be handled.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OperatorKeyResolution {
    /// The typed key completed the operator and resolved to this motion.
    Execute(OperatorMotion),
    /// The typed key extends the pending prefix, so wait for another key.
    Pending,
    /// The typed key does not belong to the operator sequence.
    Reject,
}

/// Store the keys already typed while waiting for an operator target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingOperator {
    kind: OperatorKind,
    /// Key that started this operator-pending session.
    ///
    /// Remapped operators use this to recognize repeated linewise commands such
    /// as `ll` for change-current-line and to display the active prefix exactly
    /// as the user typed it.
    trigger: KeyInput,
    register: Option<ClipboardRegister>,
    count: Option<usize>,
    motion_count: Option<usize>,
    /// Intermediate prefix key that has been typed but not yet resolved.
    ///
    /// `Inner`/`Around` wait for a text-object key; `GotoLine` waits for the
    /// second key of a two-key goto-line motion (e.g. `gg`).
    prefix: Option<OperatorPrefix>,
    find_target: Option<PendingFindTarget>,
}

impl PendingOperator {
    /// Build a pending operator from the typed operator key and outer count.
    ///
    /// `trigger` stores the exact key that entered operator-pending mode. When
    /// absent, the built-in Vim-style default key for `kind` is used.
    pub(super) fn new(
        kind: OperatorKind,
        trigger: Option<KeyInput>,
        count: Option<usize>,
        register: Option<ClipboardRegister>,
    ) -> Self {
        Self {
            kind,
            trigger: trigger.unwrap_or_else(|| KeyInput::Char(kind.key_char())),
            register,
            count,
            motion_count: None,
            prefix: None,
            find_target: None,
        }
    }

    /// Return the effective operator count after combining outer and motion counts.
    pub(super) fn effective_count(&self) -> usize {
        let outer = self.count.unwrap_or(1);
        let inner = self.motion_count.unwrap_or(1);
        outer.saturating_mul(inner).max(1)
    }

    /// Build the currently typed prefix label for the status line.
    pub(super) fn prefix_label(&self) -> String {
        let mut label = String::new();
        if let Some(count) = self.count {
            label.push_str(&count.to_string());
        }
        if let Some(register) = self.register {
            label.push('"');
            label.push(register.key_char());
        }
        label.push_str(&self.trigger.label());
        if let Some(motion_count) = self.motion_count {
            label.push_str(&motion_count.to_string());
        }
        if let Some(prefix) = self.prefix {
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
    pub(super) register: Option<ClipboardRegister>,
}

/// Return the buffer range and register shape produced by one operator target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedOperatorRange {
    selection: SelectionRange,
    yank_kind: YankKind,
}

impl EditorState {
    /// Enter operator-pending mode for one Normal-mode delete/change/yank command.
    ///
    /// `trigger` records the exact bound key that started the operator so later
    /// repeated-key linewise resolution follows custom remaps as well.
    pub(super) fn begin_operator(
        &mut self,
        kind: OperatorKind,
        trigger: Option<KeyInput>,
        count: Option<usize>,
        register: Option<ClipboardRegister>,
    ) {
        self.pending_sequence.clear();
        self.pending_sequence_count = None;
        self.pending_sequence_motion_count = None;
        self.pending_find = None;
        self.pending_operator = Some(PendingOperator::new(kind, trigger, count, register));
    }

    /// Return the operator-pending discovery popup, if an operator is active.
    pub(super) fn operator_discovery_popup(&self) -> Option<SequenceDiscoveryPopup> {
        let pending = self.pending_operator.as_ref()?;
        if pending.find_target.is_some() {
            return None;
        }

        let entries = match pending.prefix {
            Some(OperatorPrefix::Inner) => {
                self.operator_text_object_popup_entries(pending.kind, TextObjectPrefix::Inner)
            }
            Some(OperatorPrefix::Around) => {
                self.operator_text_object_popup_entries(pending.kind, TextObjectPrefix::Around)
            }
            Some(OperatorPrefix::GotoLine) => self.operator_goto_line_popup_entries(pending.kind),
            None => self.operator_motion_popup_entries(pending),
        };

        Some(SequenceDiscoveryPopup {
            prefix: pending.prefix_label(),
            entries,
        })
    }

    /// Build the top-level discovery entries for one pending operator.
    fn operator_motion_popup_entries(
        &self,
        pending: &PendingOperator,
    ) -> Vec<SequenceDiscoveryEntry> {
        let kind = pending.kind;
        let mut entries = vec![SequenceDiscoveryEntry {
            keys: pending.trigger.label(),
            action: format!("{} current line", kind.label()),
        }];

        entries.extend(self.operator_action_entries(
            OperatorBinding::WordForward,
            &format!("{} word forward", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::WordEnd,
            &format!("{} word end", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::WordBackward,
            &format!("{} word backward", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::WordForwardBig,
            &format!("{} WORD forward", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::WordEndBig,
            &format!("{} WORD end", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::WordBackwardBig,
            &format!("{} WORD backward", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::ParagraphBackward,
            &format!("{} paragraph backward", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::ParagraphForward,
            &format!("{} paragraph forward", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::FindForward,
            &format!("{} find forward", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::FindBackward,
            &format!("{} find backward", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::TillForward,
            &format!("{} till forward", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::TillBackward,
            &format!("{} till backward", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::MatchDelimiter,
            &format!("{} matching delimiter", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::TextObjectInner,
            &format!("{} inner text object", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::TextObjectAround,
            &format!("{} around text object", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::LineToLast,
            &format!("{} to last line", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::LineToFirst,
            &format!("{} to first line", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::LineEnd,
            &format!("{} to line end", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::LineStart,
            &format!("{} to line start", kind.label()),
        ));
        entries.extend(self.operator_action_entries(
            OperatorBinding::FirstNonBlank,
            &format!("{} to first non-blank", kind.label()),
        ));
        entries
    }

    /// Build the goto-line continuation entries shown after a goto-line prefix key.
    ///
    /// The completion key is whatever is bound to `LineToFirst`, so the popup
    /// stays correct when the user remaps the goto-line prefix in their config.
    fn operator_goto_line_popup_entries(&self, kind: OperatorKind) -> Vec<SequenceDiscoveryEntry> {
        self.operator_action_entries(
            OperatorBinding::LineToFirst,
            &format!("{} to first line", kind.label()),
        )
    }

    /// Build the text-object continuation entries after `i` or `a`.
    fn operator_text_object_popup_entries(
        &self,
        kind: OperatorKind,
        prefix: TextObjectPrefix,
    ) -> Vec<SequenceDiscoveryEntry> {
        let mut entries = self
            .keybindings
            .keys_for_operator_binding(OperatorBinding::WordForward)
            .into_iter()
            .map(|key| SequenceDiscoveryEntry {
                keys: key.label(),
                action: format!(
                    "{} {}",
                    kind.label(),
                    TextObjectSpec {
                        prefix,
                        kind: TextObjectKind::Word(WordStyle::Small),
                    }
                    .label()
                ),
            })
            .collect::<Vec<_>>();
        entries.extend(
            self.keybindings
                .keys_for_operator_binding(OperatorBinding::WordForwardBig)
                .into_iter()
                .map(|key| SequenceDiscoveryEntry {
                    keys: key.label(),
                    action: format!(
                        "{} {}",
                        kind.label(),
                        TextObjectSpec {
                            prefix,
                            kind: TextObjectKind::Word(WordStyle::Big),
                        }
                        .label()
                    ),
                }),
        );
        entries.extend(
            [
                DelimiterTextObject::Paren,
                DelimiterTextObject::Bracket,
                DelimiterTextObject::Brace,
            ]
            .into_iter()
            .map(|delimiter| {
                let (open, _) = delimiter.delimiters();
                SequenceDiscoveryEntry {
                    keys: open.to_string(),
                    action: format!(
                        "{} {}",
                        kind.label(),
                        TextObjectSpec {
                            prefix,
                            kind: TextObjectKind::Delimiter(delimiter),
                        }
                        .label()
                    ),
                }
            }),
        );
        entries
    }

    /// Return discovery entries for every operator-pending key bound to one meaning.
    fn operator_action_entries(
        &self,
        binding: OperatorBinding,
        label: &str,
    ) -> Vec<SequenceDiscoveryEntry> {
        self.keybindings
            .keys_for_operator_binding(binding)
            .into_iter()
            .map(|key| SequenceDiscoveryEntry {
                keys: key.label(),
                action: label.to_string(),
            })
            .collect()
    }

    /// Consume one key while an operator target is pending.
    ///
    /// Returns `true` when the key belongs to the operator-pending flow, whether
    /// that means continuing the prefix, executing the operator, or cancelling it.
    /// Returns `false` only when no operator was pending or a yank prefix rejected
    /// the key so normal dispatch should reprocess it.
    pub(super) fn handle_pending_operator_key(&mut self, key: Key) -> bool {
        let Some(mut pending) = self.pending_operator.take() else {
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
                    register: pending.register,
                });
                return true;
            }
            self.pending_operator = None;
            return true;
        }

        if pending.prefix.is_none()
            && let Some(digit) = Self::key_count_digit(key)
            && let Some(next) = Self::append_count_digit(pending.motion_count, digit)
        {
            pending.motion_count = Some(next);
            self.pending_operator = Some(pending);
            return true;
        }

        let reprocess = pending.kind == OperatorKind::Yank;
        let motion_count = pending.motion_count;
        let resolution = match pending.prefix {
            Some(OperatorPrefix::Inner) => {
                self.resolve_text_object_motion(TextObjectPrefix::Inner, key)
            }
            Some(OperatorPrefix::Around) => {
                self.resolve_text_object_motion(TextObjectPrefix::Around, key)
            }
            Some(OperatorPrefix::GotoLine) => self.resolve_goto_line_motion(key, motion_count),
            None => self.resolve_pending_operator_motion(&mut pending, key),
        };

        match resolution {
            OperatorKeyResolution::Execute(motion) => {
                self.pending_operator = None;
                self.execute_operator_command(ExecutedOperatorCommand {
                    kind: pending.kind,
                    motion,
                    count: pending.effective_count().min(Self::MAX_COUNT),
                    register: pending.register,
                });
                true
            }
            OperatorKeyResolution::Pending => {
                self.pending_operator = Some(pending);
                true
            }
            OperatorKeyResolution::Reject => {
                self.pending_operator = None;
                !reprocess
            }
        }
    }

    /// Consume one key while a visual-mode text-object prefix (`i`/`a`) is pending.
    ///
    /// Returns `true` when the key belongs to this flow (consumed, regardless of
    /// whether a span was found). Returns `false` when no visual text-object prefix
    /// was pending.
    pub(super) fn handle_pending_visual_text_object_key(&mut self, key: Key) -> bool {
        let Some(prefix) = self.pending_visual_text_object.take() else {
            return false;
        };

        if matches!(key, Key::Esc) {
            // Cancellation: leave visual mode active with its current selection.
            return true;
        }

        let resolution = self.resolve_text_object_motion(prefix, key);
        if let OperatorKeyResolution::Execute(OperatorMotion::TextObject(spec)) = resolution {
            let cursor_idx = self.cursor.to_char_index(&self.buffer);
            if let Some(ResolvedOperatorRange { selection, .. }) =
                Self::resolve_text_object_range_in_buffer(
                    &self.buffer,
                    &self.syntax,
                    cursor_idx,
                    spec,
                )
            {
                // Move the anchor to the start and cursor to the last char of the span.
                // Visual selection is inclusive on both ends, so the cursor sits on
                // `end - 1` (the last character inside the span).
                self.visual_anchor = Some(Cursor::from_char_index(&self.buffer, selection.start));
                let last_char = selection.end.saturating_sub(1);
                self.cursor = Cursor::from_char_index(&self.buffer, last_char);
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            // If no span was found, leave cursor/anchor unchanged (no-op, stay in visual).
        }
        // Any non-pending resolution (Execute or Reject) consumes the key.
        true
    }

    ///
    /// Returns `Pending` when the key extends the operator prefix, `Execute` when
    /// it resolves to a complete operator motion, and `Reject` when the key does
    /// not belong to the current operator sequence.
    fn resolve_pending_operator_motion(
        &self,
        pending: &mut PendingOperator,
        key: Key,
    ) -> OperatorKeyResolution {
        if KeyInput::from(key) == pending.trigger {
            return OperatorKeyResolution::Execute(OperatorMotion::Line);
        }

        self.keybindings
            .get_operator_binding(key)
            .map(|binding| Self::resolve_operator_motion_binding(binding, pending))
            .unwrap_or(OperatorKeyResolution::Reject)
    }

    /// Resolve the completion key of a goto-line operator motion sequence.
    ///
    /// The completion key is whatever is currently bound to `OperatorBinding::LineToFirst`
    /// (default: `g`). Any other key rejects the sequence. The `motion_count` carries
    /// the explicit 1-indexed line number typed between the operator and the prefix key
    /// (e.g. `d5gg`); `None` means "go to the very first line".
    fn resolve_goto_line_motion(
        &self,
        key: Key,
        motion_count: Option<usize>,
    ) -> OperatorKeyResolution {
        if self.keybindings.get_operator_binding(key) == Some(OperatorBinding::LineToFirst) {
            return OperatorKeyResolution::Execute(OperatorMotion::LineToFirst(motion_count));
        }
        OperatorKeyResolution::Reject
    }

    /// Resolve one trailing text-object key while an operator is pending.
    fn resolve_text_object_motion(
        &self,
        prefix: TextObjectPrefix,
        key: Key,
    ) -> OperatorKeyResolution {
        if let Some(delimiter) = DelimiterTextObject::from_key(key) {
            return OperatorKeyResolution::Execute(OperatorMotion::TextObject(TextObjectSpec {
                prefix,
                kind: TextObjectKind::Delimiter(delimiter),
            }));
        }

        self.keybindings
            .get_operator_binding(key)
            .and_then(|binding| Self::resolve_text_object_binding(binding, prefix))
            .unwrap_or(OperatorKeyResolution::Reject)
    }

    /// Resolve one operator binding while no prefix is active.
    fn resolve_operator_motion_binding(
        binding: OperatorBinding,
        pending: &mut PendingOperator,
    ) -> OperatorKeyResolution {
        match binding {
            OperatorBinding::TextObjectInner => {
                pending.prefix = Some(OperatorPrefix::Inner);
                OperatorKeyResolution::Pending
            }
            OperatorBinding::TextObjectAround => {
                pending.prefix = Some(OperatorPrefix::Around);
                OperatorKeyResolution::Pending
            }
            OperatorBinding::LineToFirst => {
                pending.prefix = Some(OperatorPrefix::GotoLine);
                OperatorKeyResolution::Pending
            }
            OperatorBinding::WordForward => {
                OperatorKeyResolution::Execute(OperatorMotion::WordForward(WordStyle::Small))
            }
            OperatorBinding::WordForwardBig => {
                OperatorKeyResolution::Execute(OperatorMotion::WordForward(WordStyle::Big))
            }
            OperatorBinding::WordEnd => {
                OperatorKeyResolution::Execute(OperatorMotion::WordEnd(WordStyle::Small))
            }
            OperatorBinding::WordEndBig => {
                OperatorKeyResolution::Execute(OperatorMotion::WordEnd(WordStyle::Big))
            }
            OperatorBinding::WordBackward => {
                OperatorKeyResolution::Execute(OperatorMotion::WordBackward(WordStyle::Small))
            }
            OperatorBinding::WordBackwardBig => {
                OperatorKeyResolution::Execute(OperatorMotion::WordBackward(WordStyle::Big))
            }
            OperatorBinding::ParagraphForward => {
                OperatorKeyResolution::Execute(OperatorMotion::ParagraphForward)
            }
            OperatorBinding::ParagraphBackward => {
                OperatorKeyResolution::Execute(OperatorMotion::ParagraphBackward)
            }
            OperatorBinding::FindForward => {
                pending.find_target = Some(PendingFindTarget::FindForward);
                OperatorKeyResolution::Pending
            }
            OperatorBinding::FindBackward => {
                pending.find_target = Some(PendingFindTarget::FindBackward);
                OperatorKeyResolution::Pending
            }
            OperatorBinding::TillForward => {
                pending.find_target = Some(PendingFindTarget::TillForward);
                OperatorKeyResolution::Pending
            }
            OperatorBinding::TillBackward => {
                pending.find_target = Some(PendingFindTarget::TillBackward);
                OperatorKeyResolution::Pending
            }
            OperatorBinding::MatchDelimiter => {
                OperatorKeyResolution::Execute(OperatorMotion::MatchDelimiter)
            }
            OperatorBinding::LineToLast => {
                OperatorKeyResolution::Execute(OperatorMotion::LineToLast(pending.motion_count))
            }
            OperatorBinding::LineEnd => OperatorKeyResolution::Execute(OperatorMotion::LineEnd),
            OperatorBinding::LineStart => OperatorKeyResolution::Execute(OperatorMotion::LineStart),
            OperatorBinding::FirstNonBlank => {
                OperatorKeyResolution::Execute(OperatorMotion::FirstNonBlank)
            }
        }
    }

    /// Resolve one operator binding after an `i` or `a` text-object prefix.
    fn resolve_text_object_binding(
        binding: OperatorBinding,
        prefix: TextObjectPrefix,
    ) -> Option<OperatorKeyResolution> {
        let kind = match binding {
            OperatorBinding::WordForward => TextObjectKind::Word(WordStyle::Small),
            OperatorBinding::WordForwardBig => TextObjectKind::Word(WordStyle::Big),
            OperatorBinding::WordEnd
            | OperatorBinding::WordEndBig
            | OperatorBinding::WordBackward
            | OperatorBinding::WordBackwardBig
            | OperatorBinding::ParagraphForward
            | OperatorBinding::ParagraphBackward
            | OperatorBinding::FindForward
            | OperatorBinding::FindBackward
            | OperatorBinding::TillForward
            | OperatorBinding::TillBackward
            | OperatorBinding::MatchDelimiter
            | OperatorBinding::TextObjectInner
            | OperatorBinding::TextObjectAround
            | OperatorBinding::LineToFirst
            | OperatorBinding::LineToLast
            | OperatorBinding::LineEnd
            | OperatorBinding::LineStart
            | OperatorBinding::FirstNonBlank => return None,
        };

        Some(OperatorKeyResolution::Execute(OperatorMotion::TextObject(
            TextObjectSpec { prefix, kind },
        )))
    }

    /// Execute one resolved operator command and update repeat/history state.
    pub(super) fn execute_operator_command(&mut self, command: ExecutedOperatorCommand) {
        let undo_depth_before = self.undo_stack.len();
        let selection_source = match command.kind {
            OperatorKind::Delete => {
                self.apply_delete_operator(&command);
                None
            }
            OperatorKind::Change => {
                self.apply_change_operator(&command);
                None
            }
            OperatorKind::Yank => {
                self.apply_yank_operator(&command);
                None
            }
            OperatorKind::Reindent => self.apply_reindent_operator(&command),
            OperatorKind::Indent => self.apply_indent_operator(&command),
            OperatorKind::Dedent => self.apply_dedent_operator(&command),
        };

        self.capture_repeat_after_operator(command, selection_source, undo_depth_before);
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
        if let OperatorMotion::TextObject(spec) = command.motion {
            self.apply_delete_text_object(spec, command.count, command.register);
            return;
        }

        self.with_history_transaction(|editor| {
            let Some(range) = editor.resolve_operator_range(command) else {
                return;
            };

            // Delete operators copy into the unnamed register before removing text
            // so later paste commands can reuse the deleted payload.
            editor.delete_range_into_yank_buffer(range.selection, range.yank_kind);
            editor.queue_clipboard_write_from_yank_buffer(command.register);
            editor.cursor = Cursor::from_char_index(&editor.buffer, range.selection.start);
        });
    }

    /// Apply one change operator by deleting text and entering Insert mode.
    fn apply_change_operator(&mut self, command: &ExecutedOperatorCommand) {
        if let OperatorMotion::TextObject(spec) = command.motion {
            self.apply_change_text_object(spec, command.count, command.register);
            return;
        }

        let Some(range) = self.resolve_operator_range(command) else {
            return;
        };

        if matches!(&command.motion, OperatorMotion::Line) {
            self.begin_history_transaction();
            self.apply_line_change(range.selection, command.register);
            return;
        }

        // Change commands keep the delete and following insert session inside one
        // undo transaction so `.` and undo replay the full edit coherently.
        self.begin_history_transaction();
        self.delete_range_into_yank_buffer(range.selection, range.yank_kind);
        self.queue_clipboard_write_from_yank_buffer(command.register);
        self.cursor = Cursor::from_char_index(&self.buffer, range.selection.start);
        if self.active_transaction_has_edits() {
            self.enter_insert_mode();
        } else {
            self.finish_history_transaction();
        }
    }

    /// Delete one linewise change target while keeping one editable line in place.
    fn apply_line_change(
        &mut self,
        selection: SelectionRange,
        register: Option<ClipboardRegister>,
    ) {
        // Capture the target line index before deletion so it remains valid
        // regardless of how the buffer shrinks afterward.
        let line_idx = self.buffer.char_to_line(selection.start);
        self.delete_range_into_yank_buffer(selection, YankKind::Line);
        self.queue_clipboard_write_from_yank_buffer(register);
        // Always insert the blank line slot so the indent prefix has a line to
        // land on and following content stays on separate lines.
        self.insert_buffer_text(selection.start, "\n");
        self.cursor = Cursor::from_char_index(&self.buffer, selection.start);
        // Re-indent the blank replacement line using the same auto-indent
        // algorithm as `o`/`O`/Enter so the cursor lands at the correct
        // indentation level for the current context.
        self.apply_indent_prefix_to_line(selection.start, line_idx);
        self.enter_insert_mode();
    }

    /// Apply one yank operator without changing the current buffer contents.
    fn apply_yank_operator(&mut self, command: &ExecutedOperatorCommand) {
        if let OperatorMotion::TextObject(spec) = command.motion {
            self.apply_yank_text_object(spec, command.count, command.register);
            return;
        }

        let Some(range) = self.resolve_operator_range(command) else {
            return;
        };
        self.store_yank_range(range.selection, range.yank_kind);
        self.queue_clipboard_write_from_yank_buffer(command.register);
    }

    /// Apply one indent operator by reindenting the resolved line range.
    fn apply_reindent_operator(
        &mut self,
        command: &ExecutedOperatorCommand,
    ) -> Option<SelectionRepeatCommand> {
        let Some(range) = self.resolve_operator_range(command) else {
            return None;
        };
        let line_count = self.indentation_line_count(range.selection);
        self.reindent_selection(range.selection);
        Some(SelectionRepeatCommand {
            action: SelectionRepeatAction::Reindent,
            target: SelectionRepeatTarget::Lines { line_count },
            register: None,
        })
    }

    /// Apply one manual indent operator over the resolved line range.
    fn apply_indent_operator(
        &mut self,
        command: &ExecutedOperatorCommand,
    ) -> Option<SelectionRepeatCommand> {
        let Some(range) = self.resolve_operator_range(command) else {
            return None;
        };
        let line_count = self.indentation_line_count(range.selection);
        self.adjust_selection_indentation(range.selection, IndentDirection::Indent);
        Some(SelectionRepeatCommand {
            action: SelectionRepeatAction::Indent,
            target: SelectionRepeatTarget::Lines { line_count },
            register: None,
        })
    }

    /// Apply one manual dedent operator over the resolved line range.
    fn apply_dedent_operator(
        &mut self,
        command: &ExecutedOperatorCommand,
    ) -> Option<SelectionRepeatCommand> {
        let Some(range) = self.resolve_operator_range(command) else {
            return None;
        };
        let line_count = self.indentation_line_count(range.selection);
        self.adjust_selection_indentation(range.selection, IndentDirection::Dedent);
        Some(SelectionRepeatCommand {
            action: SelectionRepeatAction::Dedent,
            target: SelectionRepeatTarget::Lines { line_count },
            register: None,
        })
    }

    /// Return whether the active undo transaction already recorded buffer edits.
    fn active_transaction_has_edits(&self) -> bool {
        // Change operators only enter Insert mode after the delete phase produced
        // at least one concrete history edit, so no-op changes stay in Normal mode.
        self.active_undo
            .as_ref()
            .is_some_and(|active| !active.edits.is_empty())
    }

    /// Apply one counted delete over a generic text object.
    fn apply_delete_text_object(
        &mut self,
        spec: TextObjectSpec,
        count: usize,
        register: Option<ClipboardRegister>,
    ) {
        self.with_history_transaction(|editor| {
            let Some((first_start, deleted_text)) = editor.delete_text_object_count(spec, count)
            else {
                return;
            };
            editor.yank_buffer = Some(YankBuffer {
                text: deleted_text,
                kind: YankKind::Character,
            });
            editor.queue_clipboard_write_from_yank_buffer(register);
            editor.cursor = Cursor::from_char_index(&editor.buffer, first_start);
        });
    }

    /// Apply one counted change over a generic text object and enter Insert mode on success.
    fn apply_change_text_object(
        &mut self,
        spec: TextObjectSpec,
        count: usize,
        register: Option<ClipboardRegister>,
    ) {
        self.begin_history_transaction();
        let Some((first_start, deleted_text)) = self.delete_text_object_count(spec, count) else {
            self.finish_history_transaction();
            return;
        };

        self.yank_buffer = Some(YankBuffer {
            text: deleted_text,
            kind: YankKind::Character,
        });
        self.queue_clipboard_write_from_yank_buffer(register);
        self.cursor = Cursor::from_char_index(&self.buffer, first_start);
        if self.active_transaction_has_edits() {
            self.enter_insert_mode();
        } else {
            self.finish_history_transaction();
        }
    }

    /// Apply one counted yank over a generic text object without mutating the buffer.
    fn apply_yank_text_object(
        &mut self,
        spec: TextObjectSpec,
        count: usize,
        register: Option<ClipboardRegister>,
    ) {
        let Some(text) = self.collect_text_object_text(spec, count) else {
            return;
        };
        self.yank_buffer = Some(YankBuffer {
            text,
            kind: YankKind::Character,
        });
        self.queue_clipboard_write_from_yank_buffer(register);
    }

    /// Delete one text object repeatedly, returning the first cursor site and collected text.
    fn delete_text_object_count(
        &mut self,
        spec: TextObjectSpec,
        count: usize,
    ) -> Option<(usize, String)> {
        let mut first_start = None;
        let mut deleted_text = String::new();

        // Re-resolve the text object after each deletion so counted `diw`/`daw`
        // style commands keep operating on the object that moved under the cursor.
        for _ in 0..count.max(1) {
            let Some(range) = self.resolve_text_object_range(spec) else {
                break;
            };
            first_start.get_or_insert(range.selection.start);
            deleted_text.push_str(
                &self
                    .buffer
                    .slice_string(range.selection.start, range.selection.end),
            );
            self.remove_buffer_range(range.selection.start, range.selection.end);
            self.cursor = Cursor::from_char_index(&self.buffer, range.selection.start);
        }

        first_start.map(|start| (start, deleted_text))
    }

    /// Collect the repeated yank payload for one counted text object.
    fn collect_text_object_text(&self, spec: TextObjectSpec, count: usize) -> Option<String> {
        let mut buffer = self.buffer.clone();
        let mut cursor_idx = self.cursor.to_char_index(&self.buffer);
        let mut payload = String::new();

        // Simulate the same deletions on a scratch buffer so counted yanks gather
        // exactly the objects that counted delete/change would consume.
        for _ in 0..count.max(1) {
            let Some(range) =
                Self::resolve_text_object_range_in_buffer(&buffer, &self.syntax, cursor_idx, spec)
            else {
                break;
            };
            payload.push_str(&buffer.slice_string(range.selection.start, range.selection.end));
            buffer.remove(range.selection.start, range.selection.end);
            cursor_idx = range.selection.start.min(buffer.chars_count());
        }

        (!payload.is_empty()).then_some(payload)
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
            OperatorMotion::ParagraphForward => self.resolve_forward_paragraph_range(command.count),
            OperatorMotion::ParagraphBackward => {
                self.resolve_backward_paragraph_range(command.count)
            }
            OperatorMotion::Find(find, target) => {
                self.resolve_find_range(*find, *target, command.count)
            }
            OperatorMotion::MatchDelimiter => self.resolve_match_delimiter_range(),
            OperatorMotion::TextObject(spec) => self.resolve_text_object_range(*spec),
            OperatorMotion::LineToFirst(line) => self.resolve_to_first_line_range(*line),
            OperatorMotion::LineToLast(line) => self.resolve_to_last_line_range(*line),
            OperatorMotion::LineEnd => self.resolve_line_end_range(),
            OperatorMotion::LineStart => self.resolve_line_start_range(),
            OperatorMotion::FirstNonBlank => self.resolve_first_non_blank_range(),
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
                // The motion helper stopped making progress at a buffer edge, so
                // keep the last reachable boundary instead of looping forever.
                break;
            }
            target = next;
        }
        if target == cursor_idx {
            // Returning `None` preserves operator all-or-nothing behavior for
            // no-op motions like `dw` at EOF instead of recording empty edits.
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
            // If the backward motion could not leave the current cursor position,
            // the operator would be empty and should stay a no-op instead.
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

    /// Resolve a forward paragraph motion into a characterwise operator range.
    fn resolve_forward_paragraph_range(&self, count: usize) -> Option<ResolvedOperatorRange> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let mut target_line = self.cursor.line();

        for _ in 0..count.max(1) {
            let next_line = find_next_paragraph_line(&self.buffer, target_line);
            if next_line == target_line {
                // Paragraph search returning the same line means there is no later
                // paragraph boundary to extend the operator range toward.
                break;
            }
            target_line = next_line;
        }

        let target = self.buffer.line_to_char(target_line);
        if target == cursor_idx {
            // A paragraph motion that never moved would produce an empty span, so
            // reject it the same way other no-op operator motions are rejected.
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

    /// Resolve a backward paragraph motion into a characterwise operator range.
    fn resolve_backward_paragraph_range(&self, count: usize) -> Option<ResolvedOperatorRange> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let mut target_line = self.cursor.line();

        for _ in 0..count.max(1) {
            let next_line = find_prev_paragraph_line(&self.buffer, target_line);
            if next_line == target_line {
                // Once the search stops moving, there is no earlier paragraph
                // boundary left to include in the operator range.
                break;
            }
            target_line = next_line;
        }

        let target = self.buffer.line_to_char(target_line);
        if target == cursor_idx {
            // Keep backward paragraph operators all-or-nothing when the cursor is
            // already at the first reachable paragraph boundary.
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
                // Hitting `None` here means the helper already returned the last
                // reachable character at EOF, so another step cannot extend the range.
                break;
            }
            target = next;
        }

        let end = target.saturating_add(1).min(self.buffer.chars_count());
        if end <= cursor_idx {
            // Keep no-op end motions from creating empty delete/change/yank spans.
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

    /// Resolve `$` into a characterwise range from the cursor through the end of the line.
    ///
    /// Returns `None` when the cursor is already past the last character on the line,
    /// preserving the all-or-nothing behavior of operator motions.
    fn resolve_line_end_range(&self) -> Option<ResolvedOperatorRange> {
        let selection = self.cursor_to_line_end_selection()?;
        Some(ResolvedOperatorRange {
            selection,
            yank_kind: YankKind::Character,
        })
    }

    /// Resolve `0` into a characterwise range from the start of the line through the cursor.
    ///
    /// Returns `None` when the cursor is already at column 0, matching Vim behavior where
    /// operators with a no-op motion are rejected rather than producing an empty edit.
    fn resolve_line_start_range(&self) -> Option<ResolvedOperatorRange> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let line_start = self.buffer.line_to_char(self.cursor.line());
        if cursor_idx <= line_start {
            // The cursor sits at the first character of the line, so the motion
            // covers no characters and the operator should be a no-op.
            return None;
        }
        Some(ResolvedOperatorRange {
            selection: SelectionRange {
                start: line_start,
                end: cursor_idx,
            },
            yank_kind: YankKind::Character,
        })
    }

    /// Resolve `^` into a characterwise range between the cursor and the first non-blank.
    ///
    /// The range spans from the lesser index to the greater index so both forward
    /// (cursor after first-non-blank) and backward (cursor before first-non-blank)
    /// cases are handled.  Returns `None` when the cursor is already on the first
    /// non-blank character, keeping operator motion semantics all-or-nothing.
    fn resolve_first_non_blank_range(&self) -> Option<ResolvedOperatorRange> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let line_start = self.buffer.line_to_char(self.cursor.line());
        let line_len = self.buffer.line_len(self.cursor.line());

        // Scan forward from the start of the line to find the first non-blank column.
        let first_non_blank = (line_start..line_start + line_len)
            .find(|&idx| {
                self.buffer
                    .char_at(idx)
                    .is_some_and(|ch| !ch.is_ascii_whitespace())
            })
            .unwrap_or(line_start);

        if cursor_idx == first_non_blank {
            // The cursor already sits on the first non-blank character, so the
            // motion covers nothing and the operator should remain a no-op.
            return None;
        }

        let (start, end) = if cursor_idx > first_non_blank {
            // Cursor is past the first non-blank: delete/yank backward to it.
            (first_non_blank, cursor_idx)
        } else {
            // Cursor is before the first non-blank: delete/yank forward to it.
            (cursor_idx, first_non_blank)
        };
        Some(ResolvedOperatorRange {
            selection: SelectionRange { start, end },
            yank_kind: YankKind::Character,
        })
    }

    /// Resolve one generic text object at the current cursor position.
    fn resolve_text_object_range(&self, spec: TextObjectSpec) -> Option<ResolvedOperatorRange> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        Self::resolve_text_object_range_in_buffer(&self.buffer, &self.syntax, cursor_idx, spec)
    }

    /// Resolve one generic text object against an arbitrary buffer/cursor pair.
    fn resolve_text_object_range_in_buffer(
        buffer: &TextBuffer,
        syntax: &SyntaxEngine,
        cursor_idx: usize,
        spec: TextObjectSpec,
    ) -> Option<ResolvedOperatorRange> {
        let (start, end) = match (spec.prefix, spec.kind) {
            (TextObjectPrefix::Inner, TextObjectKind::Word(style)) => {
                find_inner_word_span_with_style(buffer, cursor_idx, style)?
            }
            (TextObjectPrefix::Around, TextObjectKind::Word(style)) => {
                find_around_word_span(buffer, cursor_idx, style)?
            }
            // Quote delimiters use a symmetric scan that handles escape sequences
            // and does not track nesting depth.
            (TextObjectPrefix::Inner, TextObjectKind::Delimiter(delimiter))
                if delimiter.is_quote() =>
            {
                let (quote_char, _) = delimiter.delimiters();
                find_inner_quote_span(buffer, syntax, cursor_idx, quote_char)?
            }
            (TextObjectPrefix::Around, TextObjectKind::Delimiter(delimiter))
                if delimiter.is_quote() =>
            {
                let (quote_char, _) = delimiter.delimiters();
                find_around_quote_span(buffer, syntax, cursor_idx, quote_char)?
            }
            (TextObjectPrefix::Inner, TextObjectKind::Delimiter(delimiter)) => {
                let (open, close) = delimiter.delimiters();
                find_inner_delimiter_span(buffer, syntax, cursor_idx, open, close)?
            }
            (TextObjectPrefix::Around, TextObjectKind::Delimiter(delimiter)) => {
                let (open, close) = delimiter.delimiters();
                find_around_delimiter_span(buffer, syntax, cursor_idx, open, close)?
            }
        };
        Some(ResolvedOperatorRange {
            selection: SelectionRange { start, end },
            yank_kind: YankKind::Character,
        })
    }

    /// Resolve a `gg` motion into a linewise range from the target line to the current line.
    ///
    /// With no count the target is line 0 (the first line). With a count the target
    /// is the 1-indexed line number given by the count (`5gg` → line 5, stored as
    /// 0-indexed line 4). Returns `None` when the cursor is already on or above the
    /// target line so the operator stays a no-op in that case.
    fn resolve_to_first_line_range(&self, line: Option<usize>) -> Option<ResolvedOperatorRange> {
        let current_line = self.cursor.line();
        // No explicit count means "go to the first line" (0-indexed line 0).
        // An explicit count is a 1-indexed line number; saturating_sub converts it.
        let target_line = line.map_or(0, |n| n.saturating_sub(1));
        let line_count = self.buffer.lines_count();
        if line_count == 0 {
            return None;
        }
        let target_line = target_line.min(line_count.saturating_sub(1));
        // The motion goes upward; if the cursor is already at or above the target
        // there is nothing to delete.
        if current_line <= target_line {
            return None;
        }
        // The range spans from the start of target_line through the end of current_line.
        let start = self.buffer.line_to_char(target_line);
        let end_line_exclusive = current_line.saturating_add(1);
        let end = if end_line_exclusive < line_count {
            self.buffer.line_to_char(end_line_exclusive)
        } else {
            self.buffer.chars_count()
        };
        Some(ResolvedOperatorRange {
            selection: SelectionRange { start, end },
            yank_kind: YankKind::Line,
        })
    }

    /// Resolve a `G` motion into a linewise range from the current line to the target line.
    ///
    /// With no count the target is the last line. With a count the target is the
    /// 1-indexed line number given by the count (`5G` → line 5, stored as 0-indexed
    /// line 4). Returns `None` when the cursor is already at or beyond the target
    /// line so the operator stays a no-op in that case.
    fn resolve_to_last_line_range(&self, line: Option<usize>) -> Option<ResolvedOperatorRange> {
        let current_line = self.cursor.line();
        let last_line = self.buffer.lines_count().saturating_sub(1);
        // No explicit count means "go to the last line"; an explicit count is 1-indexed.
        let target_line = line.map_or(last_line, |n| n.saturating_sub(1).min(last_line));
        // The motion goes downward; if the cursor is already at or past the target
        // there is nothing to delete.
        if current_line >= target_line {
            return None;
        }
        // The range spans from the start of current_line through the end of target_line.
        let start = self.buffer.line_to_char(current_line);
        let end_line_exclusive = target_line.saturating_add(1);
        let end = if end_line_exclusive < self.buffer.lines_count() {
            self.buffer.line_to_char(end_line_exclusive)
        } else {
            self.buffer.chars_count()
        };
        Some(ResolvedOperatorRange {
            selection: SelectionRange { start, end },
            yank_kind: YankKind::Line,
        })
    }
}
