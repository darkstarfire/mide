#!/usr/bin/env bash
# Bateria de regressão da interface, em seções paralelas.
#
# Uso:
#   tests/ui.sh                  todas as seções (tests/ui/NN-*.sh)
#   tests/ui.sh captura dock     só as seções cujo nome contém "captura" ou "dock"
#   tests/ui.sh -j 4 ...         quantas seções ao mesmo tempo (padrão: 6)
#   tests/ui.sh -l               lista as seções
#
# Cada seção roda num processo próprio, com o próprio socket (mide-ui-<seção>), a
# própria pasta de saída e a própria configuração temporária: as seções não se
# atrapalham. Dentro de uma seção, os cenários seguem em ordem (alguns usam o
# resultado do anterior). As funções (check, check_last...) estão em tests/ui/comum.sh.
# Os binários são os de target/debug: compile antes (cargo build --bins --examples).
set -u
cd "$(dirname "$0")/.."

JOBS=6
FILTERS=()
while [ $# -gt 0 ]; do
    case "$1" in
        -j) JOBS="$2"; shift 2 ;;
        -j*) JOBS="${1#-j}"; shift ;;
        -l) for f in tests/ui/[0-9]*.sh; do basename "$f" .sh; done; exit 0 ;;
        *) FILTERS+=("$1"); shift ;;
    esac
done

SECTIONS=()
for f in tests/ui/[0-9]*.sh; do
    name="$(basename "$f" .sh)"
    if [ ${#FILTERS[@]} -eq 0 ]; then
        SECTIONS+=("$name")
    else
        for want in "${FILTERS[@]}"; do
            case "$name" in *"$want"*) SECTIONS+=("$name"); break ;; esac
        done
    fi
done
if [ ${#SECTIONS[@]} -eq 0 ]; then
    echo "nenhuma seção com: ${FILTERS[*]} (tests/ui.sh -l lista as seções)"
    exit 2
fi

# Compilação incremental grande demais (enche o disco): apaga; os programas ficam.
data/mide-trim-target

# Trava: a bateria não pode alterar a configuração real do usuário.
REAL_CFG="${XDG_CONFIG_HOME:-$HOME/.config}/mide"
cfg_fingerprint() { (cd "$REAL_CFG" 2>/dev/null && ls -A | sort && cat ./* 2>/dev/null) | md5sum; }
CFG_BEFORE="$(cfg_fingerprint)"

RUN="$(mktemp -d)"
# Interrompida (Ctrl+C, kill), a bateria encerra as seções que lançou: uma seção órfã
# continuaria com o mesmo socket da próxima execução, e as duas se atrapalhariam.
stop_sections() {
    local pid
    for pid in $(jobs -p); do
        pkill -TERM -P "$pid" 2>/dev/null
        kill "$pid" 2>/dev/null
    done
}
trap 'stop_sections; exit 130' INT TERM
trap 'rm -rf "$RUN"' EXIT
START=$SECONDS

run_section() {
    local name="$1"
    local t0
    t0=$(date +%s)
    # O socket leva o número desta execução: duas baterias ao mesmo tempo (outra cópia
    # do repositório, outro agente) não podem dividir o socket de uma seção.
    MIDE_TEST_SOCKET="mide-ui-$$-$name" SCENARIO_OUT="$RUN/$name/out" \
        bash -c 'source tests/ui/comum.sh; source "tests/ui/$1.sh"; echo "seção: $PASS $FAIL"' _ "$name" \
        >"$RUN/$name.txt" 2>&1
    local got
    got="$(grep '^seção: ' "$RUN/$name.txt" | tail -1)"
    printf '%-26s %s (%ss)\n' "$name" "${got:-sem resultado}" "$(($(date +%s) - t0))" >&2
}

echo "${#SECTIONS[@]} seção(ões), $JOBS ao mesmo tempo" >&2
for name in "${SECTIONS[@]}"; do
    while [ "$(jobs -rp | wc -l)" -ge "$JOBS" ]; do
        wait -n
    done
    run_section "$name" &
done
wait

PASS=0
FAIL=0
for name in "${SECTIONS[@]}"; do
    grep -v '^seção: ' "$RUN/$name.txt"
    read -r p f < <(grep '^seção: ' "$RUN/$name.txt" | tail -1 | cut -d' ' -f2-)
    if [ -z "${p:-}" ]; then
        echo "FALHA a seção $name não terminou"
        FAIL=$((FAIL + 1))
    else
        PASS=$((PASS + p))
        FAIL=$((FAIL + f))
    fi
    p=""
done

if [ "$(cfg_fingerprint)" != "$CFG_BEFORE" ]; then
    echo "FALHA a bateria alterou $REAL_CFG"
    FAIL=$((FAIL + 1))
else
    echo "ok    configuração real intacta"
    PASS=$((PASS + 1))
fi

echo "resultado: $PASS ok, $FAIL falha(s) em $((SECONDS - START))s"
[ "$FAIL" = 0 ]
