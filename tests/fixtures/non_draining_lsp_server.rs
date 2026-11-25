//! Fake language server that completes the handshake and then stops reading.
//!
//! It answers `initialize` so Ordex treats the session as healthy, then leaves
//! its stdin unread from the point `ORDEX_FAKE_LSP_WEDGE_AFTER` names. The
//! editor's next document notification fills the pipe buffer and the write
//! blocks part-way through the payload. Its definition modes keep reading
//! instead and never answer a request: one holds the definition request
//! forever, the other exits under it.
//!
//! Compiled with `rustc` by `tests/lsp_liveness_test.rs`; it is a fixture rather
//! than a Cargo target so it never ends up in what Ordex ships.

use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

/// How long the process lingers after answering, holding its stdin unread.
const WEDGE_DURATION: Duration = Duration::from_secs(120);

/// Environment variable that selects when the server stops reading its stdin.
const WEDGE_POINT_VARIABLE: &str = "ORDEX_FAKE_LSP_WEDGE_AFTER";

/// Environment variable naming the directory the server signals the test through.
const SIGNAL_DIRECTORY_VARIABLE: &str = "ORDEX_FAKE_LSP_SIGNAL_DIR";

/// File the server creates once it holds a definition request it will not answer.
const REQUEST_PENDING_FILE: &str = "definition-pending";

/// Request id the server uses for the capability registration it sends itself.
const REGISTRATION_REQUEST_ID: u32 = 9001;

/// Point in the conversation at which the server stops reading its stdin.
enum WedgePoint {
    /// Stop as soon as the handshake is answered, before any document arrives.
    Handshake,
    /// Keep reading until the editor opens a document, then stop.
    DocumentOpen,
    /// Read until the document opens, register a watcher for every file in the
    /// workspace, then stop, so saves have somewhere to send notifications.
    WatchedFiles,
    /// Keep reading but never answer, and announce the first definition request
    /// so the test knows a lookup is pending.
    UnansweredDefinition,
    /// Keep reading but never answer, and exit on the first definition request,
    /// which closes the transport under that request.
    ExitOnDefinition,
}

impl WedgePoint {
    /// Read the wedge point from the environment, defaulting to the handshake.
    fn from_environment() -> Self {
        let selection = std::env::var(WEDGE_POINT_VARIABLE).unwrap_or_default();
        match selection.as_str() {
            "did_open" => Self::DocumentOpen,
            "watched_files" => Self::WatchedFiles,
            "unanswered_definition" => Self::UnansweredDefinition,
            "exit_on_definition" => Self::ExitOnDefinition,
            _ => Self::Handshake,
        }
    }
}

/// Read one LSP message and return its body, or `None` at end of stream.
fn read_message(stdin: &mut impl BufRead) -> Option<String> {
    // Header block: `Content-Length` matters, every other field is ignored, and
    // the empty line ends the block.
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if stdin.read_line(&mut line).unwrap_or(0) == 0 {
            return None;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let lowered = trimmed.to_ascii_lowercase();
        if let Some(value) = lowered.strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }

    let mut body = vec![0u8; content_length];
    stdin.read_exact(&mut body).ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

/// Extract the `id` field of one JSON-RPC request body.
///
/// The handshake is the only request this server ever sees, so scanning for the
/// field beats pulling in a JSON parser.
fn request_id(body: &str) -> String {
    body.split("\"id\":")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .map(|value| value.trim().trim_end_matches('}').to_string())
        .unwrap_or_else(|| "1".to_string())
}

/// Write one framed message to stdout.
///
/// Returns `true` when the whole message reached the editor, and `false` when
/// the pipe broke, which means the editor is already gone.
fn send_message(body: &str) -> bool {
    let mut stdout = std::io::stdout();
    write!(stdout, "Content-Length: {}\r\n\r\n{}", body.len(), body)
        .and_then(|()| stdout.flush())
        .is_ok()
}

/// Read messages until the editor opens a document.
///
/// Returns `true` when the `didOpen` arrived, and `false` when the stream ended
/// first, which leaves nothing worth wedging against.
fn read_until_document_open(stdin: &mut impl BufRead) -> bool {
    while let Some(body) = read_message(stdin) {
        if body.contains("textDocument/didOpen") {
            return true;
        }
    }
    false
}

/// Tell the test that a definition request is now pending, when it asked to know.
fn announce_pending_request() {
    if let Some(signal_directory) = std::env::var_os(SIGNAL_DIRECTORY_VARIABLE).map(PathBuf::from)
    {
        let _ = std::fs::write(signal_directory.join(REQUEST_PENDING_FILE), b"");
    }
}

/// Answer `initialize`, then hold stdin unread until the editor gives up on us.
fn main() {
    let wedge_point = WedgePoint::from_environment();
    let mut stdin = std::io::stdin().lock();
    let Some(body) = read_message(&mut stdin) else {
        return;
    };

    let response = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"capabilities\":{{\"textDocumentSync\":1}}}}}}",
        request_id(&body)
    );
    if !send_message(&response) {
        return;
    }

    match wedge_point {
        WedgePoint::Handshake => {}
        WedgePoint::DocumentOpen => {
            if !read_until_document_open(&mut stdin) {
                return;
            }
        }
        WedgePoint::WatchedFiles => {
            if !read_until_document_open(&mut stdin) {
                return;
            }
            let registration = format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{REGISTRATION_REQUEST_ID},\
                 \"method\":\"client/registerCapability\",\
                 \"params\":{{\"registrations\":[{{\"id\":\"watch-all\",\
                 \"method\":\"workspace/didChangeWatchedFiles\",\
                 \"registerOptions\":{{\"watchers\":[{{\"globPattern\":\"**/*\"}}]}}}}]}}}}"
            );
            if !send_message(&registration) {
                return;
            }
        }
        WedgePoint::UnansweredDefinition => {
            // Reading on until the editor closes stdin keeps every later write
            // flowing, so only the unanswered request is left pending.
            while let Some(body) = read_message(&mut stdin) {
                if body.contains("textDocument/definition") {
                    announce_pending_request();
                }
            }
            return;
        }
        WedgePoint::ExitOnDefinition => {
            while let Some(body) = read_message(&mut stdin) {
                if body.contains("textDocument/definition") {
                    // Returning closes both pipes, which is how the editor
                    // learns that this request will never be answered.
                    return;
                }
            }
            return;
        }
    }

    thread::sleep(WEDGE_DURATION);
}
