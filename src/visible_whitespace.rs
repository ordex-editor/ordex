//! Visible-whitespace settings and rendering helpers.

use crate::display_columns;

/// Marker shown for one visible tab cell.
const TAB_MARKER: char = '▸';
/// Marker shown for one visible non-breaking space.
const NBSP_MARKER: char = '⍽';
/// Marker shown for one visible trailing ASCII space.
const TRAILING_SPACE_MARKER: char = '·';

/// One selectable visible-whitespace kind from configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisibleWhitespaceKind {
    Nbsp,
    Tab,
    TrailingSpace,
}

impl VisibleWhitespaceKind {
    /// Parse one config token into a visible-whitespace kind.
    pub(crate) fn parse(token: &str) -> Option<Self> {
        match token {
            "nbsp" => Some(Self::Nbsp),
            "tab" => Some(Self::Tab),
            "trailing-space" => Some(Self::TrailingSpace),
            _ => None,
        }
    }
}

/// Runtime toggle set for visible whitespace markers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct VisibleWhitespace {
    pub(crate) nbsp: bool,
    pub(crate) tab: bool,
    pub(crate) trailing_space: bool,
}

impl VisibleWhitespace {
    /// Return toggles with all markers disabled.
    pub(crate) const fn none() -> Self {
        Self {
            nbsp: false,
            tab: false,
            trailing_space: false,
        }
    }

    /// Return toggles with all marker kinds enabled.
    pub(crate) const fn all() -> Self {
        Self {
            nbsp: true,
            tab: true,
            trailing_space: true,
        }
    }

    /// Enable one marker kind in this toggle set.
    pub(crate) fn enable(&mut self, kind: VisibleWhitespaceKind) {
        match kind {
            VisibleWhitespaceKind::Nbsp => self.nbsp = true,
            VisibleWhitespaceKind::Tab => self.tab = true,
            VisibleWhitespaceKind::TrailingSpace => self.trailing_space = true,
        }
    }

    /// Return whether at least one marker kind is enabled.
    ///
    /// Returns `true` when any visible-whitespace marker is active, and `false`
    /// when rendering should keep plain source glyphs.
    pub(crate) const fn any_enabled(self) -> bool {
        self.nbsp || self.tab || self.trailing_space
    }
}

/// Render one visible display window with optional whitespace markers.
///
/// `start_display` is the first display column to include in the output. Display
/// columns count expanded tab width; they are not buffer character indices.
pub(crate) fn expand_display_window_with_visible_whitespace(
    chars: impl Iterator<Item = char> + Clone,
    start_display: usize,
    max_display: usize,
    tab_width: usize,
    markers: VisibleWhitespace,
) -> String {
    if !markers.any_enabled() {
        return display_columns::expand_display_window_chars(
            chars,
            start_display,
            max_display,
            tab_width,
        );
    }
    if max_display == 0 {
        return String::new();
    }

    // Trailing-space markers only apply to ASCII spaces after the final
    // non-space character on the logical line.
    let trailing_space_start = if markers.trailing_space {
        trailing_ascii_space_start(chars.clone())
    } else {
        0
    };
    let mut output = String::new();
    let mut current_display = 0;
    let end_display = start_display.saturating_add(max_display);

    // Keep one pass over source characters so display clipping and tab expansion
    // stay consistent with viewport math.
    for (column, ch) in chars.enumerate() {
        let next_display = display_columns::advance_display_column(current_display, ch, tab_width);
        if next_display <= start_display {
            current_display = next_display;
            continue;
        }
        if current_display >= end_display {
            break;
        }

        // Compute the overlap between the character's display cells and the
        // visible window. Tabs can overlap by more than one cell.
        let visible_start = current_display.max(start_display);
        let visible_end = next_display.min(end_display);
        let visible_cells = visible_end.saturating_sub(visible_start);

        if ch == '\t' {
            push_tab_cells(
                &mut output,
                current_display,
                visible_start,
                visible_cells,
                markers.tab,
            );
        } else if ch == '\u{00A0}' && markers.nbsp && visible_cells > 0 {
            output.push(NBSP_MARKER);
        } else if ch == ' '
            && markers.trailing_space
            && column >= trailing_space_start
            && visible_cells > 0
        {
            output.push(TRAILING_SPACE_MARKER);
        } else if visible_cells > 0 {
            output.push(ch);
        }

        current_display = next_display;
        if current_display >= end_display {
            break;
        }
    }

    output
}

