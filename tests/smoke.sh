#!/usr/bin/env bash
# Teste de fumaça do mide: sobe o compositor, conecta um cliente real e
# confere os logs. Precisa de uma sessão gráfica (winit abre uma janela).
#
# Uso: tests/smoke.sh [caminho-do-binario-mide]
# Requisitos opcionais: wayland-info (wayland-utils) e gnome-calculator.
set -u

BIN="${1:-$(dirname "$0")/../target/debug/mide}"
# Socket próprio: uma sessão aberta do mide (mide-0) não é afetada.
SOCKET="mide-smoke"
export MIDE_SOCKET="$SOCKET"
LOG="$(mktemp)"
FAIL=0

check() {
    local name="$1" ok="$2"
    if [ "$ok" = "1" ]; then
        echo "ok    $name"
    else
        echo "FALHA $name"
        FAIL=1
    fi
}

if [ ! -x "$BIN" ]; then
    echo "binário não encontrado: $BIN (rode cargo build antes)"
    exit 1
fi

RUST_LOG=mide=debug timeout 25 "$BIN" >"$LOG" 2>&1 &
PID=$!
sleep 3

check "compositor sobe e cria o socket" \
    "$( [ -S "$XDG_RUNTIME_DIR/$SOCKET" ] && echo 1 || echo 0 )"

if command -v wayland-info >/dev/null; then
    INFO="$(WAYLAND_DISPLAY=$SOCKET wayland-info 2>/dev/null)"
    check "anuncia wl_compositor" "$(echo "$INFO" | grep -q wl_compositor && echo 1 || echo 0)"
    check "anuncia xdg_wm_base" "$(echo "$INFO" | grep -q xdg_wm_base && echo 1 || echo 0)"
    check "anuncia wl_seat" "$(echo "$INFO" | grep -q wl_seat && echo 1 || echo 0)"
else
    echo "pulo  wayland-info ausente (sudo apt install wayland-utils)"
fi

if command -v gnome-calculator >/dev/null; then
    WAYLAND_DISPLAY=$SOCKET GDK_BACKEND=wayland timeout 8 gnome-calculator >/dev/null 2>&1 &
    CALC=$!
    sleep 5
    check "cliente cria janela (nova janela criada)" \
        "$(grep -q 'nova janela criada' "$LOG" && echo 1 || echo 0)"
    check "janela aparece no desenho (frame com 1 janela)" \
        "$(grep -q 'frame: 1 janela' "$LOG" && echo 1 || echo 0)"
    kill "$CALC" 2>/dev/null
else
    echo "pulo  gnome-calculator ausente"
fi

# Teste de digitação no buscador (precisa de wtype e fuzzel).
if command -v wtype >/dev/null && command -v fuzzel >/dev/null; then
    FZ_LOG="$(mktemp)"
    WAYLAND_DEBUG=1 WAYLAND_DISPLAY=$SOCKET timeout 8 fuzzel >"$FZ_LOG" 2>&1 &
    FZ_PID=$!
    sleep 2
    WAYLAND_DISPLAY=$SOCKET wtype "abc"
    sleep 2
    KEYS="$(grep -cE 'wl_keyboard@[0-9]+\.key\(' "$FZ_LOG")"
    FIRST="$(grep -nE 'wl_keyboard@[0-9]+\.key\(' "$FZ_LOG" | head -1 | cut -d: -f1)"
    COMMITS="$(tail -n +"${FIRST:-1}" "$FZ_LOG" | grep -c 'wl_surface@[0-9]*.commit')"
    check "buscador recebe teclas do wtype ($KEYS eventos)" "$([ "$KEYS" -ge 6 ] && echo 1 || echo 0)"
    check "buscador redesenha depois das teclas (commits: $COMMITS)" "$([ "$COMMITS" -ge 1 ] && echo 1 || echo 0)"
    rm -f "$FZ_LOG"
    kill "$FZ_PID" 2>/dev/null
else
    echo "pulo  teste de digitação (precisa de wtype e fuzzel)"
fi

kill "$PID" 2>/dev/null
wait "$PID" 2>/dev/null

check "sem panic nos logs" "$(grep -qi panic "$LOG" && echo 0 || echo 1)"

rm -f "$LOG"
if [ "$FAIL" = 0 ]; then
    echo "resultado: todos os testes passaram"
else
    echo "resultado: há falhas"
fi
exit "$FAIL"
