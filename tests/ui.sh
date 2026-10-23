#!/usr/bin/env bash
# Bateria de regressão da interface: janelas, controles, popups, barra e dock.
#
# Usa tests/scenario.sh (autoteste por MIDE_SELFTEST) e confere o log de cada
# cenário. Usa o Terminal e a Calculadora do mide (a gnome-calculator, se
# instalada, testa também as janelas com cabeçalho do próprio app).
#
# Coordenadas (saída 1366x768, a tela mais comum no Brasil; barra superior de 28 px):
# - A primeira janela nasce em (48, 76) e a segunda em (80, 108).
# - Barra do mide numa janela em (48, 76): moldura em y=46, botões no centro
#   y=61: fechar x=63, minimizar x=83, maximizar x=103.
# - Calculadora (360x509, cabeçalho próprio, configurações padrão do GTK: só o X) em
#   (48, 76): X em (384, 99), menu (345, 99); duplo clique no cabeçalho (300, 99)
#   maximiza; borda direita em x=408.
# - Maximizada: área útil de y=28 até 2 px acima da dock (y=701).
# - Dock padrão (ícones de 40 px) em (541, 703) 285x57: Launchpad em (568, 728),
#   Terminal em (611, 728), Calculadora em (654, 728), Configurações em (697, 728),
#   Downloads em (756, 728) e Lixeira em (799, 728).
# - Menu de contexto de um app aberto (clique direito): "Manter/Remover da Dock" em
#   y=609, "Ocultar" em y=644 e "Encerrar" em y=670. App fechado: "Remover" em y=670.
# - Barra superior: marca em (25, 14); menu: "Sobre este computador" em (100, 49).
#   À direita: relógio (1331, 14), central de controle (1293, 14), Bluetooth (1257, 14),
#   Wi-Fi (1221, 14), som (1185, 14) e engrenagem (1149, 14).
set -u

cd "$(dirname "$0")/.."
SCEN=tests/scenario.sh
export MIDE_WINIT_SIZE=1366x768
export SCENARIO_WAIT="${SCENARIO_WAIT:-10}"
PASS=0
FAIL=0

