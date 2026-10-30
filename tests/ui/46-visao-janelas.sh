# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== visão de todas as janelas (quatro dedos)"
# Duas janelas: o Terminal (700x500 em (48, 76)) e a de teste "Comum" (400x300 com a
# barra do mide, a de cima). Quatro dedos para cima andando 200 (2/3 do caminho) e
# soltos: a visão abre; no mosaico, o Terminal fica à direita (o ponto (1000, 300) cai
# nele). A lista vai do fundo para o topo.
JANELA="$PWD/target/debug/examples/mide-janela-teste"
if [ -x "$JANELA" ]; then
    ABRE="400 swipe4 0 -200; 404 swipe4 end"
    check "quatro dedos para cima abrem a visão" "visão das janelas: aberta" "$ABRE; 480 list" "$TERMINAL" "$JANELA"
    check "pouco caminho: a visão volta" "gesto: visão das janelas fechada \\(em 13%\\)" "400 swipe4 0 -40; 404 swipe4 end; 480 list" "$TERMINAL" "$JANELA"
    check_last "clicar numa janela da visão traz ela para a frente" 'janela "Terminal"' "$ABRE; 470 move 1000 300; 475 click 1000 300; 560 list" "$TERMINAL" "$JANELA"
    check "clicar numa janela fecha a visão" "visão das janelas: fechada" "$ABRE; 470 move 1000 300; 475 click 1000 300; 560 list" "$TERMINAL" "$JANELA"
    check_last "Esc fecha a visão sem mudar o foco" 'janela "Comum"' "$ABRE; 470 key esc; 560 list" "$TERMINAL" "$JANELA"
    check "quatro dedos para baixo fecham a visão" "gesto: visão das janelas fechada" "$ABRE; 600 swipe4 0 200; 604 swipe4 end; 680 list" "$TERMINAL" "$JANELA"
    check_last "quatro dedos para o lado trocam a janela da frente" 'janela "Terminal"' "400 swipe4 -200 0; 404 swipe4 end; 480 list" "$TERMINAL" "$JANELA"
    check "Super+W abre a visão" "visão das janelas: aberta" "400 combo super+w; 480 list" "$TERMINAL" "$JANELA"

    # No meio do gesto (1/3 do caminho), as janelas estão a caminho do mosaico: o
    # Terminal, que começa em x=48, já saiu para a direita (o fundo aparece em (60, 400)).
    VJ="$(mktemp -d)"
    run_frame() { # run_frame pasta escala roteiro quadro
        mkdir -p "$VJ/$1/cfg/mide" "$VJ/$1/out"
        [ -n "$2" ] && printf '[monitors.outputs."HEADLESS-1"]\nscale = %s\n' "$2" > "$VJ/$1/cfg/mide/config.toml"
        # Com escala, a tela de 1366x768 pontos em pixels (2049x1152 em 1,5).
        local size="$MIDE_WINIT_SIZE"
        [ -n "$2" ] && size="2049x1152"
        MIDE_WINIT_SIZE="$size" XDG_CONFIG_HOME="$VJ/$1/cfg" SCENARIO_OUT="$VJ/$1/out" timeout 150 "$SCEN" "$3" "$4" -- "$TERMINAL" "$JANELA" > "$VJ/$1/log" 2>&1
    }
    pixel() { python3 -c "import sys; from PIL import Image; print('%d,%d,%d' % Image.open(sys.argv[1]).convert('RGB').getpixel((int(sys.argv[2]), int(sys.argv[3]))))" "$1" "$2" "$3" 2>/dev/null; }
    run_frame antes "" "400 list" 400
    run_frame meio "" "400 swipe4 0 -100; 440 list" 420
    ANTES="$(pixel "$VJ/antes/out/frame.png" 60 400)"
    MEIO="$(pixel "$VJ/meio/out/frame.png" 60 400)"
    if [ "$ANTES" = "255,255,255" ] && [ "$MEIO" != "255,255,255" ] && [ -n "$MEIO" ]; then
        echo "ok    visão no meio do gesto: as janelas acompanham os dedos"; PASS=$((PASS + 1))
    else
        echo "FALHA visão no meio do gesto: as janelas acompanham os dedos (antes $ANTES, meio $MEIO)"; FAIL=$((FAIL + 1))
    fi
    # Em 1,5: a janela sob o ponteiro (a "Comum", à esquerda no mosaico) com o contorno
    # na cor de destaque (azul).
    run_frame escala 1.5 "$ABRE; 600 move 300 300; 640 list" 638
    if grep -q "visão das janelas: aberta" "$VJ/escala/log" && python3 - "$VJ/escala/out/frame.png" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
# A "Comum" fica no mosaico em x=154 (pontos): o contorno, 4 pontos para fora, cai
# entre x=222 e 230 (pixels), num fundo escuro e não azul.
ring = any((lambda p: p[2] > 120 and p[2] > 2 * p[0])(im.getpixel((x, 472))) for x in range(222, 231))
sys.exit(0 if im.size[0] == 2049 and ring else 1)
PY
    then
        echo "ok    visão em escala 1,5 com o destaque"; PASS=$((PASS + 1))
    else
        echo "FALHA visão em escala 1,5 com o destaque"; FAIL=$((FAIL + 1))
    fi
    # Jogo em tela cheia em foco (a janela de teste com a categoria Game): os quatro
    # dedos não fazem nada.
    mkdir -p "$VJ/jogo/data/applications"
    printf '[Desktop Entry]\nType=Application\nName=Jogo\nExec=true\nCategories=Game;\n' > "$VJ/jogo/data/applications/mide-janela-teste.desktop"
    XDG_DATA_HOME="$VJ/jogo/data" check "jogo em tela cheia: quatro dedos ignorados" "gesto: 4 dedos ignorados" "400 combo ctrl+super+f; 480 swipe4 0 -200; 484 swipe4 end; 560 list" "$TERMINAL" "$JANELA"
    rm -rf "$VJ"
else
    echo "FALHA visão das janelas: falta o examples/mide-janela-teste (cargo build --examples)"
    FAIL=$((FAIL + 1))
fi
