# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
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
# À mostra de verdade: no lugar de sempre (y=703), não só sem o aviso de escondida.
if echo "$HLOG" | grep -Eq 'camada "mide-dock" em \([0-9]+, 703\)' && ! echo "$HLOG" | grep -q "dock escondida"; then
    echo "ok    ocultar inteligente: sem janela por cima, a dock fica à mostra"; PASS=$((PASS + 1))
else
    echo "FALHA ocultar inteligente: sem janela por cima, a dock fica à mostra"; FAIL=$((FAIL + 1))
fi
# Em 1366x768 a janela nova não chega na dock: maximizada, ela cobre a dock. O Terminal
# é de conteúdo inteiro: o botão verde fica dentro dela, em (109, 95) (seção janela).
if XDG_CONFIG_HOME="$HCFG" timeout 150 "$SCEN" "400 click 109 95; 700 list" -- "$TERMINAL" 2>&1 | grep -q "dock escondida"; then
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
# (218, 14) na barra; "Abrir" em (215, 55) chega ao app; o app tem id próprio e
# NON_UNIQUE. Barramento de teste sem serviços ativáveis: no barramento comum do
# dbus-run-session, o GDK 3 no Wayland pede o portal de configurações
# (org.freedesktop.portal.Desktop) ao abrir, o dbus tenta ativar o xdg-desktop-portal,
# que não sobe ali, e o app fica 25 s parado ("Settings portal not found: Timeout was
# reached") sem mostrar janela até o cenário acabar. Sem nada para ativar, o pedido
# falha na hora e o app segue.
if python3 -c "import gi; gi.require_version('Gtk', '3.0'); from gi.repository import Gtk" 2>/dev/null; then
    GTKM="$(mktemp -d)"
    mkdir -p "$GTKM/services"
    cat >"$GTKM/bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><servicedir>$GTKM/services</servicedir>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>
EOF
    MIDE_PRIVATE_BUS=1 SCENARIO_OUT="$GTKM/scen" SCENARIO_WAIT=12 timeout 100 dbus-run-session --config-file="$GTKM/bus.conf" -- tests/scenario.sh "450 click 218 14; 490 click 215 55; 530 list" -- "GTK_TESTE_ARQUIVO=$GTKM/acao.txt python3 $PWD/tests/apps/gtk-menu-teste.py" >/dev/null 2>&1
    if grep -q "menu do app gtk-menu-teste.py: Arquivo, Exibir, Ajuda" "$GTKM/scen/mide.log" && [ "$(cat "$GTKM/acao.txt" 2>/dev/null)" = "abrir" ]; then
        echo "ok    menu global: app GTK no Wayland (menus na barra, ação chega ao app)"; PASS=$((PASS + 1))
    else
        echo "FALHA menu global: app GTK no Wayland"; FAIL=$((FAIL + 1))
    fi
    rm -rf "$GTKM"
else
    echo "pulo  menu global GTK (sem python3-gi)"
fi
