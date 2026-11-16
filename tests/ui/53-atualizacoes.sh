# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== Atualização de Software (M6)"
# A janela abre e a verificação termina pelo PackageKit da máquina (só leitura: atualizar
# a lista não pede senha numa sessão local; instalar pede e não é feito aqui).
UPD="$PWD/target/debug/mide-updates"
UDIR="$(mktemp -d)"
check "a Atualização de Software abre" 'janela "Atualização de Software"' "500 list" "$UPD"
if busctl --system status org.freedesktop.PackageKit >/dev/null 2>&1 || [ -f /usr/share/dbus-1/system-services/org.freedesktop.PackageKit.service ]; then
    SCENARIO_OUT="$UDIR/o" SCENARIO_MAX=120 timeout 200 "$SCEN" "1500 list" -- "$UPD" >/dev/null 2>&1
    if grep -aqE "atualizações: [0-9]+ disponível|atualizações: não foi possível" "$UDIR/o/state/mide/logs/mide.log" "${XDG_STATE_HOME:-/nonexistent}/mide/logs/mide.log" 2>/dev/null; then
        echo "ok    a verificação de atualizações termina (PackageKit)"; PASS=$((PASS + 1))
    else
        echo "FALHA a verificação de atualizações termina (PackageKit)"; FAIL=$((FAIL + 1))
    fi
else
    echo "pulo  verificação de atualizações (sem PackageKit)"
fi
rm -rf "$UDIR"
