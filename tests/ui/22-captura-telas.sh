# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== captura em vários monitores (telas virtuais)"
# A Captura (Print) com uma camada em cada tela (B22): HEADLESS-2 (1920x1080) fica em
# (1366, 0), à direita da principal. A seleção atravessa as telas e, depois de escolhida,
# se ajusta: com "Ajustar" (a ferramenta padrão) arrastar dentro dela muda o lugar, e as
# bolinhas (pontas e meio dos lados) mudam o tamanho. Painel na principal: "Janela" em
# (461, 665), "Seta" em (453, 707). O app registra cada área em `captura: área LxA em
# (X, Y)`.
CAP2="$(mktemp -d)"
# ffmpeg de teste para a gravação (grava os argumentos e conta os bytes, como na seção
# da captura).
printf '#!/bin/sh\nfor a; do last=$a; done\nprintf "%%s\\n" "$@" > "$last.args"\nwc -c > "$last"\n' > "$CAP2/ffmpeg"
chmod +x "$CAP2/ffmpeg"
# cap2 "roteiro" "config.toml" "comando extra": roda com as duas telas; log em
# $CAP2/scen/mide.log, imagem salva em $CAP2/pics/Capturas de tela, vídeo em
# $CAP2/videos/Gravações de tela. CAP2_DUMP=<quadro> salva um quadro (da tela
# CAP2_DUMP_OUTPUT, se dada) em $CAP2/scen/frame.png.
cap2() {
    rm -rf "$CAP2/cfg" "$CAP2/pics" "$CAP2/scen" "$CAP2/videos"
    mkdir -p "$CAP2/cfg/mide" "$CAP2/videos"
    printf '%b' "${2:-}" > "$CAP2/cfg/mide/config.toml"
    printf 'XDG_PICTURES_DIR="%s"\nXDG_VIDEOS_DIR="%s"\n' "$CAP2/pics" "$CAP2/videos" > "$CAP2/cfg/user-dirs.dirs"
    XDG_CONFIG_HOME="$CAP2/cfg" SCENARIO_OUT="$CAP2/scen" MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT="${CAP2_WAIT:-16}" \
        MIDE_FFMPEG="$CAP2/ffmpeg" env ${CAP2_DUMP_OUTPUT:+"MIDE_DUMP_OUTPUT=$CAP2_DUMP_OUTPUT"} timeout 150 "$SCEN" "$1" ${CAP2_DUMP:+"$CAP2_DUMP"} \
        -- ${3:+"$3"} > "$CAP2/out.txt" 2>&1
}
# cap2_size: tamanho da imagem salva ("nada" sem imagem).
cap2_size() {
    python3 -c "
import glob, sys
from PIL import Image
files = glob.glob(sys.argv[1] + '/pics/Capturas de tela/*.png')
print('%dx%d' % Image.open(files[0]).size if files else 'nada')" "$CAP2" 2>/dev/null
}
# cap2_check "nome" "LxA esperado" "regex no log (vazio: não confere)"
cap2_check() {
    local got
    got="$(cap2_size)"
    if [ "$got" = "$2" ] && { [ -z "$3" ] || grep -aEq "$3" "$CAP2/scen/mide.log"; }; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1 (esperado $2, obtido $got; $(grep -a "captura: área" "$CAP2/scen/mide.log" | tail -1 | sed 's/.*captura: //'))"; FAIL=$((FAIL + 1))
    fi
}
SAVE="660 combo ctrl+s; 720 list"
cap2 "400 key print; 600 press 1500 300; 610 move 1650 400; 620 release 1800 500; $SAVE"
cap2_check "captura em duas telas: área na segunda tela" 300x200 'captura: área 300x200 em \(1500, 300\)'
cap2 "400 key print; 600 press 1200 200; 610 move 1400 300; 620 release 1566 450; $SAVE"
cap2_check "captura em duas telas: área atravessando da principal para a segunda" 366x250 'captura: área 366x250 em \(1200, 200\)'
cap2 "400 key print; 600 press 1600 300; 610 move 1400 400; 620 release 1266 500; $SAVE"
cap2_check "captura em duas telas: área da segunda para a principal" 334x200 'captura: área 334x200 em \(1266, 300\)'
# As duas telas escurecidas, a área clara nas duas, bolinhas na cor de destaque nas
# duas e o painel só na principal (comparando com as telas sem a Captura).
cap2 "400 list" "" "sh -c 'sleep 5; mide-ctl screenshot all $CAP2/base > /dev/null'"
CAP2_WAIT=20 cap2 "400 key print; 600 press 1200 200; 610 move 1400 300; 620 release 1566 450; 1000 list" "" "sh -c 'sleep 14; mide-ctl screenshot all $CAP2/t > /dev/null'"
if python3 -c "
import sys
from PIL import Image
d = sys.argv[1]
img = lambda n: Image.open(d + '/' + n + '.png').convert('RGB')
b1, b2, t1, t2 = img('base-HEADLESS-1'), img('base-HEADLESS-2'), img('t-HEADLESS-1'), img('t-HEADLESS-2')
lum = lambda p: sum(p)
near = lambda p, q: max(abs(a - b) for a, b in zip(p, q)) <= 40
accent = (10, 110, 235)
erros = []
if not lum(t1.getpixel((100, 400))) < lum(b1.getpixel((100, 400))) * 0.8: erros.append('principal não escureceu')
if not lum(t2.getpixel((1000, 800))) < lum(b2.getpixel((1000, 800))) * 0.8: erros.append('segunda não escureceu')
if not near(t1.getpixel((1300, 400)), b1.getpixel((1300, 400))): erros.append('área clara na principal')
if not near(t2.getpixel((100, 400)), b2.getpixel((100, 400))): erros.append('área clara na segunda')
for (im, p, nome) in ((t1, (1200, 200), 'ponta de cima à esquerda'), (t1, (1200, 325), 'meio da esquerda'), (t1, (1200, 450), 'ponta de baixo à esquerda'),
                      (t2, (200, 200), 'ponta de cima à direita'), (t2, (200, 325), 'meio da direita'), (t2, (200, 450), 'ponta de baixo à direita'), (t2, (17, 200), 'meio de cima')):
    if not near(im.getpixel(p), accent): erros.append('bolinha %s: %s' % (nome, im.getpixel(p)))
