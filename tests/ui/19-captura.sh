# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== captura de tela (Print)"
# A imagem vai para Imagens/Capturas de tela: no teste, uma pasta temporária.
SHOTS="$(mktemp -d)"
printf 'XDG_PICTURES_DIR="%s"\n' "$SHOTS" > "$TESTCFG/user-dirs.dirs"
# O Ctrl+S vem uns 11 s depois da partida: com a espera padrão (10 s) o teste ficava no limite.
SCENARIO_WAIT=16 timeout 150 "$SCEN" "400 key print; 600 press 300 200; 610 move 500 350; 620 release 700 450; 660 combo ctrl+s; 720 list" >/dev/null 2>&1
if ls "$SHOTS/Capturas de tela/"*.png >/dev/null 2>&1 && python3 -c "
import sys, glob
from PIL import Image
img = Image.open(glob.glob(sys.argv[1] + '/Capturas de tela/*.png')[0])
sys.exit(0 if img.size == (400, 250) else 1)" "$SHOTS"; then
    echo "ok    captura de uma área salva em Imagens/Capturas de tela (400x250)"; PASS=$((PASS + 1))
else
    echo "FALHA captura de uma área salva em Imagens/Capturas de tela"; FAIL=$((FAIL + 1))
fi
SCENARIO_WAIT=16 check "captura: Enter copia a imagem para a área de transferência" "bytes de image/png copiados" "400 key print; 600 press 300 200; 610 move 500 350; 620 release 700 450; 660 key enter; 720 list"
# Estilo Mac: ⌘S (a tecla Alt) também salva (antes, o Ctrl virava Super e nem o
# Ctrl+S salvava).
MSHOT="$(mktemp -d)"
mkdir -p "$MSHOT/cfg/mide"
printf '[keyboard]\nmac_style = true\n' > "$MSHOT/cfg/mide/config.toml"
printf 'XDG_PICTURES_DIR="%s"\n' "$MSHOT/pics" > "$MSHOT/cfg/user-dirs.dirs"
XDG_CONFIG_HOME="$MSHOT/cfg" SCENARIO_WAIT=16 timeout 150 "$SCEN" "400 key print; 600 press 300 200; 610 move 500 350; 620 release 700 450; 660 combo alt+s; 720 list" >/dev/null 2>&1
if ls "$MSHOT/pics/Capturas de tela/Captura de tela "*.png >/dev/null 2>&1; then
    echo "ok    captura: ⌘S salva no estilo Mac (Imagens/Capturas de tela, nome com data e hora)"; PASS=$((PASS + 1))
else
    echo "FALHA captura: ⌘S salva no estilo Mac"; FAIL=$((FAIL + 1))
fi
rm -rf "$MSHOT"
# Atalhos do Mac: ⌘⇧4 (uma área, salva ao soltar o mouse) e ⌘⇧3 (a tela inteira, sem
# janela), também com o launcher aberto (a imagem sai com ele). Fora do estilo Mac o ⌘
# é a tecla Super; no estilo Mac, a Alt.
shot_test() {
    local name="$1" script="$2" want="$3" cfg pics got
    cfg="$(mktemp -d)"
    pics="$(mktemp -d)"
    mkdir -p "$cfg/mide"
    [ -n "${4:-}" ] && printf '[keyboard]\nmac_style = true\n' > "$cfg/mide/config.toml"
    printf 'XDG_PICTURES_DIR="%s"\n' "$pics" > "$cfg/user-dirs.dirs"
    XDG_CONFIG_HOME="$cfg" SCENARIO_WAIT=14 timeout 150 "$SCEN" "$script" >/dev/null 2>&1
    got="$(python3 -c "
import glob, sys
from PIL import Image
files = glob.glob(sys.argv[1] + '/Capturas de tela/Captura de tela *.png')
print('%dx%d' % Image.open(files[0]).size if files else 'nada')" "$pics" 2>/dev/null)"
    if [ "$got" = "$want" ]; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name (esperado $want, obtido $got)"; FAIL=$((FAIL + 1))
    fi
    rm -rf "$cfg" "$pics"
}
AREA="600 press 300 200; 610 move 450 300; 620 move 600 400; 625 release 600 400; 700 list"
shot_test "captura: ⌘⇧4 salva a área ao soltar o mouse" "400 combo super+shift+4; $AREA" 300x200
shot_test "captura: ⌘⇧3 salva a tela inteira" "400 combo super+shift+3; 600 list" 1366x768
if grep -aq "som (teste): screen-capture" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log"; then
    echo "ok    captura toca o som do obturador"; PASS=$((PASS + 1))
