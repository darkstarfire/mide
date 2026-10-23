# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
source tests/ui/calculadora-comum.sh
echo "== Calculadora (parte 3)"
# Moeda (E3b, etapa 2): cotações do Banco Central pelo curl falso.
calc_convert "Moeda: 100 USD em BRL pela cotação de 21/10" "100 USD = 502,77 BRL (BCB 2026-10-21)" "400 type 100; 500 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"\ncurrency_online = true'
calc_calls "Moeda: um pedido ao Banco Central" 1
CALC_KEEP_DATA=1 calc_convert "Moeda: reabrir no mesmo dia usa o cache" "100 USD = 502,77 BRL (BCB 2026-10-21)" "400 type 100; 500 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"\ncurrency_online = true'
calc_calls "Moeda: reabrir no mesmo dia não busca de novo" 1
calc_run "400 key enter; 460 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"'
calc_config "Moeda: a primeira vez pergunta, e o Enter (Buscar) fica lembrado" '^currency_online = true'
calc_calls "Moeda: depois de aceitar, busca" 1
# Sem rede, com o boletim de 20/10 guardado: avisa e usa o de 20/10.
rm -rf "$CDIR/data"; mkdir -p "$CDIR/data/cache/mide/cotacoes" "$CDIR/data/curl"
cp tests/data/bcb/20261020.csv "$CDIR/data/cache/mide/cotacoes/bcb.csv"; touch "$CDIR/data/curl/sem-rede"
CALC_KEEP_DATA=1 calc_convert "Moeda: sem rede vale a cotação guardada (20/10)" "100 USD = 501,00 BRL (BCB 2026-10-20)" "400 key enter; 420 type 100; 500 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"\ncurrency_online = true'
calc_log "Moeda: sem rede, o log registra" "cotações BCB de 2026-10-20 \(sem conexão"
rm -f "$CDIR/data/curl/sem-rede" "$CDIR/data/curl/chamadas"
CALC_KEEP_DATA=1 calc_convert "Moeda: busca desligada usa o cache" "100 USD = 501,00 BRL (BCB 2026-10-20)" "400 type 100; 500 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"\ncurrency_online = false'
calc_calls "Moeda: busca desligada não pede nada" 0
CALC_NO_CURL=1 calc_run "400 list" 'convert = true\nconvert_pairs = "moeda=USD>BRL"\ncurrency_online = true'
calc_log "Moeda: na bateria, sem o curl falso, nada é buscado" "cotações \(teste\): sem MIDE_CURL, nada buscado"
# Menus da Calculadora na barra (num barramento D-Bus de teste, como os do Terminal).
CALC_SCEN=tests/scenario-dbus.sh calc_window "Calculadora: Visualizar > Científica pela barra" 'janela "Calculadora" .*visível=620x380' "500 click 232 14; 540 click 250 79; 580 list"
CALC_SCEN=tests/scenario-dbus.sh calc_window "Calculadora: Visualizar > Conversor de unidades pela barra" 'janela "Calculadora" .*visível=250x444' "500 click 232 14; 540 click 250 208; 580 list"
