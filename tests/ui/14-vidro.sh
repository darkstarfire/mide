# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== vidro em barras e menus"
check "barra superior sobre vidro" 'vidro: desfoque refeito \([^:]*:mide-bar#0\)' "300 list"
check "menu da barra sobre vidro" 'vidro: desfoque refeito \([^:]*:mide-bar-menu#0\)' "300 click 25 14; 360 list"
check "menu da dock sobre vidro" 'vidro: desfoque refeito \([^:]*:mide-dock-menu#0\)' "300 rclick 799 728; 360 list"
GCFG="$(mktemp -d)"
mkdir -p "$GCFG/mide"
printf '[effects]\npanel_glass = false\n' >"$GCFG/mide/config.toml"
GLOG="$(XDG_CONFIG_HOME="$GCFG" timeout 150 "$SCEN" "300 click 25 14; 360 list" 2>&1)"
if ! echo "$GLOG" | grep -q 'camada "mide-bar"' || echo "$GLOG" | grep -Eq "vidro: desfoque refeito \([^:]*:mide-bar"; then
    echo "FALHA sem vidro na barra e nos menus com o efeito desligado"; FAIL=$((FAIL + 1))
else
    echo "ok    sem vidro na barra e nos menus com o efeito desligado"; PASS=$((PASS + 1))
fi
rm -rf "$GCFG"

