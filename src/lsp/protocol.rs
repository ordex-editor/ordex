//! Narrow JSON-RPC and LSP message helpers for LSP-backed editor features.

use super::diagnostics::{
    DiagnosticTransport, LspDiagnostic, LspDiagnosticSeverity, LspFileDiagnostics,
};
use json::{JsonValue, object};
use std::borrow::Cow;
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

/// Server-advertised save-notification behavior for one open document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextDocumentSaveOptions {
    /// Whether `didSave` should include the saved document contents.
    pub(crate) include_text: bool,
}

/// Negotiated text-document synchronization behavior for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextDocumentSyncOptions {
    /// Whether the server accepts `didOpen` / `didClose` ownership notifications.
    pub(crate) open_close: bool,
    /// How edits should be synchronized after open.
    pub(crate) change: TextDocumentSyncKind,
    /// Whether the server wants `didSave`, and if so whether it wants text included.
    pub(crate) save: Option<TextDocumentSaveOptions>,
}

impl Default for TextDocumentSyncOptions {
    /// Return compatibility defaults for servers that omit sync capabilities.
    fn default() -> Self {
        Self {
            open_close: true,
            change: TextDocumentSyncKind::Full,
            save: None,
        }
    }
}

/// Server-advertised support for pull-based document diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentDiagnosticProvider {
    /// Optional identifier that the client should echo in diagnostic requests.
    pub(crate) identifier: Option<String>,
}

/// One parsed pull-diagnostics report with its reusable server result id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentDiagnosticReport {
    /// Optional opaque result id the client should feed back on the next pull.
    pub(crate) result_id: Option<String>,
    /// Full replacement diagnostics snapshot, or `None` when the server replied unchanged.
    pub(crate) diagnostics: Option<LspFileDiagnostics>,
}

/// One JSON-RPC response error returned by the language server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspResponseError {
    /// Standard or implementation-defined JSON-RPC/LSP error code.
    pub(crate) code: i32,
    /// Human-readable error message supplied by the server.
    pub(crate) message: String,
}

/// One typed `$/progress` notification emitted by the language server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LspProgressNotification {
    Begin {
        token: String,
        title: String,
        message: Option<String>,
        percentage: Option<u8>,
    },
    Report {
        token: String,
        message: Option<String>,
        percentage: Option<u8>,
    },
    End {
        token: String,
        message: Option<String>,
    },
}

/// One file location returned by a navigation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspLocation {
    /// Canonical file URI for the target document.
    pub(crate) uri: String,
    /// Zero-based line index.
    pub(crate) line: usize,
    /// Zero-based UTF-16 code-unit column.
    pub(crate) character: usize,
}

/// One textual replacement inside a workspace edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspTextEdit {
    /// Replaced span in zero-based LSP coordinates.
    pub(crate) range: LspRange,
    /// Replacement text for the edited span.
    pub(crate) new_text: String,
}

/// One document-local edit batch inside a workspace edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspDocumentEdit {
    /// Canonical filesystem path for the edited document.
    pub(crate) path: PathBuf,
    /// Ordered edits returned for that document.
    pub(crate) edits: Vec<LspTextEdit>,
}

/// One rename/apply-edit payload returned by the language server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspWorkspaceEdit {
    /// Per-document edit groups that the client must apply.
    pub(crate) document_edits: Vec<LspDocumentEdit>,
}

/// One server response decoded into the subset Ordex needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerMessage {
    Response {
        id: u64,
        result: Option<JsonValue>,
        error: Option<LspResponseError>,
    },
    Request {
        id: u64,
        method: String,
        params: Option<JsonValue>,
    },
    Notification {
        method: String,
        params: Option<JsonValue>,
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
    UnsupportedWorkspaceEdit(String),
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
            Self::UnsupportedWorkspaceEdit(error) => {
                write!(f, "unsupported workspace edit: {error}")
            }
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

    if let Some(method) = parsed["method"].as_str()
        && let Some(id) = parsed["id"].as_u64()
    {
        let params = (!parsed["params"].is_null()).then(|| parsed["params"].clone());
        return Ok(ServerMessage::Request {
            id,
            method: method.to_string(),
            params,
        });
    }
    if let Some(method) = parsed["method"].as_str() {
        let params = (!parsed["params"].is_null()).then(|| parsed["params"].clone());
        return Ok(ServerMessage::Notification {
            method: method.to_string(),
            params,
        });
    }
    if let Some(id) = parsed["id"].as_u64() {
        let result = (!parsed["result"].is_null()).then(|| parsed["result"].clone());
        let error = if parsed["error"].is_null() {
            None
        } else {
            Some(LspResponseError {
                code: parsed["error"]["code"].as_i32().ok_or_else(|| {
                    ProtocolError::InvalidResponse("response error is missing code".to_string())
                })?,
                message: parsed["error"]["message"]
                    .as_str()
                    .unwrap_or("LSP error")
                    .to_string(),
            })
        };
        return Ok(ServerMessage::Response { id, result, error });
    }
    Err(ProtocolError::InvalidResponse(
        "message is missing both id and method".to_string(),
    ))
}

