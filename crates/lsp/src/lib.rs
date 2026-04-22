use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::PathBuf;

use lsp_server::{Connection, Message, Notification};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as _, PublishDiagnostics,
};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, InitializeParams, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, Url,
};

use beer_codegen as codegen;
use beer_driver as driver;
use beer_errors::CompileError;
use beer_source::FileTable;

type DynErr = Box<dyn Error + Sync + Send>;

pub fn run() -> Result<(), DynErr> {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::FULL,
        )),
        ..Default::default()
    };
    let init_params = connection.initialize(serde_json::to_value(&capabilities)?)?;
    let _: InitializeParams = serde_json::from_value(init_params)?;

    let mut state = ServerState::default();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    break;
                }
            }
            Message::Notification(not) => {
                handle_notification(&connection, &mut state, not)?;
            }
            Message::Response(_) => {}
        }
    }

    io_threads.join()?;
    Ok(())
}

#[derive(Default)]
struct ServerState {
    /// Open documents — URI to current buffer text.
    docs: HashMap<Url, String>,
    /// Canonical disk path to the URI the client sent. Editors track open
    /// files by their original URI, so diagnostics must be published under
    /// that URI — not the canonicalized form (which can differ, e.g. macOS
    /// `/tmp` ↔ `/private/tmp`).
    canonical_to_uri: HashMap<PathBuf, Url>,
    /// URIs we have published non-empty diagnostics for. Next compile pass
    /// must publish empties to any URI no longer reporting, to clear stale
    /// squiggles in closed files.
    last_reported: HashSet<Url>,
}

impl ServerState {
    fn remember(&mut self, uri: Url, text: String) {
        if let Ok(p) = uri.to_file_path() {
            if let Ok(canon) = std::fs::canonicalize(&p) {
                self.canonical_to_uri.insert(canon, uri.clone());
            }
        }
        self.docs.insert(uri, text);
    }

    fn forget(&mut self, uri: &Url) {
        self.docs.remove(uri);
        self.canonical_to_uri.retain(|_, u| u != uri);
    }

    fn client_uri(&self, canonical: &PathBuf) -> Option<Url> {
        self.canonical_to_uri.get(canonical).cloned()
    }
}

fn handle_notification(
    connection: &Connection,
    state: &mut ServerState,
    not: Notification,
) -> Result<(), DynErr> {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams = serde_json::from_value(not.params)?;
            let uri = params.text_document.uri.clone();
            state.remember(uri.clone(), params.text_document.text);
            recompile(connection, state, &uri)?;
        }
        DidChangeTextDocument::METHOD => {
            let params: DidChangeTextDocumentParams = serde_json::from_value(not.params)?;
            if let Some(change) = params.content_changes.into_iter().last() {
                state.remember(params.text_document.uri.clone(), change.text);
            }
            recompile(connection, state, &params.text_document.uri)?;
        }
        DidSaveTextDocument::METHOD => {
            let params: DidSaveTextDocumentParams = serde_json::from_value(not.params)?;
            if let Some(text) = params.text {
                state.remember(params.text_document.uri.clone(), text);
            }
            recompile(connection, state, &params.text_document.uri)?;
        }
        DidCloseTextDocument::METHOD => {
            let params: DidCloseTextDocumentParams = serde_json::from_value(not.params)?;
            let uri = params.text_document.uri;
            state.forget(&uri);
            publish(connection, &uri, vec![])?;
            state.last_reported.remove(&uri);
        }
        _ => {}
    }
    Ok(())
}

fn recompile(
    connection: &Connection,
    state: &mut ServerState,
    focused: &Url,
) -> Result<(), DynErr> {
    let Ok(root_path) = focused.to_file_path() else {
        return Ok(());
    };

    // Seed every open doc as an overlay so unsaved buffers feed into the
    // compiler pipeline instead of the on-disk copy.
    let mut files = FileTable::new();
    for (uri, text) in &state.docs {
        if let Ok(p) = uri.to_file_path() {
            files.set_overlay(p, text.clone());
        }
    }

    let compile_error = run_pipeline(files, &root_path);

    // Bucket diagnostics by URI. Pre-seed every currently-open doc with an
    // empty list so stale squiggles from prior passes are cleared.
    let mut by_uri: HashMap<Url, Vec<Diagnostic>> = HashMap::new();
    for uri in state.docs.keys() {
        by_uri.insert(uri.clone(), Vec::new());
    }

    if let Some((err, files_opt)) = compile_error {
        let (uri, range) = diag_location(&err, files_opt.as_ref(), focused, state);
        by_uri
            .entry(uri)
            .or_default()
            .push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("beer".into()),
                message: err.msg,
                ..Default::default()
            });
    }

    // Clear any URIs we previously reported to that aren't in this batch.
    for stale in state.last_reported.iter() {
        by_uri.entry(stale.clone()).or_default();
    }

    let mut new_reported = HashSet::new();
    for (uri, diags) in by_uri {
        if !diags.is_empty() {
            new_reported.insert(uri.clone());
        }
        publish(connection, &uri, diags)?;
    }
    state.last_reported = new_reported;

    Ok(())
}

/// Run driver + codegen check; return any error alongside the (partial) file table.
fn run_pipeline(
    files: FileTable,
    root: &std::path::Path,
) -> Option<(CompileError, Option<FileTable>)> {
    match driver::load_program_with(files, root) {
        Ok((program, ft)) => match codegen::compile(&program, None) {
            Ok(()) => None,
            Err(e) => Some((e, Some(ft))),
        },
        Err((ft, e)) => Some((e, Some(ft))),
    }
}

fn diag_location(
    err: &CompileError,
    files: Option<&FileTable>,
    fallback: &Url,
    state: &ServerState,
) -> (Url, Range) {
    if let (Some(span), Some(ft)) = (err.span, files) {
        let lf = ft.get(span.file);
        let uri = state
            .client_uri(&lf.path)
            .or_else(|| Url::from_file_path(&lf.path).ok());
        if let Some(uri) = uri {
            let line = span.line.saturating_sub(1);
            let col = span.col.saturating_sub(1);
            let range = Range {
                start: Position { line, character: col },
                end: Position { line, character: col.saturating_add(1) },
            };
            return (uri, range);
        }
    }
    let range = Range {
        start: Position { line: 0, character: 0 },
        end: Position { line: 0, character: 0 },
    };
    (fallback.clone(), range)
}

fn publish(connection: &Connection, uri: &Url, diagnostics: Vec<Diagnostic>) -> Result<(), DynErr> {
    let params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics,
        version: None,
    };
    let not = Notification {
        method: PublishDiagnostics::METHOD.to_string(),
        params: serde_json::to_value(&params)?,
    };
    connection.sender.send(Message::Notification(not))?;
    Ok(())
}
