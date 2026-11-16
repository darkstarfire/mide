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
# Barra lateral (M11): arrastar um item da lista (Arquivos em lista, a primeira linha
# em (360, 183)) até um atalho (Downloads em (141, 218)) move para a pasta dele.
mkdir -p "$DBASE/home/Downloads" "$DBASE/home/Docs" "$DBASE/cfg"
echo x >"$DBASE/home/Docs/arrastado.txt"
printf 'XDG_DOWNLOAD_DIR="$HOME/Downloads"\nXDG_DESKTOP_DIR="$HOME"\nXDG_DOCUMENTS_DIR="$HOME"\n' >"$DBASE/cfg/user-dirs.dirs"
SIDE="400 click 529 102; 450 press 360 183"
for i in $(seq 1 20); do SIDE="$SIDE; $((450 + i * 2)) move $((360 - i * 11)) $((183 + i * 2))"; done
SIDE="$SIDE; 495 move 140 218; 500 move 141 218; 506 release 141 218; 540 list"
HOME="$DBASE/home" XDG_CONFIG_HOME="$DBASE/cfg" timeout 150 "$SCEN" "$SIDE" -- "NO_COLOR=1 $FILES '$DBASE/home/Docs' >'$DBASE/side.log' 2>&1" >/dev/null 2>&1
if [ -f "$DBASE/home/Downloads/arrastado.txt" ] && [ ! -f "$DBASE/home/Docs/arrastado.txt" ]; then
    echo "ok    soltar sobre um atalho da barra lateral move para a pasta dele"; PASS=$((PASS + 1))
else
    echo "FALHA soltar sobre um atalho da barra lateral move para a pasta dele"; FAIL=$((FAIL + 1))
fi
# Terminal (M11): um arquivo da mesa (o terceiro, em (1297, 360)) solto na janela do
# Terminal (em (48, 76)) vira o caminho completo, entre aspas, na linha de comando.
mkdir -p "$DBASE/mesa"
echo x >"$DBASE/mesa/a.txt"
echo x >"$DBASE/mesa/b.txt"
echo x >"$DBASE/mesa/meu arquivo.txt"
TERM_DROP="500 list; 520 press 1297 360"
for i in $(seq 1 30); do TERM_DROP="$TERM_DROP; $((520 + i * 2)) move $((1297 - i * 25)) $((360 - i * 2))"; done
TERM_DROP="$TERM_DROP; 590 move 500 300; 595 release 500 300; 640 list"
MIDE_DESKTOP_DIR="$DBASE/mesa" SCENARIO_OUT="$DBASE/term" timeout 150 "$SCEN" "$TERM_DROP" -- "$PWD/target/debug/mide-terminal" >/dev/null 2>&1
if cat "$DBASE/term/state/mide/logs/mide.log" "$XDG_STATE_HOME/mide/logs/mide.log" 2>/dev/null | grep -aq "caminhos soltos no terminal: 1"; then
    echo "ok    arquivo solto no Terminal vira o caminho na linha de comando"; PASS=$((PASS + 1))
else
    echo "FALHA arquivo solto no Terminal vira o caminho na linha de comando"; FAIL=$((FAIL + 1))
fi
unset XDG_STATE_HOME
if [ -n "$OLD_CACHE" ]; then export XDG_CACHE_HOME="$OLD_CACHE"; else unset XDG_CACHE_HOME; fi
rm -rf "$DBASE"
