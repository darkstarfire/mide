# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== menu do app na barra (menu global, barramento D-Bus de teste)"
# App de teste (examples/mide-menu-teste) que publica "Projeto" e "Ajuda" pelo dbusmenu.
MENU_APP="$PWD/target/debug/examples/mide-menu-teste"
# "Projeto" na barra em (186, 14); no menu, "Compilar" é a primeira linha (y=49).
MENU_X=186
if [ -x "$MENU_APP" ]; then
    SCEN_PLAIN="$SCEN"
    SCEN=tests/scenario-dbus.sh
    RUST_LOG="mide=debug,mide_bar=info" check "a barra mostra os menus do app em foco" "menu do app mide-menu-teste: Projeto, Ajuda" "500 list" "$MENU_APP"
    CLICK="$(mktemp)"
    MENU_TESTE_ARQUIVO="$CLICK" timeout 150 "$SCEN" "500 click $MENU_X 14; 540 click $MENU_X 49; 580 list" -- "$MENU_APP" >/dev/null 2>&1
    if [ "$(cat "$CLICK")" = "11" ]; then
        echo "ok    clique no menu do app chega ao app (Projeto → Compilar)"; PASS=$((PASS + 1))
    else
        echo "FALHA clique no menu do app chega ao app (recebeu: '$(cat "$CLICK")')"; FAIL=$((FAIL + 1))
    fi
    rm -f "$CLICK"
    # Menus demais para a tela (1024x768, `MENU_TESTE_MUITOS`: 13 menus): os que não
    # cabem saem da barra e vão para o menu com o nome do app ("Menu de teste" em x=60),
    # como submenus depois de Ocultar/Encerrar. Lá, "Ferramentas" (3º dos 7) fica em
    # y=169; o submenu abre à direita (x≥255) com o item na mesma altura.
    MENU_TESTE_MUITOS=1 MIDE_WINIT_SIZE=1024x768 check "menus que não cabem vão para o menu do nome do app" \
        "menus do app sem espaço: 7 de 13 no menu do nome \(Executar Tarefas, Terminal Integrado, Ferramentas" "500 list" "$MENU_APP"
    CLICK="$(mktemp)"
    MUITOS_OUT="$(mktemp -d)"
    MENU_TESTE_MUITOS=1 MIDE_WINIT_SIZE=1024x768 SCENARIO_OUT="$MUITOS_OUT" MENU_TESTE_ARQUIVO="$CLICK" timeout 150 "$SCEN" \
        "500 click 60 14; 530 move 150 160; 540 move 150 169; 560 move 300 169; 580 list; 600 click 300 169; 640 list" 580 -- "$MENU_APP" >/dev/null 2>&1
    if grep -q "submenu aberto: Native(8)" "$MUITOS_OUT/mide.log" && [ "$(cat "$CLICK")" = "110" ]; then
        echo "ok    submenu no menu do nome abre ao passar e o clique chega ao app (Ferramentas → item)"; PASS=$((PASS + 1))
    else
        echo "FALHA submenu no menu do nome (recebeu: '$(cat "$CLICK")')"; FAIL=$((FAIL + 1))
    fi
    # No quadro: o submenu aberto, com o item destacado em azul em (360, 160).
    if python3 -c "
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert('RGB')
r, g, b = im.getpixel((360, 160))
blue = b > 200 and r < 80
sys.exit(0 if blue else 1)" "$MUITOS_OUT/frame.png" 2>/dev/null; then
        echo "ok    quadro: submenu desenhado ao lado do menu do nome"; PASS=$((PASS + 1))
    else
        echo "FALHA quadro: submenu desenhado ao lado do menu do nome ($MUITOS_OUT/frame.png)"; FAIL=$((FAIL + 1))
    fi
    rm -rf "$CLICK" "$MUITOS_OUT"
    # Escala 1,5 numa tela 1920x1080 (1280x720 em pontos): cabem 8 dos 13.
    MUITOS_CFG="$(mktemp -d)"
    mkdir -p "$MUITOS_CFG/mide"
    printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.5\n' > "$MUITOS_CFG/mide/config.toml"
    XDG_CONFIG_HOME="$MUITOS_CFG" MENU_TESTE_MUITOS=1 MIDE_WINIT_SIZE=1920x1080 check "menus que não cabem, em escala 1,5" \
        "menus do app sem espaço: 5 de 13 no menu do nome \(Ferramentas," "500 list" "$MENU_APP"
    rm -rf "$MUITOS_CFG"
    # Na tela de 1366x768 com os menus de sempre (Projeto, Ajuda), nada vai para o nome.
    if timeout 150 "$SCEN" "500 list" -- "$MENU_APP" 2>&1 | grep -q "menus do app sem espaço"; then
        echo "FALHA menus que cabem ficam na barra"; FAIL=$((FAIL + 1))
    else
        echo "ok    menus que cabem ficam na barra"; PASS=$((PASS + 1))
    fi
    SCEN="$SCEN_PLAIN"
else
    echo "pulo  menu do app (rode cargo build --examples)"
fi
