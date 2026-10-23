# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== monitores (telas virtuais)"
MONCFG="$(mktemp -d)"
mkdir -p "$MONCFG/mide"
OLD_MONCFG="$XDG_CONFIG_HOME"
export XDG_CONFIG_HOME="$MONCFG"
export MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080
SETTINGS="$PWD/target/debug/mide-settings"
check "duas telas lado a lado" 'tela HEADLESS-2 em \(1366, 0\) 1920x1080' "300 list"
printf '[monitors]\nprimary = "HEADLESS-2"\n' > "$MONCFG/mide/config.toml"
check "barra na tela principal escolhida" 'camada "mide-bar" em \(0, 0\) 1920x28' "300 list"
printf '[monitors.outputs."HEADLESS-2"]\nenabled = false\n' > "$MONCFG/mide/config.toml"
TELAS="$(timeout 150 "$SCEN" "300 list" 2>&1 | grep -c "autoteste: tela HEADLESS")"
if [ "$TELAS" = 1 ]; then
    echo "ok    tela desligada fica de fora"; PASS=$((PASS + 1))
else
    echo "FALHA tela desligada fica de fora (esperado 1, obtido $TELAS)"; FAIL=$((FAIL + 1))
fi
# Espelhar (A15): as duas telas no mesmo lugar, a segunda com a escala que iguala a
# largura da principal (1920 ÷ 1366 → 1366 lógicos).
printf '[monitors]\nmirror = true\n' > "$MONCFG/mide/config.toml"
check "espelhar telas: a segunda mostra a principal" 'tela HEADLESS-2 em \(0, 0\) 1366x' "300 list"
: > "$MONCFG/mide/config.toml"
# Configurações → Monitores: HEADLESS-2 no arranjo em (654, 255); "Usar como tela
# principal" em (404, 526); "Aplicar" em (430, 422); na janelinha de confirmação,
# "Manter" em (426, 289).
check "Monitores: tornar principal e manter" 'camada "mide-bar" em \(1366, 0\) 1920x28' "300 click 654 255; 330 click 404 526; 360 click 430 422; 460 click 426 289; 520 list" "$SETTINGS displays"
if grep -q 'primary = "HEADLESS-2"' "$MONCFG/mide/config.toml"; then
    echo "ok    Monitores grava a tela principal no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA Monitores grava a tela principal no config.toml"; FAIL=$((FAIL + 1))
fi
: > "$MONCFG/mide/config.toml"
# Sem confirmar, volta sozinho em 15 s (a barra volta para a primeira tela).
SCENARIO_WAIT=34 check "Monitores: sem confirmar, volta ao anterior em 15 s" 'camada "mide-bar" em \(0, 0\) 1366x28' "300 click 654 255; 330 click 404 526; 360 click 430 422; 1500 list" "$SETTINGS displays"
# Principal à direita da outra: a lâmpada (minimizar) vai até o ícone na dock dela,
# e não para a tela da esquerda. Janela em (1968, 76): minimizar em (2003, 61).
printf '[monitors]\nprimary = "HEADLESS-1"\n[monitors.outputs."HEADLESS-2"]\nx = 0\ny = 0\n[monitors.outputs."HEADLESS-1"]\nx = 1920\ny = 0\n' > "$MONCFG/mide/config.toml"
check "principal à direita: a lâmpada vai até a dock dela" "alvo no ícone da dock em Point.*x: 2511" "300 click 2003 61; 400 list" "$TERMINAL"
unset MIDE_HEADLESS_OUTPUTS
printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.25\n' > "$MONCFG/mide/config.toml"
MIDE_HEADLESS_OUTPUTS=1920x1080 check "escala 125%: tela de 1920x1080 com 1536x864 pontos" 'tela HEADLESS-1 em \(0, 0\) 1536x864' "300 list"
export XDG_CONFIG_HOME="$OLD_MONCFG"
rm -rf "$MONCFG"