if not lum(t1.getpixel((683, 690))) > lum(b1.getpixel((683, 690))) + 100: erros.append('sem painel na principal')
if not lum(t2.getpixel((960, 1000))) < lum(b2.getpixel((960, 1000))): erros.append('painel na segunda')
sys.exit('; '.join(erros) if erros else 0)
" "$CAP2"; then
    echo "ok    captura em duas telas: escurecido e bolinhas nas duas, painel só na principal"; PASS=$((PASS + 1))
else
    echo "FALHA captura em duas telas: escurecido e bolinhas nas duas, painel só na principal"; FAIL=$((FAIL + 1))
fi
SEL="400 key print; 600 press 300 200; 610 move 500 350; 620 release 700 450"
SAVE2="760 combo ctrl+s; 820 list"
cap2 "$SEL; 680 press 700 450; 690 move 750 480; 700 release 800 500; $SAVE2"
cap2_check "área ajustável: a bolinha da ponta muda o tamanho" 500x300 'captura: área 500x300 em \(300, 200\)'
cap2 "$SEL; 680 press 500 300; 690 move 550 330; 700 release 600 350; $SAVE2"
cap2_check "área ajustável: arrastar dentro (Ajustar) muda o lugar" 400x250 'captura: área 400x250 em \(400, 250\)'
cap2 "400 key print; 600 press 1200 200; 610 move 1250 250; 620 release 1300 300; 680 press 1250 250; 690 move 1400 250; 700 release 1550 250; $SAVE2"
cap2_check "área ajustável: mudar a área para a outra tela" 100x100 'captura: área 100x100 em \(1500, 200\)'
cap2 "$SEL; 680 press 300 325; 690 move 600 325; 700 release 1000 325; $SAVE2"
cap2_check "área ajustável: bolinha da esquerda passando da direita vira a área" 300x250 'captura: área 300x250 em \(700, 200\)'
# Com a Seta escolhida, arrastar dentro da área desenha (e a área não muda).
cap2 "$SEL; 660 click 453 707; 680 press 350 250; 690 move 450 300; 700 release 600 400; $SAVE2"
if [ "$(cap2_size)" = 400x250 ] && grep -aq "captura: ferramenta Seta" "$CAP2/scen/mide.log" && python3 -c "
import glob, sys
from PIL import Image
img = Image.open(glob.glob(sys.argv[1] + '/pics/Capturas de tela/*.png')[0]).convert('RGB')
red = sum(1 for p in img.getdata() if p[0] > 200 and p[1] < 90 and p[2] < 90)
sys.exit(0 if red > 200 else 'vermelho: %d' % red)" "$CAP2"; then
    echo "ok    área ajustável: com a Seta, arrastar dentro desenha"; PASS=$((PASS + 1))
