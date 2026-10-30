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
# Monitor externo (HEADLESS-2 em (1366, 0)): a mesa principal abre a mesa dele, sem
# ícones e com o mesmo menu do fundo; a pasta criada lá vai para a área de trabalho.
reset_desk
DSTATE="$(mktemp -d)"
MIDE_DESKTOP_DIR="$DESK" XDG_STATE_HOME="$DSTATE" MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 timeout 150 "$SCEN" "400 rclick 1766 300; 430 click 1816 326; 480 list" >"$DSTATE/scen.txt" 2>&1
if grep -aq 'camada "mide-desktop@HEADLESS-2" em (1366, 0)' "$DSTATE/scen.txt"; then
    echo "ok    área de trabalho no monitor externo"; PASS=$((PASS + 1))
else
    echo "FALHA área de trabalho no monitor externo"; FAIL=$((FAIL + 1))
fi
if [ -d "$DESK/Nova pasta" ] && ! grep -q "Nova pasta" "$DSTATE/mide/desktop-positions" 2>/dev/null; then
    echo "ok    menu da área de trabalho no monitor externo → Nova pasta"; PASS=$((PASS + 1))
else
    echo "FALHA menu da área de trabalho no monitor externo → Nova pasta"; FAIL=$((FAIL + 1))
fi
rm -rf "$DESK" "$DSTATE"

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
# Atalho .desktop com ícone XPM (como o do Eclipse, `Icon=.../icon.xpm`): a mesa mostra
# o ícone do app e o arrasto leva o próprio ícone (ele sai do lugar), junto do ponteiro. A pasta da mesa fica
# dentro de `applications/` (atalho confiável sem perguntar).
EDATA="$(mktemp -d)"; EDESK="$EDATA/applications"; mkdir -p "$EDESK"
python3 -c "
rows = [''.join('.' if 4 <= x < 60 and 4 <= y < 60 else ' ' for x in range(64)) for y in range(64)]
print('/* XPM */')
print('static char * icon_xpm[] = {')
print('\"64 64 2 1\",')
print('\"  c None\",')
print('\". c #F7941E\",')
print(',\n'.join('\"%s\"' % r for r in rows) + '};')" >"$EDATA/icon.xpm"
printf '[Desktop Entry]\nType=Application\nName=Eclipse de teste\nExec=true\nIcon=%s\n' "$EDATA/icon.xpm" >"$EDESK/eclipse.desktop"
ELOG="$(MIDE_DESKTOP_DIR="$EDESK" XDG_STATE_HOME="$DSTATE" XDG_DATA_HOME="$EDATA" XDG_CACHE_HOME="$EDATA/cache" SCENARIO_OUT="$EDATA/out" timeout 150 "$SCEN" \
    "300 press 1298 76; 306 move 1280 100; 312 move 1100 200; 318 move 900 300; 324 move 700 400; 330 move 701 401; 420 release 701 401; 460 list" 380 2>&1)"
if echo "$ELOG" | grep -q "arrasto entre apps começou (ícone: true)" && ls "$EDATA"/cache/mide/icons/xpm-*.png >/dev/null 2>&1 \
    && python3 -c "
import sys
from PIL import Image
img = Image.open(sys.argv[1]).convert('RGB')
near = lambda p: abs(p[0] - 0xF7) < 40 and abs(p[1] - 0x94) < 40 and abs(p[2] - 0x1E) < 50
# Ícone do arrasto junto do ponteiro (701, 401); no lugar dele na mesa (1298, 76), nada.
drag = sum(near(img.getpixel((x, y))) for x in range(661, 742, 4) for y in range(361, 442, 4))
desk = sum(near(img.getpixel((x, y))) for x in range(1270, 1327, 4) for y in range(50, 103, 4))
sys.exit(0 if drag >= 40 and desk <= 4 else 'arrasto %d, mesa %d' % (drag, desk))" "$EDATA/out/frame.png"; then
    echo "ok    atalho com ícone XPM: o próprio ícone do app sai do lugar e segue o ponteiro"; PASS=$((PASS + 1))
else
    echo "FALHA atalho com ícone XPM: o próprio ícone do app sai do lugar e segue o ponteiro"; FAIL=$((FAIL + 1))
    echo "$ELOG" | grep -a "arrasto\|atalho" | sed 's/^/      /' | tail -4
fi
# Mesa à mostra (clique no vazio afasta as janelas): escolher um ícone e arrastar um
# retângulo de seleção não traz as janelas de volta; um clique simples no vazio traz.
rm -rf "$DESK"/* ; for n in a b c; do echo x >"$DESK/$n.txt"; done
MIDE_DESKTOP_DIR="$DESK" XDG_STATE_HOME="$DSTATE" check "mesa à mostra: seleção por retângulo não traz as janelas de volta" 'janela "Terminal" popups=0 posição=\(-644, 76\)' "400 click 900 600; 440 click 1298 76; 480 press 1000 600; 485 move 1100 500; 490 move 1250 300; 495 move 1330 100; 500 release 1330 100; 540 list" "$TERMINAL"
MIDE_DESKTOP_DIR="$DESK" XDG_STATE_HOME="$DSTATE" check "mesa à mostra: clique simples no vazio traz as janelas de volta" 'janela "Terminal" popups=0 posição=\(48, 76\)' "400 click 900 600; 460 click 1000 600; 520 list" "$TERMINAL"
rm -rf "$DESK" "$DSTATE" "$EDATA"
