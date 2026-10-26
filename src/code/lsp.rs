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
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub label: String,
    pub detail: String,
    /// Texto que entra (o `insertText`, ou o rótulo).
    pub insert: String,
}

/// Eventos vindos do servidor.
#[derive(Debug, Clone)]
pub enum Event {
    Diagnostics { uri: String, items: Vec<Diagnostic> },
    Completions { id: u64, items: Vec<Completion> },
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
        let mut child = Command::new(command).args(args).current_dir(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
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
        let mut client = Self { child, stdin, rx, next_id: 1, pending: HashMap::new(), utf8: false, ready: false, queued: Vec::new(), name: command.to_string() };
        let params = json!({
            "processId": std::process::id(),
            "rootUri": uri(root),
            "workspaceFolders": [{ "uri": uri(root), "name": root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() }],
            "clientInfo": { "name": "mide-code" },
            "capabilities": {
                "general": { "positionEncodings": ["utf-8", "utf-16"] },
                "textDocument": {
                    "synchronization": { "didSave": true },
                    "publishDiagnostics": {},
                    "completion": { "completionItem": { "snippetSupport": false } },
                    "definition": {},
                    "hover": { "contentFormat": ["plaintext", "markdown"] }
                }
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
        // Pedido do servidor (ex.: workspace/configuration): responde vazio.
        if let (Some(id), Some(method)) = (msg.get("id"), msg.get("method").and_then(Value::as_str)) {
            let result = if method == "workspace/configuration" { json!([]) } else { Value::Null };
            let _ = self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            return Vec::new();
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
                self.ready = true;
                let _ = self.send(&json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
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
                                Some(Completion { detail: it["detail"].as_str().unwrap_or_default().to_string(), insert, label })
                            })
                            .take(200)
                            .collect()
                    })
                    .unwrap_or_default();
                vec![Event::Completions { id, items }]
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