# Trava: a bateria não pode alterar a configuração real do usuário.
REAL_CFG="${XDG_CONFIG_HOME:-$HOME/.config}/mide"
cfg_fingerprint() { (cd "$REAL_CFG" 2>/dev/null && ls -A | sort && cat ./* 2>/dev/null) | md5sum; }
CFG_BEFORE="$(cfg_fingerprint)"

# Todas as sessões de teste usam uma configuração temporária e vazia: nada que
# aconteça numa janela de teste (nem um clique de quem estiver usando o PC)
# grava na configuração real.
TESTCFG="$(mktemp -d)"
export XDG_CONFIG_HOME="$TESTCFG"
trap 'rm -rf "$TESTCFG"' EXIT

# check "nome" "regex esperado no log" "roteiro" app...
check() {
    local name="$1" expected="$2" script="$3"
    shift 3
    local log
    log="$(timeout 150 "$SCEN" "$script" -- "$@" 2>&1)"
    if echo "$log" | grep -Eq "$expected"; then
        echo "ok    $name"
        PASS=$((PASS + 1))
    else
        echo "FALHA $name"
        echo "      esperado: $expected"
        echo "$log" | grep -E "autoteste: (janela|camada)|janela (fechada|minimizada|restaurada)" | sed 's/^/      /' | tail -4
        FAIL=$((FAIL + 1))
    fi
}

# check_last "nome" "regex" "roteiro" app...: a última janela listada (a do topo) casa.
check_last() {
    local name="$1" expected="$2" script="$3"
    shift 3
    local last
    last="$(timeout 150 "$SCEN" "$script" -- "$@" 2>&1 | grep "autoteste: janela" | tail -1)"
    if echo "$last" | grep -Eq "$expected"; then
        echo "ok    $name"
        PASS=$((PASS + 1))
    else
        echo "FALHA $name"
        echo "      topo: $last"
        FAIL=$((FAIL + 1))
    fi
}

# check_count "nome" "regex" quantidade "roteiro" app...: quantas linhas de listagem casam.
check_count() {
    local name="$1" pattern="$2" want="$3" script="$4"
    shift 4
    local got
    got="$(timeout 150 "$SCEN" "$script" -- "$@" 2>&1 | grep -E "autoteste: (janela|camada|tela)" | grep -Ec "$pattern")"
    if [ "$got" = "$want" ]; then
        echo "ok    $name"
        PASS=$((PASS + 1))
    else
        echo "FALHA $name (esperado $want, obtido $got)"
        FAIL=$((FAIL + 1))
    fi
}

TERMINAL="$PWD/target/debug/mide-terminal --title Terminal"
CALCULATOR="$PWD/target/debug/mide-calculator"
FOOT='janela "Terminal" popups=0'
CALC='janela "Calculadora"'
CALC_OPEN='janela "Calculadora"'

echo "== barra superior"
check "barra no topo, largura total" 'camada "mide-bar" em \(0, 0\) 1366x28' "400 list" "$TERMINAL"
check "janela nova nasce abaixo da barra" "$FOOT posição=\(48, 76\)" "400 list" "$TERMINAL"
check "menu abre ao clicar na marca" 'camada "mide-bar-menu"' "400 click 25 14; 430 list"
check_count "menu fecha ao clicar fora" 'camada "mide-bar-menu"' 0 "400 click 25 14; 430 click 600 400; 460 list"
check "arrastar para cima para na barra" "$FOOT posição=\(48, 58\)" "400 press 400 61; 405 move 400 30; 410 move 400 5; 415 release 400 5; 440 list" "$TERMINAL"
# App que esconde a janela sem fechar e mostra de novo (como o Chromium/Brave em
# segundo plano; `MIDE_KIT_TEST_HIDE` faz o kit esconder 2 s e mostrar 2 s depois).
SCENARIO_WAIT=14 check "janela escondida pelo app volta quando ele mostra" "janela mostrada de novo pelo app" "150 list; 600 list" "MIDE_KIT_TEST_HIDE=1 $CALCULATOR"
# Transparência de janelas inteiras ([opacity] inactive): o Terminal sem foco deixa o
# papel de parede aparecer através dele (em (500, 500) ele é branco quando opaco).
OPC="$(mktemp -d)"
mkdir -p "$OPC/cfg/mide"
printf '[opacity]\ninactive = 0.5\n' > "$OPC/cfg/mide/config.toml"
XDG_CONFIG_HOME="$OPC/cfg" SCENARIO_OUT="$OPC/scen" timeout 150 "$SCEN" "400 list" 420 -- "$TERMINAL" "$CALCULATOR" >/dev/null 2>&1
if python3 -c "
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert('RGB')
sys.exit(0 if im.getpixel((500, 500)) != (255, 255, 255) and im.getpixel((150, 300)) == (36, 36, 40) else 1)" "$OPC/scen/frame.png" 2>/dev/null; then
    echo "ok    janela sem foco translúcida ([opacity] inactive), a com foco opaca"; PASS=$((PASS + 1))
else
    echo "FALHA janela sem foco translúcida ([opacity] inactive)"; FAIL=$((FAIL + 1))
fi
rm -rf "$OPC"
# Clique numa janela por cima da barra de título de outra (Calculadora em (48, 46),
# Terminal arrastado para cobrir a barra dela): o clique é do Terminal, a de trás não sobe.
check_last "clique não atravessa para a barra de título de trás" 'janela "Terminal"' "400 press 400 93; 405 move 400 60; 410 move 400 20; 415 release 400 20; 460 click 200 68; 490 list" "$CALCULATOR" "$TERMINAL"
# Menus padrão do mide (o Arquivos não publica menus): "Editar" em x=213, "Janela" em 268.
FILES_APP="$PWD/target/debug/mide-files"
check "menu Editar abre pela barra" "menu aberto: Edit" "400 click 213 14; 440 list" "$FILES_APP"
check "Janela > Zoom maximiza" 'janela "Pasta pessoal" popups=0 posição=\(0, 58\) visível=1366x643' "400 click 268 14; 430 click 280 73; 480 list" "$FILES_APP"
# Menus do próprio Terminal (dbusmenu, num barramento de teste): Shell em x=140,
# Editar 194, Janela 382; no menu, a primeira linha em y=55 e a segunda em y=79.
RUST_LOG="mide=debug,mide_bar=info" SCEN=tests/scenario-dbus.sh check "a barra mostra os menus do Terminal" "menu do app mide-terminal: Shell, Editar, Visualizar, Perfis, Janela" "500 list" "$TERMINAL"
SCEN=tests/scenario-dbus.sh check "Terminal: Janela > Zoom pelo menu do app" "$FOOT posição=\(0, 58\) visível=1366x643" "500 click 382 14; 540 click 400 79; 600 list" "$TERMINAL"
check "app > Encerrar fecha o app" "janela fechada" "400 click 65 14; 430 click 90 82" "$TERMINAL"
check "janela Sobre redimensiona" 'janela "Sobre este computador" .*visível=580x400' "400 click 25 14; 430 click 100 49; 520 list; 530 press 587 435; 535 move 607 455; 540 move 627 475; 545 release 627 475; 580 list"
check "relógio abre o calendário" "menu aberto: Calendar" "400 click 1331 14; 440 list"
# Calendário (x 1078..1358): "Formato da hora…" em y=323; nas opções, "Segundos" em (1323, 213).
check "calendário: Formato da hora liga os segundos" "relógio: Segundos ligado" "400 click 1331 14; 440 click 1218 323; 470 click 1323 213; 500 list"
# Os segundos alargam o relógio e empurram os ícones: volta ao padrão (só a hora).
sed -i '/^seconds = true/d' "$XDG_CONFIG_HOME/mide/config.toml" 2>/dev/null
# Central de controle (ícone em 1293, painel em x 1038..1358): "Não perturbe" em (1276, 102)
# (ladrilho da direita).
check "central de controle abre" "menu aberto: Control" "400 click 1293 14; 440 list"
check "central de controle: Não perturbe" "não perturbe: ligado" "400 click 1293 14; 440 click 1276 102; 470 list"
sed -i '/^dnd = true/d' "$XDG_CONFIG_HOME/mide/config.toml" 2>/dev/null
check "engrenagem abre as Configurações" "sistema \(teste\): mide-settings" "400 click 1149 14; 440 list"
# Menu do mide: Repouso em y=153, Reiniciar… em y=177; confirmação no meio da tela:
# Reiniciar em (753, 412), Cancelar em (614, 412). Nada é executado de verdade (simulado).
check "menu do mide: Repouso (simulado)" "sistema \(teste\): logind Suspend" "400 click 25 14; 430 click 100 153; 460 list"
check "menu do mide: Reiniciar pede confirmação" "menu aberto: ConfirmRestart" "400 click 25 14; 430 click 100 177; 460 list"
check "confirmar reinicia (simulado)" "sistema \(teste\): logind Reboot" "400 click 25 14; 430 click 100 177; 460 click 753 412; 500 list"
CLOG="$(timeout 150 "$SCEN" "400 click 25 14; 430 click 100 177; 460 click 614 412; 500 list" 2>&1)"
if echo "$CLOG" | grep -q "menu aberto: ConfirmRestart" && ! echo "$CLOG" | grep -q "logind Reboot"; then
    echo "ok    Cancelar não reinicia"; PASS=$((PASS + 1))
else
    echo "FALHA Cancelar não reinicia"; FAIL=$((FAIL + 1))
fi
check "Wi-Fi: interruptor desliga (simulado)" "sistema \(teste\): nmcli radio wifi off" "400 click 1221 14; 440 click 1306 61; 470 list"
check "som: trilho muda o volume (simulado)" "sistema \(teste\): wpctl set-volume @DEFAULT_AUDIO_SINK@ 0\.[0-9]+" "400 click 1185 14; 440 click 1197 96; 470 list"
check "Bluetooth: clique desconecta o fone (simulado)" "sistema \(teste\): bluetoothctl disconnect" "400 click 1257 14; 440 click 1216 99; 470 list"
check "Sobre minimizada aparece na dock e volta" "janela restaurada" "400 click 25 14; 430 click 100 49; 520 click 83 61; 600 click 726 728; 700 list"
check "Sobre este computador abre" 'janela "Sobre este computador"' "400 click 25 14; 430 click 100 49; 520 list"

echo "== launcher (ctrl+shift+space)"
# Grade em 1366x768: ícones de 90 px, células de 144x149, 8 colunas a partir de
# x=107, 3 linhas a partir de y=160; o primeiro item fica em (179, 210) e o segundo
# em (323, 210). Abaixo da grade (683, 650) é vazio.
LAUNCH="400 combo ctrl+shift+space"
check "atalho abre o launcher" 'camada "mide-launcher" em \(0, 0\) 1366x768' "$LAUNCH; 460 list"
check_count "Esc fecha" 'camada "mide-launcher"' 0 "$LAUNCH; 450 key esc; 480 list"
check_count "atalho de novo fecha" 'camada "mide-launcher"' 0 "$LAUNCH; 450 combo ctrl+shift+space; 480 list"
check_count "clique no vazio fecha" 'camada "mide-launcher"' 0 "$LAUNCH; 450 click 683 650; 480 list"
check "buscar e Enter abre o app" "$CALC_OPEN" "$LAUNCH; 440 type calc; 450 key enter; 600 list"
check "buscar e clicar abre o app" "$CALC_OPEN" "$LAUNCH; 440 type calc; 450 click 179 210; 600 list"

check "roda para baixo vai para a página da direita" "página 0 → 1" "$LAUNCH; 450 move 683 384; 460 wheel down; 500 list"
check "roda para cima volta para a página da esquerda" "página 1 → 0" "$LAUNCH; 450 move 683 384; 460 wheel down; 500 wheel up; 540 list"
check "clique do meio vai para a próxima página" "página 0 → 1" "$LAUNCH; 450 mclick 683 384; 490 list"
# Pasta Utilitários (2 páginas) na página 2 da grade, em (346, 397).
# A pasta (Utilitários) muda de lugar conforme os apps instalados: o launcher
# registra onde ela fica na página 2, e o teste clica lá.
UPOS="$(timeout 150 "$SCEN" "$LAUNCH; 440 key pagedown; 480 list" 2>&1 | grep -oE "pasta Utilitários na página 1 em \([0-9]+, [0-9]+\)" | head -1 | grep -oE "[0-9]+, [0-9]+" | tr -d ',')"
check "roda dentro da pasta troca de página (carrossel)" "página da pasta 0 → 1" "$LAUNCH; 440 key pagedown; 470 click ${UPOS:-346 397}; 500 move 683 384; 510 wheel down; 550 list"
check "Configurações aparecem no launcher" 'janela "Configurações do Sistema"' "$LAUNCH; 440 type configuracoes_do_sistema; 450 key enter; 560 list"
check_count "clicar num app da dock fecha o launcher" 'camada "mide-launcher"' 0 "$LAUNCH; 460 click 654 728; 520 list"

echo "== launcher: arrastar, pastas e renomear (configuração própria)"
LCFG="$(mktemp -d)"
OLD_LCFG="$XDG_CONFIG_HOME"
export XDG_CONFIG_HOME="$LCFG"
# As pastas padrão dependem dos apps instalados: conta as do começo.
LFOLDERS="$(timeout 150 "$SCEN" "$LAUNCH; 460 list" 2>&1 | grep -oE "e [0-9]+ pasta" | head -1 | grep -oE "[0-9]+")"
DRAG="460 press 179 210; 465 move 200 210; 470 move 260 210; 475 move 323 210; 480 release 323 210"
check "arrastar app sobre app cria uma pasta" "pasta .*\(2 apps\)" "$LAUNCH; $DRAG; 520 list"
check "pasta fica salva" "em [0-9]+ lugar\(es\) e $((LFOLDERS + 1)) pasta" "$LAUNCH; 460 list"
check "clicar no nome da pasta e renomear" "pasta renomeada para gravar" "$LAUNCH; 460 click 179 210; 490 click 683 258; 510 key backspace; 511 key backspace; 512 key backspace; 513 key backspace; 514 key backspace; 520 type gravar; 530 key enter; 560 list"
check "arrastar para fora tira o app da pasta" "saiu da pasta gravar" "$LAUNCH; 460 click 179 210; 490 press 611 368; 495 move 560 420; 500 move 400 520; 505 move 200 600; 510 release 150 620; 540 list"
export XDG_CONFIG_HOME="$OLD_LCFG"
rm -rf "$LCFG"

echo "== notificações (barramento D-Bus de teste)"
# O servidor do mide atende num barramento próprio (tests/scenario-dbus.sh). A
# camada fica em (1002, 28): banner em x=1012..1356; com botões, "Abrir" em (1238, 121);
# o × aparece com o mouse em cima, no canto em (1016, 42).
NOTE1="notify-send -a Calculadora 'Download concluído' 'Arquivo pronto'"
NOTE2="notify-send -a Terminal -A ok=Abrir -A no=Ignorar 'Atualizações' 'Há 12 atualizações'"
SCEN_PLAIN="$SCEN"
SCEN=tests/scenario-dbus.sh
check "notificação aparece abaixo da barra, à direita" 'camada "mide-notifications" em \(1002, 28\) 364x94' "250 list" "$NOTE1"
check "notificação some sozinha depois de 5 s" "notificação 1 expirou" "700 list" "$NOTE1"
# Sons (A22; nos testes, só registrados no log): notificação toca; com os efeitos
# sonoros desligados, não.
SONS="$(mktemp -d)"
mkdir -p "$SONS/mide"
timeout 150 "$SCEN" "250 list" -- "$NOTE1" >/dev/null 2>&1
if grep -aq "som (teste): message-new-instant" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log"; then
    echo "ok    som de notificação"; PASS=$((PASS + 1))
else
    echo "FALHA som de notificação"; FAIL=$((FAIL + 1))
fi
printf '[sounds]\nenabled = false\n' > "$SONS/mide/config.toml"
XDG_CONFIG_HOME="$SONS" timeout 150 "$SCEN" "250 list" -- "$NOTE1" >/dev/null 2>&1
if grep -aq "notificação 1: Download" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log" && ! grep -aq "som (teste)" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log"; then
    echo "ok    efeitos sonoros desligados: notificação sem som"; PASS=$((PASS + 1))
else
    echo "FALHA efeitos sonoros desligados: notificação sem som"; FAIL=$((FAIL + 1))
fi
rm -rf "$SONS"
check "× fecha a notificação" "notificação 1 fechada" "250 move 1166 80; 260 click 1016 42; 300 list" "$NOTE1"
check "botão de ação aciona o app" "ação ok da notificação 1" "250 click 1238 121; 300 list" "$NOTE2"
ACAO="$(mktemp)"
timeout 150 "$SCEN" "250 click 1238 121; 300 list" -- "$NOTE2 > $ACAO" >/dev/null 2>&1
if [ "$(cat "$ACAO")" = "ok" ]; then
    echo "ok    app recebe a ação pelo D-Bus"; PASS=$((PASS + 1))
else
    echo "FALHA app recebe a ação pelo D-Bus (recebeu: '$(cat "$ACAO")')"; FAIL=$((FAIL + 1))
fi
rm -f "$ACAO"
# Central de notificações: histórico em $XDG_RUNTIME_DIR/mide-test-notificacoes.
HIST="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/mide-test-notificacoes"
rm -f "$HIST"
timeout 150 "$SCEN" "250 list" -- "$NOTE1" >/dev/null 2>&1
if grep -q "Download concluído" "$HIST" 2>/dev/null; then
    echo "ok    notificação vai para o histórico"; PASS=$((PASS + 1))
else
    echo "FALHA notificação vai para o histórico"; FAIL=$((FAIL + 1))
fi
# Painel do relógio (x 1078..1358): "Limpar" da central em (1321, 364).
timeout 150 "$SCEN" "300 click 1331 14; 340 click 1321 364; 380 list" -- "$NOTE1" >/dev/null 2>&1
if [ ! -s "$HIST" ]; then
    echo "ok    Limpar apaga o histórico de notificações"; PASS=$((PASS + 1))
else
    echo "FALHA Limpar apaga o histórico de notificações"; FAIL=$((FAIL + 1))
fi
# Não perturbe: o banner não aparece, mas a notificação fica no histórico.
NCFG="$(mktemp -d)"
mkdir -p "$NCFG/mide"
printf '[notifications]\ndnd = true\n' >"$NCFG/mide/config.toml"
rm -f "$HIST"
NLOG="$(XDG_CONFIG_HOME="$NCFG" timeout 150 "$SCEN" "250 list" -- "$NOTE1" 2>&1)"
if ! echo "$NLOG" | grep -q 'camada "mide-notifications"' && grep -q "Download concluído" "$HIST" 2>/dev/null; then
    echo "ok    Não perturbe silencia o banner e guarda no histórico"; PASS=$((PASS + 1))
else
    echo "FALHA Não perturbe silencia o banner e guarda no histórico"; FAIL=$((FAIL + 1))
fi
rm -rf "$NCFG" "$HIST"
SCEN="$SCEN_PLAIN"

echo "== janela com barra do mide (Terminal)"
check "maximizar pelo verde (abaixo da barra)" "$FOOT posição=\(0, 58\) visível=1366x643" "400 click 103 61; 450 list" "$TERMINAL"
check "restaurar pelo verde" "$FOOT posição=\(48, 76\) visível=700x500" "400 click 103 61; 450 click 55 43; 500 list" "$TERMINAL"
check "duplo clique maximiza" "$FOOT posição=\(0, 58\)" "400 click 400 61; 404 click 400 61; 450 list" "$TERMINAL"
check "arrastar pela barra" "$FOOT posição=\(148, 176\)" "400 press 400 61; 405 move 450 111; 410 move 500 161; 415 release 500 161; 430 list" "$TERMINAL"
check "redimensionar pelo canto" "$FOOT posição=\(48, 76\) visível=650x450" "400 press 745 573; 405 move 720 548; 410 move 695 523; 415 release 695 523; 450 list" "$TERMINAL"
check "redimensionar pela esquerda" "$FOOT posição=\(88, 76\) visível=660x500" "400 press 50 330; 405 move 70 330; 410 move 90 330; 415 release 90 330; 450 list" "$TERMINAL"
check "fechar pelo vermelho" "janela fechada" "400 click 63 61" "$TERMINAL"
# O app fecha sozinho (aqui, o processo termina): a foto dele esmaece mesmo assim (A20).
SCENARIO_WAIT=12 check "app que fecha sozinho também sai com transição" "janela fechando com transição" "900 list" "sh -c '$CALCULATOR & sleep 4; kill \$!'"
check "minimizar pelo amarelo" "janela minimizada" "400 click 83 61" "$TERMINAL"
check "restaurar minimizada" "janela restaurada" "400 click 83 61; 440 restore" "$TERMINAL"

# Cabeçalho desenhado pelo próprio app (GTK): só com a gnome-calculator instalada.
if command -v gnome-calculator >/dev/null; then
    echo "== janela com cabeçalho do app (calculadora)"
    check "fechar pelo X do app" "janela fechada" "400 click 384 99" gnome-calculator
    check "arrastar pelo cabeçalho do app" "$CALC popups=0 posição=\(108, 116\)" "400 press 200 98; 405 move 230 118; 410 move 260 138; 415 release 260 138; 450 list" gnome-calculator
    check "maximizar com duplo clique no cabeçalho do app" "$CALC popups=0 posição=\(0, 28\) visível=1366x673" "400 click 300 99; 405 click 300 99; 460 list" gnome-calculator
    check "abrir menu do app" "$CALC popups=1" "400 click 345 99; 440 list" gnome-calculator
    check "fechar menu clicando fora" "$CALC popups=0" "400 click 345 99; 430 click 700 400; 460 list" gnome-calculator
    check "redimensionar pela borda" "$CALC .*visível=400x509" "400 press 405 330; 405 move 425 330; 410 move 445 330; 415 release 445 330; 460 list" gnome-calculator
else
    echo "pulo  janela com cabeçalho do app (gnome-calculator ausente)"
fi

echo "== várias janelas"
check "cascata (segunda em 80,108)" "$CALC popups=0 posição=\(80, 108\)" "400 list" "$TERMINAL" "$CALCULATOR"
check_last "clique traz para frente (topo = terminal)" "janela \"Terminal\"" "400 click 700 300; 440 list" "$TERMINAL" "$CALCULATOR"

echo "== atalhos de janela (como no GNOME)"
# Dois Terminais: a de cima (80, 108) tem o foco. Alt+Tab escolhe a outra ao soltar o Alt.
check "Alt+Tab foca a outra janela" "alternador: janela [0-9]+ escolhida" "500 combo alt+tab; 560 list" "$TERMINAL" "$TERMINAL"
check "Super+← põe a janela na metade esquerda" "$FOOT posição=\(0, 58\) visível=683x643" "500 combo super+left; 560 list" "$TERMINAL"
check "Super+→ põe a janela na metade direita" "$FOOT posição=\(683, 58\) visível=683x643" "500 combo super+right; 560 list" "$TERMINAL"
check "Super+↑ maximiza" "$FOOT posição=\(0, 58\) visível=1366x643" "500 combo super+up; 560 list" "$TERMINAL"
check "Super+↓ restaura" "$FOOT posição=\(48, 76\) visível=700x500" "500 combo super+up; 560 combo super+down; 640 list" "$TERMINAL"
check "Super+H minimiza" "janela minimizada" "500 combo super+h; 560 list" "$TERMINAL"
# Teclas multimídia (A23): do mide, com qualquer app em foco (nos testes, os comandos
# só vão para o log).
check "tecla de volume aumenta 5%" "sistema \\(teste\\): wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%\\+" "500 key volumeup; 540 list" "$TERMINAL"
check "tecla de mudo" "sistema \\(teste\\): wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle" "500 key mute; 540 list" "$TERMINAL"
check "tecla de brilho" "sistema \\(teste\\): brightnessctl -c backlight -n 1% set 5%-" "500 key brightnessdown; 540 list" "$TERMINAL"
check "tecla tocar/pausar vai ao player (MPRIS)" "sistema \\(teste\\): mpris PlayPause" "500 key playpause; 540 list" "$TERMINAL"
check "Alt+F4 fecha a janela" "fechar janela pedido pelo atalho" "500 combo alt+f4; 560 list" "$TERMINAL"

echo "== saída redimensionada (janela do mide maximizada no host)"
check "dock volta ao centro inferior" 'camada "mide-dock" em \(458, 735\) 285x57' "400 resize 1200 800; 420 list" "$TERMINAL"
check "barra acompanha a largura" 'camada "mide-bar" em \(0, 0\) 1200x28' "400 resize 1200 800; 420 list" "$TERMINAL"
check "maximizada acompanha a nova área" "$FOOT posição=\(0, 58\) visível=1200x675" "400 click 103 61; 440 resize 1200 800; 460 list" "$TERMINAL"

echo "== dock"
check "clique no ícone abre o app" "nova janela criada" "400 click 611 728"
# App em foco: o clique minimiza (como o "minimize or previews" do dash-to-dock); outro
# clique traz de volta, sem abrir outra janela.
check "clique no ícone do app em foco minimiza" "em foco: minimizado pela dock" "400 click 611 728; 480 list" "$TERMINAL"
check_count "clique de novo restaura, sem abrir outro" "janela \"Terminal\"" 1 "400 click 611 728; 460 click 611 728; 540 list" "$TERMINAL"
check "app em foco com duas janelas: o clique mostra as prévias" "em foco com 2 janelas: prévias" "400 click 611 728; 480 list" "$TERMINAL" "$TERMINAL"
# A instância nova (aberta pela dock, sem --title) tem o nome do shell no título.
check_count "ctrl+clique abre nova instância" 'janela "[^"]+"' 2 "390 ctrl on; 400 click 611 728; 480 list" "$TERMINAL"
check "clique no ícone restaura minimizada" "janela restaurada" "400 combo super+h; 440 click 654 728" "$CALCULATOR"
check "Launchpad na dock abre o launcher" 'camada "mide-launcher"' "400 click 568 728; 470 list"
check "abrir pela dock sai do ícone (lâmpada)" "abrindo pela dock: mide-calculator" "400 click 654 728; 700 list"
check "Configurações fixadas na dock abrem" 'janela "Configurações do Sistema"' "400 click 697 728; 560 list"
check "menu do app: Ocultar" "janela minimizada" "400 rclick 611 728; 440 click 611 644" "$TERMINAL"
check "menu do app: Encerrar" "janela fechada" "400 rclick 611 728; 440 click 611 670" "$TERMINAL"
check "menu do app lista as janelas e foca a escolhida" "controle: janela [0-9]+ focada" "400 rclick 611 728; 440 click 611 513; 480 list" "$TERMINAL" "$TERMINAL"
check "prévias ao parar o mouse no ícone" "prévias: 2 janela" "400 move 623 728; 405 move 611 728; 470 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
check "clique na prévia escolhe a janela" "prévia: janela [0-9]+ escolhida" "400 move 623 728; 405 move 611 728; 470 move 520 598; 480 click 503 588; 520 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
# Com o Launchpad aberto, escolher uma janela nas prévias fecha o launcher.
check_count "prévia com o launcher aberto: escolher fecha o launcher" 'camada "mide-launcher"' 0 "400 combo ctrl+shift+space; 460 move 623 728; 465 move 611 728; 530 move 520 598; 540 click 503 588; 600 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
check "× da prévia fecha a janela" "prévia: fechar janela" "400 move 623 728; 405 move 611 728; 470 move 520 598; 475 move 600 533; 480 click 600 533; 540 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
check "clique direito abre o menu do app" "menu de Terminal aberto" "400 rclick 611 728; 470 list" "$TERMINAL"
check "clique no ícone durante a animação de minimizar restaura" "janela restaurada" "400 combo super+h; 420 click 654 728" "$CALCULATOR"

# Tema de ícones do mide (fork do WhiteSur): instalado numa pasta de dados de teste a
# partir do clone (o do mide-atualizar ou o do lado deste projeto); sem clone, pulado.
ICONSRC=""
for d in "${XDG_CACHE_HOME:-$HOME/.cache}/mide/mide-icons" "$PWD/../mide-icons"; do
    if [ -x "$d/install.sh" ]; then ICONSRC="$d"; break; fi
done
if [ -n "$ICONSRC" ]; then
    IDATA="$(mktemp -d)"
    (cd "$ICONSRC" && ./install.sh -n mide -d "$IDATA/icons" >/dev/null 2>&1)
    XDG_DATA_HOME="$IDATA" RUST_LOG="mide=debug,mide_dock=info" check "dock: ícones dos apps do mide vêm do tema do mide" "ícone mide-terminal: tema do mide" "400 list"
    rm -rf "$IDATA"
else
    echo "pulado: dock com o tema do mide (sem o clone do mide-icons)"
fi

echo "== dock: Downloads e Lixeira (pastas temporárias)"
# Downloads de teste com 3 itens (o mais novo é "projeto") e uma Lixeira de teste.
# Nada aqui toca as pastas reais: o abrir só vai para o log (MIDE_OPEN_DRY_RUN).
PLACES="$(mktemp -d)"
mkdir -p "$PLACES/dl/projeto"
echo x > "$PLACES/dl/antigo.txt"
echo x > "$PLACES/dl/foto.jpg"
touch -d "3 hours ago" "$PLACES/dl/antigo.txt"
touch -d "1 hour ago" "$PLACES/dl/foto.jpg"
printf 'XDG_DOWNLOAD_DIR="%s"\n' "$PLACES/dl" > "$TESTCFG/user-dirs.dirs"
fill_trash() {
    rm -rf "$PLACES/data" "$PLACES/orig"
    mkdir -p "$PLACES/data/Trash/files" "$PLACES/data/Trash/info" "$PLACES/orig"
    for f in a.txt b.txt; do
        echo x > "$PLACES/data/Trash/files/$f"
        printf '[Trash Info]\nPath=%s\nDeletionDate=2026-01-01T10:00:00\n' "$PLACES/orig/$f" > "$PLACES/data/Trash/info/$f.trashinfo"
    done
}
# Painel de Downloads (320x177) acima do ícone: linhas em y=563 (projeto) e 597
# (foto.jpg). O menu de um arquivo abre por cima da lista: clicando em (756, 563),
# "Copiar caminho" fica em (821, 662); em (756, 597), "Mover para a Lixeira" fica
# em (821, 731). Menu do ícone da Lixeira: "Esvaziar Lixeira…" em (799, 670).
# Confirmação: "Esvaziar Lixeira" em (799, 644) e "Cancelar" em (799, 670). Lista da
# Lixeira com 2 itens: o primeiro (a.txt) em y=571; menu dele: "Restaurar" em
# (887, 618), "Apagar permanentemente" em (887, 653).
check "Downloads mostra os recentes" "pilha Downloads aberta: 3 item" "400 click 756 728; 470 list"
check "clique num download abre o item" "abrir \(teste\): .*/dl/projeto" "400 click 756 728; 440 click 756 563"
check "Abrir no Gerenciador de Arquivos" "abrir \(teste\): .*/dl$" "400 click 756 728; 440 click 756 670"
check_count "clique fora fecha o painel" 'camada "mide-dock-menu"' 0 "400 click 756 728; 440 click 100 200; 470 list"
check "menu do arquivo abre sobre a lista" "menu do arquivo projeto aberto" "400 click 756 728; 440 rclick 756 563; 470 list"
check_count "lista continua aberta com o menu do arquivo" 'camada "mide-dock-menu"' 1 "400 click 756 728; 440 rclick 756 563; 470 list"
check "menu de arquivo: Copiar caminho" "copiar \(teste\): .*/dl/projeto" "400 click 756 728; 440 rclick 756 563; 480 click 821 662"
check "menu de arquivo: Mover para a Lixeira" "movido para a lixeira: foto.jpg" "400 click 756 728; 440 rclick 756 597; 480 click 821 731"
# Depois de agir num item, a lista continua aberta (para agir em vários); Esc fecha.
check_count "lista de Downloads continua aberta depois de mover um item" 'camada "mide-dock-menu"' 1 "400 click 756 728; 440 rclick 756 597; 480 click 821 731; 540 list"
check_count "Esc fecha a lista de Downloads" 'camada "mide-dock-menu"' 0 "400 click 756 728; 440 key esc; 480 list"
# Copiar caminho chega de verdade a outro app: o editor de texto (aberto depois da
# cópia, para o ícone dele não mexer na dock antes dos cliques) cola
# com Ctrl+V e salva. O texto fica com o compositor.
CPY="$(mktemp)"
# Os testes de cima levaram itens para a Lixeira: com menos itens, a lista e o menu
# ficam em outro lugar. A pasta volta aos 3 itens ("projeto" o mais novo).
mkdir -p "$PLACES/dl/projeto"
echo x > "$PLACES/dl/antigo.txt"
echo x > "$PLACES/dl/foto.jpg"
touch -d "3 hours ago" "$PLACES/dl/antigo.txt"
touch -d "1 hour ago" "$PLACES/dl/foto.jpg"
touch "$PLACES/dl/projeto"
# O editor aparece por volta do quadro 900; sob carga, mais tarde (folga até o 1200).
# O roteiro conta quadros e o cenário espera por tempo: sob a carga da bateria saem
# menos quadros por segundo, então a espera e o limite vão com sobra.
SCENARIO_WAIT=45 SCENARIO_TIMEOUT=75 timeout 150 "$SCEN" "400 click 756 728; 440 rclick 756 563; 480 click 821 662; 1200 click 400 300; 1220 combo ctrl+v; 1250 combo ctrl+s; 1300 list" -- "sleep 12; $PWD/target/debug/mide-text '$CPY'" >/dev/null 2>&1
if grep -q "/dl/projeto" "$CPY"; then
    echo "ok    Copiar caminho da dock cola em outro app"; PASS=$((PASS + 1))
else
    echo "FALHA Copiar caminho da dock cola em outro app (colado: '$(cat "$CPY")')"; FAIL=$((FAIL + 1))
fi
rm -f "$CPY"
OLD_DATA="${XDG_DATA_HOME-}"
export XDG_DATA_HOME="$PLACES/data"
fill_trash
check "clique na Lixeira mostra os itens apagados" "lista da Lixeira aberta: 2 item" "400 click 799 728; 440 list"
check "item da Lixeira: Restaurar" "restaurado da lixeira: .*/orig/a.txt" "400 click 799 728; 440 rclick 799 571; 480 click 887 618; 520 list"
fill_trash
check_count "lista da Lixeira continua aberta depois de restaurar" 'camada "mide-dock-menu"' 1 "400 click 799 728; 440 rclick 799 571; 480 click 887 618; 540 list"
fill_trash
check "item da Lixeira: Apagar permanentemente" "apagado permanentemente: a.txt" "400 click 799 728; 440 rclick 799 571; 480 click 887 653; 520 list"
fill_trash
check "Lixeira: esvaziar com confirmação" "lixeira esvaziada: 2 item" "400 rclick 799 728; 440 click 799 670; 480 click 799 644; 520 list"
if [ -z "$(ls -A "$PLACES/data/Trash/files")" ]; then
    echo "ok    Lixeira: arquivos apagados de verdade"; PASS=$((PASS + 1))
else
    echo "FALHA Lixeira: arquivos continuam lá"; FAIL=$((FAIL + 1))
fi
fill_trash
timeout 150 "$SCEN" "400 rclick 799 728; 440 click 799 670; 480 click 799 670; 520 list" -- >/dev/null 2>&1
if [ "$(ls "$PLACES/data/Trash/files" | wc -l)" = 2 ]; then
    echo "ok    Lixeira: cancelar não apaga"; PASS=$((PASS + 1))
else
    echo "FALHA Lixeira: cancelar apagou arquivos"; FAIL=$((FAIL + 1))
fi
if [ -n "$OLD_DATA" ]; then export XDG_DATA_HOME="$OLD_DATA"; else unset XDG_DATA_HOME; fi
rm -f "$TESTCFG/user-dirs.dirs"
rm -rf "$PLACES"

echo "== dock: fixar, desafixar e reordenar (configuração temporária)"
DOCKCFG="$(mktemp -d)"
mkdir -p "$DOCKCFG/mide"
dock_pins() { printf 'pinned = [%s]\n' "$1" > "$DOCKCFG/mide/dock.toml"; }
FOOT_ID='"mide-terminal"'
CALC_ID='"mide-calculator"'
# Atenção: "VAR=x funcao" não exporta VAR para os programas que a função chama.
# Por isso a variável é exportada só nesta seção e restaurada no fim.
OLD_XDG_CONFIG_HOME="${XDG_CONFIG_HOME-}"
export XDG_CONFIG_HOME="$DOCKCFG"
dock_pins "$FOOT_ID"
check "app aberto não fixado entra na dock" 'camada "mide-dock" em \(554, 703\) 258x57' "500 list" "$CALCULATOR"
dock_pins "$FOOT_ID"
check "menu do app: Manter na Dock" "dock salva: mide-terminal, mide-calculator" "500 rclick 683 728; 540 click 683 613" "$CALCULATOR"
dock_pins "$FOOT_ID, $CALC_ID"
check "menu do app: Remover da Dock" "dock salva: mide-calculator$" "300 rclick 632 728; 340 click 632 674"
dock_pins "$FOOT_ID, $CALC_ID"
check "arrastar reordena" "dock salva: mide-calculator, mide-terminal" "300 press 632 728; 305 move 643 728; 310 move 658 728; 315 move 675 728; 320 release 675 728"
# Ícone pulando enquanto o app abre (A20): um app de teste que demora 2 s (num
# XDG_DATA_DIRS próprio), fixado sozinho; o ícone dele fica em (654, 728).
PULO="$DOCKCFG/apps"
mkdir -p "$PULO/applications"
printf '[Desktop Entry]\nType=Application\nName=Lento\nExec=sh -c "sleep 2; exec %s/target/debug/mide-calculator"\nIcon=folder\nStartupWMClass=mide-calculator\n' "$PWD" > "$PULO/applications/lento.desktop"
dock_pins '"lento"'
XDG_DATA_DIRS="$PULO:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}" SCENARIO_WAIT=12 timeout 150 "$SCEN" "300 click 654 728; 900 list" >/dev/null 2>&1
if grep -aq "Lento pulando até abrir" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log" && grep -aq "lento abriu, fim do pulo" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log"; then
    echo "ok    dock: o ícone pula até o app abrir"; PASS=$((PASS + 1))
else
    echo "FALHA dock: o ícone pula até o app abrir"; FAIL=$((FAIL + 1))
fi
if [ -n "$OLD_XDG_CONFIG_HOME" ]; then export XDG_CONFIG_HOME="$OLD_XDG_CONFIG_HOME"; else unset XDG_CONFIG_HOME; fi
rm -rf "$DOCKCFG"

echo "== Configurações do Sistema (configuração própria)"
check "menu do mide abre as Configurações" "sistema \(teste\): mide-settings" "400 click 25 14; 430 click 100 88; 460 list"
SCFG="$(mktemp -d)"
mkdir -p "$SCFG/mide"
printf '# Meu config\n[keybindings]\n"ctrl+shift+t" = "launch:foot" # terminal\n' > "$SCFG/mide/config.toml"
OLD_SCFG="$XDG_CONFIG_HOME"
export XDG_CONFIG_HOME="$SCFG"
SETTINGS="$PWD/target/debug/mide-settings"
# Papéis de parede do sistema de teste (os da máquina mudam de uma distribuição para
# outra): só "Beach". Janela em (48, 76): miniatura "Beach" em (548, 185); interruptor
# de Sombras em (807, 169).
export MIDE_BACKGROUNDS_DIR="$SCFG/backgrounds"
mkdir -p "$MIDE_BACKGROUNDS_DIR"
python3 -c 'import sys; from PIL import Image; Image.new("RGB", (640, 400), (40, 120, 200)).save(sys.argv[1])' "$MIDE_BACKGROUNDS_DIR/Beach.jpg"
timeout 150 "$SCEN" "400 click 548 185; 460 list" -- "$SETTINGS wallpaper" >/dev/null 2>&1
if grep -q "wallpaper = \"$MIDE_BACKGROUNDS_DIR/Beach" "$SCFG/mide/config.toml" && grep -q "# terminal" "$SCFG/mide/config.toml"; then
    echo "ok    escolher papel de parede grava (e mantém os comentários)"; PASS=$((PASS + 1))
else
    echo "FALHA escolher papel de parede grava"; FAIL=$((FAIL + 1))
fi
check "compositor usa o papel de parede gravado" "papel de parede: $MIDE_BACKGROUNDS_DIR/Beach" "400 list; 420 list" "$SETTINGS wallpaper"
timeout 150 "$SCEN" "400 click 807 169; 460 list" -- "$SETTINGS effects" >/dev/null 2>&1
if grep -q "shadows = false" "$SCFG/mide/config.toml"; then
    echo "ok    desligar sombras grava no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA desligar sombras grava no config.toml"; FAIL=$((FAIL + 1))
fi
# Efeitos: "Desfoque na dock" em (807, 393) e "Desfoque no launcher" em (807, 449).
timeout 150 "$SCEN" "400 click 807 393; 440 click 807 449; 480 list" -- "$SETTINGS effects" >/dev/null 2>&1
if grep -q "dock_blur = false" "$SCFG/mide/config.toml" && grep -q "launcher_blur = false" "$SCFG/mide/config.toml"; then
    echo "ok    interruptores de desfoque gravam no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA interruptores de desfoque gravam no config.toml"; FAIL=$((FAIL + 1))
fi
# O teste anterior desligou o desfoque da dock; o vidro líquido depende dele.
sed -i '/dock_blur = false/d' "$SCFG/mide/config.toml"
# Aparência: a lista em (789, 162) abre com apertar e soltar; "Escuro" em (680, 227).
# Grava no config.toml e a janela fica escura na hora (fundo em (600, 400)).
timeout 150 "$SCEN" "300 move 789 162; 310 press 789 162; 320 release 789 162; 360 click 680 227; 440 list" 430 -- "$SETTINGS appearance" >/dev/null 2>&1
if grep -q 'theme = "dark"' "$SCFG/mide/config.toml" && [ "$(python3 -c "import sys; from PIL import Image; print('%d,%d,%d' % Image.open(sys.argv[1]).convert('RGB').getpixel((600, 400)))" "${SCENARIO_OUT:-/tmp/mide-scenario}/frame.png" 2>/dev/null)" = "30,30,32" ]; then
    echo "ok    Aparência: tema escuro grava e a janela muda na hora"; PASS=$((PASS + 1))
else
    echo "FALHA Aparência: tema escuro"; FAIL=$((FAIL + 1))
fi
sed -i '/^\[appearance\]/d; /^theme = "dark"/d' "$SCFG/mide/config.toml"
# Teclado → "Adicionar layout" em (778, 204) (apertar e soltar); "Inglês (EUA)" em
# (605, 268). A lista fica com o do sistema e o novo, e o compositor monta os dois.
# (A linha do layout não sai no resumo do cenário: confere no mide.log.)
timeout 150 "$SCEN" "300 move 778 204; 310 press 778 204; 320 release 778 204; 370 click 605 268; 420 list; 480 list" -- "$SETTINGS keyboard" >/dev/null 2>&1
if grep -aq "teclado: layout br,us" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log"; then
    echo "ok    Teclado: adicionar layout vale na hora"; PASS=$((PASS + 1))
else
    echo "FALHA Teclado: adicionar layout vale na hora"; FAIL=$((FAIL + 1))
fi
if grep -q 'layouts = "br,us"' "$SCFG/mide/config.toml"; then
    echo "ok    Teclado: layouts gravados no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA Teclado: layouts gravados no config.toml"; FAIL=$((FAIL + 1))
fi
# Com dois layouts, Ctrl+Super+Espaço passa ao dos EUA: a tecla do acento do ABNT2
# escreve "[".
: > "$SCFG/layout.txt"
timeout 150 "$SCEN" "400 click 400 300; 420 combo ctrl+super+space; 440 key leftbrace; 450 key a; 480 combo ctrl+s; 540 list" -- "$PWD/target/debug/mide-text '$SCFG/layout.txt'" >/dev/null 2>&1
if [ "$(cat "$SCFG/layout.txt")" = "[a" ]; then
    echo "ok    Teclado: Ctrl+Super+Espaço troca o layout"; PASS=$((PASS + 1))
else
    echo "FALHA Teclado: Ctrl+Super+Espaço troca o layout ($(cat "$SCFG/layout.txt"))"; FAIL=$((FAIL + 1))
fi
# Seletor na barra (B18): "BR" em (1256, 14); no menu, "Inglês (EUA)" em (1190, 78).
: > "$SCFG/layout.txt"
timeout 150 "$SCEN" "400 click 1256 14; 440 click 1190 78; 480 click 400 300; 500 key leftbrace; 510 key a; 540 combo ctrl+s; 600 list" -- "$PWD/target/debug/mide-text '$SCFG/layout.txt'" >/dev/null 2>&1
if [ "$(cat "$SCFG/layout.txt")" = "[a" ]; then
    echo "ok    barra: seletor de layout troca o layout"; PASS=$((PASS + 1))
else
    echo "FALHA barra: seletor de layout troca o layout ($(cat "$SCFG/layout.txt"))"; FAIL=$((FAIL + 1))
fi
sed -i '/^layouts = "br,us"/d' "$SCFG/mide/config.toml"
# Grupos de atalhos recolhem (B19): o título "Janelas" em (330, 347); fica gravado.
timeout 150 "$SCEN" "300 click 330 347; 360 list" -- "$SETTINGS keyboard" >/dev/null 2>&1
if grep -q 'shortcuts_closed = "Janelas"' "$SCFG/mide/config.toml"; then
    echo "ok    Teclado: grupo de atalhos recolhe (e fica lembrado)"; PASS=$((PASS + 1))
else
    echo "FALHA Teclado: grupo de atalhos recolhe"; FAIL=$((FAIL + 1))
fi
sed -i '/^shortcuts_closed = /d' "$SCFG/mide/config.toml"
# Energia: "Suspender o computador após" em (783, 162) (apertar e soltar); "30 minutos"
# é o terceiro item (y=253).
timeout 150 "$SCEN" "300 move 783 162; 310 press 783 162; 320 release 783 162; 370 click 700 253; 420 list" -- "$SETTINGS energy" >/dev/null 2>&1
if grep -q 'suspend_minutes = 30' "$SCFG/mide/config.toml"; then
    echo "ok    Energia: suspender após 30 minutos grava"; PASS=$((PASS + 1))
else
    echo "FALHA Energia: suspender após 30 minutos grava"; FAIL=$((FAIL + 1))
fi
sed -i '/^suspend_minutes = 30/d' "$SCFG/mide/config.toml"
# Som → "Som de alerta" em (794, 410) (apertar e soltar); "Gota" em (675, 474).
timeout 150 "$SCEN" "300 move 794 410; 310 press 794 410; 320 release 794 410; 360 click 675 474; 420 list" -- "$SETTINGS sound" >/dev/null 2>&1
if grep -q 'alert = "gota"' "$SCFG/mide/config.toml"; then
    echo "ok    Som: escolher o som de alerta grava"; PASS=$((PASS + 1))
else
    echo "FALHA Som: escolher o som de alerta grava"; FAIL=$((FAIL + 1))
fi
sed -i '/^alert = "gota"/d' "$SCFG/mide/config.toml"
# Idioma e região: a lista em (768, 162); "Português (Brasil)" é o 12º item (y=460)
# quando os idiomas da lista estão instalados (Debian com locales-all).
if locale -a 2>/dev/null | grep -q '^pt_BR.utf8' && [ "$(locale -a | grep -cE '^(de_DE|en_GB|en_US|es_AR|es_ES|es_MX|fr_FR|it_IT|nl_NL|pt_BR|pt_PT|ru_RU|zh_CN|ja_JP)\.utf8')" = 14 ]; then
    timeout 150 "$SCEN" "300 move 768 162; 310 press 768 162; 320 release 768 162; 370 click 688 460; 420 list" -- "$SETTINGS region" >/dev/null 2>&1
    if grep -q 'lang = "pt_BR.UTF-8"' "$SCFG/mide/config.toml"; then
        echo "ok    Idioma e região: escolher o idioma grava"; PASS=$((PASS + 1))
    else
        echo "FALHA Idioma e região: escolher o idioma grava"; FAIL=$((FAIL + 1))
    fi
    sed -i '/^lang = "pt_BR.UTF-8"/d' "$SCFG/mide/config.toml"
else
    echo "pulo  Idioma e região (os idiomas da lista não estão todos instalados)"
fi
# Apps padrão: só os apps de teste (XDG_DATA_DIRS próprio); "Navegador" em (783, 162),
# com "Nenhum", "Navegador A" e "Navegador B" (y=253). Grava no mimeapps.list.
APPS_DIR="$SCFG/apps-teste"
mkdir -p "$APPS_DIR/applications"
for n in A B; do printf '[Desktop Entry]\nType=Application\nName=Navegador %s\nExec=true\nMimeType=x-scheme-handler/http;text/html;\n' "$n" > "$APPS_DIR/applications/navegador-$(echo "$n" | tr AB ab).desktop"; done
timeout 150 "$SCEN" "300 move 783 162; 310 press 783 162; 320 release 783 162; 370 click 700 253; 420 list" -- "XDG_DATA_DIRS='$APPS_DIR' XDG_DATA_HOME='$APPS_DIR' $SETTINGS default-apps" >/dev/null 2>&1
if grep -q '^x-scheme-handler/http=navegador-b.desktop' "$SCFG/mimeapps.list" 2>/dev/null; then
    echo "ok    Apps padrão: escolher o navegador grava no mimeapps.list"; PASS=$((PASS + 1))
else
    echo "FALHA Apps padrão: escolher o navegador grava no mimeapps.list"; FAIL=$((FAIL + 1))
fi
rm -rf "$APPS_DIR" "$SCFG/mimeapps.list"
# "Vidro líquido" em (807, 505): grava no config.toml e a dock troca para o vidro clássico.
check "Efeitos → Vidro líquido desligado: a dock volta ao vidro clássico" "vidro líquido na dock: desligado" "400 click 807 505; 480 list" "$SETTINGS effects"
if grep -q "liquid_glass = false" "$SCFG/mide/config.toml"; then
    echo "ok    interruptor do vidro líquido grava no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA interruptor do vidro líquido grava no config.toml"; FAIL=$((FAIL + 1))
fi
# Seção Dock: arrastar o trilho até o fim grava 64 px e a dock cresce sozinha.
check "Configurações → Dock: tamanho dos ícones muda a dock" 'camada "mide-dock" em \([0-9]+, [0-9]+\) [0-9]+x81' "400 list; 420 press 500 161; 425 move 600 161; 430 move 700 161; 435 move 790 161; 440 release 790 161; 520 list" "$SETTINGS dock"
# Dock: "Ocultar e mostrar a Dock automaticamente" em (807, 212); a dock desce e esconde.
# Ocultar é inteligente: só esconde com janela sobre a dock (aqui, maximizando a janela).
check "Configurações → Dock: ocultar automaticamente esconde a dock sob uma janela" "dock escondida" "400 click 807 212; 500 click 103 61; 800 list" "$SETTINGS dock"
if grep -q "autohide = true" "$SCFG/mide/config.toml"; then
    echo "ok    ocultar a dock grava no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA ocultar a dock grava no config.toml"; FAIL=$((FAIL + 1))
fi
sed -i '/autohide = true/d' "$SCFG/mide/config.toml"
# Barra superior (B12): "Configurações" (engrenagem) é o primeiro interruptor, em
# (807, 169). Escondido, some da barra na hora (a barra relê a cada meio segundo).
RUST_LOG="mide=debug,mide_bar=info" check "Configurações → Barra superior: esconder a engrenagem tira o item da barra" "itens da barra: sound wifi bluetooth battery control$" "400 click 807 169; 520 list" "$SETTINGS bar"
if grep -q "gear = false" "$SCFG/mide/config.toml"; then
    echo "ok    esconder item da barra grava no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA esconder item da barra grava no config.toml"; FAIL=$((FAIL + 1))
fi
sed -i '/gear = false/d' "$SCFG/mide/config.toml"
# Mouse e trackpad: "Rolagem natural no mouse" em (807, 266), "Toque para clicar" em
# (807, 416); o trilho da velocidade vai de x=488 a 778 em y=162 (meio = padrão).
timeout 150 "$SCEN" "400 click 807 266; 430 click 807 416; 460 press 633 162; 465 move 560 162; 470 move 500 162; 475 release 488 162; 520 list" -- "$SETTINGS mouse" >/dev/null 2>&1
if grep -q "mouse_natural_scroll = true" "$SCFG/mide/config.toml" && grep -q "tap_to_click = false" "$SCFG/mide/config.toml" && grep -q "pointer_speed = -1" "$SCFG/mide/config.toml"; then
    echo "ok    Mouse e trackpad gravam no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA Mouse e trackpad gravam no config.toml"; FAIL=$((FAIL + 1))
fi
python3 - "$SCFG/mide/config.toml" <<'PY'
import re, sys
p = sys.argv[1]
s = open(p).read()
s = re.sub(r"(?ms)^\[input\]\n(.*?)(?=^\[|\Z)", "", s)
open(p, "w").write(s)
PY
# Busca na barra lateral (campo em (158, 104)): "teclas" acha "Teclas no estilo Mac"; o
# resultado (primeira linha, y=158) abre a seção Teclado, com o interruptor em (807, 169).
timeout 150 "$SCEN" "400 click 158 104; 420 type teclas; 460 click 500 158; 500 click 807 253; 560 list" -- "$SETTINGS" >/dev/null 2>&1
if grep -q "mac_style = true" "$SCFG/mide/config.toml"; then
    echo "ok    busca nas Configurações leva à opção (teclas no estilo Mac gravado)"; PASS=$((PASS + 1))
else
    echo "FALHA busca nas Configurações leva à opção (teclas no estilo Mac gravado)"; FAIL=$((FAIL + 1))
fi
sed -i '/mac_style = true/d' "$SCFG/mide/config.toml"
# Teclado → Atalhos (A9): busca em (600, 224); buscando "fechar", "Fechar a janela"
# (Ctrl ⇧ Q) fica em (500, 340) e "Fechar a janela" (Alt F4) em (500, 298); no
# diálogo, "Gravar…" em (525, 315). O compositor grava a combinação apertada.
timeout 150 "$SCEN" "400 click 600 307; 410 type fechar; 440 click 500 434; 470 click 525 315; 500 combo ctrl+alt+k; 560 list" -- "$SETTINGS keyboard" >/dev/null 2>&1
if grep -q '"ctrl+alt+k" = "close_window"' "$SCFG/mide/config.toml" && grep -q '"ctrl+shift+q" = "none"' "$SCFG/mide/config.toml"; then
    echo "ok    gravar atalho nas Configurações troca a combinação"; PASS=$((PASS + 1))
else
    echo "FALHA gravar atalho nas Configurações troca a combinação"; FAIL=$((FAIL + 1))
fi
check "atalho gravado vale no compositor" "fechar janela pedido pelo atalho" "400 combo ctrl+alt+k; 460 list" "$SETTINGS keyboard"
check_count "combinação antiga desligada (a janela continua aberta)" 'janela "Configurações do Sistema"' 1 "400 combo ctrl+shift+q; 460 list" "$SETTINGS keyboard"
# Combinação já usada (Ctrl+Espaço é do buscador): pede confirmação, não grava nem abre o buscador.
check_count "gravando, o atalho não dispara" 'camada "mide-search"' 0 "400 click 600 307; 410 type fechar; 440 click 500 392; 470 click 525 315; 500 combo ctrl+space; 560 list" "$SETTINGS keyboard"
if ! grep -q '"ctrl+space" = "close_window"' "$SCFG/mide/config.toml"; then
    echo "ok    combinação de outra ação pede confirmação"; PASS=$((PASS + 1))
else
    echo "FALHA combinação de outra ação pede confirmação"; FAIL=$((FAIL + 1))
fi
# "Restaurar os padrões" (buscando algo sem resultado, a lista some) em (510, 292): a
# seção de atalhos sai do arquivo.
timeout 150 "$SCEN" "400 click 600 307; 410 type zzz; 440 click 510 375; 500 list" -- "$SETTINGS keyboard" >/dev/null 2>&1
if ! grep -q "keybindings" "$SCFG/mide/config.toml"; then
    echo "ok    restaurar os atalhos padrão"; PASS=$((PASS + 1))
else
    echo "FALHA restaurar os atalhos padrão"; FAIL=$((FAIL + 1))
fi
export XDG_CONFIG_HOME="$OLD_SCFG"
unset MIDE_BACKGROUNDS_DIR
rm -rf "$SCFG"

echo "== buscador (ctrl+space)"
SEARCH="400 combo ctrl+space"
check "ctrl+space abre o buscador em tela cheia" 'camada "mide-search" em \(0, 0\) 1366x768' "$SEARCH; 460 list"
check "buscador sobre vidro" 'vidro: desfoque refeito \([^:]*:mide-search#0\)' "$SEARCH; 440 type calc; 480 list"
check "buscar e Enter abre o app" "nova janela criada" "$SEARCH; 440 type calc; 470 key enter; 600 list"
check_count "Esc fecha o buscador" 'camada "mide-search"' 0 "$SEARCH; 450 key esc; 490 list"
check_count "o atalho de novo fecha o buscador" 'camada "mide-search"' 0 "$SEARCH; 450 combo ctrl+space; 490 list"
check_count "clique fora do painel fecha o buscador" 'camada "mide-search"' 0 "$SEARCH; 450 click 60 560; 490 list"
check_count "abrir um app pelo buscador fecha o buscador" 'camada "mide-search"' 0 "$SEARCH; 440 type calc; 470 key enter; 600 list"

MCFG="$(mktemp -d)"
mkdir -p "$MCFG/mide"
printf '[keyboard]\nmac_style = true\n' >"$MCFG/mide/config.toml"
# Estilo Mac: o Alt é o ⌘ e o Super é o ⌥; o Ctrl continua Ctrl.
if XDG_CONFIG_HOME="$MCFG" timeout 150 "$SCEN" "400 combo alt+space; 460 list" 2>&1 | grep -q 'camada "mide-search"'; then
    echo "ok    teclas no estilo Mac: Command+Espaço abre o buscador"; PASS=$((PASS + 1))
else
    echo "FALHA teclas no estilo Mac: Command+Espaço abre o buscador"; FAIL=$((FAIL + 1))
fi
XDG_CONFIG_HOME="$MCFG" SCENARIO_WAIT=16 check "teclas no estilo Mac: Ctrl+⌘+Q bloqueia" "autoteste: tela bloqueada" "430 combo ctrl+alt+q; 650 list" "$TERMINAL"
XDG_CONFIG_HOME="$MCFG" check_count "teclas no estilo Mac: Ctrl+L e ⌘L não bloqueiam (no Terminal)" "tela bloqueada" 0 "430 combo ctrl+l; 440 combo alt+l; 460 combo super+l; 600 list" "$TERMINAL"
XDG_CONFIG_HOME="$MCFG" check_count "teclas no estilo Mac: Ctrl+L e ⌘L não bloqueiam (num app)" "tela bloqueada" 0 "430 combo ctrl+l; 440 combo alt+l; 460 combo super+l; 600 list" "$CALCULATOR"
XDG_CONFIG_HOME="$MCFG" SCENARIO_WAIT=16 check "teclas no estilo Mac: Ctrl+⌘+Q bloqueia no Terminal" "autoteste: tela bloqueada" "430 combo ctrl+alt+q; 650 list" "$TERMINAL"
XDG_CONFIG_HOME="$MCFG" check "teclas no estilo Mac: ⌘Tab troca de janela" "alternador: janela" "400 keydown alt; 410 key tab; 440 list; 450 keyup alt" "$TERMINAL" "$CALCULATOR"
rm -rf "$MCFG"

HCFG="$(mktemp -d)"
mkdir -p "$HCFG/mide"
printf '[dock]\nautohide = true\n' >"$HCFG/mide/config.toml"
HLOG="$(XDG_CONFIG_HOME="$HCFG" timeout 150 "$SCEN" "500 list" 2>&1)"
if echo "$HLOG" | grep -q 'camada "mide-dock"' && ! echo "$HLOG" | grep -q "dock escondida"; then
    echo "ok    ocultar inteligente: sem janela por cima, a dock fica à mostra"; PASS=$((PASS + 1))
else
    echo "FALHA ocultar inteligente: sem janela por cima, a dock fica à mostra"; FAIL=$((FAIL + 1))
fi
# Em 1366x768 a janela nova não chega na dock: maximizada, ela cobre a dock.
if XDG_CONFIG_HOME="$HCFG" timeout 150 "$SCEN" "400 click 103 61; 700 list" -- "$TERMINAL" 2>&1 | grep -q "dock escondida"; then
    echo "ok    ocultar inteligente: janela sobre a dock a esconde"; PASS=$((PASS + 1))
else
    echo "FALHA ocultar inteligente: janela sobre a dock a esconde"; FAIL=$((FAIL + 1))
fi
rm -rf "$HCFG"

# Painel de terceiros (como a Waybar): camada com um popup adotado por ela (o menu de
# um ícone). O popup é configurado, fica embaixo do painel e aparece na tela (laranja).
LPP="$(mktemp -d)"
SCENARIO_OUT="$LPP/scen" SCENARIO_WAIT=8 timeout 150 "$SCEN" "400 list" 420 -- "$PWD/target/debug/examples/mide-layer-popup-teste > $LPP/saida.txt" >/dev/null 2>&1
if grep -q "popup configurado" "$LPP/saida.txt" 2>/dev/null && python3 -c "
import sys
from PIL import Image
sys.exit(0 if Image.open(sys.argv[1]).convert('RGB').getpixel((300, 300)) == (255, 138, 31) else 1)" "$LPP/scen/frame.png"; then
    echo "ok    layer-shell: popup de um painel de terceiros aparece embaixo dele"; PASS=$((PASS + 1))
else
    echo "FALHA layer-shell: popup de um painel de terceiros"; FAIL=$((FAIL + 1))
fi
rm -rf "$LPP"

# Tray (StatusNotifierItem), num barramento D-Bus de teste: o ícone do app aparece na
# barra (quadrado laranja em (1117, 14)), o clique ativa o app, o clique direito abre o
# menu dele e o item clicado chega ao app; quando o app sai, o ícone some.
if [ -x target/debug/examples/mide-tray-teste ]; then
    TRAY="$(mktemp -d)"
    tray_run() {
        SCENARIO_OUT="$TRAY/scen" SCENARIO_WAIT="${2:-10}" timeout 150 tests/scenario-dbus.sh "$1" "${3:-}" -- "TRAY_TESTE_ARQUIVO=$TRAY/acao.txt TRAY_TESTE_SEGUNDOS=${4:-60} $PWD/target/debug/examples/mide-tray-teste" >/dev/null 2>&1
    }
    pixel() { python3 -c "import sys; from PIL import Image; print('%d,%d,%d' % Image.open(sys.argv[1]).convert('RGB').getpixel((1117, 14)))" "$TRAY/scen/frame.png" 2>/dev/null; }
    tray_run "400 list" 10 420
    [ "$(pixel)" = "255,138,31" ] && { echo "ok    tray: o ícone do app aparece na barra"; PASS=$((PASS + 1)); } || { echo "FALHA tray: o ícone do app aparece na barra ($(pixel))"; FAIL=$((FAIL + 1)); }
    rm -f "$TRAY/acao.txt"; tray_run "400 click 1117 14; 450 list"
    [ "$(cat "$TRAY/acao.txt" 2>/dev/null)" = "ativado" ] && { echo "ok    tray: clique ativa o app"; PASS=$((PASS + 1)); } || { echo "FALHA tray: clique ativa o app"; FAIL=$((FAIL + 1)); }
    rm -f "$TRAY/acao.txt"; tray_run "400 rclick 1117 14; 440 click 1134 88; 480 list"
    [ "$(cat "$TRAY/acao.txt" 2>/dev/null)" = "clique 1" ] && { echo "ok    tray: menu do ícone (clique direito) e o item chega ao app"; PASS=$((PASS + 1)); } || { echo "FALHA tray: menu do ícone"; FAIL=$((FAIL + 1)); }
    tray_run "700 list" 12 690 3
    [ "$(pixel)" != "255,138,31" ] && { echo "ok    tray: o ícone some quando o app sai"; PASS=$((PASS + 1)); } || { echo "FALHA tray: o ícone some quando o app sai"; FAIL=$((FAIL + 1)); }
    rm -rf "$TRAY"
else
    echo "pulo  tray (rode cargo build --examples)"
fi

# Menu global de app GTK no Wayland (gtk-shell + org.gtk.Menus): "Arquivo" do app em
# (218, 14) na barra; "Abrir" em (215, 55) chega ao app. No barramento da sessão (o GTK
# não mostra janela no barramento de teste); o app tem id próprio e NON_UNIQUE.
if python3 -c "import gi; gi.require_version('Gtk', '3.0'); from gi.repository import Gtk" 2>/dev/null; then
    GTKM="$(mktemp -d)"
    SCENARIO_OUT="$GTKM/scen" SCENARIO_WAIT=12 timeout 100 tests/scenario.sh "450 click 218 14; 490 click 215 55; 530 list" -- "GTK_TESTE_ARQUIVO=$GTKM/acao.txt python3 $PWD/tests/apps/gtk-menu-teste.py" >/dev/null 2>&1
    if grep -q "menu do app gtk-menu-teste.py: Arquivo, Exibir, Ajuda" "$GTKM/scen/mide.log" && [ "$(cat "$GTKM/acao.txt" 2>/dev/null)" = "abrir" ]; then
        echo "ok    menu global: app GTK no Wayland (menus na barra, ação chega ao app)"; PASS=$((PASS + 1))
    else
        echo "FALHA menu global: app GTK no Wayland"; FAIL=$((FAIL + 1))
    fi
    rm -rf "$GTKM"
else
    echo "pulo  menu global GTK (sem python3-gi)"
fi

echo "== área de trabalho (ícones)"
# Pasta de teste com um arquivo: o ícone fica na coluna da direita, em (1298, 76)
# (área útil abaixo da barra e acima da dock). Fundo vazio em (400, 300): menu com
# "Nova pasta" em (450, 326).
DESK="$(mktemp -d)"
reset_desk() { rm -rf "$DESK"/* ; echo oi >"$DESK/notas.txt"; }
reset_desk
export MIDE_DESKTOP_DIR="$DESK"
check "área de trabalho aparece como camada de fundo" 'camada "mide-desktop"' "300 list"
check "duplo clique no ícone abre o arquivo" "abrir \(teste\): .*notas.txt" "300 click 1298 76; 306 click 1298 76; 340 list"
unset MIDE_DESKTOP_DIR
reset_desk
MIDE_DESKTOP_DIR="$DESK" timeout 150 "$SCEN" "300 click 1298 76; 330 key delete; 380 list" >/dev/null 2>&1
if [ ! -e "$DESK/notas.txt" ]; then
    echo "ok    Delete manda o ícone da área de trabalho para a Lixeira"; PASS=$((PASS + 1))
else
    echo "FALHA Delete manda o ícone da área de trabalho para a Lixeira"; FAIL=$((FAIL + 1))
fi
reset_desk
MIDE_DESKTOP_DIR="$DESK" timeout 150 "$SCEN" "300 rclick 400 300; 330 click 450 326; 370 key enter; 420 list" >/dev/null 2>&1
if [ -d "$DESK/Nova pasta" ]; then
    echo "ok    menu da área de trabalho → Nova pasta"; PASS=$((PASS + 1))
else
    echo "FALHA menu da área de trabalho → Nova pasta"; FAIL=$((FAIL + 1))
fi
rm -rf "$DESK"

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

echo "== editor de texto (mide-text)"
TDIR="$(mktemp -d)"
TEXT_APP="$PWD/target/debug/mide-text"
printf 'fn main() {\n    println!("olá");\n}\n' > "$TDIR/teste.rs"
# Abre com o foco no texto: Ctrl+End vai ao fim, Enter e digitar, Ctrl+S salva.
timeout 150 "$SCEN" "400 list; 420 combo ctrl+end; 440 key enter; 450 type editado; 470 combo ctrl+s; 520 list" -- "$TEXT_APP '$TDIR/teste.rs'" >/dev/null 2>&1
if [ "$(tail -n 1 "$TDIR/teste.rs")" = "editado" ] && grep -q 'println!("olá");' "$TDIR/teste.rs"; then
    echo "ok    editor: editar no fim e salvar com Ctrl+S"; PASS=$((PASS + 1))
else
    echo "FALHA editor: editar no fim e salvar com Ctrl+S"; FAIL=$((FAIL + 1))
fi
# Copiar num editor e colar em outro (área de transferência do Wayland). O primeiro
# fica em (48, 76) e o segundo em (80, 108), por cima: barra do primeiro em (300, 61),
# texto do segundo visível em (925, 400).
printf 'copiar isto' > "$TDIR/a.txt"; : > "$TDIR/b.txt"
# O segundo abre ~3 s depois do primeiro: os passos começam no quadro 2000 (o
# autoteste conta quadros, e sem GPU eles passam rápido).
SCENARIO_WAIT=40 timeout 150 "$SCEN" "2000 list; 2020 click 300 61; 2040 combo ctrl+a; 2050 combo ctrl+c; 2070 click 925 400; 2090 combo ctrl+v; 2120 combo ctrl+s; 2180 list" -- "$TEXT_APP '$TDIR/a.txt'" "sleep 1; $TEXT_APP '$TDIR/b.txt'" >/dev/null 2>&1
if [ "$(cat "$TDIR/b.txt")" = "copiar isto" ]; then
    echo "ok    copiar num app e colar em outro"; PASS=$((PASS + 1))
else
    echo "FALHA copiar num app e colar em outro"; FAIL=$((FAIL + 1))
fi
# Teclas mortas (layout ABNT2 do sistema de teste): ´ + a = á e ~ + o = õ, também com
# o idioma "C" (sessão iniciada do console, sem LANG).
: > "$TDIR/acentos.txt"
LANG=C LC_ALL= timeout 150 "$SCEN" "400 click 400 300; 420 key leftbrace; 430 key a; 440 key apostrophe; 450 key o; 480 combo ctrl+s; 530 list" -- "$TEXT_APP '$TDIR/acentos.txt'" >/dev/null 2>&1
if [ "$(cat "$TDIR/acentos.txt")" = "áõ" ]; then
    echo "ok    teclas mortas: ´ + a = á e ~ + o = õ (idioma C)"; PASS=$((PASS + 1))
elif ! grep -q 'XKBLAYOUT="br"' /etc/default/keyboard 2>/dev/null; then
    echo "pulo  teclas mortas (o teclado do sistema não é ABNT2)"
else
    echo "FALHA teclas mortas ($(cat "$TDIR/acentos.txt"))"; FAIL=$((FAIL + 1))
fi
# Abrir… (Ctrl+O) pelo seletor de arquivos: pasta com Sub/, b.md e nota.txt; duplo
# clique em "nota.txt" (190, 443) abre outra janela (a atual já tem documento).
mkdir -p "$TDIR/abrir/Sub"; printf 'outro\n' > "$TDIR/abrir/b.md"; printf 'nota\n' > "$TDIR/abrir/nota.txt"
check "editor: Abrir… pelo seletor de arquivos" 'janela "nota.txt — Editor de Texto"' "400 click 400 300; 420 combo ctrl+o; 480 click 190 443; 485 click 190 443; 600 list" "$TEXT_APP '$TDIR/abrir/b.md'"
# Salvar sem título: o seletor abre em Documentos (pasta pessoal de teste) com o nome
# sugerido; Ctrl+A, "novo" e Enter salvam lá.
mkdir -p "$TDIR/home/Documentos"
timeout 150 "$SCEN" "400 click 400 300; 420 type ola; 440 combo ctrl+s; 480 combo ctrl+a; 490 type novo; 500 key enter; 560 list" -- "HOME='$TDIR/home' $TEXT_APP" >/dev/null 2>&1
if [ "$(cat "$TDIR/home/Documentos/novo" 2>/dev/null)" = "ola" ]; then
    echo "ok    editor: Salvar como… pelo seletor de arquivos"; PASS=$((PASS + 1))
else
    echo "FALHA editor: Salvar como… pelo seletor de arquivos"; FAIL=$((FAIL + 1))
fi
check "editor: alteração não salva marca o título" 'janela "• teste.rs' "400 type x; 460 list" "$TEXT_APP '$TDIR/teste.rs'"
check "fechar com alteração pergunta antes (janela continua)" 'janela "• teste.rs' "400 type x; 430 click 63 61; 480 list" "$TEXT_APP '$TDIR/teste.rs'"
rm -rf "$TDIR"

echo "== Calculadora (mide-calculator: modos, contas, RPN e histórico)"
# A Calculadora lembra o modo em [calculator] do config.toml: cada teste usa uma
# configuração própria (um modo lembrado mudaria o tamanho dela nos outros testes) e o
# log dela (RUST_LOG=mide_calculator=debug, no stdout) registra a tela ("calculadora:
# tela 14"). Janela em (48, 76).
# - Básica (250x380): coluna c em x = 84 + 59,5c, linha r em y = 187 + 58,4r
#   (linha 1: 7 8 9 ×; linha 4: 0 , =).
# - Científica (620x380): coluna c em x = 84 + 60,8c, linhas como na Básica
#   (linha 0: ( ) mc m+ m− mr; linha 3: x! sen cos tg e EE).
# - Programador (520x560): bases HEX/DEC/OCT/BIN em y=95, x = 84 + 58i; bits em
#   y=189 (63–32) e y=233 (31–0), bit n de 31 a 0 em x = 50 + 15,6(31 − n) + 8.
# - Barra: "Visualizar" em x=232; no menu, Básica y=55, Científica 79, Programador 103,
#   Notas matemáticas 127.
# - Notas (720x480): a nota começa em x=276, y=145 (linhas de 23 pt); botão da Lixeira
#   em (738, 102).
CDIR="$(mktemp -d)"
# calc_run "roteiro" [opções de [calculator]]: abre a Calculadora (com CALC_SCEN, outro
# cenário, como o do barramento D-Bus de teste). As notas e a Lixeira ficam em
# $CDIR/data (XDG_DATA_HOME só da Calculadora), apagada a cada execução (com
# CALC_KEEP_DATA=1, fica a da execução anterior). As cotações vêm do curl falso
# (tests/data/bcb/curl, com os boletins de 20 e 21/10/2026; nada vai à internet), com
# o cache em $CDIR/data/cache e os pedidos anotados em $CDIR/data/curl/chamadas; a hora
# fica em 21/10/2026 15h (o boletim do dia já saiu). CALC_NO_CURL=1 tira o curl falso.
calc_run() {
    local script="$1" saved="${2:-}"
    rm -rf "$CDIR/cfg" "$CDIR/calc.log"
    [ -z "${CALC_KEEP_DATA:-}" ] && rm -rf "$CDIR/data"
    mkdir -p "$CDIR/cfg/mide" "$CDIR/data/curl"
    [ -n "$saved" ] && printf '[calculator]\n%b\n' "$saved" > "$CDIR/cfg/mide/config.toml"
    local curl="MIDE_CURL='$PWD/tests/data/bcb/curl'"
    [ -n "${CALC_NO_CURL:-}" ] && curl=""
    XDG_CONFIG_HOME="$CDIR/cfg" timeout 150 "${CALC_SCEN:-$SCEN}" "$script" -- "RUST_LOG=mide_calculator=debug XDG_DATA_HOME='$CDIR/data' XDG_CACHE_HOME='$CDIR/data/cache' MIDE_CURL_DIR='$CDIR/data/curl' MIDE_RATES_NOW=2026-10-21T15:00:00-03:00 $curl $CALCULATOR >'$CDIR/calc.log' 2>&1" > "$CDIR/scen.log" 2>&1
}
# calc_calls "nome" N: o curl falso recebeu N pedidos na última execução.
calc_calls() {
    local got
    got="$(cat "$CDIR/data/curl/chamadas" 2>/dev/null | wc -l)"
    if [ "$got" = "$2" ]; then echo "ok    $1"; PASS=$((PASS + 1)); else echo "FALHA $1 (esperado $2 pedido(s), obtido $got)"; FAIL=$((FAIL + 1)); fi
}
# calc_screen "nome" "tela esperada" "roteiro" [opções]: confere a última tela.
calc_screen() {
    calc_run "$3" "${4:-}"
    local got
    got="$(grep -a "calculadora: tela " "$CDIR/calc.log" | tail -1 | sed 's/.*calculadora: tela //')"
    if [ "$got" = "$2" ]; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1 (esperado \"$2\", obtido \"$got\")"; FAIL=$((FAIL + 1))
    fi
}
# calc_window "nome" "regex da janela no autoteste" "roteiro" [opções].
calc_window() {
    calc_run "$3" "${4:-}"
    if grep -a "autoteste: janela" "$CDIR/scen.log" | tail -1 | grep -Eq "$2"; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1"; echo "      esperado: $2"
        grep -a "autoteste: janela" "$CDIR/scen.log" | tail -1 | sed 's/^/      /'; FAIL=$((FAIL + 1))
    fi
}
calc_screen "Calculadora: precedência pelo teclado (2+3*4= dá 14)" "14" "400 type 2+3*4=; 440 list"
calc_screen "Calculadora: clicar nas teclas (7 × 8 = 56)" "56" "400 click 84 246; 410 click 262 246; 420 click 143 246; 430 click 262 421; 460 list"
calc_screen "Calculadora: precisão entre contas (1 ÷ 3 = × 3 = dá 1)" "1" "400 type 1/3=*3=; 440 list"
calc_screen "Calculadora: desfazer (Ctrl+Z) volta antes do =" "3" "400 type 12+3=; 420 combo ctrl+z; 450 list"
calc_screen "Calculadora: refazer (Ctrl+Shift+Z)" "15" "400 type 12+3=; 420 combo ctrl+z; 430 combo ctrl+shift+z; 460 list"
calc_screen "Calculadora: 2 casas decimais, sem separador de milhar" "1234,50" "400 type 1234,5=; 440 list" 'decimals = 2\nthousands = false'
calc_window "Calculadora: o modo lembrado abre no tamanho dele" 'janela "Calculadora" .*visível=620x380' "400 list" 'mode = "cientifica"'
calc_window "Calculadora: Ctrl+3 abre o Programador" 'janela "Calculadora" .*visível=520x560' "400 combo ctrl+3; 440 list"
calc_window "Calculadora: Ctrl+T mostra o histórico ao lado" 'janela "Calculadora" .*visível=480x380' "400 combo ctrl+t; 440 list"
calc_screen "Calculadora: Científica, sen 30° = 0,5" "0,5" "400 type 30; 420 click 145 362; 450 list" 'mode = "cientifica"'
calc_screen "Calculadora: Científica, 2 ( 3 + 4 ) = 14 (multiplicação implícita)" "14" "400 type 2; 410 click 84 187; 420 type 3+4; 430 click 145 187; 440 type =; 470 list" 'mode = "cientifica"'
calc_screen "Calculadora: Programador, 255 em HEX é FF" "FF" "400 type 255; 420 click 84 95; 450 list" 'mode = "programador"'
calc_screen "Calculadora: Programador, clicar no bit 4 de 1 dá 17" "17" "400 type 1; 420 click 488 233; 450 list" 'mode = "programador"'
calc_screen "Calculadora: RPN, 3 Enter 4 + dá 7" "7" "400 type 3; 410 key enter; 420 type 4+; 450 list" 'rpn = true'
calc_screen "Calculadora: o número passa do Programador para a Básica" "255" "400 type 255; 420 combo ctrl+1; 450 list" 'mode = "programador"'
# ⌫ na tela (Básica) em (73, 136); na Científica, 2nd em (84, 246), yˣ (com 2nd) em
# (328, 246) e EE em (388, 362); no Básico com RPN, a fileira da pilha em y=187 (LASTx
# em x=262) e a janela 58 pt mais alta.
calc_screen "Calculadora: ⌫ na tela apaga o último algarismo" "12" "400 type 123; 420 click 73 136; 450 list"
calc_screen "Calculadora: 2nd troca eˣ por yˣ (3 yˣ 2 = 8)" "8" "400 type 3; 410 click 84 246; 420 click 328 246; 430 type 2=; 460 list" 'mode = "cientifica"'
calc_screen "Calculadora: EE mostra o expoente enquanto digita (2 EE 3)" "2e3" "400 type 2; 410 click 388 362; 420 type 3; 450 list" 'mode = "cientifica"'
calc_window "Calculadora: RPN no Básico ganha a fileira da pilha" 'janela "Calculadora" .*visível=250x438' "400 list" 'rpn = true'
calc_window "Calculadora: Ctrl+R liga o RPN" 'janela "Calculadora" .*visível=250x438' "400 combo ctrl+r; 440 list"
calc_screen "Calculadora: RPN no Básico, LASTx pela tecla da tela" "4" "400 type 3; 410 key enter; 420 type 4+; 430 click 262 187; 460 list" 'rpn = true'
calc_window "Calculadora: histórico também no Programador" 'janela "Calculadora" .*visível=750x560' "400 list" 'mode = "programador"\nhistory = true'
calc_screen "Calculadora: a base do Programador fica lembrada" "FF" "400 type ff; 440 list" 'mode = "programador"\nbase = "hex"'
# Conversor de unidades (E3b, etapa 2): o visor ganha 64 pt e as teclas descem; o ⇅
# fica em (73, 159). O log registra "calculadora: conversão 100 °F = 37,7777777778 °C".
# calc_convert "nome" "conversão esperada" "roteiro" [opções]: a última conversão.
calc_convert() {
    calc_run "$3" "${4:-}"
    local got
    got="$(grep -a "calculadora: conversão " "$CDIR/calc.log" | tail -1 | sed 's/.*calculadora: conversão //')"
    if [ "$got" = "$2" ]; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1 (esperado \"$2\", obtido \"$got\")"; FAIL=$((FAIL + 1))
    fi
}
# calc_log "nome" "regex" e calc_config "nome" "regex": conferem o log e o config.toml da
# última execução (várias conferências de uma vez só).
calc_log() {
    if grep -aEq "$2" "$CDIR/calc.log"; then echo "ok    $1"; PASS=$((PASS + 1)); else echo "FALHA $1 (sem \"$2\" no log)"; FAIL=$((FAIL + 1)); fi
}
calc_config() {
    if grep -Eq "$2" "$CDIR/cfg/mide/config.toml" 2>/dev/null; then echo "ok    $1"; PASS=$((PASS + 1)); else echo "FALHA $1 (sem \"$2\" no config.toml)"; FAIL=$((FAIL + 1)); fi
}
calc_window "Calculadora: Ctrl+U liga o conversor (janela 64 pt mais alta)" 'janela "Calculadora" .*visível=250x444' "400 combo ctrl+u; 440 list"
calc_config "Calculadora: o conversor fica lembrado" '^convert = true'
calc_window "Calculadora: conversor com RPN no Básico" 'janela "Calculadora" .*visível=250x502' "400 list" 'convert = true\nrpn = true'
calc_window "Calculadora: o Programador não mostra o conversor" 'janela "Calculadora" .*visível=520x560' "400 combo ctrl+3; 440 list" 'convert = true'
calc_convert "Calculadora: ⇅ troca as unidades (100 °C = 212 °F)" "100 °C = 212 °F" "400 type 100; 420 click 73 159; 450 list" 'convert = true\nconvert_pairs = "temperatura=f>c"'
calc_log "Calculadora: o conversor converte a cada tecla (100 °F)" "conversão 100 °F = 37,7777777778 °C"
calc_screen "Calculadora: ↓ usa a conversão na conta (2 + 12,5 pol em cm = 33,75)" "33,75" "400 type 2+12,5; 410 key down; 420 type =; 450 list" 'convert = true'
calc_screen "Calculadora: Ctrl+Z desfaz a conversão" "12,5" "400 type 2+12,5; 410 key down; 420 combo ctrl+z; 450 list" 'convert = true'
calc_screen "Calculadora: no RPN, ↓ troca o X (7 pol = 17,78 cm)" "17,78" "400 type 3; 410 key enter; 420 type 4+; 430 key down; 460 list" 'convert = true\nrpn = true'
calc_convert "Calculadora: seletor de unidades, buscar e Enter (milha)" "1 mi = 160.934,4 cm" "400 type 1; 410 combo ctrl+l; 440 type milha; 470 key enter; 500 list" 'convert = true'
calc_config "Calculadora: o par escolhido fica lembrado" 'convert_pairs = "comprimento=mi>cm'
calc_convert "Calculadora: unidade de outra categoria leva o par dela (°F → °C)" "1 °F = -17,2222222222 °C" "400 type 1; 410 combo ctrl+l; 440 type fahrenheit; 470 key enter; 500 list" 'convert = true'
calc_screen "Calculadora: Esc fecha o seletor sem apagar a conta" "12,5" "400 type 12,5; 410 combo ctrl+l; 440 key esc; 470 list" 'convert = true'
# O nome da unidade da linha de baixo (78, 195) abre o seletor para ela.
calc_convert "Calculadora: clicar no nome da linha de baixo escolhe a unidade dela" "1 pol = 0,0254 m" "400 type 1; 420 click 78 195; 450 type metro; 480 key enter; 510 list" 'convert = true'
# Moeda (E3b, etapa 2): cotações do Banco Central pelo curl falso.
calc_convert "Moeda: 100 USD em BRL pela cotação de 21/10" "100 USD = 502,77 BRL (BCB 2026-10-21)" "400 type 100; 500 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"\ncurrency_online = true'
calc_calls "Moeda: um pedido ao Banco Central" 1
CALC_KEEP_DATA=1 calc_convert "Moeda: reabrir no mesmo dia usa o cache" "100 USD = 502,77 BRL (BCB 2026-10-21)" "400 type 100; 500 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"\ncurrency_online = true'
calc_calls "Moeda: reabrir no mesmo dia não busca de novo" 1
calc_run "400 key enter; 460 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"'
calc_config "Moeda: a primeira vez pergunta, e o Enter (Buscar) fica lembrado" '^currency_online = true'
calc_calls "Moeda: depois de aceitar, busca" 1
# Sem rede, com o boletim de 20/10 guardado: avisa e usa o de 20/10.
rm -rf "$CDIR/data"; mkdir -p "$CDIR/data/cache/mide/cotacoes" "$CDIR/data/curl"
cp tests/data/bcb/20261020.csv "$CDIR/data/cache/mide/cotacoes/bcb.csv"; touch "$CDIR/data/curl/sem-rede"
CALC_KEEP_DATA=1 calc_convert "Moeda: sem rede vale a cotação guardada (20/10)" "100 USD = 501,00 BRL (BCB 2026-10-20)" "400 key enter; 420 type 100; 500 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"\ncurrency_online = true'
calc_log "Moeda: sem rede, o log registra" "cotações BCB de 2026-10-20 \(sem conexão"
rm -f "$CDIR/data/curl/sem-rede" "$CDIR/data/curl/chamadas"
CALC_KEEP_DATA=1 calc_convert "Moeda: busca desligada usa o cache" "100 USD = 501,00 BRL (BCB 2026-10-20)" "400 type 100; 500 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"\ncurrency_online = false'
calc_calls "Moeda: busca desligada não pede nada" 0
CALC_NO_CURL=1 calc_run "400 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"\ncurrency_online = true'
calc_log "Moeda: na bateria, sem o curl falso, nada é buscado" "cotações \(teste\): sem MIDE_CURL, nada buscado"
# Menus da Calculadora na barra (num barramento D-Bus de teste, como os do Terminal).
CALC_SCEN=tests/scenario-dbus.sh calc_window "Calculadora: Visualizar > Científica pela barra" 'janela "Calculadora" .*visível=620x380' "500 click 232 14; 540 click 250 79; 580 list"
CALC_SCEN=tests/scenario-dbus.sh calc_window "Calculadora: Visualizar > Conversor de unidades pela barra" 'janela "Calculadora" .*visível=250x444' "500 click 232 14; 540 click 250 208; 580 list"
# Notas matemáticas (E3b, etapa 2): o log registra o que muda em cada linha
# ("calculadora: notas: linha 1 = 360" ou "...linha 4: erro: ...").
# notes_line "nome" N "resultado": a última linha "notas: linha N" da última execução
# (" = 360" ou ": erro: ..."); várias conferências de uma vez só.
notes_line() {
    local got
    got="$(grep -a "calculadora: notas: linha $2[ :]" "$CDIR/calc.log" | tail -1 | sed "s/.*calculadora: notas: linha $2//")"
    if [ "$got" = "$3" ]; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1 (esperado \"$3\", obtido \"$got\")"; FAIL=$((FAIL + 1))
    fi
}
# calc_file "nome" "caminho dentro de $CDIR/data (com *)" "conteúdo": confere um arquivo
# das notas ou da Lixeira.
calc_file() {
    local got
    got="$(cat "$CDIR"/data/$2 2>/dev/null)"
    if [ "$got" = "$3" ]; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1 (esperado \"$3\", obtido \"$got\")"; FAIL=$((FAIL + 1))
    fi
}
# A24: "minimizar tudo" duas vezes (a segunda pega a janela ainda saindo) e restaurar:
# a janela volta recebendo cliques (antes ficava na lista das minimizadas e o mide
# ignorava o clique: parecia congelada). O 7 fica em (84, 246).
calc_screen "Super+D duas vezes e restaurar: a janela volta recebendo cliques" "7" "400 combo super+d; 410 combo super+d; 500 restore; 560 click 84 246; 600 list"
calc_window "Calculadora: Ctrl+4 abre as Notas (720x480)" 'janela "Calculadora" .*visível=720x480' "400 combo ctrl+4; 440 list"
calc_run "400 type 120x3=; 420 key enter; 440 type aluguel=1.200; 460 key enter; 480 type aluguel*12=; 500 key enter; 520 type y*2=; 540 key enter; 560 type hoje_gastei_2*3=; 580 key up; 590 key up; 600 key up; 610 key end; 620 key backspace; 625 key backspace; 630 key backspace; 635 key backspace; 640 key backspace; 650 type 1.500; 720 list" 'mode = "notas"'
notes_line "Notas: 120x3= dá 360" 1 " = 360"
calc_log "Notas: variável (aluguel*12= dá 14.400)" "notas: linha 3 = 14\.400"
notes_line "Notas: nome desconhecido dá erro" 4 ": erro: “y” não é uma variável nem uma unidade"
notes_line "Notas: prosa antes da conta (hoje gastei 2*3= dá 6)" 5 " = 6"
notes_line "Notas: mudar a variável em cima refaz a conta de baixo (18.000)" 3 " = 18.000"
calc_file "Notas: a nota fica gravada só com o que foi digitado" "mide/calculadora/notas/*.txt" "$(printf '120x3=\naluguel=1.500\naluguel*12=\ny*2=\nhoje gastei 2*3=')"
calc_run "400 combo shift+9; 410 type 2+3; 420 combo shift+0; 430 type *4=; 440 key enter; 450 type 100+10; 460 combo shift+5; 470 type =; 520 list" 'mode = "notas"'
notes_line "Notas: parênteses ((2+3)*4= dá 20)" 1 " = 20"
notes_line "Notas: porcentagem (100+10%= dá 110)" 2 " = 110"
# O cenário dura 9 s ao todo (SCENARIO_WAIT é o tempo total): a tecla vem perto de 5,5 s
# e a nota é gravada 1 s depois; uma espera de 3,5 s ou mais deixaria a nota sem arquivo.
SCENARIO_WAIT=9 calc_run "400 type 2+2=; 520 list" 'mode = "notas"'
calc_file "Notas: grava 1 s depois da última tecla" "mide/calculadora/notas/*.txt" "2+2="
CALC_KEEP_DATA=1 calc_run "400 list" 'mode = "notas"'
notes_line "Notas: reabrir mostra a nota com o resultado" 1 " = 4"
calc_run "400 type 2+2=; 480 key backspace; 485 key backspace; 490 type 3=; 560 combo ctrl+z; 570 combo ctrl+z; 620 list" 'mode = "notas"'
notes_line "Notas: Ctrl+Z volta a conta de antes (depois de gravar)" 1 " = 4"
calc_run "400 type 1+1=; 420 combo ctrl+n; 460 list" 'mode = "notas"'
calc_log "Notas: Ctrl+N cria outra nota" "nova nota, 2 nota\(s\)"
calc_run "400 type 1+1=; 500 click 738 102; 540 list" 'mode = "notas"'
calc_log "Notas: Lixeira pela barra de ferramentas" "notas: na Lixeira "
calc_file "Notas: a nota vai para a Lixeira" "Trash/files/*.txt" "1+1="
calc_run "400 type 120x3=; 500 rclick 345 145; 530 click 380 170; 560 list" 'mode = "notas"'
calc_log "Notas: clique direito no resultado copia ele" "notas: copiado 360"
calc_run "400 type 2x21=; 500 rclick 330 145; 530 click 410 222; 560 list" 'mode = "notas"'
calc_log "Notas: Levar para a calculadora volta ao teclado com o número" "calculadora: tela 42$"
# Unidades e moedas nas notas (E3b, parte 11); a moeda pelo curl falso.
calc_run "400 type 10_km_em_m=; 420 key enter; 440 type 1/2_kg_em_g=; 460 key enter; 480 type 1_h_30_min_em_min=; 540 list" 'mode = "notas"'
notes_line "Notas com unidades: 10 km em m dá 10.000 m" 1 " = 10.000 m"
notes_line "Notas com unidades: 1/2 kg em g dá 500 g" 2 " = 500 g"
notes_line "Notas com unidades: 1 h 30 min em min dá 90 min" 3 " = 90 min"
calc_run "400 type 100_usd_em_brl=; 520 list" 'mode = "notas"\ncurrency_online = true'
notes_line "Notas com moeda: 100 USD em BRL pela cotação (curl falso)" 1 " = R\$ 502,77"
calc_calls "Notas com moeda: a nota pediu as cotações ao Banco Central" 1
calc_run "400 type 2+2=; 460 list" 'mode = "notas"'
calc_calls "Notas sem moeda não buscam cotações" 0
CALC_SCEN=tests/scenario-dbus.sh calc_window "Calculadora: Visualizar > Notas matemáticas pela barra" 'janela "Calculadora" .*visível=720x480' "500 click 232 14; 540 click 250 127; 580 list"
rm -rf "$CDIR"

echo "== apps X11 (XWayland, precisa de xwayland-satellite, Xwayland e xclock)"
if command -v xclock >/dev/null && { command -v xwayland-satellite >/dev/null || [ -x "$HOME/.cargo/bin/xwayland-satellite" ]; }; then
    # O app X11 usa o display do XWayland deste mide (gravado ao lado do socket).
    XAPP="sh -c 'sleep 2; DISPLAY=\$(cat \"\$XDG_RUNTIME_DIR/\$MIDE_SOCKET.xdisplay\") exec xclock -geometry 300x300'"
    # O xclock abre ~3 s depois do mide (espera do app e partida do Xwayland): os
    # passos começam no quadro 2000.
    MIDE_XWAYLAND=1 SCENARIO_WAIT=42 SCENARIO_TIMEOUT=60 check "app X11 (xclock) abre como janela do mide" 'janela "xclock" .*barra=30' "2000 list" "$XAPP"
    # Aparece na lista de antes do clique (1 vez) e some depois dele.
    MIDE_XWAYLAND=1 SCENARIO_WAIT=42 SCENARIO_TIMEOUT=60 check_count "fechar pela barra do mide fecha o app X11" 'janela "xclock"' 1 "2000 list; 2020 click 63 61; 2100 list" "$XAPP"
else
    echo "pulo  apps X11 (falta xclock ou xwayland-satellite)"
fi

echo "== bloqueio de tela (mide-lock)"
# Senha só dos testes (vale apenas nas compilações de desenvolvimento).
export MIDE_LOCK_TEST_PASSWORD=teste
SCENARIO_WAIT=16 check "Super+L bloqueia a tela" "autoteste: tela bloqueada \(1 tela" "430 combo super+l; 650 list" "$TERMINAL"
SCENARIO_WAIT=16 check "senha errada mantém bloqueada" "autoteste: tela bloqueada" "430 combo super+l; 620 type errada; 622 key enter; 760 list" "$TERMINAL"
SCENARIO_WAIT=16 check "senha certa desbloqueia" "autoteste: tela livre, desbloqueios: 1" "430 combo super+l; 620 type teste; 622 key enter; 760 list" "$TERMINAL"
unset MIDE_LOCK_TEST_PASSWORD
# Dois monitores: uma tela de bloqueio em cada.
MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT=16 check "bloqueio cobre os dois monitores" "autoteste: tela bloqueada \(2 tela" "400 combo super+l; 650 list"
# Com dois monitores, a resposta da senha não pode se perder na cópia da tela de
# bloqueio do outro monitor (antes, a senha certa às vezes não desbloqueava).
MIDE_LOCK_TEST_PASSWORD=teste MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT=16 check "senha certa desbloqueia com dois monitores" "autoteste: tela livre, desbloqueios: 1" "400 combo super+l; 620 type teste; 622 key enter; 760 list"
# Botão de ligar: o mide pergunta antes de desligar (o logind não desliga direto); com a
# tela bloqueada, não abre nada por cima dela.
check "botão de ligar pergunta antes de desligar" 'camada "mide-bar-menu"' "400 key power; 460 list"
MIDE_LOCK_TEST_PASSWORD=teste SCENARIO_WAIT=16 check_count "botão de ligar com a tela bloqueada não abre a confirmação" 'camada "mide-bar-menu"' 0 "400 combo super+l; 600 key power; 680 list"
# Tempo sem uso (contado em segundos nos testes): apaga e acende ao mexer.
IDLECFG="$(mktemp -d)"
mkdir -p "$IDLECFG/mide"
printf '[lock]\nidle_minutes = 0\nblank_minutes = 3\n' > "$IDLECFG/mide/config.toml"
OLD_IDLE_CFG="$XDG_CONFIG_HOME"
export XDG_CONFIG_HOME="$IDLECFG" MIDE_LOCK_IDLE=1 MIDE_IDLE_UNIT=seconds
check "tela apaga depois do tempo sem uso" "autoteste: tela apagada" "200 move 500 400; 600 list"
check_count "mexer o mouse acende a tela" "autoteste: tela apagada" 0 "200 move 500 400; 600 move 520 420; 610 list"
printf '[lock]\nidle_minutes = 3\nblank_minutes = 0\n' > "$IDLECFG/mide/config.toml"
SCENARIO_WAIT=16 check "bloqueia sozinho depois do tempo sem uso" "autoteste: tela bloqueada" "200 move 500 400; 700 list"
# Energia: suspender depois do tempo sem uso (simulado: só registra o pedido ao logind)
# e o botão de ligar configurado para suspender.
printf '[lock]\nidle_minutes = 0\nblank_minutes = 0\n[power]\nsuspend_minutes = 3\n' > "$IDLECFG/mide/config.toml"
check "Energia: suspende depois do tempo sem uso" "sistema \(teste\): logind Suspend" "200 move 500 400; 600 list"
printf '[lock]\nidle_minutes = 0\nblank_minutes = 0\n[power]\nbutton = "suspend"\n' > "$IDLECFG/mide/config.toml"
check "Energia: botão de ligar configurado para suspender" "sistema \(teste\): logind Suspend" "400 key power; 460 list"
export XDG_CONFIG_HOME="$OLD_IDLE_CFG"
unset MIDE_LOCK_IDLE MIDE_IDLE_UNIT
rm -rf "$IDLECFG"

echo "== captura de tela (Print)"
# A imagem vai para Imagens/Capturas de tela: no teste, uma pasta temporária.
SHOTS="$(mktemp -d)"
printf 'XDG_PICTURES_DIR="%s"\n' "$SHOTS" > "$TESTCFG/user-dirs.dirs"
# O Ctrl+S vem uns 11 s depois da partida: com a espera padrão (10 s) o teste ficava no limite.
SCENARIO_WAIT=16 timeout 150 "$SCEN" "400 key print; 600 press 300 200; 610 move 500 350; 620 release 700 450; 660 combo ctrl+s; 720 list" >/dev/null 2>&1
if ls "$SHOTS/Capturas de tela/"*.png >/dev/null 2>&1 && python3 -c "
import sys, glob
from PIL import Image
img = Image.open(glob.glob(sys.argv[1] + '/Capturas de tela/*.png')[0])
sys.exit(0 if img.size == (400, 250) else 1)" "$SHOTS"; then
    echo "ok    captura de uma área salva em Imagens/Capturas de tela (400x250)"; PASS=$((PASS + 1))
else
    echo "FALHA captura de uma área salva em Imagens/Capturas de tela"; FAIL=$((FAIL + 1))
fi
SCENARIO_WAIT=16 check "captura: Enter copia a imagem para a área de transferência" "bytes de image/png copiados" "400 key print; 600 press 300 200; 610 move 500 350; 620 release 700 450; 660 key enter; 720 list"
# Estilo Mac: ⌘S (a tecla Alt) também salva (antes, o Ctrl virava Super e nem o
# Ctrl+S salvava).
MSHOT="$(mktemp -d)"
mkdir -p "$MSHOT/cfg/mide"
printf '[keyboard]\nmac_style = true\n' > "$MSHOT/cfg/mide/config.toml"
printf 'XDG_PICTURES_DIR="%s"\n' "$MSHOT/pics" > "$MSHOT/cfg/user-dirs.dirs"
XDG_CONFIG_HOME="$MSHOT/cfg" SCENARIO_WAIT=16 timeout 150 "$SCEN" "400 key print; 600 press 300 200; 610 move 500 350; 620 release 700 450; 660 combo alt+s; 720 list" >/dev/null 2>&1
if ls "$MSHOT/pics/Capturas de tela/Captura de tela "*.png >/dev/null 2>&1; then
    echo "ok    captura: ⌘S salva no estilo Mac (Imagens/Capturas de tela, nome com data e hora)"; PASS=$((PASS + 1))
else
    echo "FALHA captura: ⌘S salva no estilo Mac"; FAIL=$((FAIL + 1))
fi
rm -rf "$MSHOT"
# Atalhos do Mac: ⌘⇧4 (uma área, salva ao soltar o mouse) e ⌘⇧3 (a tela inteira, sem
# janela), também com o launcher aberto (a imagem sai com ele). Fora do estilo Mac o ⌘
# é a tecla Super; no estilo Mac, a Alt.
shot_test() {
    local name="$1" script="$2" want="$3" cfg pics got
    cfg="$(mktemp -d)"
    pics="$(mktemp -d)"
    mkdir -p "$cfg/mide"
    [ -n "${4:-}" ] && printf '[keyboard]\nmac_style = true\n' > "$cfg/mide/config.toml"
    printf 'XDG_PICTURES_DIR="%s"\n' "$pics" > "$cfg/user-dirs.dirs"
    XDG_CONFIG_HOME="$cfg" SCENARIO_WAIT=14 timeout 150 "$SCEN" "$script" >/dev/null 2>&1
    got="$(python3 -c "
import glob, sys
from PIL import Image
files = glob.glob(sys.argv[1] + '/Capturas de tela/Captura de tela *.png')
print('%dx%d' % Image.open(files[0]).size if files else 'nada')" "$pics" 2>/dev/null)"
    if [ "$got" = "$want" ]; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name (esperado $want, obtido $got)"; FAIL=$((FAIL + 1))
    fi
    rm -rf "$cfg" "$pics"
}
AREA="600 press 300 200; 610 move 450 300; 620 move 600 400; 625 release 600 400; 700 list"
shot_test "captura: ⌘⇧4 salva a área ao soltar o mouse" "400 combo super+shift+4; $AREA" 300x200
shot_test "captura: ⌘⇧3 salva a tela inteira" "400 combo super+shift+3; 600 list" 1366x768
if grep -aq "som (teste): screen-capture" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log"; then
    echo "ok    captura toca o som do obturador"; PASS=$((PASS + 1))
else
    echo "FALHA captura toca o som do obturador"; FAIL=$((FAIL + 1))
fi
shot_test "captura: ⌘⇧3 com o launcher aberto" "400 combo ctrl+shift+space; 470 combo super+shift+3; 650 list" 1366x768
shot_test "captura: ⌘⇧4 no estilo Mac (Alt+Shift+4)" "400 combo alt+shift+4; $AREA" 300x200 mac

# Captura por tela (B22): `screenshot all` salva uma imagem de cada tela, em pixels, e
# responde com a posição, o tamanho em pontos e a escala de cada uma.
SA="$(mktemp -d)"
MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT=8 timeout 150 "$SCEN" "400 list" -- "sh -c 'sleep 3; mide-ctl screenshot all $SA/t > $SA/resposta.txt'" >/dev/null 2>&1
if grep -qP '^HEADLESS-2\t1366\t0\t1920\t1080\t1\t-\t' "$SA/resposta.txt" 2>/dev/null && python3 -c "
import sys
from PIL import Image
d = sys.argv[1]
sys.exit(0 if Image.open(d + '/t-HEADLESS-1.png').size == (1366, 768) and Image.open(d + '/t-HEADLESS-2.png').size == (1920, 1080) else 1)" "$SA" 2>/dev/null; then
    echo "ok    captura: screenshot all salva cada tela (1366x768 e 1920x1080)"; PASS=$((PASS + 1))
else
    echo "FALHA captura: screenshot all salva cada tela ($(head -c 300 "$SA/resposta.txt" 2>/dev/null))"; FAIL=$((FAIL + 1))
fi
rm -rf "$SA"
# Tela em 1,25 (1920x1080 pixels, 1536x864 pontos): a área (300, 200)–(700, 450) em
# pontos sai com 500x313 pixels, do lugar certo da imagem (igual ao quadro sem a
# Captura), e a tela escurece fora dela (antes, nem escurecia e o recorte saía do
# lugar errado).
SC="$(mktemp -d)"
mkdir -p "$SC/cfg/mide"
printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.25\n' > "$SC/cfg/mide/config.toml"
printf 'XDG_PICTURES_DIR="%s"\n' "$SC/pics" > "$SC/cfg/user-dirs.dirs"
XDG_CONFIG_HOME="$SC/cfg" SCENARIO_OUT="$SC/antes" MIDE_HEADLESS_OUTPUTS=1920x1080 SCENARIO_WAIT=8 timeout 150 "$SCEN" "300 list" 300 >/dev/null 2>&1
XDG_CONFIG_HOME="$SC/cfg" SCENARIO_OUT="$SC/durante" MIDE_HEADLESS_OUTPUTS=1920x1080 SCENARIO_WAIT=16 timeout 150 "$SCEN" "400 key print; 600 press 300 200; 610 move 500 350; 620 release 700 450; 660 combo ctrl+s; 720 list" 645 >/dev/null 2>&1
if python3 -c "
import sys, glob
from PIL import Image, ImageChops
d = sys.argv[1]
files = glob.glob(d + '/pics/Capturas de tela/*.png')
if not files: sys.exit('nada salvo')
shot = Image.open(files[0]).convert('RGB')
if shot.size != (500, 313): sys.exit('tamanho %dx%d' % shot.size)
before = Image.open(d + '/antes/frame.png').convert('RGB')
during = Image.open(d + '/durante/frame.png').convert('RGB')
diff = ImageChops.difference(shot, before.crop((375, 250, 875, 563)))
if sum(1 for p in diff.getdata() if max(p) > 40) > 500 * 313 // 100: sys.exit('recorte fora do lugar')
a, b = before.getpixel((100, 100)), during.getpixel((100, 100))
if not (sum(b) < sum(a) * 0.7): sys.exit('não escureceu: %s %s' % (a, b))
" "$SC"; then
    echo "ok    captura em escala 1,25: área de 500x313 no lugar certo, tela escurecida"; PASS=$((PASS + 1))
else
    echo "FALHA captura em escala 1,25"; FAIL=$((FAIL + 1))
fi
rm -rf "$SC"
# ⌘⇧3 em 1366x768 a 1,25: o compositor dá 1093x615 pontos (arredonda para cima), que
# vezes 1,25 seriam 1366x769; o arquivo sai com 1366x768, sem linha preta no fim.
SF="$(mktemp -d)"
mkdir -p "$SF/cfg/mide"
printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.25\n' > "$SF/cfg/mide/config.toml"
printf 'XDG_PICTURES_DIR="%s"\n' "$SF/pics" > "$SF/cfg/user-dirs.dirs"
XDG_CONFIG_HOME="$SF/cfg" SCENARIO_OUT="$SF/scen" SCENARIO_WAIT=14 timeout 150 "$SCEN" "400 combo super+shift+3; 600 list" >/dev/null 2>&1
if python3 -c "
import sys, glob
from PIL import Image
files = glob.glob(sys.argv[1] + '/pics/Capturas de tela/*.png')
if not files: sys.exit('nada salvo')
img = Image.open(files[0]).convert('RGB')
if img.size != (1366, 768): sys.exit('tamanho %dx%d' % img.size)
if all(img.getpixel((x, 767)) == (0, 0, 0) for x in range(0, 1366, 50)): sys.exit('linha preta no fim')
" "$SF"; then
    echo "ok    captura: ⌘⇧3 em 1366x768 a 1,25 sai com 1366x768, sem linha preta"; PASS=$((PASS + 1))
else
    echo "FALHA captura: ⌘⇧3 em 1366x768 a 1,25"; FAIL=$((FAIL + 1))
fi
rm -rf "$SF"

# Gravar a tela (A21, ⌘⇧5), com um ffmpeg de teste (`MIDE_FFMPEG`) que grava os
# argumentos e conta os bytes recebidos; Vídeos numa pasta de teste. Painel: "Gravar"
# em (980, 710); botão de parar na barra em (1220, 14).
REC="$(mktemp -d)"
mkdir -p "$REC/cfg/mide" "$REC/videos"
printf 'XDG_VIDEOS_DIR="%s/videos"\n' "$REC" > "$REC/cfg/user-dirs.dirs"
printf '#!/bin/sh\nfor a; do last=$a; done\nprintf "%%s\\n" "$@" > "$last.args"\nwc -c > "$last"\n' > "$REC/ffmpeg"
chmod +x "$REC/ffmpeg"
rec_file() { ls "$REC/videos/Gravações de tela/"*.mp4 2>/dev/null | head -n 1; }
XDG_CONFIG_HOME="$REC/cfg" MIDE_FFMPEG="$REC/ffmpeg" SCENARIO_WAIT=20 SCENARIO_TIMEOUT=40 timeout 150 "$SCEN" "300 combo super+shift+5; 420 click 980 710; 600 click 1220 14; 700 list" >/dev/null 2>&1
VIDEO="$(rec_file)"
if [ -n "$VIDEO" ] && [ "$(cat "$VIDEO")" -gt $((1366 * 768 * 4 * 10)) ] && grep -qx "1366x768" "$VIDEO.args" && grep -qx "@DEFAULT_MONITOR@" "$VIDEO.args"; then
    echo "ok    gravar a tela: ⌘⇧5, Gravar e parar pela barra (tela inteira, som do sistema)"; PASS=$((PASS + 1))
else
    echo "FALHA gravar a tela: ⌘⇧5, Gravar e parar pela barra"; FAIL=$((FAIL + 1))
fi
rm -rf "$REC/videos"/*
XDG_CONFIG_HOME="$REC/cfg" MIDE_FFMPEG="$REC/ffmpeg" SCENARIO_WAIT=14 timeout 150 "$SCEN" "700 list" -- "sh -c 'sleep 2; mide-ctl record start 100 100 401 301 microfone; sleep 2; mide-ctl record stop'" >/dev/null 2>&1
VIDEO="$(rec_file)"
if [ -n "$VIDEO" ] && grep -qx "400x300" "$VIDEO.args" && grep -qx "@DEFAULT_SOURCE@" "$VIDEO.args" && ! grep -qx "@DEFAULT_MONITOR@" "$VIDEO.args"; then
    echo "ok    gravar a tela: área (lados pares) com o microfone"; PASS=$((PASS + 1))
else
    echo "FALHA gravar a tela: área com o microfone"; FAIL=$((FAIL + 1))
fi
# Com o ffmpeg de verdade: o arquivo é um vídeo H.264 da tela com som (AAC) e dura o
# tempo gravado (4 s, menos o começo descartado).
if command -v ffmpeg >/dev/null && command -v ffprobe >/dev/null; then
    rm -rf "$REC/videos"/*
    XDG_CONFIG_HOME="$REC/cfg" SCENARIO_WAIT=20 SCENARIO_TIMEOUT=40 timeout 150 "$SCEN" "1500 list" -- "$CALCULATOR" "sh -c 'sleep 3; mide-ctl record start full sistema; sleep 4; mide-ctl record stop'" >/dev/null 2>&1
    VIDEO="$(rec_file)"
    INFO="$([ -n "$VIDEO" ] && ffprobe -v error -show_entries stream=codec_name,width,height:format=duration -of compact "$VIDEO" 2>/dev/null | tr '\n' ' ')"
    if echo "$INFO" | grep -q "codec_name=h264|width=1366|height=768" && echo "$INFO" | grep -q "codec_name=aac" && python3 -c "import re,sys; d=float(re.search(r'duration=([0-9.]+)', sys.argv[1]).group(1)); sys.exit(0 if 3.0 <= d <= 5.0 else 1)" "$INFO" 2>/dev/null; then
        echo "ok    gravar a tela com o ffmpeg: vídeo H.264 com som, duração certa"; PASS=$((PASS + 1))
    else
        echo "FALHA gravar a tela com o ffmpeg ($INFO)"; FAIL=$((FAIL + 1))
    fi
else
    echo "pulo  gravar a tela com o ffmpeg de verdade (sem o ffmpeg)"
fi
rm -rf "$REC"

# Captura pelos apps (wlr-screencopy, o que o portal do OBS usa): o grim tira a tela
# inteira e uma região; as imagens têm de ser iguais ao quadro do mide.
if command -v grim >/dev/null; then
    GR="$(mktemp -d)"
    SCENARIO_OUT="$GR/scen" SCENARIO_WAIT=10 timeout 150 "$SCEN" "400 list" 500 -- "$CALCULATOR" "sleep 5; grim $GR/tela.png; grim -g '40,40 300x420' $GR/regiao.png" >/dev/null 2>&1
    if python3 -c "
import sys
from PIL import Image, ImageChops
d = sys.argv[1]
frame = Image.open(d + '/scen/frame.png').convert('RGB')
full = Image.open(d + '/tela.png').convert('RGB')
part = Image.open(d + '/regiao.png').convert('RGB')
same = lambda a, b: a.size == b.size and sum(1 for p in ImageChops.difference(a, b).getdata() if max(p) > 40) < a.size[0] * a.size[1] // 100
sys.exit(0 if same(full, frame) and same(part, frame.crop((40, 40, 340, 460))) else 1)" "$GR"; then
        echo "ok    captura pelos apps (wlr-screencopy): tela inteira e região iguais ao quadro"; PASS=$((PASS + 1))
    else
        echo "FALHA captura pelos apps (wlr-screencopy)"; FAIL=$((FAIL + 1))
    fi
    rm -rf "$GR"
else
    echo "pulado: captura pelos apps (sem o grim)"
fi
# Janela de escolha do monitor (o portal lê o nome dele na saída).
PK="$(mktemp -d)"
MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT=8 timeout 150 "$SCEN" "400 click 300 224; 420 click 231 270; 460 list" -- "$PWD/target/debug/mide-screencast-picker > $PK/saida.txt" >/dev/null 2>&1
if [ "$(cat "$PK/saida.txt" 2>/dev/null)" = "HEADLESS-2" ]; then
    echo "ok    compartilhar a tela: escolher o segundo monitor"; PASS=$((PASS + 1))
else
    echo "FALHA compartilhar a tela: escolher o segundo monitor (saída: '$(cat "$PK/saida.txt" 2>/dev/null)')"; FAIL=$((FAIL + 1))
fi
rm -rf "$PK"
rm -f "$TESTCFG/user-dirs.dirs"
rm -rf "$SHOTS"

# Escala fracionária (wp_fractional_scale): um app GTK 4 numa tela em 1,25 recebe a
# escala exata (150 = 1,25 × 120) e desenha nítido. Precisa do loupe; sem ele, pulado.
if command -v loupe >/dev/null; then
    FS="$(mktemp -d)"
    mkdir -p "$FS/cfg/mide"
    printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.25\n' > "$FS/cfg/mide/config.toml"
    XDG_CONFIG_HOME="$FS/cfg" timeout 150 "$SCEN" "400 list" -- "WAYLAND_DEBUG=1 loupe 2> $FS/wl.log" >/dev/null 2>&1
    if grep -q "preferred_scale(150)" "$FS/wl.log" && grep -q '"xdg_activation_v1"' "$FS/wl.log"; then
        echo "ok    escala fracionária: app GTK 4 recebe 1,25 (e o xdg-activation existe)"; PASS=$((PASS + 1))
    else
        echo "FALHA escala fracionária: app GTK 4 recebe 1,25"; FAIL=$((FAIL + 1))
    fi
    rm -rf "$FS"
else
    echo "pulado: escala fracionária (sem o loupe)"
fi

echo "== monitores (telas virtuais)"
MONCFG="$(mktemp -d)"
mkdir -p "$MONCFG/mide"
OLD_MONCFG="$XDG_CONFIG_HOME"
export XDG_CONFIG_HOME="$MONCFG"
export MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080
SETTINGS="$PWD/target/debug/mide-settings"
check "duas telas lado a lado" 'tela HEADLESS-2 em \(1366, 0\) 1920x1080' "300 list"
printf '[monitors]\nprimary = "HEADLESS-2"\n' > "$MONCFG/mide/config.toml"
check "barra na tela principal escolhida" 'camada "mide-bar" em \(0, 0\) 1920x28' "300 list"
printf '[monitors.outputs."HEADLESS-2"]\nenabled = false\n' > "$MONCFG/mide/config.toml"
TELAS="$(timeout 150 "$SCEN" "300 list" 2>&1 | grep -c "autoteste: tela HEADLESS")"
if [ "$TELAS" = 1 ]; then
    echo "ok    tela desligada fica de fora"; PASS=$((PASS + 1))
else
    echo "FALHA tela desligada fica de fora (esperado 1, obtido $TELAS)"; FAIL=$((FAIL + 1))
fi
# Espelhar (A15): as duas telas no mesmo lugar, a segunda com a escala que iguala a
# largura da principal (1920 ÷ 1366 → 1366 lógicos).
printf '[monitors]\nmirror = true\n' > "$MONCFG/mide/config.toml"
check "espelhar telas: a segunda mostra a principal" 'tela HEADLESS-2 em \(0, 0\) 1366x' "300 list"
: > "$MONCFG/mide/config.toml"
# Configurações → Monitores: HEADLESS-2 no arranjo em (654, 255); "Usar como tela
# principal" em (404, 526); "Aplicar" em (430, 422); na janelinha de confirmação,
# "Manter" em (426, 289).
check "Monitores: tornar principal e manter" 'camada "mide-bar" em \(1366, 0\) 1920x28' "300 click 654 255; 330 click 404 526; 360 click 430 422; 460 click 426 289; 520 list" "$SETTINGS displays"
if grep -q 'primary = "HEADLESS-2"' "$MONCFG/mide/config.toml"; then
    echo "ok    Monitores grava a tela principal no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA Monitores grava a tela principal no config.toml"; FAIL=$((FAIL + 1))
fi
: > "$MONCFG/mide/config.toml"
# Sem confirmar, volta sozinho em 15 s (a barra volta para a primeira tela).
SCENARIO_WAIT=34 check "Monitores: sem confirmar, volta ao anterior em 15 s" 'camada "mide-bar" em \(0, 0\) 1366x28' "300 click 654 255; 330 click 404 526; 360 click 430 422; 1500 list" "$SETTINGS displays"
# Principal à direita da outra: a lâmpada (minimizar) vai até o ícone na dock dela,
# e não para a tela da esquerda. Janela em (1968, 76): minimizar em (2003, 61).
printf '[monitors]\nprimary = "HEADLESS-1"\n[monitors.outputs."HEADLESS-2"]\nx = 0\ny = 0\n[monitors.outputs."HEADLESS-1"]\nx = 1920\ny = 0\n' > "$MONCFG/mide/config.toml"
check "principal à direita: a lâmpada vai até a dock dela" "alvo no ícone da dock em Point.*x: 2511" "300 click 2003 61; 400 list" "$TERMINAL"
unset MIDE_HEADLESS_OUTPUTS
printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.25\n' > "$MONCFG/mide/config.toml"
MIDE_HEADLESS_OUTPUTS=1920x1080 check "escala 125%: tela de 1920x1080 com 1536x864 pontos" 'tela HEADLESS-1 em \(0, 0\) 1536x864' "300 list"
export XDG_CONFIG_HOME="$OLD_MONCFG"
rm -rf "$MONCFG"

echo "== mide-ui em todas as telas (exemplo kit-telas, telas virtuais)"
# examples/kit-telas (`Window::on_every_output`): uma camada escurecida em cada tela; o
# app registra no log os cliques e arrastos em coordenadas globais. HEADLESS-2 fica em
# (1366, 0), à direita da principal. Arrastar de uma tela para a outra prova a pega
# implícita do compositor (o ponteiro continua com a superfície onde o botão desceu).
KIT_TELAS="$PWD/target/debug/examples/kit-telas"
if [ -x "$KIT_TELAS" ]; then
    KT="$(mktemp -d)"
    mkdir -p "$KT/cfg/mide"
    # kit_telas "roteiro" "config.toml" "comando extra": roda o exemplo; log do app em
    # $KT/app.log, do compositor em $KT/scen.txt, uma captura de cada tela em $KT/t-*.png.
    kit_telas() {
        printf '%b' "$2" > "$KT/cfg/mide/config.toml"
        rm -f "$KT"/app.log "$KT"/t-*.png
        XDG_CONFIG_HOME="$KT/cfg" SCENARIO_OUT="$KT/scen" MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT="${KT_WAIT:-14}" timeout 150 "$SCEN" "$1" \
            -- "RUST_LOG=kit_telas=info,mide=info $KIT_TELAS > $KT/app.log 2>&1" "sh -c 'sleep 5; mide-ctl screenshot all $KT/t > /dev/null; ${3:-true}'" > "$KT/scen.txt" 2>&1
    }
    # kt_check "nome" "regex" [arquivo]: o log (do app, por padrão) casa.
    kt_check() {
        if grep -aEq "$2" "${3:-$KT/app.log}"; then
            echo "ok    $1"; PASS=$((PASS + 1))
        else
            echo "FALHA $1 (esperado: $2)"; FAIL=$((FAIL + 1))
        fi
    }
    # Sem o app: as telas como estão, para comparar o escurecido.
    XDG_CONFIG_HOME="$KT/cfg" SCENARIO_OUT="$KT/scen" MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT=10 timeout 150 "$SCEN" "400 list" \
        -- "sh -c 'sleep 5; mide-ctl screenshot all $KT/base > /dev/null'" >/dev/null 2>&1
    kit_telas "400 click 1600 300; 450 press 1300 300; 460 move 1400 300; 480 release 1500 300; 520 press 1600 300; 530 move 1400 300; 540 release 1266 300; 580 click 200 200; 620 list" ""
    kt_check "kit em todas as telas: camada na segunda tela" 'camada "kit-telas@HEADLESS-2" em \(1366, 0\) 1920x1080' "$KT/scen.txt"
    kt_check "kit em todas as telas: o passe da segunda tela sabe qual é" "desenhou a tela HEADLESS-2 em \(1366, 0\) 1920x1080 \(passe extra\)"
    kt_check "kit em todas as telas: clique na segunda tela em coordenadas globais" "clique global \(1600, 300\) na tela HEADLESS-2"
    kt_check "kit em todas as telas: arrasto da principal para a segunda" "arrasto global de \(1300, 300\) a \(1500, 300\), soltou na tela HEADLESS-2"
    kt_check "kit em todas as telas: arrasto da segunda para a principal" "arrasto global de \(1600, 300\) a \(1266, 300\), soltou na tela HEADLESS-1"
    kt_check "kit em todas as telas: clique na principal" "clique global \(200, 200\) na tela HEADLESS-1"
    if python3 -c "
import sys
from PIL import Image
d = sys.argv[1]
for n in ('HEADLESS-1', 'HEADLESS-2'):
    a = Image.open(d + '/base-' + n + '.png').convert('RGB').getpixel((700, 400))
    b = Image.open(d + '/t-' + n + '.png').convert('RGB').getpixel((700, 400))
    if not sum(b) < sum(a) * 0.8: sys.exit('%s não escureceu: %s %s' % (n, a, b))
" "$KT"; then
        echo "ok    kit em todas as telas: as duas telas escurecidas"; PASS=$((PASS + 1))
    else
        echo "FALHA kit em todas as telas: as duas telas escurecidas"; FAIL=$((FAIL + 1))
    fi
    # Principal à direita (HEADLESS-1 em (1920, 0), HEADLESS-2 em (0, 0)): a superfície
    # do app começa em 1920 e a outra tela fica à esquerda, com coordenadas menores.
    kit_telas "400 click 2000 200; 430 click 300 300; 450 press 1800 300; 460 move 1900 300; 480 release 2100 300; 520 press 2100 300; 530 move 1900 300; 540 release 1800 300; 600 key esc; 640 list" \
        '[monitors]\nprimary = "HEADLESS-1"\n[monitors.outputs."HEADLESS-2"]\nx = 0\ny = 0\n[monitors.outputs."HEADLESS-1"]\nx = 1920\ny = 0\n'
    kt_check "kit com a principal à direita: clique na principal" "clique global \(2000, 200\) na tela HEADLESS-1"
    kt_check "kit com a principal à direita: clique na outra tela" "clique global \(300, 300\) na tela HEADLESS-2"
    kt_check "kit com a principal à direita: arrasto da outra para a principal" "arrasto global de \(1800, 300\) a \(2100, 300\), soltou na tela HEADLESS-1"
    kt_check "kit com a principal à direita: arrasto da principal para a outra" "arrasto global de \(2100, 300\) a \(1800, 300\), soltou na tela HEADLESS-2"
    if grep -q 'autoteste: camada "kit-telas' "$KT/scen.txt"; then
        echo "FALHA kit em todas as telas: Esc fecha (teclado na camada principal)"; FAIL=$((FAIL + 1))
    else
        echo "ok    kit em todas as telas: Esc fecha (teclado na camada principal)"; PASS=$((PASS + 1))
    fi
    # Espelhando, as duas telas ocupam o mesmo lugar: uma camada só.
    kit_telas "400 list" '[monitors]\nmirror = true\n'
    if grep -aq "telas do app: HEADLESS-1 em (0, 0) 1366x768 principal" "$KT/app.log" && ! grep -aq "criada" "$KT/app.log"; then
        echo "ok    kit em todas as telas: espelhando, uma camada só"; PASS=$((PASS + 1))
    else
        echo "FALHA kit em todas as telas: espelhando, uma camada só"; FAIL=$((FAIL + 1))
    fi
    # A segunda tela é desligada com o app aberto e volta: a camada dela sai e volta, e
    # o app continua (só a camada principal fechada encerra o app).
    printf '[monitors.outputs."HEADLESS-2"]\nenabled = false\n' > "$KT/desligada.toml"
    : > "$KT/ligada.toml"
    KT_WAIT=20 kit_telas "900 click 1600 300; 1000 list" "" \
        "cp $KT/desligada.toml $KT/cfg/mide/config.toml; mide-ctl outputs apply > /dev/null; sleep 3; cp $KT/ligada.toml $KT/cfg/mide/config.toml; mide-ctl outputs apply > /dev/null"
    kt_check "kit em todas as telas: monitor desligado tira a camada dele" "tela HEADLESS-2 saiu: camada fechada"
    kt_check "kit em todas as telas: monitor ligado de novo ganha a camada" "clique global \(1600, 300\) na tela HEADLESS-2"
    kt_check "kit em todas as telas: o app continua aberto" 'camada "kit-telas@HEADLESS-2" em \(1366, 0\)' "$KT/scen.txt"
    # A principal muda com o app aberto: o compositor não muda a camada principal de
    # lugar (continua em HEADLESS-1, agora em (1920, 0)); a extra fica na HEADLESS-2,
    # que vai para (0, 0) e recebe o clique.
    printf '[monitors]\nprimary = "HEADLESS-2"\n' > "$KT/p2.toml"
    KT_WAIT=20 kit_telas "900 click 600 300; 1000 list" "" "cp $KT/p2.toml $KT/cfg/mide/config.toml; mide-ctl outputs apply > /dev/null"
    kt_check "kit com a principal trocada: a camada principal fica onde estava" 'camada "kit-telas" em \(1920, 0\) 1366x768' "$KT/scen.txt"
    kt_check "kit com a principal trocada: a nova principal tem a camada extra" 'camada "kit-telas@HEADLESS-2" em \(0, 0\) 1920x1080' "$KT/scen.txt"
    kt_check "kit com a principal trocada: clique na nova principal chega ao app" "clique global \(600, 300\) na tela HEADLESS-2"
    # A tela da camada principal é desligada com o app aberto: o app fecha (a camada
    # ficaria invisível segurando o teclado).
    printf '[monitors.outputs."HEADLESS-1"]\nenabled = false\n' > "$KT/d1.toml"
    rm -f "$KT/vivo"
    KT_WAIT=20 kit_telas "1000 list" "" "cp $KT/d1.toml $KT/cfg/mide/config.toml; mide-ctl outputs apply > /dev/null; sleep 4; pgrep -x kit-telas > /dev/null && touch $KT/vivo"
    kt_check "kit com a principal desligada: o app percebe" "a tela HEADLESS-1 da camada principal saiu"
    if [ -e "$KT/vivo" ] || grep -aq 'autoteste: camada "kit-telas' "$KT/scen.txt"; then
        echo "FALHA kit com a principal desligada: o app fecha"; FAIL=$((FAIL + 1))
    else
        echo "ok    kit com a principal desligada: o app fecha"; PASS=$((PASS + 1))
    fi
    rm -rf "$KT"
else
    echo "pulo  kit em todas as telas (rode cargo build --examples)"
fi

echo "== menu do app na barra (menu global, barramento D-Bus de teste)"
# App de teste (examples/mide-menu-teste) que publica "Projeto" e "Ajuda" pelo dbusmenu.
MENU_APP="$PWD/target/debug/examples/mide-menu-teste"
# "Projeto" na barra em (186, 14); no menu, "Compilar" é a primeira linha (y=49).
MENU_X=186
if [ -x "$MENU_APP" ]; then
    SCEN_PLAIN="$SCEN"
    SCEN=tests/scenario-dbus.sh
    RUST_LOG="mide=debug,mide_bar=info" check "a barra mostra os menus do app em foco" "menu do app mide-menu-teste: Projeto, Ajuda" "500 list" "$MENU_APP"
    CLICK="$(mktemp)"
    MENU_TESTE_ARQUIVO="$CLICK" timeout 150 "$SCEN" "500 click $MENU_X 14; 540 click $MENU_X 49; 580 list" -- "$MENU_APP" >/dev/null 2>&1
    if [ "$(cat "$CLICK")" = "11" ]; then
        echo "ok    clique no menu do app chega ao app (Projeto → Compilar)"; PASS=$((PASS + 1))
    else
        echo "FALHA clique no menu do app chega ao app (recebeu: '$(cat "$CLICK")')"; FAIL=$((FAIL + 1))
    fi
    rm -f "$CLICK"
    SCEN="$SCEN_PLAIN"
else
    echo "pulo  menu do app (rode cargo build --examples)"
fi

if [ "$(cfg_fingerprint)" != "$CFG_BEFORE" ]; then
    echo "FALHA a bateria alterou $REAL_CFG"
    FAIL=$((FAIL + 1))
else
    echo "ok    configuração real intacta"
    PASS=$((PASS + 1))
fi

echo "resultado: $PASS ok, $FAIL falha(s)"
[ "$FAIL" = 0 ]
