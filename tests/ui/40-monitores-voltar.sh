# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== monitores: janelas voltam à tela que volta (como no macOS)"
# Duas telas virtuais: HEADLESS-1 1366x768 em (0, 0) (principal, com barra e dock) e
# HEADLESS-2 1920x1080 em (1366, 0), sem barra (área útil = tela inteira). O roteiro
# finge o monitor desconectado e conectado de novo (`unplug`/`plug`, como o HDMI que
# some ao acordar do DPMS). A tela só sai depois de 3 s (folga) e, quando volta, as
# janelas tiradas dela voltam ao lugar e ao estado de antes, se ninguém as mexeu.
# "Comum" (400x300) arrastada pela barra de (300, 61) até (1900, 300) fica em
# (1648, 315); o duplo clique na barra (1900, 300) a maximiza na segunda tela:
# (1366, 30) 1920x1050.
JANELA="$PWD/target/debug/examples/mide-janela-teste"
PARA_SEGUNDA="400 press 300 61; 405 move 600 100; 410 move 1200 200; 415 move 1700 300; 420 move 1900 300; 425 release 1900 300"
MAXIMIZA="460 click 1900 300; 465 click 1900 300"
if [ -x "$JANELA" ]; then
    export MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080
    # Piscada curta (volta em 1 s, dentro da folga): nada muda.
    check "monitor que pisca (1 s) não tira a janela maximizada da tela" 'janela "Comum" popups=0 posição=\(1366, 30\) visível=1920x1050' \
        "$PARA_SEGUNDA; $MAXIMIZA; 520 unplug HEADLESS-2; 580 plug HEADLESS-2; 640 list" "$JANELA comum"
    # Sumiu de vez: a maximizada vai maximizada para a principal...
    check "monitor desconectado: a maximizada vai maximizada para a principal" 'janela "Comum" popups=0 posição=\(0, 58\) visível=1366x643' \
        "$PARA_SEGUNDA; $MAXIMIZA; 520 unplug HEADLESS-2; 800 list" "$JANELA comum"
    # ... e volta maximizada à segunda tela quando ela volta.
    check "monitor de volta: a janela volta maximizada à tela dela" 'janela de volta à tela HEADLESS-2 em \(1366, 30\)' \
        "$PARA_SEGUNDA; $MAXIMIZA; 520 unplug HEADLESS-2; 800 plug HEADLESS-2; 860 list" "$JANELA comum"
    check "monitor de volta: maximizada na tela dela, 1920x1050" 'janela "Comum" popups=0 posição=\(1366, 30\) visível=1920x1050' \
        "$PARA_SEGUNDA; $MAXIMIZA; 520 unplug HEADLESS-2; 800 plug HEADLESS-2; 860 list" "$JANELA comum"
    # Janela solta volta ao mesmo lugar da tela.
    check "monitor de volta: a janela solta volta ao mesmo lugar" 'janela "Comum" popups=0 posição=\(1648, 315\) visível=400x300' \
        "$PARA_SEGUNDA; 440 unplug HEADLESS-2; 700 list; 710 plug HEADLESS-2; 760 list" "$JANELA comum"
    # Solta: vai para o meio da área útil da principal (barra dela em y 199..229).
    check "monitor desconectado: a janela solta vai para o meio da principal" 'janela "Comum" popups=0 posição=\(483, 229\) visível=400x300' \
        "$PARA_SEGUNDA; 440 unplug HEADLESS-2; 700 list" "$JANELA comum"
    # Mudada de lugar pelo usuário enquanto a tela estava fora: fica onde está.
    check "janela mexida enquanto a tela estava fora fica onde está" 'janela "Comum" popups=0 posição=\(383, 329\) visível=400x300' \
        "$PARA_SEGUNDA; 440 unplug HEADLESS-2; 700 press 683 214; 705 move 640 260; 710 move 583 314; 715 release 583 314; 730 plug HEADLESS-2; 790 list" "$JANELA comum"
    # Maior que a principal (1600x900 na segunda tela): encolhe para caber em 90% da
    # área útil, no meio; com a tela de volta, volta ao tamanho de antes.
    GRANDE_SEGUNDA="410 press 300 61; 415 move 900 100; 420 move 1600 100; 425 release 1600 100"
    check "monitor desconectado: janela maior que a principal encolhe e fica no meio" 'janela "Grande" popups=0 posição=\(68, 92\) visível=1229x575' \
        "$GRANDE_SEGUNDA; 450 unplug HEADLESS-2; 700 list" "$JANELA grande"
    check "monitor de volta: a janela encolhida volta ao tamanho de antes" 'janela "Grande" popups=0 posição=\(1366, 115\) visível=1600x900' \
        "$GRANDE_SEGUNDA; 450 unplug HEADLESS-2; 700 plug HEADLESS-2; 780 list" "$JANELA grande"
    unset MIDE_HEADLESS_OUTPUTS
else
    echo "pulo  monitores: janelas voltam (rode cargo build --examples)"
fi
