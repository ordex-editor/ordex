//! Read-only signature-help popup state rendered near the cursor.

use crate::lsp::protocol::{
    LspSignatureHelp, LspSignatureInformation, signature_parameter_highlight_char_range,
};

/// Render-facing popup model for one LSP signature-help response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SignatureHelpPopup {
    /// Title shown in the popup border.
    pub(crate) title: String,
    /// Signature line shown before render-side wrapping.
    pub(crate) signature_line: String,
    /// Character range of the active parameter within `signature_line`.
    pub(crate) active_parameter_range: Option<(usize, usize)>,
    /// Optional documentation lines shown after the signature body.
    pub(crate) documentation_lines: Vec<String>,
    /// Stable popup anchor kept at the opening call site while typing.
    pub(crate) anchor_char_idx: usize,
}

impl SignatureHelpPopup {
    /// Build one signature-help popup from one parsed LSP signature-help result.
    pub(crate) fn new(help: &LspSignatureHelp, anchor_char_idx: usize) -> Self {
        let signature = help
            .signatures
            .get(help.active_signature)
            .unwrap_or(&help.signatures[0]);
        let active_parameter = signature.active_parameter.or(help.active_parameter);
        let signature_line = signature.label.clone();
        let active_parameter_range = highlight_signature_label(signature, active_parameter);
        let mut documentation_lines = Vec::new();
        if let Some(documentation) = signature.documentation.as_deref() {
            // Documentation remains visually separated from the signature line so
            // long prose blocks stay readable when render-side wrapping kicks in.
            documentation_lines.push(String::new());
            documentation_lines.extend(documentation.lines().map(str::to_string));
        }
        Self {
            title: "Signature Help".to_string(),
            signature_line,
            active_parameter_range,
            documentation_lines,
            anchor_char_idx,
        }
    }
}

/// Return the character range of the active parameter inside the signature label.
fn highlight_signature_label(
    signature: &LspSignatureInformation,
    active_parameter: Option<usize>,
) -> Option<(usize, usize)> {
    let Some(active_parameter) = active_parameter else {
        return None;
    };
    let Some(parameter) = signature.parameters.get(active_parameter) else {
        return None;
    };
    signature_parameter_highlight_char_range(&signature.label, &parameter.label)
}
