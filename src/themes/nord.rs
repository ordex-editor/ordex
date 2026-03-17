use super::*;

// Adapted from Helix runtime theme `nord.toml`.
// Upstream Helix runtime themes are distributed under MPL-2.0.
const BACKGROUND: ThemeColor = rgb(0x2e, 0x34, 0x40);
const PANEL: ThemeColor = rgb(0x3b, 0x42, 0x52);
const TEXT: ThemeColor = rgb(0xd8, 0xde, 0xe9);
const MUTED: ThemeColor = rgb(0x4c, 0x56, 0x6a);
const LIGHT: ThemeColor = rgb(0xec, 0xef, 0xf4);
const CYAN: ThemeColor = rgb(0x88, 0xc0, 0xd0);
const TEAL: ThemeColor = rgb(0x8f, 0xbc, 0xbb);
const GREEN: ThemeColor = rgb(0xa3, 0xbe, 0x8c);
const PURPLE: ThemeColor = rgb(0xb4, 0x8e, 0xad);

pub(super) const THEME: Theme = Theme {
    name: "nord",
    background: fg_bg(TEXT, BACKGROUND),
    gutter: fg(MUTED),
    gutter_current: fg_bold(rgb(0xe5, 0xe9, 0xf0)),
    eof_marker: fg(MUTED),
    selection: bg(MUTED),
    cursor_block: Some(TEXT),
    cursor_beam: Some(TEXT),
    statusline: fg_bg(TEXT, PANEL),
    statusline_normal: fg_bg_bold(PANEL, CYAN),
    statusline_insert: fg_bg_bold(PANEL, LIGHT),
    statusline_visual: fg_bg_bold(PANEL, TEAL),
    message_line: fg_bg(TEXT, BACKGROUND),
    pending_prefix: fg_bold(rgb(0xeb, 0xcb, 0x8b)),
    popup: fg_bg(TEXT, PANEL),
    syntax_comment: fg(rgb(0x61, 0x6e, 0x88)),
    syntax_doc_comment: fg(GREEN),
    syntax_string: fg(GREEN),
    syntax_number: fg(PURPLE),
    syntax_keyword: fg_bold(rgb(0x81, 0xa1, 0xc1)),
    syntax_punctuation: fg(LIGHT),
    syntax_markup_heading: fg_bold(CYAN),
    syntax_markup_code_fence: fg(MUTED),
    syntax_markup_inline_code: fg(TEAL),
    syntax_markup_list_marker: fg(rgb(0x81, 0xa1, 0xc1)),
    syntax_markup_quote: fg(PURPLE),
    syntax_markup_link: fg_underline(CYAN),
    syntax_markup_emphasis: fg(PURPLE),
    syntax_markup_strong: fg_bold(CYAN),
    syntax_markup_default: fg(CYAN),
};
