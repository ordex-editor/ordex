use super::*;

// Adapted from Helix runtime theme `bogster.toml`.
// Upstream Helix runtime themes are distributed under MPL-2.0.
const BACKGROUND: ThemeColor = rgb(0x16, 0x1c, 0x23);
const CURRENT_LINE: ThemeColor = rgb(0x1c, 0x24, 0x2d);
const PANEL: ThemeColor = rgb(0x23, 0x2d, 0x38);
const TEXT: ThemeColor = rgb(0xe5, 0xde, 0xd6);
const GUTTER: ThemeColor = rgb(0x41, 0x53, 0x67);
const SELECTION: ThemeColor = rgb(0x31, 0x3f, 0x4e);
const CYAN: ThemeColor = rgb(0x36, 0xb2, 0xd4);
const GREEN: ThemeColor = rgb(0x7f, 0xdc, 0x59);
const GOLD: ThemeColor = rgb(0xdc, 0xb6, 0x59);
const TEAL: ThemeColor = rgb(0x59, 0xdc, 0xb7);
const CRIMSON: ThemeColor = rgb(0xd3, 0x2c, 0x5d);
const CURSOR: ThemeColor = rgb(0xab, 0xb2, 0xbf);

pub(super) const THEME: Theme = Theme {
    name: "bogster",
    background: fg_bg(TEXT, BACKGROUND),
    gutter: fg(GUTTER),
    gutter_current: fg_bold(TEXT),
    eof_marker: fg(GUTTER),
    selection: bg(SELECTION),
    current_line: bg(CURRENT_LINE),
    passive_match: reverse_video(),
    search_match: fg_bg(SEARCH_MATCH_TEXT, GOLD),
    long_line_overflow: bg(CURRENT_LINE),
    cursor_block: Some(CURSOR),
    cursor_beam: Some(CURSOR),
    statusline: fg_bg(TEXT, PANEL),
    statusline_normal: fg_bg_bold(BACKGROUND, CYAN),
    statusline_insert: fg_bg_bold(BACKGROUND, GREEN),
    statusline_visual: fg_bg_bold(BACKGROUND, CRIMSON),
    message_line: fg_bg(TEXT, BACKGROUND),
    pending_prefix: fg_bold(GOLD),
    popup: fg_bg(TEXT, PANEL),
    diagnostic_error: fg_bold(CRIMSON),
    diagnostic_warning: fg_bold(GOLD),
    diagnostic_information: fg(CYAN),
    diagnostic_hint: fg(GUTTER),
    syntax_comment: fg(rgb(0xab, 0xb2, 0xbf)),
    syntax_doc_comment: fg(GREEN),
    syntax_string: fg(TEAL),
    syntax_number: fg(CYAN),
    syntax_keyword: fg_bold(GOLD),
    syntax_preprocessor: fg_bold(CRIMSON),
    syntax_punctuation: fg(rgb(0xdc, 0x77, 0x59)),
    syntax_markup_heading: fg_bold(CYAN),
    syntax_markup_code_fence: fg(GUTTER),
    syntax_markup_inline_code: fg(GOLD),
    syntax_markup_list_marker: fg(CRIMSON),
    syntax_markup_quote: fg(TEAL),
    syntax_markup_link: fg_underline(GOLD),
    syntax_markup_emphasis: fg(rgb(0xb7, 0x59, 0xdc)),
    syntax_markup_strong: fg_bold(GOLD),
    syntax_markup_default: fg(rgb(0x59, 0xdc, 0xd8)),
    syntax_todo: fg_bold(CURSOR),
};
