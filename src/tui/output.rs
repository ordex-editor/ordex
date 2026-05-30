//! Terminal output primitives, styling helpers, and batched frame writes.

use super::{RESET_CURSOR_COLOR, SYNC_UPDATE_BEGIN, SYNC_UPDATE_END, Terminal};
use crate::editor_state::VisibleMatchRole;
use crate::lsp::LspDiagnosticSeverity;
use crate::syntax::{SyntaxClass, SyntaxModifier};
use crate::themes::{ColorCapability, Theme, ThemeColor, ThemeStyle};
use std::fmt;
use std::fmt::Write as _;
use std::io::{self, Write};

/// Buffered terminal commands that should be emitted as one frame.
///
/// Batching writes through this type avoids the flickering that happens when
/// the terminal redraw is flushed in smaller steps.
pub(crate) struct TerminalBatch {
    output: String,
}

/// Combined terminal styling for one rendered cell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CellStyle {
    /// Semantic syntax class for this cell.
    syntax_class: Option<SyntaxClass>,
    /// Semantic syntax modifier for this cell.
    syntax_modifier: Option<SyntaxModifier>,
    /// Whether selection highlighting is active for this cell.
    selected: bool,
    /// Whether this cell belongs to the current logical cursor line.
    current_line: bool,
    /// Whether this cell participates in visible passive match highlighting.
    match_role: Option<VisibleMatchRole>,
    /// Whether this cell participates in visible search-result highlighting.
    search_match: bool,
    /// Whether this cell is covered by a rendered diagnostic range.
    diagnostic_severity: Option<LspDiagnosticSeverity>,
}

/// Terminal cursor-shape variants supported by Ordex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorShape {
    /// A steady block cursor for Normal and Visual modes.
    Block,
    /// A steady beam cursor for Insert-style input modes.
    Beam,
}

/// Which side of the terminal color state an escape should update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorLayer {
    /// Update the foreground color.
    Foreground,
    /// Update the background color.
    Background,
}

impl CellStyle {
    /// Build one combined cell style from syntax and selection state.
    pub(crate) fn from_syntax(
        syntax_class: Option<SyntaxClass>,
        syntax_modifier: Option<SyntaxModifier>,
        selected: bool,
        current_line: bool,
        match_role: Option<VisibleMatchRole>,
        search_match: bool,
        diagnostic_severity: Option<LspDiagnosticSeverity>,
    ) -> Self {
        Self {
            syntax_class,
            syntax_modifier,
            selected,
            current_line,
            match_role,
            search_match,
            diagnostic_severity,
        }
    }
}

/// Push one styled text run, emitting only the necessary ANSI transitions.
pub(crate) fn push_styled_text(
    output: &mut String,
    active_style: &mut Option<CellStyle>,
    next_style: CellStyle,
    theme: &Theme,
    color_capability: ColorCapability,
    text: &str,
) {
    if text.is_empty() {
        return;
    }
    if *active_style != Some(next_style) {
        output.push_str(termion::style::Reset.as_ref());
        style_escape(output, next_style, theme, color_capability);
        *active_style = Some(next_style);
    }
    output.push_str(text);
}

/// Push one styled character, emitting only the necessary ANSI transitions.
pub(crate) fn push_styled_char(
    output: &mut String,
    active_style: &mut Option<CellStyle>,
    next_style: CellStyle,
    theme: &Theme,
    color_capability: ColorCapability,
    ch: char,
) {
    let mut encoded = [0; 4];
    push_styled_text(
        output,
        active_style,
        next_style,
        theme,
        color_capability,
        ch.encode_utf8(&mut encoded),
    );
}

/// Finish one styled output run by resetting the terminal when needed.
pub(crate) fn finish_styled_output(output: &mut String, active_style: &mut Option<CellStyle>) {
    if active_style.is_some() {
        output.push_str(termion::style::Reset.as_ref());
        *active_style = None;
    }
}

