#!/bin/bash
# Bordas das janelas X11 decoradas pelo mide: a borda (3 px, na cor do estado) fica por cima
# do cliente e de qualquer janela que passe por baixo, em vez de ficar escondida no fundo.
# Um xclock (acima) sobre um xterm: as bordas do xclock aparecem azuis nos lados e embaixo,
# também dentro da área do xterm. Usa Xvfb num display de teste (recusa :0 e :99).
#
# Uso: tests/x11/bordas-sobreposicao.sh [:74]
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
  pkill -f "bordaA" 2>/dev/null; pkill -f "bordaB" 2>/dev/null
  rm -rf "$OUT"
}
trap cleanup EXIT

[ -x "$DIR/target/debug/mide" ] || { echo "falta target/debug/mide (cargo build --bin mide -j 2)"; exit 2; }
if DISPLAY="$DISP" xdpyinfo >/dev/null 2>&1; then echo "recusado: $DISP já está em uso"; exit 2; fi

mkdir -p "$OUT/rt" "$OUT/home" && chmod 700 "$OUT/rt"
Xvfb "$DISP" -screen 0 1366x768x24 -nolisten tcp >/dev/null 2>&1 &
PID_XV=$!
sleep 2
export DISPLAY="$DISP" MIDE_BACKEND=x11 MIDE_SOCKET=mide-bordas XDG_RUNTIME_DIR="$OUT/rt" HOME="$OUT/home" XDG_CONFIG_HOME="$OUT/home/.config"
unset WAYLAND_DISPLAY

"$DIR/target/debug/mide" > "$OUT/mide.log" 2>&1 &
PID_MIDE=$!
for _ in $(seq 1 100); do xwininfo -root -tree 2>/dev/null | grep -q 'mide-desktop' && break; sleep 0.1; done
sleep 1

# Fundo (xterm) embaixo, relógio (xclock) por cima, cobrindo parte dele.
xterm -name bordaB -geometry 100x30+60+150 -e sleep 60 >/dev/null 2>&1 &
for _ in $(seq 1 50); do xdotool search --class xterm >/dev/null 2>&1 && break; sleep 0.1; done
sleep 1
xclock -name bordaA -geometry 220x220+320+230 >/dev/null 2>&1 &
for _ in $(seq 1 50); do xdotool search --class xclock >/dev/null 2>&1 && break; sleep 0.1; done
sleep 2

CLK=$(xdotool search --class xclock | head -1)
# Geometria absoluta do cliente (a moldura é 30 px mais acima; as bordas estão na moldura).
GEO=$(xwininfo -id "$CLK" | awk '/Absolute upper-left X/ {x=$4} /Absolute upper-left Y/ {y=$4} /Width:/ {w=$2} /Height:/ {h=$2} END {print x, y, w, h}')
read -r AX AY AW AH <<<"$GEO"
echo "== bordas do xclock por cima do xterm (cliente em $AX,$AY, ${AW}x$AH)"

import -window root "$OUT/tela.png" 2>/dev/null
px() { python3 -c "
from PIL import Image
im = Image.open('$OUT/tela.png').convert('RGB')
print('%d,%d,%d' % im.getpixel(($1, $2)))
"; }
AZUL="16,48,96"
check "borda esquerda do xclock em azul (sobre o xterm)" test "$(px $((AX+1)) $((AY+AH/2)))" = "$AZUL"
check "borda direita do xclock em azul" test "$(px $((AX+AW-2)) $((AY+AH/2)))" = "$AZUL"
check "borda inferior do xclock em azul" test "$(px $((AX+AW/2)) $((AY+AH-2)))" = "$AZUL"
check "fora do xclock continua o xterm (sem borda azul)" test "$(px $((AX-30)) $((AY+AH/2)))" != "$AZUL"

echo
echo "resultado: $PASS ok, $FAIL falhas"
[ "$FAIL" -eq 0 ]