else
    echo "FALHA área ajustável: com a Seta, arrastar dentro desenha ($(cap2_size))"; FAIL=$((FAIL + 1))
fi
# Área com a borda de baixo na faixa do painel (y=700): o painel sobe para o topo da
# tela (y=44, "Seta" em (453, 111)) e a bolinha do meio de baixo continua pegando o
# clique.
TALL="400 key print; 600 press 300 200; 610 move 500 450; 620 release 700 700"
cap2 "$TALL; 680 press 500 700; 690 move 500 650; 700 release 500 600; $SAVE2"
cap2_check "área ajustável: bolinha embaixo do painel muda o tamanho" 400x400 'captura: área 400x400 em \(300, 200\)'
cap2 "$TALL; 660 click 453 111; $SAVE2"
cap2_check "área ajustável: o painel sai de cima das bolinhas (vai para o topo)" 400x500 'captura: ferramenta Seta'
# Mover a área para o canto sem tela (abaixo da principal, à esquerda da segunda): ela
# fica na segunda tela, encostada, com as bolinhas alcançáveis (antes ia para
# (1080, 780), quase toda no buraco).
cap2 "400 key print; 600 press 1400 600; 610 move 1550 750; 620 release 1700 900; 680 press 1690 890; 690 move 1500 980; 700 release 1370 1075; $SAVE2"
cap2_check "área ajustável: mover não leva a área para o buraco entre as telas" 300x300 'captura: área 300x300 em \(1366, 780\)'
# ⌘⇧4: atravessa as telas e salva ao soltar, sem ficar aberta com as bolinhas.
cap2 "400 combo super+shift+4; 600 press 1266 200; 610 move 1400 300; 625 release 1566 400; 760 list"
if [ "$(cap2_size)" = 300x200 ] && grep -aq "captura rápida de uma área" "$CAP2/scen/mide.log" && ! grep -aq 'autoteste: camada "mide-screenshot' "$CAP2/scen/mide.log"; then
    echo "ok    captura em duas telas: ⌘⇧4 atravessando salva ao soltar (sem bolinhas)"; PASS=$((PASS + 1))
else
    echo "FALHA captura em duas telas: ⌘⇧4 atravessando ($(cap2_size))"; FAIL=$((FAIL + 1))
fi
# Modo Janela com a principal à direita (HEADLESS-1 em (1920, 0)): o painel fica na
# principal ("Janela" em (2381, 665)) e o clique no Terminal (nasce em (1968, 76)) salva
# a janela.
CAP2_WAIT=18 cap2 "400 key print; 600 click 2381 665; 640 click 2200 300; 700 combo ctrl+s; 760 list" \
    '[monitors]\nprimary = "HEADLESS-1"\n[monitors.outputs."HEADLESS-2"]\nx = 0\ny = 0\n[monitors.outputs."HEADLESS-1"]\nx = 1920\ny = 0\n' "$TERMINAL"
AREA2="$(grep -a "captura: área" "$CAP2/scen/mide.log" | tail -1 | sed -E 's/.*captura: área ([0-9]+x[0-9]+) em \(([0-9]+),.*/\1 \2/')"
if [ -n "$AREA2" ] && [ "${AREA2#* }" -ge 1920 ] && [ "$(cap2_size)" = "${AREA2% *}" ]; then
    echo "ok    captura com a principal à direita: modo Janela salva a janela ($AREA2)"; PASS=$((PASS + 1))
else
    echo "FALHA captura com a principal à direita: modo Janela (área '$AREA2', imagem $(cap2_size))"; FAIL=$((FAIL + 1))
