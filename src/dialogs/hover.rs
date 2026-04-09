//! Read-only hover popup state rendered near the cursor.

/// Render-facing popup model for one LSP hover response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HoverPopup {
    /// Title shown in the popup border.
    pub(crate) title: String,
    /// Raw hover lines shown in the popup body before render-side wrapping.
    pub(crate) lines: Vec<String>,
}

impl HoverPopup {
    /// Build one hover popup from a language-server response string.
    pub(crate) fn new(text: &str) -> Self {
        let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Self {
            title: "Hover".to_string(),
            lines,
        }
    }
}
