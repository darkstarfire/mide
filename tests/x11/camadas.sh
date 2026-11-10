#!/bin/bash
# Camadas do mide no backend X11: a barra fica no topo e a dock no fundo, onde pediram
# (não em cascata, como janelas comuns), e o menu da marca abre com um clique.
# Recusa :0 e :99.
#
# Uso: tests/x11/camadas.sh [:69]
set -u
DISP="${1:-:69}"
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
export DISPLAY="$DISP" MIDE_BACKEND=x11 MIDE_SOCKET=mide-camadas XDG_CONFIG_HOME="$OUT/cfg"
unset WAYLAND_DISPLAY
mkdir -p "$XDG_CONFIG_HOME"

"$DIR/target/debug/mide" > "$OUT/mide.log" 2>&1 &
PID_MIDE=$!
for _ in $(seq 1 100); do xwininfo -root -tree 2>/dev/null | grep -q 'mide-bar"' && break; sleep 0.1; done
sleep 1

echo "== posição"
check "barra no topo, em 0,0 (1366 de largura)" bash -c "xwininfo -root -tree | grep 'mide-bar\"' | grep -q '1366x28+0+0'"
check "dock encostada no fundo (y=703, 57 de altura)" bash -c "xwininfo -root -tree | grep 'mide-dock\"' | grep -q '+703'"
check "barra sem moldura de cascata (não em 40,40)" bash -c "! xwininfo -root -tree | grep 'mide-bar\"' | grep -q '+40+40'"

echo "== menu da marca"
xdotool mousemove 25 14 click 1
for _ in $(seq 1 50); do grep -q "menu aberto" "$OUT/mide.log" 2>/dev/null && break; sleep 0.1; done
check "clique na marca abre o menu do sistema" grep -q "menu aberto" "$OUT/mide.log"

echo
echo "resultado: $PASS ok, $FAIL falha(s)"
[ "$FAIL" -eq 0 ]
