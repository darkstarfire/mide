#!/bin/bash
# Minimizar e restaurar janelas X11 (decoradas pelo mide): o cliente pede para minimizar
# (WM_CHANGE_STATE, como o xdotool windowminimize), some do display e aparece em
# "minimized" no controle; o controle (`focus <id>`, o mesmo caminho do Alt+Tab e da dock)
# devolve a janela, mapeada e em foco. Recusa :0 e :99. Nada toca na configuração real.
#
# Uso: tests/x11/minimizar-restaurar.sh [:73]
set -u
DISP="${1:-:73}"
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
  pkill -f "xclock -name minrest" 2>/dev/null
  rm -rf "$OUT"
}
trap cleanup EXIT

[ -x "$DIR/target/debug/mide" ] || { echo "falta target/debug/mide (cargo build --bin mide -j 2)"; exit 2; }
if DISPLAY="$DISP" xdpyinfo >/dev/null 2>&1; then echo "recusado: $DISP já está em uso"; exit 2; fi

mkdir -p "$OUT/rt" "$OUT/home" && chmod 700 "$OUT/rt"
Xvfb "$DISP" -screen 0 1366x768x24 -nolisten tcp >/dev/null 2>&1 &
PID_XV=$!
sleep 2
export DISPLAY="$DISP" MIDE_BACKEND=x11 MIDE_SOCKET=mide-minrest XDG_RUNTIME_DIR="$OUT/rt" HOME="$OUT/home" XDG_CONFIG_HOME="$OUT/home/.config"
unset WAYLAND_DISPLAY

"$DIR/target/debug/mide" > "$OUT/mide.log" 2>&1 &
PID_MIDE=$!
for _ in $(seq 1 100); do xwininfo -root -tree 2>/dev/null | grep -q 'mide-desktop' && break; sleep 0.1; done
sleep 1
xclock -name minrest -geometry 200x200+300+200 >/dev/null 2>&1 &
for _ in $(seq 1 50); do xdotool search --class xclock >/dev/null 2>&1 && break; sleep 0.1; done
sleep 1
CLK=$(xdotool search --class xclock | head -1)

# Comando ao controle do mide, uma resposta por linha (ver src/compositor/control.rs).
ctl() {
  python3 - "$OUT/rt/mide-minrest.ctl" "$1" <<'PY' 2>/dev/null
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
estado_x() { xwininfo -id "$CLK" 2>/dev/null | grep -o 'Map State: .*' | awk '{print $3}'; }
estado_controle() { ctl windows | awk -F'\t' -v id="$CLK" '$1==id {print $4}'; }

echo "== minimizar e restaurar (xclock)"
check "xclock visível antes de minimizar" test "$(estado_x)" = "IsViewable"
check "controle lista o xclock como focado ou normal" bash -c "case \"$(estado_controle)\" in focused|normal) exit 0;; *) exit 1;; esac"

xdotool windowminimize "$CLK" 2>/dev/null
sleep 1
check "xclock não visível depois de minimizar" test "$(estado_x)" = "IsUnMapped"
check "controle lista o xclock como minimized" test "$(estado_controle)" = "minimized"

RESP_FOCUS=$(ctl "focus $CLK" | head -n 1 | tr -d '\n')
echo "    resposta do controle a focus $CLK: [$RESP_FOCUS]"
check "controle restaura pelo id (focus)" test "$RESP_FOCUS" = "ok"
sleep 1
check "xclock visível de novo depois de restaurar" test "$(estado_x)" = "IsViewable"
check "controle lista o xclock como focado ou normal" bash -c "case \"$(estado_controle)\" in focused|normal) exit 0;; *) exit 1;; esac"

echo "== Alt+Tab restaura a minimizada"
xdotool windowminimize "$CLK" 2>/dev/null
sleep 1
check "xclock minimizado antes do Alt+Tab" test "$(estado_x)" = "IsUnMapped"
xdotool key --clearmodifiers alt+Tab 2>/dev/null
sleep 1
check "Alt+Tab devolve o xclock (visível)" test "$(estado_x)" = "IsViewable"
check "controle lista o xclock como focado ou normal após Alt+Tab" bash -c "case \"$(estado_controle)\" in focused|normal) exit 0;; *) exit 1;; esac"

echo
echo "resultado: $PASS ok, $FAIL falhas"
[ "$FAIL" -eq 0 ]
