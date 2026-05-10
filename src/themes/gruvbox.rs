use super::*;

// Adapted from Helix runtime theme `gruvbox.toml`.
// Upstream Helix runtime themes are distributed under MPL-2.0.
const BACKGROUND: ThemeColor = rgb(0x28, 0x28, 0x28);
const PANEL: ThemeColor = rgb(0x3c, 0x38, 0x36);
const TEXT: ThemeColor = rgb(0xeb, 0xdb, 0xb2);
const MUTED: ThemeColor = rgb(0x92, 0x83, 0x74);
const YELLOW: ThemeColor = rgb(0xfa, 0xbd, 0x2f);
const GREEN: ThemeColor = rgb(0x8e, 0xc0, 0x7c);
const RED: ThemeColor = rgb(0xfb, 0x49, 0x34);
const MAGENTA: ThemeColor = rgb(0xd3, 0x86, 0x9b);
const BLUE: ThemeColor = rgb(0x83, 0xa5, 0x98);

pub(super) const THEME: Theme = Theme {
    name: "gruvbox",
    background: fg_bg(TEXT, BACKGROUND),
    gutter: fg(rgb(0x66, 0x5c, 0x54)),
    gutter_current: fg_bold(YELLOW),
    eof_marker: fg(MUTED),
    selection: bg(rgb(0x50, 0x49, 0x45)),
    passive_match: bg(PANEL),
    search_match: fg_bg(SEARCH_MATCH_TEXT, YELLOW),
    cursor_block: Some(rgb(0xbd, 0xae, 0x93)),
    cursor_beam: Some(rgb(0x83, 0xa5, 0x98)),
    statusline: fg_bg(TEXT, rgb(0x50, 0x49, 0x45)),
    statusline_normal: fg_bg_bold(PANEL, rgb(0xbd, 0xae, 0x93)),
    statusline_insert: fg_bg_bold(PANEL, rgb(0x83, 0xa5, 0x98)),
    statusline_visual: fg_bg_bold(PANEL, rgb(0xfe, 0x80, 0x19)),
    message_line: fg_bg(TEXT, BACKGROUND),
    pending_prefix: fg_bold(YELLOW),
    popup: fg_bg(TEXT, PANEL),
    diagnostic_error: fg_bold(RED),
    diagnostic_warning: fg_bold(YELLOW),
    diagnostic_information: fg(BLUE),
    diagnostic_hint: fg(MUTED),
    syntax_comment: fg(MUTED),
    syntax_doc_comment: fg(GREEN),
    syntax_string: fg(rgb(0xb8, 0xbb, 0x26)),
    syntax_number: fg(MAGENTA),
    syntax_keyword: fg_bold(RED),
    syntax_preprocessor: fg_bold(YELLOW),
    syntax_punctuation: fg(rgb(0xfe, 0x80, 0x19)),
    syntax_markup_heading: fg_bold(GREEN),
    syntax_markup_code_fence: fg(rgb(0x50, 0x49, 0x45)),
    syntax_markup_inline_code: fg(RED),
    syntax_markup_list_marker: fg(RED),
    syntax_markup_quote: fg(YELLOW),
    syntax_markup_link: fg_underline(rgb(0xb8, 0xbb, 0x26)),
    syntax_markup_emphasis: fg(MAGENTA),
    syntax_markup_strong: fg_bold(YELLOW),
    syntax_markup_default: fg(GREEN),
};
