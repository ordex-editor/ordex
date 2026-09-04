//! Shared fake language-server executables used by LSP unit tests.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use test_utils::TempTree;

/// Describe how the shared fake Rust language server should answer completion requests.
#[cfg(test)]
pub(crate) enum FakeRustAnalyzerCompletionMode<'a> {
    /// Omit completion support from server capabilities and request handling.
    None,
    /// Advertise completion support and return one empty result batch immediately.
    Empty {
        /// Trigger characters advertised in initialize capabilities.
        trigger_characters: &'a [&'a str],
    },
    /// Advertise completion support and return one delayed result that supports cancellation.
    Slow {
        /// Trigger characters advertised in initialize capabilities.
        trigger_characters: &'a [&'a str],
        /// Delay before the worker returns a completion response.
        delay_ms: u64,
    },
}

/// Describe how the shared fake Rust language server should answer definition requests.
#[cfg(test)]
pub(crate) enum FakeRustAnalyzerDefinitionMode {
    /// Omit definition support from server capabilities and request handling.
    None,
    /// Report a workspace that is still loading before it can resolve symbols.
    ///
    /// The server announces a non-quiescent workspace and answers every
    /// definition and signature-help request with `null` while loading, then
    /// announces quiescence and starts answering both for real.
    ColdWorkspace {
        /// How long the workspace stays non-quiescent after initialize.
        loading_ms: u64,
    },
    /// Keep answering definitions emptily for a while after reporting quiescence.
    ///
    /// Real servers announce a loaded workspace before every cross-crate query
    /// can be answered, and they report that gap as an empty location array
    /// rather than `null`.
    EmptyAfterQuiescent {
        /// How long empty arrays keep coming back once the workspace is loaded.
        empty_ms: u64,
    },
}

/// Describe the behavior of the shared fake Rust language server.
#[cfg(test)]
pub(crate) struct FakeRustAnalyzerConfig<'a> {
    /// Log file that records observed request kinds for test assertions.
    pub(crate) log_path: &'a Path,
    /// Completion behavior exposed by the fake server.
    pub(crate) completion_mode: FakeRustAnalyzerCompletionMode<'a>,
    /// Definition behavior exposed by the fake server.
    pub(crate) definition_mode: FakeRustAnalyzerDefinitionMode,
    /// Glob the server dynamically registers for watched-file notifications.
    pub(crate) watched_files_glob: Option<&'a str>,
    /// Whether the server reports one never-ending background progress task.
    pub(crate) background_progress: bool,
    /// Whether initialize should advertise pull diagnostics support.
    pub(crate) diagnostic_provider: bool,
    /// Whether initialize should advertise save support in textDocumentSync.
    pub(crate) include_save_support: bool,
    /// Whether `textDocument/didChange` notifications should be logged.
    pub(crate) log_did_change: bool,
    /// Whether `textDocument/didSave` notifications should be logged.
    pub(crate) log_did_save: bool,
    /// Whether `textDocument/diagnostic` requests should be logged.
    pub(crate) log_diagnostics: bool,
}

impl<'a> FakeRustAnalyzerConfig<'a> {
    /// Build one config that only logs pull-diagnostic requests.
    pub(crate) fn diagnostics_only(log_path: &'a Path) -> Self {
        Self {
            log_path,
            completion_mode: FakeRustAnalyzerCompletionMode::None,
            definition_mode: FakeRustAnalyzerDefinitionMode::None,
            watched_files_glob: None,
            background_progress: false,
            diagnostic_provider: true,
            include_save_support: true,
            log_did_change: false,
            log_did_save: false,
            log_diagnostics: true,
        }
    }

    /// Build one config that returns empty completion batches immediately.
    pub(crate) fn empty_completion(log_path: &'a Path, trigger_characters: &'a [&'a str]) -> Self {
        Self {
            log_path,
            completion_mode: FakeRustAnalyzerCompletionMode::Empty { trigger_characters },
            definition_mode: FakeRustAnalyzerDefinitionMode::None,
            watched_files_glob: None,
            background_progress: false,
            diagnostic_provider: false,
            include_save_support: false,
            log_did_change: false,
            log_did_save: false,
            log_diagnostics: false,
        }
    }

    /// Build one config that delays completion batches and logs sync traffic.
    pub(crate) fn slow_completion(
        log_path: &'a Path,
        trigger_characters: &'a [&'a str],
        delay_ms: u64,
    ) -> Self {
        // Save-priority tests need the full sync + diagnostic surface enabled so
        // they can prove save traffic bypasses cancelled completion work cleanly.
        Self {
            log_path,
            completion_mode: FakeRustAnalyzerCompletionMode::Slow {
                trigger_characters,
                delay_ms,
            },
            definition_mode: FakeRustAnalyzerDefinitionMode::None,
            watched_files_glob: None,
            background_progress: false,
            diagnostic_provider: true,
            include_save_support: true,
            log_did_change: true,
            log_did_save: true,
            log_diagnostics: true,
        }
    }

