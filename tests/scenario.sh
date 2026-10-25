#!/usr/bin/env bash
# Cenário de autoteste do mide: sobe o compositor com um roteiro de ponteiro
# (MIDE_SELFTEST), abre apps, opcionalmente salva um quadro e mostra o log.
#
# Uso:
#   tests/scenario.sh "<roteiro>" [frame-do-quadro] -- "app 1" "app 2" ...
#
# Exemplo:
#   tests/scenario.sh "500 list; 520 click 540 160" 560 -- foot gnome-calculator
#
# Saída em $SCENARIO_OUT (padrão /tmp/mide-scenario): mide.log e frame.png.
# Usa o socket mide-test e encerra só os processos dele; uma sessão aberta do
# mide (mide-0) não é afetada.
# Barramento D-Bus próprio do teste: sem ele, a barra e as notificações do teste tomam
# os nomes do barramento da sessão de verdade (menus dos apps, notificações) e o desktop
# do usuário perde os menus. Só fica no barramento da sessão com MIDE_TEST_SESSION_BUS=1.
if [ -z "${MIDE_PRIVATE_BUS:-}" ] && [ -z "${MIDE_TEST_SESSION_BUS:-}" ] && command -v dbus-run-session >/dev/null; then
    export MIDE_PRIVATE_BUS=1
    exec dbus-run-session -- "$0" "$@"
fi

set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Apps abertos pelo compositor (atalhos `launch:`, dock) vêm deste build, não do
# instalado na sessão de quem roda os testes.
export PATH="$ROOT/target/debug:$PATH"
# Apps GTK do teste com as configurações padrão (sem o portal nem o dconf de quem roda
# os testes): tema, botões do cabeçalho e tamanhos iguais em qualquer máquina.
export GDK_DEBUG=no-portals ADW_DISABLE_PORTAL=1 GSETTINGS_BACKEND=memory
OUT="${SCENARIO_OUT:-/tmp/mide-scenario}"
RUNTIME="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export XDG_RUNTIME_DIR="$RUNTIME"

SCRIPT="${1:-}"
shift || true
DUMP=""
if [ "${1:-}" != "--" ] && [ -n "${1:-}" ]; then
    DUMP="$1"
    shift
fi
[ "${1:-}" = "--" ] && shift

mkdir -p "$OUT"
rm -f "$OUT"/frame.*

# Tamanho da saída: as coordenadas do tests/ui.sh são para 800x600 (o mide abre em
# 1366x768 por padrão; tests/res1366.sh testa nesse tamanho).
export MIDE_WINIT_SIZE="${MIDE_WINIT_SIZE:-800x600}"
export MIDE_WINIT_FIXED=1
# Rodando de dentro de uma sessão do mide: o mide de teste não é a sessão de login (não
# publica o ambiente no D-Bus/systemd nem inicia os apps automáticos).
unset MIDE_SESSION
# Sem janela na tela: o mide de teste desenha na GPU pelo render node (backend headless,
# com a orientação do DRM). MIDE_TEST_BACKEND=winit usa uma janela no desktop atual.
# Nunca herda o MIDE_BACKEND de quem chamou: dentro de uma sessão do mide ele é "drm",
# e um mide de teste não pode tentar tomar a tela.
if [ -n "${MIDE_TEST_BACKEND:-}" ]; then
    export MIDE_BACKEND="$MIDE_TEST_BACKEND"
elif [ -r /dev/dri/renderD128 ] && [ -w /dev/dri/renderD128 ]; then
    export MIDE_BACKEND=headless
else
    export MIDE_BACKEND=winit
fi
# Sem XWayland por padrão (um Xwayland a menos por cenário); os testes de apps X11
# ligam com MIDE_XWAYLAND=1.
export MIDE_XWAYLAND="${MIDE_XWAYLAND:-0}"
# Nada da bateria usa a internet: os apps que buscam algo (as cotações da Calculadora)
# só usam o programa falso que o teste der (MIDE_CURL); sem ele, não buscam.
export MIDE_NO_NETWORK=1

# Socket próprio dos testes: uma sessão do mide aberta (mide-0) não é tocada.
SOCK="${MIDE_TEST_SOCKET:-mide-test}"
export MIDE_SOCKET="$SOCK"

# Dados do usuário protegidos: Lixeira e dados de apps vão para uma pasta do teste
# (esvaziar a Lixeira num teste não apaga a Lixeira de verdade), e abrir arquivos
# só registra no log em vez de abrir apps no desktop.
if [ -z "${XDG_DATA_HOME:-}" ]; then
    export XDG_DATA_HOME="$OUT/data"
    rm -rf "$XDG_DATA_HOME"
    mkdir -p "$XDG_DATA_HOME"
fi
export MIDE_OPEN_DRY_RUN=1
# Estado dos apps (visualização do Arquivos, sessão do Terminal...) também numa pasta do
# teste, limpa a cada cenário; quem quer guardar entre cenários passa XDG_STATE_HOME.
if [ -z "${XDG_STATE_HOME:-}" ]; then
    export XDG_STATE_HOME="$OUT/state"
    rm -rf "$XDG_STATE_HOME"
    mkdir -p "$XDG_STATE_HOME"
