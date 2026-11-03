# Funções das seções do Code (tests/ui/34-code-*.sh e 35-code-*.sh), sobre as de
# tests/ui/comum.sh: abrir o Code num cenário com o log do app num arquivo ($CLOG) e
# conferir o resultado (no log do app ou nos arquivos).
CODE_BIN="$PWD/target/debug/mide-code"
CDIR="$(mktemp -d)"
CLOG="$CDIR/code.log"

# code_run "roteiro" [VAR=valor...] -- argumentos do Code: o log do app fica em $CLOG.
code_run() {
    local script="$1"
    shift
    local envs=""
    while [ $# -gt 0 ] && [ "$1" != "--" ]; do
        envs="$envs $1"
        shift
    done
    [ "${1:-}" = "--" ] && shift
    rm -f "$CLOG"
    # Sem o painel de símbolos (ele só aparece com servidor de linguagem instalado, o
    # que mudaria as posições de uma máquina para outra).
    mkdir -p "$XDG_CONFIG_HOME/mide"
    grep -q '^outline' "$XDG_CONFIG_HOME/mide/code.toml" 2>/dev/null || echo 'outline = false' >> "$XDG_CONFIG_HOME/mide/code.toml"
    timeout 150 "$SCEN" "$script" -- "RUST_LOG=mide_code=info,mide_ui=info $envs $CODE_BIN $* >'$CLOG' 2>&1" >/dev/null 2>&1
}

# ok_if "nome" comando...: passa se o comando der certo.
ok_if() {
    local name="$1"
    shift
    if "$@"; then
        echo "ok    $name"
        PASS=$((PASS + 1))
    else
        echo "FALHA $name"
        FAIL=$((FAIL + 1))
    fi
}

