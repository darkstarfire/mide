# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== miniatura da captura e editor de capturas (B23)"
# Imagens numa pasta temporária (nada em ~/Imagens). A miniatura de uma área de
# 300x200 tem 180x120 pontos e fica na camada "mide-captura-miniatura" de 280x300 no
# canto de baixo à direita, acima da dock: (1086, 401) na tela de 1366x768, a
# miniatura em (1170, 565)–(1350, 685).
MIN="$(mktemp -d)"
mkdir -p "$MIN/cfg"
printf 'XDG_PICTURES_DIR="%s"\n' "$MIN/pics" > "$MIN/cfg/user-dirs.dirs"
AREA4="400 combo super+shift+4; 600 press 300 200; 610 move 450 300; 620 move 600 400; 625 release 600 400"
min_log() { sed 's/\x1b\[[0-9;]*m//g' "$MIN/$1/mide.log" 2>/dev/null; }

# ⌘⇧4: a captura é salva, a miniatura aparece no canto e some sozinha em 5 s.
XDG_CONFIG_HOME="$MIN/cfg" SCENARIO_OUT="$MIN/some" SCENARIO_WAIT=18 timeout 150 "$SCEN" "$AREA4; 700 list" 700 >/dev/null 2>&1
LOG="$(min_log some)"
T_IN="$(echo "$LOG" | grep -m1 "miniatura: apareceu" | cut -c12-23)"
T_OUT="$(echo "$LOG" | grep -m1 "miniatura: some sozinha" | cut -c12-23)"
if echo "$LOG" | grep -q 'autoteste: camada "mide-captura-miniatura" em (1086, 401) 280x300' && [ -n "$T_IN" ] && [ -n "$T_OUT" ] \
    && python3 -c "
import sys
f = lambda s: sum(float(x) * m for x, m in zip(s.split(':'), (3600, 60, 1)))
d = f(sys.argv[2]) - f(sys.argv[1])
sys.exit(0 if 4.8 <= d <= 5.8 else 1)" "$T_IN" "$T_OUT" && echo "$LOG" | grep -q "miniatura: fechada (Timeout)" \
    && python3 -c "
import sys
from PIL import Image
img = Image.open(sys.argv[1] + '/some/frame.png').convert('RGB')
# A moldura branca da miniatura (em 1172, 600) e a imagem dentro dela.
r, g, b = img.getpixel((1172, 600))
sys.exit(0 if min(r, g, b) > 235 else 1)" "$MIN"; then
    echo "ok    miniatura: aparece no canto depois do ⌘⇧4 e some sozinha em 5 s"; PASS=$((PASS + 1))
else
    echo "FALHA miniatura: aparece no canto depois do ⌘⇧4 e some sozinha em 5 s (entrou $T_IN, saiu $T_OUT)"; FAIL=$((FAIL + 1))
fi
# A captura continua salva e copiada como antes.
if ls "$MIN/pics/Capturas de tela/"*.png >/dev/null 2>&1 && echo "$LOG" | grep -q "captura copiada (300x200)"; then
    echo "ok    miniatura: a captura continua salva e copiada"; PASS=$((PASS + 1))
else
    echo "FALHA miniatura: a captura continua salva e copiada"; FAIL=$((FAIL + 1))
fi
rm -rf "$MIN/pics"

# Clique na miniatura: abre o editor com a captura (e a miniatura sai).
XDG_CONFIG_HOME="$MIN/cfg" SCENARIO_OUT="$MIN/clique" SCENARIO_WAIT=16 timeout 150 "$SCEN" "$AREA4; 700 click 1260 625; 900 list" >/dev/null 2>&1
LOG="$(min_log clique)"
if echo "$LOG" | grep -q "miniatura: abrindo no editor" && echo "$LOG" | grep -q "editor da captura aberto: .*Capturas de tela/Captura de tela .*(300x200)" \
    && echo "$LOG" | grep -q 'autoteste: janela "Captura de tela .* — Editor de Capturas"' && echo "$LOG" | grep -q "miniatura: fechada (Editor)"; then
    echo "ok    miniatura: o clique abre a captura no editor"; PASS=$((PASS + 1))
else
    echo "FALHA miniatura: o clique abre a captura no editor"; FAIL=$((FAIL + 1))