fi
# Área de trabalho de teste (vazia, a menos que o teste ponha arquivos): os cliques
# dos testes nunca chegam aos arquivos da área de trabalho de verdade.
if [ -z "${MIDE_DESKTOP_DIR:-}" ]; then
    export MIDE_DESKTOP_DIR="$OUT/desktop"
    rm -rf "$MIDE_DESKTOP_DIR"
    mkdir -p "$MIDE_DESKTOP_DIR"
fi
# Som, Wi-Fi e Bluetooth simulados: nenhum teste mexe no áudio ou na rede de verdade.
export MIDE_SYSTEM_DRY_RUN=1
# Sem bateria (pasta vazia no lugar de /sys/class/power_supply): num notebook, o item
# da bateria empurra os ícones da barra e os cliques dos testes caem no vizinho.
if [ -z "${MIDE_POWER_SUPPLY_DIR:-}" ]; then
    export MIDE_POWER_SUPPLY_DIR="$OUT/power_supply"
    mkdir -p "$MIDE_POWER_SUPPLY_DIR"
fi

# Encerra só os processos deste teste: os que têm o socket de teste no ambiente
# (o compositor, a dock, a barra, o "Sobre" e os apps abertos aqui).
kill_test_processes() {
    local pid
    for pid in $(pgrep -u "$(id -u)" .); do
        [ "$pid" = "$$" ] && continue
        # Processos sem permissão de leitura (de outro usuário ou já encerrados) ficam de fora.
        if { tr '\0' '\n' < "/proc/$pid/environ"; } 2>/dev/null \
            | grep -qxE "MIDE_SOCKET=$SOCK|WAYLAND_DISPLAY=$SOCK"; then
            kill "$pid" 2>/dev/null
        fi
    done
}

cleanup() {
    kill_test_processes
    sleep 1
    rm -f "$RUNTIME/$SOCK" "$RUNTIME/$SOCK.lock" "$RUNTIME/$SOCK.ctl"
}
cleanup

ENV_DUMP=()
if [ -n "$DUMP" ]; then
    ENV_DUMP=(MIDE_DUMP_AT="$DUMP" MIDE_DUMP_FRAME="$OUT/frame")
fi

env MIDE_SELFTEST="$SCRIPT" "${ENV_DUMP[@]}" RUST_LOG="${RUST_LOG:-mide=debug,mide_dock=info}" \
    timeout "${SCENARIO_TIMEOUT:-45}" "$ROOT/target/debug/mide" >"$OUT/mide.log" 2>&1 &

for _ in $(seq 1 50); do
    [ -S "$RUNTIME/$SOCK" ] && break
    sleep 0.2
done
sleep 1

for app in "$@"; do
    WAYLAND_DISPLAY=$SOCK GDK_BACKEND=wayland timeout "${SCENARIO_TIMEOUT:-45}" sh -c "$app" >/dev/null 2>&1 &
    sleep 2
done

# Espera: com SCENARIO_WAIT, um tempo fixo (cenários que esperam algo depois do roteiro,
# como a gravação automática); sem ele, até o compositor avisar que o roteiro acabou (e o
# quadro pedido ter sido salvo), mais SCENARIO_GRACE segundos para os apps registrarem.
if [ -n "${SCENARIO_WAIT:-}" ] || [ -z "$SCRIPT" ]; then
    sleep "${SCENARIO_WAIT:-10}"
else
    for _ in $(seq 1 $((${SCENARIO_MAX:-60} * 10))); do
        if grep -aq "autoteste: roteiro concluído" "$OUT/mide.log" 2>/dev/null \
            && { [ -z "$DUMP" ] || ls "$OUT"/frame.*.rgba >/dev/null 2>&1; }; then
            break
        fi
        sleep 0.1
    done
    sleep "${SCENARIO_GRACE:-1.5}"
fi

if [ -n "$DUMP" ] && ls "$OUT"/frame.*.rgba >/dev/null 2>&1; then
    python3 - "$OUT" <<'PY'
import glob, sys
from PIL import Image
out = sys.argv[1]
path = glob.glob(out + "/frame.*.rgba")[0]
w, h = map(int, path.rsplit(".", 2)[1].split("x"))
data = open(path, "rb").read()
img = Image.frombytes("RGBA", (w, h), data).convert("RGB").transpose(Image.FLIP_TOP_BOTTOM)
img.save(out + "/frame.png")
PY
fi

sed 's/\x1b\[[0-9;]*m//g' "$OUT/mide.log" | grep -E "autoteste|clique|tratado|janela|camada|mide_dock|mide_bar|mide_launcher|mide_notifications|mide_settings|(mide|mide_ui)::([a-z_]+::)?(places|programs|system)|lâmpada|papel de parede|configuração recarregada|vidro: |área de transferência|arrasto entre apps|abrindo pela dock|tema do mide|panic|ERROR|WARN" | grep -v "frame:"
cleanup
