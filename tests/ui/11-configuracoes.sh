# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== Configurações do Sistema (configuração própria)"
check "menu do mide abre as Configurações" "sistema \(teste\): mide-settings" "400 click 25 14; 430 click 100 88; 460 list"
SCFG="$(mktemp -d)"
mkdir -p "$SCFG/mide"
printf '# Meu config\n[keybindings]\n"ctrl+shift+t" = "launch:foot" # terminal\n' > "$SCFG/mide/config.toml"
OLD_SCFG="$XDG_CONFIG_HOME"
export XDG_CONFIG_HOME="$SCFG"
SETTINGS="$PWD/target/debug/mide-settings"
# Papéis de parede do sistema de teste (os da máquina mudam de uma distribuição para
# outra): só "Beach". Janela em (48, 76): miniatura "Beach" em (548, 250) (abaixo do
# interruptor "Clicar no papel de parede mostra a mesa"); interruptor
# de Sombras em (807, 169).
export MIDE_BACKGROUNDS_DIR="$SCFG/backgrounds"
mkdir -p "$MIDE_BACKGROUNDS_DIR"
python3 -c 'import sys; from PIL import Image; Image.new("RGB", (640, 400), (40, 120, 200)).save(sys.argv[1])' "$MIDE_BACKGROUNDS_DIR/Beach.jpg"
timeout 150 "$SCEN" "400 click 548 250; 460 list" -- "$SETTINGS wallpaper" >/dev/null 2>&1
if grep -q "wallpaper = \"$MIDE_BACKGROUNDS_DIR/Beach" "$SCFG/mide/config.toml" && grep -q "# terminal" "$SCFG/mide/config.toml"; then
    echo "ok    escolher papel de parede grava (e mantém os comentários)"; PASS=$((PASS + 1))
else
    echo "FALHA escolher papel de parede grava"; FAIL=$((FAIL + 1))
fi
check "compositor usa o papel de parede gravado" "papel de parede: $MIDE_BACKGROUNDS_DIR/Beach" "400 list; 420 list" "$SETTINGS wallpaper"
timeout 150 "$SCEN" "400 click 807 169; 460 list" -- "$SETTINGS effects" >/dev/null 2>&1
if grep -q "shadows = false" "$SCFG/mide/config.toml"; then
    echo "ok    desligar sombras grava no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA desligar sombras grava no config.toml"; FAIL=$((FAIL + 1))
fi
# Efeitos: "Desfoque na dock" em (807, 393) e "Desfoque no launcher" em (807, 449).
timeout 150 "$SCEN" "400 click 807 393; 440 click 807 449; 480 list" -- "$SETTINGS effects" >/dev/null 2>&1
if grep -q "dock_blur = false" "$SCFG/mide/config.toml" && grep -q "launcher_blur = false" "$SCFG/mide/config.toml"; then
    echo "ok    interruptores de desfoque gravam no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA interruptores de desfoque gravam no config.toml"; FAIL=$((FAIL + 1))
fi
# O teste anterior desligou o desfoque da dock; o vidro líquido depende dele.
sed -i '/dock_blur = false/d' "$SCFG/mide/config.toml"
# Aparência: a lista em (789, 162) abre com apertar e soltar; "Escuro" em (680, 227).
# Grava no config.toml e a janela fica escura na hora (fundo em (600, 400)).
timeout 150 "$SCEN" "300 move 789 162; 310 press 789 162; 320 release 789 162; 360 click 680 227; 440 list" 430 -- "$SETTINGS appearance" >/dev/null 2>&1
if grep -q 'theme = "dark"' "$SCFG/mide/config.toml" && [ "$(python3 -c "import sys; from PIL import Image; print('%d,%d,%d' % Image.open(sys.argv[1]).convert('RGB').getpixel((600, 400)))" "${SCENARIO_OUT:-/tmp/mide-scenario}/frame.png" 2>/dev/null)" = "30,30,32" ]; then
    echo "ok    Aparência: tema escuro grava e a janela muda na hora"; PASS=$((PASS + 1))
