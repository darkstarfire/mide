# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== janelas de abertura e diálogos no centro (A26)"
# examples/mide-janela-teste: "fixa" (300x200, mínimo == máximo, como o
# xwayland-satellite manda os splashes do X11), "sem-decoracao" (400x300, modo
# client_side do xdg-decoration), "filho" (Pai 600x400 e Diálogo 300x200 com
# set_parent) e "comum" (400x300, fica na cascata). Área útil da tela principal:
# (0, 28) 1366x673; com a barra do mide (30 px), uma janela 300x200 no meio fica em
# (533, 279).
JANELA="$PWD/target/debug/examples/mide-janela-teste"
if [ -x "$JANELA" ]; then
    check "janela de tamanho fixo (splash) abre no meio da tela" 'janela "Abertura" popups=0 posição=\(533, 279\) visível=300x200' "400 list" "$JANELA fixa"
    check "janela pequena sem decoração (splash) abre no meio da tela" 'janela "Sem decoração" popups=0 posição=\(483, 229\) visível=400x300' "400 list" "$JANELA sem-decoracao"
    check "diálogo com janela-pai abre centrado sobre a pai" 'janela "Diálogo" popups=0 posição=\(198, 176\) visível=300x200' "400 list" "$JANELA filho"
    check "a janela-pai continua na cascata" 'janela "Pai" popups=0 posição=\(48, 76\) visível=600x400' "400 list" "$JANELA filho"
    check "janela comum continua na cascata" 'janela "Comum" popups=0 posição=\(48, 76\) visível=400x300' "400 list" "$JANELA comum"
    # Duas telas: com o ponteiro na segunda (1920x1080 em (1366, 0), sem barra nem dock),
    # o splash abre no meio dela: x = 1366 + (1920 - 300) / 2, y = 30 + (1080 - 230) / 2.
    MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 check "splash abre no meio da tela onde está o ponteiro" 'janela "Abertura" popups=0 posição=\(2176, 455\)' \
        "100 move 1600 300; 500 list" "sh -c 'sleep 4; exec $JANELA fixa'"
else
    echo "pulo  janelas no centro (rode cargo build --examples)"
fi
