# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== modo tablet: gestos das bordas, apps em tela inteira e launcher pelo toque"
# Gestos (só no modo tablet, `tablet on`): da borda de baixo para cima abre o launcher;
# subindo e parando, a visão das janelas; dos cantos de cima para baixo, a central de
# controle (direita) e as notificações (esquerda, no painel do relógio).
# Launcher em 1366x768: grade de 8x3 ícones de 90 pt; o primeiro ícone tem o centro
# em (179, 213); o vazio abaixo da grade fica em (683, 640).
ok() { echo "ok    $1"; PASS=$((PASS + 1)); }
falha() { echo "FALHA $1"; FAIL=$((FAIL + 1)); }
JANELA="$PWD/target/debug/examples/mide-janela-teste"
TG="$(mktemp -d)"
tg_run() { # tg_run nome "roteiro" [app...]
    local name="$1" script="$2"
    shift 2
    mkdir -p "$TG/$name"
    SCENARIO_OUT="$TG/$name" timeout 150 "$SCEN" "$script" ${1:+--} "$@" >/dev/null 2>&1
}
tg_log() { sed 's/\x1b\[[0-9;]*m//g' "$TG/$1/mide.log" 2>/dev/null; }

# De baixo para cima: o launcher sobe acompanhando o dedo e abre.
tg_run baixo "300 tablet on; 340 touch down 683 765; 344 touch move 683 740; 348 touch move 683 600; 352 touch move 683 420; 356 touch up; 460 list"
if tg_log baixo | grep -q "toque: gesto da borda de baixo" && tg_log baixo | grep -q "gesto: launcher aberto" && tg_log baixo | grep -q 'camada "mide-launcher"'; then
    ok "deslizar de baixo para cima abre o launcher"
else
    falha "deslizar de baixo para cima abre o launcher"
fi
# Subir e parar: a visão das janelas no lugar do launcher.
tg_run segura "300 tablet on; 340 touch down 683 765; 344 touch move 683 700; 348 touch move 683 560; 420 touch up; 460 list"
if tg_log segura | grep -q "toque: subir e parar" && tg_log segura | grep -q "visão das janelas: abrindo"; then
    ok "deslizar de baixo e parar abre a visão das janelas"
else
    falha "deslizar de baixo e parar abre a visão das janelas"
fi
# Fora do modo tablet a borda é um toque comum.
tg_run notebook "340 touch down 683 765; 344 touch move 683 600; 356 touch up; 420 list"
if ! tg_log notebook | grep -q "gesto da borda"; then
    ok "fora do modo tablet a borda de baixo não tem gesto"
else
    falha "fora do modo tablet a borda de baixo não tem gesto"
fi
# Canto de cima à direita: a central de controle; à esquerda: as notificações.
tg_run cantos "300 tablet on; 340 touch down 1300 3; 344 touch move 1300 80; 348 touch up; 420 key esc; 440 touch down 30 3; 444 touch move 30 80; 448 touch up; 520 list"
if tg_log cantos | grep -q "barra: central de controle (gesto do canto)" && tg_log cantos | grep -q "barra: notificações (gesto do canto)"; then
    ok "cantos de cima abrem a central de controle e as notificações"
else
    falha "cantos de cima abrem a central de controle e as notificações"
fi
# Modo tablet: janela nova abre ocupando a tela.
if [ -x "$JANELA" ]; then
    tg_run maxi "300 tablet on; 500 list" "sleep 2; $JANELA comum"
    if tg_log maxi | grep -qE 'janela "Comum" popups=0 posição=\(0, 58\) visível=1366x'; then
        ok "no modo tablet a janela nova ocupa a tela"
    else
        falha "no modo tablet a janela nova ocupa a tela ($(tg_log maxi | grep -o 'janela "Comum" popups=0 posição=([0-9, ]*) visível=[0-9x]*' | tail -1))"
    fi
fi
# Dock maior no modo tablet (alvos para o dedo).
tg_run dock "300 tablet on; 460 list"
if tg_log dock | grep -q "dock: modo tablet (ícones maiores)"; then
    ok "no modo tablet a dock aumenta os ícones"
else
    falha "no modo tablet a dock aumenta os ícones"
fi

# Launcher pelo dedo: arrastar sobre os ícones passa a página (não arrasta o ícone).
tg_run pagina "300 combo ctrl+shift+space; 400 touch down 900 300; 404 touch move 850 300; 408 touch move 700 300; 412 touch move 500 300; 416 touch up; 480 list"
if tg_log pagina | grep -q "página 0 → 1 (com o dedo)" && ! tg_log pagina | grep -q "arrastando o item"; then
    ok "launcher: o dedo arrastando passa a página"
else
    falha "launcher: o dedo arrastando passa a página"
fi
# Segurar um ícone: modo de edição (os ícones balançam); tocar no vazio sai.
tg_run edita "300 combo ctrl+shift+space; 400 touch down 179 213; 560 touch up; 590 touch down 683 640; 592 touch up; 630 list"
if tg_log edita | grep -q "launcher: modo de edição" && tg_log edita | grep -q "launcher: fim do modo de edição" && ! tg_log edita | grep -q "launcher fechado"; then
    ok "launcher: segurar um ícone entra no modo de edição, tocar no vazio sai"
else
    falha "launcher: segurar um ícone entra no modo de edição, tocar no vazio sai"
fi
# Com o mouse: arrastar no vazio passa a página; segurar um ícone 1 s entra no modo de
# edição e aí ele se arrasta; Esc sai.
tg_run mouse "300 combo ctrl+shift+space; 400 press 900 640; 404 move 800 640; 408 move 600 640; 412 release 500 640; 480 press 179 213; 640 move 200 230; 644 move 260 260; 648 release 260 260; 700 key esc; 740 list"
if tg_log mouse | grep -q "página 0 → 1 (com o dedo)" && tg_log mouse | grep -q "launcher: modo de edição" && tg_log mouse | grep -q "launcher: fim do modo de edição"; then
    ok "launcher: o mouse arrasta a página no vazio e o ícone entra no modo de edição"
else
    falha "launcher: o mouse arrasta a página no vazio e o ícone entra no modo de edição"
fi
# Arrastar um ícone sem segurar não o tira do lugar (passa a página, como o dedo).
tg_run rapido "300 combo ctrl+shift+space; 400 press 179 213; 404 move 150 213; 408 move 100 213; 412 move 60 213; 416 release 60 213; 480 list"
if ! tg_log rapido | grep -q "launcher: modo de edição\|arrastando o item" && tg_log rapido | grep -q "página arrastada"; then
    ok "launcher: arrastar um ícone sem segurar passa a página e não entra no modo de edição"
else
    falha "launcher: arrastar um ícone sem segurar passa a página e não entra no modo de edição"
fi
rm -rf "$TG"
