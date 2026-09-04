//! Fake language server that completes the handshake and then stops reading.
//!
//! It answers `initialize` so Ordex treats the session as healthy, then leaves
//! its stdin unread. The editor's next document notification fills the pipe
//! buffer and the write blocks part-way through the payload.
//!
//! Compiled with `rustc` by `tests/lsp_liveness_test.rs`; it is a fixture rather
//! than a Cargo target so it never ends up in what Ordex ships.

use std::io::{BufRead, Read, Write};
use std::thread;
use std::time::Duration;

/// How long the process lingers after answering, holding its stdin unread.
const WEDGE_DURATION: Duration = Duration::from_secs(120);

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

/// Answer `initialize`, then hold stdin unread until the editor gives up on us.
fn main() {
    let mut stdin = std::io::stdin().lock();
    let Some(body) = read_message(&mut stdin) else {
        return;
    };

    let response = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"capabilities\":{{\"textDocumentSync\":1}}}}}}",
        request_id(&body)
    );
    let mut stdout = std::io::stdout();
    if write!(
        stdout,
        "Content-Length: {}\r\n\r\n{}",
        response.len(),
        response
    )
    .and_then(|()| stdout.flush())
    .is_err()
    {
        return;
    }

    thread::sleep(WEDGE_DURATION);
}