/// Build one success response for a server-initiated request.
pub(crate) fn server_request_response(id: u64, result: JsonValue) -> JsonValue {
    object! {
        jsonrpc: "2.0",
        id: id,
        result: result,
    }
}

/// Build one best-effort result for an incoming server request.
pub(crate) fn server_request_result(method: &str, params: Option<&JsonValue>) -> JsonValue {
    if method == "workspace/applyEdit" {
        return object! {
            applied: true
        };
    }
    if method != "workspace/configuration" {
        return JsonValue::Null;
    }

    // Some language servers ask for configuration items during startup. Reply
    // with one `null` entry per requested item so the request completes without
    // requiring Ordex to implement a full configuration surface.
    let item_count = params
        .map(|params| params["items"].members().count())
        .unwrap_or(0);
    JsonValue::Array(vec![JsonValue::Null; item_count])
}

/// Decode one `workspace/applyEdit` request into a client-side workspace edit.
pub(crate) fn parse_apply_edit_request(
    params: Option<&JsonValue>,
) -> Result<LspWorkspaceEdit, ProtocolError> {
    let params = params.ok_or_else(|| {
        ProtocolError::InvalidResponse("workspace/applyEdit request is missing params".to_string())
    })?;
    parse_workspace_edit(&params["edit"])
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
            capabilities: {
                window: {
                    workDoneProgress: true,
                },
                workspace: {
                    applyEdit: true,
                    workspaceEdit: {
                        documentChanges: true
                    }
                },
                textDocument: {
                    synchronization: {
                        didSave: true
                    },
                    diagnostic: {
                        dynamicRegistration: false
                    },
                    publishDiagnostics: {
                        versionSupport: true
                    },
                    rename: {
                        dynamicRegistration: false
                    }
                }
            },
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
    let content_changes = changes
        .iter()
        .map(|change| {
            if let Some(range) = change.range {
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
            }
        })
        .collect();
    object! {
        jsonrpc: "2.0",
        method: "textDocument/didChange",
        params: {
            textDocument: {
                uri: uri.as_str(),
                version: version
            },
            contentChanges: JsonValue::Array(content_changes)
        }
    }
}

/// Build the `didSave` notification payload for one saved buffer snapshot.
pub(crate) fn did_save_notification(path: &Path, text: Option<&str>) -> JsonValue {
    let uri = path_to_file_uri(path);
    let mut params = object! {
        textDocument: {
            uri: uri.as_str()
        }
    };
    if let Some(text) = text {
        // Save notifications include the whole snapshot only when the server
        // explicitly asked for it during initialize negotiation.
        params["text"] = JsonValue::String(text.to_string());
    }
    object! {
        jsonrpc: "2.0",
        method: "textDocument/didSave",
        params: params
    }
}

/// Build the `didClose` notification payload for one tracked document.
pub(crate) fn did_close_notification(path: &Path) -> JsonValue {
    let uri = path_to_file_uri(path);
    object! {
        jsonrpc: "2.0",
        method: "textDocument/didClose",
        params: {
            textDocument: {
                uri: uri.as_str()
            }
        }
    }
}

/// Parse one initialize response and return the negotiated text sync behavior.
pub(crate) fn parse_text_document_sync_options(
    result: Option<&JsonValue>,
) -> Result<TextDocumentSyncOptions, ProtocolError> {
    let capabilities = result.ok_or_else(|| {
        ProtocolError::InvalidResponse("initialize result is missing capabilities".to_string())
    })?;
    let sync = &capabilities["capabilities"]["textDocumentSync"];

    // Keep compatibility with servers that omit the field entirely by falling
    // back to the previous whole-document behavior.
    if sync.is_null() {
        return Ok(TextDocumentSyncOptions::default());
    }
    if let Some(kind) = sync.as_u8() {
        return Ok(TextDocumentSyncOptions {
            change: parse_sync_kind(kind)?,
            ..TextDocumentSyncOptions::default()
        });
    }
    if sync.is_object() {
        // Older servers often omit individual object fields, so parse each one
        // independently and keep compatibility defaults for anything absent.
        let change = match sync["change"].as_u8() {
            Some(kind) => parse_sync_kind(kind)?,
            None => TextDocumentSyncKind::Full,
        };
        let save = match &sync["save"] {
            JsonValue::Boolean(true) => Some(TextDocumentSaveOptions {
                include_text: false,
            }),
            JsonValue::Boolean(false) | JsonValue::Null => None,
            value if value.is_object() => Some(TextDocumentSaveOptions {
                include_text: value["includeText"].as_bool().unwrap_or(false),
            }),
            _ => {
                return Err(ProtocolError::InvalidResponse(
                    "textDocumentSync.save is neither a boolean nor an object".to_string(),
                ));
            }
        };
        return Ok(TextDocumentSyncOptions {
            open_close: sync["openClose"].as_bool().unwrap_or(true),
            change,
            save,
        });
    }
    Err(ProtocolError::InvalidResponse(
        "textDocumentSync is neither a number nor an object".to_string(),
    ))
}

