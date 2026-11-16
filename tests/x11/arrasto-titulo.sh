#!/bin/bash
# Arrastar uma janela X11 pela barra de título do mide: o cliente acompanha o ponteiro.
# Xvfb num display de teste (recusa :0 e :99).
#
# Uso: tests/x11/arrasto-titulo.sh [:76]
set -u
DISP="${1:-:76}"
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

mkdir -p "$OUT/rt" "$OUT/home" && chmod 700 "$OUT/rt"
Xvfb "$DISP" -screen 0 1366x768x24 -nolisten tcp >/dev/null 2>&1 &
PID_XV=$!
sleep 2
export DISPLAY="$DISP" MIDE_BACKEND=x11 MIDE_SOCKET=mide-arrasto XDG_RUNTIME_DIR="$OUT/rt" HOME="$OUT/home" XDG_CONFIG_HOME="$OUT/home/.config"
unset WAYLAND_DISPLAY

"$DIR/target/debug/mide" > "$OUT/mide.log" 2>&1 &
PID_MIDE=$!
for _ in $(seq 1 100); do xwininfo -root -tree 2>/dev/null | grep -q 'mide-desktop' && break; sleep 0.1; done
sleep 1
xclock -name arrastoA -geometry 200x200+100+100 >/dev/null 2>&1 &
for _ in $(seq 1 50); do xdotool search --class xclock >/dev/null 2>&1 && break; sleep 0.1; done
sleep 2
CLK=$(xdotool search --class xclock | head -1)
pos() { xwininfo -id "$CLK" | awk '/Absolute upper-left X/ {x=$4} /Absolute upper-left Y/ {y=$4} END {print x, y}'; }

read -r X0 Y0 <<<"$(pos)"
echo "== arrastar pela barra de título (cliente em $X0,$Y0)"
# A barra de título fica 30 px acima do cliente: o meio dela, com folga das bordas.
TX=$((X0 + 80)); TY=$((Y0 - 15))
xdotool mousemove "$TX" "$TY" mousedown 1
for step in 1 2 3 4 5; do
  xdotool mousemove $((TX + step * 20)) $((TY + step * 8))
  sleep 0.1
done
xdotool mouseup 1
sleep 1
read -r X1 Y1 <<<"$(pos)"
DX=$((X1 - X0)); DY=$((Y1 - Y0))
check "cliente andou na horizontal (≈100 px)" test "$DX" -ge 90 -a "$DX" -le 110
check "cliente andou na vertical (≈40 px)" test "$DY" -ge 30 -a "$DY" -le 50

echo
echo "resultado: $PASS ok, $FAIL falhas"
[ "$FAIL" -eq 0 ]
