#!/usr/bin/env bash
# Desempenho e resposta: roda cenários típicos numa sessão de teste isolada e mostra
# o tempo de quadro (mediana, p95, máximo) de cada parte e o tempo de resposta às
# entradas (do clique/tecla/roda até o app enviar o quadro novo).
#
# Uso: tests/perf.sh            (todos os cenários)
#      tests/perf.sh dock       (só um: desktop, dock, launcher, buscador, arquivos)
# Não rodar junto com tests/ui.sh nem durante uma compilação.
set -u
cd "$(dirname "$0")/.."
SCEN=tests/scenario.sh
OUT="$(mktemp -d)"
export SCENARIO_OUT="$OUT/scen"
# Medido em 1366x768 (a resolução de tela mais comum no Brasil).
export MIDE_WINIT_SIZE=1366x768
export XDG_CONFIG_HOME="$OUT/cfg"
mkdir -p "$XDG_CONFIG_HOME"
trap 'rm -rf "$OUT"' EXIT
export RUST_LOG="mide=debug,mide_dock=debug,mide_launcher=debug,mide_files=debug,mide_search=debug,mide::kit=debug"
ONLY="${1:-}"

# stats "rótulo" "regex com o número em ms": mediana, p95, máximo e quantidade.
stats() {
    local label="$1" pattern="$2" file="$3"
    sed 's/\x1b\[[0-9;]*m//g' "$file" | grep -oE "$pattern" | grep -oE '[0-9]+\.[0-9]+ ms' | grep -oE '[0-9.]+' | sort -n | awk -v l="$label" '
        { v[NR] = $1 }
        END {
            if (NR == 0) { printf "  %-34s (sem amostras)\n", l; exit }
            p50 = v[int((NR + 1) / 2)]; i = int(NR * 0.95); if (i < 1) i = 1; p95 = v[i]
            printf "  %-34s mediana %6.2f ms   p95 %6.2f ms   máx %6.2f ms   (%d)\n", l, p50, p95, v[NR], NR
        }'
}

run() {
    local name="$1" script="$2"
    shift 2
    [ -n "$ONLY" ] && [ "$ONLY" != "$name" ] && return
    echo "== $name"
    SCENARIO_WAIT="${SCENARIO_WAIT:-8}" timeout 150 "$SCEN" "$script" -- "$@" >/dev/null 2>&1
    local log="$SCENARIO_OUT/mide.log"
    cat "$OUT"/app.log >>"$log" 2>/dev/null
    rm -f "$OUT"/app.log
    stats "quadro do compositor (CPU)" "quadro do compositor: [0-9.]+ ms" "$log"
    stats "quadro da dock" "quadro da dock: [0-9.]+ ms" "$log"
    stats "menu da dock" "menu da dock desenhado: [0-9.]+ ms" "$log"
    stats "quadro do launcher" "quadro do launcher[^:]*: [0-9.]+ ms" "$log"
    stats "quadro de app mide-ui" "quadro do app: [0-9.]+ ms" "$log"
    stats "resposta a clique/tecla/roda" "resposta \([a-z]+\): [0-9.]+ ms" "$log"
    echo "  vidro refeito (todas as regiões): $(grep -c 'desfoque refeito' "$log") vez(es)"
}

# Desktop parado: só o custo de base (vidro da dock com o fundo parado).
run desktop "100 move 400 300; 400 list"
# Dock: passar o mouse por todos os ícones e clicar no Launchpad e fechar.
# (Dock padrão em 1366x768: ícones em y=728, de x=568 a x=799.)
run dock "100 move 568 728; 120 move 611 728; 140 move 654 728; 160 move 697 728; 180 move 756 728; 200 move 799 728; 260 rclick 799 728; 300 key esc; 400 list"
# Launcher: abrir, trocar de página com a roda, buscar e fechar.
run launcher "100 combo ctrl+shift+space; 180 wheel down; 220 wheel up; 260 type calc; 300 key esc; 330 key esc; 400 list"
# Buscador: abrir, digitar letra a letra (cada tecla refaz a busca), descer e fechar.
run buscador "100 combo ctrl+space; 160 type c; 170 type a; 180 type l; 190 type c; 220 key down; 240 key down; 260 key up; 300 key esc; 400 list"
# Arquivos: navegar, menu de contexto, lista e voltar.
FDIR="$OUT/pasta"
mkdir -p "$FDIR"/{Fotos,Projetos,Docs}
for i in $(seq 1 40); do echo "$i" >"$FDIR/arquivo $i.txt"; done
run arquivos "200 click 463 190; 230 click 694 106; 260 rclick 500 203; 290 key esc; 320 click 652 106; 350 click 463 190; 356 click 463 190; 400 key backspace; 440 wheel down; 460 wheel down; 480 list" \
    "RUST_LOG=mide_files=debug,mide::kit=debug target/debug/mide-files '$FDIR' >>'$OUT/app.log' 2>&1"
