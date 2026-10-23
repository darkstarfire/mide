# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== aviso de volume e brilho (OSD, A23)"
# O aviso é desenhado pelo compositor no meio, embaixo: painel de 260x48 em
# (553, 590) na tela de 1366x768; ícone em (567, 602), barra de nível de (603, 611) a
# (751, 617) e o valor à direita. Com MIDE_SYSTEM_DRY_RUN, o volume simulado começa em
# 50% e o brilho em 70% (nada muda no computador, e sem som: só o log).
OSD="$(mktemp -d)"
osd_log() { sed 's/\x1b\[[0-9;]*m//g' "$OSD/$1/mide.log" 2>/dev/null; }
# Cor de um ponto do quadro salvo ("r,g,b").
osd_pixel() { python3 -c "import sys; from PIL import Image; print('%d,%d,%d' % Image.open(sys.argv[1]).convert('RGB').getpixel((int(sys.argv[2]), int(sys.argv[3]))))" "$OSD/$1/frame.png" "$2" "$3" 2>/dev/null; }
# A parte cheia da barra está no azul de destaque (claro: 10,110,235; escuro: 30,125,245).
osd_blue() { echo "$1" | awk -F, '{ exit !($3 > 200 && $1 < 90 && $2 > 90 && $2 < 150) }'; }
osd_run() { SCENARIO_OUT="$OSD/$1" SCENARIO_WAIT="${3:-12}" timeout 150 "$SCEN" "$2" "${4:-530}" >/dev/null 2>&1; }
ok() { echo "ok    $1"; PASS=$((PASS + 1)); }
falha() { echo "FALHA $1"; FAIL=$((FAIL + 1)); }

# Volume +: o aviso aparece com o novo nível, desenha a barra e some ~1,5 s depois.
osd_run sobe "500 key volumeup; 700 list"
LOG="$(osd_log sobe)"
T_IN="$(echo "$LOG" | grep -m1 "osd: volume 55%" | cut -c12-23)"
T_OUT="$(echo "$LOG" | grep -m1 "osd: escondido" | cut -c12-23)"
if echo "$LOG" | grep -q "osd: aparece" && [ -n "$T_IN" ] && [ -n "$T_OUT" ] && python3 -c "
import sys
f = lambda s: sum(float(x) * m for x, m in zip(s.split(':'), (3600, 60, 1)))
d = f(sys.argv[2]) - f(sys.argv[1])
sys.exit(0 if 1.45 <= d <= 2.4 else 1)" "$T_IN" "$T_OUT"; then
    ok "volume +: o aviso aparece com 55% e some sozinho em ~1,5 s"
else
    falha "volume +: o aviso aparece com 55% e some sozinho em ~1,5 s (entrou $T_IN, saiu $T_OUT)"
fi
BAR="$(osd_pixel sobe 615 614)"
VEIL="$(osd_pixel sobe 700 598)"
EMPTY="$(osd_pixel sobe 740 614)"
if osd_blue "$BAR" && echo "$VEIL" | awk -F, '{ exit !($1 > 150 && $2 > 150 && $3 > 150) }' && ! osd_blue "$EMPTY"; then
    ok "volume +: o quadro mostra o painel claro com a barra cheia até o nível"
else
    falha "volume +: o quadro mostra o painel claro com a barra cheia até o nível (barra $BAR, véu $VEIL, vazio $EMPTY)"
fi

# Apertos seguidos: o mesmo aviso é atualizado (aparece uma vez só) e some depois do último.
osd_run seguidos "500 key volumeup; 520 key volumeup; 540 key volumedown; 760 list"
LOG="$(osd_log seguidos)"
if [ "$(echo "$LOG" | grep -c "osd: aparece")" = 1 ] && [ "$(echo "$LOG" | grep -c "osd: escondido")" = 1 ] \
    && [ "$(echo "$LOG" | grep -oE "osd: volume [0-9]+%" | tr '\n' ' ')" = "osd: volume 55% osd: volume 60% osd: volume 55% " ]; then
    ok "apertos seguidos atualizam o mesmo aviso (55, 60, 55%) sem piscar"
else
    falha "apertos seguidos atualizam o mesmo aviso (55, 60, 55%) sem piscar"
    echo "$LOG" | grep "osd:" | sed 's/^/      /'
fi

# Apertos rápidos com o comando demorando (o primeiro 400 ms, o segundo 200 ms): os
# apertos rodam em ordem numa thread só, então o aviso termina no nível de verdade
# (60%), e não no de uma leitura feita antes de o primeiro comando valer.
MIDE_SYSTEM_DRY_RUN_DELAY_MS=400 osd_run ordem "500 key volumeup; 502 key volumeup; 700 list"
LAST="$(osd_log ordem | grep -oE "osd: volume [0-9]+%" | tail -n1)"
if [ "$LAST" = "osd: volume 60%" ]; then
    ok "apertos rápidos com o comando lento: o aviso termina no nível real (60%)"
else
    falha "apertos rápidos com o comando lento: o aviso termina no nível real (60%) (último: $LAST)"