fi
# A segunda tela é desligada com a Captura aberta: a camada dela sai (listada antes e
# não depois), a Captura continua e salva a área da principal.
printf '[monitors.outputs."HEADLESS-2"]\nenabled = false\n' > "$CAP2/desligada.toml"
CAP2_WAIT=24 cap2 "$SEL; 640 list; 960 list; 1000 combo ctrl+s; 1060 list" "" "sh -c 'sleep 13; cp $CAP2/desligada.toml $CAP2/cfg/mide/config.toml; mide-ctl outputs apply > /dev/null'"
EXTRA2="$(grep -ac 'autoteste: camada "mide-screenshot@HEADLESS-2"' "$CAP2/scen/mide.log")"
MAIN2="$(grep -ac 'autoteste: camada "mide-screenshot" ' "$CAP2/scen/mide.log")"
if [ "$(cap2_size)" = 400x250 ] && [ "$EXTRA2" = 1 ] && [ "$MAIN2" = 2 ]; then
    echo "ok    captura em duas telas: desligar a segunda com a Captura aberta"; PASS=$((PASS + 1))
else
    echo "FALHA captura em duas telas: desligar a segunda com a Captura aberta ($(cap2_size), extra $EXTRA2, principal $MAIN2)"; FAIL=$((FAIL + 1))
fi
# Tela inteira com duas telas (B22): o botão "Tela inteira" (560, 665) pergunta qual; a
# tela sob o ponteiro fica clara, com o contorno na cor de destaque e o rótulo em vidro;
# o clique escolhe; Esc volta para a Seleção.
cap2 "400 list" "" "sh -c 'sleep 5; mide-ctl screenshot all $CAP2/base > /dev/null'"
CAP2_DUMP=760 CAP2_DUMP_OUTPUT=HEADLESS-2 cap2 "400 key print; 600 click 560 665; 700 move 1900 300; 740 move 2000 500; 800 click 2000 500; 860 combo ctrl+s; 920 list"
if [ "$(cap2_size)" = 1920x1080 ] && grep -aq "captura: tela HEADLESS-2 escolhida" "$CAP2/scen/mide.log" && python3 -c "
import sys
from PIL import Image
d = sys.argv[1]
base = Image.open(d + '/base-HEADLESS-2.png').convert('RGB')
during = Image.open(d + '/scen/frame.png').convert('RGB')
near = lambda p, q, t=40: max(abs(a - b) for a, b in zip(p, q)) <= t
erros = []
if during.size != (1920, 1080): erros.append('quadro %dx%d' % during.size)
if not near(during.getpixel((300, 900)), base.getpixel((300, 900))): erros.append('tela não ficou clara')
if not near(during.getpixel((2, 500)), (10, 110, 235), 60): erros.append('sem contorno: %s' % (during.getpixel((2, 500)),))
if not sum(during.getpixel((960, 520))) > sum(base.getpixel((960, 520))) + 60: erros.append('sem rótulo no meio')
sys.exit('; '.join(erros) if erros else 0)
" "$CAP2"; then
    echo "ok    tela inteira em duas telas: a tela sob o ponteiro destacada, o clique escolhe (1920x1080)"; PASS=$((PASS + 1))
else
    echo "FALHA tela inteira em duas telas: escolher a tela ($(cap2_size))"; FAIL=$((FAIL + 1))
fi
cap2 "400 key print; 600 click 560 665; 660 key esc; 700 press 300 200; 710 move 500 350; 720 release 700 450; 780 combo ctrl+s; 840 list"
cap2_check "tela inteira em duas telas: Esc volta para a Seleção" 400x250 'escolha da tela cancelada'
# Uma tela só: "Tela inteira" é ela, sem perguntar.
UMA="$(mktemp -d)"
mkdir -p "$UMA/cfg/mide"
printf 'XDG_PICTURES_DIR="%s"\n' "$UMA/pics" > "$UMA/cfg/user-dirs.dirs"
XDG_CONFIG_HOME="$UMA/cfg" SCENARIO_OUT="$UMA/scen" MIDE_HEADLESS_OUTPUTS=1366x768 SCENARIO_WAIT=16 timeout 150 "$SCEN" "400 key print; 600 click 560 665; 700 combo ctrl+s; 760 list" >/dev/null 2>&1
UMA_SIZE="$(python3 -c "
import glob, sys
from PIL import Image
files = glob.glob(sys.argv[1] + '/pics/Capturas de tela/*.png')
print('%dx%d' % Image.open(files[0]).size if files else 'nada')" "$UMA" 2>/dev/null)"
if [ "$UMA_SIZE" = 1366x768 ] && ! grep -aq "escolhida" "$UMA/scen/mide.log"; then
    echo "ok    tela inteira com uma tela: salva sem perguntar (1366x768)"; PASS=$((PASS + 1))
