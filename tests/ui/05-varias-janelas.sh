# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== várias janelas"
check "cascata (segunda em 80,108)" "$CALC popups=0 posição=\(80, 108\)" "400 list" "$TERMINAL" "$CALCULATOR"
check_last "clique traz para frente (topo = terminal)" "janela \"Terminal\"" "400 click 700 300; 440 list" "$TERMINAL" "$CALCULATOR"
# Pilha de foco: clicar no Terminal, depois na Calculadora (por cima) e fechar a
# Calculadora com Alt+F4: o foco volta ao Terminal, a anterior da pilha. A Calculadora
# (80, 108) 300x490 fica atrás do Terminal; só a faixa de baixo (y > 576) aparece.
check "fechar a janela em foco passa o foco à anterior da pilha" "foco: passa para a próxima janela da pilha \(Terminal\)" "400 click 700 300; 430 click 200 590; 460 combo alt+f4; 520 list" "$TERMINAL" "$CALCULATOR"
