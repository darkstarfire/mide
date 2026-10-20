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
# Menus padrão do mide (a Calculadora não publica menus): "Editar" em x=234, "Janela" em 287.
check "menu Editar abre pela barra" "menu aberto: Edit" "400 click 234 14; 440 list" "$CALCULATOR"
check "Janela > Zoom maximiza" "$CALC popups=0 posição=\(0, 58\) visível=1366x643" "400 click 287 14; 430 click 299 73; 480 list" "$CALCULATOR"
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
check "menu do mide: Repouso (simulado)" "sistema \(teste\): systemctl suspend" "400 click 25 14; 430 click 100 153; 460 list"
check "menu do mide: Reiniciar pede confirmação" "menu aberto: ConfirmRestart" "400 click 25 14; 430 click 100 177; 460 list"
check "confirmar reinicia (simulado)" "sistema \(teste\): systemctl reboot" "400 click 25 14; 430 click 100 177; 460 click 753 412; 500 list"
CLOG="$(timeout 150 "$SCEN" "400 click 25 14; 430 click 100 177; 460 click 614 412; 500 list" 2>&1)"
if echo "$CLOG" | grep -q "menu aberto: ConfirmRestart" && ! echo "$CLOG" | grep -q "systemctl reboot"; then
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
DRAG="460 press 179 210; 465 move 200 210; 470 move 260 210; 475 move 323 210; 480 release 323 210"
check "arrastar app sobre app cria uma pasta" "pasta .*\(2 apps\)" "$LAUNCH; $DRAG; 520 list"
check "pasta fica salva" "em [0-9]+ lugar\(es\) e 4 pasta" "$LAUNCH; 460 list"
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
SCENARIO_WAIT=24 timeout 150 "$SCEN" "400 click 756 728; 440 rclick 756 563; 480 click 821 662; 1000 click 400 300; 1020 combo ctrl+v; 1050 combo ctrl+s; 1100 list" -- "sleep 12; $PWD/target/debug/mide-text '$CPY'" >/dev/null 2>&1
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
# Janela em (48, 76): miniatura "Beach" em (548, 185); interruptor de Sombras em (807, 169).
timeout 150 "$SCEN" "400 click 548 185; 460 list" -- "$SETTINGS wallpaper" >/dev/null 2>&1
if grep -q 'wallpaper = "/usr/share/backgrounds/Beach' "$SCFG/mide/config.toml" && grep -q "# terminal" "$SCFG/mide/config.toml"; then
    echo "ok    escolher papel de parede grava (e mantém os comentários)"; PASS=$((PASS + 1))
else
    echo "FALHA escolher papel de parede grava"; FAIL=$((FAIL + 1))
fi
check "compositor usa o papel de parede gravado" "papel de parede: /usr/share/backgrounds/Beach" "400 list; 420 list" "$SETTINGS wallpaper"
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
timeout 150 "$SCEN" "400 click 158 104; 420 type teclas; 460 click 500 158; 500 click 807 169; 560 list" -- "$SETTINGS" >/dev/null 2>&1
if grep -q "mac_style = true" "$SCFG/mide/config.toml"; then
    echo "ok    busca nas Configurações leva à opção (teclas no estilo Mac gravado)"; PASS=$((PASS + 1))
else
    echo "FALHA busca nas Configurações leva à opção (teclas no estilo Mac gravado)"; FAIL=$((FAIL + 1))
fi
sed -i '/mac_style = true/d' "$SCFG/mide/config.toml"
# Teclado → Atalhos (A9): busca em (600, 224); buscando "fechar", "Fechar a janela"
# (Ctrl ⇧ Q) fica em (500, 340) e "Fechar a janela" (Alt F4) em (500, 298); no
# diálogo, "Gravar…" em (525, 315). O compositor grava a combinação apertada.
timeout 150 "$SCEN" "400 click 600 224; 410 type fechar; 440 click 500 340; 470 click 525 315; 500 combo ctrl+alt+k; 560 list" -- "$SETTINGS keyboard" >/dev/null 2>&1
if grep -q '"ctrl+alt+k" = "close_window"' "$SCFG/mide/config.toml" && grep -q '"ctrl+shift+q" = "none"' "$SCFG/mide/config.toml"; then
    echo "ok    gravar atalho nas Configurações troca a combinação"; PASS=$((PASS + 1))
