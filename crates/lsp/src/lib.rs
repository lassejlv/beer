use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, Hover, HoverContents, HoverParams,
    HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams, MarkupContent,
    MarkupKind, OneOf, Position, Range, ServerCapabilities, ServerInfo,
    TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use beer_ast::{Func, Program, Type};
use beer_codegen as codegen;
use beer_driver as driver;
use beer_errors::CompileError;
use beer_source::FileTable;

type DynErr = Box<dyn Error + Sync + Send>;

pub fn run() -> Result<(), DynErr> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_async());
    Ok(())
}

async fn run_async() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        state: Mutex::new(ServerState::default()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}

struct Backend {
    client: Client,
    state: Mutex<ServerState>,
}

#[derive(Default)]
struct ServerState {
    /// Open documents — URI as received from the client to current buffer text.
    docs: HashMap<Url, String>,
    /// Canonical disk path -> the URI the client sent. Editors track open
    /// files by their original URI, so diagnostics must be published under
    /// that URI — not the canonicalized form (`/tmp` vs `/private/tmp` etc.).
    canonical_to_uri: HashMap<PathBuf, Url>,
    /// URIs we published non-empty diagnostics to last pass. On the next
    /// pass we must publish empties to any that drop out, to clear stale
    /// squiggles in closed files.
    last_reported: HashSet<Url>,
    /// Function name -> formatted signature ("fn add(a: int, b: int) -> int").
    /// Cached from the most recent successful driver pass; powers completion
    /// and hover even when the current buffer has parse errors.
    fn_signatures: HashMap<String, String>,
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

    fn client_uri(&self, canonical: &Path) -> Option<Url> {
        self.canonical_to_uri.get(canonical).cloned()
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> LspResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: None,
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(false)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "beer".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {}

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        {
            let mut st = self.state.lock().unwrap();
            st.remember(uri.clone(), params.text_document.text);
        }
        self.recompile(&uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Some(change) = params.content_changes.into_iter().last() {
            let mut st = self.state.lock().unwrap();
            st.remember(uri.clone(), change.text);
        }
        self.recompile(&uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Some(text) = params.text {
            let mut st = self.state.lock().unwrap();
            st.remember(uri.clone(), text);
        }
        self.recompile(&uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        {
            let mut st = self.state.lock().unwrap();
            st.forget(&uri);
            st.last_reported.remove(&uri);
        }
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn completion(&self, _: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        const KEYWORDS: &[&str] = &[
            "let", "fn", "if", "else", "while", "return", "true", "false", "as", "use", "print",
        ];
        const TYPES: &[&str] = &["int", "float", "bool", "str"];

        let sigs = {
            let st = self.state.lock().unwrap();
            st.fn_signatures.clone()
        };

        let mut items = Vec::with_capacity(KEYWORDS.len() + TYPES.len() + sigs.len());
        for kw in KEYWORDS {
            items.push(CompletionItem {
                label: (*kw).into(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            });
        }
        for t in TYPES {
            items.push(CompletionItem {
                label: (*t).into(),
                kind: Some(CompletionItemKind::TYPE_PARAMETER),
                ..Default::default()
            });
        }
        for (name, sig) in sigs {
            items.push(CompletionItem {
                label: name,
                detail: Some(sig),
                kind: Some(CompletionItemKind::FUNCTION),
                ..Default::default()
            });
        }
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let (source, sig) = {
            let st = self.state.lock().unwrap();
            let text = st.docs.get(&uri).cloned().unwrap_or_default();
            let word = word_at(&text, pos.line as usize, pos.character as usize);
            let sig = word.and_then(|w| {
                keyword_hover(w)
                    .map(|s| s.to_string())
                    .or_else(|| st.fn_signatures.get(w).cloned())
            });
            (text, sig)
        };
        let _ = source; // kept for symmetry; not used further

        let Some(text) = sig else {
            return Ok(None);
        };

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!("```beer\n{}\n```", text),
            }),
            range: None,
        }))
    }
}

