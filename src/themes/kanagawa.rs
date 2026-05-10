use super::*;

// Adapted from Helix runtime theme `kanagawa.toml`.
// Upstream Helix runtime themes are distributed under MPL-2.0.
const BACKGROUND: ThemeColor = rgb(0x1f, 0x1f, 0x28);
const CURRENT_LINE: ThemeColor = rgb(0x22, 0x22, 0x2c);
const PANEL: ThemeColor = rgb(0x16, 0x16, 0x1d);
const TEXT: ThemeColor = rgb(0xdc, 0xd7, 0xba);
const STATUS_TEXT: ThemeColor = rgb(0xc8, 0xc0, 0x93);
const MUTED: ThemeColor = rgb(0x54, 0x54, 0x6d);
const GREEN: ThemeColor = rgb(0x98, 0xbb, 0x6c);
const PURPLE: ThemeColor = rgb(0x95, 0x7f, 0xb8);
const PINK: ThemeColor = rgb(0xd2, 0x7e, 0x99);
const PALE_PURPLE: ThemeColor = rgb(0xb8, 0xb4, 0xd0);
const ORANGE: ThemeColor = rgb(0xff, 0xa0, 0x66);

pub(super) const THEME: Theme = Theme {
    name: "kanagawa",
    background: fg_bg(TEXT, BACKGROUND),
    gutter: fg(MUTED),
    gutter_current: fg_bold(rgb(0xff, 0x9e, 0x3b)),
    eof_marker: fg(MUTED),
    selection: bg(rgb(0x2d, 0x4f, 0x67)),
    current_line: bg(CURRENT_LINE),
    passive_match: reverse_video(),
    search_match: fg_bg(SEARCH_MATCH_TEXT, rgb(0xe6, 0xc3, 0x84)),
    cursor_block: Some(TEXT),
    cursor_beam: Some(TEXT),
    statusline: fg_bg(STATUS_TEXT, PANEL),
    statusline_normal: fg_bg_bold(PANEL, rgb(0x7e, 0x9c, 0xd8)),
    statusline_insert: fg_bg_bold(PANEL, rgb(0x76, 0x94, 0x6a)),
    statusline_visual: fg_bg_bold(PANEL, PURPLE),
    message_line: fg_bg(TEXT, BACKGROUND),
    pending_prefix: fg_bold(rgb(0xff, 0xa0, 0x66)),
    popup: fg_bg(TEXT, PANEL),
    diagnostic_error: fg_bold(PINK),
    diagnostic_warning: fg_bold(ORANGE),
    diagnostic_information: fg(rgb(0x7e, 0x9c, 0xd8)),
    diagnostic_hint: fg(MUTED),
    syntax_comment: fg(rgb(0x72, 0x71, 0x69)),
    syntax_doc_comment: fg(GREEN),
    syntax_string: fg(GREEN),
    syntax_number: fg(PINK),
    syntax_keyword: fg_bold(PURPLE),
    syntax_preprocessor: fg_bold(ORANGE),
    syntax_punctuation: fg(rgb(0x9c, 0xab, 0xca)),
    syntax_markup_heading: fg_bold(rgb(0x7e, 0x9c, 0xd8)),
    syntax_markup_code_fence: fg(MUTED),
    syntax_markup_inline_code: fg(GREEN),
    syntax_markup_list_marker: fg(PINK),
    syntax_markup_quote: fg(PALE_PURPLE),
    syntax_markup_link: fg_underline(rgb(0xa3, 0xd4, 0xd5)),
    syntax_markup_emphasis: fg(PALE_PURPLE),
    syntax_markup_strong: fg_bold(rgb(0xe6, 0xc3, 0x84)),
    syntax_markup_default: fg(rgb(0x7f, 0xb4, 0xca)),
};