/// Append the ANSI escape sequence for one combined cell style.
fn style_escape(
    output: &mut String,
    style: CellStyle,
    theme: &Theme,
    color_capability: ColorCapability,
) {
    // Content cells always inherit the theme background so both visible text and
    // trailing spaces render on the active palette instead of the terminal default.
    let mut combined = theme.background_style();
    if style.current_line {
        combined = combined.overlay(theme.current_line_style());
    }
    if let Some(class) = style.syntax_class {
        combined = combined.overlay(theme.syntax_style(class, style.syntax_modifier));
    }
    if style.search_match {
        combined = combined.overlay(theme.search_match_style());
    }
    if matches!(style.match_role, Some(VisibleMatchRole::Target)) && !style.selected {
        combined = combined.overlay(theme.passive_match_style());
    }
    if style.selected {
        combined = combined.overlay(theme.selection_style());
    }
    if let Some(severity) = style.diagnostic_severity {
        combined = combined.overlay(theme.diagnostic_inline_style(severity));
    }
    if matches!(
        style.match_role,
        Some(VisibleMatchRole::Source | VisibleMatchRole::Target)
    ) {
        combined = combined.overlay(ThemeStyle {
            bold: true,
            ..ThemeStyle::default()
        });
    }
    push_theme_style_escape(output, combined, color_capability);
}

/// Append one themed style escape sequence.
fn push_theme_style_escape(
    output: &mut String,
    style: ThemeStyle,
    color_capability: ColorCapability,
) {
    // Emit colors before text attributes so each reset rebuilds the full style.
    if let Some(bg) = style.bg {
        push_color_escape(output, ColorLayer::Background, bg, color_capability);
    }
    if let Some(fg) = style.fg {
        push_color_escape(output, ColorLayer::Foreground, fg, color_capability);
    }
    if style.bold {
        output.push_str(termion::style::Bold.as_ref());
    }
    if style.underline {
        output.push_str(termion::style::Underline.as_ref());
    }
    if style.undercurl {
        output.push_str("\u{1b}[4:3m");
    }
    if style.reverse {
        output.push_str("\u{1b}[7m");
    }
}

/// Append one foreground or background color escape sequence.
fn push_color_escape(
    output: &mut String,
    layer: ColorLayer,
    color: ThemeColor,
    color_capability: ColorCapability,
) {
    // Map the theme color into the active terminal palette before writing the escape.
    match (layer, color_capability) {
        (ColorLayer::Foreground, ColorCapability::Ansi256) => {
            write!(
                output,
                "{}",
                termion::color::AnsiValue(color.ansi256_index()).fg_string()
            )
        }
        (ColorLayer::Background, ColorCapability::Ansi256) => {
            write!(
                output,
                "{}",
                termion::color::AnsiValue(color.ansi256_index()).bg_string()
            )
        }
        (ColorLayer::Foreground, ColorCapability::TrueColor) => write!(
            output,
            "{}",
            termion::color::Rgb(color.red, color.green, color.blue).fg_string()
        ),
        (ColorLayer::Background, ColorCapability::TrueColor) => write!(
            output,
            "{}",
            termion::color::Rgb(color.red, color.green, color.blue).bg_string()
        ),
    }
    .expect("writing an ANSI color escape into a String cannot fail");
}

/// Append one terminal cursor-color escape sequence.
fn push_cursor_color_escape(
    output: &mut String,
    color: ThemeColor,
    color_capability: ColorCapability,
) {
    let color = match color_capability {
        ColorCapability::Ansi256 => color.ansi256_rgb(),
        ColorCapability::TrueColor => color,
    };
    write!(
        output,
        "\u{1b}]12;#{:02x}{:02x}{:02x}\u{7}",
        color.red, color.green, color.blue
    )
    .expect("writing a cursor-color escape into a String cannot fail");
}

/// Append one OSC 2 terminal window-title escape sequence.
fn push_window_title_escape(output: &mut String, title: &str) {
    let safe_title = sanitize_terminal_title_text(title);
    write!(output, "\u{1b}]2;{safe_title}\u{7}")
        .expect("writing a window-title escape into a String cannot fail");
}

/// Return one terminal-title payload with control characters removed.
fn sanitize_terminal_title_text(title: &str) -> String {
    let mut sanitized = String::with_capacity(title.len());
    // OSC payloads must not carry control bytes because they can terminate or
    // corrupt the control sequence in terminals that parse title updates.
    for ch in title.chars() {
        if ch.is_control() {
            sanitized.push(' ');
            continue;
        }
        sanitized.push(ch);
    }
    sanitized
}

impl CursorShape {
    /// Return the ANSI escape sequence for this cursor shape.
    pub(crate) fn escape_sequence(self) -> &'static str {
        match self {
            Self::Block => "\u{1b}[2 q",
            Self::Beam => "\u{1b}[6 q",
        }
    }
}

