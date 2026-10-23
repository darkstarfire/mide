# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
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