else
    echo "FALHA Aparência: tema escuro"; FAIL=$((FAIL + 1))
fi
sed -i '/^\[appearance\]/d; /^theme = "dark"/d' "$SCFG/mide/config.toml"
# Teclado → "Adicionar layout" em (778, 204) (apertar e soltar); "Inglês (EUA)" em
# (605, 268). A lista fica com o do sistema e o novo, e o compositor monta os dois.
# (A linha do layout não sai no resumo do cenário: confere no mide.log.)
timeout 150 "$SCEN" "300 move 778 204; 310 press 778 204; 320 release 778 204; 370 click 605 268; 420 list; 480 list" -- "$SETTINGS keyboard" >/dev/null 2>&1
if grep -aq "teclado: layout br,us" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log"; then
    echo "ok    Teclado: adicionar layout vale na hora"; PASS=$((PASS + 1))
else
    echo "FALHA Teclado: adicionar layout vale na hora"; FAIL=$((FAIL + 1))
fi
if grep -q 'layouts = "br,us"' "$SCFG/mide/config.toml"; then
    echo "ok    Teclado: layouts gravados no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA Teclado: layouts gravados no config.toml"; FAIL=$((FAIL + 1))
fi
# Com dois layouts, Ctrl+Super+Espaço passa ao dos EUA: a tecla do acento do ABNT2
# escreve "[".
: > "$SCFG/layout.txt"
timeout 150 "$SCEN" "400 click 400 300; 420 combo ctrl+super+space; 440 key leftbrace; 450 key a; 480 combo ctrl+s; 540 list" -- "$PWD/target/debug/mide-code '$SCFG/layout.txt'" >/dev/null 2>&1
if [ "$(cat "$SCFG/layout.txt")" = "[a" ]; then
    echo "ok    Teclado: Ctrl+Super+Espaço troca o layout"; PASS=$((PASS + 1))
else
    echo "FALHA Teclado: Ctrl+Super+Espaço troca o layout ($(cat "$SCFG/layout.txt"))"; FAIL=$((FAIL + 1))
fi
# Seletor na barra (B18): "BR" em (1256, 14); no menu, "Inglês (EUA)" em (1190, 78).
: > "$SCFG/layout.txt"
timeout 150 "$SCEN" "400 click 1256 14; 440 click 1190 78; 480 click 400 300; 500 key leftbrace; 510 key a; 540 combo ctrl+s; 600 list" -- "$PWD/target/debug/mide-code '$SCFG/layout.txt'" >/dev/null 2>&1
if [ "$(cat "$SCFG/layout.txt")" = "[a" ]; then
    echo "ok    barra: seletor de layout troca o layout"; PASS=$((PASS + 1))
else
    echo "FALHA barra: seletor de layout troca o layout ($(cat "$SCFG/layout.txt"))"; FAIL=$((FAIL + 1))
fi
sed -i '/^layouts = "br,us"/d' "$SCFG/mide/config.toml"
# Grupos de atalhos recolhem (B19): o título "Janelas" em (330, 443), abaixo do cartão da repetição de teclas; fica gravado.
timeout 150 "$SCEN" "300 click 330 443; 360 list" -- "$SETTINGS keyboard" >/dev/null 2>&1
if grep -q 'shortcuts_closed = "Janelas"' "$SCFG/mide/config.toml"; then
    echo "ok    Teclado: grupo de atalhos recolhe (e fica lembrado)"; PASS=$((PASS + 1))
else
    echo "FALHA Teclado: grupo de atalhos recolhe"; FAIL=$((FAIL + 1))
fi
sed -i '/^shortcuts_closed = /d' "$SCFG/mide/config.toml"
# Energia: "Suspender o computador após" em (783, 162) (apertar e soltar); "30 minutos"
# é o terceiro item (y=253).
timeout 150 "$SCEN" "300 move 783 162; 310 press 783 162; 320 release 783 162; 370 click 700 253; 420 list" -- "$SETTINGS energy" >/dev/null 2>&1
if grep -q 'suspend_minutes = 30' "$SCFG/mide/config.toml"; then
    echo "ok    Energia: suspender após 30 minutos grava"; PASS=$((PASS + 1))
