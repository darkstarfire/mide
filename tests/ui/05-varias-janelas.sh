# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== várias janelas"
check "cascata (segunda em 80,108)" "$CALC popups=0 posição=\(80, 108\)" "400 list" "$TERMINAL" "$CALCULATOR"
check_last "clique traz para frente (topo = terminal)" "janela \"Terminal\"" "400 click 700 300; 440 list" "$TERMINAL" "$CALCULATOR"
