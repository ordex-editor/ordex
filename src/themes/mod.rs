//! Built-in editor themes and terminal color capability helpers.
//!
//! Theme values in this module are adapted from upstream theme palettes so Ordex
//! can ship a curated bundled set without adding parser or theme dependencies.

mod bogster;
mod catppuccin_frappe;
mod catppuccin_latte;
mod catppuccin_macchiato;
mod catppuccin_mocha;
mod gruvbox;
mod kanagawa;
mod nord;
mod onedark;
mod tokyonight;

use crate::lsp::LspDiagnosticSeverity;
use crate::syntax::{SyntaxClass, SyntaxModifier};

/// The default bundled theme name.
pub(crate) const DEFAULT_THEME_NAME: &str = "bogster";
const SEARCH_MATCH_TEXT: ThemeColor = rgb(0x1f, 0x23, 0x2a);

/// Terminal color capability used when rendering theme colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorCapability {
    /// Emit xterm 256-color escape sequences.
    Ansi256,
    /// Emit 24-bit truecolor escape sequences.
    TrueColor,
}

/// One RGB color stored in theme data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ThemeColor {
    /// Red channel.
    pub(crate) red: u8,
    /// Green channel.
    pub(crate) green: u8,
    /// Blue channel.
    pub(crate) blue: u8,
}

/// One terminal style fragment used by syntax or UI rendering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ThemeStyle {
    /// Optional foreground color.
    pub(crate) fg: Option<ThemeColor>,
    /// Optional background color.
    pub(crate) bg: Option<ThemeColor>,
    /// Whether bold should be enabled.
    pub(crate) bold: bool,
    /// Whether underline should be enabled.
    pub(crate) underline: bool,
    /// Whether curly underline should be enabled.
    pub(crate) undercurl: bool,
    /// Whether reverse-video should be enabled.
    pub(crate) reverse: bool,
}

/// One fully resolved built-in theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Theme {
    /// Stable config-facing theme name.
    pub(crate) name: &'static str,
    background: ThemeStyle,
    gutter: ThemeStyle,
    gutter_current: ThemeStyle,
    eof_marker: ThemeStyle,
    selection: ThemeStyle,
    current_line: ThemeStyle,
    /// Reverse-video style for the visible mate of the current `%` match.
    passive_match: ThemeStyle,
    /// Background overlay for visible search-result matches.
    search_match: ThemeStyle,
    cursor_block: Option<ThemeColor>,
    cursor_beam: Option<ThemeColor>,
    statusline: ThemeStyle,
    statusline_normal: ThemeStyle,
    statusline_insert: ThemeStyle,
    statusline_visual: ThemeStyle,
    message_line: ThemeStyle,
    pending_prefix: ThemeStyle,
    popup: ThemeStyle,
    diagnostic_error: ThemeStyle,
    diagnostic_warning: ThemeStyle,
    diagnostic_information: ThemeStyle,
    diagnostic_hint: ThemeStyle,
    syntax_comment: ThemeStyle,
    syntax_doc_comment: ThemeStyle,
    syntax_string: ThemeStyle,
    syntax_number: ThemeStyle,
    syntax_keyword: ThemeStyle,
    syntax_preprocessor: ThemeStyle,
    syntax_punctuation: ThemeStyle,
    syntax_markup_heading: ThemeStyle,
    syntax_markup_code_fence: ThemeStyle,
    syntax_markup_inline_code: ThemeStyle,
    syntax_markup_list_marker: ThemeStyle,
    syntax_markup_quote: ThemeStyle,
    syntax_markup_link: ThemeStyle,
    syntax_markup_emphasis: ThemeStyle,
    syntax_markup_strong: ThemeStyle,
    syntax_markup_default: ThemeStyle,
}

/// Shared Catppuccin palette inputs used to build multiple bundled variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CatppuccinPalette {
    pub(super) rosewater: ThemeColor,
    pub(super) pink: ThemeColor,
    pub(super) mauve: ThemeColor,
    pub(super) red: ThemeColor,
    pub(super) peach: ThemeColor,
    pub(super) green: ThemeColor,
    pub(super) teal: ThemeColor,
    pub(super) sapphire: ThemeColor,
    pub(super) blue: ThemeColor,
    pub(super) lavender: ThemeColor,
    pub(super) text: ThemeColor,
    pub(super) subtext1: ThemeColor,
    pub(super) overlay2: ThemeColor,
    pub(super) overlay0: ThemeColor,
    pub(super) surface2: ThemeColor,
    pub(super) surface1: ThemeColor,
    pub(super) surface0: ThemeColor,
    pub(super) base: ThemeColor,
    pub(super) mantle: ThemeColor,
}