else
    echo "FALHA Energia: suspender após 30 minutos grava"; FAIL=$((FAIL + 1))
fi
sed -i '/^suspend_minutes = 30/d' "$SCFG/mide/config.toml"
# Som → saídas da placa simulada: "DELL U2717D" (HDMI 1) em (350, 327).
# (o log do app vai para um arquivo: o roteiro só mostra o do compositor)
timeout 150 "$SCEN" "300 click 350 327; 360 list" -- "RUST_LOG=mide=info $SETTINGS sound >'$SCFG/som.log' 2>&1" >/dev/null 2>&1
if grep -aq "sistema (teste): pactl set-default-sink [^ ]*HiFi__HDMI1__sink" "$SCFG/som.log"; then
    echo "ok    Som: escolher o monitor HDMI como saída (simulado)"; PASS=$((PASS + 1))
else
    echo "FALHA Som: escolher o monitor HDMI como saída (simulado)"; FAIL=$((FAIL + 1))
fi
# Som → rolar até o fim; "Som de alerta" em (794, 534) (apertar e soltar); "Gota" em (675, 445).
timeout 150 "$SCEN" "300 move 600 400; 310 wheel 20; 340 move 794 534; 350 press 794 534; 360 release 794 534; 400 click 675 445; 440 list" -- "$SETTINGS sound" >/dev/null 2>&1
if grep -q 'alert = "gota"' "$SCFG/mide/config.toml"; then
    echo "ok    Som: escolher o som de alerta grava"; PASS=$((PASS + 1))
else
    echo "FALHA Som: escolher o som de alerta grava"; FAIL=$((FAIL + 1))
fi
sed -i '/^alert = "gota"/d' "$SCFG/mide/config.toml"
# Idioma e região: a lista em (768, 162); "Português (Brasil)" é o 12º item (y=460)
# quando os idiomas da lista estão instalados (Debian com locales-all).
if locale -a 2>/dev/null | grep -q '^pt_BR.utf8' && [ "$(locale -a | grep -cE '^(de_DE|en_GB|en_US|es_AR|es_ES|es_MX|fr_FR|it_IT|nl_NL|pt_BR|pt_PT|ru_RU|zh_CN|ja_JP)\.utf8')" = 14 ]; then
    timeout 150 "$SCEN" "300 move 768 162; 310 press 768 162; 320 release 768 162; 370 click 688 460; 420 list" -- "$SETTINGS region" >/dev/null 2>&1
    if grep -q 'lang = "pt_BR.UTF-8"' "$SCFG/mide/config.toml"; then
        echo "ok    Idioma e região: escolher o idioma grava"; PASS=$((PASS + 1))
    else
        echo "FALHA Idioma e região: escolher o idioma grava"; FAIL=$((FAIL + 1))
    fi
    sed -i '/^lang = "pt_BR.UTF-8"/d' "$SCFG/mide/config.toml"
else
    echo "pulo  Idioma e região (os idiomas da lista não estão todos instalados)"
fi
# Apps padrão: só os apps de teste (XDG_DATA_DIRS próprio); "Navegador" em (783, 162),
# com "Nenhum", "Navegador A" e "Navegador B" (y=253). Grava no mide-mimeapps.list (B25: só
# vale na sessão do mide); o mimeapps.list geral (Cinnamon, GNOME) fica como estava.
APPS_DIR="$SCFG/apps-teste"
printf '[Default Applications]\nx-scheme-handler/http=outro.desktop\n' >"$SCFG/mimeapps.list"
mkdir -p "$APPS_DIR/applications"
for n in A B; do printf '[Desktop Entry]\nType=Application\nName=Navegador %s\nExec=true\nMimeType=x-scheme-handler/http;text/html;\n' "$n" > "$APPS_DIR/applications/navegador-$(echo "$n" | tr AB ab).desktop"; done
timeout 150 "$SCEN" "300 move 783 162; 310 press 783 162; 320 release 783 162; 370 click 700 253; 420 list" -- "XDG_DATA_DIRS='$APPS_DIR' XDG_DATA_HOME='$APPS_DIR' $SETTINGS default-apps" >/dev/null 2>&1
if grep -q '^x-scheme-handler/http=navegador-b.desktop' "$SCFG/mide-mimeapps.list" 2>/dev/null \
    && [ "$(cat "$SCFG/mimeapps.list")" = "$(printf '[Default Applications]\nx-scheme-handler/http=outro.desktop')" ]; then
    echo "ok    Apps padrão: escolher o navegador grava no mide-mimeapps.list (o geral fica igual)"; PASS=$((PASS + 1))
