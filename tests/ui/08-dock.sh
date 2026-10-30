# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== dock"
check "clique no ícone abre o app" "nova janela criada" "400 click 611 728"
# App em foco: o clique minimiza (como o "minimize or previews" do dash-to-dock); outro
# clique traz de volta, sem abrir outra janela.
check "clique no ícone do app em foco minimiza" "em foco: minimizado pela dock" "400 click 611 728; 480 list" "$TERMINAL"
check_count "clique de novo restaura, sem abrir outro" "janela \"Terminal\"" 1 "400 click 611 728; 460 click 611 728; 540 list" "$TERMINAL"
check "app em foco com duas janelas: o clique mostra as prévias" "em foco com 2 janelas: prévias" "400 click 611 728; 480 list" "$TERMINAL" "$TERMINAL"
# A instância nova (aberta pela dock, sem --title) tem o nome do shell no título.
check_count "ctrl+clique abre nova instância" 'janela "[^"]+"' 2 "390 ctrl on; 400 click 611 728; 480 list" "$TERMINAL"
check "clique no ícone restaura minimizada" "janela restaurada" "400 combo super+h; 440 click 654 728" "$CALCULATOR"
check "Launchpad na dock abre o launcher" 'camada "mide-launcher"' "400 click 568 728; 470 list"
check "abrir pela dock sai do ícone (lâmpada)" "abrindo pela dock: mide-calculator" "400 click 654 728; 700 list"
check "Configurações fixadas na dock abrem" 'janela "Configurações do Sistema"' "400 click 697 728; 560 list"
check "menu do app: Ocultar" "janela minimizada" "400 rclick 611 728; 440 click 611 644" "$TERMINAL"
check "menu do app: Encerrar" "janela fechada" "400 rclick 611 728; 440 click 611 670" "$TERMINAL"
check "menu do app lista as janelas e foca a escolhida" "controle: janela [0-9]+ focada" "400 rclick 611 728; 440 click 611 513; 480 list" "$TERMINAL" "$TERMINAL"
check "prévias ao parar o mouse no ícone" "prévias: 2 janela" "400 move 623 728; 405 move 611 728; 470 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
check "clique na prévia escolhe a janela" "prévia: janela [0-9]+ escolhida" "400 move 623 728; 405 move 611 728; 470 move 520 598; 480 click 503 588; 520 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
# Com o Launchpad aberto, escolher uma janela nas prévias fecha o launcher.
check_count "prévia com o launcher aberto: escolher fecha o launcher" 'camada "mide-launcher"' 0 "400 combo ctrl+shift+space; 460 move 623 728; 465 move 611 728; 530 move 520 598; 540 click 503 588; 600 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
check "× da prévia fecha a janela" "prévia: fechar janela" "400 move 623 728; 405 move 611 728; 470 move 520 598; 475 move 600 533; 480 click 600 533; 540 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
check "clique direito abre o menu do app" "menu de Terminal aberto" "400 rclick 611 728; 470 list" "$TERMINAL"
check "clique no ícone durante a animação de minimizar restaura" "janela restaurada" "400 combo super+h; 420 click 654 728" "$CALCULATOR"

# Tema de ícones do mide (fork do WhiteSur): instalado numa pasta de dados de teste a
# partir do clone (o do mide-update ou o do lado deste projeto); sem clone, pulado.
ICONSRC=""
for d in "${XDG_CACHE_HOME:-$HOME/.cache}/mide/mide-icons" "$PWD/../mide-icons"; do
    if [ -x "$d/install.sh" ]; then ICONSRC="$d"; break; fi
done
if [ -n "$ICONSRC" ]; then
    IDATA="$(mktemp -d)"
    (cd "$ICONSRC" && ./install.sh -n mide -d "$IDATA/icons" >/dev/null 2>&1)
    XDG_DATA_HOME="$IDATA" RUST_LOG="mide=debug,mide_dock=info" check "dock: ícones dos apps do mide vêm do tema do mide" "ícone mide-terminal: tema do mide" "400 list"
    rm -rf "$IDATA"
else
    echo "pulado: dock com o tema do mide (sem o clone do mide-icons)"
fi
# A dica com o nome do ícone reaproveita uma camada só: passear por vários ícones
# não cria e destrói camadas a cada um (coincidia com congelamentos da tela).
n="$(timeout 150 "$SCEN" "400 move 611 728; 430 move 654 728; 460 move 697 728; 490 move 611 728; 520 move 700 400; 560 list" 2>&1 | grep -c "camada 'mide-dock-tip' criada")"
if [ "$n" = 1 ]; then
    echo "ok    dica da dock: uma camada só passeando pelos ícones"; PASS=$((PASS + 1))
else
    echo "FALHA dica da dock: uma camada só passeando pelos ícones ($n camadas)"; FAIL=$((FAIL + 1))
