use super::*;

// Adapted from Helix runtime theme `tokyonight.toml`.
// Upstream Helix runtime themes are distributed under MPL-2.0.
const BACKGROUND: ThemeColor = rgb(0x1a, 0x1b, 0x26);
const PANEL: ThemeColor = rgb(0x16, 0x16, 0x1e);
const TEXT: ThemeColor = rgb(0xc0, 0xca, 0xf5);
const MUTED: ThemeColor = rgb(0x56, 0x5f, 0x89);
const GREEN: ThemeColor = rgb(0x9e, 0xce, 0x6a);
const BLUE: ThemeColor = rgb(0x7a, 0xa2, 0xf7);
const PURPLE: ThemeColor = rgb(0xbb, 0x9a, 0xf7);
const ORANGE: ThemeColor = rgb(0xff, 0x9e, 0x64);
const GOLD: ThemeColor = rgb(0xe0, 0xaf, 0x68);

pub(super) const THEME: Theme = Theme {
    name: "tokyonight",
    background: fg_bg(TEXT, BACKGROUND),
    gutter: fg(rgb(0x3b, 0x42, 0x61)),
    gutter_current: fg_bold(rgb(0x73, 0x7a, 0xa2)),
    eof_marker: fg(MUTED),
    selection: bg(rgb(0x28, 0x34, 0x57)),
    passive_match: bg(PANEL),
    cursor_block: None,
    cursor_beam: None,
    statusline: fg_bg(rgb(0xa9, 0xb1, 0xd6), PANEL),
    statusline_normal: fg_bg_bold(BACKGROUND, BLUE),
    statusline_insert: fg_bg_bold(BACKGROUND, GREEN),
    statusline_visual: fg_bg_bold(BACKGROUND, PURPLE),
    message_line: fg_bg(TEXT, BACKGROUND),
    pending_prefix: fg_bold(ORANGE),
    popup: fg_bg(TEXT, PANEL),
    diagnostic_error: fg_bold(rgb(0xf7, 0x76, 0x8e)),
    diagnostic_warning: fg_bold(ORANGE),
    diagnostic_information: fg(BLUE),
    diagnostic_hint: fg(MUTED),
    syntax_comment: fg(MUTED),
    syntax_doc_comment: fg(GOLD),
    syntax_string: fg(GREEN),
    syntax_number: fg(rgb(0xf7, 0x76, 0x8e)),
    syntax_keyword: fg_bold(rgb(0x9d, 0x7c, 0xd8)),
    syntax_preprocessor: fg_bold(ORANGE),
    syntax_punctuation: fg(rgb(0x89, 0xdd, 0xff)),
    syntax_markup_heading: fg_bold(BLUE),
    syntax_markup_code_fence: fg(MUTED),
    syntax_markup_inline_code: fg(rgb(0x1a, 0xbc, 0x9c)),
    syntax_markup_list_marker: fg(ORANGE),
    syntax_markup_quote: fg(GOLD),
    syntax_markup_link: fg_underline(BLUE),
    syntax_markup_emphasis: fg(PURPLE),
    syntax_markup_strong: fg_bold(BLUE),
    syntax_markup_default: fg(rgb(0x7d, 0xcf, 0xff)),
};
