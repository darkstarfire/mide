//! Cliente LSP (Language Server Protocol) próprio do `mide-code`: roda o servidor da
//! linguagem (rust-analyzer, gopls, pylsp, clangd...) como programa filho e conversa
//! com ele em JSON-RPC pela entrada e saída padrão (cabeçalho `Content-Length`).
//!
//! O que o editor usa: abrir/mudar/salvar documentos (texto inteiro a cada mudança),
//! diagnósticos (erros e avisos), completar, ir para a definição e a dica (hover).
//! Respostas chegam numa thread e ficam numa fila lida a cada quadro (`poll`).
//!
//! Posições: o padrão do LSP conta em UTF-16; o cliente pede UTF-8 (`positionEncodings`)
//! e, se o servidor não aceitar, converte (`to_position`/`to_offset`).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};

use serde_json::{json, Value};

/// Gravidade de um diagnóstico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Um diagnóstico (erro ou aviso) num trecho do documento, em bytes do texto.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub severity: Severity,
    pub message: String,
}

/// Item de completar.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Completion {
    pub label: String,
    pub detail: String,
    /// Texto que entra (o `insertText`, ou o rótulo).
    pub insert: String,
    /// Tipo do item (`CompletionItemKind` do LSP: 3 função, 6 variável, 7 classe,
    /// 14 palavra-chave, 15 trecho...); 0 = palavra do arquivo.
    pub kind: u8,
    /// Documentação (texto ou markdown), quando o servidor manda ou depois do
    /// `completionItem/resolve`.
    pub doc: String,
    /// `insert` é um trecho com marcas (`${1:nome}`, `$0`): ver `expand_snippet`.
    pub snippet: bool,
    /// O item como o servidor mandou (para o `completionItem/resolve`).
    pub raw: Option<Value>,
    /// Ordem que o servidor sugere (`sortText`; sem ele, o rótulo).
    pub sort: String,
    /// Texto usado para filtrar (`filterText`; sem ele, o rótulo).
    pub filter: String,
}

/// Texto de uma documentação do LSP (texto puro, `MarkupContent` ou lista).
pub fn markup_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or_default().to_string(),
        Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string).or_else(|| v["value"].as_str().map(str::to_string))).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Abre um trecho (snippet) do LSP: as marcas `${1:nome}` viram o texto delas,
/// `$1`/`$0` somem e `\$` vira `$`. Devolve o texto e onde fica a primeira parada
/// (início e fim, em bytes): o texto da `${1:...}` fica escolhido, como no VS Code.
pub fn expand_snippet(s: &str) -> (String, Option<(usize, usize)>) {
    let mut out = String::new();
    // (número da parada, início, fim) de cada marca encontrada.
    let mut stops: Vec<(u32, usize, usize)> = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    fn number(chars: &[char], i: &mut usize) -> Option<u32> {
        let start = *i;
        while *i < chars.len() && chars[*i].is_ascii_digit() {
            *i += 1;
        }
        chars[start..*i].iter().collect::<String>().parse().ok()
    }
    // Pilha das marcas `${n:` abertas (número e onde o texto delas começa).
    let mut open: Vec<(u32, usize)> = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '$' && i + 1 < chars.len() {
            if chars[i + 1].is_ascii_digit() {
                i += 1;
                if let Some(n) = number(&chars, &mut i) {
                    stops.push((n, out.len(), out.len()));
                }
                continue;
            }
            if chars[i + 1] == '{' {
                let mut j = i + 2;
                if let Some(n) = number(&chars, &mut j) {
                    match chars.get(j) {
                        Some(':') => {
                            open.push((n, out.len()));
                            i = j + 1;
                            continue;
                        }
                        // `${1|um,dois|}`: fica a primeira escolha.
                        Some('|') => {
                            let end = chars[j + 1..].iter().position(|&c| c == '|').map_or(chars.len(), |p| j + 1 + p);
                            let first: String = chars[j + 1..end].iter().collect::<String>().split(',').next().unwrap_or_default().to_string();
                            let start = out.len();
                            out.push_str(&first);
                            stops.push((n, start, out.len()));
                            i = (end + 2).min(chars.len());
                            continue;
                        }
                        Some('}') => {
                            stops.push((n, out.len(), out.len()));
                            i = j + 1;
                            continue;
                        }
                        _ => {}
                    }
                }
            }
        }
        if c == '}' && let Some((n, start)) = open.pop() {
            stops.push((n, start, out.len()));
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    // A primeira parada é a de menor número maior que zero; sem elas, a `$0`.
    let first = stops.iter().filter(|s| s.0 > 0).min_by_key(|s| (s.0, s.1)).or_else(|| stops.iter().find(|s| s.0 == 0));
    (out, first.map(|s| (s.1, s.2)))
}

/// Uma troca de texto num arquivo (`TextEdit` do LSP): de (linha, coluna) até
/// (linha, coluna), nas unidades do servidor.
#[derive(Debug, Clone, PartialEq)]
pub struct TextEdit {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub new_text: String,
}

/// Trocas em vários arquivos (`WorkspaceEdit`): endereço do arquivo e as trocas.
pub type WorkspaceEdit = Vec<(String, Vec<TextEdit>)>;

/// Ajuda de assinatura: a assinatura, onde está o parâmetro atual (bytes) e a
/// documentação.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Signature {
    pub label: String,
    pub active: Option<(usize, usize)>,
    pub doc: String,
}