    /// Build one config whose workspace stays unqueryable for `loading_ms`.
    pub(crate) fn cold_workspace_definition(log_path: &'a Path, loading_ms: u64) -> Self {
        Self {
            log_path,
            completion_mode: FakeRustAnalyzerCompletionMode::None,
            definition_mode: FakeRustAnalyzerDefinitionMode::ColdWorkspace { loading_ms },
            watched_files_glob: None,
            background_progress: false,
            diagnostic_provider: false,
            include_save_support: false,
            log_did_change: false,
            log_did_save: false,
            log_diagnostics: false,
        }
    }

    /// Build one config that reports a loaded workspace before it can answer.
    pub(crate) fn empty_definitions_after_quiescent(log_path: &'a Path, empty_ms: u64) -> Self {
        Self {
            log_path,
            completion_mode: FakeRustAnalyzerCompletionMode::None,
            definition_mode: FakeRustAnalyzerDefinitionMode::EmptyAfterQuiescent { empty_ms },
            watched_files_glob: None,
            background_progress: false,
            diagnostic_provider: false,
            include_save_support: false,
            log_did_change: false,
            log_did_save: false,
            log_diagnostics: false,
        }
    }

    /// Build one config that advertises signature help but never has an answer.
    ///
    /// The workspace never reports itself as loading, so every empty answer
    /// already reflects a server that has nothing to offer at that position.
    pub(crate) fn signature_help_without_answers(log_path: &'a Path) -> Self {
        Self {
            log_path,
            completion_mode: FakeRustAnalyzerCompletionMode::None,
            definition_mode: FakeRustAnalyzerDefinitionMode::ColdWorkspace { loading_ms: 0 },
            watched_files_glob: None,
            background_progress: false,
            diagnostic_provider: false,
            include_save_support: false,
            log_did_change: false,
            log_did_save: false,
            log_diagnostics: false,
        }
    }

    /// Build one config that answers empty while a background task stays running.
    pub(crate) fn empty_completion_during_background_work(
        log_path: &'a Path,
        trigger_characters: &'a [&'a str],
    ) -> Self {
        Self {
            log_path,
            completion_mode: FakeRustAnalyzerCompletionMode::Empty { trigger_characters },
            definition_mode: FakeRustAnalyzerDefinitionMode::None,
            watched_files_glob: None,
            background_progress: true,
            diagnostic_provider: false,
            include_save_support: false,
            log_did_change: false,
            log_did_save: false,
            log_diagnostics: false,
        }
    }

    /// Build one config that registers `glob` for watched-file notifications.
    pub(crate) fn watched_files(log_path: &'a Path, glob: &'a str) -> Self {
        Self {
            log_path,
            completion_mode: FakeRustAnalyzerCompletionMode::None,
            definition_mode: FakeRustAnalyzerDefinitionMode::None,
            watched_files_glob: Some(glob),
            background_progress: false,
            diagnostic_provider: false,
            include_save_support: true,
            log_did_change: false,
            log_did_save: false,
            log_diagnostics: false,
        }
    }
}

