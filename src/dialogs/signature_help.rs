//! Read-only signature-help popup state rendered near the cursor.

use crate::lsp::protocol::{LspParameterLabel, LspSignatureHelp, LspSignatureInformation};

/// Render-facing popup model for one LSP signature-help response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SignatureHelpPopup {
    /// Title shown in the popup border.
    pub(crate) title: String,
    /// Raw popup lines shown before render-side wrapping.
    pub(crate) lines: Vec<String>,
}

impl SignatureHelpPopup {
    /// Build one signature-help popup from one parsed LSP signature-help result.
    pub(crate) fn new(help: &LspSignatureHelp) -> Self {
        let signature = help
            .signatures
            .get(help.active_signature)
            .unwrap_or(&help.signatures[0]);
        let active_parameter = signature.active_parameter.or(help.active_parameter);
        let mut lines = vec![highlight_signature_label(signature, active_parameter)];
        if let Some(documentation) = signature.documentation.as_deref() {
            // Documentation remains visually separated from the signature line so
            // long prose blocks stay readable when render-side wrapping kicks in.
            lines.push(String::new());
            lines.extend(documentation.lines().map(str::to_string));
        }
        Self {
            title: "Signature Help".to_string(),
            lines,
        }
    }
}

/// Return the signature label with the active parameter wrapped in brackets.
fn highlight_signature_label(
    signature: &LspSignatureInformation,
    active_parameter: Option<usize>,
) -> String {
    let Some(active_parameter) = active_parameter else {
        return signature.label.clone();
    };
    let Some(parameter) = signature.parameters.get(active_parameter) else {
        return signature.label.clone();
    };
    let Some((start, end)) = parameter_highlight_range(&signature.label, &parameter.label) else {
        return signature.label.clone();
    };
    format!(
        "{}[{}]{}",
        &signature.label[..start],
        &signature.label[start..end],
        &signature.label[end..]
    )
}

/// Return the byte range inside `label` that should be emphasized.
fn parameter_highlight_range(label: &str, parameter: &LspParameterLabel) -> Option<(usize, usize)> {
    match parameter {
        LspParameterLabel::Text(text) => label
            .find(text)
            .map(|start| (start, start.saturating_add(text.len()))),
        LspParameterLabel::Offsets { start, end } => {
            utf16_offsets_to_byte_range(label, *start, *end)
        }
    }
}

/// Convert UTF-16 code-unit offsets into a byte range inside `text`.
fn utf16_offsets_to_byte_range(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let mut utf16_offset = 0;
    let mut start_byte = None;
    let mut end_byte = None;
    for (byte_index, ch) in text.char_indices() {
        // Offset-based labels can begin or end between iterations, so check the
        // current accumulated UTF-16 position before advancing past `ch`.
        if start_byte.is_none() && utf16_offset == start {
            start_byte = Some(byte_index);
        }
        if end_byte.is_none() && utf16_offset == end {
            end_byte = Some(byte_index);
        }
        utf16_offset += ch.len_utf16();
    }
    if start_byte.is_none() && utf16_offset == start {
        start_byte = Some(text.len());
    }
    if end_byte.is_none() && utf16_offset == end {
        end_byte = Some(text.len());
    }
    match (start_byte, end_byte) {
        (Some(start_byte), Some(end_byte)) if start_byte <= end_byte => {
            Some((start_byte, end_byte))
        }
        _ => None,
    }
}