fi
rm -rf "$MIN/pics"

# Clique direito: o menu; "Mover para a Lixeira" (o último item, em cima da miniatura)
# manda a captura para a Lixeira do teste.
XDG_CONFIG_HOME="$MIN/cfg" SCENARIO_OUT="$MIN/menu" SCENARIO_WAIT=16 timeout 150 "$SCEN" "$AREA4; 700 rclick 1260 625; 760 list" 760 >/dev/null 2>&1
MENU_FRAME="$MIN/menu/frame.png"
rm -rf "$MIN/pics"
XDG_CONFIG_HOME="$MIN/cfg" SCENARIO_OUT="$MIN/lixeira" SCENARIO_WAIT=16 timeout 150 "$SCEN" "$AREA4; 700 rclick 1260 625; 760 click 1216 538; 900 list" >/dev/null 2>&1
LOG="$(min_log lixeira)"
if echo "$LOG" | grep -q "miniatura: movida para a Lixeira" && ls "$MIN/lixeira/data/Trash/files/Captura de tela "*.png >/dev/null 2>&1 \
    && ! ls "$MIN/pics/Capturas de tela/"*.png >/dev/null 2>&1; then
    echo "ok    miniatura: menu com Mover para a Lixeira (a captura vai para a Lixeira)"; PASS=$((PASS + 1))
else
    echo "FALHA miniatura: menu com Mover para a Lixeira ($(echo "$LOG" | grep -c miniatura) linhas; quadro do menu em $MENU_FRAME)"; FAIL=$((FAIL + 1))
fi
rm -rf "$MIN/pics"

# Arrastar a miniatura para a direita: some na hora; o arquivo fica.
XDG_CONFIG_HOME="$MIN/cfg" SCENARIO_OUT="$MIN/arrasto" SCENARIO_WAIT=14 timeout 150 "$SCEN" "$AREA4; 700 press 1260 625; 705 move 1290 625; 710 move 1330 625; 715 move 1360 625; 720 release 1360 625; 800 list" >/dev/null 2>&1
LOG="$(min_log arrasto)"
if echo "$LOG" | grep -q "miniatura: descartada" && ! echo "$LOG" | grep -q "miniatura: some sozinha" && ls "$MIN/pics/Capturas de tela/"*.png >/dev/null 2>&1; then
    echo "ok    miniatura: arrastada para fora some na hora (a captura fica salva)"; PASS=$((PASS + 1))
else
    echo "FALHA miniatura: arrastada para fora some na hora"; FAIL=$((FAIL + 1))
fi
rm -rf "$MIN/pics"
# Apertada perto da borda direita (x=1340, a 26 pontos da borda da tela): não dá para
# arrastar 40 pontos, e chegar à borda basta para descartar.
XDG_CONFIG_HOME="$MIN/cfg" SCENARIO_OUT="$MIN/borda" SCENARIO_WAIT=14 timeout 150 "$SCEN" "$AREA4; 700 press 1340 625; 705 move 1352 625; 710 move 1365 625; 720 release 1365 625; 800 list" >/dev/null 2>&1
LOG="$(min_log borda)"
if echo "$LOG" | grep -q "miniatura: descartada" && ! echo "$LOG" | grep -q "miniatura: abrindo no editor" && ls "$MIN/pics/Capturas de tela/"*.png >/dev/null 2>&1; then
    echo "ok    miniatura: arrastada até a borda da tela some (apertada perto da borda)"; PASS=$((PASS + 1))
else
    echo "FALHA miniatura: arrastada até a borda da tela some ($(echo "$LOG" | grep -m1 'miniatura: \(descartada\|fechada\|some\)'))"; FAIL=$((FAIL + 1))
fi
rm -rf "$MIN/pics"