/// Ação de código (lâmpada): título, trocas e/ou comando a rodar no servidor.
#[derive(Debug, Clone, PartialEq)]
pub struct CodeAction {
    pub title: String,
    pub kind: String,
    pub edit: WorkspaceEdit,
    pub command: Option<Value>,
    /// A ação crua, quando falta pedir os detalhes (`codeAction/resolve`).
    pub raw: Option<Value>,
}

/// Símbolo do arquivo (função, classe...), com a profundidade na árvore.
#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub detail: String,
    pub kind: u8,
    pub line: u32,
    pub col: u32,
    /// Última linha do símbolo (o corpo inteiro), para saber onde o cursor está.
    pub end_line: u32,
    pub depth: usize,
}

/// Dica em linha (`inlayHint`): posição e texto.
#[derive(Debug, Clone, PartialEq)]
pub struct InlayHint {
    pub line: u32,
    pub col: u32,
    pub label: String,
    /// Espaço antes/depois da dica.
    pub pad_left: bool,
    pub pad_right: bool,
}

fn position(v: &Value) -> (u32, u32) {
    (v["line"].as_u64().unwrap_or(0) as u32, v["character"].as_u64().unwrap_or(0) as u32)
}

fn parse_text_edit(v: &Value) -> Option<TextEdit> {
    let range = if v["range"].is_object() { &v["range"] } else { &v["insert"] };
    Some(TextEdit { start: position(&range["start"]), end: position(&range["end"]), new_text: v["newText"].as_str()?.to_string() })
}

/// Lê um `WorkspaceEdit` (`changes` ou `documentChanges`).
pub fn parse_workspace_edit(v: &Value) -> WorkspaceEdit {
    let mut out: WorkspaceEdit = Vec::new();
    if let Some(changes) = v["changes"].as_object() {
        for (uri, edits) in changes {
            let edits = edits.as_array().map(|a| a.iter().filter_map(parse_text_edit).collect()).unwrap_or_default();
            out.push((uri.clone(), edits));
        }
    }
    if let Some(docs) = v["documentChanges"].as_array() {
        for d in docs {
            if let Some(uri) = d["textDocument"]["uri"].as_str() {
                let edits = d["edits"].as_array().map(|a| a.iter().filter_map(parse_text_edit).collect()).unwrap_or_default();
                out.push((uri.to_string(), edits));
            }
        }
    }
    out
}

/// Aplica trocas a um texto (das últimas para as primeiras, para as posições não
/// andarem). Devolve o texto novo.
pub fn apply_edits(text: &str, edits: &[TextEdit], utf8: bool) -> String {
    let mut spans: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|e| {
            let a = to_offset(text, e.start.0, e.start.1, utf8);
            let b = to_offset(text, e.end.0, e.end.1, utf8).max(a);
            (a, b, e.new_text.as_str())
        })
        .collect();
    spans.sort_by(|x, y| y.0.cmp(&x.0).then(y.1.cmp(&x.1)));
    let mut out = text.to_string();
    let mut limit = usize::MAX;
    for (a, b, t) in spans {
        // Trocas sobrepostas: a de trás vale.
        if b > limit {
            continue;
        }
        out.replace_range(a..b, t);
        limit = a;
    }
    out
}

fn parse_symbols(v: &Value, depth: usize, out: &mut Vec<Symbol>) {
    for s in v.as_array().into_iter().flatten() {
        // `DocumentSymbol` (com `selectionRange` e filhos) ou `SymbolInformation`.
        let range = if s["selectionRange"].is_object() { &s["selectionRange"] } else if s["range"].is_object() { &s["range"] } else { &s["location"]["range"] };
        let (line, col) = position(&range["start"]);
        let whole = if s["range"].is_object() { &s["range"] } else { &s["location"]["range"] };
        let end_line = position(&whole["end"]).0.max(line);
        out.push(Symbol {
            end_line,
            name: s["name"].as_str().unwrap_or_default().to_string(),
            detail: s["detail"].as_str().or_else(|| s["containerName"].as_str()).unwrap_or_default().to_string(),
            kind: symbol_to_completion_kind(s["kind"].as_u64().unwrap_or(0)),
            line,
            col,
            depth,
        });
        parse_symbols(&s["children"], depth + 1, out);
    }
}