else
    echo "FALHA captura toca o som do obturador"; FAIL=$((FAIL + 1))
fi
shot_test "captura: ⌘⇧3 com o launcher aberto" "400 combo ctrl+shift+space; 470 combo super+shift+3; 650 list" 1366x768
shot_test "captura: ⌘⇧4 no estilo Mac (Alt+Shift+4)" "400 combo alt+shift+4; $AREA" 300x200 mac

# Captura por tela (B22): `screenshot all` salva uma imagem de cada tela, em pixels, e
# responde com a posição, o tamanho em pontos e a escala de cada uma.
SA="$(mktemp -d)"
MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT=8 timeout 150 "$SCEN" "400 list" -- "sh -c 'sleep 3; mide-ctl screenshot all $SA/t > $SA/resposta.txt'" >/dev/null 2>&1
if grep -qP '^HEADLESS-2\t1366\t0\t1920\t1080\t1\t-\t' "$SA/resposta.txt" 2>/dev/null && python3 -c "
import sys
from PIL import Image
d = sys.argv[1]
sys.exit(0 if Image.open(d + '/t-HEADLESS-1.png').size == (1366, 768) and Image.open(d + '/t-HEADLESS-2.png').size == (1920, 1080) else 1)" "$SA" 2>/dev/null; then
    echo "ok    captura: screenshot all salva cada tela (1366x768 e 1920x1080)"; PASS=$((PASS + 1))
else
    echo "FALHA captura: screenshot all salva cada tela ($(head -c 300 "$SA/resposta.txt" 2>/dev/null))"; FAIL=$((FAIL + 1))
fi
rm -rf "$SA"
# Tela em 1,25 (1920x1080 pixels, 1536x864 pontos): a área (300, 200)–(700, 450) em
# pontos sai com 500x313 pixels, do lugar certo da imagem (igual ao quadro sem a
# Captura), e a tela escurece fora dela (antes, nem escurecia e o recorte saía do
# lugar errado).
SC="$(mktemp -d)"
mkdir -p "$SC/cfg/mide"
printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.25\n' > "$SC/cfg/mide/config.toml"
printf 'XDG_PICTURES_DIR="%s"\n' "$SC/pics" > "$SC/cfg/user-dirs.dirs"
XDG_CONFIG_HOME="$SC/cfg" SCENARIO_OUT="$SC/antes" MIDE_HEADLESS_OUTPUTS=1920x1080 SCENARIO_WAIT=8 timeout 150 "$SCEN" "300 list" 300 >/dev/null 2>&1
XDG_CONFIG_HOME="$SC/cfg" SCENARIO_OUT="$SC/durante" MIDE_HEADLESS_OUTPUTS=1920x1080 SCENARIO_WAIT=16 timeout 150 "$SCEN" "400 key print; 600 press 300 200; 610 move 500 350; 620 release 700 450; 660 combo ctrl+s; 720 list" 645 >/dev/null 2>&1
if python3 -c "
import sys, glob
from PIL import Image, ImageChops
d = sys.argv[1]
files = glob.glob(d + '/pics/Capturas de tela/*.png')
if not files: sys.exit('nada salvo')
shot = Image.open(files[0]).convert('RGB')
if shot.size != (500, 313): sys.exit('tamanho %dx%d' % shot.size)
before = Image.open(d + '/antes/frame.png').convert('RGB')
during = Image.open(d + '/durante/frame.png').convert('RGB')
diff = ImageChops.difference(shot, before.crop((375, 250, 875, 563)))
if sum(1 for p in diff.getdata() if max(p) > 40) > 500 * 313 // 100: sys.exit('recorte fora do lugar')
a, b = before.getpixel((100, 100)), during.getpixel((100, 100))
if not (sum(b) < sum(a) * 0.7): sys.exit('não escureceu: %s %s' % (a, b))
" "$SC"; then
    echo "ok    captura em escala 1,25: área de 500x313 no lugar certo, tela escurecida"; PASS=$((PASS + 1))
else
    echo "FALHA captura em escala 1,25"; FAIL=$((FAIL + 1))