# Editor (o log do app vai para um arquivo: o scenario.sh descarta a saída dos apps
# que abre): uma imagem de 400x300 (metade esquerda azul, direita branca) abre em
# (318, 247) na janela de 940x600 em (48, 76) (a imagem cabe sem reduzir).
ED="$MIN/ed"
mkdir -p "$ED"
make_png() {
    python3 -c "
import sys
from PIL import Image
img = Image.new('RGB', (400, 300), (255, 255, 255))
for x in range(200):
    for y in range(300):
        img.putpixel((x, y), (10, 80, 200))
img.save(sys.argv[1])" "$1"
}
EDITOR="$PWD/target/debug/mide-screenshot-editor"
# Recortar (R) arrastando a bolinha de cima à esquerda de (348, 267) para (448, 317):
# fica a área de (100, 50) a (400, 300); Enter recorta e Ctrl+S substitui o arquivo.
make_png "$ED/recorte.png"
SCENARIO_OUT="$MIN/recorte" timeout 150 "$SCEN" "400 key r; 450 press 348 267; 460 move 390 290; 470 move 448 317; 480 release 448 317; 520 key enter; 560 combo ctrl+s; 620 list" -- "$EDITOR '$ED/recorte.png' >'$ED/recorte.log' 2>&1" >/dev/null 2>&1
LOG="$(sed 's/\x1b\[[0-9;]*m//g' "$ED/recorte.log")"
if echo "$LOG" | grep -q "editor: imagem em (300, 191) 400x300" && echo "$LOG" | grep -q "editor: recortado em 300x250 a partir de (100, 50)" && python3 -c "
import sys
from PIL import Image
img = Image.open(sys.argv[1]).convert('RGB')
ok = img.size == (300, 250) and img.getpixel((50, 100)) == (10, 80, 200) and img.getpixel((150, 100)) == (255, 255, 255)
sys.exit(0 if ok else 'tamanho %s' % (img.size,))" "$ED/recorte.png"; then
    echo "ok    editor: recortar com as bolinhas e salvar (o PNG fica com 300x250, do lugar certo)"; PASS=$((PASS + 1))
else
    echo "FALHA editor: recortar e salvar ($(echo "$LOG" | grep -m1 'editor: imagem em'))"; FAIL=$((FAIL + 1))
fi
# Lápis (L, hoje o Esboço: a reta vira linha) atravessando a imagem na altura y=420 (linha 153 da imagem): os pixels
# ficam vermelhos ali e só ali; Ctrl+Z desfaz e Ctrl+Shift+Z refaz antes de salvar.
make_png "$ED/lapis.png"
SCENARIO_OUT="$MIN/lapis" timeout 150 "$SCEN" "400 key l; 450 press 380 420; 460 move 480 420; 470 move 580 420; 480 move 680 420; 490 release 680 420; 520 combo ctrl+z; 540 combo ctrl+shift+z; 560 combo ctrl+s; 620 list" -- "$EDITOR '$ED/lapis.png' >'$ED/lapis.log' 2>&1" >/dev/null 2>&1
LOG="$(sed 's/\x1b\[[0-9;]*m//g' "$ED/lapis.log")"
if echo "$LOG" | grep -Eq "editor: (lápis desenhado|esboço reconhecido como Line)" && echo "$LOG" | grep -q "editor: desfeito" && echo "$LOG" | grep -q "editor: refeito" && python3 -c "
import sys
from PIL import Image
img = Image.open(sys.argv[1]).convert('RGB')
red = lambda p: p[0] > 200 and p[1] < 90 and p[2] < 90
ok = img.size == (400, 300) and all(red(img.getpixel((x, 153))) for x in (40, 180, 300)) and img.getpixel((180, 120)) == (10, 80, 200) and img.getpixel((300, 200)) == (255, 255, 255)
sys.exit(0 if ok else 1)" "$ED/lapis.png"; then
    echo "ok    editor: o lápis pinta os pixels do traço (desfazer e refazer antes de salvar)"; PASS=$((PASS + 1))
else
    echo "FALHA editor: o lápis pinta os pixels do traço"; FAIL=$((FAIL + 1))
fi
# Ctrl+Delete (Mover para a Lixeira): o arquivo vai para a Lixeira do teste e o editor fecha.
make_png "$ED/apagar.png"
SCENARIO_OUT="$MIN/apagar" timeout 150 "$SCEN" "400 combo ctrl+delete; 500 list" -- "$EDITOR '$ED/apagar.png'" >/dev/null 2>&1
LOG="$(min_log apagar)"
if [ ! -e "$ED/apagar.png" ] && [ -f "$MIN/apagar/data/Trash/files/apagar.png" ] && grep -q "^Path=.*apagar.png" "$MIN/apagar/data/Trash/info/apagar.png.trashinfo" \
    && ! echo "$LOG" | grep -q 'autoteste: janela .*Editor de Capturas'; then
    echo "ok    editor: Mover para a Lixeira manda o arquivo para a Lixeira e fecha"; PASS=$((PASS + 1))
