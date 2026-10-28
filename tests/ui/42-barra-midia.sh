# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== barra superior: Tocando Agora (MPRIS)"
# Player falso (tests/apps/mpris-teste.py) no barramento de teste. Com ele, a barra
# mostra, à esquerda da engrenagem: nota (abre o painel) em (1041, 14), anterior
# (1070, 14), tocar/pausar (1096, 14) e próxima (1123, 14). No painel aberto pela
# nota, Aleatório em (955, 279) e Repetir em (1122, 279).
PLAYER="python3 $PWD/tests/apps/mpris-teste.py 30"
check "controle rápido: tocar/pausar vai ao player" "controle rápido de mídia PlayPause" "450 click 1096 14; 480 list" "$PLAYER"
check "controle rápido: próxima vai ao player" "controle rápido de mídia Next" "450 click 1123 14; 480 list" "$PLAYER"
check "controle rápido: anterior vai ao player" "controle rápido de mídia Previous" "450 click 1070 14; 480 list" "$PLAYER"
check "nota abre o painel Tocando Agora" "menu aberto: Media" "450 click 1041 14; 490 list" "$PLAYER"
