# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh e code-comum.sh.
# Code (mide-code), M10: o painel do Claude (Ctrl+Shift+L) com um `claude` falso
# (tests/fake/claude-bin/claude, o mesmo protocolo stream-json do Claude Code): a
# mensagem vai ao programa, o pedido de licença aparece e Permitir/Negar respondem.
#
# Coordenadas (1366x768; a janela do Code nasce em (48, 76)): campo da mensagem em
# (900, 634); no pedido de licença, Negar em (805, 480) e Permitir em (894, 480).
echo "== Code: painel do Claude (mide-code)"
source tests/ui/code-comum.sh

FAKE="$PWD/tests/fake/claude-bin"
claude_project() {
    rm -rf "$CDIR/proj" "$CDIR/claude.log"
    mkdir -p "$CDIR/proj/src"
    printf 'fn main() {}\n' > "$CDIR/proj/src/main.rs"
}

claude_project
code_run "300 click 600 400; 330 combo ctrl+shift+l; 380 click 900 634; 400 type um arquivo; 420 key enter; 470 click 894 480; 520 list" "PATH=$FAKE:\$PATH" "FAKE_CLAUDE_LOG=$CDIR/claude.log" -- "'$CDIR/proj'"
ok_if "code: a mensagem chega ao Claude" grep -q '"content":"um arquivo"' "$CDIR/claude.log"
ok_if "code: Permitir responde allow e o Claude escreve" grep -q '"behavior": "allow"' "$CDIR/claude.log"
ok_if "code: o arquivo permitido foi criado" test -f "$CDIR/proj/criado.txt"

claude_project
code_run "300 click 600 400; 330 combo ctrl+shift+l; 380 click 900 634; 400 type outro; 420 key enter; 470 click 805 480; 520 list" "PATH=$FAKE:\$PATH" "FAKE_CLAUDE_LOG=$CDIR/claude.log" -- "'$CDIR/proj'"
ok_if "code: Negar responde deny" grep -q '"behavior": "deny"' "$CDIR/claude.log"
ok_if "code: nada escrito sem licença" test ! -e "$CDIR/proj/criado.txt"