fi
rm -rf "$SC"
# ⌘⇧3 em 1366x768 a 1,25: o compositor dá 1093x615 pontos (arredonda para cima), que
# vezes 1,25 seriam 1366x769; o arquivo sai com 1366x768, sem linha preta no fim.
SF="$(mktemp -d)"
mkdir -p "$SF/cfg/mide"
printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.25\n' > "$SF/cfg/mide/config.toml"
printf 'XDG_PICTURES_DIR="%s"\n' "$SF/pics" > "$SF/cfg/user-dirs.dirs"
XDG_CONFIG_HOME="$SF/cfg" SCENARIO_OUT="$SF/scen" SCENARIO_WAIT=14 timeout 150 "$SCEN" "400 combo super+shift+3; 600 list" >/dev/null 2>&1
if python3 -c "
import sys, glob
from PIL import Image
files = glob.glob(sys.argv[1] + '/pics/Capturas de tela/*.png')
if not files: sys.exit('nada salvo')
img = Image.open(files[0]).convert('RGB')
if img.size != (1366, 768): sys.exit('tamanho %dx%d' % img.size)
if all(img.getpixel((x, 767)) == (0, 0, 0) for x in range(0, 1366, 50)): sys.exit('linha preta no fim')
" "$SF"; then
    echo "ok    captura: ⌘⇧3 em 1366x768 a 1,25 sai com 1366x768, sem linha preta"; PASS=$((PASS + 1))
else
    echo "FALHA captura: ⌘⇧3 em 1366x768 a 1,25"; FAIL=$((FAIL + 1))
fi
rm -rf "$SF"

# Gravar a tela (A21, ⌘⇧5), com um ffmpeg de teste (`MIDE_FFMPEG`) que grava os
# argumentos e conta os bytes recebidos; Vídeos numa pasta de teste. Painel: "Gravar"
# em (980, 710); botão de parar na barra em (1220, 14).
REC="$(mktemp -d)"
mkdir -p "$REC/cfg/mide" "$REC/videos"
printf 'XDG_VIDEOS_DIR="%s/videos"\n' "$REC" > "$REC/cfg/user-dirs.dirs"
printf '#!/bin/sh\nfor a; do last=$a; done\nprintf "%%s\\n" "$@" > "$last.args"\nwc -c > "$last"\n' > "$REC/ffmpeg"
chmod +x "$REC/ffmpeg"
rec_file() { ls "$REC/videos/Gravações de tela/"*.mp4 2>/dev/null | head -n 1; }
XDG_CONFIG_HOME="$REC/cfg" MIDE_FFMPEG="$REC/ffmpeg" SCENARIO_WAIT=20 SCENARIO_TIMEOUT=40 timeout 150 "$SCEN" "300 combo super+shift+5; 420 click 980 710; 600 click 1220 14; 700 list" >/dev/null 2>&1
VIDEO="$(rec_file)"
if [ -n "$VIDEO" ] && [ "$(cat "$VIDEO")" -gt $((1366 * 768 * 4 * 10)) ] && grep -qx "1366x768" "$VIDEO.args" && grep -qx "@DEFAULT_MONITOR@" "$VIDEO.args"; then
    echo "ok    gravar a tela: ⌘⇧5, Gravar e parar pela barra (tela inteira, som do sistema)"; PASS=$((PASS + 1))
else
    echo "FALHA gravar a tela: ⌘⇧5, Gravar e parar pela barra"; FAIL=$((FAIL + 1))
