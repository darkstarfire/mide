# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== Discos (M8)"
# A janela abre e os discos vêm do UDisks2 (só leitura; nada é formatado aqui: os
# discos são os da máquina). O teste com um disco de mentira (loop) fica em
# `cargo test -- --ignored` (precisa do /dev/loop-control).
DISKS="$PWD/target/debug/mide-disks"
DDIR="$(mktemp -d)"
check "os Discos abrem" 'janela "Discos"' "500 list" "$DISKS"
SCENARIO_OUT="$DDIR/o" timeout 200 "$SCEN" "700 list" -- "$DISKS" >/dev/null 2>&1
if cat "$DDIR/o/state/mide/logs/mide.log" "${XDG_STATE_HOME:-/nonexistent}/mide/logs/mide.log" 2>/dev/null | grep -aqE "discos: [1-9][0-9]* disco"; then
    echo "ok    os Discos leem os discos pelo UDisks2"; PASS=$((PASS + 1))
else
    echo "FALHA os Discos leem os discos pelo UDisks2"; FAIL=$((FAIL + 1))
fi
rm -rf "$DDIR"
