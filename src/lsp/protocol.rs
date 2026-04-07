//! Narrow JSON-RPC and LSP message helpers for definition lookups.

use json::{JsonValue, object};
use std::fmt;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

/// One text position in LSP coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LspPosition {
    /// Zero-based line index.
    pub(crate) line: usize,
    /// Zero-based UTF-16 code-unit column.
    pub(crate) character: usize,
}

/// One text range in LSP coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LspRange {
    /// Inclusive range start in zero-based LSP coordinates.
    pub(crate) start: LspPosition,
    /// Exclusive range end in zero-based LSP coordinates.
    pub(crate) end: LspPosition,
}

/// One text change payload ready for `textDocument/didChange`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspTextChange {
    /// Replaced range for incremental sync, or `None` for whole-document sync.
    pub(crate) range: Option<LspRange>,
    /// Replacement text inserted for this change event.
    pub(crate) text: String,
}

/// Server-advertised text sync mode for open documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextDocumentSyncKind {
    /// The server does not accept text sync updates after open.
    None,
    /// The server expects whole-document replacement text in each change.
    Full,
    /// The server accepts ranged incremental change events.
    Incremental,
}

/// One file location returned by a definition request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspLocation {
    /// Canonical file URI for the target document.
    pub(crate) uri: String,
    /// Zero-based line index.
    pub(crate) line: usize,
    /// Zero-based UTF-16 code-unit column.
    pub(crate) character: usize,
}

/// One server response decoded into the subset Ordex needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerMessage {
    Response {
        id: u64,
        result: Option<JsonValue>,
        error: Option<String>,
    },
    Notification {
        method: String,
    },
}

/// Failure returned while reading or decoding one LSP message.
#[derive(Debug)]
pub(crate) enum ProtocolError {
    Io(io::Error),
    MissingContentLength,
    InvalidContentLength(String),
    InvalidJson(String),
    InvalidResponse(String),
    UnsupportedUri(String),
}

