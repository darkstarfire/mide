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
check "Ctrl+O abre o ícone escolhido" "abrir \(teste\): .*notas.txt" "300 click 1298 76; 340 combo ctrl+o; 380 list"
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
# Enter renomeia no lugar, como no Finder (o nome sem a extensão vem escolhido),
# com uma janela na frente: a camada sobe e recebe o teclado.
rm -rf "$DESK"/* ; echo oi >"$DESK/x.txt"
MIDE_DESKTOP_DIR="$DESK" XDG_STATE_HOME="$DSTATE" timeout 150 "$SCEN" "300 click 680 380; 340 click 1298 76; 380 key enter; 420 key n; 425 key o; 430 key v; 435 key a; 470 key enter; 520 list" -- mide-writer >/dev/null 2>&1
if [ -e "$DESK/nova.txt" ]; then
    echo "ok    Enter renomeia no lugar (sem a extensão) com uma janela na frente"; PASS=$((PASS + 1))
else
    echo "FALHA Enter renomeia no lugar (sem a extensão) com uma janela na frente"; FAIL=$((FAIL + 1))
fi
# Arrastar o ícone para dentro do Writer abre o arquivo nele (o ícone vai
# por cima da janela, pelo arrasto do Wayland).
rm -rf "$DESK"/* ; echo "texto" >"$DESK/arrastado.txt"
MIDE_DESKTOP_DIR="$DESK" XDG_STATE_HOME="$DSTATE" check "arrastar o ícone para o Writer abre o arquivo" 'janela "arrastado.txt — Writer"' "300 press 1298 76; 306 move 1280 100; 312 move 1100 200; 318 move 800 350; 324 move 700 400; 340 release 700 400; 420 list" mide-writer
# Pasta de mola: segurar o arrasto em cima de uma pasta abre a pasta no Arquivos.
rm -rf "$DESK"/* ; mkdir "$DESK/Pasta"; echo x >"$DESK/notas.txt"
MIDE_DESKTOP_DIR="$DESK" XDG_STATE_HOME="$DSTATE" check "segurar o arrasto em cima de uma pasta abre a pasta (pasta de mola)" "abrir \(teste\): .*/Pasta$" "300 press 1298 182; 306 move 1290 160; 312 move 1295 100; 318 move 1298 80; 330 move 1299 78; 400 move 1298 79; 470 move 1299 79; 540 release 900 400; 580 list"
rm -rf "$DESK" "$DSTATE"