/// Build one RGB theme color.
pub(crate) const fn rgb(red: u8, green: u8, blue: u8) -> ThemeColor {
    ThemeColor { red, green, blue }
}

/// Build one explicit terminal style fragment.
pub(crate) const fn style(
    fg: Option<ThemeColor>,
    bg: Option<ThemeColor>,
    bold: bool,
    underline: bool,
) -> ThemeStyle {
    ThemeStyle {
        fg,
        bg,
        bold,
        underline,
        undercurl: false,
        reverse: false,
    }
}

/// Build a foreground-only style fragment.
pub(crate) const fn fg(color: ThemeColor) -> ThemeStyle {
    style(Some(color), None, false, false)
}

/// Build a foreground-only bold style fragment.
pub(crate) const fn fg_bold(color: ThemeColor) -> ThemeStyle {
    style(Some(color), None, true, false)
}

/// Build a foreground-only underlined style fragment.
pub(crate) const fn fg_underline(color: ThemeColor) -> ThemeStyle {
    style(Some(color), None, false, true)
}

/// Build a foreground/background style fragment.
pub(crate) const fn fg_bg(fg_color: ThemeColor, bg_color: ThemeColor) -> ThemeStyle {
    style(Some(fg_color), Some(bg_color), false, false)
}

/// Build a foreground/background bold style fragment.
pub(crate) const fn fg_bg_bold(fg_color: ThemeColor, bg_color: ThemeColor) -> ThemeStyle {
    style(Some(fg_color), Some(bg_color), true, false)
}

/// Build a background-only style fragment.
pub(crate) const fn bg(color: ThemeColor) -> ThemeStyle {
    style(None, Some(color), false, false)
}

/// Build a reverse-video-only style fragment.
pub(crate) const fn reverse_video() -> ThemeStyle {
    ThemeStyle {
        fg: None,
        bg: None,
        bold: false,
        underline: false,
        undercurl: false,
        reverse: true,
    }
}

impl ThemeStyle {
    /// Layer `overlay` onto this style while keeping unspecified fields intact.
    pub(crate) fn overlay(self, overlay: ThemeStyle) -> ThemeStyle {
        ThemeStyle {
            fg: overlay.fg.or(self.fg),
            bg: overlay.bg.or(self.bg),
            bold: self.bold || overlay.bold,
            underline: self.underline || overlay.underline,
            undercurl: self.undercurl || overlay.undercurl,
            reverse: self.reverse || overlay.reverse,
        }
    }
}

impl ThemeColor {
    /// Convert this RGB color to the nearest xterm 256-color palette index.
    pub(crate) fn ansi256_index(self) -> u8 {
        // Xterm offers two useful approximations for arbitrary RGB colors:
        // the 6x6x6 color cube and the dedicated grayscale ramp. We compute
        // both candidates up front because neutral backgrounds often look
        // noticeably better on the grayscale ramp than on the color cube.
        let cube_r = Self::cube_component(self.red);
        let cube_g = Self::cube_component(self.green);
        let cube_b = Self::cube_component(self.blue);
        let cube_index = 16 + 36 * cube_r + 6 * cube_g + cube_b;
        let cube_color = rgb(
            Self::cube_component_value(cube_r),
            Self::cube_component_value(cube_g),
            Self::cube_component_value(cube_b),
        );

        // The grayscale ramp is addressed separately from the color cube. Its
        // entries are evenly spaced 10 intensity steps apart, starting at 8.
        let gray_index = Self::gray_component(self.red, self.green, self.blue);
        let gray_value = 8 + gray_index * 10;
        let gray_color = rgb(gray_value, gray_value, gray_value);

        // Pick whichever approximation lands closer to the original RGB color.
        if self.distance_squared(gray_color) < self.distance_squared(cube_color) {
            232 + gray_index
        } else {
            cube_index
        }
    }

