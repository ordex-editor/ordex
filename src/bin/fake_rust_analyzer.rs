//! Deterministic stdio LSP stub used by Ordex integration tests.

use json::{JsonValue, object};
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// One decoded message from the fake LSP transport.
enum Message {
    /// One request that expects a matching response id.
    Request {
        id: i32,
        method: String,
        params: JsonValue,
    },
    /// One notification that does not expect a response.
    Notification { method: String, params: JsonValue },
}

/// Run the fake rust-analyzer main loop until the client exits.
fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut documents = std::collections::HashMap::<String, String>::new();

    loop {
        let message = match read_message(&mut reader) {
            Ok(message) => message,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error),
        };
        match message {
            Message::Request { id, method, params } => {
                // Keep request handling synchronous so the tests see a predictable
                // one-request-in, one-response-out ordering on stdio.
                let response = handle_request(id, &method, &params, &documents)?;
                write_message(&mut io::stdout().lock(), &response)?;
            }
            Message::Notification { method, params } => {
                if handle_notification(&method, &params, &mut documents)? {
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Read one LSP-framed JSON message from `reader`.
fn read_message(reader: &mut impl BufRead) -> io::Result<Message> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(value.trim().parse::<usize>().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid Content-Length header: {error}"),
                )
            })?);
        }
    }

    let Some(content_length) = content_length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing Content-Length header",
        ));
    };
    let mut body = vec![0_u8; content_length];
    reader.read_exact(&mut body)?;
    let parsed = json::parse(std::str::from_utf8(&body).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid UTF-8 JSON payload: {error}"),
        )
    })?)
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;

    if let Some(id) = parsed["id"].as_i32() {
        return Ok(Message::Request {
            id,
            method: parsed["method"].as_str().unwrap_or_default().to_string(),
            params: parsed["params"].clone(),
        });
    }
    Ok(Message::Notification {
        method: parsed["method"].as_str().unwrap_or_default().to_string(),
        params: parsed["params"].clone(),
    })
}

/// Write one JSON-RPC payload to `writer` with LSP framing.
fn write_message(writer: &mut impl Write, payload: &JsonValue) -> io::Result<()> {
    let body = payload.dump();
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()
}

/// Handle one fake rust-analyzer request and build the JSON-RPC response.
fn handle_request(
    id: i32,
    method: &str,
    params: &JsonValue,
    documents: &std::collections::HashMap<String, String>,
) -> io::Result<JsonValue> {
    match method {
        "initialize" => {
            log_workspace(params["rootUri"].as_str());
            Ok(object! {
                jsonrpc: "2.0",
                id: id,
                result: {
                    capabilities: {
                        definitionProvider: true
                    }
                }
            })
        }
        "shutdown" => Ok(object! {
            jsonrpc: "2.0",
            id: id,
            result: JsonValue::Null,
        }),
        "textDocument/definition" => {
            // Route definition responses by opened file path so end-to-end tests can
            // drive predictable outcomes without depending on Rust parsing logic.
            let uri = params["textDocument"]["uri"]
                .as_str()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request uri"))?;
            Ok(object! {
                jsonrpc: "2.0",
                id: id,
                result: definition_result(uri, documents)?
            })
        }
        _ => Ok(object! {
            jsonrpc: "2.0",
            id: id,
            result: JsonValue::Null,
        }),
    }
}

/// Apply one notification and return whether the server should exit.
fn handle_notification(
    method: &str,
    params: &JsonValue,
    documents: &mut std::collections::HashMap<String, String>,
) -> io::Result<bool> {
    match method {
        "initialized" => Ok(false),
        "textDocument/didOpen" => {
            let uri = params["textDocument"]["uri"]
                .as_str()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing didOpen uri"))?;
            let text = params["textDocument"]["text"].as_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "missing didOpen text")
            })?;
            documents.insert(uri.to_string(), text.to_string());
            Ok(false)
        }
        "textDocument/didChange" => {
            let uri = params["textDocument"]["uri"].as_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "missing didChange uri")
            })?;
            let text = params["contentChanges"][0]["text"]
                .as_str()
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "missing didChange text")
                })?;
            documents.insert(uri.to_string(), text.to_string());
            Ok(false)
        }
        "exit" => Ok(true),
        _ => Ok(false),
    }
}

/// Build one fake definition response payload for `uri`.
fn definition_result(
    uri: &str,
    documents: &std::collections::HashMap<String, String>,
) -> io::Result<JsonValue> {
    let path = file_uri_to_path(uri)?;
    if path.ends_with(Path::new("workspace_one/src/main.rs")) {
        return Ok(single_location(
            path.parent().expect("main.rs parent").join("lib.rs"),
            0,
            7,
        ));
    }
    if path.ends_with(Path::new("workspace_two/src/main.rs")) {
        return Ok(single_location(
            path.parent().expect("main.rs parent").join("lib.rs"),
            0,
            7,
        ));
    }
    if path.ends_with(Path::new("workspace_picker/src/main.rs")) {
        return Ok(JsonValue::Array(vec![
            single_location(
                path.parent().expect("main.rs parent").join("defs_a.rs"),
                0,
                7,
            ),
            single_location(
                path.parent().expect("main.rs parent").join("defs_b.rs"),
                0,
                7,
            ),
        ]));
    }

    let _ = documents;
    Ok(JsonValue::Null)
}

/// Build one LSP Location object from a filesystem path and zero-based position.
fn single_location(path: PathBuf, line: usize, character: usize) -> JsonValue {
    object! {
        uri: path_to_file_uri(&path),
        range: {
            start: {
                line: line,
                character: character
            }
        }
    }
}

/// Convert one path into the simple `file://` URI form used by the test server.
fn path_to_file_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// Convert one fake-server `file://` URI back into a filesystem path.
fn file_uri_to_path(uri: &str) -> io::Result<PathBuf> {
    uri.strip_prefix("file://")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unsupported URI"))
}

/// Append one initialized workspace root to the optional fake-server log file.
fn log_workspace(root_uri: Option<&str>) {
    let Some(log_path) = std::env::var_os("ORDEX_FAKE_RA_LOG") else {
        return;
    };
    let Some(root_uri) = root_uri else {
        return;
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) else {
        return;
    };
    let _ = writeln!(file, "{root_uri}");
}
