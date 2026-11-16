#!/bin/bash
# Foco ao clicar no conteúdo de uma janela X11 decorada: um clique no conteúdo de uma janela
# que não está em foco a traz para a frente e a torna a janela focada (o controle a lista
# como focused). Usa Xvfb num display de teste (recusa :0 e :99).
#
# Uso: tests/x11/foco-clique.sh [:75]
set -u
DISP="${1:-:75}"
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
export DISPLAY="$DISP" MIDE_BACKEND=x11 MIDE_SOCKET=mide-foco XDG_RUNTIME_DIR="$OUT/rt" HOME="$OUT/home" XDG_CONFIG_HOME="$OUT/home/.config"
unset WAYLAND_DISPLAY

"$DIR/target/debug/mide" > "$OUT/mide.log" 2>&1 &
PID_MIDE=$!
for _ in $(seq 1 100); do xwininfo -root -tree 2>/dev/null | grep -q 'mide-desktop' && break; sleep 0.1; done
sleep 1

xclock -name focoA -geometry 200x200+100+100 >/dev/null 2>&1 &
sleep 1.5
xclock -name focoB -geometry 200x200+500+300 >/dev/null 2>&1 &
sleep 1.5
A=$(xdotool search --class xclock | sed -n 1p)
B=$(xdotool search --class xclock | sed -n 2p)

ctl() {
  python3 - "$OUT/rt/mide-foco.ctl" "$1" <<'PY' 2>/dev/null
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sys.argv[1])
s.sendall((sys.argv[2] + "\n").encode())
out = b""
while True:
    c = s.recv(4096)
    if not c: break
    out += c
print(out.decode())
PY
}
estado() { ctl windows | awk -F'\t' -v id="$1" '$1==id {print $4}'; }
centro() { xwininfo -id "$1" | awk '/Absolute upper-left X/ {x=$4} /Absolute upper-left Y/ {y=$4} /Width:/ {w=$2} /Height:/ {h=$2} END {print x+int(w/2), y+int(h/2)}'; }

echo "== clique no conteúdo traz a janela para o foco"
check "xclock B começa focado (último aberto)" test "$(estado "$B")" = "focused"
# B é posicionada em cascata por cima de A: o centro de A fica coberto. Clica-se no canto de
# cima à esquerda de A, que está fora de B.
read -r AX0 AY0 <<<"$(xwininfo -id "$A" | awk '/Absolute upper-left X/ {x=$4} /Absolute upper-left Y/ {y=$4} END {print x, y}')"
AX=$((AX0 + 10)); AY=$((AY0 + 10))
xdotool mousemove "$AX" "$AY" click 1
sleep 1
check "clique no conteúdo de A dá o foco a A" test "$(estado "$A")" = "focused"
check "B deixa de ser o focado" test "$(estado "$B")" = "normal"
# Agora A está por cima de B: o centro de B fica coberto. Clica-se no canto de baixo à direita.
read -r BX0 BY0 BW BH <<<"$(xwininfo -id "$B" | awk '/Absolute upper-left X/ {x=$4} /Absolute upper-left Y/ {y=$4} /Width:/ {w=$2} /Height:/ {h=$2} END {print x, y, w, h}')"
BX=$((BX0 + BW - 10)); BY=$((BY0 + BH - 10))
xdotool mousemove "$BX" "$BY" click 1
sleep 1
check "clique no conteúdo de B devolve o foco a B" test "$(estado "$B")" = "focused"

echo
echo "resultado: $PASS ok, $FAIL falhas"
[ "$FAIL" -eq 0 ]
