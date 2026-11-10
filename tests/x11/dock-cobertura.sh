#!/bin/bash
# Dock no X11 com ocultar automático: uma janela que cobre a faixa da dock faz a dock
# descer (escondida) e o mide sabe disso (dock_covered); afastada, a dock volta.
# Usa a configuração de teste (autohide ligado) numa cópia, nunca a real. Recusa :0 e :99.
#
# Uso: tests/x11/dock-cobertura.sh [:72]
set -u
DISP="${1:-:72}"
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

# Configuração de teste: ocultar automático ligado (cópia, não a configuração real).
mkdir -p "$OUT/cfg/mide"
printf '[dock]\nautohide = true\n' > "$OUT/cfg/mide/config.toml"

Xvfb "$DISP" -screen 0 1366x768x24 -nolisten tcp >/dev/null 2>&1 &
PID_XV=$!
sleep 2
export DISPLAY="$DISP" MIDE_BACKEND=x11 MIDE_SOCKET=mide-dockcob XDG_CONFIG_HOME="$OUT/cfg"
unset WAYLAND_DISPLAY

"$DIR/target/debug/mide" > "$OUT/mide.log" 2>&1 &
PID_MIDE=$!
for _ in $(seq 1 100); do xwininfo -root -tree 2>/dev/null | grep -q 'mide-dock"' && break; sleep 0.1; done
sleep 1
dock_y() { xwininfo -root -tree | grep 'mide-dock"' | grep -o '+[0-9]*+[0-9]*' | tail -1 | cut -d+ -f3; }
covered_bool() { python3 - <<PY 2>/dev/null
import socket, os
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(os.path.join(os.environ["XDG_RUNTIME_DIR"], "mide-dockcob.ctl"))
s.sendall(b"dock_covered\n")
out = b""
while True:
    c = s.recv(256)
    if not c: break
    out += c
print(out.decode().split()[0] if out.split() else "")
PY
}

echo "== ocultar automático"
check "dock na posição normal (y=703)" test "$(dock_y)" = "703"
X_TERM_ID=""
(xterm -geometry 110x40 -e sleep 60 2>/dev/null &)
for _ in $(seq 1 50); do X_TERM_ID=$(xdotool search --class xterm 2>/dev/null | head -1); [ -n "$X_TERM_ID" ] && break; sleep 0.1; done
xdotool windowmove "$X_TERM_ID" 400 380
sleep 2
check "janela cobrindo a dock: dock desce (y=773)" test "$(dock_y)" = "773"
check "mide reconhece a cobertura (dock_covered yes)" test "$(covered_bool)" = "yes"
xdotool windowmove "$X_TERM_ID" 900 40
sleep 2
check "janela afastada: dock volta (y=703)" test "$(dock_y)" = "703"
check "mide reconhece que a dock está livre (dock_covered no)" test "$(covered_bool)" = "no"
xdotool windowmove "$X_TERM_ID" 900 40 2>/dev/null
xdotool windowkill "$X_TERM_ID" 2>/dev/null
echo
echo "resultado: $PASS ok, $FAIL falha(s)"
[ "$FAIL" -eq 0 ]