impl fmt::Display for ProtocolError {
    /// Format one protocol failure for status messages and tests.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::MissingContentLength => write!(f, "missing Content-Length header"),
            Self::InvalidContentLength(value) => {
                write!(f, "invalid Content-Length header: {value}")
            }
            Self::InvalidJson(error) => write!(f, "invalid JSON payload: {error}"),
            Self::InvalidResponse(error) => write!(f, "invalid LSP response: {error}"),
            Self::UnsupportedUri(uri) => write!(f, "unsupported file URI: {uri}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<io::Error> for ProtocolError {
    /// Wrap one I/O failure as a protocol failure.
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Write one JSON-RPC payload with LSP framing.
pub(crate) fn write_message(
    writer: &mut impl Write,
    payload: &JsonValue,
) -> Result<(), ProtocolError> {
    let body = payload.dump();
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()?;
    Ok(())
}

/// Read one complete LSP message and decode the response subset Ordex uses.
pub(crate) fn read_message(reader: &mut impl BufRead) -> Result<ServerMessage, ProtocolError> {
    let content_length = read_content_length(reader)?;
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;
    let parsed = json::parse(
        std::str::from_utf8(&body)
            .map_err(|error| ProtocolError::InvalidJson(error.to_string()))?,
    )
    .map_err(|error| ProtocolError::InvalidJson(error.to_string()))?;

    if let Some(method) = parsed["method"].as_str() {
        return Ok(ServerMessage::Notification {
            method: method.to_string(),
        });
    }
    if let Some(id) = parsed["id"].as_u64() {
        let result = (!parsed["result"].is_null()).then(|| parsed["result"].clone());
        let error = if parsed["error"].is_null() {
            None
        } else {
            Some(
                parsed["error"]["message"]
                    .as_str()
                    .unwrap_or("LSP error")
                    .to_string(),
            )
        };
        return Ok(ServerMessage::Response { id, result, error });
    }
    Err(ProtocolError::InvalidResponse(
        "message is missing both id and method".to_string(),
    ))
}

/// Build the initialize request payload for one workspace root.
pub(crate) fn initialize_request(id: u64, workspace_root: &Path) -> JsonValue {
    let root_uri = path_to_file_uri(workspace_root);
    object! {
        jsonrpc: "2.0",
        id: id,
        method: "initialize",
        params: {
            processId: std::process::id() as i32,
            rootUri: root_uri.as_str(),
            capabilities: {},
            workspaceFolders: [{
                uri: root_uri.as_str(),
                name: workspace_root.file_name().and_then(|value| value.to_str()).unwrap_or("workspace")
            }]
        }
    }
}

/// Build the `initialized` notification payload.
pub(crate) fn initialized_notification() -> JsonValue {
    object! {
        jsonrpc: "2.0",
        method: "initialized",
        params: {}
    }
}

/// Build the `didOpen` notification payload for one buffer snapshot.
pub(crate) fn did_open_notification(path: &Path, version: i32, text: &str) -> JsonValue {
    let uri = path_to_file_uri(path);
    object! {
        jsonrpc: "2.0",
        method: "textDocument/didOpen",
        params: {
            textDocument: {
                uri: uri.as_str(),
                languageId: "rust",
                version: version,
                text: text
            }
        }
    }
}

/// Build the `didChange` notification payload for one or more text changes.
pub(crate) fn did_change_notification(
    path: &Path,
    version: i32,
    changes: &[LspTextChange],
) -> JsonValue {
    let uri = path_to_file_uri(path);
    let mut content_changes = JsonValue::new_array();

    // The JSON crate used here does not offer ergonomic array literals for
    // dynamically sized payloads, so build the change list incrementally.
    for change in changes {
        let payload = if let Some(range) = change.range {
            object! {
                range: {
                    start: json_position(range.start),
                    end: json_position(range.end),
                },
                text: change.text.as_str(),
            }
        } else {
            object! {
                text: change.text.as_str(),
            }
        };
        content_changes
            .push(payload)
            .expect("didChange payload array should accept appended change");
    }
    object! {
        jsonrpc: "2.0",
        method: "textDocument/didChange",
        params: {
            textDocument: {
                uri: uri.as_str(),
                version: version
            },
            contentChanges: content_changes
        }
    }
}

/// Parse one initialize response and return the negotiated text sync mode.
pub(crate) fn parse_text_document_sync_kind(
    result: Option<&JsonValue>,
) -> Result<TextDocumentSyncKind, ProtocolError> {
    let capabilities = result.ok_or_else(|| {
        ProtocolError::InvalidResponse("initialize result is missing capabilities".to_string())
    })?;
    let sync = &capabilities["capabilities"]["textDocumentSync"];

    // Keep compatibility with servers that omit the field entirely by falling
    // back to the previous whole-document behavior.
    if sync.is_null() {
        return Ok(TextDocumentSyncKind::Full);
    }
    if let Some(kind) = sync.as_u8() {
        return parse_sync_kind(kind);
    }
    if sync.is_object() {
        return match sync["change"].as_u8() {
            Some(kind) => parse_sync_kind(kind),
            None => Ok(TextDocumentSyncKind::Full),
        };
    }
    Err(ProtocolError::InvalidResponse(
        "textDocumentSync is neither a number nor an object".to_string(),
    ))
}

/// Build the go-to-definition request payload.
pub(crate) fn definition_request(id: u64, path: &Path, position: LspPosition) -> JsonValue {
    let uri = path_to_file_uri(path);
    object! {
        jsonrpc: "2.0",
        id: id,
        method: "textDocument/definition",
        params: {
            textDocument: {
                uri: uri.as_str()
            },
            position: {
                line: position.line,
                character: position.character
            }
        }
    }
}

/// Build the `shutdown` request payload.
pub(crate) fn shutdown_request(id: u64) -> JsonValue {
    object! {
        jsonrpc: "2.0",
        id: id,
        method: "shutdown",
        params: JsonValue::Null
    }
}

/// Build the `exit` notification payload.
pub(crate) fn exit_notification() -> JsonValue {
    object! {
        jsonrpc: "2.0",
        method: "exit",
        params: JsonValue::Null
    }
}

/// Decode one definition response payload into normalized locations.
pub(crate) fn parse_definition_result(
    result: Option<&JsonValue>,
) -> Result<Vec<LspLocation>, ProtocolError> {
    let Some(result) = result else {
        return Ok(Vec::new());
    };
    if result.is_null() {
        return Ok(Vec::new());
    }
    if result.is_array() {
        let mut locations = Vec::new();
        for item in result.members() {
            parse_location_like(item, &mut locations)?;
        }
        return Ok(locations);
    }
    let mut locations = Vec::new();
    parse_location_like(result, &mut locations)?;
    Ok(locations)
}

/// Convert one filesystem path into a `file://` URI.
pub(crate) fn path_to_file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().as_bytes() {
        match byte {
            // Preserve RFC 3986 unreserved bytes plus `/` so ordinary Unix paths
            // stay readable and rust-analyzer receives a standard file URI.
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(char::from(*byte))
            }
            _ => {
                // Percent-encode everything else so spaces and other special
                // bytes remain unambiguous in the URI transport payload.
                uri.push('%');
                uri.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
                uri.push(char::from(b"0123456789ABCDEF"[(byte & 0x0F) as usize]));
            }
        }
    }
    uri
}