else
    echo "FALHA Apps padrão: escolher o navegador grava no mide-mimeapps.list (o geral fica igual)"; FAIL=$((FAIL + 1))
fi
rm -rf "$APPS_DIR" "$SCFG/mimeapps.list" "$SCFG/mide-mimeapps.list"
# "Vidro líquido" em (807, 505): grava no config.toml e a dock troca para o vidro clássico.
check "Efeitos → Vidro líquido desligado: a dock volta ao vidro clássico" "vidro líquido na dock: desligado" "400 click 807 505; 480 list" "$SETTINGS effects"
if grep -q "liquid_glass = false" "$SCFG/mide/config.toml"; then
    echo "ok    interruptor do vidro líquido grava no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA interruptor do vidro líquido grava no config.toml"; FAIL=$((FAIL + 1))
fi
# Seção Dock: arrastar o trilho até o fim grava 64 px e a dock cresce sozinha.
check "Configurações → Dock: tamanho dos ícones muda a dock" 'camada "mide-dock" em \([0-9]+, [0-9]+\) [0-9]+x81' "400 list; 420 press 500 161; 425 move 600 161; 430 move 700 161; 435 move 790 161; 440 release 790 161; 520 list" "$SETTINGS dock"
# Dock: "Ocultar e mostrar a Dock automaticamente" em (807, 212); a dock desce e esconde.
# Ocultar é inteligente: só esconde com janela sobre a dock (aqui, maximizando a janela
# pelo botão verde, que no Tahoe fica dentro da barra lateral, em (115, 102)).
check "Configurações → Dock: ocultar automaticamente esconde a dock sob uma janela" "dock escondida" "400 click 807 212; 500 click 115 102; 800 list" "$SETTINGS dock"
if grep -q "autohide = true" "$SCFG/mide/config.toml"; then
    echo "ok    ocultar a dock grava no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA ocultar a dock grava no config.toml"; FAIL=$((FAIL + 1))
fi
sed -i '/autohide = true/d' "$SCFG/mide/config.toml"
# Barra superior (B12): "Configurações" (engrenagem) é o segundo interruptor da lista,
# em (808, 293), depois de "Controles de mídia" (808, 237). Escondido, some da barra na hora
# (a barra relê a cada meio segundo).
RUST_LOG="mide=debug,mide_bar=info" check "Configurações → Barra superior: esconder a engrenagem tira o item da barra" "itens da barra: media sound wifi bluetooth battery control$" "400 click 808 293; 520 list" "$SETTINGS bar"
if grep -q "gear = false" "$SCFG/mide/config.toml"; then
    echo "ok    esconder item da barra grava no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA esconder item da barra grava no config.toml"; FAIL=$((FAIL + 1))
fi
sed -i '/gear = false/d' "$SCFG/mide/config.toml"
# Mouse e trackpad: "Rolagem natural no mouse" em (807, 266), "Toque para clicar" em
# (807, 416); o trilho da velocidade vai de x=488 a 778 em y=162 (meio = padrão).
timeout 150 "$SCEN" "400 click 807 266; 430 click 807 416; 460 press 633 162; 465 move 560 162; 470 move 500 162; 475 release 488 162; 520 list" -- "$SETTINGS mouse" >/dev/null 2>&1
if grep -q "mouse_natural_scroll = true" "$SCFG/mide/config.toml" && grep -q "tap_to_click = false" "$SCFG/mide/config.toml" && grep -q "pointer_speed = -1" "$SCFG/mide/config.toml"; then
    echo "ok    Mouse e trackpad gravam no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA Mouse e trackpad gravam no config.toml"; FAIL=$((FAIL + 1))