/// Return the first buffer column that belongs to trailing ASCII spaces.
fn trailing_ascii_space_start(chars: impl Iterator<Item = char>) -> usize {
    let mut len = 0usize;
    let mut trailing_run = 0usize;

    // Track the trailing ASCII-space suffix length in one forward scan.
    for ch in chars {
        len += 1;
        if ch == ' ' {
            trailing_run += 1;
        } else {
            trailing_run = 0;
        }
    }

    len.saturating_sub(trailing_run)
}

/// Append visible cells for one tab character.
///
/// `tab_start_display` is the display column where the tab begins.
/// `visible_start_display` is the first tab cell that falls inside the window.
/// `visible_cells` is how many tab cells overlap the visible window.
/// `show_marker` is `true` to replace the first visible tab cell with the tab
/// marker glyph, and `false` to render only spaces.
fn push_tab_cells(
    output: &mut String,
    tab_start_display: usize,
    visible_start_display: usize,
    visible_cells: usize,
    show_marker: bool,
) {
    if visible_cells == 0 {
        return;
    }

    // Tabs keep their expanded width. The marker appears only when the first
    // tab cell is visible in the current window.
    if show_marker && visible_start_display == tab_start_display {
        output.push(TAB_MARKER);
        if visible_cells > 1 {
            output.push_str(&" ".repeat(visible_cells - 1));
        }
    } else {
        output.push_str(&" ".repeat(visible_cells));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify default marker settings keep whitespace glyphs hidden.
    #[test]
    fn keeps_default_whitespace_rendering() {
        let rendered = expand_display_window_with_visible_whitespace(
            "a\tb\u{00A0}c  ".chars(),
            0,
            32,
            8,
            VisibleWhitespace::none(),
        );
        assert_eq!(rendered, "a       b\u{00A0}c  ");
    }

    /// Verify all marker kinds can render in one mixed line.
    #[test]
    fn shows_all_marker_kinds() {
        let rendered = expand_display_window_with_visible_whitespace(
            "a\tb\u{00A0}c  ".chars(),
            0,
            32,
            8,
            VisibleWhitespace::all(),
        );
        assert_eq!(rendered, "a▸      b⍽c··");
    }

    /// Verify tabs keep alignment and skip the marker when clipped mid-tab.
    #[test]
    fn preserves_tab_width_when_clipped_inside_tab() {
        let rendered = expand_display_window_with_visible_whitespace(
            "a\tb".chars(),
            2,
            3,
            8,
            VisibleWhitespace {
                tab: true,
                ..VisibleWhitespace::none()
            },
        );
        assert_eq!(rendered, "   ");
    }

    /// Verify trailing-space markers only target ASCII spaces at line end.
    #[test]
    fn marks_only_trailing_ascii_spaces() {
        let rendered = expand_display_window_with_visible_whitespace(
            "x y z  ".chars(),
            0,
            32,
            8,
            VisibleWhitespace {
                trailing_space: true,
                ..VisibleWhitespace::none()
            },
        );
        assert_eq!(rendered, "x y z··");
    }

    /// Verify non-breaking spaces can be highlighted without enabling other kinds.
    #[test]
    fn marks_only_nbsp_when_requested() {
        let rendered = expand_display_window_with_visible_whitespace(
            "a\u{00A0} b".chars(),
            0,
            32,
            8,
            VisibleWhitespace {
                nbsp: true,
                ..VisibleWhitespace::none()
            },
        );
        assert_eq!(rendered, "a⍽ b");
    }
}