/// Convert one `file://` URI into a filesystem path.
pub(crate) fn file_uri_to_path(uri: &str) -> Result<PathBuf, ProtocolError> {
    let Some(path) = uri.strip_prefix("file://") else {
        return Err(ProtocolError::UnsupportedUri(uri.to_string()));
    };
    let mut decoded = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(ProtocolError::UnsupportedUri(uri.to_string()));
            }
            let high = decode_hex_digit(bytes[index + 1])?;
            let low = decode_hex_digit(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let decoded =
        String::from_utf8(decoded).map_err(|_| ProtocolError::UnsupportedUri(uri.to_string()))?;
    Ok(PathBuf::from(decoded))
}

/// Read the LSP headers and return the declared content length.
fn read_content_length(reader: &mut impl BufRead) -> Result<usize, ProtocolError> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Err(ProtocolError::MissingContentLength);
        }
        // LSP terminates its header block with one empty line, so keep reading
        // header rows until that separator appears.
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        // Only `Content-Length` matters for this transport subset. Unknown
        // headers are ignored so optional metadata does not break decoding.
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| ProtocolError::InvalidContentLength(value.trim().to_string()))?,
            );
        }
    }
    content_length.ok_or(ProtocolError::MissingContentLength)
}

/// Convert one LSP position into the JSON object shape used by requests.
fn json_position(position: LspPosition) -> JsonValue {
    object! {
        line: position.line,
        character: position.character,
    }
}

/// Convert one numeric sync kind into the local enum.
fn parse_sync_kind(kind: u8) -> Result<TextDocumentSyncKind, ProtocolError> {
    match kind {
        0 => Ok(TextDocumentSyncKind::None),
        1 => Ok(TextDocumentSyncKind::Full),
        2 => Ok(TextDocumentSyncKind::Incremental),
        _ => Err(ProtocolError::InvalidResponse(format!(
            "unsupported textDocumentSync change kind: {kind}"
        ))),
    }
}

/// Decode one hexadecimal ASCII digit from a percent-encoded URI.
fn decode_hex_digit(byte: u8) -> Result<u8, ProtocolError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(ProtocolError::InvalidResponse(
            "invalid percent-encoded URI byte".to_string(),
        )),
    }
}