    /// Return the concrete xterm 256-color RGB approximation for this color.
    pub(crate) fn ansi256_rgb(self) -> ThemeColor {
        let index = self.ansi256_index();
        if index >= 232 {
            let value = 8 + (index - 232) * 10;
            return rgb(value, value, value);
        }
        let offset = index - 16;
        let levels = [0, 95, 135, 175, 215, 255];
        rgb(
            levels[(offset / 36) as usize],
            levels[((offset % 36) / 6) as usize],
            levels[(offset % 6) as usize],
        )
    }

    /// Return the squared Euclidean distance to another RGB color.
    pub(crate) fn distance_squared(self, other: ThemeColor) -> u32 {
        let red = i32::from(self.red) - i32::from(other.red);
        let green = i32::from(self.green) - i32::from(other.green);
        let blue = i32::from(self.blue) - i32::from(other.blue);
        (red * red + green * green + blue * blue) as u32
    }

    /// Quantize one 0-255 channel into the xterm 6-level cube domain.
    fn cube_component(value: u8) -> u8 {
        // The xterm cube uses the fixed channel values 0, 95, 135, 175, 215,
        // and 255. These thresholds pick the nearest bucket for the incoming
        // 8-bit channel without requiring floating-point math.
        if value < 48 {
            0
        } else if value < 115 {
            1
        } else {
            (((value as u16) - 35) / 40) as u8
        }
    }

    /// Return the concrete 0-255 value for one xterm cube component.
    fn cube_component_value(level: u8) -> u8 {
        // Cube level zero is a special case that maps to pure black. Every
        // higher level steps through the 95/135/175/215/255 sequence.
        match level {
            0 => 0,
            level => 55 + level * 40,
        }
    }

    /// Quantize an RGB triple into the xterm grayscale ramp domain.
    fn gray_component(red: u8, green: u8, blue: u8) -> u8 {
        // The grayscale ramp contains 24 entries that span intensities 8..=238.
        // We approximate perceived brightness with a simple average because the
        // ramp itself is neutral; once we only care about grayness, the average
        // is good enough and keeps the mapping fast and deterministic.
        let luminance = (u16::from(red) + u16::from(green) + u16::from(blue)) / 3;
        if luminance < 8 {
            0
        } else if luminance > 238 {
            23
        } else {
            ((luminance - 8) / 10) as u8
        }
    }
}

impl Theme {
    /// Return the style for one semantic syntax category.
    pub(crate) fn syntax_style(
        self,
        class: SyntaxClass,
        modifier: Option<SyntaxModifier>,
    ) -> ThemeStyle {
        match (class, modifier) {
            (SyntaxClass::Comment, Some(SyntaxModifier::DocComment)) => self.syntax_doc_comment,
            (SyntaxClass::Comment, _) => self.syntax_comment,
            (SyntaxClass::String, _) => self.syntax_string,
            (SyntaxClass::Number, _) => self.syntax_number,
            (SyntaxClass::Keyword, Some(SyntaxModifier::Preprocessor)) => self.syntax_preprocessor,
            (SyntaxClass::Keyword, _) => self.syntax_keyword,
            (SyntaxClass::Punctuation, _) => self.syntax_punctuation,
            (SyntaxClass::Markup, Some(SyntaxModifier::Heading)) => self.syntax_markup_heading,
            (SyntaxClass::Markup, Some(SyntaxModifier::CodeFence)) => self.syntax_markup_code_fence,
            (SyntaxClass::Markup, Some(SyntaxModifier::InlineCode)) => {
                self.syntax_markup_inline_code
            }
            (SyntaxClass::Markup, Some(SyntaxModifier::ListMarker)) => {
                self.syntax_markup_list_marker
            }
            (SyntaxClass::Markup, Some(SyntaxModifier::Quote)) => self.syntax_markup_quote,
            (SyntaxClass::Markup, Some(SyntaxModifier::Link)) => self.syntax_markup_link,
            (SyntaxClass::Markup, Some(SyntaxModifier::Emphasis)) => self.syntax_markup_emphasis,
            (SyntaxClass::Markup, Some(SyntaxModifier::Strong)) => self.syntax_markup_strong,
            (SyntaxClass::Markup, _) => self.syntax_markup_default,
        }
    }