fi

# Captura de tela (⌘⇧3) com o aviso na tela: o aviso fica fora da imagem, como o
# cursor (o quadro da tela, salvo no mesmo momento, mostra o aviso).
mkdir -p "$OSD/cap-cfg/mide" "$OSD/cap-pics"
printf 'XDG_PICTURES_DIR="%s"\n' "$OSD/cap-pics" > "$OSD/cap-cfg/user-dirs.dirs"
XDG_CONFIG_HOME="$OSD/cap-cfg" osd_run captura "500 key volumeup; 520 combo super+shift+3; 700 list" 14 525
SHOT="$(ls "$OSD/cap-pics/Capturas de tela/"*.png 2>/dev/null | head -n1)"
SHOT_BAR="$([ -n "$SHOT" ] && python3 -c "import sys; from PIL import Image; print('%d,%d,%d' % Image.open(sys.argv[1]).convert('RGB').getpixel((615, 614)))" "$SHOT" 2>/dev/null)"
if [ -n "$SHOT_BAR" ] && ! osd_blue "$SHOT_BAR" && osd_blue "$(osd_pixel captura 615 614)"; then
    ok "captura de tela: o aviso não sai na imagem (só na tela)"
else
    falha "captura de tela: o aviso não sai na imagem (imagem $SHOT_BAR, tela $(osd_pixel captura 615 614))"
fi

# Mudo, microfone e brilho, cada um com o seu valor.
osd_run mudo "500 key mute; 600 list"
osd_run microfone "500 key micmute; 600 list"
osd_run brilho "500 key brightnessup; 600 list"
if osd_log mudo | grep -q "osd: volume 50% (mudo)" && ! osd_blue "$(osd_pixel mudo 615 614)"; then
    ok "mudo: o aviso mostra o alto-falante cortado e a barra cinza"
else
    falha "mudo: o aviso mostra o alto-falante cortado e a barra cinza ($(osd_pixel mudo 615 614))"
fi
if osd_log microfone | grep -q "osd: microfone desligado"; then
    ok "microfone mudo: o aviso diz \"Microfone desligado\""
else
    falha "microfone mudo: o aviso diz \"Microfone desligado\""
fi
if osd_log brilho | grep -q "osd: brilho 75%" && osd_blue "$(osd_pixel brilho 700 614)"; then
    ok "brilho +: o aviso mostra 75% com a barra"
else
    falha "brilho +: o aviso mostra 75% com a barra ($(osd_pixel brilho 700 614))"
fi

# Tema escuro: o véu do painel fica grafite.
mkdir -p "$OSD/escuro-cfg/mide"
printf '[appearance]\ntheme = "dark"\n' > "$OSD/escuro-cfg/mide/config.toml"
XDG_CONFIG_HOME="$OSD/escuro-cfg" osd_run escuro "500 key volumeup; 600 list"
VEIL="$(osd_pixel escuro 700 598)"
if echo "$VEIL" | awk -F, '{ exit !($1 < 110 && $2 < 110 && $3 < 120) }' && osd_blue "$(osd_pixel escuro 615 614)"; then
    ok "tema escuro: o aviso é grafite com a barra azul"
else
    falha "tema escuro: o aviso é grafite com a barra azul (véu $VEIL)"
fi

# Tela bloqueada: volume e microfone (teclas que valem com a tela bloqueada) aparecem
# por cima da tela de bloqueio.
export MIDE_LOCK_TEST_PASSWORD=teste
osd_run bloqueada "430 combo super+l; 650 key volumeup; 720 list" 16 670
osd_run bloqueada-mic "430 combo super+l; 650 key micmute; 720 list" 16
unset MIDE_LOCK_TEST_PASSWORD
LOG="$(osd_log bloqueada)"
if echo "$LOG" | grep -q "autoteste: tela bloqueada" && echo "$LOG" | grep -q "osd: volume 55%" && osd_blue "$(osd_pixel bloqueada 615 614)"; then
    ok "tela bloqueada: o aviso de volume aparece por cima da tela de bloqueio"
else
    falha "tela bloqueada: o aviso de volume aparece por cima da tela de bloqueio ($(osd_pixel bloqueada 615 614))"
fi
# Sem vidro atrás (o caminho do bloqueio não desfoca regiões), o véu fica opaco.
VEIL="$(osd_pixel bloqueada 700 598)"
if echo "$VEIL" | awk -F, '{ exit !($1 > 228 && $2 > 228 && $3 > 228) }'; then
    ok "tela bloqueada: o painel do aviso é opaco (legível sem o vidro)"
else
    falha "tela bloqueada: o painel do aviso é opaco (legível sem o vidro) (véu $VEIL)"
fi
if osd_log bloqueada-mic | grep -q "autoteste: tela bloqueada" && osd_log bloqueada-mic | grep -q "osd: microfone desligado"; then
    ok "tela bloqueada: microfone mudo também mostra o aviso"
else
    falha "tela bloqueada: microfone mudo também mostra o aviso"
fi
rm -rf "$OSD"
