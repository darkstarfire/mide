# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== tela de toque, modo tablet e rotação"
# Passos do roteiro: `touch down x y`, `touch move x y`, `touch up`, `tablet on|off` e
# `orientation normal|left-up|right-up|bottom-up` (o sensor do iio-sensor-proxy).
# examples/mide-janela-teste toque: janela "Toque" 400x300 em (48, 76) (app_id
# teste-toque, como um app de terceiros) que fica verde quando um dedo encosta.
ok() { echo "ok    $1"; PASS=$((PASS + 1)); }
falha() { echo "FALHA $1"; FAIL=$((FAIL + 1)); }
JANELA="$PWD/target/debug/examples/mide-janela-teste"
TT="$(mktemp -d)"
# Roda um cenário com a saída própria; `tt_log nome` e `tt_pixel nome x y` leem depois.
tt_run() { # tt_run nome "roteiro" [quadro] [app...]
    local name="$1" script="$2" frame="${3:-}"
    shift 3 2>/dev/null || shift $#
    mkdir -p "$TT/$name"
    SCENARIO_OUT="$TT/$name" timeout 150 "$SCEN" "$script" $frame ${1:+--} "$@" >/dev/null 2>&1
}
tt_log() { sed 's/\x1b\[[0-9;]*m//g' "$TT/$1/mide.log" 2>/dev/null; }
tt_pixel() {
    python3 -c "import sys; from PIL import Image; print('%d,%d,%d' % Image.open(sys.argv[1]).convert('RGB').getpixel((int(sys.argv[2]), int(sys.argv[3]))))" "$TT/$1/frame.png" "$2" "$3" 2>/dev/null
}

if [ -x "$JANELA" ]; then
    # Toque num app que trata `wl_touch`: ele recebe o dedo (fica verde).
    tt_run app "400 touch down 250 200; 404 touch up; 470 list" 460 "$JANELA toque"
    PIX="$(tt_pixel app 250 200)"
    if tt_log app | grep -q "toque: no app" && echo "$PIX" | awk -F, '{ exit !($2 > 150 && $1 < 100 && $3 < 120) }'; then
        ok "toque num app chega como wl_touch"
    else
        falha "toque num app chega como wl_touch (pixel $PIX)"
    fi
    # Arrastar com o dedo pela barra de título do mide move a janela.
    tt_run arrasto "400 touch down 150 61; 404 touch move 200 100; 408 touch move 400 300; 412 touch up; 470 list" "" "$JANELA comum"
    if tt_log arrasto | grep -q 'autoteste: janela "Comum" popups=0 posição=(298, 315)'; then
        ok "arrastar o dedo pela barra de título move a janela"
    else
        falha "arrastar o dedo pela barra de título move a janela ($(tt_log arrasto | grep -o 'janela "Comum" popups=0 posição=([0-9, ]*)' | tail -1))"
    fi
    # Dois toques rápidos na barra de título: maximiza (o duplo clique).
    tt_run duplo "400 touch down 300 61; 402 touch up; 406 touch down 300 61; 408 touch up; 470 list" "" "$JANELA comum"
    if tt_log duplo | grep -qE 'autoteste: janela "Comum" popups=0 posição=\(0, 58\)'; then
        ok "dois toques na barra de título maximizam"
    else
        falha "dois toques na barra de título maximizam ($(tt_log duplo | grep -o 'janela "Comum" popups=0 posição=([0-9, ]*)' | tail -1))"
    fi
else
    falha "toque: falta o examples/mide-janela-teste (cargo build --examples)"
fi

# Segurar o dedo na mesa: o botão direito abre o menu dela (Nova pasta, como o
# clique direito da seção 13).
DESK="$TT/mesa"
mkdir -p "$DESK"
MIDE_DESKTOP_DIR="$DESK" tt_run segurar "300 touch down 400 300; 350 touch up; 380 touch down 450 326; 382 touch up; 410 key enter; 460 list"
if tt_log segurar | grep -q "toque: segurar" && [ -d "$DESK/Nova pasta" ]; then
    ok "segurar o dedo na mesa abre o menu (Nova pasta)"
else
    falha "segurar o dedo na mesa abre o menu (Nova pasta)"
fi

# Modo tablet: liga e desliga pela chave (o teclado na tela pergunta `tablet_mode`).
tt_run tablet "400 tablet on; 440 tablet off; 480 list"
if tt_log tablet | grep -q "modo tablet: ligado" && tt_log tablet | grep -q "modo tablet: desligado"; then
    ok "modo tablet liga e desliga pela chave"
else
    falha "modo tablet liga e desliga pela chave"
