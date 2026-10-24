# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== arrastar arquivos entre apps (Wayland DnD, B26)"
# Duas janelas do Arquivos: A (pasta A) nasce em (48, 76) e B (pasta B) em (80, 108),
# por cima. O nota.txt da B fica em (378, 222); a faixa da A que aparece à esquerda
# da B (x < 80) recebe o arquivo. Sair da janela B arrastando vira arrasto entre apps
# (text/uri-list); soltar em outro app copia.
FILES="$PWD/target/debug/mide-files"
DBASE="$(mktemp -d)"
OLD_CACHE="${XDG_CACHE_HOME:-}"
export XDG_STATE_HOME="$DBASE/state" XDG_CACHE_HOME="$DBASE/cache"
mkdir -p "$DBASE/A" "$DBASE/B"
echo "olá" >"$DBASE/B/nota.txt"
LOG="$(timeout 150 "$SCEN" "300 press 378 222; 320 move 390 230; 340 move 200 300; 360 move 64 300; 380 move 65 310; 390 move 66 312; 420 release 66 312; 470 list" -- \
    "NO_COLOR=1 $FILES '$DBASE/A' >'$DBASE/a.log' 2>&1" "sleep 1; NO_COLOR=1 $FILES '$DBASE/B' >'$DBASE/b.log' 2>&1" 2>&1)"
if echo "$LOG" | grep -q "arrasto entre apps começou (ícone: true)"; then
    echo "ok    sair da janela arrastando começa o arrasto entre apps, com ícone"; PASS=$((PASS + 1))
else
    echo "FALHA sair da janela arrastando começa o arrasto entre apps, com ícone"; FAIL=$((FAIL + 1))
fi
if [ -f "$DBASE/A/nota.txt" ] && [ -f "$DBASE/B/nota.txt" ]; then
    echo "ok    soltar na outra janela copia o arquivo (o original fica)"; PASS=$((PASS + 1))
else
    echo "FALHA soltar na outra janela copia o arquivo (o original fica)"; FAIL=$((FAIL + 1))
    sed 's/\x1b\[[0-9;]*m//g' "$DBASE/a.log" "$DBASE/b.log" | grep -a "arrast\|copiad" | sed 's/^/      /' | tail -4
fi
unset XDG_STATE_HOME
if [ -n "$OLD_CACHE" ]; then export XDG_CACHE_HOME="$OLD_CACHE"; else unset XDG_CACHE_HOME; fi
rm -rf "$DBASE"