fn word_at(text: &str, line: usize, col: usize) -> Option<&str> {
    let line_text = text.lines().nth(line)?;
    let bytes = line_text.as_bytes();
    if col > bytes.len() {
        return None;
    }
    let is_id = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    // The cursor can sit immediately after the word (col == end). Walk left
    // from min(col, len-1) to find the identifier.
    let probe = col.min(bytes.len().saturating_sub(1));
    if bytes.is_empty() || !is_id(bytes[probe]) {
        return None;
    }
    let mut start = probe;
    while start > 0 && is_id(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = probe;
    while end < bytes.len() && is_id(bytes[end]) {
        end += 1;
    }
    Some(&line_text[start..end])
}

fn keyword_hover(word: &str) -> Option<&'static str> {
    Some(match word {
        "let" => "let — declare a (mutable) variable",
        "fn" => "fn — declare a function",
        "if" => "if — conditional branch",
        "else" => "else — alternate branch of an `if`",
        "while" => "while — loop while a condition holds",
        "return" => "return — return a value from the enclosing function",
        "true" | "false" => "bool literal",
        "as" => "as — cast between int and float",
        "use" => "use \"path.beer\" — import another file",
        "print" => "print(x) — built-in, prints an int / float / str / bool",
        "int" => "int — 64-bit signed integer",
        "float" => "float — 64-bit floating-point number",
        "bool" => "bool — true / false",
        "str" => "str — null-terminated string (C-string)",
        _ => return None,
    })
}

impl Backend {
    async fn recompile(&self, focused: &Url) {
        let Ok(root_path) = focused.to_file_path() else {
            return;
        };

        // Snapshot open-doc overlays under the lock, then drop it before the
        // compile — we don't want to hold state across CPU-heavy work.
        let overlays: Vec<(PathBuf, String)> = {
            let st = self.state.lock().unwrap();
            st.docs
                .iter()
                .filter_map(|(uri, text)| uri.to_file_path().ok().map(|p| (p, text.clone())))
                .collect()
        };

        let mut files = FileTable::new();
        for (p, text) in overlays {
            files.set_overlay(p, text);
        }

        let (program_opt, compile_error) = run_pipeline(files, &root_path);

        // Refresh the cached symbol table whenever parsing succeeded (even
        // if codegen later errored), so completion + hover stay useful while
        // the user is mid-edit.
        if let Some(prog) = program_opt {
            let sigs: HashMap<String, String> = prog
                .funcs
                .iter()
                .map(|f| (f.name.clone(), format_signature(f)))
                .collect();
            let mut st = self.state.lock().unwrap();
            st.fn_signatures = sigs;
        }

        // Bucket diagnostics by URI. Seed every currently-open doc with an
        // empty vec so stale errors from prior passes get cleared.
        let open_uris: Vec<Url> = {
            let st = self.state.lock().unwrap();
            st.docs.keys().cloned().collect()
        };
        let mut by_uri: HashMap<Url, Vec<Diagnostic>> = HashMap::new();
        for uri in &open_uris {
            by_uri.insert(uri.clone(), Vec::new());
        }

        if let Some((err, files_opt)) = compile_error {
            let (uri, range) = {
                let st = self.state.lock().unwrap();
                diag_location(&err, files_opt.as_ref(), focused, &st)
            };
            by_uri.entry(uri).or_default().push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("beer".into()),
                message: err.msg,
                ..Default::default()
            });
        }

        // Also clear any URIs we reported to last pass that aren't in this batch.
        let prev: Vec<Url> = {
            let st = self.state.lock().unwrap();
            st.last_reported.iter().cloned().collect()
        };
        for stale in prev {
            by_uri.entry(stale).or_default();
        }

        let mut new_reported = HashSet::new();
        for (uri, diags) in by_uri {
            if !diags.is_empty() {
                new_reported.insert(uri.clone());
            }
            self.client.publish_diagnostics(uri, diags, None).await;
        }

        let mut st = self.state.lock().unwrap();
        st.last_reported = new_reported;
    }
}

/// Run driver + codegen check. Returns both the parsed Program (if the driver
/// reached it, regardless of codegen) and any error along with the FileTable
/// for source lookup during diagnostic rendering.
fn run_pipeline(
    files: FileTable,
    root: &Path,
) -> (Option<Program>, Option<(CompileError, Option<FileTable>)>) {
    match driver::load_program_with(files, root) {
        Ok((program, ft)) => match codegen::compile(&program, None) {
            Ok(()) => (Some(program), None),
            Err(e) => (Some(program), Some((e, Some(ft)))),
        },
        Err((ft, e)) => (None, Some((e, Some(ft)))),
    }
}

fn format_signature(f: &Func) -> String {
    let params = f
        .params
        .iter()
        .map(|(n, t)| format!("{}: {}", n, format_type(*t)))
        .collect::<Vec<_>>()
        .join(", ");
    let ret = if f.ret == Type::Void {
        String::new()
    } else {
        format!(" -> {}", format_type(f.ret))
    };
    format!("fn {}({}){}", f.name, params, ret)
}

fn format_type(t: Type) -> &'static str {
    match t {
        Type::Int => "int",
        Type::Float => "float",
        Type::Bool => "bool",
        Type::Str => "str",
        Type::Void => "void",
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
