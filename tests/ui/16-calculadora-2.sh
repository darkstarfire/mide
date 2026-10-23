# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
source tests/ui/calculadora-comum.sh
echo "== Calculadora (parte 2)"
calc_window "Calculadora: RPN no Básico ganha a fileira da pilha" 'janela "Calculadora" .*visível=250x438' "400 list" 'rpn = true'
calc_window "Calculadora: Ctrl+R liga o RPN" 'janela "Calculadora" .*visível=250x438' "400 combo ctrl+r; 440 list"
calc_screen "Calculadora: RPN no Básico, LASTx pela tecla da tela" "4" "400 type 3; 410 key enter; 420 type 4+; 430 click 262 187; 460 list" 'rpn = true'
calc_window "Calculadora: histórico também no Programador" 'janela "Calculadora" .*visível=750x560' "400 list" 'mode = "programador"\nhistory = true'
calc_screen "Calculadora: a base do Programador fica lembrada" "FF" "400 type ff; 440 list" 'mode = "programador"\nbase = "hex"'
calc_window "Calculadora: Ctrl+U liga o conversor (janela 64 pt mais alta)" 'janela "Calculadora" .*visível=250x444' "400 combo ctrl+u; 440 list"
calc_config "Calculadora: o conversor fica lembrado" '^convert = true'
calc_window "Calculadora: conversor com RPN no Básico" 'janela "Calculadora" .*visível=250x502' "400 list" 'convert = true\nrpn = true'
calc_window "Calculadora: o Programador não mostra o conversor" 'janela "Calculadora" .*visível=520x560' "400 combo ctrl+3; 440 list" 'convert = true'
calc_convert "Calculadora: ⇅ troca as unidades (100 °C = 212 °F)" "100 °C = 212 °F" "400 type 100; 420 click 73 159; 450 list" 'convert = true\nconvert_pairs = "temperatura=f>c"'
calc_log "Calculadora: o conversor converte a cada tecla (100 °F)" "conversão 100 °F = 37,7777777778 °C"
calc_screen "Calculadora: ↓ usa a conversão na conta (2 + 12,5 pol em cm = 33,75)" "33,75" "400 type 2+12,5; 410 key down; 420 type =; 450 list" 'convert = true'
calc_screen "Calculadora: Ctrl+Z desfaz a conversão" "12,5" "400 type 2+12,5; 410 key down; 420 combo ctrl+z; 450 list" 'convert = true'
calc_screen "Calculadora: no RPN, ↓ troca o X (7 pol = 17,78 cm)" "17,78" "400 type 3; 410 key enter; 420 type 4+; 430 key down; 460 list" 'convert = true\nrpn = true'
calc_convert "Calculadora: seletor de unidades, buscar e Enter (milha)" "1 mi = 160.934,4 cm" "400 type 1; 410 combo ctrl+l; 440 type milha; 470 key enter; 500 list" 'convert = true'
calc_config "Calculadora: o par escolhido fica lembrado" 'convert_pairs = "comprimento=mi>cm'
calc_convert "Calculadora: unidade de outra categoria leva o par dela (°F → °C)" "1 °F = -17,2222222222 °C" "400 type 1; 410 combo ctrl+l; 440 type fahrenheit; 470 key enter; 500 list" 'convert = true'
calc_screen "Calculadora: Esc fecha o seletor sem apagar a conta" "12,5" "400 type 12,5; 410 combo ctrl+l; 440 key esc; 470 list" 'convert = true'
# O nome da unidade da linha de baixo (78, 195) abre o seletor para ela.
calc_convert "Calculadora: clicar no nome da linha de baixo escolhe a unidade dela" "1 pol = 0,0254 m" "400 type 1; 420 click 78 195; 450 type metro; 480 key enter; 510 list" 'convert = true'