else
    echo "FALHA gravar atalho nas Configurações troca a combinação"; FAIL=$((FAIL + 1))
fi
check "atalho gravado vale no compositor" "fechar janela pedido pelo atalho" "400 combo ctrl+alt+k; 460 list" "$SETTINGS keyboard"
check_count "combinação antiga desligada (a janela continua aberta)" 'janela "Configurações do Sistema"' 1 "400 combo ctrl+shift+q; 460 list" "$SETTINGS keyboard"
# Combinação já usada (Ctrl+Espaço é do buscador): pede confirmação, não grava nem abre o buscador.
check_count "gravando, o atalho não dispara" 'camada "mide-search"' 0 "400 click 600 224; 410 type fechar; 440 click 500 298; 470 click 525 315; 500 combo ctrl+space; 560 list" "$SETTINGS keyboard"
if ! grep -q '"ctrl+space" = "close_window"' "$SCFG/mide/config.toml"; then
    echo "ok    combinação de outra ação pede confirmação"; PASS=$((PASS + 1))
else
    echo "FALHA combinação de outra ação pede confirmação"; FAIL=$((FAIL + 1))
fi
# "Restaurar os padrões" (buscando algo sem resultado, a lista some) em (510, 292): a
# seção de atalhos sai do arquivo.
timeout 150 "$SCEN" "400 click 600 224; 410 type zzz; 440 click 510 292; 500 list" -- "$SETTINGS keyboard" >/dev/null 2>&1
if ! grep -q "keybindings" "$SCFG/mide/config.toml"; then
    echo "ok    restaurar os atalhos padrão"; PASS=$((PASS + 1))
else
    echo "FALHA restaurar os atalhos padrão"; FAIL=$((FAIL + 1))
fi
export XDG_CONFIG_HOME="$OLD_SCFG"
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
if XDG_CONFIG_HOME="$MCFG" timeout 150 "$SCEN" "400 combo super+space; 460 list" 2>&1 | grep -q 'camada "mide-search"'; then
    echo "ok    teclas no estilo Mac: Command+Espaço abre o buscador"; PASS=$((PASS + 1))
else
    echo "FALHA teclas no estilo Mac: Command+Espaço abre o buscador"; FAIL=$((FAIL + 1))
fi
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
file_check "lista + menu do item → Mover para a Lixeira" '[ ! -e "$FDIR/notas.txt" ] && [ -e "$TRASH/notas.txt" ]' "400 click 694 106; 430 rclick 500 293; 460 click 550 515; 500 list"
file_check "menu do item → Renomear" '[ -e "$FDIR/imagens" ] && [ ! -e "$FDIR/Fotos" ]' "400 rclick 463 190; 430 click 515 410; 470 key backspace; 471 key backspace; 472 key backspace; 473 key backspace; 474 key backspace; 480 type imagens; 490 key enter; 530 list"
file_check "clique fora fecha o menu sem fazer nada" '[ -e "$FDIR/relatorio.pdf" ] && [ -e "$FDIR/notas.txt" ]' "400 rclick 581 310; 430 click 700 470; 460 click 630 281; 500 list"
# Lista: setinha de "Fotos" em (303, 215) abre a pasta na própria lista; o item de
# dentro aparece na linha seguinte (y=241).
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
check "editor: alteração não salva marca o título" 'janela "• teste.rs' "400 type x; 460 list" "$TEXT_APP '$TDIR/teste.rs'"
check "fechar com alteração pergunta antes (janela continua)" 'janela "• teste.rs' "400 type x; 430 click 63 61; 480 list" "$TEXT_APP '$TDIR/teste.rs'"
rm -rf "$TDIR"

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
rm -f "$TESTCFG/user-dirs.dirs"
rm -rf "$SHOTS"

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
