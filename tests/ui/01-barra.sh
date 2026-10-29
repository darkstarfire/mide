# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== barra superior"
check "barra no topo, largura total" 'camada "mide-bar" em \(0, 0\) 1366x28' "400 list" "$TERMINAL"
check "janela nova nasce abaixo da barra" "$FOOT posição=\(48, 76\)" "400 list" "$TERMINAL"
check "menu abre ao clicar na marca" 'camada "mide-bar-menu"' "400 click 25 14; 430 list"
check_count "menu fecha ao clicar fora" 'camada "mide-bar-menu"' 0 "400 click 25 14; 430 click 600 400; 460 list"
# O Terminal desenha a própria barra de título (visual do Tahoe): arrasta por ela, em y=90,
# e solta abaixo da borda da tela (lá em cima vira o encaixe pelo topo).
check "arrastar para cima para na barra" "$FOOT posição=\(48, 28\)" "400 press 400 90; 405 move 400 60; 410 move 400 36; 415 release 400 36; 440 list" "$TERMINAL"
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
sys.exit(0 if im.getpixel((500, 500)) != (255, 255, 255) and im.getpixel((150, 300)) == (30, 30, 32) else 1)" "$OPC/scen/frame.png" 2>/dev/null; then
    echo "ok    janela sem foco translúcida ([opacity] inactive), a com foco opaca"; PASS=$((PASS + 1))
else
    echo "FALHA janela sem foco translúcida ([opacity] inactive)"; FAIL=$((FAIL + 1))
fi
rm -rf "$OPC"
# Clique numa janela por cima da barra de título de outra (Calculadora em (48, 46),
# Terminal arrastado para cobrir a barra dela): o clique é do Terminal, a de trás não sobe.
check_last "clique não atravessa para a barra de título de trás" 'janela "Terminal"' "400 press 400 93; 405 move 400 60; 410 move 400 20; 415 release 400 20; 460 click 200 68; 490 list" "$CALCULATOR" "$TERMINAL"
# Menus do Arquivos (publicados por ele, num barramento de teste): Arquivo, Editar em
# x=213, Visualizar, Ir, Janela em x=375; no menu, Zoom é a segunda linha (y=79).
FILES_APP="$PWD/target/debug/mide-files"
SCEN=tests/scenario-dbus.sh check "menu Editar abre pela barra" "menu aberto: " "500 click 213 14; 540 list" "$FILES_APP"
SCEN=tests/scenario-dbus.sh check "Janela > Zoom maximiza" 'janela "Pasta pessoal" popups=0 posição=\(0, 28\) visível=1366x673' "500 click 375 14; 540 click 390 79; 600 list" "$FILES_APP"
# Menus do próprio Terminal (dbusmenu, num barramento de teste): Shell em x=140,
# Editar 194, Janela 382; no menu, a primeira linha em y=55 e a segunda em y=79.
RUST_LOG="mide=debug,mide_bar=info" SCEN=tests/scenario-dbus.sh check "a barra mostra os menus do Terminal" "menu do app mide-terminal: Shell, Editar, Visualizar, Perfis, Janela" "500 list" "$TERMINAL"
SCEN=tests/scenario-dbus.sh check "Terminal: Janela > Zoom pelo menu do app" "$FOOT posição=\(0, 28\) visível=1366x673" "500 click 382 14; 540 click 400 79; 600 list" "$TERMINAL"
check "app > Encerrar fecha o app" "janela fechada" "400 click 65 14; 430 click 90 82" "$TERMINAL"
# "Sobre este computador" tem tamanho fixo (330x610): abre no meio da tela (A26) e não
# muda de tamanho puxando o canto.
check "janela Sobre abre no meio e não redimensiona" 'janela "Sobre este computador" popups=0 posição=\(518, 59\) visível=330x610' "400 click 25 14; 430 click 100 49; 520 list; 530 press 846 667; 535 move 866 687; 540 move 886 697; 545 release 886 697; 580 list"
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
# Painel de som (x 1038..1318): saídas de verdade da placa simulada (SOF com dois
# perfis): "Alto-falantes" em y=200 e o monitor "DELL U2717D" (HDMI 1) em y=236; com
# um fone ligado (MIDE_SYSTEM_DRY_RUN_HEADPHONES), "Fones de ouvido" vem antes, em
# y=200. Entrada: "Microfone estéreo" em y=341.
check "som: monitor HDMI pelo nome vira a saída (simulado)" "sistema \(teste\): pactl set-default-sink [^ ]*HiFi__HDMI1__sink" "400 click 1185 14; 440 click 1150 236; 470 list"
MIDE_SYSTEM_DRY_RUN_HEADPHONES=1 check "som: fone ligado aparece e escolher troca o perfil da placa (simulado)" "sistema \(teste\): pactl set-card-profile alsa_card[^ ]* HiFi \(HDMI1, HDMI2, HDMI3, Headphones" "400 click 1185 14; 440 click 1150 200; 470 list"
check "som: escolher o microfone (simulado)" "sistema \(teste\): pactl set-default-source [^ ]*Mic2__source" "400 click 1185 14; 440 click 1150 341; 470 list"
# Rede cabeada simulada como principal (MIDE_SYSTEM_DRY_RUN_WIRED): ícone do cabo no
# lugar do Wi-Fi; no painel, "Configurações de rede…" em y=265.
MIDE_SYSTEM_DRY_RUN_WIRED=1 check "rede: cabo principal mostra o ícone da rede cabeada" "rede: ícone da barra = cabo" "400 list"
MIDE_SYSTEM_DRY_RUN_WIRED=1 check "rede: painel com a Ethernet abre Configurações → Rede" "sistema \(teste\): mide-settings network" "400 click 1221 14; 440 click 1150 265; 470 list"
check "Bluetooth: clique desconecta o fone (simulado)" "sistema \(teste\): bluetoothctl disconnect" "400 click 1257 14; 440 click 1216 99; 470 list"
check "Sobre minimizada (Super+H) volta" "janela restaurada" "400 click 25 14; 430 click 100 49; 520 combo super+h; 600 restore; 700 list"
check "Sobre este computador abre" 'janela "Sobre este computador"' "400 click 25 14; 430 click 100 49; 520 list"
# Menu da bateria com tempo restante, consumo, saúde e ciclos (bateria falsa do kernel).
BATDIR="$(mktemp -d)"; mkdir -p "$BATDIR/BAT0"
for f in "type Battery" "capacity 75" "status Discharging" "energy_now 30000000" "energy_full 40000000" "energy_full_design 50000000" "power_now 12000000" "cycle_count 213"; do set -- $f; echo "$2" > "$BATDIR/BAT0/$1"; done
MIDE_POWER_SUPPLY_DIR="$BATDIR" check "bateria: menu com tempo, consumo, saúde e ciclos" "menu da bateria: .* \| 2 h 30 min .* \| .*12[,.]0 W \| .*80% \| .*213" "500 click 1222 14; 560 list"
rm -rf "$BATDIR"