else
    echo "FALHA tela inteira com uma tela ($UMA_SIZE)"; FAIL=$((FAIL + 1))
fi
rm -rf "$UMA"
# ⌘⇧3 com duas telas: um arquivo por tela, com o nome dela, e a principal copiada.
cap2 "400 combo super+shift+3; 600 list"
if python3 -c "
import glob, sys
from PIL import Image
d = sys.argv[1] + '/pics/Capturas de tela/'
sizes = {f.rsplit('(', 1)[-1].rstrip(').png'): Image.open(f).size for f in glob.glob(d + 'Captura de tela * (*).png')}
sys.exit(0 if sizes == {'HEADLESS-1': (1366, 768), 'HEADLESS-2': (1920, 1080)} else str(sizes))" "$CAP2" && grep -aq "captura copiada (1366x768)" "$CAP2/scen/mide.log"; then
    echo "ok    ⌘⇧3 em duas telas: um arquivo por tela e a principal copiada"; PASS=$((PASS + 1))
else
    echo "FALHA ⌘⇧3 em duas telas ($(ls "$CAP2/pics/Capturas de tela" 2>/dev/null | tr '\n' ' '))"; FAIL=$((FAIL + 1))
fi
# ⌘⇧4 numa área toda na segunda tela.
cap2 "400 combo super+shift+4; 600 press 1500 300; 610 move 1650 400; 625 release 1800 500; 760 list"
cap2_check "⌘⇧4 na segunda tela salva ao soltar" 300x200 'captura: área 300x200 em \(1500, 300\)'

# ⌘⇧5 com duas telas (ffmpeg de teste): "Área" (469, 710) numa tela qualquer, presa à
# tela onde começou (a gravação é de uma tela: `record start x y w h tela NOME`, pontos
# dela); "Tela inteira" pergunta a tela e o clique já começa a gravar. "Gravar" em
# (980, 710); parar pelo botão na barra (1220, 14).
# rec2_check "nome" "regex no log" "LxA no -video_size"
rec2_check() {
    local video
    video="$(ls "$CAP2/videos/Gravações de tela/"*.mp4 2>/dev/null | head -n 1)"
    if [ -n "$video" ] && grep -aEq "$2" "$CAP2/scen/mide.log" && grep -qx "$3" "$video.args"; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1 ($(grep -a "gravando" "$CAP2/scen/mide.log" | tail -1 | sed 's/.*gravando/gravando/'))"; FAIL=$((FAIL + 1))
    fi
}
CAP2_WAIT=22 cap2 "300 combo super+shift+5; 420 click 469 710; 500 press 1500 300; 510 move 1650 400; 520 release 1800 500; 600 click 980 710; 800 click 1220 14; 900 list"
rec2_check "gravar em duas telas: área na segunda tela" 'gravando HEADLESS-2 \(300x200 em 134,300\)' 300x200
CAP2_WAIT=22 cap2 "300 combo super+shift+5; 420 click 469 710; 500 press 1200 200; 510 move 1400 300; 520 release 1566 450; 600 click 980 710; 800 click 1220 14; 900 list"
rec2_check "gravar em duas telas: área presa à tela onde começou" 'gravando HEADLESS-1 \(166x250 em 1200,200\)' 166x250
CAP2_WAIT=22 cap2 "300 combo super+shift+5; 420 move 1900 400; 460 click 2000 500; 700 click 1220 14; 800 list"
rec2_check "gravar em duas telas: tela inteira, o clique escolhe a tela" 'gravando HEADLESS-2 \(1920x1080 em 0,0\)' 1920x1080