fi
# Ocultar automaticamente: com o ponteiro nas prévias das janelas (fora da dock), ela
# não se recolhe; só depois de o ponteiro ir embora.
AH="$(mktemp -d)"
mkdir -p "$AH/mide"
printf '[dock]\nautohide = true\n' > "$AH/mide/config.toml"
# O roteiro anda por quadros (com a tela parada, não anda): o ponteiro já está nas
# prévias quando a janela é maximizada por cima da dock; depois, 9 s de espera (sem
# o ponteiro nas prévias, nesse tempo ela se esconde).
AHLOG="$(XDG_CONFIG_HOME="$AH" SCENARIO_WAIT=9 timeout 150 "$SCEN" "400 move 623 728; 405 move 611 728; 470 move 520 598; 480 move 503 590; 490 combo super+up; 520 list" -- "$TERMINAL" "$TERMINAL" 2>&1 | grep -E "dock (à mostra|escondida)|prévias: |janela por cima")"
if echo "$AHLOG" | grep -q "prévias: " && ! echo "$AHLOG" | grep -q "dock escondida"; then
    echo "ok    ocultar automaticamente: com o ponteiro nas prévias, a dock não se recolhe"; PASS=$((PASS + 1))
else
    echo "FALHA ocultar automaticamente: com o ponteiro nas prévias, a dock não se recolhe"; FAIL=$((FAIL + 1))
    echo "$AHLOG" | tail -5 | sed 's/^/      /'
fi
rm -rf "$AH"
# Prévias no tema escuro: o painel fica grafite (o título vem em letra clara).
# Espiar: com o ponteiro parado numa prévia, a janela dela aparece por cima das outras
# (só no desenho); antes dos 400 ms, a ordem é a de sempre.
PREV="$(mktemp -d)"
mkdir -p "$PREV/cfg/mide"
printf '[appearance]\ntheme = "dark"\n' > "$PREV/cfg/mide/config.toml"
prev_pixel() { # prev_pixel quadro x y: cor do quadro tirado no tempo pedido
    XDG_CONFIG_HOME="$PREV/cfg" SCENARIO_OUT="$PREV/$1" timeout 150 "$SCEN" "400 click 611 728; 480 move 503 590; 520 list" "$1" -- "$TERMINAL" "$TERMINAL" >/dev/null 2>&1
    python3 -c "import sys; from PIL import Image; print('%d,%d,%d' % Image.open(sys.argv[1]).convert('RGB').getpixel((int(sys.argv[2]), int(sys.argv[3]))))" "$PREV/$1/frame.png" "$2" "$3" 2>/dev/null
}
PANEL="$(prev_pixel 470 611 685)"
if echo "$PANEL" | awk -F, '{ exit !($1 < 90 && $2 < 90 && $3 < 95) }'; then
    echo "ok    prévias no tema escuro: painel grafite"; PASS=$((PASS + 1))
else
    echo "FALHA prévias no tema escuro: painel grafite ($PANEL)"; FAIL=$((FAIL + 1))
fi
# No ponto (98,143) fica o cursor do terminal de trás quando ele está por cima.
BEFORE="$(python3 -c "import sys; from PIL import Image; print('%d,%d,%d' % Image.open(sys.argv[1]).convert('RGB').getpixel((98, 143)))" "$PREV/470/frame.png" 2>/dev/null)"
AFTER="$(prev_pixel 620 98 143)"
if [ "$BEFORE" = "255,255,255" ] && echo "$AFTER" | awk -F, '{ exit !($1 < 150) }'; then
    echo "ok    prévia sob o ponteiro: a janela dela aparece por cima"; PASS=$((PASS + 1))
else
    echo "FALHA prévia sob o ponteiro: a janela dela aparece por cima (antes $BEFORE, depois $AFTER)"; FAIL=$((FAIL + 1))
fi
rm -rf "$PREV"
# Lâmpada que inverte: clicar no ícone de novo no meio de minimizar traz a janela de
# volta de onde a lâmpada está (sem recomeçar).
check "lâmpada: clique no meio de minimizar inverte" "lâmpada invertida em [0-9]+%" "400 click 611 728; 415 click 611 728; 480 list" "$TERMINAL"
# Com a ampliação ligada, o alvo da lâmpada é o ícone ampliado (maior que os 40 do
# tamanho normal), onde ele está no momento.
LAMPCFG="$(mktemp -d)"; mkdir -p "$LAMPCFG/mide"
printf '[dock]\nmagnification = true\n' > "$LAMPCFG/mide/config.toml"
XDG_CONFIG_HOME="$LAMPCFG" check "lâmpada: com a ampliação, o alvo é o ícone ampliado" "alvo no ícone da dock em .* w: ([5-9][0-9]|1[0-9][0-9]), h:" "380 move 600 728; 390 move 611 728; 420 click 611 728; 480 list" "$TERMINAL"
rm -rf "$LAMPCFG"
