# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh e code-comum.sh.
# Code (mide-code), E11: servidor de linguagem de mentira (tests/code-lsp-fake.py) para
# o completar (Tab/Shift+Tab/Enter), ir para a definição e o próximo problema; o
# arquivo dos servidores aberto pelas Preferências e lido de novo ao salvar; buscar e
# substituir no projeto; mover linhas, ir para a linha e salvar tudo.
#
# Coordenadas (1366x768; a janela do Code nasce em (48, 76)): Preferências (Ctrl+,)
# com "Editar servidores de linguagem…" em (420, 642). Buscar no projeto
# (Ctrl+Shift+F): "Substituir por" em (568, 336), "Substituir em tudo" em
# (627, 378) e a primeira linha encontrada em (320, 444).
echo "== Code: completar, servidores de linguagem, busca e edição (mide-code)"
source tests/ui/code-comum.sh
FAKE_LSP="$PWD/tests/code-lsp-fake.py"
mkdir -p "$XDG_CONFIG_HOME/mide" "$CDIR/proj/src"

if command -v python3 >/dev/null; then
    printf '[[server]]\nlanguages = ["rust"]\ncommand = "%s"\n' "$FAKE_LSP" > "$XDG_CONFIG_HOME/mide/code-servers.toml"
    # Completar com Alt+Espaço: Tab, Tab e Shift+Tab param no segundo ("beta"); Enter
    # põe no texto (o espaço do atalho não fica).
    printf 'a\n' > "$CDIR/t.rs"
    code_run "400 click 600 300; 420 combo ctrl+end; 460 combo alt+space; 560 key tab; 580 key tab; 600 combo shift+tab; 620 key enter; 640 combo ctrl+s; 680 list" -- "'$CDIR/t.rs'"
    ok_if "code: completar do LSP com Tab, Shift+Tab e Enter" test "$(cat "$CDIR/t.rs")" = "$(printf 'a\nbeta')"
    # Esc fecha a lista sem pôr nada.
    printf 'a\n' > "$CDIR/t.rs"
    code_run "400 click 600 300; 420 combo ctrl+end; 460 combo alt+space; 560 key esc; 580 key enter; 600 combo ctrl+s; 640 list" -- "'$CDIR/t.rs'"
    ok_if "code: Esc fecha o completar" test "$(cat "$CDIR/t.rs")" = "$(printf 'a\n\n')"
    # F8 vai ao erro que o servidor mandou; Ctrl+clique em "bb" (linha 2, em (95, 163))
    # vai à definição (o servidor diz linha 3). O F12 é do terminal de atalho do mide.
    printf 'a\nbb\ncccccc\n' > "$CDIR/t.rs"
    code_run "400 click 600 300; 460 key f8; 500 keydown ctrl; 510 click 95 163; 520 keyup ctrl; 560 list" -- "'$CDIR/t.rs'"
    ok_if "code: ir para a definição (Ctrl+clique)" grep -q 'definição: .*t.rs:3' "$CLOG"
    ok_if "code: próximo problema (F8) mostra o erro do servidor" grep -q 'aviso: erro de teste' "$CLOG"
else
    echo "pulo  code: completar, definição e problemas (sem python3 para o servidor de mentira)"
fi

# Preferências → "Editar servidores de linguagem…" abre o code-servers.toml no Code;
# salvar com erro avisa a linha; desfeito o erro, os servidores valem de novo.
rm -f "$XDG_CONFIG_HOME/mide/code-servers.toml"
code_run "300 click 600 400; 320 combo ctrl+,; 360 click 420 642; 420 combo ctrl+end; 430 type x; 440 combo ctrl+s; 480 key backspace; 490 combo ctrl+s; 540 list" -- "'$CDIR/proj'"
ok_if "code: Preferências abrem o code-servers.toml no editor" grep -q 'aberto: .*code-servers.toml' "$CLOG"
ok_if "code: erro no code-servers.toml mostra a linha" grep -q 'Erro no code-servers.toml, linha' "$CLOG"
ok_if "code: code-servers.toml salvo vale na hora" grep -q 'Servidores de linguagem atualizados (6)' "$CLOG"
rm -f "$XDG_CONFIG_HOME/mide/code-servers.toml"

# Buscar no projeto e substituir em todos os arquivos (fechados: gravados no disco).
printf 'fn main() {\n    println!("oi");\n}\n' > "$CDIR/proj/src/main.rs"
code_run "300 click 600 400; 320 combo ctrl+shift+f; 360 type oi; 370 key enter; 400 click 568 336; 420 type ola; 440 click 627 378; 500 list" -- "'$CDIR/proj'"
ok_if "code: buscar no projeto" grep -q 'busca no projeto: "oi" → 1 linha' "$CLOG"
ok_if "code: substituir em todo o projeto" grep -q 'println!("ola");' "$CDIR/proj/src/main.rs"
# Clique numa linha encontrada abre o arquivo.
check "code: linha encontrada abre o arquivo" 'janela "main.rs — Code"' "300 click 600 400; 320 combo ctrl+shift+f; 360 type ola; 370 key enter; 420 click 320 444; 480 list" "$CODE_BIN '$CDIR/proj'"

# Mover linha (Alt+↓), ir para a linha (Ctrl+L) e salvar tudo (Ctrl+Alt+S).
printf 'um\ndois\n' > "$CDIR/m.txt"
code_run "400 click 600 300; 420 combo ctrl+home; 440 combo alt+down; 460 combo ctrl+s; 500 list" -- "'$CDIR/m.txt'"
ok_if "code: Alt+↓ move a linha" test "$(cat "$CDIR/m.txt")" = "$(printf 'dois\num\n')"
printf 'a\nb\nc\n' > "$CDIR/l.txt"
code_run "400 click 600 300; 420 combo ctrl+l; 440 type 3; 450 key enter; 470 type x; 490 combo ctrl+s; 530 list" -- "'$CDIR/l.txt'"
ok_if "code: Ctrl+L vai para a linha" test "$(sed -n 3p "$CDIR/l.txt")" = "xc"
printf 'um\n' > "$CDIR/s1.txt"; printf 'dois\n' > "$CDIR/s2.txt"
# A aba aberta por último (s2) é a atual: x vai nela e y no s1.
code_run "400 click 600 400; 420 combo ctrl+home; 430 type x; 450 combo ctrl+tab; 470 combo ctrl+home; 480 type y; 500 combo ctrl+alt+s; 540 list" -- "'$CDIR/s1.txt'" "'$CDIR/s2.txt'"
ok_if "code: salvar tudo (Ctrl+Alt+S)" test "$(cat "$CDIR/s1.txt")$(cat "$CDIR/s2.txt")" = "yumxdois"
rm -rf "$CDIR"