/// Parse one initialize response and return the negotiated text sync mode.
#[cfg(test)]
pub(crate) fn parse_text_document_sync_kind(
    result: Option<&JsonValue>,
) -> Result<TextDocumentSyncKind, ProtocolError> {
    Ok(parse_text_document_sync_options(result)?.change)
}

/// Parse one initialize response and return pull-diagnostics support, if any.
pub(crate) fn parse_document_diagnostic_provider(
    result: Option<&JsonValue>,
) -> Result<Option<DocumentDiagnosticProvider>, ProtocolError> {
    let capabilities = result.ok_or_else(|| {
        ProtocolError::InvalidResponse("initialize result is missing capabilities".to_string())
    })?;
    let provider = &capabilities["capabilities"]["diagnosticProvider"];
    if provider.is_null() {
        return Ok(None);
    }
    // Dynamic registrations are not implemented here, so only accept the
    // initialize-time object shape that the current client wiring can honor.
    if !provider.is_object() {
        return Err(ProtocolError::InvalidResponse(
            "diagnosticProvider is not an object".to_string(),
        ));
    }
    Ok(Some(DocumentDiagnosticProvider {
        identifier: provider["identifier"].as_str().map(ToString::to_string),
    }))
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

/// Build the go-to-references request payload.
pub(crate) fn references_request(id: u64, path: &Path, position: LspPosition) -> JsonValue {
    let uri = path_to_file_uri(path);
    object! {
        jsonrpc: "2.0",
        id: id,
        method: "textDocument/references",
        params: {
            textDocument: {
                uri: uri.as_str()
            },
            position: {
                line: position.line,
                character: position.character
            },
            context: {
                includeDeclaration: false
            }
        }
    }
}

/// Build the pull-diagnostics request payload for one already-synchronized document.
pub(crate) fn document_diagnostic_request(
    id: u64,
    path: &Path,
    identifier: Option<&str>,
    previous_result_id: Option<&str>,
) -> JsonValue {
    let uri = path_to_file_uri(path);
    let mut params = object! {
        textDocument: {
            uri: uri.as_str()
        }
    };
    if let Some(identifier) = identifier {
        // The optional identifier lets the server correlate this request with
        // the diagnostic collection it advertised during initialize.
        params["identifier"] = JsonValue::String(identifier.to_string());
    }
    if let Some(previous_result_id) = previous_result_id {
        // Pull-diagnostics servers can use the prior result id to decide whether
        // the current document still needs a full replacement snapshot.
        params["previousResultId"] = JsonValue::String(previous_result_id.to_string());
    }
    object! {
        jsonrpc: "2.0",
        id: id,
        method: "textDocument/diagnostic",
        params: params
    }
}

/// Build the hover request payload.
pub(crate) fn hover_request(id: u64, path: &Path, position: LspPosition) -> JsonValue {
    let uri = path_to_file_uri(path);
    object! {
        jsonrpc: "2.0",
        id: id,
        method: "textDocument/hover",
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

/// Build the rename request payload.
pub(crate) fn rename_request(
    id: u64,
    path: &Path,
    position: LspPosition,
    new_name: &str,
) -> JsonValue {
    let uri = path_to_file_uri(path);
    object! {
        jsonrpc: "2.0",
        id: id,
        method: "textDocument/rename",
        params: {
            textDocument: {
                uri: uri.as_str()
            },
            position: {
                line: position.line,
                character: position.character
            },
            newName: new_name
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

/// Decode one location-bearing response payload into normalized locations.
pub(crate) fn parse_location_result(
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

/// Decode one hover response payload into display-ready text.
pub(crate) fn parse_hover_result(
    result: Option<&JsonValue>,
) -> Result<Option<Cow<'_, str>>, ProtocolError> {
    let Some(result) = result else {
        return Ok(None);
    };
    if result.is_null() {
        return Ok(None);
    }
    let text = parse_hover_contents(&result["contents"])?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else if trimmed.len() == text.len() {
        Ok(Some(text))
    } else {
        Ok(Some(Cow::Owned(trimmed.to_string())))
    }
}

/// Decode one rename/apply-edit response payload into client-side document edits.
pub(crate) fn parse_workspace_edit_result(
    result: Option<&JsonValue>,
) -> Result<Option<LspWorkspaceEdit>, ProtocolError> {
    let Some(result) = result else {
        return Ok(None);
    };
    if result.is_null() {
        return Ok(None);
    }
    Ok(Some(parse_workspace_edit(result)?))
}

/// Decode one `$/progress` notification into the subset Ordex renders.
pub(crate) fn parse_progress_notification(
    method: &str,
    params: Option<&JsonValue>,
) -> Result<Option<LspProgressNotification>, ProtocolError> {
    if method != "$/progress" {
        return Ok(None);
    }

    let params = params.ok_or_else(|| {
        ProtocolError::InvalidResponse("$/progress notification is missing params".to_string())
    })?;
    let token = parse_progress_token(&params["token"])?;
    let value = &params["value"];
    let kind = value["kind"].as_str().ok_or_else(|| {
        ProtocolError::InvalidResponse("$/progress value is missing kind".to_string())
    })?;
    let message = value["message"].as_str().map(str::to_string);
    let percentage = parse_progress_percentage(&value["percentage"])?;

    // Each progress kind has a stable field subset. Ordex keeps the raw token so
    // later report/end notifications can update the same in-flight task.
    let notification = match kind {
        "begin" => LspProgressNotification::Begin {
            token,
            title: value["title"]
                .as_str()
                .ok_or_else(|| {
                    ProtocolError::InvalidResponse("progress begin is missing title".to_string())
                })?
                .to_string(),
            message,
            percentage,
        },
        "report" => LspProgressNotification::Report {
            token,
            message,
            percentage,
        },
        "end" => LspProgressNotification::End { token, message },
        other => {
            return Err(ProtocolError::InvalidResponse(format!(
                "unsupported progress kind: {other}"
            )));
        }
    };
    Ok(Some(notification))
}

/// Decode one `textDocument/publishDiagnostics` notification into normalized diagnostics.
pub(crate) fn parse_publish_diagnostics_notification(
    method: &str,
    params: Option<&JsonValue>,
) -> Result<Option<LspFileDiagnostics>, ProtocolError> {
    if method != "textDocument/publishDiagnostics" {
        return Ok(None);
    }

    let params = params.ok_or_else(|| {
        ProtocolError::InvalidResponse(
            "textDocument/publishDiagnostics notification is missing params".to_string(),
        )
    })?;
    let uri = params["uri"].as_str().ok_or_else(|| {
        ProtocolError::InvalidResponse("publishDiagnostics notification is missing uri".to_string())
    })?;
    let diagnostics = params["diagnostics"]
        .members()
        .map(parse_diagnostic)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(LspFileDiagnostics::new(
        file_uri_to_path(uri)?,
        params["version"].as_i32(),
        diagnostics,
    )))
}

/// Decode one pull-diagnostics response into a document-local diagnostics snapshot.
///
/// Returns one report carrying the server's reusable `resultId` plus either a
/// full replacement diagnostics snapshot or an unchanged marker, and returns
/// `Err` when the response is missing required fields or uses an unsupported shape.
pub(crate) fn parse_document_diagnostic_report(
    result: Option<&JsonValue>,
    file_path: &Path,
    version: i32,
) -> Result<DocumentDiagnosticReport, ProtocolError> {
    let result = result.ok_or_else(|| {
        ProtocolError::InvalidResponse("document diagnostic result is missing".to_string())
    })?;
    let kind = result["kind"].as_str().ok_or_else(|| {
        ProtocolError::InvalidResponse(
            "document diagnostic result is missing report kind".to_string(),
        )
    })?;
    let result_id = result["resultId"].as_str().map(ToString::to_string);
    match kind {
        "full" => {
            // Pull diagnostics replace the client's current snapshot exactly the
            // same way pushed diagnostics do, including clearing on empty items.
            let diagnostics = result["items"]
                .members()
                .map(parse_diagnostic)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DocumentDiagnosticReport {
                result_id,
                diagnostics: Some(LspFileDiagnostics::with_transport(
                    file_path.to_path_buf(),
                    Some(version),
                    diagnostics,
                    DiagnosticTransport::Pull,
                )),
            })
        }
        "unchanged" => Ok(DocumentDiagnosticReport {
            result_id,
            diagnostics: None,
        }),
        other => Err(ProtocolError::InvalidResponse(format!(
            "unsupported document diagnostic report kind: {other}"
        ))),
    }
}

/// Convert one filesystem path into a `file://` URI.
pub(crate) fn path_to_file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().as_bytes() {
        match byte {
            // Preserve RFC 3986 unreserved bytes plus `/` so ordinary Unix paths
            // stay readable and the server receives a standard file URI.
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

/// Convert one progress token into a stable string key.
fn parse_progress_token(value: &JsonValue) -> Result<String, ProtocolError> {
    if let Some(token) = value.as_str() {
        return Ok(token.to_string());
    }
    if let Some(token) = value.as_u64() {
        return Ok(token.to_string());
    }
    if let Some(token) = value.as_i64() {
        return Ok(token.to_string());
    }
    Err(ProtocolError::InvalidResponse(
        "progress token is neither a string nor an integer".to_string(),
    ))
}

/// Convert one optional progress percentage into a bounded integer.
fn parse_progress_percentage(value: &JsonValue) -> Result<Option<u8>, ProtocolError> {
    if value.is_null() {
        return Ok(None);
    }
    let percentage = value.as_usize().ok_or_else(|| {
        ProtocolError::InvalidResponse("progress percentage is not an integer".to_string())
    })?;
    Ok(Some(percentage.min(100) as u8))
}

/// Decode one published diagnostic entry into the normalized local model.
fn parse_diagnostic(value: &JsonValue) -> Result<LspDiagnostic, ProtocolError> {
    let severity = match value["severity"].as_u8() {
        Some(1) | None => LspDiagnosticSeverity::Error,
        Some(2) => LspDiagnosticSeverity::Warning,
        Some(3) => LspDiagnosticSeverity::Information,
        Some(4) => LspDiagnosticSeverity::Hint,
        Some(other) => {
            return Err(ProtocolError::InvalidResponse(format!(
                "unsupported diagnostic severity: {other}"
            )));
        }
    };
    let code = if let Some(code) = value["code"].as_str() {
        Some(code.to_string())
    } else {
        value["code"].as_i32().map(|code| code.to_string())
    };
    Ok(LspDiagnostic {
        range: LspRange {
            start: LspPosition {
                line: value["range"]["start"]["line"].as_usize().ok_or_else(|| {
                    ProtocolError::InvalidResponse(
                        "publishDiagnostics entry is missing range.start.line".to_string(),
                    )
                })?,
                character: value["range"]["start"]["character"]
                    .as_usize()
                    .ok_or_else(|| {
                        ProtocolError::InvalidResponse(
                            "publishDiagnostics entry is missing range.start.character".to_string(),
                        )
                    })?,
            },
            end: LspPosition {
                line: value["range"]["end"]["line"].as_usize().ok_or_else(|| {
                    ProtocolError::InvalidResponse(
                        "publishDiagnostics entry is missing range.end.line".to_string(),
                    )
                })?,
                character: value["range"]["end"]["character"]
                    .as_usize()
                    .ok_or_else(|| {
                        ProtocolError::InvalidResponse(
                            "publishDiagnostics entry is missing range.end.character".to_string(),
                        )
                    })?,
            },
        },
        severity,
        message: value["message"]
            .as_str()
            .ok_or_else(|| {
                ProtocolError::InvalidResponse(
                    "publishDiagnostics entry is missing message".to_string(),
                )
            })?
            .trim()
            .to_string(),
        source: value["source"].as_str().map(str::to_string),
        code,
    })
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
        "location payload is missing uri/targetUri".to_string(),
    ))
}

/// Decode one workspace edit object into the subset Ordex applies locally.
fn parse_workspace_edit(value: &JsonValue) -> Result<LspWorkspaceEdit, ProtocolError> {
    let mut document_edits = Vec::new();
    if value["changes"].is_object() {
        // The simple `changes` map is enough for many servers, so support it
        // directly before falling back to the richer `documentChanges` shape.
        for (uri, edits) in value["changes"].entries() {
            document_edits.push(parse_workspace_change_entry(uri, edits)?);
        }
    }
    if value["documentChanges"].is_array() {
        for change in value["documentChanges"].members() {
            document_edits.push(parse_document_change(change)?);
        }
    }
    Ok(LspWorkspaceEdit { document_edits })
}

/// Decode one `changes` map entry into a document-local edit batch.
fn parse_workspace_change_entry(
    uri: &str,
    edits: &JsonValue,
) -> Result<LspDocumentEdit, ProtocolError> {
    let path = file_uri_to_path(uri)?;
    let edits = parse_text_edits(edits)?;
    Ok(LspDocumentEdit { path, edits })
}

/// Decode one `documentChanges` entry into a document-local edit batch.
fn parse_document_change(value: &JsonValue) -> Result<LspDocumentEdit, ProtocolError> {
    if let Some(kind) = value["kind"].as_str() {
        return Err(ProtocolError::UnsupportedWorkspaceEdit(kind.to_string()));
    }
    let uri = value["textDocument"]["uri"].as_str().ok_or_else(|| {
        ProtocolError::InvalidResponse(
            "documentChanges entry is missing textDocument.uri".to_string(),
        )
    })?;
    let path = file_uri_to_path(uri)?;
    let edits = parse_text_edits(&value["edits"])?;
    Ok(LspDocumentEdit { path, edits })
}

/// Decode one text-edit array into strongly typed ranged replacements.
fn parse_text_edits(value: &JsonValue) -> Result<Vec<LspTextEdit>, ProtocolError> {
    if !value.is_array() {
        return Err(ProtocolError::InvalidResponse(
            "workspace edit entry is missing an edits array".to_string(),
        ));
    }
    let mut edits = Vec::new();
    for edit in value.members() {
        edits.push(parse_text_edit(edit)?);
    }
    Ok(edits)
}

/// Decode one LSP text edit into the local replacement shape.
fn parse_text_edit(value: &JsonValue) -> Result<LspTextEdit, ProtocolError> {
    let start = parse_position(&value["range"]["start"], "range.start")?;
    let end = parse_position(&value["range"]["end"], "range.end")?;
    let new_text = value["newText"].as_str().ok_or_else(|| {
        ProtocolError::InvalidResponse("text edit is missing newText".to_string())
    })?;
    Ok(LspTextEdit {
        range: LspRange { start, end },
        new_text: new_text.to_string(),
    })
}

/// Decode one JSON position object into zero-based LSP coordinates.
fn parse_position(value: &JsonValue, field_name: &str) -> Result<LspPosition, ProtocolError> {
    Ok(LspPosition {
        line: value["line"]
            .as_usize()
            .ok_or_else(|| ProtocolError::InvalidResponse(format!("missing {field_name}.line")))?,
        character: value["character"].as_usize().ok_or_else(|| {
            ProtocolError::InvalidResponse(format!("missing {field_name}.character"))
        })?,
    })
}

/// Decode one hover `contents` field into plain display text.
fn parse_hover_contents<'a>(value: &'a JsonValue) -> Result<Cow<'a, str>, ProtocolError> {
    if value.is_null() {
        return Ok(Cow::Borrowed(""));
    }
    if let Some(text) = value.as_str() {
        return Ok(Cow::Borrowed(text));
    }
    if value.is_array() {
        // Arrays require joining distinct markup blocks, so borrowed slices are
        // upgraded only for that combined representation.
        let mut blocks = Vec::new();
        for item in value.members() {
            let block = parse_hover_content_block(item)?;
            if !block.is_empty() {
                blocks.push(block.into_owned());
            }
        }
        return Ok(Cow::Owned(blocks.join("\n\n")));
    }
    parse_hover_content_block(value)
}

/// Decode one hover content block from either markup or marked-string form.
fn parse_hover_content_block<'a>(value: &'a JsonValue) -> Result<Cow<'a, str>, ProtocolError> {
    if let Some(text) = value.as_str() {
        return Ok(Cow::Borrowed(text));
    }
    if let Some(text) = value["value"].as_str() {
        return Ok(Cow::Borrowed(text));
    }
    Err(ProtocolError::InvalidResponse(
        "hover contents are missing string/value text".to_string(),
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
            .expect("write fixture file");
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
    fn test_read_message_parses_server_requests_separately_from_notifications() {
        let payload = object! {
            jsonrpc: "2.0",
            id: 11,
            method: "workspace/configuration",
            params: {
                items: [{ section: "test-lsp" }]
            }
        };
        let mut output = Vec::new();
        write_message(&mut output, &payload).expect("write message");

        let message = read_message(&mut Cursor::new(output)).expect("read message");

        assert!(matches!(
            message,
            ServerMessage::Request {
                id: 11,
                ref method,
                ..
            } if method == "workspace/configuration"
        ));
    }

    #[test]
    fn test_read_message_preserves_response_error_codes() {
        let payload = object! {
            jsonrpc: "2.0",
            id: 12,
            error: {
                code: -32800,
                message: "request cancelled"
            }
        };
        let mut output = Vec::new();
        write_message(&mut output, &payload).expect("write message");

        let message = read_message(&mut Cursor::new(output)).expect("read message");

        assert!(matches!(
            message,
            ServerMessage::Response {
                id: 12,
                error: Some(LspResponseError { code: -32800, .. }),
                ..
            }
        ));
    }

    #[test]
    fn test_read_message_keeps_notification_params() {
        let payload = object! {
            jsonrpc: "2.0",
            method: "$/progress",
            params: {
                token: "cargo-index",
                value: {
                    kind: "report",
                    message: "indexing",
                    percentage: 42,
                }
            }
        };
        let mut output = Vec::new();
        write_message(&mut output, &payload).expect("write message");

        let message = read_message(&mut Cursor::new(output)).expect("read message");

        assert!(matches!(
            message,
            ServerMessage::Notification {
                ref method,
                params: Some(_),
            } if method == "$/progress"
        ));
    }

    #[test]
    fn test_server_request_result_returns_null_entries_for_configuration_items() {
        let params = object! {
            items: [
                { section: "test-lsp" },
                { section: "cargo" }
            ]
        };

        let result = server_request_result("workspace/configuration", Some(&params));

        assert_eq!(result.len(), 2);
        assert!(result.members().all(JsonValue::is_null));
    }

    #[test]
    fn test_parse_location_result_handles_location_arrays() {
        let parsed = json::parse(
            r#"[{"uri":"file:///tmp/lib.rs","range":{"start":{"line":4,"character":9}}}]"#,
        )
        .expect("parse definition result");

        let locations = parse_location_result(Some(&parsed)).expect("locations");

        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].line, 4);
        assert_eq!(locations[0].character, 9);
    }

    #[test]
    fn test_parse_location_result_handles_location_links() {
        let parsed = json::parse(
            r#"[{"targetUri":"file:///tmp/lib.rs","targetSelectionRange":{"start":{"line":2,"character":3}}}]"#,
        )
        .expect("parse location link");

        let locations = parse_location_result(Some(&parsed)).expect("locations");

        assert_eq!(locations[0].line, 2);
        assert_eq!(locations[0].character, 3);
    }

    #[test]
    fn test_parse_location_result_handles_single_location_object() {
        let parsed = json::parse(
            r#"{"uri":"file:///tmp/lib.rs","range":{"start":{"line":7,"character":11}}}"#,
        )
        .expect("parse definition result");

        let locations = parse_location_result(Some(&parsed)).expect("locations");

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
    fn test_hover_request_uses_file_uri() {
        let path = fixture_path();
        let request = hover_request(
            12,
            &path,
            LspPosition {
                line: 8,
                character: 13,
            },
        );

        assert_eq!(request["id"].as_i32(), Some(12));
        assert_eq!(
            request["params"]["textDocument"]["uri"].as_str(),
            Some(path_to_file_uri(&path).as_str())
        );
        assert_eq!(request["params"]["position"]["line"].as_usize(), Some(8));
        assert_eq!(
            request["params"]["position"]["character"].as_usize(),
            Some(13)
        );
    }

    #[test]
    fn test_rename_request_uses_file_uri_and_new_name() {
        let path = fixture_path();
        let request = rename_request(
            15,
            &path,
            LspPosition {
                line: 2,
                character: 4,
            },
            "helper_total",
        );

        assert_eq!(request["id"].as_i32(), Some(15));
        assert_eq!(
            request["params"]["textDocument"]["uri"].as_str(),
            Some(path_to_file_uri(&path).as_str())
        );
        assert_eq!(request["params"]["position"]["line"].as_usize(), Some(2));
        assert_eq!(request["params"]["newName"].as_str(), Some("helper_total"));
    }

    #[test]
    fn test_parse_workspace_edit_result_handles_changes_map() {
        let parsed = json::parse(
            r#"{
                "changes": {
                    "file:///tmp/main.rs": [
                        {
                            "range": {
                                "start": { "line": 0, "character": 4 },
                                "end": { "line": 0, "character": 10 }
                            },
                            "newText": "helper_total"
                        }
                    ]
                }
            }"#,
        )
        .expect("parse workspace edit");

        let edit = parse_workspace_edit_result(Some(&parsed))
            .expect("workspace edit")
            .expect("non-null workspace edit");

        assert_eq!(edit.document_edits.len(), 1);
        assert_eq!(edit.document_edits[0].path, PathBuf::from("/tmp/main.rs"));
        assert_eq!(edit.document_edits[0].edits.len(), 1);
        assert_eq!(
            edit.document_edits[0].edits[0].new_text,
            "helper_total".to_string()
        );
    }

    #[test]
    fn test_parse_hover_result_handles_markup_content() {
        let parsed = json::parse(
            r#"{"contents":{"kind":"markdown","value":"```rust\nfn helper_value() -> i32\n```"}}"#,
        )
        .expect("parse hover result");

        let hover = parse_hover_result(Some(&parsed)).expect("hover");

        assert_eq!(
            hover.as_deref(),
            Some("```rust\nfn helper_value() -> i32\n```")
        );
    }

    #[test]
    fn test_parse_hover_result_handles_marked_string_arrays() {
        let parsed = json::parse(
            r#"{"contents":["helper docs",{"language":"rust","value":"fn helper_value() -> i32"}]}"#,
        )
        .expect("parse hover array result");

        let hover = parse_hover_result(Some(&parsed)).expect("hover");

        assert_eq!(
            hover.as_deref(),
            Some("helper docs\n\nfn helper_value() -> i32")
        );
    }

    #[test]
    fn test_parse_hover_result_handles_missing_hover() {
        let parsed = json::parse("null").expect("parse null hover");

        let hover = parse_hover_result(Some(&parsed)).expect("hover");

        assert_eq!(hover, None);
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
    fn test_did_save_notification_includes_text_only_when_requested() {
        let path = fixture_path();
        let with_text = did_save_notification(&path, Some("fn main() {}\n"));
        let without_text = did_save_notification(&path, None);

        assert_eq!(with_text["params"]["text"].as_str(), Some("fn main() {}\n"));
        assert!(without_text["params"]["text"].is_null());
    }

    #[test]
    fn test_did_close_notification_uses_document_uri_only() {
        let path = fixture_path();
        let payload = did_close_notification(&path);

        assert_eq!(
            payload["params"]["textDocument"]["uri"].as_str(),
            Some(path_to_file_uri(&path).as_str())
        );
        assert!(payload["params"]["textDocument"]["version"].is_null());
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
    fn test_parse_text_document_sync_options_reads_save_support() {
        let parsed = json::parse(
            r#"{"capabilities":{"textDocumentSync":{"openClose":false,"change":2,"save":{"includeText":true}}}}"#,
        )
        .expect("parse initialize result");

        assert_eq!(
            parse_text_document_sync_options(Some(&parsed)).expect("parse sync options"),
            TextDocumentSyncOptions {
                open_close: false,
                change: TextDocumentSyncKind::Incremental,
                save: Some(TextDocumentSaveOptions { include_text: true }),
            }
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
    fn test_parse_document_diagnostic_provider_reads_identifier() {
        let parsed =
            json::parse(r#"{"capabilities":{"diagnosticProvider":{"identifier":"test-lsp"}}}"#)
                .expect("parse initialize result");

        assert_eq!(
            parse_document_diagnostic_provider(Some(&parsed)).expect("parse diagnostic provider"),
            Some(DocumentDiagnosticProvider {
                identifier: Some("test-lsp".to_string()),
            })
        );
    }

    #[test]
    fn test_document_diagnostic_request_omits_identifier_when_absent() {
        let path = fixture_path();
        let payload = document_diagnostic_request(9, &path, None, None);

        assert_eq!(
            payload["params"]["textDocument"]["uri"].as_str(),
            Some(path_to_file_uri(&path).as_str())
        );
        assert!(payload["params"]["identifier"].is_null());
        assert!(payload["params"]["previousResultId"].is_null());
    }

    #[test]
    fn test_parse_document_diagnostic_report_builds_snapshot() {
        let parsed = json::parse(
            r#"{
                "kind":"full",
                "resultId":"diag-1",
                "items":[
                    {
                        "range":{
                            "start":{"line":0,"character":3},
                            "end":{"line":0,"character":10}
                        },
                        "severity":1,
                        "message":"cannot find value `missing_three` in this scope",
                        "source":"rustc",
                        "code":"E0425"
                    }
                ]
            }"#,
        )
        .expect("parse document diagnostics");

        let report = parse_document_diagnostic_report(Some(&parsed), Path::new("/tmp/main.rs"), 7)
            .expect("document diagnostics");
        let update = report.diagnostics.expect("diagnostics update");

        assert_eq!(report.result_id.as_deref(), Some("diag-1"));
        assert_eq!(update.file_path, PathBuf::from("/tmp/main.rs"));
        assert_eq!(update.version, Some(7));
        assert_eq!(update.diagnostics.len(), 1);
        assert_eq!(
            update.diagnostics[0].message,
            "cannot find value `missing_three` in this scope"
        );
    }

    #[test]
    fn test_initialize_request_advertises_save_and_diagnostic_version_support() {
        let path = fixture_path();
        let payload = initialize_request(7, &path);

        assert_eq!(
            payload["params"]["capabilities"]["textDocument"]["synchronization"]["didSave"]
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            payload["params"]["capabilities"]["textDocument"]["publishDiagnostics"]
                ["versionSupport"]
                .as_bool(),
            Some(true)
        );
    }

    #[test]
    fn test_parse_progress_notification_handles_begin() {
        let parsed = json::parse(
            r#"{"token":"cargo-index","value":{"kind":"begin","title":"Indexing","message":"crate graph","percentage":5}}"#,
        )
        .expect("parse progress notification");

        assert_eq!(
            parse_progress_notification("$/progress", Some(&parsed)).expect("progress"),
            Some(LspProgressNotification::Begin {
                token: "cargo-index".to_string(),
                title: "Indexing".to_string(),
                message: Some("crate graph".to_string()),
                percentage: Some(5),
            })
        );
    }

    #[test]
    fn test_parse_progress_notification_handles_report_and_end() {
        let report = json::parse(
            r#"{"token":7,"value":{"kind":"report","message":"macros","percentage":73}}"#,
        )
        .expect("parse report");
        let end = json::parse(r#"{"token":7,"value":{"kind":"end","message":"done"}}"#)
            .expect("parse end");

        assert_eq!(
            parse_progress_notification("$/progress", Some(&report)).expect("report"),
            Some(LspProgressNotification::Report {
                token: "7".to_string(),
                message: Some("macros".to_string()),
                percentage: Some(73),
            })
        );
        assert_eq!(
            parse_progress_notification("$/progress", Some(&end)).expect("end"),
            Some(LspProgressNotification::End {
                token: "7".to_string(),
                message: Some("done".to_string()),
            })
        );
    }

    #[test]
    fn test_parse_publish_diagnostics_notification_handles_entries() {
        let parsed = json::parse(
            r#"{
                "uri":"file:///tmp/main.rs",
                "version":12,
                "diagnostics":[
                    {
                        "range":{
                            "start":{"line":1,"character":4},
                            "end":{"line":1,"character":16}
                        },
                        "severity":2,
                        "message":"cannot find value `missing` in this scope",
                        "source":"rustc",
                        "code":"E0425"
                    }
                ]
            }"#,
        )
        .expect("parse diagnostics notification");

        let update = parse_publish_diagnostics_notification(
            "textDocument/publishDiagnostics",
            Some(&parsed),
        )
        .expect("diagnostics")
        .expect("diagnostics update");

        assert_eq!(update.file_path, PathBuf::from("/tmp/main.rs"));
        assert_eq!(update.version, Some(12));
        assert_eq!(update.diagnostics.len(), 1);
        assert_eq!(
            update.diagnostics[0].severity,
            LspDiagnosticSeverity::Warning
        );
        assert_eq!(
            update.diagnostics[0].message,
            "cannot find value `missing` in this scope"
        );
        assert_eq!(update.diagnostics[0].source.as_deref(), Some("rustc"));
        assert_eq!(update.diagnostics[0].code.as_deref(), Some("E0425"));
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