else
    echo "FALHA editor: Mover para a Lixeira"; FAIL=$((FAIL + 1))
fi
# Esboço: um quadrado feito à mão (cantos arredondados e tremor) vira retângulo, não
# oval; Ctrl+= amplia a imagem (zoom de 125%).
make_png "$ED/esboco.png"
SCENARIO_OUT="$MIN/esboco" timeout 150 "$SCEN" "400 key s; 420 press 446 292; 421 move 460 291; 422 move 469 288; 423 move 480 289; 424 move 494 291; 425 move 507 292; 426 move 515 290; 427 move 527 288; 428 move 542 289; 429 move 553 291; 430 move 562 292; 431 move 564 290; 432 move 570 291; 433 move 575 295; 434 move 579 300; 435 move 580 306; 436 move 582 305; 437 move 579 316; 438 move 579 329; 439 move 582 343; 440 move 581 354; 441 move 578 364; 442 move 579 375; 443 move 582 389; 444 move 580 402; 445 move 578 413; 446 move 580 423; 447 move 580 424; 448 move 579 430; 449 move 575 435; 450 move 570 439; 451 move 564 440; 452 move 562 439; 453 move 551 441; 454 move 542 442; 455 move 529 440; 456 move 515 438; 457 move 505 439; 458 move 495 441; 459 move 481 442; 460 move 468 440; 461 move 459 438; 462 move 448 439; 463 move 446 440; 464 move 440 439; 465 move 435 435; 466 move 431 430; 467 move 430 424; 468 move 430 426; 469 move 432 414; 470 move 430 400; 471 move 428 388; 472 move 431 379; 473 move 432 369; 474 move 429 357; 475 move 429 343; 476 move 432 331; 477 move 431 322; 478 move 428 312; 480 release 428 312; 520 list" -- "$EDITOR '$ED/esboco.png' >'$ED/esboco.log' 2>&1" >/dev/null 2>&1
if sed 's/\x1b\[[0-9;]*m//g' "$ED/esboco.log" | grep -q "editor: esboço reconhecido como Rect"; then
    echo "ok    editor: quadrado à mão no Esboço vira retângulo"; PASS=$((PASS + 1))
else
    echo "FALHA editor: quadrado à mão no Esboço vira retângulo"; FAIL=$((FAIL + 1))
fi
make_png "$ED/zoom.png"
SCENARIO_OUT="$MIN/zoom" timeout 150 "$SCEN" "400 click 900 600; 420 combo ctrl+=; 440 combo ctrl+=; 470 list" -- "$EDITOR '$ED/zoom.png' >'$ED/zoom.log' 2>&1" >/dev/null 2>&1
if sed 's/\x1b\[[0-9;]*m//g' "$ED/zoom.log" | grep -q "editor: zoom 156%"; then
    echo "ok    editor: Ctrl+= amplia a imagem"; PASS=$((PASS + 1))
else
    echo "FALHA editor: Ctrl+= amplia a imagem"; FAIL=$((FAIL + 1))
fi
# Deslizante de zoom na barra de baixo (trilho de x=899 a x=1025, em y=702): arrastar
# até a metade amplia (zoom logarítmico de 1x a 16x, perto de 4x).
make_png "$ED/zoom2.png"
SCENARIO_OUT="$MIN/zoom2" timeout 150 "$SCEN" "400 press 900 702; 410 move 930 702; 420 move 962 702; 430 release 962 702; 470 list" -- "$EDITOR '$ED/zoom2.png' >'$ED/zoom2.log' 2>&1" >/dev/null 2>&1
if sed 's/\x1b\[[0-9;]*m//g' "$ED/zoom2.log" | grep -Eq "editor: zoom (3[0-9][0-9]|4[0-9][0-9])%"; then
    echo "ok    editor: deslizante de zoom amplia a imagem"; PASS=$((PASS + 1))
else
    echo "FALHA editor: deslizante de zoom amplia a imagem"; FAIL=$((FAIL + 1))
fi
rm -rf "$MIN"
