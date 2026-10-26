#!/usr/bin/env python3
# Servidor de linguagem (LSP) de mentira para os testes do Code (tests/ui/34-code-*.sh):
# responde ao initialize (posições em UTF-8), manda um erro na linha 1 ao abrir um
# documento, completa com "alfa", "beta" e "gama", diz que a definição de qualquer
# coisa fica na linha 3 (coluna 4) do mesmo arquivo e dá uma dica fixa.
# Cada mensagem recebida vai para o arquivo de $FAKE_LSP_LOG (se definido).
import json
import os
import sys

LOG = os.environ.get("FAKE_LSP_LOG")


def log(text):
    if LOG:
        with open(LOG, "a") as f:
            f.write(text + "\n")


def read():
    n = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        line = line.strip()
        if not line:
            break
        if line.lower().startswith(b"content-length:"):
            n = int(line.split(b":")[1])
    return json.loads(sys.stdin.buffer.read(n))


def send(msg):
    body = json.dumps(msg).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.flush()


while True:
    m = read()
    method = m.get("method")
    log(f"recebido {method}")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": m["id"], "result": {"capabilities": {"positionEncoding": "utf-8"}}})
    elif method == "textDocument/didOpen":
        uri = m["params"]["textDocument"]["uri"]
        diag = {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}}, "severity": 1, "message": "erro de teste"}
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": [diag]}})
    elif method == "textDocument/completion":
        items = [{"label": w, "detail": "teste"} for w in ("alfa", "beta", "gama")]
        send({"jsonrpc": "2.0", "id": m["id"], "result": {"isIncomplete": False, "items": items}})
    elif method == "textDocument/definition":
        uri = m["params"]["textDocument"]["uri"]
        loc = {"uri": uri, "range": {"start": {"line": 2, "character": 4}, "end": {"line": 2, "character": 5}}}
        send({"jsonrpc": "2.0", "id": m["id"], "result": [loc]})
    elif method == "textDocument/hover":
        send({"jsonrpc": "2.0", "id": m["id"], "result": {"contents": {"kind": "plaintext", "value": "dica de teste"}}})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": m["id"], "result": None})
    elif method == "exit":
        sys.exit(0)