fi

# Modo tablet abre o teclado na tela sozinho com um campo de texto ativo, sem a opção
# `[keyboard] on_screen` (o mide-keyboard pergunta `tablet_mode` ao compositor). O campo
# é o do `zenity --entry`, como na seção 49.
if command -v zenity >/dev/null; then
    mkdir -p "$TT/osk"
    SCENARIO_OUT="$TT/osk" timeout 150 "$SCEN" "300 tablet on; 600 list" -- \
        "sh -c 'GTK_IM_MODULE=wayland zenity --entry --text Teste > /dev/null'" > "$TT/osk.log" 2>&1
    if grep -q "modo tablet: ligado" "$TT/osk/mide.log" && grep -q 'camada "mide-keyboard"' "$TT/osk.log"; then
        ok "modo tablet abre o teclado sozinho"
    else
        falha "modo tablet abre o teclado sozinho"
    fi
else
    echo "pulado: modo tablet abre o teclado (sem o zenity)"
fi

# Rotação: sem a chave de modo tablet, a tela gira com o sensor (como no GNOME); o
# tamanho lógico troca de lado e a barra vai para a borda esquerda do painel.
tt_run gira "400 orientation left-up; 480 list" 470
if tt_log gira | grep -q 'camada "mide-bar" em (0, 0) 768x28'; then
    ok "sensor em left-up gira a tela (768 de largura)"
else
    falha "sensor em left-up gira a tela ($(tt_log gira | grep -o 'camada "mide-bar" em [^ ]* [^ ]*' | tail -1))"
fi
# No quadro de verdade (o painel não gira), a barra fica na borda esquerda: perto do
# meio da borda esquerda é a barra (clara e translúcida), não o papel de parede.
LEFT="$(tt_pixel gira 4 384)"; MID="$(tt_pixel gira 683 384)"
if [ -n "$LEFT" ] && [ "$LEFT" != "$MID" ] && [ "$(python3 -c "from PIL import Image; print(Image.open('$TT/gira/frame.png').size)" 2>/dev/null)" = "(1366, 768)" ]; then
    ok "quadro girado no painel (barra na borda esquerda)"
else
    falha "quadro girado no painel (esquerda $LEFT, meio $MID)"
fi
# Com a chave de modo tablet, no modo notebook a tela não gira; no modo tablet, gira.
tt_run notebook "400 tablet off; 420 orientation left-up; 480 list"
if tt_log notebook | grep -q 'camada "mide-bar" em (0, 0) 1366x28'; then
    ok "modo notebook: a tela não gira com o sensor"
else
    falha "modo notebook: a tela não gira com o sensor"
fi
tt_run tabgira "400 tablet on; 420 orientation right-up; 480 list"
if tt_log tabgira | grep -q 'camada "mide-bar" em (0, 0) 768x28' && tt_log tabgira | grep -q "rotação: tela HEADLESS-1 em _270"; then
    ok "modo tablet: a tela gira com o sensor"
else
    falha "modo tablet: a tela gira com o sensor"
fi
# Trava de rotação: com ela, o sensor não gira a tela.
mkdir -p "$TT/trava-cfg/mide"
printf '[display]\nrotation_lock = true\n' > "$TT/trava-cfg/mide/config.toml"
XDG_CONFIG_HOME="$TT/trava-cfg" tt_run trava "400 orientation left-up; 480 list"
if tt_log trava | grep -q 'camada "mide-bar" em (0, 0) 1366x28' && ! tt_log trava | grep -q "rotação: tela"; then
    ok "trava de rotação impede o giro"
else
    falha "trava de rotação impede o giro"
fi
# Escala 1,5: girada e nítida (o tamanho lógico troca: 768/1,5 = 512 de largura).
mkdir -p "$TT/e15-cfg/mide"
printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.5\n' > "$TT/e15-cfg/mide/config.toml"
MIDE_WINIT_SIZE=2049x1152 XDG_CONFIG_HOME="$TT/e15-cfg" tt_run e15 "400 orientation bottom-up; 480 list" 470
if tt_log e15 | grep -q "rotação: tela HEADLESS-1 em _180" && tt_log e15 | grep -q 'camada "mide-bar" em (0, 0) 1366x28'; then
    ok "escala 1,5: a tela gira de cabeça para baixo"
else
    falha "escala 1,5: a tela gira de cabeça para baixo"
