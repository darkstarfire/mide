# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== Writer"
# Salvamento automático (`[writer] autosave`, ligado por padrão): um documento com
# arquivo é gravado sozinho 3 s depois da última tecla; desligado, o arquivo fica igual.
WDIR="$(mktemp -d)"
RTF='{\rtf1\ansi{\fonttbl\f0 Helvetica;}\f0\fs24 Inicio\par}'
printf '%s' "$RTF" >"$WDIR/auto.rtf"
XDG_STATE_HOME="$WDIR/state" SCENARIO_OUT="$WDIR/out1" timeout 150 "$SCEN" \
    "150 click 683 300; 170 key End; 180 type abc; 600 list" 590 -- "mide-writer $WDIR/auto.rtf" >/dev/null 2>&1
if grep -q 'Inicioabc' "$WDIR/auto.rtf"; then
    echo "ok    salvamento automático grava o documento sozinho"; PASS=$((PASS + 1))
else
    echo "FALHA salvamento automático grava o documento sozinho"; FAIL=$((FAIL + 1))
    sed 's/^/      /' "$WDIR/auto.rtf" | head -6
fi
mkdir -p "$WDIR/cfg/mide"; printf '[writer]\nautosave = false\n' >"$WDIR/cfg/mide/config.toml"
printf '%s' "$RTF" >"$WDIR/manual.rtf"
XDG_CONFIG_HOME="$WDIR/cfg" XDG_STATE_HOME="$WDIR/state" SCENARIO_OUT="$WDIR/out2" timeout 150 "$SCEN" \
    "150 click 683 300; 170 key End; 180 type abc; 600 list" 590 -- "mide-writer $WDIR/manual.rtf" >/dev/null 2>&1
if ! grep -q 'Inicioabc' "$WDIR/manual.rtf" && grep -q 'Inicio' "$WDIR/manual.rtf"; then
    echo "ok    sem o salvamento automático o arquivo fica como estava"; PASS=$((PASS + 1))
else
    echo "FALHA sem o salvamento automático o arquivo fica como estava"; FAIL=$((FAIL + 1))
fi
rm -rf "$WDIR"
