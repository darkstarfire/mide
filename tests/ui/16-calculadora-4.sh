# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
source tests/ui/calculadora-comum.sh
echo "== Calculadora (parte 4)"
# A24: "minimizar tudo" duas vezes (a segunda pega a janela ainda saindo) e restaurar:
# a janela volta recebendo cliques (antes ficava na lista das minimizadas e o mide
# ignorava o clique: parecia congelada). O 7 fica em (84, 246).
calc_screen "Super+D duas vezes e restaurar: a janela volta recebendo cliques" "7" "400 combo super+d; 410 combo super+d; 500 restore; 560 click 84 246; 600 list"
calc_window "Calculadora: Ctrl+4 abre as Notas (720x480)" 'janela "Calculadora" .*visível=720x480' "400 combo ctrl+4; 440 list"
calc_run "400 type 120x3=; 420 key enter; 440 type aluguel=1.200; 460 key enter; 480 type aluguel*12=; 500 key enter; 520 type y*2=; 540 key enter; 560 type hoje_gastei_2*3=; 580 key up; 590 key up; 600 key up; 610 key end; 620 key backspace; 625 key backspace; 630 key backspace; 635 key backspace; 640 key backspace; 650 type 1.500; 720 list" 'mode = "notas"'
notes_line "Notas: 120x3= dá 360" 1 " = 360"
calc_log "Notas: variável (aluguel*12= dá 14.400)" "notas: linha 3 = 14\.400"
notes_line "Notas: nome desconhecido dá erro" 4 ": erro: “y” não é uma variável nem uma unidade"
notes_line "Notas: prosa antes da conta (hoje gastei 2*3= dá 6)" 5 " = 6"
notes_line "Notas: mudar a variável em cima refaz a conta de baixo (18.000)" 3 " = 18.000"
calc_file "Notas: a nota fica gravada só com o que foi digitado" "mide/calculadora/notas/*.txt" "$(printf '120x3=\naluguel=1.500\naluguel*12=\ny*2=\nhoje gastei 2*3=')"
calc_run "400 combo shift+9; 410 type 2+3; 420 combo shift+0; 430 type *4=; 440 key enter; 450 type 100+10; 460 combo shift+5; 470 type =; 520 list" 'mode = "notas"'
notes_line "Notas: parênteses ((2+3)*4= dá 20)" 1 " = 20"
notes_line "Notas: porcentagem (100+10%= dá 110)" 2 " = 110"
# O cenário dura 9 s ao todo (SCENARIO_WAIT é o tempo total): a tecla vem perto de 5,5 s
# e a nota é gravada 1 s depois; uma espera de 3,5 s ou mais deixaria a nota sem arquivo.
SCENARIO_WAIT=9 calc_run "400 type 2+2=; 520 list" 'mode = "notas"'
calc_file "Notas: grava 1 s depois da última tecla" "mide/calculadora/notas/*.txt" "2+2="
CALC_KEEP_DATA=1 calc_run "400 list" 'mode = "notas"'
notes_line "Notas: reabrir mostra a nota com o resultado" 1 " = 4"
calc_run "400 type 2+2=; 480 key backspace; 485 key backspace; 490 type 3=; 560 combo ctrl+z; 570 combo ctrl+z; 620 list" 'mode = "notas"'
notes_line "Notas: Ctrl+Z volta a conta de antes (depois de gravar)" 1 " = 4"
calc_run "400 type 1+1=; 420 combo ctrl+n; 460 list" 'mode = "notas"'
calc_log "Notas: Ctrl+N cria outra nota" "nova nota, 2 nota\(s\)"