# Escalas diferentes: a segunda tela em 1,25 (1920x1080 pixels, 1536x864 pontos em
# (1366, 0)). Uma área de 200x200 pontos nela sai com 250x250 pixels, do lugar certo da
# imagem; atravessando, a parte da principal (escala 1) é ampliada para 1,25.
SCALE2='[monitors.outputs."HEADLESS-2"]\nscale = 1.25\n'
cap2 "400 list" "$SCALE2" "sh -c 'sleep 5; mide-ctl screenshot all $CAP2/base > /dev/null'"
cap2 "400 key print; 600 press 1466 100; 610 move 1566 200; 620 release 1666 300; $SAVE" "$SCALE2"
if [ "$(cap2_size)" = 250x250 ] && python3 -c "
import glob, sys
from PIL import Image, ImageChops
d = sys.argv[1]
shot = Image.open(glob.glob(d + '/pics/Capturas de tela/*.png')[0]).convert('RGB')
base = Image.open(d + '/base-HEADLESS-2.png').convert('RGB')
if base.size != (1920, 1080): sys.exit('base %dx%d' % base.size)
diff = ImageChops.difference(shot, base.crop((125, 125, 375, 375)))
bad = sum(1 for p in diff.getdata() if max(p) > 40)
sys.exit(0 if bad < 250 * 250 // 100 else 'fora do lugar: %d' % bad)" "$CAP2"; then
    echo "ok    escalas diferentes: área na tela em 1,25 sai com 250x250, no lugar certo"; PASS=$((PASS + 1))
else
    echo "FALHA escalas diferentes: área na tela em 1,25 ($(cap2_size))"; FAIL=$((FAIL + 1))
fi
cap2 "400 key print; 600 press 1266 100; 610 move 1366 150; 620 release 1466 200; $SAVE" "$SCALE2"
if [ "$(cap2_size)" = 250x125 ] && python3 -c "
import glob, sys
from PIL import Image, ImageChops
d = sys.argv[1]
shot = Image.open(glob.glob(d + '/pics/Capturas de tela/*.png')[0]).convert('RGB')
b2 = Image.open(d + '/base-HEADLESS-2.png').convert('RGB')
# A metade direita (da segunda tela) é a imagem dela, sem mudar de tamanho.
diff = ImageChops.difference(shot.crop((125, 0, 250, 125)), b2.crop((0, 125, 125, 250)))
bad = sum(1 for p in diff.getdata() if max(p) > 40)
sys.exit(0 if bad < 125 * 125 // 50 else 'fora do lugar: %d' % bad)" "$CAP2" && grep -aq 'captura: área 200x100 em (1266, 100) \[250x125 px\]' "$CAP2/scen/mide.log"; then
    echo "ok    escalas diferentes: área atravessando sai na escala maior (250x125)"; PASS=$((PASS + 1))
else
    echo "FALHA escalas diferentes: área atravessando ($(cap2_size))"; FAIL=$((FAIL + 1))
fi
cap2 "400 combo super+shift+3; 600 list" "$SCALE2"
if python3 -c "
import glob, sys
from PIL import Image
d = sys.argv[1] + '/pics/Capturas de tela/'
sizes = sorted(Image.open(f).size for f in glob.glob(d + 'Captura de tela * (*).png'))
sys.exit(0 if sizes == [(1366, 768), (1920, 1080)] else str(sizes))" "$CAP2"; then
    echo "ok    escalas diferentes: ⌘⇧3 salva cada tela em pixels (1366x768 e 1920x1080)"; PASS=$((PASS + 1))
else
    echo "FALHA escalas diferentes: ⌘⇧3"; FAIL=$((FAIL + 1))
fi
cap2 "400 key print; 600 click 560 665; 700 move 1900 300; 740 move 2000 500; 800 click 2000 500; 860 combo ctrl+s; 920 list" "$SCALE2"
cap2_check "escalas diferentes: tela inteira escolhida em 1,25 sai com 1920x1080" 1920x1080 'captura: tela HEADLESS-2 escolhida'
CAP2_WAIT=22 cap2 "300 combo super+shift+5; 420 click 469 710; 500 press 1500 300; 510 move 1650 400; 520 release 1800 500; 600 click 980 710; 800 click 1220 14; 900 list" "$SCALE2"
rec2_check "escalas diferentes: gravar uma área da tela em 1,25 (pixels, lados pares)" 'gravando HEADLESS-2 \(374x250 em 168,375\)' 374x250
rm -rf "$CAP2"
