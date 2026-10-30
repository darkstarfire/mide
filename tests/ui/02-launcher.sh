# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== launcher (ctrl+shift+space)"
# Grade em 1366x768: ícones de 90 px, células de 144x149, 8 colunas a partir de
# x=107, 3 linhas a partir de y=160; o primeiro item fica em (179, 210) e o segundo
# em (323, 210). Abaixo da grade (683, 650) é vazio.
LAUNCH="400 combo ctrl+shift+space"
check "atalho abre o launcher" 'camada "mide-launcher" em \(0, 0\) 1366x768' "$LAUNCH; 460 list"
check_count "Esc fecha" 'camada "mide-launcher"' 0 "$LAUNCH; 450 key esc; 480 list"
check_count "atalho de novo fecha" 'camada "mide-launcher"' 0 "$LAUNCH; 450 combo ctrl+shift+space; 480 list"
check_count "clique no vazio fecha" 'camada "mide-launcher"' 0 "$LAUNCH; 450 click 683 650; 480 list"
check "buscar e Enter abre o app" "$CALC_OPEN" "$LAUNCH; 440 type calc; 450 key enter; 600 list"
check "buscar e clicar abre o app" "$CALC_OPEN" "$LAUNCH; 440 type calc; 450 click 179 210; 600 list"

check "roda para baixo vai para a página da direita" "página 0 → 1" "$LAUNCH; 450 move 683 384; 460 wheel down; 500 list"
check "roda para cima volta para a página da esquerda" "página 1 → 0" "$LAUNCH; 450 move 683 384; 460 wheel down; 500 wheel up; 540 list"
check "clique do meio vai para a próxima página" "página 0 → 1" "$LAUNCH; 450 mclick 683 384; 490 list"
# Pasta Utilitários (2 páginas) na página 2 da grade, em (346, 397).
# A pasta (Utilitários) muda de página e de lugar conforme os apps instalados (as
# pastas ficam no fim da grade): o launcher registra onde ela fica a cada página
# mostrada; o teste passa as páginas, acha a pasta e volta lá para clicar nela.
ULINE="$(timeout 150 "$SCEN" "$LAUNCH; 440 key pagedown; 455 key pagedown; 470 key pagedown; 485 key pagedown; 520 list" 2>&1 | grep -oE "pasta Utilitários na página [0-9]+ em \([0-9]+, [0-9]+\)" | head -1)"
if [ -n "$ULINE" ]; then
    UPAGE="$(echo "$ULINE" | sed -E 's/.*página ([0-9]+) em.*/\1/')"
    UPOS="$(echo "$ULINE" | grep -oE "[0-9]+, [0-9]+" | tr -d ',')"
    UKEYS=""
    for i in $(seq 1 "$UPAGE"); do UKEYS="$UKEYS; $((425 + 15 * i)) key pagedown"; done
    T=$((440 + 15 * UPAGE + 20))
    check "roda dentro da pasta troca de página (carrossel)" "página da pasta 0 → 1" "$LAUNCH$UKEYS; $T click $UPOS; $((T + 30)) move 683 384; $((T + 40)) wheel down; $((T + 80)) list"
else
    echo "pulo  roda dentro da pasta troca de página (a pasta Utilitários não apareceu nas 4 primeiras páginas)"
fi
check "Configurações aparecem no launcher" 'janela "Configurações do Sistema"' "$LAUNCH; 440 type configuracoes_do_sistema; 450 key enter; 560 list"
check_count "clicar num app da dock fecha o launcher" 'camada "mide-launcher"' 0 "$LAUNCH; 460 click 654 728; 520 list"

echo "== launcher: arrastar, pastas e renomear (configuração própria)"
LCFG="$(mktemp -d)"
OLD_LCFG="$XDG_CONFIG_HOME"
export XDG_CONFIG_HOME="$LCFG"
# As pastas padrão dependem dos apps instalados: conta as do começo.
LFOLDERS="$(timeout 150 "$SCEN" "$LAUNCH; 460 list" 2>&1 | grep -oE "e [0-9]+ pasta" | head -1 | grep -oE "[0-9]+")"
# Clique direito num app: o menu abre onde se clicou (itens em y = 231 e 261).
LDESK="$(mktemp -d)"
check "clique direito: Adicionar à dock fixa o app e a dock mostra" "dock: .* fixado por fora" "$LAUNCH; 460 rclick 179 210; 480 click 260 231; 640 list"
MIDE_DESKTOP_DIR="$LDESK" check "clique direito: Adicionar à área de trabalho cria o atalho" "launcher: atalho de .* na área de trabalho" "$LAUNCH; 460 rclick 179 210; 480 click 260 261; 520 list"
if ls "$LDESK"/*.desktop >/dev/null 2>&1 && [ -x "$(ls "$LDESK"/*.desktop | head -1)" ]; then
    echo "ok    o atalho na área de trabalho é a cópia executável do .desktop"; PASS=$((PASS + 1))
else
    echo "FALHA o atalho na área de trabalho é a cópia executável do .desktop"; FAIL=$((FAIL + 1))
fi
rm -rf "$LDESK"
rm -f "$LCFG/mide/dock.toml"
# Arrastar só no modo de edição: segurar o ícone 1 s (150 quadros) antes de mover.
DRAG="460 press 179 210; 610 move 200 210; 615 move 260 210; 620 move 323 210; 625 release 323 210"
check "arrastar app sobre app cria uma pasta" "pasta .*\(2 apps\)" "$LAUNCH; $DRAG; 680 list"
check "pasta fica salva" "em [0-9]+ lugar\(es\) e $((LFOLDERS + 1)) pasta" "$LAUNCH; 460 list"
check "clicar no nome da pasta e renomear" "pasta renomeada para gravar" "$LAUNCH; 460 click 179 210; 490 click 683 258; 510 key backspace; 511 key backspace; 512 key backspace; 513 key backspace; 514 key backspace; 520 type gravar; 530 key enter; 560 list"
check "arrastar para fora tira o app da pasta" "saiu da pasta gravar" "$LAUNCH; 460 click 179 210; 490 press 611 368; 640 move 560 420; 645 move 400 520; 650 move 200 600; 655 release 150 620; 700 list"
# Três dedos no touchpad: o launcher entra acompanhando os dedos e, soltos depois de
# um terço do caminho, termina de abrir; para baixo, sai; pouco caminho volta.
check "três dedos para cima abrem o launcher" "gesto: launcher aberto" "400 swipe 0 -200; 404 swipe end; 560 list"
check "três dedos para baixo fecham o launcher" "gesto: launcher fechado" "400 swipe 0 -200; 404 swipe end; 560 swipe 0 200; 564 swipe end; 660 list"
check "pouco caminho para cima: o launcher volta" "gesto: launcher fechado \(em 13%\)" "400 swipe 0 -40; 404 swipe end; 560 list"
export XDG_CONFIG_HOME="$OLD_LCFG"
rm -rf "$LCFG"