    /// Return the background style used to prefill visible rows.
    pub(crate) fn background_style(self) -> ThemeStyle {
        self.background
    }

    /// Return the style for plain message-line content.
    pub(crate) fn message_line_style(self) -> ThemeStyle {
        self.background.overlay(self.message_line)
    }

    /// Return the style used for pending key-sequence prefixes.
    pub(crate) fn pending_prefix_style(self) -> ThemeStyle {
        self.message_line_style().overlay(self.pending_prefix)
    }

    /// Return the style used for the shortcut discovery popup.
    pub(crate) fn popup_style(self) -> ThemeStyle {
        self.background.overlay(self.popup)
    }

    /// Return the style used for the EOF marker gutter rows.
    pub(crate) fn eof_marker_style(self) -> ThemeStyle {
        self.background.overlay(self.eof_marker)
    }

    /// Return the style used for one gutter cell.
    pub(crate) fn gutter_style(self, current_line: bool) -> ThemeStyle {
        if current_line {
            self.background.overlay(self.gutter_current)
        } else {
            self.background.overlay(self.gutter)
        }
    }

    /// Return the base style used for the full status line.
    pub(crate) fn statusline_base_style(self) -> ThemeStyle {
        self.background.overlay(self.statusline)
    }

    /// Return the accent style used for the mode segment of the status line.
    pub(crate) fn statusline_mode_style(self, mode_label: &str) -> ThemeStyle {
        if mode_label == "INSERT" {
            self.statusline_insert
        } else if mode_label.starts_with("VISUAL") || mode_label == "V-LINE" {
            self.statusline_visual
        } else {
            self.statusline_normal
        }
    }

    /// Return the accent style used for the read-only status-line marker.
    pub(crate) fn statusline_readonly_style(self) -> ThemeStyle {
        let accent = self.diagnostic_accent_style(LspDiagnosticSeverity::Warning);
        let base = self.statusline_base_style();
        ThemeStyle {
            fg: accent.fg.or(base.fg),
            bg: base.bg,
            bold: true,
            underline: false,
            undercurl: false,
            reverse: false,
        }
    }

    /// Return the warning style used for a swap-detected message-line alert.
    pub(crate) fn message_line_swap_alert_style(self) -> ThemeStyle {
        let accent = self.diagnostic_accent_style(LspDiagnosticSeverity::Warning);
        let base = self.message_line_style();
        ThemeStyle {
            fg: base.bg.or(base.fg),
            bg: accent.fg.or(base.bg),
            bold: true,
            underline: false,
            undercurl: false,
            reverse: false,
        }
    }

    /// Return the selection overlay style.
    pub(crate) fn selection_style(self) -> ThemeStyle {
        self.selection
    }

    /// Return the current-line background overlay style.
    pub(crate) fn current_line_style(self) -> ThemeStyle {
        self.current_line
    }

    /// Return the base severity accent style for diagnostic UI elements.
    pub(crate) fn diagnostic_accent_style(self, severity: LspDiagnosticSeverity) -> ThemeStyle {
        match severity {
            LspDiagnosticSeverity::Error => self.diagnostic_error,
            LspDiagnosticSeverity::Warning => self.diagnostic_warning,
            LspDiagnosticSeverity::Information => self.diagnostic_information,
            LspDiagnosticSeverity::Hint => self.diagnostic_hint,
        }
    }

    /// Return the inline diagnostic overlay style for one severity.
    pub(crate) fn diagnostic_inline_style(self, severity: LspDiagnosticSeverity) -> ThemeStyle {
        let severity_style = self.diagnostic_accent_style(severity);
        ThemeStyle {
            fg: severity_style.fg,
            bg: None,
            bold: severity_style.bold,
            underline: false,
            undercurl: true,
            reverse: false,
        }
    }

    /// Return the gutter-marker style for one diagnostic severity.
    pub(crate) fn diagnostic_marker_style(
        self,
        severity: LspDiagnosticSeverity,
        current_line: bool,
    ) -> ThemeStyle {
        let base = self.gutter_style(current_line);
        let accent = self.diagnostic_accent_style(severity);
        ThemeStyle {
            fg: accent.fg.or(base.fg),
            bg: base.bg,
            bold: base.bold || accent.bold,
            underline: false,
            undercurl: false,
            reverse: false,
        }
    }