/// Render one Python list literal from `items`.
#[cfg(test)]
fn python_list_literal(items: &[&str]) -> String {
    let rendered_items = items
        .iter()
        .map(|item| format!("{item:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rendered_items}]")
}

/// Render one Python boolean literal from `value`.
#[cfg(test)]
fn python_bool_literal(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

/// Render one initialize-capabilities expression for the fake server.
#[cfg(test)]
fn fake_rust_analyzer_capabilities(config: &FakeRustAnalyzerConfig<'_>) -> String {
    // The capability payload mirrors only the fields each test needs so the fake
    // server stays small while still exercising Ordex's real capability parsing.
    let mut capabilities = vec![format!(
        "'textDocumentSync': {{'openClose': True, 'change': 1, 'save': {}}}",
        if config.include_save_support {
            "{}"
        } else {
            "None"
        }
    )];
    if config.diagnostic_provider {
        capabilities.push("'diagnosticProvider': {'identifier': 'fake-server'}".to_string());
    }
    match config.completion_mode {
        FakeRustAnalyzerCompletionMode::None => {}
        FakeRustAnalyzerCompletionMode::Empty { trigger_characters }
        | FakeRustAnalyzerCompletionMode::Slow {
            trigger_characters, ..
        } => capabilities.push(format!(
            "'completionProvider': {{'triggerCharacters': {}}}",
            python_list_literal(trigger_characters)
        )),
    }
    match config.definition_mode {
        FakeRustAnalyzerDefinitionMode::None => {}
        FakeRustAnalyzerDefinitionMode::ColdWorkspace { .. }
        | FakeRustAnalyzerDefinitionMode::EmptyAfterQuiescent { .. } => {
            capabilities.push("'definitionProvider': True".to_string());
            capabilities
                .push("'signatureHelpProvider': {'triggerCharacters': ['(', ',']}".to_string());
        }
    }
    format!("{{{}}}", capabilities.join(", "))
}

/// Write one shared fake Rust language server with behavior selected by `config`.
#[cfg(test)]
pub(crate) fn write_fake_rust_analyzer(tree: &TempTree, config: &FakeRustAnalyzerConfig<'_>) {
    let (completion_mode, completion_delay_ms) = match config.completion_mode {
        FakeRustAnalyzerCompletionMode::None => ("none", 0),
        FakeRustAnalyzerCompletionMode::Empty { .. } => ("empty", 0),
        FakeRustAnalyzerCompletionMode::Slow { delay_ms, .. } => ("slow", delay_ms),
    };
    let workspace_loading_ms = match config.definition_mode {
        FakeRustAnalyzerDefinitionMode::None
        | FakeRustAnalyzerDefinitionMode::EmptyAfterQuiescent { .. } => 0,
        FakeRustAnalyzerDefinitionMode::ColdWorkspace { loading_ms } => loading_ms,
    };
    let empty_after_quiescent_ms = match config.definition_mode {
        FakeRustAnalyzerDefinitionMode::None
        | FakeRustAnalyzerDefinitionMode::ColdWorkspace { .. } => 0,
        FakeRustAnalyzerDefinitionMode::EmptyAfterQuiescent { empty_ms } => empty_ms,
    };
    let watched_files_glob = match config.watched_files_glob {
        Some(glob) => format!("{glob:?}"),
        None => "None".to_string(),
    };
    let capabilities = fake_rust_analyzer_capabilities(config);
    let log_did_change = python_bool_literal(config.log_did_change);
    let log_did_save = python_bool_literal(config.log_did_save);
    let log_diagnostics = python_bool_literal(config.log_diagnostics);
    // One configurable script keeps the fake-server protocol behavior centralized
    // while still letting each test opt into only the traffic it needs to assert.
    tree.write_file(
        "rust-analyzer",
        &format!(
            r#"#!/usr/bin/env python3
import json, sys, threading, time
LOG = {log_path:?}
CAPABILITIES = {capabilities}
COMPLETION_MODE = {completion_mode:?}
DELAY = {completion_delay_ms} / 1000.0
WORKSPACE_LOADING = {workspace_loading_ms} / 1000.0
QUIESCENT_AT = [None]
WATCHED_FILES_GLOB = {watched_files_glob}
BACKGROUND_PROGRESS = {background_progress}
EMPTY_AFTER_QUIESCENT = {empty_after_quiescent_ms} / 1000.0
ANSWERS_AT = [None]
LOG_DID_CHANGE = {log_did_change}
LOG_DID_SAVE = {log_did_save}
LOG_DIAGNOSTIC = {log_diagnostics}
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

def server_status(quiescent):
    send({{'jsonrpc': '2.0', 'method': 'experimental/serverStatus',
          'params': {{'health': 'ok', 'quiescent': quiescent, 'message': None}}}})

def workspace_loader():
    QUIESCENT_AT[0] = time.monotonic() + WORKSPACE_LOADING
    server_status(False)
    while time.monotonic() < QUIESCENT_AT[0]:
        time.sleep(0.01)
    server_status(True)

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
        send({{'jsonrpc': '2.0', 'id': message['id'], 'result': {{'capabilities': CAPABILITIES}}}})
    elif method == 'initialized':
        if WORKSPACE_LOADING > 0:
            threading.Thread(target=workspace_loader, daemon=True).start()
        if EMPTY_AFTER_QUIESCENT > 0:
            ANSWERS_AT[0] = time.monotonic() + EMPTY_AFTER_QUIESCENT
            server_status(True)
        if BACKGROUND_PROGRESS:
            send({{'jsonrpc': '2.0', 'id': 9002, 'method': 'window/workDoneProgress/create',
                  'params': {{'token': 'background'}}}})
            send({{'jsonrpc': '2.0', 'method': '$/progress', 'params': {{
                'token': 'background',
                'value': {{'kind': 'begin', 'title': 'Indexing'}}}}}})
        if WATCHED_FILES_GLOB is not None:
            send({{'jsonrpc': '2.0', 'id': 9001, 'method': 'client/registerCapability',
                  'params': {{'registrations': [{{
                      'id': 'watched-files-1',
                      'method': 'workspace/didChangeWatchedFiles',
                      'registerOptions': {{'watchers': [{{'globPattern': WATCHED_FILES_GLOB}}]}}}}]}}}})
    elif method == 'workspace/didChangeWatchedFiles':
        for change in message['params']['changes']:
            log(f"watched {{change['type']}} {{change['uri']}}")
    elif method == 'textDocument/definition':
        log('definition')
        if EMPTY_AFTER_QUIESCENT > 0:
            # A loaded workspace that cannot answer yet reports an empty array,
            # not `null`, until the cross-crate data it needs is available.
            if time.monotonic() < ANSWERS_AT[0]:
                send({{'jsonrpc': '2.0', 'id': message['id'], 'result': []}})
            else:
                send({{'jsonrpc': '2.0', 'id': message['id'], 'result': [{{
                    'uri': message['params']['textDocument']['uri'],
                    'range': {{'start': {{'line': 0, 'character': 3}},
                              'end': {{'line': 0, 'character': 7}}}}}}]}})
            continue
        loading = QUIESCENT_AT[0] is None or time.monotonic() < QUIESCENT_AT[0]
        if loading:
            send({{'jsonrpc': '2.0', 'id': message['id'], 'result': None}})
        else:
            send({{'jsonrpc': '2.0', 'id': message['id'], 'result': [{{
                'uri': message['params']['textDocument']['uri'],
                'range': {{'start': {{'line': 0, 'character': 3}},
                          'end': {{'line': 0, 'character': 7}}}}}}]}})
    elif method == 'textDocument/signatureHelp':
        log('signature-help')
        loading = QUIESCENT_AT[0] is None or time.monotonic() < QUIESCENT_AT[0]
        if loading:
            send({{'jsonrpc': '2.0', 'id': message['id'], 'result': None}})
        else:
            send({{'jsonrpc': '2.0', 'id': message['id'], 'result': {{
                'signatures': [{{'label': 'fn helper(value: i32)'}}],
                'activeSignature': 0}}}})
    elif method == 'textDocument/completion':
        if COMPLETION_MODE == 'empty':
            log('completion')
            send({{'jsonrpc': '2.0', 'id': message['id'], 'result': []}})
        elif COMPLETION_MODE == 'slow':
            threading.Thread(target=completion_worker, args=(message['id'],), daemon=True).start()
    elif method == '$/cancelRequest':
        if COMPLETION_MODE == 'slow':
            with CANCEL_LOCK:
                CANCELLED.add(message['params']['id'])
            log('cancel')
    elif method == 'textDocument/didChange':
        if LOG_DID_CHANGE:
            log('did-change')
    elif method == 'textDocument/didSave':
        if LOG_DID_SAVE:
            log('did-save')
    elif method == 'textDocument/diagnostic':
        if LOG_DIAGNOSTIC:
            log('diagnostic')
        send({{'jsonrpc': '2.0', 'id': message['id'], 'result': {{'kind': 'full', 'resultId': 'fake-result', 'items': []}}}})
    elif method == 'shutdown':
        send({{'jsonrpc': '2.0', 'id': message['id'], 'result': None}})
"#
            ,
            log_path = config.log_path,
            capabilities = capabilities,
            completion_mode = completion_mode,
            completion_delay_ms = completion_delay_ms,
            log_did_change = log_did_change,
            log_did_save = log_did_save,
            log_diagnostics = log_diagnostics,
            workspace_loading_ms = workspace_loading_ms,
            watched_files_glob = watched_files_glob,
            background_progress = python_bool_literal(config.background_progress),
            empty_after_quiescent_ms = empty_after_quiescent_ms,
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
