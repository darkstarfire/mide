# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== Monitor de Recursos (M1)"
# Janela de 980x640 em (48, 76); abas no segmentado: CPU (463, 99), Memória (538, 99),
# Energia (613, 99), Disco (688, 99), Rede (763, 99). A primeira leitura do /proc vai
# ao log do app; os processos aparecem na lista.
MON="$PWD/target/debug/mide-monitor"
MDIR="$(mktemp -d)"
check "o Monitor de Recursos abre" 'janela "Monitor de Recursos"' "500 list" "$MON"
SCENARIO_OUT="$MDIR/o" timeout 150 "$SCEN" "500 click 538 99; 560 click 763 99; 700 list" -- "NO_COLOR=1 $MON >'$MDIR/app.log' 2>&1" >/dev/null 2>&1
if cat "$MDIR/app.log" "$MDIR/o/state/mide/logs/mide.log" 2>/dev/null | grep -aqE "monitor: [0-9]{2,} processos, [0-9]+ núcleos"; then
    echo "ok    lê os processos e os núcleos do /proc"; PASS=$((PASS + 1))
else
    echo "FALHA lê os processos e os núcleos do /proc"; FAIL=$((FAIL + 1))
fi
rm -rf "$MDIR"
