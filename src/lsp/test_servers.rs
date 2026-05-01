//! Shared fake language-server executables used by LSP unit tests.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use test_utils::TempTree;

/// Write one fake Rust-language server that delays completion responses.
#[cfg(test)]
pub(crate) fn write_fake_rust_analyzer_with_slow_completion(
    tree: &TempTree,
    log_path: &Path,
    completion_delay_ms: u64,
) {
    // The helper logs completion, save, and diagnostic traffic so tests can
    // verify that save work does not stall behind superseded completion requests.
    tree.write_file(
        "rust-analyzer",
        &format!(
            r#"#!/usr/bin/env python3
import json, sys, threading, time
LOG = {log_path:?}
DELAY = {completion_delay_ms} / 1000.0
SEND_LOCK = threading.Lock()
CANCEL_LOCK = threading.Lock()
CANCELLED = set()

def read_message():
    headers = {{}}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b'\r\n', b'\n'):
            break
        name, value = line.decode().split(':', 1)
        headers[name.lower()] = value.strip()
    body = sys.stdin.buffer.read(int(headers['content-length']))
    return json.loads(body)

def send(payload):
    data = json.dumps(payload).encode()
    with SEND_LOCK:
        sys.stdout.buffer.write(f'Content-Length: {{len(data)}}\r\n\r\n'.encode() + data)
        sys.stdout.buffer.flush()

def log(label):
    with open(LOG, 'a', encoding='utf-8') as handle:
        handle.write(f'{{time.monotonic()}} {{label}}\n')

def completion_worker(request_id):
    log('completion-start')
    deadline = time.monotonic() + DELAY
    while time.monotonic() < deadline:
        time.sleep(0.01)
        with CANCEL_LOCK:
            if request_id in CANCELLED:
                log('completion-cancelled')
                send({{'jsonrpc': '2.0', 'id': request_id, 'error': {{'code': -32800, 'message': 'request cancelled'}}}})
                return
    log('completion-end')
    send({{'jsonrpc': '2.0', 'id': request_id, 'result': [{{'label': 'value', 'kind': 6}}]}})

while True:
    message = read_message()
    if message is None:
        break
    method = message.get('method')
    if method == 'initialize':
        send({{'jsonrpc': '2.0', 'id': message['id'], 'result': {{'capabilities': {{'textDocumentSync': {{'openClose': True, 'change': 1, 'save': {{}}}}, 'diagnosticProvider': {{'identifier': 'fake-server'}}, 'completionProvider': {{'triggerCharacters': ['.']}}}}}}}})
    elif method == 'textDocument/completion':
        threading.Thread(target=completion_worker, args=(message['id'],), daemon=True).start()
    elif method == '$/cancelRequest':
        with CANCEL_LOCK:
            CANCELLED.add(message['params']['id'])
        log('cancel')
    elif method == 'textDocument/didChange':
        log('did-change')
    elif method == 'textDocument/didSave':
        log('did-save')
    elif method == 'textDocument/diagnostic':
        log('diagnostic')
        send({{'jsonrpc': '2.0', 'id': message['id'], 'result': {{'kind': 'full', 'resultId': 'fake-result', 'items': []}}}})
    elif method == 'shutdown':
        send({{'jsonrpc': '2.0', 'id': message['id'], 'result': None}})
"#
        ),
    )
    .expect("write fake rust-analyzer");
    let script_path = tree.path().join("rust-analyzer");
    let mut permissions = fs::metadata(&script_path)
        .expect("stat fake rust-analyzer")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod fake rust-analyzer");
}
