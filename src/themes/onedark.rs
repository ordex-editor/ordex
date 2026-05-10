use super::*;

// Adapted from Helix runtime theme `onedark.toml`.
// Upstream Helix runtime themes are distributed under MPL-2.0.
const BACKGROUND: ThemeColor = rgb(0x28, 0x2c, 0x34);
const PANEL: ThemeColor = rgb(0x2c, 0x32, 0x3c);
const TEXT: ThemeColor = rgb(0xab, 0xb2, 0xbf);
const COMMENT: ThemeColor = rgb(0x5c, 0x63, 0x70);
const GREEN: ThemeColor = rgb(0x98, 0xc3, 0x79);
const YELLOW: ThemeColor = rgb(0xe5, 0xc0, 0x7b);
const RED: ThemeColor = rgb(0xe0, 0x6c, 0x75);
const BLUE: ThemeColor = rgb(0x61, 0xaf, 0xef);

pub(super) const THEME: Theme = Theme {
    name: "onedark",
    background: fg_bg(TEXT, BACKGROUND),
    gutter: fg(rgb(0x4b, 0x52, 0x63)),
    gutter_current: fg_bold(TEXT),
    eof_marker: fg(COMMENT),
    selection: bg(rgb(0x3b, 0x40, 0x48)),
    passive_match: bg(PANEL),
    search_match: fg_bg(SEARCH_MATCH_TEXT, YELLOW),
    cursor_block: Some(TEXT),
    cursor_beam: Some(TEXT),
    statusline: fg_bg(TEXT, PANEL),
    statusline_normal: fg_bg_bold(PANEL, BLUE),
    statusline_insert: fg_bg_bold(PANEL, GREEN),
    statusline_visual: fg_bg_bold(PANEL, rgb(0xc6, 0x78, 0xdd)),
    message_line: fg_bg(TEXT, BACKGROUND),
    pending_prefix: fg_bold(YELLOW),
    popup: fg_bg(TEXT, rgb(0x3e, 0x44, 0x52)),
    diagnostic_error: fg_bold(RED),
    diagnostic_warning: fg_bold(YELLOW),
    diagnostic_information: fg(BLUE),
    diagnostic_hint: fg(COMMENT),
    syntax_comment: fg(COMMENT),
    syntax_doc_comment: fg(GREEN),
    syntax_string: fg(GREEN),
    syntax_number: fg(rgb(0xd1, 0x9a, 0x66)),
    syntax_keyword: fg_bold(RED),
    syntax_preprocessor: fg_bold(BLUE),
    syntax_punctuation: fg(RED),
    syntax_markup_heading: fg_bold(BLUE),
    syntax_markup_code_fence: fg(COMMENT),
    syntax_markup_inline_code: fg(GREEN),
    syntax_markup_list_marker: fg(RED),
    syntax_markup_quote: fg(YELLOW),
    syntax_markup_link: fg_underline(rgb(0x56, 0xb6, 0xc2)),
    syntax_markup_emphasis: fg(rgb(0xc6, 0x78, 0xdd)),
    syntax_markup_strong: fg_bold(YELLOW),
    syntax_markup_default: fg(BLUE),
};
