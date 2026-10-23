# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== terminal: grade, quadros, powerline e ícones (E1b)"
# Amostra com blocos, linhas de quadro, uma seta do powerline, "i" normal e em
# negrito e ícones da Nerd Font (tests/data/terminal/amostra-grade.sh), conferida
# pixel a pixel por conferir-grade.py: blocos sem frestas, linhas contínuas entre as
# células, a seta encostando nas vizinhas, o negrito na mesma posição do normal (sem
# trocar de fonte) e os ícones dentro das células (até duas com espaço depois).
GRADE="$(mktemp -d)"
NERD_OPT=""
fc-list 2>/dev/null | grep -q "Nerd Font" || NERD_OPT="--sem-icones"
grade_check() {
    local name="$1" font="$2" size="$3" dir="$GRADE/$4" out
    mkdir -p "$dir/cfg/mide"
    printf 'default = "T"\n[[profiles]]\nname = "T"\nfont_size = %s\nfont = "%s"\n' "$size" "$font" > "$dir/cfg/mide/terminal.toml"
    XDG_CONFIG_HOME="$dir/cfg" SCENARIO_OUT="$dir/scen" timeout 150 "$SCEN" "500 list" 500 -- "$TERMINAL -e $PWD/tests/data/terminal/amostra-grade.sh" >/dev/null 2>&1
    if out="$(python3 tests/data/terminal/conferir-grade.py "$dir/scen/frame.png" $NERD_OPT 2>&1)"; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name"; echo "$out" | sed 's/^/      /'; FAIL=$((FAIL + 1))
    fi
}
grade_check "terminal: quadros, blocos, powerline e letras na grade (fonte padrão, 13)" "" 13 p13
grade_check "terminal: quadros, blocos, powerline e letras na grade (fonte padrão, 17)" "" 17 p17
for nerd in "Cousine Nerd Font" "CodeNewRoman Nerd Font Mono"; do
    if fc-list 2>/dev/null | grep -q "$nerd:"; then
        grade_check "terminal: grade e ícones com a $nerd" "$nerd" 15 "${nerd// /}"
    else
        echo "pulo  terminal com a $nerd (não instalada)"
    fi
done
rm -rf "$GRADE"