    /// Return the plain text style used by the cursor diagnostic overlay.
    pub(crate) fn diagnostic_message_style(self, severity: LspDiagnosticSeverity) -> ThemeStyle {
        let accent = self.diagnostic_accent_style(severity);
        ThemeStyle {
            fg: accent.fg,
            bg: self.background.bg,
            bold: accent.bold,
            underline: false,
            undercurl: false,
            reverse: false,
        }
    }

    /// Return the passive matching-delimiter style.
    pub(crate) fn passive_match_style(self) -> ThemeStyle {
        self.passive_match
    }

    /// Return the search-result overlay style.
    pub(crate) fn search_match_style(self) -> ThemeStyle {
        self.search_match
    }

    /// Return the preferred terminal cursor color for the active cursor shape.
    pub(crate) fn cursor_color(self, shape: crate::tui::CursorShape) -> Option<ThemeColor> {
        match shape {
            crate::tui::CursorShape::Block => self.cursor_block,
            crate::tui::CursorShape::Beam => self.cursor_beam.or(self.cursor_block),
        }
    }
}

/// Return all bundled themes in stable config-listing order.
pub(crate) fn all() -> &'static [Theme] {
    &THEMES
}

/// Return the stable names of all bundled themes.
pub(crate) fn names() -> &'static [&'static str] {
    &THEME_NAMES
}

/// Build one theme-name array from a bundled theme table.
const fn theme_names<const N: usize>(themes: &[Theme; N]) -> [&'static str; N] {
    let mut names = [""; N];
    let mut index = 0;
    while index < N {
        names[index] = themes[index].name;
        index += 1;
    }
    names
}

/// Look up one bundled theme by its config-facing name.
pub(crate) fn find(name: &str) -> Option<&'static Theme> {
    all().iter().find(|theme| theme.name == name)
}

/// Return the default bundled theme.
pub(crate) fn default_theme() -> &'static Theme {
    find(DEFAULT_THEME_NAME).expect("default theme must exist")
}

/// Detect the terminal color capability from common environment hints.
pub(crate) fn detect_color_capability(
    colorterm: Option<&str>,
    term: Option<&str>,
) -> ColorCapability {
    if colorterm.is_some_and(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        normalized.contains("truecolor") || normalized.contains("24bit")
    }) || term.is_some_and(|value| value.to_ascii_lowercase().contains("direct"))
    {
        ColorCapability::TrueColor
    } else {
        ColorCapability::Ansi256
    }
}

/// Build one Catppuccin variant using the shared role mapping.
pub(super) const fn catppuccin_theme(name: &'static str, palette: CatppuccinPalette) -> Theme {
    Theme {
        name,
        background: fg_bg(palette.text, palette.base),
        gutter: fg(palette.surface1),
        gutter_current: fg_bold(palette.lavender),
        eof_marker: fg(palette.surface2),
        selection: bg(palette.surface1),
        current_line: bg(palette.mantle),
        passive_match: reverse_video(),
        search_match: fg_bg(SEARCH_MATCH_TEXT, rgb(0xf9, 0xe2, 0x73)),
        cursor_block: Some(palette.rosewater),
        cursor_beam: Some(palette.green),
        statusline: fg_bg(palette.subtext1, palette.mantle),
        statusline_normal: fg_bg_bold(palette.base, palette.rosewater),
        statusline_insert: fg_bg_bold(palette.base, palette.green),
        statusline_visual: fg_bg_bold(palette.base, palette.lavender),
        message_line: fg_bg(palette.text, palette.base),
        pending_prefix: fg_bold(palette.rosewater),
        popup: fg_bg(palette.text, palette.surface0),
        diagnostic_error: fg_bold(palette.red),
        diagnostic_warning: fg_bold(palette.peach),
        diagnostic_information: fg(palette.blue),
        diagnostic_hint: fg(palette.overlay2),
        syntax_comment: fg(palette.overlay2),
        syntax_doc_comment: fg(palette.green),
        syntax_string: fg(palette.green),
        syntax_number: fg(palette.peach),
        syntax_keyword: fg_bold(palette.mauve),
        syntax_preprocessor: fg_bold(palette.pink),
        syntax_punctuation: fg(palette.overlay2),
        syntax_markup_heading: fg_bold(palette.blue),
        syntax_markup_code_fence: fg(palette.overlay0),
        syntax_markup_inline_code: fg(palette.green),
        syntax_markup_list_marker: fg(palette.teal),
        syntax_markup_quote: fg(palette.pink),
        syntax_markup_link: fg_underline(palette.blue),
        syntax_markup_emphasis: fg(palette.lavender),
        syntax_markup_strong: fg_bold(palette.red),
        syntax_markup_default: fg(palette.sapphire),
    }
}

