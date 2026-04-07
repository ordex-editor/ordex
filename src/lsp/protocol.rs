//! Narrow JSON-RPC and LSP message helpers for Rust go-to-definition.

use json::{JsonValue, object};
use std::fmt;
use std::io::{self, BufRead, Write};
use std::path::Path;

/// One text position in LSP coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LspPosition {
    pub(crate) line: usize,
    pub(crate) character: usize,
}

/// One file location returned by a definition request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspLocation {
    pub(crate) uri: String,
    pub(crate) line: usize,
    pub(crate) character: usize,
}

/// One server response decoded into the subset the MVP needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerMessage {
    Response {
        id: i32,
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

/// Read one complete LSP message and decode the MVP response subset.
pub(crate) fn read_message(reader: &mut impl BufRead) -> Result<ServerMessage, ProtocolError> {
    let content_length = read_content_length(reader)?;
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;
    let parsed = json::parse(
        std::str::from_utf8(&body)
            .map_err(|error| ProtocolError::InvalidJson(error.to_string()))?,
    )
    .map_err(|error| ProtocolError::InvalidJson(error.to_string()))?;

    if let Some(id) = parsed["id"].as_i32() {
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
    if let Some(method) = parsed["method"].as_str() {
        return Ok(ServerMessage::Notification {
            method: method.to_string(),
        });
    }
    Err(ProtocolError::InvalidResponse(
        "message is missing both id and method".to_string(),
    ))
}

/// Build the initialize request payload for rust-analyzer.
pub(crate) fn initialize_request(id: i32, workspace_root: &Path) -> JsonValue {
    object! {
        jsonrpc: "2.0",
        id: id,
        method: "initialize",
        params: {
            processId: std::process::id() as i32,
            rootUri: path_to_file_uri(workspace_root),
            capabilities: {},
            workspaceFolders: [{
                uri: path_to_file_uri(workspace_root),
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
    object! {
        jsonrpc: "2.0",
        method: "textDocument/didOpen",
        params: {
            textDocument: {
                uri: path_to_file_uri(path),
                languageId: "rust",
                version: version,
                text: text
            }
        }
    }
}

/// Build the full-text `didChange` notification payload.
pub(crate) fn did_change_notification(path: &Path, version: i32, text: &str) -> JsonValue {
    object! {
        jsonrpc: "2.0",
        method: "textDocument/didChange",
        params: {
            textDocument: {
                uri: path_to_file_uri(path),
                version: version
            },
            contentChanges: [{
                text: text
            }]
        }
    }
}

/// Build the go-to-definition request payload.
pub(crate) fn definition_request(id: i32, path: &Path, position: LspPosition) -> JsonValue {
    object! {
        jsonrpc: "2.0",
        id: id,
        method: "textDocument/definition",
        params: {
            textDocument: {
                uri: path_to_file_uri(path)
            },
            position: {
                line: position.line,
                character: position.character
            }
        }
    }
}

/// Build the `shutdown` request payload.
pub(crate) fn shutdown_request(id: i32) -> JsonValue {
    object! {
        jsonrpc: "2.0",
        id: id,
        method: "shutdown",
        params: {}
    }
}

/// Build the `exit` notification payload.
pub(crate) fn exit_notification() -> JsonValue {
    object! {
        jsonrpc: "2.0",
        method: "exit",
        params: {}
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
    format!("file://{}", path.display())
}

/// Convert one `file://` URI into a filesystem path.
pub(crate) fn file_uri_to_path(uri: &str) -> Result<std::path::PathBuf, ProtocolError> {
    let Some(path) = uri.strip_prefix("file://") else {
        return Err(ProtocolError::UnsupportedUri(uri.to_string()));
    };
    Ok(std::path::PathBuf::from(path))
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
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
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
    }
}