fi
rm -rf "$REC/videos"/*
XDG_CONFIG_HOME="$REC/cfg" MIDE_FFMPEG="$REC/ffmpeg" SCENARIO_WAIT=14 timeout 150 "$SCEN" "700 list" -- "sh -c 'sleep 2; mide-ctl record start 100 100 401 301 microfone; sleep 2; mide-ctl record stop'" >/dev/null 2>&1
VIDEO="$(rec_file)"
if [ -n "$VIDEO" ] && grep -qx "400x300" "$VIDEO.args" && grep -qx "@DEFAULT_SOURCE@" "$VIDEO.args" && ! grep -qx "@DEFAULT_MONITOR@" "$VIDEO.args"; then
    echo "ok    gravar a tela: área (lados pares) com o microfone"; PASS=$((PASS + 1))
else
    echo "FALHA gravar a tela: área com o microfone"; FAIL=$((FAIL + 1))
fi
# Com o ffmpeg de verdade: o arquivo é um vídeo H.264 da tela com som (AAC) e dura o
# tempo gravado (4 s, menos o começo descartado).
if command -v ffmpeg >/dev/null && command -v ffprobe >/dev/null; then
    rm -rf "$REC/videos"/*
    XDG_CONFIG_HOME="$REC/cfg" SCENARIO_WAIT=20 SCENARIO_TIMEOUT=40 timeout 150 "$SCEN" "1500 list" -- "$CALCULATOR" "sh -c 'sleep 3; mide-ctl record start full sistema; sleep 4; mide-ctl record stop'" >/dev/null 2>&1
    VIDEO="$(rec_file)"
    INFO="$([ -n "$VIDEO" ] && ffprobe -v error -show_entries stream=codec_name,width,height:format=duration -of compact "$VIDEO" 2>/dev/null | tr '\n' ' ')"
    if echo "$INFO" | grep -q "codec_name=h264|width=1366|height=768" && echo "$INFO" | grep -q "codec_name=aac" && python3 -c "import re,sys; d=float(re.search(r'duration=([0-9.]+)', sys.argv[1]).group(1)); sys.exit(0 if 3.0 <= d <= 5.0 else 1)" "$INFO" 2>/dev/null; then
        echo "ok    gravar a tela com o ffmpeg: vídeo H.264 com som, duração certa"; PASS=$((PASS + 1))
    else
        echo "FALHA gravar a tela com o ffmpeg ($INFO)"; FAIL=$((FAIL + 1))
    fi
else
    echo "pulo  gravar a tela com o ffmpeg de verdade (sem o ffmpeg)"
fi
rm -rf "$REC"

# Captura pelos apps (wlr-screencopy, o que o portal do OBS usa): o grim tira a tela
# inteira e uma região; as imagens têm de ser iguais ao quadro do mide.
if command -v grim >/dev/null; then
    GR="$(mktemp -d)"
    SCENARIO_OUT="$GR/scen" SCENARIO_WAIT=10 timeout 150 "$SCEN" "400 list" 500 -- "$CALCULATOR" "sleep 5; grim $GR/tela.png; grim -g '40,40 300x420' $GR/regiao.png" >/dev/null 2>&1
    if python3 -c "
import sys
from PIL import Image, ImageChops
d = sys.argv[1]
frame = Image.open(d + '/scen/frame.png').convert('RGB')
full = Image.open(d + '/tela.png').convert('RGB')
part = Image.open(d + '/regiao.png').convert('RGB')
same = lambda a, b: a.size == b.size and sum(1 for p in ImageChops.difference(a, b).getdata() if max(p) > 40) < a.size[0] * a.size[1] // 100
sys.exit(0 if same(full, frame) and same(part, frame.crop((40, 40, 340, 460))) else 1)" "$GR"; then
        echo "ok    captura pelos apps (wlr-screencopy): tela inteira e região iguais ao quadro"; PASS=$((PASS + 1))
    else
        echo "FALHA captura pelos apps (wlr-screencopy)"; FAIL=$((FAIL + 1))
    fi
    rm -rf "$GR"
else
    echo "pulado: captura pelos apps (sem o grim)"
fi

# Apps na GPU e captura por dmabuf (A25). Sem o `linux-dmabuf` no compositor, todo app
# com OpenGL/Vulkan caía no llvmpipe (o OBS mostrou isso na sessão) e a captura pelo
# portal saía preta. Confere: o eglinfo de um app Wayland mostra a placa (não o
# llvmpipe); um app X11 (glxinfo pelo XWayland) também; e o exemplo
# `mide-screencopy-dmabuf` (faz o que o portal faz: buffer na GPU, pelo main_device do
# feedback) recebe a tela e uma região iguais às do grim, com buffer linear e com os
# modificadores que a placa prefere (na Intel, comprimido), também em escala 1,25.
DMA_EX="$PWD/target/debug/examples/mide-screencopy-dmabuf"
if [ "${MIDE_TEST_BACKEND:-}" = winit ] || ! [ -r /dev/dri/renderD128 ]; then
    echo "pulado: linux-dmabuf (sem render node ou no winit)"
elif ! [ -x "$DMA_EX" ] || ! command -v eglinfo >/dev/null || ! command -v grim >/dev/null; then
    echo "pulo  linux-dmabuf (rode cargo build --examples; precisa do eglinfo e do grim)"
else
    DM="$(mktemp -d)"
    dma_capture() {
        # $1: pasta; $2: escala (vazio: 1).
        mkdir -p "$1/cfg/mide"
        [ -n "$2" ] && printf '[monitors.outputs."HEADLESS-1"]\nscale = %s\n' "$2" > "$1/cfg/mide/config.toml"
        XDG_CONFIG_HOME="$1/cfg" SCENARIO_OUT="$1/scen" SCENARIO_WAIT=14 timeout 150 "$SCEN" "400 list" -- "$CALCULATOR" \
            "sleep 6; eglinfo -B -p wayland > $1/egl.txt 2>&1; grim $1/grim.png; timeout 10 $DMA_EX $1/lin.png --linear > $1/lin.txt 2>&1; timeout 10 $DMA_EX $1/til.png > $1/til.txt 2>&1; timeout 10 $DMA_EX $1/reg.png --region 40,40,300,420 > $1/reg.txt 2>&1; grim $1/grim2.png" >/dev/null 2>&1
    }
    # As capturas por dmabuf iguais às do grim (antes e depois delas: a tela não mudou)
    # e a região igual ao recorte da tela (em pixels: região em pontos vezes a escala).
    dma_same() {
        python3 -c "
import sys
from PIL import Image, ImageChops
d, k = sys.argv[1], float(sys.argv[2])
g = Image.open(d + '/grim.png').convert('RGB')
# Em 1,25 o grim arredonda a altura para baixo (1366x767): compara a parte comum.
fit = lambda a, b: a.crop((0, 0, min(a.size[0], b.size[0]), min(a.size[1], b.size[1])))
same = lambda a, b: abs(a.size[1] - b.size[1]) <= 1 and a.size[0] == b.size[0] and sum(1 for p in ImageChops.difference(fit(a, b), fit(b, a)).getdata() if max(p) > 40) < a.size[0] * a.size[1] // 100
full = [Image.open(d + '/' + n + '.png').convert('RGB') for n in ('grim2', 'lin', 'til')]
r = lambda v: round(v * k)
part = Image.open(d + '/reg.png').convert('RGB')
crop = full[1].crop((r(40), r(40), r(40) + part.size[0], r(40) + part.size[1]))
sys.exit(0 if all(same(f, g) for f in full) and full[1].size == full[2].size and part.size == (r(300), r(420)) and same(part, crop) else 1)" "$1" "$2" 2>/dev/null
    }
    dma_capture "$DM/k1" ""
    if grep -aq "linux-dmabuf anunciado" "$DM/k1/scen/mide.log" && grep -q "OpenGL ES profile renderer" "$DM/k1/egl.txt" && ! grep "OpenGL ES profile renderer" "$DM/k1/egl.txt" | grep -q llvmpipe; then
        echo "ok    app Wayland com OpenGL na GPU pelo linux-dmabuf ($(grep -m1 'OpenGL ES profile renderer' "$DM/k1/egl.txt" | sed 's/.*renderer: //'))"; PASS=$((PASS + 1))
    else
        echo "FALHA app Wayland com OpenGL na GPU pelo linux-dmabuf ($(grep -m1 'OpenGL ES profile renderer' "$DM/k1/egl.txt" 2>/dev/null))"; FAIL=$((FAIL + 1))
    fi
    if grep -q "^ok .*Linear" "$DM/k1/lin.txt" && grep -q "^ok" "$DM/k1/til.txt" && grep -q "^ok 300x420" "$DM/k1/reg.txt" && dma_same "$DM/k1" 1; then
        echo "ok    captura por dmabuf (linear, modificador da placa e região) igual ao grim"; PASS=$((PASS + 1))
    else
        echo "FALHA captura por dmabuf igual ao grim ($(cat "$DM/k1"/lin.txt "$DM/k1"/til.txt "$DM/k1"/reg.txt 2>/dev/null | tr '\n' ' '))"; FAIL=$((FAIL + 1))
    fi
    dma_capture "$DM/k125" "1.25"
    if grep -q "^ok 375x525" "$DM/k125/reg.txt" && dma_same "$DM/k125" 1.25; then
        echo "ok    captura por dmabuf em escala 1,25 (tela e região em pixels) igual ao grim"; PASS=$((PASS + 1))
    else
        echo "FALHA captura por dmabuf em escala 1,25 ($(cat "$DM/k125"/reg.txt 2>/dev/null))"; FAIL=$((FAIL + 1))
    fi
    # MIDE_NO_DMABUF=1 (diagnóstico): sem o protocolo, os apps voltam à memória
    # compartilhada e a captura do grim continua funcionando.
    MIDE_NO_DMABUF=1 SCENARIO_OUT="$DM/off/scen" SCENARIO_WAIT=10 timeout 150 "$SCEN" "400 list" -- "sleep 4; grim $DM/off/grim.png; timeout 10 $DMA_EX $DM/off/x.png > $DM/off/x.txt 2>&1" >/dev/null 2>&1
    if grep -aq "linux-dmabuf desligado" "$DM/off/scen/mide.log" && [ -s "$DM/off/grim.png" ] && grep -q "não oferece linux-dmabuf" "$DM/off/x.txt"; then
        echo "ok    MIDE_NO_DMABUF=1 desliga o linux-dmabuf e o grim continua"; PASS=$((PASS + 1))
    else
        echo "FALHA MIDE_NO_DMABUF=1"; FAIL=$((FAIL + 1))
    fi
    # App X11 com OpenGL (glxinfo pelo XWayland do mide) também na GPU.
    if command -v glxinfo >/dev/null && { command -v xwayland-satellite >/dev/null || [ -x "$HOME/.cargo/bin/xwayland-satellite" ]; }; then
        MIDE_XWAYLAND=1 SCENARIO_OUT="$DM/x11/scen" SCENARIO_WAIT=14 SCENARIO_TIMEOUT=60 timeout 150 "$SCEN" "400 list" -- "sh -c 'sleep 3; DISPLAY=\$(cat \"\$XDG_RUNTIME_DIR/\$MIDE_SOCKET.xdisplay\") glxinfo -B > $DM/x11/glx.txt 2>&1'" >/dev/null 2>&1
        if grep -q "OpenGL renderer string" "$DM/x11/glx.txt" && ! grep "OpenGL renderer string" "$DM/x11/glx.txt" | grep -q llvmpipe; then
            echo "ok    app X11 com OpenGL na GPU pelo XWayland ($(grep -m1 'OpenGL renderer string' "$DM/x11/glx.txt" | sed 's/.*string: //'))"; PASS=$((PASS + 1))
        else
            echo "FALHA app X11 com OpenGL na GPU ($(grep -m1 'OpenGL renderer string' "$DM/x11/glx.txt" 2>/dev/null))"; FAIL=$((FAIL + 1))
        fi
    else
        echo "pulo  app X11 com OpenGL (falta glxinfo ou xwayland-satellite)"
    fi
    rm -rf "$DM"
fi
# Janela de escolha do monitor (o portal lê o nome dele na saída).
PK="$(mktemp -d)"
MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT=8 timeout 150 "$SCEN" "400 click 300 224; 420 click 231 270; 460 list" -- "$PWD/target/debug/mide-screencast-picker > $PK/saida.txt" >/dev/null 2>&1
if [ "$(cat "$PK/saida.txt" 2>/dev/null)" = "HEADLESS-2" ]; then
    echo "ok    compartilhar a tela: escolher o segundo monitor"; PASS=$((PASS + 1))
else
    echo "FALHA compartilhar a tela: escolher o segundo monitor (saída: '$(cat "$PK/saida.txt" 2>/dev/null)')"; FAIL=$((FAIL + 1))
fi
rm -rf "$PK"
rm -f "$TESTCFG/user-dirs.dirs"
rm -rf "$SHOTS"

# Escala fracionária (wp_fractional_scale): um app GTK 4 numa tela em 1,25 recebe a
# escala exata (150 = 1,25 × 120) e desenha nítido. Precisa do loupe; sem ele, pulado.
if command -v loupe >/dev/null; then
    FS="$(mktemp -d)"
    mkdir -p "$FS/cfg/mide"
    printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.25\n' > "$FS/cfg/mide/config.toml"
    XDG_CONFIG_HOME="$FS/cfg" timeout 150 "$SCEN" "400 list" -- "WAYLAND_DEBUG=1 loupe 2> $FS/wl.log" >/dev/null 2>&1
    if grep -q "preferred_scale(150)" "$FS/wl.log" && grep -q '"xdg_activation_v1"' "$FS/wl.log"; then
        echo "ok    escala fracionária: app GTK 4 recebe 1,25 (e o xdg-activation existe)"; PASS=$((PASS + 1))
    else
        echo "FALHA escala fracionária: app GTK 4 recebe 1,25"; FAIL=$((FAIL + 1))
    fi
    rm -rf "$FS"
else
    echo "pulado: escala fracionária (sem o loupe)"
fi