fi
python3 - "$SCFG/mide/config.toml" <<'PY'
import re, sys
p = sys.argv[1]
s = open(p).read()
s = re.sub(r"(?ms)^\[input\]\n(.*?)(?=^\[|\Z)", "", s)
open(p, "w").write(s)
PY
# Busca na barra lateral (campo em cápsula, embaixo dos botões da janela, em (162, 146)): "teclas" acha "Teclas no estilo Mac"; o
# resultado (primeira linha, y=158) abre a seção Teclado, com o interruptor em (807, 169).
timeout 150 "$SCEN" "400 click 162 146; 420 type teclas; 460 click 500 158; 500 click 807 253; 560 list" -- "$SETTINGS" >/dev/null 2>&1
if grep -q "mac_style = true" "$SCFG/mide/config.toml"; then
    echo "ok    busca nas Configurações leva à opção (teclas no estilo Mac gravado)"; PASS=$((PASS + 1))
else
    echo "FALHA busca nas Configurações leva à opção (teclas no estilo Mac gravado)"; FAIL=$((FAIL + 1))
fi
sed -i '/mac_style = true/d' "$SCFG/mide/config.toml"
# Teclado → Atalhos (A9): busca em (600, 403); buscando "fechar", "Fechar a janela"
# (Ctrl ⇧ Q) fica em (500, 340) e "Fechar a janela" (Alt F4) em (500, 298); no
# diálogo, "Gravar…" em (525, 315). O compositor grava a combinação apertada.
timeout 150 "$SCEN" "400 click 600 403; 410 type fechar; 440 click 500 530; 470 click 525 315; 500 combo ctrl+alt+k; 560 list" -- "$SETTINGS keyboard" >/dev/null 2>&1
if grep -q '"ctrl+alt+k" = "close_window"' "$SCFG/mide/config.toml" && grep -q '"ctrl+shift+q" = "none"' "$SCFG/mide/config.toml"; then
    echo "ok    gravar atalho nas Configurações troca a combinação"; PASS=$((PASS + 1))
else
    echo "FALHA gravar atalho nas Configurações troca a combinação"; FAIL=$((FAIL + 1))
fi
check "atalho gravado vale no compositor" "fechar janela pedido pelo atalho" "400 combo ctrl+alt+k; 460 list" "$SETTINGS keyboard"
check_count "combinação antiga desligada (a janela continua aberta)" 'janela "Configurações do Sistema"' 1 "400 combo ctrl+shift+q; 460 list" "$SETTINGS keyboard"
# Combinação já usada (Ctrl+Espaço é do buscador): pede confirmação, não grava nem abre o buscador.
check_count "gravando, o atalho não dispara" 'camada "mide-search"' 0 "400 click 600 403; 410 type fechar; 440 click 500 488; 470 click 525 315; 500 combo ctrl+space; 560 list" "$SETTINGS keyboard"
if ! grep -q '"ctrl+space" = "close_window"' "$SCFG/mide/config.toml"; then
    echo "ok    combinação de outra ação pede confirmação"; PASS=$((PASS + 1))
else
    echo "FALHA combinação de outra ação pede confirmação"; FAIL=$((FAIL + 1))
fi
# "Restaurar os padrões" (buscando algo sem resultado, a lista some) em (510, 292): a
# seção de atalhos sai do arquivo.
timeout 150 "$SCEN" "400 click 600 403; 410 type zzz; 440 click 510 471; 500 list" -- "$SETTINGS keyboard" >/dev/null 2>&1
if ! grep -q "keybindings" "$SCFG/mide/config.toml"; then
    echo "ok    restaurar os atalhos padrão"; PASS=$((PASS + 1))
else
    echo "FALHA restaurar os atalhos padrão"; FAIL=$((FAIL + 1))
fi
export XDG_CONFIG_HOME="$OLD_SCFG"
unset MIDE_BACKGROUNDS_DIR
rm -rf "$SCFG"
