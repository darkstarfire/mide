# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh e code-comum.sh.
# Code (mide-code), E11: a gaveta do projeto (menu de clique direito, divisória), as
# Preferências numa janela filha de vidro, a divisória do terminal, a
# divisão do editor arrastando uma aba ou um arquivo para a borda, o terminal integrado
# e a volta ao projeto da última vez.
#
# Coordenadas (1366x768; a janela do Code, 1040x640, nasce em (48, 76)):
# - Barra de ferramentas em y=102 (com a gaveta): abrir arquivo x=833, terminal
#   x=1008, Preferências x=1048.
# - Gaveta (barra lateral de vidro do kit, 230 de largura): primeira linha em y=190,
#   segunda em y=216; a divisória do kit entre x=274 e x=282. O menu de uma linha abre logo abaixo dela: Renomear +100,
#   Duplicar +126, Lixeira +152, Copiar caminho +187, Mostrar no Arquivos +213 (em y,
#   a partir da linha). No fundo da gaveta, o menu abre no ponto do clique: Novo
#   arquivo +25 e Nova pasta +51.
# - Abas (com dois arquivos e sem gaveta): a primeira entre x=52 e x=548, em y=145
#   (o × fica nos primeiros 24 pontos dela).
echo "== Code: projeto, divisão e terminal (mide-code)"
source tests/ui/code-comum.sh

new_project() {
    rm -rf "$CDIR/proj"
    mkdir -p "$CDIR/proj/src"
    printf 'fn main() {\n    println!("oi");\n}\n' > "$CDIR/proj/src/main.rs"
    printf '# Projeto\n' > "$CDIR/proj/README.md"
}

# Menu do fundo da gaveta: Novo arquivo "novo" e Nova pasta "pasta".
new_project
code_run "300 click 600 300; 330 rclick 150 400; 360 click 200 425; 400 type novo; 420 key enter; 450 rclick 150 450; 480 click 200 501; 520 type pasta; 540 key enter; 580 list" -- "'$CDIR/proj'"
ok_if "code: Novo arquivo pela gaveta (cria e abre)" test -f "$CDIR/proj/novo"
ok_if "code: Nova pasta pela gaveta" test -d "$CDIR/proj/pasta"

# Menu de um arquivo: Renomear (o nome sem a extensão vem escolhido), Duplicar, Mover
# para a Lixeira (a cópia), Copiar caminho e Mostrar no Arquivos.
new_project
code_run "300 click 600 300; 330 rclick 150 216; 360 click 200 316; 400 type novo; 420 key enter; 460 rclick 150 216; 490 click 200 342; 530 rclick 150 216; 560 click 200 368; 600 rclick 150 216; 630 click 200 403; 670 rclick 150 216; 700 click 200 429; 760 list" -- "'$CDIR/proj'"
ok_if "code: Renomear pela gaveta (README.md → novo.md)" test -f "$CDIR/proj/novo.md" -a ! -e "$CDIR/proj/README.md"
ok_if "code: Duplicar e mover a cópia para a Lixeira" grep -q 'para a Lixeira: .*novo cópia.md' "$CLOG"
ok_if "code: a cópia saiu do projeto" test ! -e "$CDIR/proj/novo cópia.md"
ok_if "code: Copiar caminho" grep -q "copiar (teste): $CDIR/proj/novo.md" "$CLOG"
ok_if "code: Mostrar no Arquivos" grep -q "mostrar no Arquivos (teste): $CDIR/proj/novo.md" "$CLOG"

# Arrastar uma aba até a borda direita do editor divide lado a lado.
new_project
code_run "300 click 600 400; 330 press 240 145; 340 move 260 160; 350 move 700 300; 360 move 1000 400; 370 move 1060 400; 380 release 1060 400; 420 list" -- "'$CDIR/proj/src/main.rs'" "'$CDIR/proj/README.md'"
ok_if "code: aba arrastada para a borda direita divide o editor" grep -q 'editor dividido: Right com main.rs' "$CLOG"

# Arrastar um arquivo da gaveta até a borda de baixo divide em cima/embaixo.
new_project
code_run "300 click 600 400; 330 press 130 216; 340 move 150 230; 350 move 500 500; 360 move 700 650; 370 move 700 670; 380 release 700 670; 420 list" -- "'$CDIR/proj'" "'$CDIR/proj/src/main.rs'"
ok_if "code: arquivo da gaveta solto embaixo divide o editor" grep -q 'editor dividido: Bottom com README.md' "$CLOG"

# Terminal integrado: botão da barra abre o shell na pasta do projeto; Ctrl+J fecha
# e abre de novo (o mesmo shell).
new_project
code_run "300 click 600 400; 330 click 1008 102; 400 type touch_criado; 420 key enter; 460 combo ctrl+j; 480 combo ctrl+j; 500 type touch_outro; 510 key enter; 560 list" MIDE_CODE_TEST_SHELL=/bin/sh -- "'$CDIR/proj'"
ok_if "code: terminal integrado roda comandos na pasta do projeto" test -f "$CDIR/proj/criado"
ok_if "code: Ctrl+J fecha e abre o terminal" test -f "$CDIR/proj/outro"

# Volta ao projeto da última vez: aberto sem nada, com a mesma pasta de estado.
new_project
code_run "300 list" XDG_STATE_HOME="'$CDIR/state'" -- "'$CDIR/proj'" "'$CDIR/proj/src/main.rs'"
check "code: aberto sem nada, volta ao projeto e às abas da última vez" 'janela "main.rs — Code"' "300 list" "XDG_STATE_HOME='$CDIR/state' $CODE_BIN"

# Divisória da gaveta: arrastar de x=275 a x=420 deixa a gaveta com 375 (guardado).
new_project
code_run "300 click 600 300; 330 press 275 400; 340 move 300 400; 350 move 380 400; 360 move 420 400; 370 release 420 400; 400 list" -- "'$CDIR/proj'"
ok_if "code: divisória da gaveta arrastável e guardada" grep -q '^drawer_width = 375' "$XDG_CONFIG_HOME/mide/code.toml"
rm -f "$XDG_CONFIG_HOME/mide/code.toml"

# Divisória do terminal (split_vertical do kit, linha em y=470 com 220 de altura):
# arrastar de y=470 a y=400 deixa o terminal com 290 (guardado).
new_project
code_run "300 click 600 300; 320 combo ctrl+j; 360 press 700 470; 370 move 700 450; 380 move 700 420; 390 move 700 400; 400 release 700 400; 430 list" MIDE_CODE_TEST_SHELL=/bin/sh -- "'$CDIR/proj'"
ok_if "code: divisória do terminal arrastável e guardada" grep -q '^terminal_height = 290' "$XDG_CONFIG_HOME/mide/code.toml"
rm -f "$XDG_CONFIG_HOME/mide/code.toml"

# Preferências (Ctrl+,) numa janela filha própria, centrada sobre a principal
# (moldura 560x500 em (288, 146)), com o vidro pelo nome app:<app_id>><título>.
check "code: Preferências numa janela filha centrada sobre a principal" 'janela "Preferências do Code" .*moldura=\(288, 146, 560, 500\)' "300 click 600 300; 320 combo ctrl+,; 420 list" "$CODE_BIN '$CDIR/proj'"
check "code: vidro na janela filha (app:mide-code>Preferências%20do%20Code)" 'vidro: desfoque refeito \([^:]*:app:mide-code>Preferências%20do%20Code:' "300 click 600 300; 320 combo ctrl+,; 420 list" "$CODE_BIN '$CDIR/proj'"
rm -f "$XDG_CONFIG_HOME/mide/code.toml"
rm -rf "$CDIR"
