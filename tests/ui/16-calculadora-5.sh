# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
source tests/ui/calculadora-comum.sh
echo "== Calculadora (parte 5)"
calc_run "400 type 1+1=; 500 click 738 102; 540 list" 'mode = "notas"'
calc_log "Notas: Lixeira pela barra de ferramentas" "notas: na Lixeira "
calc_file "Notas: a nota vai para a Lixeira" "Trash/files/*.txt" "1+1="
calc_run "400 type 120x3=; 500 rclick 345 145; 530 click 380 170; 560 list" 'mode = "notas"'
calc_log "Notas: clique direito no resultado copia ele" "notas: copiado 360"
calc_run "400 type 2x21=; 500 rclick 330 145; 530 click 410 222; 560 list" 'mode = "notas"'
calc_log "Notas: Levar para a calculadora volta ao teclado com o número" "calculadora: tela 42$"
# Unidades e moedas nas notas (E3b, parte 11); a moeda pelo curl falso.
calc_run "400 type 10_km_em_m=; 420 key enter; 440 type 1/2_kg_em_g=; 460 key enter; 480 type 1_h_30_min_em_min=; 540 list" 'mode = "notas"'
notes_line "Notas com unidades: 10 km em m dá 10.000 m" 1 " = 10.000 m"
notes_line "Notas com unidades: 1/2 kg em g dá 500 g" 2 " = 500 g"
notes_line "Notas com unidades: 1 h 30 min em min dá 90 min" 3 " = 90 min"
calc_run "400 type 100_usd_em_brl=; 520 list" 'mode = "notas"\ncurrency_online = true'
notes_line "Notas com moeda: 100 USD em BRL pela cotação (curl falso)" 1 " = R\$ 502,77"
calc_calls "Notas com moeda: a nota pediu as cotações ao Banco Central" 1
calc_run "400 type 2+2=; 460 list" 'mode = "notas"'
calc_calls "Notas sem moeda não buscam cotações" 0
CALC_SCEN=tests/scenario-dbus.sh calc_window "Calculadora: Visualizar > Notas matemáticas pela barra" 'janela "Calculadora" .*visível=720x480' "500 click 232 14; 540 click 250 127; 580 list"
rm -rf "$CDIR"
