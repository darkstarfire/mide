# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
source tests/ui/calculadora-comum.sh
echo "== Calculadora (mide-calculator: modos, contas, RPN e histórico)"
calc_screen "Calculadora: precedência pelo teclado (2+3*4= dá 14)" "14" "400 type 2+3*4=; 440 list"
calc_screen "Calculadora: clicar nas teclas (7 × 8 = 56)" "56" "400 click 84 246; 410 click 262 246; 420 click 143 246; 430 click 262 421; 460 list"
calc_screen "Calculadora: precisão entre contas (1 ÷ 3 = × 3 = dá 1)" "1" "400 type 1/3=*3=; 440 list"
calc_screen "Calculadora: desfazer (Ctrl+Z) volta antes do =" "3" "400 type 12+3=; 420 combo ctrl+z; 450 list"
calc_screen "Calculadora: refazer (Ctrl+Shift+Z)" "15" "400 type 12+3=; 420 combo ctrl+z; 430 combo ctrl+shift+z; 460 list"
calc_screen "Calculadora: 2 casas decimais, sem separador de milhar" "1234,50" "400 type 1234,5=; 440 list" 'decimals = 2\nthousands = false'
calc_window "Calculadora: o modo lembrado abre no tamanho dele" 'janela "Calculadora" .*visível=620x380' "400 list" 'mode = "cientifica"'
calc_window "Calculadora: Ctrl+3 abre o Programador" 'janela "Calculadora" .*visível=520x560' "400 combo ctrl+3; 440 list"
calc_window "Calculadora: Ctrl+T mostra o histórico ao lado" 'janela "Calculadora" .*visível=480x380' "400 combo ctrl+t; 440 list"
calc_screen "Calculadora: Científica, sen 30° = 0,5" "0,5" "400 type 30; 420 click 145 362; 450 list" 'mode = "cientifica"'
calc_screen "Calculadora: Científica, 2 ( 3 + 4 ) = 14 (multiplicação implícita)" "14" "400 type 2; 410 click 84 187; 420 type 3+4; 430 click 145 187; 440 type =; 470 list" 'mode = "cientifica"'
calc_screen "Calculadora: Programador, 255 em HEX é FF" "FF" "400 type 255; 420 click 84 95; 450 list" 'mode = "programador"'
calc_screen "Calculadora: Programador, clicar no bit 4 de 1 dá 17" "17" "400 type 1; 420 click 488 233; 450 list" 'mode = "programador"'
calc_screen "Calculadora: RPN, 3 Enter 4 + dá 7" "7" "400 type 3; 410 key enter; 420 type 4+; 450 list" 'rpn = true'
calc_screen "Calculadora: o número passa do Programador para a Básica" "255" "400 type 255; 420 combo ctrl+1; 450 list" 'mode = "programador"'
# ⌫ na tela (Básica) em (73, 136); na Científica, 2nd em (84, 246), yˣ (com 2nd) em
# (328, 246) e EE em (388, 362); no Básico com RPN, a fileira da pilha em y=187 (LASTx
# em x=262) e a janela 58 pt mais alta.
calc_screen "Calculadora: ⌫ na tela apaga o último algarismo" "12" "400 type 123; 420 click 73 136; 450 list"
calc_screen "Calculadora: 2nd troca eˣ por yˣ (3 yˣ 2 = 8)" "8" "400 type 3; 410 click 84 246; 420 click 328 246; 430 type 2=; 460 list" 'mode = "cientifica"'
calc_screen "Calculadora: EE mostra o expoente enquanto digita (2 EE 3)" "2e3" "400 type 2; 410 click 388 362; 420 type 3; 450 list" 'mode = "cientifica"'
