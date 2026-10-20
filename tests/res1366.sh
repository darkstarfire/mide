#!/usr/bin/env bash
# Bateria em 1366x768 (a resolução de tela mais comum no Brasil): confere que barra,
# dock, launcher e janelas se ajustam a esse tamanho, e salva capturas de tela para
# revisão em $RES_OUT (padrão: /tmp/mide-1366).
#
# Coordenadas (saída 1366x768): dock padrão (ícones de 40 px) em (541, 703) 285x57,
# com Launchpad em (568, 728), foot em (611, 728), calculadora em (654, 728),
# Configurações em (697, 728), Downloads em (756, 728) e Lixeira em (799, 728).
set -u
cd "$(dirname "$0")/.."
SCEN=tests/scenario.sh
export MIDE_WINIT_SIZE=1366x768
export SCENARIO_WAIT="${SCENARIO_WAIT:-8}"
RES_OUT="${RES_OUT:-/tmp/mide-1366}"
mkdir -p "$RES_OUT"
TESTCFG="$(mktemp -d)"
export XDG_CONFIG_HOME="$TESTCFG"
trap 'rm -rf "$TESTCFG"' EXIT
PASS=0
FAIL=0
FOOT='janela "foot" popups=0'

check() {
    local name="$1" expected="$2" script="$3"
    shift 3
    if timeout 150 "$SCEN" "$script" -- "$@" 2>&1 | grep -Eq "$expected"; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name"; echo "      esperado: $expected"; FAIL=$((FAIL + 1))
    fi
}

# shot "nome" "roteiro" quadro app...: salva a captura do quadro em $RES_OUT/nome.png.
shot() {
    local name="$1" script="$2" frame="$3"
    shift 3
    local out
    out="$(mktemp -d)"
    SCENARIO_OUT="$out" timeout 150 "$SCEN" "$script" "$frame" -- "$@" >/dev/null 2>&1
    if [ -f "$out/frame.png" ]; then
        cp "$out/frame.png" "$RES_OUT/$name.png"
        echo "foto  $RES_OUT/$name.png"
    else
        echo "      (sem captura de $name: a janela de teste pode ter ficado escondida)"
    fi
    rm -rf "$out"
}

check "barra ocupa a largura toda" 'camada "mide-bar" em \(0, 0\) 1366x28' "300 list"
check "dock centralizada embaixo" 'camada "mide-dock" em \(541, 703\) 285x57' "300 list"
check "Launchpad na dock abre o launcher em tela cheia" 'camada "mide-launcher" em \(0, 0\) 1366x768' "300 click 568 728; 380 list"
check "clique na dock abre o app" "nova janela criada" "300 click 611 728; 420 list"
check "janela nova nasce abaixo da barra" "$FOOT posição=\(48, 76\)" "300 list" foot
check "maximizar ocupa a área útil" "$FOOT posição=\(0, 58\) visível=1366x643" "300 click 103 61; 360 list" foot
check "Arquivos cabe na tela" 'janela "[^"]*" popups=0 posição=\(48, 76\) visível=900x520' "300 list" "target/debug/mide-files /usr"
check "Configurações cabem na tela" 'janela "Configurações do Sistema" popups=0 posição=\(48, 76\)' "300 list" target/debug/mide-settings

shot desktop "150 move 683 400" 220
shot launcher "150 combo ctrl+shift+space; 200 move 683 400" 300
shot arquivos "250 move 700 400" 320 "target/debug/mide-files /usr/share"
shot configuracoes "250 move 700 400" 320 "target/debug/mide-settings effects"
shot menu-dock "200 rclick 799 728; 240 move 820 640" 300

# Orientação do DRM (MIDE_SIM_DRM=1: a cena é desenhada sem inversão, como no backend
# DRM): a barra de título do Arquivos (moldura em (48, 46)) sai clara e o conteúdo
# perto do topo da janela aparece. Com o recorte invertido, a barra ficava escura e
# transparente e o topo da janela sumia.
drm_out="$(mktemp -d)"
MIDE_SIM_DRM=1 SCENARIO_OUT="$drm_out" timeout 150 "$SCEN" "250 move 700 400" 320 -- "target/debug/mide-files /usr/share" >/dev/null 2>&1
if [ -f "$drm_out/frame.png" ] && python3 - "$drm_out/frame.png" "$RES_OUT/arquivos-drm.png" <<'PY'
import sys
from PIL import Image
img = Image.open(sys.argv[1])
img.save(sys.argv[2])
bright = lambda p: sum(img.getpixel(p)[:3]) / 3
# Barra de título (meio, y=61) e barra de ferramentas (y=90) claras.
sys.exit(0 if bright((300, 61)) > 200 and bright((600, 90)) > 200 else 1)
PY
then
    echo "ok    orientação do DRM: janela inteira com a barra de título clara"; PASS=$((PASS + 1))
    echo "foto  $RES_OUT/arquivos-drm.png"
else
    echo "FALHA orientação do DRM: janela inteira com a barra de título clara"; FAIL=$((FAIL + 1))
fi
rm -rf "$drm_out"

echo "resultado: $PASS ok, $FAIL falha(s)"
[ "$FAIL" = 0 ]
