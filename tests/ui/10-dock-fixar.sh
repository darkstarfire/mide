# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
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
# O pulo inteiro cabe na camada dele: a altura da dock (57) mais o pulo (40) e a
# folga (8), na mesma largura da dock (antes o topo do ícone era cortado).
XDG_DATA_DIRS="$PULO:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}" check "dock: o pulo cabe inteiro na camada dele" 'camada "mide-dock-pulo" em \([0-9]+, [0-9]+\) [0-9]+x105' "300 click 654 728; 330 list"
if [ -n "$OLD_XDG_CONFIG_HOME" ]; then export XDG_CONFIG_HOME="$OLD_XDG_CONFIG_HOME"; else unset XDG_CONFIG_HOME; fi
rm -rf "$DOCKCFG"