# Arquivos (mide-files) numa pasta de exemplo recriada a cada cenário. Janela em
# (48, 76): grade com Documentos antigos (345, 190), Fotos (463, 190), Projetos
# (581, 190); na 2ª linha "por do sol.png" (345, 310), praia.jpg (463, 310) e
# relatorio.pdf (581, 310). Botão de lista em (694, 106); Lixeira na barra lateral
# em (103, 435). Na lista, cabeçalho em y=152..176 e linhas de 26 px a partir de
# y=176 (centro da 1ª em 189).
FILES="$PWD/target/debug/mide-files"
FDIR="$(mktemp -d)/mide-arquivos-teste"
TRASH="${SCENARIO_OUT:-/tmp/mide-scenario}/data/Trash/files"
fixture() {
    rm -rf "$FDIR"
    mkdir -p "$FDIR/Documentos antigos" "$FDIR/Fotos" "$FDIR/Projetos"
    echo f >"$FDIR/Fotos/ferias.jpg"
    echo p >"$FDIR/Projetos/plano.txt"
    echo oi >"$FDIR/notas.txt"
    echo x >"$FDIR/relatorio.pdf"
    echo y >"$FDIR/música.mp3"
    cp /usr/share/pixmaps/debian-logo.png "$FDIR/praia.jpg" 2>/dev/null || echo z >"$FDIR/praia.jpg"
    echo w >"$FDIR/por do sol.png"
}
# file_check "nome" "teste do shell" "roteiro": roda o Arquivos e confere o disco.
file_check() {
    local name="$1" test="$2" script="$3"
    fixture
    timeout 150 "$SCEN" "$script" -- "$FILES '$FDIR'" >/dev/null 2>&1
    if eval "$test"; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name"; FAIL=$((FAIL + 1))
    fi
}
fixture
check "Arquivos abre a pasta pedida (título = pasta)" 'janela "mide-arquivos-teste"' "400 list" "$FILES '$FDIR'"
check "duplo clique numa pasta entra nela" 'janela "Fotos"' "400 click 463 190; 406 click 463 190; 440 list" "$FILES '$FDIR'"
check "Backspace volta para a pasta de cima" 'janela "mide-arquivos-teste"' "400 click 463 190; 406 click 463 190; 440 key backspace; 470 list" "$FILES '$FDIR'"
check "voltar da barra de ferramentas" 'janela "mide-arquivos-teste"' "400 click 463 190; 406 click 463 190; 440 click 296 106; 470 list" "$FILES '$FDIR'"
check "Lixeira na barra lateral" 'janela "Lixeira"' "400 click 103 435; 440 list" "$FILES '$FDIR'"
file_check "Delete manda o item selecionado para a Lixeira" '[ ! -e "$FDIR/relatorio.pdf" ] && [ -e "$TRASH/relatorio.pdf" ]' "400 click 581 310; 430 key delete; 470 list"
file_check "menu do fundo → Nova pasta, já pedindo o nome" '[ -d "$FDIR/arquivados" ] && [ ! -e "$FDIR/Nova pasta" ]' "400 rclick 600 450; 430 click 650 311; 470 type arquivados; 490 key enter; 530 list"
# Nova pasta e Novo arquivo abrem um diálogo (modal); só "Criar" cria.
file_check "Esc no diálogo não cria nada" '[ ! -e "$FDIR/Nova pasta" ] && [ ! -e "$FDIR/xyz" ]' "400 rclick 600 450; 430 click 650 311; 470 type xyz; 490 key esc; 530 list"
# Botão Cancelar do diálogo em (495, 306).
file_check "Cancelar no diálogo não cria nada" '[ ! -e "$FDIR/Nova pasta" ]' "400 rclick 600 450; 430 click 650 311; 470 click 495 306; 510 list"
file_check "Criar sem mudar o nome: Nova pasta, depois Nova pasta (1)" '[ -d "$FDIR/Nova pasta" ] && [ -d "$FDIR/Nova pasta (1)" ]' "400 rclick 600 450; 430 click 650 311; 460 key enter; 500 rclick 600 450; 530 click 650 311; 560 key enter; 600 list"
# Menu de um arquivo (não pasta): "Abrir com…" logo depois de "Abrir"; "Mover para a
# Lixeira" em y=541.
file_check "lista + menu do item → Mover para a Lixeira" '[ ! -e "$FDIR/notas.txt" ] && [ -e "$TRASH/notas.txt" ]' "400 click 694 106; 430 rclick 500 293; 460 click 550 541; 500 list"
file_check "menu do item → Renomear" '[ -e "$FDIR/imagens" ] && [ ! -e "$FDIR/Fotos" ]' "400 rclick 463 190; 430 click 515 410; 470 key backspace; 471 key backspace; 472 key backspace; 473 key backspace; 474 key backspace; 480 type imagens; 490 key enter; 530 list"
file_check "clique fora fecha o menu sem fazer nada" '[ -e "$FDIR/relatorio.pdf" ] && [ -e "$FDIR/notas.txt" ]' "400 rclick 581 310; 430 click 700 470; 460 click 630 281; 500 list"
# Lista: setinha de "Fotos" em (303, 215) abre a pasta na própria lista; o item de
# dentro aparece na linha seguinte (y=241).
# Fechar a pasta na lista (sanfona, A20): depois de fechar, a linha y=241 volta a ser
# Projetos (a de dentro de Fotos sumiu), e Delete leva Projetos.
file_check "lista: setinha fecha a pasta (as linhas de dentro saem)" '[ -e "$FDIR/Fotos/ferias.jpg" ] && [ -e "$TRASH/Projetos" ]' "400 click 694 106; 430 click 303 215; 480 click 303 215; 540 click 500 241; 560 key delete; 600 list"
file_check "lista: setinha abre a pasta na lista (e o item de dentro vai para a Lixeira)" '[ ! -e "$FDIR/Fotos/ferias.jpg" ] && [ -e "$TRASH/ferias.jpg" ]' "400 click 694 106; 430 click 303 215; 460 click 500 241; 480 key delete; 520 list"
# Fundo em (600, 450): o menu abre para cima (não cabe embaixo), com "Nova pasta" em
# y=311 e "Novo arquivo" em y=337. O nome vem todo selecionado e
# digitar define a extensão.
file_check "menu do fundo → Novo arquivo com a extensão digitada" '[ -f "$FDIR/leiame.md" ] && [ ! -e "$FDIR/Novo arquivo.txt" ]' "400 rclick 600 450; 430 click 650 337; 470 type leiame.md; 490 key enter; 530 list"
# Menu de "Projetos" (pasta, em (581, 190)), aberto embaixo do item: "Nova pasta aqui
# dentro" em y=349, "Novo arquivo aqui dentro" em y=375, "Comprimir" em y=462.
file_check "menu da pasta → Comprimir cria o .zip" '[ -f "$FDIR/Projetos.zip" ]' "400 rclick 581 190; 430 click 630 462; 560 list"
file_check "menu da pasta → Nova pasta aqui dentro" '[ -d "$FDIR/Projetos/rascunhos" ] && [ ! -e "$FDIR/rascunhos" ]' "400 rclick 581 190; 430 click 630 349; 470 type rascunhos; 490 key enter; 530 list"
file_check "menu da pasta → Novo arquivo aqui dentro" '[ -f "$FDIR/Projetos/ideias.md" ] && [ ! -e "$FDIR/ideias.md" ]' "400 rclick 581 190; 430 click 630 375; 470 type ideias.md; 490 key enter; 530 list"
# Barra lateral de vidro (acesso rápido): o compositor desfoca atrás da janela.
check "barra lateral do Arquivos sobre vidro" 'vidro: desfoque refeito \([^:]*:app:mide-files:[0-9]+#0\)' "400 list" "$FILES '$FDIR'"
file_check "Ctrl+D duplica" '[ -f "$FDIR/relatorio cópia.pdf" ]' "400 click 581 310; 430 combo ctrl+d; 470 list"
file_check "Ctrl+C e Ctrl+V copiam para outra pasta" '[ -f "$FDIR/Fotos/relatorio.pdf" ] && [ -f "$FDIR/relatorio.pdf" ]' "400 click 581 310; 420 combo ctrl+c; 440 click 463 190; 446 click 463 190; 480 combo ctrl+v; 520 list"
# Seleção múltipla (B8): relatorio.pdf e, com Ctrl, praia.jpg; Delete leva os dois.
file_check "Ctrl+clique escolhe vários e Delete leva todos para a Lixeira" '[ ! -e "$FDIR/relatorio.pdf" ] && [ ! -e "$FDIR/praia.jpg" ] && [ -e "$TRASH/relatorio.pdf" ] && [ -e "$TRASH/praia.jpg" ]' "400 click 581 310; 420 keydown ctrl; 430 click 463 310; 440 keyup ctrl; 460 key delete; 500 list"
# Shift+clique: de "por do sol.png" (345, 310) até relatorio.pdf (581, 310); Ctrl+C e
# Ctrl+V dentro de Projetos copiam os três.
file_check "Shift+clique escolhe o intervalo (e Ctrl+C/Ctrl+V copia todos)" '[ -f "$FDIR/Projetos/por do sol.png" ] && [ -f "$FDIR/Projetos/praia.jpg" ] && [ -f "$FDIR/Projetos/relatorio.pdf" ]' "400 click 345 310; 420 keydown shift; 430 click 581 310; 440 keyup shift; 450 combo ctrl+c; 470 click 581 190; 476 click 581 190; 510 combo ctrl+v; 550 list"
# Arrastar relatorio.pdf (581, 310) até a pasta Fotos (463, 190) move o arquivo.
file_check "arrastar um arquivo para uma pasta move" '[ -f "$FDIR/Fotos/relatorio.pdf" ] && [ ! -e "$FDIR/relatorio.pdf" ]' "400 move 581 310; 410 press 581 310; 420 move 560 290; 430 move 520 240; 440 move 470 200; 450 move 463 190; 460 release 463 190; 500 list"
# Abrir com…: com um app de teste (num XDG_DATA_DIRS próprio) que grava o caminho
# recebido; "Abrir com…" no menu de relatorio.pdf (o menu abre ao lado, subindo:
# (640, 230)) e o app na lista (335, 265).
fixture
OW="$(mktemp -d)"
mkdir -p "$OW/applications"
printf '#!/bin/sh\necho "$1" > "%s/aberto.txt"\n' "$OW" > "$OW/app.sh"; chmod +x "$OW/app.sh"
printf '[Desktop Entry]\nType=Application\nName=Leitor Teste\nExec=%s/app.sh %%F\nMimeType=application/pdf;\n' "$OW" > "$OW/applications/leitor-teste.desktop"
timeout 150 "$SCEN" "400 rclick 581 310; 430 click 640 230; 470 click 335 265; 520 list" -- "env -u MIDE_OPEN_DRY_RUN XDG_DATA_DIRS='$OW' $FILES '$FDIR'" >/dev/null 2>&1
if [ "$(cat "$OW/aberto.txt" 2>/dev/null)" = "$FDIR/relatorio.pdf" ]; then
    echo "ok    Abrir com… abre o arquivo no app escolhido"; PASS=$((PASS + 1))
else
    echo "FALHA Abrir com… abre o arquivo no app escolhido"; FAIL=$((FAIL + 1))
fi
rm -rf "$OW"
rm -rf "$(dirname "$FDIR")"
