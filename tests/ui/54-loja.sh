# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== Loja (M7)"
# A janela abre e o catálogo AppStream do sistema é lido (sem instalar nada).
STORE="$PWD/target/debug/mide-store"
LDIR="$(mktemp -d)"
check "a Loja abre" 'janela "Loja"' "500 list" "$STORE"
if ls /var/lib/swcatalog/yaml/*.yml.gz >/dev/null 2>&1; then
    SCENARIO_OUT="$LDIR/o" timeout 200 "$SCEN" "900 list" -- "$STORE" >/dev/null 2>&1
    if cat "$LDIR/o/state/mide/logs/mide.log" "${XDG_STATE_HOME:-/nonexistent}/mide/logs/mide.log" 2>/dev/null | grep -aqE "loja: [0-9]{3,} apps no catálogo"; then
        echo "ok    a Loja lê o catálogo de apps da distribuição"; PASS=$((PASS + 1))
    else
        echo "FALHA a Loja lê o catálogo de apps da distribuição"; FAIL=$((FAIL + 1))
    fi
else
    echo "pulo  catálogo da Loja (sem /var/lib/swcatalog: instale o appstream)"
fi
rm -rf "$LDIR"
