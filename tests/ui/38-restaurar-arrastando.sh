# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== arrastar a janela maximizada a restaura sob o ponteiro (A27)"
# Ao arrastar uma janela maximizada (ou encaixada) ela volta ao tamanho de antes com o
# ponteiro na mesma fração da largura (aqui 956/1366 = 70%) e na mesma altura a partir
# do topo, igual para a barra do mide, para apps Wayland com barra própria (que pedem
# xdg_toplevel.move) e para apps X11 pelo xwayland-satellite (_NET_WM_MOVERESIZE).
# Janela que nasceu maximizada (sem tamanho de antes) volta a 2/3 do tamanho; app que
# escolhe outro tamanho, ou demora a responder (como os apps X11), fica certo no quadro
# com o tamanho novo. Área útil: (0, 28) 1366x673.
JANELA="$PWD/target/debug/examples/mide-janela-teste"
if [ -x "$JANELA" ]; then
    # Barra do mide: "Comum" 400x300 maximizada pelo duplo clique e arrastada pela barra
    # (y=43, 15 px acima do conteúdo): 956 - 0,7 * 400 = 676; 300 + 15 = 315.
    check "barra do mide: volta ao tamanho de antes, ponteiro a 70% da largura" 'janela "Comum" popups=0 posição=\(676, 315\) visível=400x300' \
        "400 click 300 61; 404 click 300 61; 440 press 956 43; 445 move 956 80; 450 move 956 200; 455 move 956 300; 460 release 956 300; 500 list" "$JANELA comum"
    # Nasceu maximizada (antes, o "tamanho de antes" era 0x0 e o canto da janela grudava
    # no ponteiro): 2/3 de 1366x643 = 910x428; 956 - 0,7 * 910 = 319.
    check "barra do mide: nasceu maximizada, volta a 2/3 sob o ponteiro" 'janela "SSD" popups=0 posição=\(319, 315\) visível=910x428' \
        "400 press 956 43; 405 move 956 80; 410 move 956 200; 415 move 956 300; 420 release 956 300; 460 list" "$JANELA ssd-max"
    # Barra do próprio app (Wayland, pede move): encaixada pelo topo e arrastada.
    check "barra do app (Wayland): volta ao tamanho de antes sob o ponteiro" 'janela "CSD" popups=0 posição=\(606, 288\) visível=500x300' \
        "400 press 300 90; 405 move 400 60; 410 move 650 20; 415 move 683 3; 430 release 683 3; 470 press 956 40; 475 move 956 80; 480 move 956 200; 485 move 956 300; 490 release 956 300; 530 list" "$JANELA csd"
    check "barra do app (Wayland): nasceu maximizada, volta a 2/3 sob o ponteiro" 'janela "CSD" popups=0 posição=\(319, 288\) visível=910x448' \
        "400 press 956 40; 405 move 956 80; 410 move 956 200; 415 move 956 300; 420 release 956 300; 460 list" "$JANELA csd max"
    # App que volta ao tamanho dele (ignora o pedido): 956 - 0,7 * 500 = 606.
    check "app que escolhe o próprio tamanho fica com o ponteiro a 70%" 'janela "CSD" popups=0 posição=\(606, 288\) visível=500x300' \
        "400 press 956 40; 405 move 956 80; 410 move 956 200; 415 move 956 300; 420 release 956 300; 460 list" "$JANELA csd max propria"
    # App lento (300 ms por configure, como os X11): até responder a janela segue o
    # ponteiro no tamanho de maximizada (sem pular para fora dele)...
    check "app lento: antes da resposta a janela segue o ponteiro inteira" 'janela "CSD" popups=0 posição=\(0, 288\) visível=1366x673' \
        "400 press 956 40; 402 move 956 300; 403 list; 404 release 956 300" "$JANELA csd max lento"
    # ... e, mesmo com o botão já solto, fica certa no quadro com o tamanho novo.
    check "app lento: botão solto antes da resposta, posta sob o ponteiro depois" 'janela "CSD" popups=0 posição=\(606, 288\) visível=500x300' \
        "400 press 956 40; 402 move 956 300; 403 release 956 300; 480 list" "$JANELA csd max lento propria"
    # Solto no topo antes da resposta: maximiza de novo (o quadro atrasado não a tira de lá).
    check "app lento: solto no topo antes da resposta continua maximizada" 'janela "CSD" popups=0 posição=\(0, 28\) visível=1366x673' \
        "400 press 956 40; 402 move 956 300; 404 move 683 3; 406 release 683 3; 480 list" "$JANELA csd max lento"
else
    echo "pulo  restaurar arrastando (rode cargo build --examples)"
fi

# App X11 com barra própria (dicas do Motif sem decoração), pelo xwayland-satellite.
if python3 -c "import Xlib" 2>/dev/null && command -v Xwayland >/dev/null && { command -v xwayland-satellite >/dev/null || [ -x "$HOME/.cargo/bin/xwayland-satellite" ]; }; then
    XAPP="sh -c 'sleep 2; DISPLAY=\$(cat \"\$XDG_RUNTIME_DIR/\$MIDE_SOCKET.xdisplay\") exec python3 -I $PWD/tests/x11-janela.py'"
    # Abre no meio (pede "sem decoração" e é pequena: A26) em (433, 229), com a barra do
    # mide em y=214. Duplo clique nela maximiza; arrastar pela barra do app (y=70, 12 px
    # abaixo do conteúdo, pedido de move pelo satellite) volta a 500x300 a 70%.
    MIDE_XWAYLAND=1 SCENARIO_WAIT=45 SCENARIO_TIMEOUT=60 check "app X11: arrastar pela barra do app restaura sob o ponteiro" 'janela "X11 CSD" popups=0 posição=\(606, 288\) visível=500x300' \
        "2000 click 683 214; 2004 click 683 214; 2040 press 956 70; 2045 move 956 90; 2050 move 956 150; 2055 move 956 300; 2080 release 956 300; 2140 list" "$XAPP"
    # Encaixe pelo topo arrastando pela barra do app X11: meio do topo maximiza.
    MIDE_XWAYLAND=1 SCENARIO_WAIT=45 SCENARIO_TIMEOUT=60 check "app X11: arrastar pela barra do app até o topo maximiza" 'janela "X11 CSD" popups=0 posição=\(0, 58\) visível=1366x643' \
        "2000 press 600 245; 2005 move 620 150; 2010 move 660 40; 2015 move 683 3; 2030 release 683 3; 2100 list" "$XAPP"
else
    echo "pulo  restaurar arrastando no X11 (falta python3-xlib, Xwayland ou xwayland-satellite)"
fi
