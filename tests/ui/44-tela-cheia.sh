# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== tela cheia como no Mac"
# examples/mide-janela-teste cheia: janela "Cheia" 500x300 com a barra do mide em
# (48, 76) (barra em y=46..76, verde em (103, 61)); um clique no conteúdo pede ao
# compositor a tela cheia (xdg_toplevel.set_fullscreen, como o F11), outro sai; `ja`
# pede antes do primeiro quadro (como um jogo). Em tela cheia a janela ocupa a tela
# inteira (0, 0) 1366x768 sem a barra do mide; a barra superior e a dock somem. Com o
# ponteiro na borda de cima, a barra superior (28) e a faixa com a barra de título da
# janela descem: verde da faixa em (55, 43). ⌃⌘F é Ctrl+Super+F (sem o estilo Mac).
JANELA="$PWD/target/debug/examples/mide-janela-teste"
if [ -x "$JANELA" ]; then
    CHEIA='janela "Cheia" popups=0 posição=\(0, 0\) visível=1366x768 deslocamento=\(0, 0\) barra=0'
    ANTES='janela "Cheia" popups=0 posição=\(48, 76\) visível=500x300'
    check "verde entra em tela cheia" "$CHEIA" "400 click 103 61; 480 list" "$JANELA cheia"
    check "tela cheia: barra superior escondida" 'camada "mide-bar" escondida pela tela cheia' "400 click 103 61; 480 list" "$JANELA cheia"
    check "tela cheia: dock escondida" 'camada "mide-dock" escondida pela tela cheia' "400 click 103 61; 480 list" "$JANELA cheia"
    check "⌃⌘F entra em tela cheia" "$CHEIA" "400 combo ctrl+super+f; 480 list" "$JANELA cheia"
    check "⌃⌘F de novo volta à geometria de antes" "$ANTES" "400 combo ctrl+super+f; 480 combo ctrl+super+f; 560 list" "$JANELA cheia"
    check "app pede tela cheia (F11)" "$CHEIA" "400 click 300 200; 480 list" "$JANELA cheia"
    check "app sai da tela cheia e volta ao lugar" "$ANTES" "400 click 300 200; 480 click 300 200; 560 list" "$JANELA cheia"
    check "app pede tela cheia antes do primeiro quadro" "$CHEIA" "400 list" "$JANELA cheia ja"
    # Borda de cima: a barra desce (parada 0,3 s ou empurrando) e a faixa também.
    check "ponteiro no topo revela a barra" "tela cheia: barra revelada" "400 click 103 61; 480 move 600 0; 520 move 601 0; 560 list" "$JANELA cheia"
    check "barra revelada aparece na lista" 'camada "mide-bar" em \(0, 0\) 1366x28' "400 click 103 61; 480 move 600 0; 520 move 601 0; 560 list" "$JANELA cheia"
    check "ponteiro sai e a barra sobe" "tela cheia: barra escondida" "400 click 103 61; 480 move 600 0; 520 move 601 0; 560 move 600 300; 600 list" "$JANELA cheia"
    check "verde da faixa revelada sai da tela cheia" "$ANTES" "400 click 103 61; 480 move 600 0; 520 move 601 0; 560 move 55 43; 580 click 55 43; 660 list" "$JANELA cheia"
    check "vermelho da faixa revelada fecha" "janela fechada" "400 click 103 61; 480 move 600 0; 520 move 601 0; 560 move 15 43; 580 click 15 43; 660 list" "$JANELA cheia"
    # O duplo clique e o ⌥ (Alt) no verde continuam maximizando (área útil (0, 28) 1366x673).
    check "duplo clique na barra continua maximizando" 'janela "Cheia" popups=0 posição=\(0, 58\) visível=1366x643' "400 click 300 61; 404 click 300 61; 480 list" "$JANELA cheia"
    check "⌥+clique no verde maximiza" 'janela "Cheia" popups=0 posição=\(0, 58\) visível=1366x643' "398 keydown alt; 400 click 103 61; 402 keyup alt; 480 list" "$JANELA cheia"
    check "maximizada volta maximizada ao sair da tela cheia" 'janela "Cheia" popups=0 posição=\(0, 58\) visível=1366x643' "400 click 300 61; 404 click 300 61; 460 combo ctrl+super+f; 540 combo ctrl+super+f; 620 list" "$JANELA cheia"
    # Alt+Tab para outra janela: ela fica por cima e a barra e a dock voltam.
    check "Alt+Tab: a janela continua em tela cheia, atrás" 'janela "Cheia" em tela cheia' "400 click 520 200; 480 combo alt+tab; 560 list" "$JANELA cheia" "$JANELA comum"
    check_count "Alt+Tab para outra janela traz a barra e a dock de volta" "escondida pela tela cheia" 0 "400 click 520 200; 480 combo alt+tab; 560 list" "$JANELA cheia" "$JANELA comum"
    # Maximizada, com a barra revelada, escondida e revelada de novo: o verde da faixa
    # volta à maximizada (0, 58) 1366x643; e de novo pelo ⌃⌘F, com outra revelação.
    MAXI='janela "Cheia" popups=0 posição=\(0, 58\) visível=1366x643'
    REVELA="move 600 0"
    check "maximizada: revelar, esconder, revelar e sair volta maximizada" "$MAXI" "400 click 300 61; 404 click 300 61; 460 click 55 43; 500 $REVELA; 520 move 601 0; 560 move 600 300; 600 $REVELA; 620 move 601 0; 660 move 55 43; 680 click 55 43; 760 list" "$JANELA cheia"
    check_last "maximizada: entrar e sair de novo com a barra revelada" "$MAXI" "400 click 300 61; 404 click 300 61; 460 click 55 43; 500 $REVELA; 520 move 601 0; 560 move 55 43; 580 click 55 43; 640 combo ctrl+super+f; 680 $REVELA; 700 move 601 0; 740 move 55 43; 760 click 55 43; 840 list" "$JANELA cheia"
    # Clique sem mexer o ponteiro (toque no touchpad parado) vai para a janela que está
    # sob ele agora: depois de entrar na tela cheia pelo verde, o toque chega ao app (que
    # sai da tela cheia); depois do Alt+Tab, à janela trazida para a frente (o app pede a
    # tela cheia). Antes, ia para a superfície de antes da mudança, ou para nenhuma.
    check "toque parado depois do verde chega ao app" "$ANTES" "400 click 103 61; 480 tap; 560 list" "$JANELA cheia"
    check "toque parado depois do Alt+Tab chega à janela da frente" "$CHEIA" "400 move 300 200; 420 combo alt+tab; 460 tap; 540 list" "$JANELA cheia" "$JANELA comum"
    # Estilo Mac: ⌘Tab, Tab com o ⌘ apertado anda no alternador (antes, soltar o Tab já
    # escolhia, e só dava para trocar entre as duas de cima).
    FSMAC="$(mktemp -d)"
    mkdir -p "$FSMAC/mide"
    printf '[keyboard]\nmac_style = true\n' >"$FSMAC/mide/config.toml"
    XDG_CONFIG_HOME="$FSMAC" check "estilo Mac: ⌘Tab Tab chega à terceira janela" "alternador: janela 3 de 3" "400 keydown alt; 410 key tab; 430 key tab; 450 keyup alt; 500 list" "$JANELA cheia" "$JANELA comum" "$JANELA csd"
    XDG_CONFIG_HOME="$FSMAC" check_last "estilo Mac: ⌘Tab Tab traz a terceira para a frente" 'janela "Cheia"' "400 keydown alt; 410 key tab; 430 key tab; 450 keyup alt; 500 list" "$JANELA cheia" "$JANELA comum" "$JANELA csd"
    XDG_CONFIG_HOME="$FSMAC" check_last "estilo Mac: soltar o ⌘ escolhe a janela" 'janela "Comum"' "400 keydown alt; 410 key tab; 450 keyup alt; 500 list" "$JANELA cheia" "$JANELA comum" "$JANELA csd"
    rm -rf "$FSMAC"
    # Dois monitores: só a tela da janela fica em tela cheia.
    MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 check "dois monitores: a tela da janela em tela cheia" "tela HEADLESS-1 em tela cheia" "400 click 103 61; 480 list" "$JANELA cheia"
    MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 check_count "dois monitores: a outra tela não muda" "tela HEADLESS-2 em tela cheia" 0 "400 click 103 61; 480 list" "$JANELA cheia"
    MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 check_count "dois monitores: só a tela da janela no modo de jogo" "tela HEADLESS-2 no modo de jogo" 0 "400 click 103 61; 480 list" "$JANELA cheia"
    # Modo de jogo em tela cheia (A18): a tela fica só para a janela (cena com ela só),
    # a barra e a dock pausam as tarefas periódicas e voltam (com o relógio em dia) ao
    # sair; maximizada não liga; o interruptor em Efeitos desliga.
    JOGO="$(timeout 150 "$SCEN" "400 click 103 61; 480 list; 560 combo ctrl+super+f; 640 list" -- "$JANELA cheia" 2>&1)"
    for esperado in "autoteste: tela HEADLESS-1 no modo de jogo" "modo tela cheia: ligado na tela HEADLESS-1 .*scanout direto" \
        "desfoque\(s\) e [0-9]+ textura\(s\) soltos da tela HEADLESS-1" "barra: pausada \(modo tela cheia\)" "dock: pausada \(modo tela cheia\)" \
        "modo tela cheia: desligado na tela HEADLESS-1" "barra: de volta do modo tela cheia" "dock: de volta do modo tela cheia"; do
        if echo "$JOGO" | grep -Eq "$esperado"; then
            echo "ok    modo de jogo: $esperado"; PASS=$((PASS + 1))
        else
            echo "FALHA modo de jogo: $esperado"; FAIL=$((FAIL + 1))
        fi
    done
    # Scanout direto (A18): a entrega à placa só existe no DRM; aqui, a escolha. A janela
    # de teste desenha em memória compartilhada (não serve); com a barra revelada, o
    # motivo muda.
    check "scanout direto: buffer em memória compartilhada não vai à placa" "autoteste: scanout direto não: buffer na memória" "400 click 103 61; 480 list" "$JANELA cheia"
    check "scanout direto: barra revelada volta à composição" "autoteste: scanout direto não: barra revelada" "400 click 103 61; 480 move 600 0; 520 move 601 0; 560 list" "$JANELA cheia"
    check_count "modo de jogo: maximizada não liga" "no modo de jogo" 0 "400 click 300 61; 404 click 300 61; 480 list" "$JANELA cheia"
    check "modo de jogo: Alt+Tab para outra janela desliga" "modo tela cheia: desligado na tela HEADLESS-1" "400 click 520 200; 480 combo alt+tab; 560 list" "$JANELA cheia" "$JANELA comum"
    SEMJOGO="$(mktemp -d)"
    mkdir -p "$SEMJOGO/mide"
    printf '[effects]\ngame_mode = false\n' >"$SEMJOGO/mide/config.toml"
    XDG_CONFIG_HOME="$SEMJOGO" check_count "modo de jogo desligado: tela cheia sem o modo" "no modo de jogo" 0 "400 click 103 61; 480 list" "$JANELA cheia"
    XDG_CONFIG_HOME="$SEMJOGO" check "modo de jogo desligado: tela cheia continua igual" "$CHEIA" "400 click 103 61; 480 list" "$JANELA cheia"
    rm -rf "$SEMJOGO"
    # Notificação durante o modo de jogo: sem banner nem som, só na central. O app
    # espera o compositor dizer que o modo ligou (mide-ctl game_mode).
    HIST="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/${MIDE_TEST_SOCKET:-mide-test}-notificacoes"
    rm -f "$HIST"
    AVISO="sh -c 'until mide-ctl game_mode | grep -q on; do sleep 0.2; done; notify-send -a Jogo \"Partida salva\" \"Nível 3\"'"
    NOTA="$(timeout 150 tests/scenario-dbus.sh "400 click 103 61; 700 list" -- "$JANELA cheia" "$AVISO" 2>&1)"
    if echo "$NOTA" | grep -q "guardada na central (modo tela cheia): Partida salva" && ! echo "$NOTA" | grep -q 'camada "mide-notifications"' \
        && ! echo "$NOTA" | grep -q "som (teste): message-new-instant" && grep -q "Partida salva" "$HIST" 2>/dev/null; then
        echo "ok    modo de jogo: notificação só na central, sem banner nem som"; PASS=$((PASS + 1))
    else
        echo "FALHA modo de jogo: notificação só na central, sem banner nem som"; FAIL=$((FAIL + 1))
    fi
    rm -f "$HIST"
    # Três dedos para os lados: da tela cheia para a mesa (a janela comum vem para
    # cima) e de volta; a lista vai do fundo para o topo.
    LADO="400 combo ctrl+super+f; 470 swipe 200 0; 472 swipe end"
    check_last "três dedos para a direita: da tela cheia para a mesa" 'janela "Terminal"' "$LADO; 540 list" "$TERMINAL" "$JANELA cheia"
    check_last "três dedos para a esquerda: de volta à tela cheia" 'janela "Cheia" em tela cheia' "$LADO; 540 swipe -200 0; 542 swipe end; 620 list" "$TERMINAL" "$JANELA cheia"
    check "três dedos sem outra tela cheia: fica onde está" "gesto: já na última" "$LADO; 540 swipe -200 0; 542 swipe end; 600 swipe -200 0; 602 swipe end; 660 list" "$TERMINAL" "$JANELA cheia"
else
    echo "FALHA tela cheia: falta o examples/mide-janela-teste (cargo build --examples)"
    FAIL=$((FAIL + 1))
fi
# App de conteúdo inteiro do mide-ui (Terminal, luzes do app em (69, 95), (89, 95) e
# (109, 95)): o verde pede a tela cheia; em tela cheia, as luzes do app somem.
check "Terminal: verde do app entra em tela cheia" "$FOOT posição=\(0, 0\) visível=1366x768" "400 click 109 95; 480 list" "$TERMINAL"