/// Parse one Location or LocationLink payload into normalized locations.
fn parse_location_like(
    value: &JsonValue,
    locations: &mut Vec<LspLocation>,
) -> Result<(), ProtocolError> {
    if let Some(uri) = value["uri"].as_str() {
        locations.push(LspLocation {
            uri: uri.to_string(),
            line: value["range"]["start"]["line"].as_usize().ok_or_else(|| {
                ProtocolError::InvalidResponse("missing range.start.line".to_string())
            })?,
            character: value["range"]["start"]["character"]
                .as_usize()
                .ok_or_else(|| {
                    ProtocolError::InvalidResponse("missing range.start.character".to_string())
                })?,
        });
        return Ok(());
    }
    if let Some(uri) = value["targetUri"].as_str() {
        locations.push(LspLocation {
            uri: uri.to_string(),
            line: value["targetSelectionRange"]["start"]["line"]
                .as_usize()
                .ok_or_else(|| {
                    ProtocolError::InvalidResponse(
                        "missing targetSelectionRange.start.line".to_string(),
                    )
                })?,
            character: value["targetSelectionRange"]["start"]["character"]
                .as_usize()
                .ok_or_else(|| {
                    ProtocolError::InvalidResponse(
                        "missing targetSelectionRange.start.character".to_string(),
                    )
                })?,
        });
        return Ok(());
    }
    Err(ProtocolError::InvalidResponse(
        "definition payload is missing uri/targetUri".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use test_utils::TempTree;

    /// Return one fixture path used by protocol tests.
    fn fixture_path() -> std::path::PathBuf {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("src/main.rs", "fn main() {}\n")
            .expect("write Rust file");
        tree.path().join("src/main.rs")
    }

    #[test]
    fn test_write_and_read_message_round_trip() {
        let payload = object! {
            jsonrpc: "2.0",
            id: 7,
            result: {
                uri: "file:///tmp/main.rs",
                range: {
                    start: { line: 1, character: 2 }
                }
            }
        };
        let mut output = Vec::new();
        write_message(&mut output, &payload).expect("write message");

        let message = read_message(&mut Cursor::new(output)).expect("read message");

        assert!(matches!(message, ServerMessage::Response { id: 7, .. }));
    }

    #[test]
    fn test_parse_definition_result_handles_location_arrays() {
        let parsed = json::parse(
            r#"[{"uri":"file:///tmp/lib.rs","range":{"start":{"line":4,"character":9}}}]"#,
        )
        .expect("parse definition result");

        let locations = parse_definition_result(Some(&parsed)).expect("locations");

        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].line, 4);
        assert_eq!(locations[0].character, 9);
    }

    #[test]
    fn test_parse_definition_result_handles_location_links() {
        let parsed = json::parse(
            r#"[{"targetUri":"file:///tmp/lib.rs","targetSelectionRange":{"start":{"line":2,"character":3}}}]"#,
        )
        .expect("parse location link");

        let locations = parse_definition_result(Some(&parsed)).expect("locations");

        assert_eq!(locations[0].line, 2);
        assert_eq!(locations[0].character, 3);
    }

    #[test]
    fn test_parse_definition_result_handles_single_location_object() {
        let parsed = json::parse(
            r#"{"uri":"file:///tmp/lib.rs","range":{"start":{"line":7,"character":11}}}"#,
        )
        .expect("parse definition result");

        let locations = parse_definition_result(Some(&parsed)).expect("locations");

        assert_eq!(
            locations,
            vec![LspLocation {
                uri: "file:///tmp/lib.rs".to_string(),
                line: 7,
                character: 11,
            }]
        );
    }

    #[test]
    fn test_definition_request_uses_file_uri() {
        let path = fixture_path();
        let request = definition_request(
            9,
            &path,
            LspPosition {
                line: 3,
                character: 5,
            },
        );

        assert_eq!(request["id"].as_i32(), Some(9));
        assert_eq!(
            request["params"]["textDocument"]["uri"].as_str(),
            Some(path_to_file_uri(&path).as_str())
        );
        assert_eq!(request["params"]["position"]["line"].as_usize(), Some(3));
        assert_eq!(
            request["params"]["position"]["character"].as_usize(),
            Some(5)
        );
    }

    #[test]
    fn test_did_change_notification_uses_incremental_ranges() {
        let path = fixture_path();
        let payload = did_change_notification(
            &path,
            3,
            &[LspTextChange {
                range: Some(LspRange {
                    start: LspPosition {
                        line: 1,
                        character: 2,
                    },
                    end: LspPosition {
                        line: 1,
                        character: 4,
                    },
                }),
                text: "xy".to_string(),
            }],
        );

        assert_eq!(
            payload["params"]["contentChanges"][0]["range"]["start"]["line"].as_usize(),
            Some(1)
        );
        assert_eq!(
            payload["params"]["contentChanges"][0]["range"]["end"]["character"].as_usize(),
            Some(4)
        );
        assert_eq!(
            payload["params"]["contentChanges"][0]["text"].as_str(),
            Some("xy")
        );
    }

    #[test]
    fn test_parse_text_document_sync_kind_supports_incremental_options() {
        let parsed =
            json::parse(r#"{"capabilities":{"textDocumentSync":{"openClose":true,"change":2}}}"#)
                .expect("parse initialize result");

        assert_eq!(
            parse_text_document_sync_kind(Some(&parsed)).expect("parse sync kind"),
            TextDocumentSyncKind::Incremental
        );
    }

    #[test]
    fn test_parse_text_document_sync_kind_defaults_to_full_when_omitted() {
        let parsed = json::parse(r#"{"capabilities":{}}"#).expect("parse initialize result");

        assert_eq!(
            parse_text_document_sync_kind(Some(&parsed)).expect("default sync kind"),
            TextDocumentSyncKind::Full
        );
    }

    #[test]
    fn test_path_to_file_uri_preserves_unreserved_bytes() {
        let path = Path::new("/tmp/Alpha-09_/main.rs");

        assert_eq!(path_to_file_uri(path), "file:///tmp/Alpha-09_/main.rs");
    }

    #[test]
    fn test_path_to_file_uri_percent_encodes_reserved_bytes() {
        let path = Path::new("/tmp/needs encoding #%?.rs");

        assert_eq!(
            path_to_file_uri(path),
            "file:///tmp/needs%20encoding%20%23%25%3F.rs"
        );
    }

    #[test]
    fn test_path_to_file_uri_round_trips_utf8_paths() {
        let path = Path::new("/tmp/cafe-\u{00E9}/snowman-\u{2603}.rs");
        let uri = path_to_file_uri(path);

        assert_eq!(
            file_uri_to_path(&uri).expect("decode utf8 path"),
            PathBuf::from(path)
        );
    }

    #[test]
    fn test_path_to_file_uri_round_trips_brackets_and_plus_signs() {
        let path = Path::new("/tmp/[module]+extra.rs");
        let uri = path_to_file_uri(path);

        assert_eq!(
            file_uri_to_path(&uri).expect("decode reserved path"),
            PathBuf::from(path)
        );
    }
}
