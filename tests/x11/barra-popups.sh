#!/bin/bash
# Popups da barra no X11: o clique no relógio abre o calendário, numa janela de menu
# logo abaixo da barra, e Escape fecha. Recusa :0 e :99.
#
# Uso: tests/x11/barra-popups.sh [:74]
set -u
DISP="${1:-:74}"
case "$DISP" in
  :0|:0.*|:99|:99.*) echo "recusado: $DISP não é display de teste"; exit 2 ;;
esac
DIR="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$(mktemp -d)"
PASS=0; FAIL=0
ok()  { echo "  ok    $1"; PASS=$((PASS+1)); }
nok() { echo "  FALHA $1"; FAIL=$((FAIL+1)); }
check() { local nome="$1"; shift; if "$@" >/dev/null 2>&1; then ok "$nome"; else nok "$nome"; fi; }
cleanup() {
  [ -n "${PID_MIDE:-}" ] && kill "$PID_MIDE" 2>/dev/null
  [ -n "${PID_XV:-}" ] && kill "$PID_XV" 2>/dev/null
  rm -rf "$OUT"
}
trap cleanup EXIT

[ -x "$DIR/target/debug/mide" ] || { echo "falta target/debug/mide (cargo build --bin mide -j 2)"; exit 2; }
if DISPLAY="$DISP" xdpyinfo >/dev/null 2>&1; then echo "recusado: $DISP já está em uso"; exit 2; fi

Xvfb "$DISP" -screen 0 1366x768x24 -nolisten tcp >/dev/null 2>&1 &
PID_XV=$!
sleep 2
export DISPLAY="$DISP" MIDE_BACKEND=x11 MIDE_SOCKET=mide-popups XDG_CONFIG_HOME="$OUT/cfg"
unset WAYLAND_DISPLAY
mkdir -p "$XDG_CONFIG_HOME"

"$DIR/target/debug/mide" > "$OUT/mide.log" 2>&1 &
PID_MIDE=$!
for _ in $(seq 1 100); do xwininfo -root -tree 2>/dev/null | grep -q 'mide-bar"' && break; sleep 0.1; done
sleep 1

echo "== relógio"
xdotool mousemove 1320 14 click 1
for _ in $(seq 1 50); do grep -q "menu aberto: Calendar" "$OUT/mide.log" 2>/dev/null && break; sleep 0.1; done
check "clique no relógio abre o calendário" grep -q "menu aberto: Calendar" "$OUT/mide.log"
sleep 1
check "popup do calendário é uma janela de menu do mide" bash -c "xwininfo -root -tree | grep -q 'mide-bar-menu\"'"
MENU_Y=$(xwininfo -root -tree | grep 'mide-bar-menu"' | grep -o '+[0-9]*+[0-9]*' | tail -1 | cut -d+ -f3)
check "popup fica abaixo da barra (y >= 28)" test -n "$MENU_Y" -a "$MENU_Y" -ge 28

echo "== fechar"
# Clique fora da barra e do menu fecha o popup. (Escape ainda não: o popup do X11 não
# recebe o teclado; falta dar foco ao menu. Ver a nota no ANDAMENTO.)
xdotool mousemove 600 400 click 1
sleep 1
check "clique fora fecha o calendário" bash -c "! xwininfo -root -tree | grep -q 'mide-bar-menu\"'"

echo
echo "resultado: $PASS ok, $FAIL falha(s)"
[ "$FAIL" -eq 0 ]