impl TerminalBatch {
    /// Create an empty terminal batch.
    pub(crate) fn new() -> Self {
        Self {
            output: String::new(),
        }
    }

    /// Queue a full-screen clear in this batch.
    pub(crate) fn clear_screen(&mut self) {
        write!(self.output, "{}", termion::clear::All)
            .expect("writing a screen clear into a String cannot fail");
    }

    /// Queue text at a specific position (1-indexed).
    pub(crate) fn write_at<T>(&mut self, x: u16, y: u16, text: T)
    where
        T: fmt::Display,
    {
        write!(self.output, "{}{}", termion::cursor::Goto(x, y), text)
            .expect("writing positioned terminal output into a String cannot fail");
    }

    /// Queue styled text at a specific position (1-indexed).
    pub(crate) fn write_styled_at<T>(
        &mut self,
        x: u16,
        y: u16,
        style: ThemeStyle,
        color_capability: ColorCapability,
        text: T,
    ) where
        T: fmt::Display,
    {
        // Apply the style after moving the cursor so the positioned text and its
        // trailing reset form one self-contained terminal segment.
        write!(self.output, "{}", termion::cursor::Goto(x, y))
            .expect("writing a cursor move into a String cannot fail");
        push_theme_style_escape(&mut self.output, style, color_capability);
        write!(self.output, "{}{}", text, termion::style::Reset)
            .expect("writing positioned styled text into a String cannot fail");
    }

    /// Clear from the given cell to the end of the line using one themed style.
    pub(crate) fn clear_to_eol_styled_at(
        &mut self,
        x: u16,
        y: u16,
        style: ThemeStyle,
        color_capability: ColorCapability,
    ) {
        // Rebuild the full style before the clear so erased cells inherit the same palette.
        write!(self.output, "{}", termion::cursor::Goto(x, y))
            .expect("writing a cursor move into a String cannot fail");
        push_theme_style_escape(&mut self.output, style, color_capability);
        write!(
            self.output,
            "{}{}",
            termion::clear::UntilNewline,
            termion::style::Reset
        )
        .expect("writing a styled line clear into a String cannot fail");
    }

    /// Queue a cursor move without writing any text.
    pub(crate) fn goto(&mut self, x: u16, y: u16) {
        write!(self.output, "{}", termion::cursor::Goto(x, y))
            .expect("writing a cursor move into a String cannot fail");
    }

    /// Queue a terminal cursor-shape change in this batch.
    pub(crate) fn set_cursor_shape(&mut self, shape: CursorShape) {
        write!(self.output, "{}", shape.escape_sequence())
            .expect("writing a cursor-shape escape sequence into a String cannot fail");
    }

    /// Queue a terminal cursor-color update in this batch.
    pub(crate) fn set_cursor_color(
        &mut self,
        color: Option<ThemeColor>,
        color_capability: ColorCapability,
    ) {
        if let Some(color) = color {
            push_cursor_color_escape(&mut self.output, color, color_capability);
        } else {
            self.output.push_str(RESET_CURSOR_COLOR);
        }
    }

    /// Queue a terminal window-title update in this batch.
    pub(crate) fn set_window_title(&mut self, title: &str) {
        push_window_title_escape(&mut self.output, title);
    }

    /// Queue a cursor hide command in this batch.
    pub(crate) fn hide_cursor(&mut self) {
        write!(self.output, "{}", termion::cursor::Hide)
            .expect("writing a cursor hide command into a String cannot fail");
    }

    /// Queue a cursor show command in this batch.
    pub(crate) fn show_cursor(&mut self) {
        write!(self.output, "{}", termion::cursor::Show)
            .expect("writing a cursor show command into a String cannot fail");
    }

    /// Borrow the batched terminal frame as bytes for direct terminal writes.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.output.as_bytes()
    }
}

impl Terminal {
    /// Clear the entire screen.
    pub(crate) fn clear_screen(&mut self) -> io::Result<()> {
        let mut batch = TerminalBatch::new();
        batch.clear_screen();
        self.write_batch(&batch)
    }