/// Os tipos de símbolo (`SymbolKind`) têm outra numeração que os do completar: passa
/// para a do completar, para usar os mesmos selos.
pub fn symbol_to_completion_kind(k: u64) -> u8 {
    match k {
        1 => 17,
        2..=4 => 9,
        5 => 7,
        6 => 2,
        7 => 10,
        8 => 5,
        9 => 4,
        10 => 13,
        11 => 8,
        12 => 3,
        13 => 6,
        14 => 21,
        15..=18 => 12,
        19 => 9,
        20 => 5,
        21 => 12,
        22 => 20,
        23 => 22,
        24 => 23,
        25 => 24,
        26 => 25,
        _ => 1,
    }
}

/// Eventos vindos do servidor.
#[derive(Debug, Clone)]
pub enum Event {
    Diagnostics { uri: String, items: Vec<Diagnostic> },
    Completions { id: u64, items: Vec<Completion> },
    /// Resposta do `completionItem/resolve`: a documentação e o detalhe do item.
    Resolved { id: u64, doc: String, detail: String },
    Signature { id: u64, signature: Option<Signature> },
    /// Trocas pedidas (renomear, formatar) ou mandadas pelo servidor (`applyEdit`,
    /// com `id` 0).
    Edit { id: u64, edit: WorkspaceEdit },
    Actions { id: u64, actions: Vec<CodeAction> },
    Symbols { id: u64, symbols: Vec<Symbol> },
    Hints { id: u64, hints: Vec<InlayHint> },
    /// Onde está a definição: endereço do arquivo e posição (linha, coluna).
    Definition { id: u64, uri: String, line: u32, col: u32 },
    Hover { id: u64, text: String },
    /// O servidor terminou (ou não conseguiu começar).
    Exited(String),
}

/// Um servidor de linguagem rodando.
pub struct Client {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next_id: u64,
    /// Pedidos esperando resposta: id → método.
    pending: HashMap<u64, &'static str>,
    /// Posições em UTF-8 (o servidor aceitou); senão, UTF-16.
    pub utf8: bool,
    pub ready: bool,
    /// Mensagens que chegaram antes do `initialize` responder (abrir documentos).
    queued: Vec<Value>,
    pub name: String,
    /// O que o servidor sabe fazer (`capabilities` da resposta ao `initialize`).
    pub caps: Value,
    /// Configurações do servidor (respondidas no `workspace/configuration`), como o
    /// `settings` do Zed.
    pub settings: Value,
}

/// Endereço `file://` de um caminho (os `%XX` como o LSP espera).
pub fn uri(path: &Path) -> String {
    crate::thumbnails::file_uri(path)
}