const THEMES: [Theme; 10] = [
    bogster::THEME,
    catppuccin_frappe::THEME,
    catppuccin_latte::THEME,
    catppuccin_macchiato::THEME,
    catppuccin_mocha::THEME,
    gruvbox::THEME,
    kanagawa::THEME,
    nord::THEME,
    onedark::THEME,
    tokyonight::THEME,
];

const THEME_NAMES: [&str; 10] = theme_names(&THEMES);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_truecolor_from_colorterm() {
        assert_eq!(
            detect_color_capability(Some("truecolor"), None),
            ColorCapability::TrueColor
        );
    }

    #[test]
    fn detects_truecolor_from_direct_term() {
        assert_eq!(
            detect_color_capability(None, Some("xterm-direct")),
            ColorCapability::TrueColor
        );
    }

    #[test]
    fn defaults_to_ansi256_without_hints() {
        assert_eq!(
            detect_color_capability(None, None),
            ColorCapability::Ansi256
        );
    }

    #[test]
    fn finds_default_theme() {
        assert_eq!(default_theme().name, "bogster");
        assert!(find("nord").is_some());
    }

    #[test]
    fn theme_names_const_fn_extracts_names_in_order() {
        const SAMPLE_THEMES: [Theme; 2] = [bogster::THEME, nord::THEME];
        assert_eq!(theme_names(&SAMPLE_THEMES), ["bogster", "nord"]);
    }

    #[test]
    fn converts_rgb_to_stable_ansi256() {
        assert_eq!(rgb(0xdc, 0xb6, 0x59).ansi256_index(), 179);
        assert_eq!(rgb(0x36, 0xb2, 0xd4).ansi256_index(), 74);
    }

    #[test]
    fn catppuccin_latte_selection_and_cursor_values_match_theme_data() {
        let theme = find("catppuccin-latte").expect("theme should exist");
        assert_eq!(theme.selection_style().bg, Some(rgb(0xbc, 0xc0, 0xcc)));
        assert_eq!(theme.current_line_style().bg, Some(rgb(0xe6, 0xe9, 0xef)));
        assert_eq!(
            theme.cursor_color(crate::tui::CursorShape::Block),
            Some(rgb(0xdc, 0x8a, 0x78))
        );
        assert_eq!(
            theme.cursor_color(crate::tui::CursorShape::Beam),
            Some(rgb(0x40, 0xa0, 0x2b))
        );
        assert!(!theme.selection_style().underline);
    }

    #[test]
    fn current_line_highlight_is_distinct_from_background_for_every_theme() {
        for theme in all() {
            assert_ne!(
                theme.current_line_style().bg,
                theme.background_style().bg,
                "theme `{}` should give the current line a visible background accent",
                theme.name
            );
            assert!(theme.passive_match_style().reverse);
            assert_eq!(theme.passive_match_style().fg, None);
            assert_eq!(theme.passive_match_style().bg, None);
        }
    }

    #[test]
    fn preprocessor_styles_are_distinct_from_keyword_styles() {
        for theme in all() {
            assert_ne!(
                theme.syntax_style(SyntaxClass::Keyword, None),
                theme.syntax_style(SyntaxClass::Keyword, Some(SyntaxModifier::Preprocessor)),
                "theme `{}` should style preprocessors distinctly from keywords",
                theme.name
            );
        }
    }

    #[test]
    fn search_highlight_style_is_distinct_from_passive_match_style() {
        for theme in all() {
            assert_ne!(
                theme.search_match_style(),
                theme.passive_match_style(),
                "theme `{}` should give search results their own visible styling",
                theme.name
            );
        }
    }
}