    /// Emit one fully batched terminal frame with a single write.
    pub(crate) fn write_batch(&mut self, batch: &TerminalBatch) -> io::Result<()> {
        // Synchronized update mode lets supporting terminals present the frame
        // atomically instead of showing intermediate cursor hops or line edits.
        let mut frame = Vec::with_capacity(
            SYNC_UPDATE_BEGIN.len() + batch.as_bytes().len() + SYNC_UPDATE_END.len(),
        );
        frame.extend_from_slice(SYNC_UPDATE_BEGIN.as_bytes());
        frame.extend_from_slice(batch.as_bytes());
        frame.extend_from_slice(SYNC_UPDATE_END.as_bytes());
        self.stdout.write_all(&frame)?;
        self.stdout.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that terminal batches collect positioned output in one buffer.
    #[test]
    fn test_terminal_batch_collects_positioned_output() {
        let mut batch = TerminalBatch::new();
        batch.clear_screen();
        batch.write_at(1, 1, "test");

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        assert!(output.contains("\u{1b}[2J"));
        assert!(output.contains("\u{1b}[1;1Htest"));
    }

    /// Verify that terminal batches can carry cursor-shape escape sequences.
    #[test]
    fn test_terminal_batch_collects_cursor_shape_output() {
        let mut batch = TerminalBatch::new();
        batch.set_cursor_shape(CursorShape::Beam);

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        assert!(output.contains("\u{1b}[6 q"));
    }

    /// Verify that terminal batches collect cursor-color escapes.
    #[test]
    fn test_terminal_batch_collects_cursor_color_output() {
        let mut batch = TerminalBatch::new();
        batch.set_cursor_color(
            Some(ThemeColor {
                red: 0x72,
                green: 0x87,
                blue: 0xfd,
            }),
            ColorCapability::TrueColor,
        );

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        assert!(output.contains("\u{1b}]12;#7287fd\u{7}"));
    }

    /// Verify that ANSI256 cursor colors are quantized before emission.
    #[test]
    fn test_terminal_batch_quantizes_cursor_color_for_ansi256() {
        let mut batch = TerminalBatch::new();
        batch.set_cursor_color(
            Some(ThemeColor {
                red: 0xbc,
                green: 0xc0,
                blue: 0xcc,
            }),
            ColorCapability::Ansi256,
        );

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        assert!(output.contains("\u{1b}]12;#bcbcbc\u{7}"));
    }

    /// Verify that terminal batches collect OSC 2 window-title escapes.
    #[test]
    fn test_terminal_batch_collects_window_title_output() {
        let mut batch = TerminalBatch::new();
        batch.set_window_title("main.rs (~/ordex) - ordex");

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        assert!(output.contains("\u{1b}]2;main.rs (~/ordex) - ordex\u{7}"));
    }

    /// Verify that title control characters are sanitized before OSC emission.
    #[test]
    fn test_terminal_batch_sanitizes_window_title_control_chars() {
        let mut batch = TerminalBatch::new();
        batch.set_window_title("a\u{7}b\u{1b}c\nd");

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        assert!(output.contains("\u{1b}]2;a b c d\u{7}"));
    }

    /// Verify that one active style spans both text and trailing characters.
    #[test]
    fn test_push_styled_text_preserves_active_style() {
        let mut output = String::new();
        let mut active_style = None;
        let theme = crate::themes::find("catppuccin-latte").expect("theme should exist");
        let reset: &str = termion::style::Reset.as_ref();
        let background_escape = termion::color::AnsiValue(
            theme
                .background_style()
                .bg
                .expect("background style should set a background")
                .ansi256_index(),
        )
        .bg_string();
        let foreground_escape = termion::color::AnsiValue(
            theme
                .background_style()
                .fg
                .expect("background style should set a foreground")
                .ansi256_index(),
        )
        .fg_string();

        push_styled_text(
            &mut output,
            &mut active_style,
            CellStyle::default(),
            theme,
            ColorCapability::Ansi256,
            "ab",
        );
        push_styled_char(
            &mut output,
            &mut active_style,
            CellStyle::default(),
            theme,
            ColorCapability::Ansi256,
            'c',
        );
        finish_styled_output(&mut output, &mut active_style);

        // The style transition occurs once for the run, so the trailing
        // character reuses the active style without another reset.
        assert_eq!(output.matches(reset).count(), 2);
        assert!(output.contains(&background_escape));
        assert!(output.contains(&foreground_escape));
        assert!(output.contains("abc"));
        assert!(output.ends_with(reset));
    }
}