fi
# Toque com a tela girada: o dedo em pixels do painel passa pela rotação (`touch panel`).
# Depois do giro a janela "Toque" fica em (26, 91) 400x300 (lógico); o centro dela,
# (226, 241), fica no painel em (1125, 226) com left-up (_90: o topo da cena na borda
# direita) e em (241, 542) com right-up (_270).
if [ -x "$JANELA" ]; then
    tt_run giro90 "350 orientation left-up; 420 touch panel down 1125 226; 424 touch up; 470 list" 460 "$JANELA toque"
    PIX="$(tt_pixel giro90 1125 226)"
    if echo "$PIX" | awk -F, '{ exit !($2 > 150 && $1 < 100 && $3 < 120) }'; then
        ok "tela girada (left-up): o toque cai no lugar certo do app"
    else
        falha "tela girada (left-up): o toque cai no lugar certo do app (pixel $PIX)"
    fi
    tt_run giro270 "350 orientation right-up; 420 touch panel down 241 542; 424 touch up; 470 list" 460 "$JANELA toque"
    PIX="$(tt_pixel giro270 241 542)"
    if echo "$PIX" | awk -F, '{ exit !($2 > 150 && $1 < 100 && $3 < 120) }'; then
        ok "tela girada (right-up): o toque cai no lugar certo do app"
    else
        falha "tela girada (right-up): o toque cai no lugar certo do app (pixel $PIX)"
    fi
    # Janela maximizada (dois toques na barra) se refaz no tamanho da tela em pé.
    tt_run maxgira "400 touch down 300 61; 402 touch up; 406 touch down 300 61; 408 touch up; 450 orientation left-up; 520 list" "" "$JANELA comum"
    if tt_log maxgira | grep -qE 'janela "Comum" popups=0 posição=\(0, 58\) visível=768x'; then
        ok "janela maximizada ocupa a largura da tela girada"
    else
        falha "janela maximizada ocupa a largura da tela girada ($(tt_log maxgira | grep -o 'janela "Comum" popups=0 posição=([0-9, ]*) visível=[0-9x]*' | tail -1))"
    fi
fi
# Deitado (o sensor diz `undefined`): fica a última orientação.
tt_run deitado "400 orientation left-up; 440 orientation undefined; 480 list"
if tt_log deitado | grep -q 'camada "mide-bar" em (0, 0) 768x28' && tt_log deitado | grep -q "fica a última"; then
    ok "aparelho deitado mantém a última orientação"
else
    falha "aparelho deitado mantém a última orientação"
fi
# O giro é animado: no meio dele (giro de 3 s no teste), a foto da cena antiga gira e
# encolhe, e os cantos do painel ficam pretos.
MIDE_ROTATE_MS=3000 tt_run anima "300 orientation left-up; 600 list" 330
CORNER="$(tt_pixel anima 2 2)"
if tt_log anima | grep -q "rotação: tela HEADLESS-1 em _90" && echo "$CORNER" | awk -F, '{ exit !($1 < 12 && $2 < 12 && $3 < 12) }'; then
    ok "o giro da tela é animado (a cena gira no meio)"
else
    falha "o giro da tela é animado (canto $CORNER)"
fi
# Nitidez em 1,5 numa tela 2256x1504 (a do Chromebook): de cabeça para baixo, a barra e
# a janela saem iguais, pixel a pixel, à tela normal desvirada (sem esticar a imagem).
if [ -x "$JANELA" ]; then
    mkdir -p "$TT/n15-cfg/mide"
    printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.5\n' > "$TT/n15-cfg/mide/config.toml"
    MIDE_WINIT_SIZE=2256x1504 XDG_CONFIG_HOME="$TT/n15-cfg" tt_run n15a "420 list" 400 "$JANELA comum"
    MIDE_WINIT_SIZE=2256x1504 XDG_CONFIG_HOME="$TT/n15-cfg" tt_run n15b "300 orientation bottom-up; 420 list" 400 "$JANELA comum"
    DIFF="$(python3 -c "
import sys
from PIL import Image, ImageChops
a = Image.open(sys.argv[1]).convert('RGB'); b = Image.open(sys.argv[2]).convert('RGB').rotate(180)
n = 0
for box in [(0, 0, 700, 42), (60, 110, 700, 560)]:
    h = ImageChops.difference(a.crop(box), b.crop(box)).convert('L').histogram()
    n += sum(h[20:])
print(n)" "$TT/n15a/frame.png" "$TT/n15b/frame.png" 2>/dev/null)"
    if [ "$DIFF" = "0" ]; then
        ok "escala 1,5 girada: barra e janela nítidas (iguais à tela normal)"
    else
        falha "escala 1,5 girada: barra e janela nítidas ($DIFF pixels diferentes)"
    fi
fi
rm -rf "$TT"
