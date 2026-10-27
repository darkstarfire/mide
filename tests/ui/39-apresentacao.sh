# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== aviso de apresentação (wp_presentation)"
# examples/mide-presentation-teste: janela que redesenha a cada `frame` e pede o aviso
# de apresentação em cada commit (como o Chrome); depois de 60 avisos imprime o resumo.
# No backend sem tela, o quadro conta como apresentado ao fim do desenho, com a
# frequência da tela virtual (60 Hz: 16666666 ns) e o relógio monotônico (clock=1).
PRES="$PWD/target/debug/examples/mide-presentation-teste"
if [ -x "$PRES" ]; then
    PRES_OUT="$(mktemp)"
    SCENARIO_WAIT=6 timeout 150 "$SCEN" "" -- "$PRES 60 > $PRES_OUT" >/dev/null 2>&1
    LINE="$(cat "$PRES_OUT")"
    rm -f "$PRES_OUT"
    if echo "$LINE" | grep -Eq 'presentation: clock=1 presented=60 discarded=[0-9]+ refresh_ns=16666666 seq=crescente intervalo_ms=(1[5-9]|2[0-9]|3[0-9])\.'; then
        echo "ok    os quadros da janela chegam como apresentados (60 Hz, relógio monotônico)"
        PASS=$((PASS + 1))
    else
        echo "FALHA os quadros da janela chegam como apresentados"
        echo "      resumo: ${LINE:-nada}"
        FAIL=$((FAIL + 1))
    fi
else
    echo "pulo  apresentação (rode cargo build --examples)"
fi