/// Lê uma mensagem com cabeçalho `Content-Length` (ou `None` no fim).
pub fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            length = v.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0u8; length?];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Mensagem com o cabeçalho `Content-Length`.
pub fn frame(msg: &Value) -> Vec<u8> {
    let body = msg.to_string();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Posição LSP (linha, coluna) de um byte do texto: coluna em bytes (UTF-8) ou em
/// unidades de UTF-16.
pub fn to_position(text: &str, offset: usize, utf8: bool) -> (u32, u32) {
    let offset = offset.min(text.len());
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line = text[..line_start].matches('\n').count() as u32;
    let piece = &text[line_start..offset];
    let col = if utf8 { piece.len() } else { piece.encode_utf16().count() };
    (line, col as u32)
}

/// Byte do texto de uma posição LSP (o fim da linha, se a coluna passar dela).
pub fn to_offset(text: &str, line: u32, col: u32, utf8: bool) -> usize {
    let mut start = 0;
    for _ in 0..line {
        match text[start..].find('\n') {
            Some(i) => start += i + 1,
            None => return text.len(),
        }
    }
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    let line_text = &text[start..end];
    if utf8 {
        let mut c = (col as usize).min(line_text.len());
        while !line_text.is_char_boundary(c) {
            c -= 1;
        }
        return start + c;
    }
    let mut units = 0u32;
    for (i, ch) in line_text.char_indices() {
        if units >= col {
            return start + i;
        }
        units += ch.len_utf16() as u32;
    }
    end
}

impl Client {
    /// Roda o servidor (`command` com `args`) na pasta do projeto e manda o `initialize`.
    pub fn start(command: &str, args: &[String], root: &Path) -> std::io::Result<Self> {
        Self::start_with(command, args, root, &[], Value::Null, Value::Null)
    }

    /// Como `start`, com o ambiente, as `initializationOptions` e as configurações
    /// (respondidas no `workspace/configuration` e mandadas depois do `initialized`).
    pub fn start_with(command: &str, args: &[String], root: &Path, env: &[(String, String)], init_options: Value, settings: Value) -> std::io::Result<Self> {
        let mut child = Command::new(command).args(args).envs(env.iter().map(|(k, v)| (k, v))).current_dir(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("sem entrada"))?;
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("sem saída"))?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(msg) = read_message(&mut reader) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        let mut client = Self { child, stdin, rx, next_id: 1, pending: HashMap::new(), utf8: false, ready: false, queued: Vec::new(), name: command.to_string(), caps: Value::Null, settings };
        let params = json!({
            "processId": std::process::id(),
            "rootUri": uri(root),
            "workspaceFolders": [{ "uri": uri(root), "name": root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() }],
            "clientInfo": { "name": "mide-code" },
            "initializationOptions": init_options,
            "capabilities": {
                "general": { "positionEncodings": ["utf-8", "utf-16"] },
                "textDocument": {
                    "synchronization": { "didSave": true },
                    "publishDiagnostics": {},
                    "completion": {
                        "completionItem": {
                            "snippetSupport": true,
                            "documentationFormat": ["markdown", "plaintext"],
                            "resolveSupport": { "properties": ["documentation", "detail"] }
                        },
                        "completionItemKind": { "valueSet": (1..=25).collect::<Vec<u32>>() }
                    },
                    "definition": {},
                    "hover": { "contentFormat": ["plaintext", "markdown"] },
                    "signatureHelp": { "signatureInformation": { "documentationFormat": ["markdown", "plaintext"], "parameterInformation": { "labelOffsetSupport": true } } },
                    "rename": { "prepareSupport": false },
                    "formatting": {},
                    "codeAction": { "codeActionLiteralSupport": { "codeActionKind": { "valueSet": ["", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite", "source", "source.organizeImports"] } }, "resolveSupport": { "properties": ["edit"] } },
                    "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                    "inlayHint": {}
                },
                "workspace": { "applyEdit": true, "configuration": true, "workspaceEdit": { "documentChanges": true } }
            }
        });
        client.request("initialize", params)?;
        Ok(client)
    }

    fn send(&mut self, msg: &Value) -> std::io::Result<()> {
        self.stdin.write_all(&frame(msg))?;
        self.stdin.flush()
    }

    fn request(&mut self, method: &'static str, params: Value) -> std::io::Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, method);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if self.ready || method == "initialize" {
            self.send(&msg)?;
        } else {
            self.queued.push(msg);
        }
        Ok(id)
    }

    fn notify(&mut self, method: &str, params: Value) {
        let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        if self.ready {
            let _ = self.send(&msg);
        } else {
            self.queued.push(msg);
        }
    }

    pub fn did_open(&mut self, path: &Path, language: &str, version: i32, text: &str) {
        self.notify("textDocument/didOpen", json!({ "textDocument": { "uri": uri(path), "languageId": language, "version": version, "text": text } }));
    }

    pub fn did_change(&mut self, path: &Path, version: i32, text: &str) {
        self.notify("textDocument/didChange", json!({ "textDocument": { "uri": uri(path), "version": version }, "contentChanges": [{ "text": text }] }));
    }

    pub fn did_save(&mut self, path: &Path) {
        self.notify("textDocument/didSave", json!({ "textDocument": { "uri": uri(path) } }));
    }

    pub fn did_close(&mut self, path: &Path) {
        self.notify("textDocument/didClose", json!({ "textDocument": { "uri": uri(path) } }));
    }

    fn position_params(&self, path: &Path, text: &str, offset: usize) -> Value {
        let (line, character) = to_position(text, offset, self.utf8);
        json!({ "textDocument": { "uri": uri(path) }, "position": { "line": line, "character": character } })
    }

    pub fn completion(&mut self, path: &Path, text: &str, offset: usize) -> Option<u64> {
        let params = self.position_params(path, text, offset);
        self.request("textDocument/completion", params).ok()
    }

    /// O servidor sabe fazer `cap` (`renameProvider`, `inlayHintProvider`...).
    pub fn can(&self, cap: &str) -> bool {
        !matches!(self.caps[cap], Value::Null | Value::Bool(false))
    }

    pub fn signature_help(&mut self, path: &Path, text: &str, offset: usize) -> Option<u64> {
        let params = self.position_params(path, text, offset);
        self.request("textDocument/signatureHelp", params).ok()
    }

    pub fn rename(&mut self, path: &Path, text: &str, offset: usize, new_name: &str) -> Option<u64> {
        let mut params = self.position_params(path, text, offset);
        params["newName"] = json!(new_name);
        self.request("textDocument/rename", params).ok()
    }

    pub fn formatting(&mut self, path: &Path, tab_size: usize, spaces: bool) -> Option<u64> {
        self.request("textDocument/formatting", json!({ "textDocument": { "uri": uri(path) }, "options": { "tabSize": tab_size, "insertSpaces": spaces } })).ok()
    }

    /// Ações de código no trecho (de `from` a `to`), com os problemas que o tocam.
    pub fn code_actions(&mut self, path: &Path, text: &str, from: usize, to: usize, diags: &[Diagnostic]) -> Option<u64> {
        let (a, b) = (to_position(text, from, self.utf8), to_position(text, to, self.utf8));
        let diagnostics: Vec<Value> = diags
            .iter()
            .filter(|d| d.start.0 <= b.0 && d.end.0 >= a.0)
            .map(|d| json!({ "range": { "start": { "line": d.start.0, "character": d.start.1 }, "end": { "line": d.end.0, "character": d.end.1 } }, "message": d.message, "severity": match d.severity { Severity::Error => 1, Severity::Warning => 2, Severity::Info => 3 } }))
            .collect();
        let params = json!({ "textDocument": { "uri": uri(path) }, "range": { "start": { "line": a.0, "character": a.1 }, "end": { "line": b.0, "character": b.1 } }, "context": { "diagnostics": diagnostics } });
        self.request("textDocument/codeAction", params).ok()
    }

    pub fn resolve_action(&mut self, raw: Value) -> Option<u64> {
        self.request("codeAction/resolve", raw).ok()
    }

    pub fn execute_command(&mut self, command: &Value) {
        let params = json!({ "command": command["command"], "arguments": command.get("arguments").cloned().unwrap_or(json!([])) });
        let _ = self.request("workspace/executeCommand", params);
    }

    pub fn document_symbols(&mut self, path: &Path) -> Option<u64> {
        self.request("textDocument/documentSymbol", json!({ "textDocument": { "uri": uri(path) } })).ok()
    }

    /// Dicas em linha das linhas `first` a `last`.
    pub fn inlay_hints(&mut self, path: &Path, first: u32, last: u32) -> Option<u64> {
        self.request("textDocument/inlayHint", json!({ "textDocument": { "uri": uri(path) }, "range": { "start": { "line": first, "character": 0 }, "end": { "line": last, "character": 0 } } })).ok()
    }

    /// Pede a documentação de um item de completar (`completionItem/resolve`).
    pub fn resolve(&mut self, item: Value) -> Option<u64> {
        self.request("completionItem/resolve", item).ok()
    }

    pub fn definition(&mut self, path: &Path, text: &str, offset: usize) -> Option<u64> {
        let params = self.position_params(path, text, offset);
        self.request("textDocument/definition", params).ok()
    }

    pub fn hover(&mut self, path: &Path, text: &str, offset: usize) -> Option<u64> {
        let params = self.position_params(path, text, offset);
        self.request("textDocument/hover", params).ok()
    }

    /// Mensagens que chegaram: respostas e avisos viram eventos para o editor.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            out.extend(self.handle(msg));
        }
        if let Ok(Some(status)) = self.child.try_wait() {
            out.push(Event::Exited(format!("{} terminou ({status})", self.name)));
        }
        out
    }

    fn handle(&mut self, msg: Value) -> Vec<Event> {
        // Pedido do servidor: configurações (as do usuário, por seção), trocas a
        // aplicar (`workspace/applyEdit`) e o resto respondido vazio.
        if let (Some(id), Some(method)) = (msg.get("id"), msg.get("method").and_then(Value::as_str)) {
            let mut events = Vec::new();
            let result = match method {
                "workspace/configuration" => {
                    let items = msg["params"]["items"].as_array().cloned().unwrap_or_default();
                    Value::Array(items.iter().map(|it| config_section(&self.settings, it["section"].as_str())).collect())
                }
                "workspace/applyEdit" => {
                    events.push(Event::Edit { id: 0, edit: parse_workspace_edit(&msg["params"]["edit"]) });
                    json!({ "applied": true })
                }
                _ => Value::Null,
            };
            let _ = self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            return events;
        }
        if let Some(method) = msg.get("method").and_then(Value::as_str) {
            if method == "textDocument/publishDiagnostics" {
                let params = &msg["params"];
                let uri = params["uri"].as_str().unwrap_or_default().to_string();
                let items = params["diagnostics"].as_array().map(|a| a.iter().filter_map(parse_diagnostic).collect()).unwrap_or_default();
                return vec![Event::Diagnostics { uri, items }];
            }
            return Vec::new();
        }
        let Some(id) = msg.get("id").and_then(Value::as_u64) else {
            return Vec::new();
        };
        let Some(method) = self.pending.remove(&id) else {
            return Vec::new();
        };
        let result = &msg["result"];
        match method {
            "initialize" => {
                self.utf8 = result["capabilities"]["positionEncoding"].as_str() == Some("utf-8");
                self.caps = result["capabilities"].clone();
                self.ready = true;
                let _ = self.send(&json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
                // As configurações do usuário (servidores que não perguntam por elas).
                if !self.settings.is_null() {
                    let settings = self.settings.clone();
                    let _ = self.send(&json!({ "jsonrpc": "2.0", "method": "workspace/didChangeConfiguration", "params": { "settings": settings } }));
                }
                for queued in std::mem::take(&mut self.queued) {
                    let _ = self.send(&queued);
                }
                Vec::new()
            }
            "textDocument/completion" => {
                let list = result.get("items").unwrap_or(result);
                let items = list
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|it| {
                                let label = it["label"].as_str()?.to_string();
                                let insert = it["insertText"].as_str().or_else(|| it["textEdit"]["newText"].as_str()).unwrap_or(&label).to_string();
                                let sort = it["sortText"].as_str().unwrap_or(&label).to_string();
                                let filter = it["filterText"].as_str().unwrap_or(&label).to_string();
                                Some(Completion {
                                    detail: it["detail"].as_str().unwrap_or_default().to_string(),
                                    insert,
                                    label,
                                    kind: it["kind"].as_u64().unwrap_or(1).min(25) as u8,
                                    doc: markup_text(&it["documentation"]),
                                    snippet: it["insertTextFormat"].as_u64() == Some(2),
                                    sort,
                                    filter,
                                    raw: Some(it.clone()),
                                })
                            })
                            .take(200)
                            .collect()
                    })
                    .unwrap_or_default();
                vec![Event::Completions { id, items }]
            }
            "textDocument/signatureHelp" => {
                let sigs = result["signatures"].as_array().cloned().unwrap_or_default();
                let active = result["activeSignature"].as_u64().unwrap_or(0) as usize;
                let signature = sigs.get(active).or(sigs.first()).map(|sig| {
                    let label = sig["label"].as_str().unwrap_or_default().to_string();
                    let param = sig["activeParameter"].as_u64().or(result["activeParameter"].as_u64()).unwrap_or(0) as usize;
                    let p = &sig["parameters"][param];
                    // O parâmetro vem como texto ou como [início, fim] em UTF-16.
                    let active = match &p["label"] {
                        Value::String(t) => label.find(t.as_str()).map(|i| (i, i + t.len())),
                        Value::Array(r) => {
                            let at = |n: u64| label.char_indices().scan(0u64, |u, (i, c)| { let here = *u; *u += c.len_utf16() as u64; Some((here, i)) }).find(|(u, _)| *u >= n).map_or(label.len(), |(_, i)| i);
                            Some((at(r[0].as_u64().unwrap_or(0)), at(r[1].as_u64().unwrap_or(0))))
                        }
                        _ => None,
                    };
                    let doc = [markup_text(&p["documentation"]), markup_text(&sig["documentation"])].into_iter().filter(|d| !d.is_empty()).collect::<Vec<_>>().join("\n\n");
                    Signature { label, active, doc }
                });
                vec![Event::Signature { id, signature }]
            }
            "textDocument/rename" | "textDocument/formatting" => {
                let edit = if method == "textDocument/formatting" {
                    // As trocas do arquivo pedido (o endereço fica para quem pediu).
                    vec![(String::new(), result.as_array().map(|a| a.iter().filter_map(parse_text_edit).collect()).unwrap_or_default())]
                } else {
                    parse_workspace_edit(result)
                };
                vec![Event::Edit { id, edit }]
            }
            "textDocument/codeAction" | "codeAction/resolve" => {
                let list: Vec<Value> = if method == "codeAction/resolve" { vec![result.clone()] } else { result.as_array().cloned().unwrap_or_default() };
                let actions = list
                    .iter()
                    .map(|a| {
                        // Um `Command` solto (com `command` em texto) ou uma `CodeAction`.
                        let command = if a["command"].is_string() { Some(a.clone()) } else { a.get("command").filter(|c| c.is_object()).cloned() };
                        let edit = parse_workspace_edit(&a["edit"]);
                        let raw = (edit.is_empty() && command.is_none() && a.get("data").is_some()).then(|| a.clone());
                        CodeAction { title: a["title"].as_str().unwrap_or_default().to_string(), kind: a["kind"].as_str().unwrap_or_default().to_string(), edit, command, raw }
                    })
                    .collect();
                vec![Event::Actions { id, actions }]
            }
            "textDocument/documentSymbol" => {
                let mut symbols = Vec::new();
                parse_symbols(result, 0, &mut symbols);
                vec![Event::Symbols { id, symbols }]
            }
            "textDocument/inlayHint" => {
                let hints = result
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|h| {
                                let (line, col) = position(&h["position"]);
                                let label = match &h["label"] {
                                    Value::String(s) => s.clone(),
                                    Value::Array(parts) => parts.iter().filter_map(|p| p["value"].as_str()).collect(),
                                    _ => String::new(),
                                };
                                InlayHint { line, col, label, pad_left: h["paddingLeft"].as_bool().unwrap_or(false), pad_right: h["paddingRight"].as_bool().unwrap_or(false) }
                            })
                            .filter(|h| !h.label.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                vec![Event::Hints { id, hints }]
            }
            "completionItem/resolve" => {
                let doc = markup_text(&result["documentation"]);
                let detail = result["detail"].as_str().unwrap_or_default().to_string();
                vec![Event::Resolved { id, doc, detail }]
            }
            "textDocument/definition" => {
                let loc = if result.is_array() { result.get(0).cloned().unwrap_or(Value::Null) } else { result.clone() };
                let uri = loc["uri"].as_str().or_else(|| loc["targetUri"].as_str()).unwrap_or_default().to_string();
                let range = if loc["range"].is_object() { &loc["range"] } else { &loc["targetSelectionRange"] };
                if uri.is_empty() {
                    return Vec::new();
                }
                vec![Event::Definition { id, uri, line: range["start"]["line"].as_u64().unwrap_or(0) as u32, col: range["start"]["character"].as_u64().unwrap_or(0) as u32 }]
            }
            "textDocument/hover" => {
                let contents = &result["contents"];
                let text = match contents {
                    Value::String(s) => s.clone(),
                    Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or_default().to_string(),
                    Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string).or_else(|| v["value"].as_str().map(str::to_string))).collect::<Vec<_>>().join("\n"),
                    _ => String::new(),
                };
                if text.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![Event::Hover { id, text }]
                }
            }
            _ => Vec::new(),
        }
    }

    /// Pede para o servidor terminar (e termina o processo de qualquer jeito).
    pub fn shutdown(&mut self) {
        if self.ready {
            let _ = self.request("shutdown", Value::Null);
            let _ = self.send(&json!({ "jsonrpc": "2.0", "method": "exit" }));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn parse_diagnostic(v: &Value) -> Option<Diagnostic> {
    let r = &v["range"];
    let pos = |p: &Value| Some((p["line"].as_u64()? as u32, p["character"].as_u64()? as u32));
    Some(Diagnostic {
        start: pos(&r["start"])?,
        end: pos(&r["end"])?,
        severity: match v["severity"].as_u64() {
            Some(1) => Severity::Error,
            Some(2) => Severity::Warning,
            _ => Severity::Info,
        },
        message: v["message"].as_str().unwrap_or_default().lines().next().unwrap_or_default().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mensagens_com_cabecalho() {
        let msg = json!({ "jsonrpc": "2.0", "id": 1, "result": { "ação": "ok" } });
        let bytes = frame(&msg);
        assert!(String::from_utf8_lossy(&bytes).starts_with("Content-Length: "));
        let mut reader = std::io::Cursor::new([bytes.clone(), bytes].concat());
        assert_eq!(read_message(&mut reader).unwrap(), msg);
        assert_eq!(read_message(&mut reader).unwrap(), msg);
        assert!(read_message(&mut reader).is_none());
    }

    #[test]
    fn posicoes_utf8_e_utf16() {
        let text = "ação\n😀 x\nfim";
        let x = text.find('x').unwrap();
        assert_eq!(to_position(text, x, true), (1, 5));
        // 😀 vale 2 unidades em UTF-16.
        assert_eq!(to_position(text, x, false), (1, 3));
        assert_eq!(to_offset(text, 1, 3, false), x);
        assert_eq!(to_offset(text, 1, 5, true), x);
        assert_eq!(to_offset(text, 0, 2, false), "aç".len());
        assert_eq!(to_offset(text, 9, 0, true), text.len());
    }

    #[test]
    fn diagnostico_lido() {
        let v = json!({ "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 9 } }, "severity": 1, "message": "erro\ndetalhe" });
        let d = parse_diagnostic(&v).unwrap();
        assert_eq!(d.start, (2, 4));
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "erro");
    }

    /// Servidor de mentira (um script do shell que responde ao `initialize` e manda um
    /// diagnóstico): o cliente inicia, avisa `initialized`, abre o documento e recebe o
    /// diagnóstico.
    #[test]
    fn conversa_com_servidor() {
        if Command::new("sh").arg("-c").arg("command -v python3").output().map(|o| !o.status.success()).unwrap_or(true) {
            return;
        }
        let dir = std::env::temp_dir().join(format!("mide-lsp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let server = dir.join("fake.py");
        std::fs::write(
            &server,
            r#"import sys, json
def read():
    n = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        line = line.strip()
        if not line: break
        if line.startswith(b"Content-Length:"): n = int(line.split(b":")[1])
    return json.loads(sys.stdin.buffer.read(n))
def send(m):
    b = json.dumps(m).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(b) + b); sys.stdout.flush()
while True:
    m = read()
    if m.get("method") == "initialize":
        send({"jsonrpc": "2.0", "id": m["id"], "result": {"capabilities": {"positionEncoding": "utf-8"}}})
    elif m.get("method") == "textDocument/didOpen":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}}, "severity": 1, "message": "ruim"}]}})
    elif m.get("method") == "exit":
        sys.exit(0)
"#,
        )
        .unwrap();
        let mut client = Client::start("python3", &[server.to_string_lossy().into_owned()], &dir).unwrap();
        let file = dir.join("a.rs");
        client.did_open(&file, "rust", 1, "abc");
        let mut got = None;
        for _ in 0..200 {
            for event in client.poll() {
                if let Event::Diagnostics { uri: u, items } = event {
                    got = Some((u, items));
                }
            }
            if got.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(client.utf8);
        let (u, items) = got.expect("diagnóstico");
        assert_eq!(u, uri(&file));
        assert_eq!(items[0].message, "ruim");
        drop(client);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod snippet_tests {
    use super::expand_snippet;

    #[test]
    fn trecho_abre_com_a_primeira_parada_escolhida() {
        let (t, stop) = expand_snippet("foreach (${1:\\$array} as ${2:\\$value}) {\n\t$0\n}");
        assert_eq!(t, "foreach ($array as $value) {\n\t\n}");
        assert_eq!(stop, Some((9, 15)));
        let (t, stop) = expand_snippet("println!(\"$1\")$0");
        assert_eq!(t, "println!(\"\")");
        assert_eq!(stop, Some((10, 10)));
        assert_eq!(expand_snippet("${1|pub,priv|} fn").0, "pub fn");
        assert_eq!(expand_snippet("ok").1, None);
    }
}

/// A parte das configurações do servidor pedida no `workspace/configuration`
/// (`section` com pontos: `"rust-analyzer.cargo"`), ou nulo.
pub fn config_section(settings: &Value, section: Option<&str>) -> Value {
    let Some(section) = section.filter(|s| !s.is_empty()) else {
        return settings.clone();
    };
    let mut v = settings;
    for part in section.split('.') {
        match v.get(part) {
            Some(next) => v = next,
            None => return Value::Null,
        }
    }
    v.clone()
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    #[test]
    fn trocas_aplicadas_de_tras_para_frente() {
        let text = "let a = 1;\nlet b = a;\n";
        let edits = vec![
            TextEdit { start: (0, 4), end: (0, 5), new_text: "x".into() },
            TextEdit { start: (1, 8), end: (1, 9), new_text: "x".into() },
        ];
        assert_eq!(apply_edits(text, &edits, true), "let x = 1;\nlet b = x;\n");
    }

    #[test]
    fn workspace_edit_nos_dois_formatos() {
        let v = json!({ "changes": { "file:///a": [{ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }, "newText": "z" }] } });
        assert_eq!(parse_workspace_edit(&v).len(), 1);
        let v = json!({ "documentChanges": [{ "textDocument": { "uri": "file:///b", "version": 1 }, "edits": [] }] });
        assert_eq!(parse_workspace_edit(&v)[0].0, "file:///b");
    }

    #[test]
    fn secao_da_configuracao() {
        let s = json!({ "rust-analyzer": { "cargo": { "features": "all" } } });
        assert_eq!(config_section(&s, Some("rust-analyzer.cargo")), json!({ "features": "all" }));
        assert_eq!(config_section(&s, Some("nada")), Value::Null);
    }
}
