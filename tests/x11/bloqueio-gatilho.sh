#!/bin/bash
# Gatilho do bloqueio no backend X11 do mide (Super+L): abre o mide-lock (o programa de
# bloqueio com senha), sem duplicar, e não desbloqueia por tecla nem por propriedade.
# Só conta os mide-lock filhos do mide de teste (não os da sessão real). Recusa :0 e :99.
#
# Uso: tests/x11/bloqueio-gatilho.sh [:65]
set -u
DISP="${1:-:65}"
case "$DISP" in
  :0|:0.*|:99|:99.*) echo "recusado: $DISP não é display de teste"; exit 2 ;;
esac
DIR="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$DIR/target/debug"
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

[ -x "$BIN/mide" ] && [ -x "$BIN/mide-lock" ] || { echo "faltam os binários (cargo build --bin mide --bin mide-lock -j 2)"; exit 2; }
if DISPLAY="$DISP" xdpyinfo >/dev/null 2>&1; then echo "recusado: $DISP já está em uso"; exit 2; fi

Xvfb "$DISP" -screen 0 1366x768x24 -nolisten tcp >/dev/null 2>&1 &
PID_XV=$!
sleep 2
export DISPLAY="$DISP" MIDE_BACKEND=x11 MIDE_SOCKET=mide-gatilho XDG_CONFIG_HOME="$OUT/cfg"
unset WAYLAND_DISPLAY
mkdir -p "$XDG_CONFIG_HOME"

"$BIN/mide" > "$OUT/mide.log" 2>&1 &
PID_MIDE=$!
sleep 8
travas() { pgrep -P "$PID_MIDE" -x mide-lock 2>/dev/null | wc -l; }

echo "== Super+L"
check "mide de teste no ar (controle)" kill -0 "$PID_MIDE"
xdotool key super+l
# O mide-lock leva um pouco para mapear a janela: espera até 8 s.
for _ in $(seq 1 80); do
  [ "$(travas)" -ge 1 ] && xwininfo -root -children | grep -q 'mide-lock"' && break
  sleep 0.1
done
check "Super+L abre o mide-lock (um filho do mide)" test "$(travas)" -eq 1
check "janela do mide-lock em tela inteira" bash -c "xwininfo -root -children | grep -q 'mide-lock\"'"

echo "== não duplica"
xdotool key super+l
sleep 2
check "segundo Super+L não abre outro mide-lock" test "$(travas)" -eq 1

echo "== não desbloqueia sem senha"
xdotool key a x
sleep 1
check "tecla não desbloqueia (mide-lock continua)" test "$(travas)" -eq 1
xprop -root -f _MIDE_UNLOCK 32c -set _MIDE_UNLOCK 1 2>/dev/null
sleep 1
check "propriedade _MIDE_UNLOCK na raiz não desbloqueia" test "$(travas)" -eq 1

echo "== limpeza"
for pid in $(pgrep -P "$PID_MIDE" -x mide-lock 2>/dev/null); do kill "$pid" 2>/dev/null; done
sleep 1
echo
echo "resultado: $PASS ok, $FAIL falha(s)"
[ "$FAIL" -eq 0 ]
