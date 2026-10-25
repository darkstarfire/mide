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

# Arrastar ícones: muda de lugar (guardado) e, solto numa pasta, vai para dentro dela.
DESK="$(mktemp -d)"; DSTATE="$(mktemp -d)"
echo oi >"$DESK/notas.txt"
MIDE_DESKTOP_DIR="$DESK" XDG_STATE_HOME="$DSTATE" timeout 150 "$SCEN" "300 press 1298 76; 306 move 1250 120; 312 move 1100 250; 318 move 950 320; 330 release 950 320; 380 list" >/dev/null 2>&1
if grep -q "notas.txt" "$DSTATE/mide/desktop-positions" 2>/dev/null && ! grep -q "^0	0	notas.txt" "$DSTATE/mide/desktop-positions"; then
    echo "ok    arrastar um ícone da área de trabalho muda o lugar dele"; PASS=$((PASS + 1))
else
    echo "FALHA arrastar um ícone da área de trabalho muda o lugar dele"; FAIL=$((FAIL + 1))
fi
rm -rf "$DSTATE"/* ; mkdir "$DESK/Pasta"
# A pasta fica no primeiro lugar (1298, 76) e o arquivo logo abaixo (1298, 182).
MIDE_DESKTOP_DIR="$DESK" XDG_STATE_HOME="$DSTATE" timeout 150 "$SCEN" "300 press 1298 182; 306 move 1290 160; 312 move 1295 100; 318 move 1298 80; 330 release 1298 80; 380 list" >/dev/null 2>&1
if [ -e "$DESK/Pasta/notas.txt" ] && [ ! -e "$DESK/notas.txt" ]; then
    echo "ok    ícone solto numa pasta da área de trabalho vai para dentro dela"; PASS=$((PASS + 1))
else
    echo "FALHA ícone solto numa pasta da área de trabalho vai para dentro dela"; FAIL=$((FAIL + 1))
fi
# Renomear com uma janela na frente: o diálogo vem para a frente e recebe o teclado.
rm -rf "$DESK"/* ; echo oi >"$DESK/x.txt"
MIDE_DESKTOP_DIR="$DESK" XDG_STATE_HOME="$DSTATE" timeout 150 "$SCEN" "300 click 680 380; 340 click 1298 76; 380 combo ctrl+r; 420 key n; 425 key o; 430 key v; 435 key a; 470 key enter; 520 list" -- mide-text >/dev/null 2>&1
if [ -e "$DESK/nova" ]; then
    echo "ok    renomear na área de trabalho com uma janela na frente"; PASS=$((PASS + 1))
else
    echo "FALHA renomear na área de trabalho com uma janela na frente"; FAIL=$((FAIL + 1))
fi
rm -rf "$DESK" "$DSTATE"
