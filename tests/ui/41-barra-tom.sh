# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== barra superior: tom do que está atrás dela"
# A barra sem fundo (Tahoe) troca letras e ícones entre branco e preto pelo que o
# compositor mede atrás dela (`launcher state` diz `tone dark|light`), sem esperar outro
# evento da barra. Papéis de parede lisos (escuro e claro) e, no fim, uma faixa clara
# na camada Bottom por cima da zona da barra (examples/mide-layer-popup-teste fundo),
# que escurece sozinha depois de 4 s.
TOM="$(mktemp -d)"
python3 - "$TOM" <<'PY'
import sys
from PIL import Image
d = sys.argv[1]
Image.new("RGB", (1366, 768), (22, 24, 34)).save(d + "/escuro.png")
Image.new("RGB", (1366, 768), (236, 236, 240)).save(d + "/claro.png")
PY
# tom "nome" papel-de-parede "roteiro-do-app" regex1 regex2: o log tem regex1 e, depois
# dele, regex2.
tom() {
    local name="$1" wall="$2" app="$3" first="$4" second="$5" log
    mkdir -p "$TOM/cfg/mide"
    printf '[desktop]\nwallpaper = "%s"\n' "$TOM/$wall.png" > "$TOM/cfg/mide/config.toml"
    log="$(XDG_CONFIG_HOME="$TOM/cfg" RUST_LOG="mide=info,mide_bar=info" SCENARIO_OUT="$TOM/scen" SCENARIO_WAIT=11 timeout 150 "$SCEN" "" -- "$app" 2>&1)"
    if echo "$log" | grep -Eq "$first" && echo "$log" | sed -n "/$first/,\$p" | grep -Eq "$second"; then
        echo "ok    $name"
        PASS=$((PASS + 1))
    else
        echo "FALHA $name"
        echo "$log" | grep -E "barra: (fundo|atrás)" | sed 's/^/      /' | tail -4
        FAIL=$((FAIL + 1))
    fi
}
CTL="$PWD/target/debug/mide-ctl"
tom "papel de parede escuro: texto branco; trocado para claro, texto escuro sem outro evento" escuro \
    "sleep 4; sed -i 's/escuro.png/claro.png/' '$TOM/cfg/mide/config.toml'; $CTL reload" \
    "barra: fundo atrás escuro" "barra: fundo atrás claro"
tom "papel de parede claro: texto escuro; trocado para escuro, texto branco" claro \
    "sleep 4; sed -i 's/claro.png/escuro.png/' '$TOM/cfg/mide/config.toml'; $CTL reload" \
    "barra: fundo atrás claro" "barra: fundo atrás escuro"
BAND="$PWD/target/debug/examples/mide-layer-popup-teste"
if [ -x "$BAND" ]; then
    tom "algo claro atrás da barra (papel escuro) muda o texto na hora e volta quando escurece" escuro \
        "$BAND fundo" "barra: fundo atrás claro" "barra: fundo atrás escuro"
else
    echo "pulo  faixa atrás da barra (rode cargo build --examples)"
fi
# Quadro: com a faixa clara atrás, o texto do relógio (à direita, y 8..20) é escuro.
if [ -x "$BAND" ]; then
    printf '[desktop]\nwallpaper = "%s"\n' "$TOM/escuro.png" > "$TOM/cfg/mide/config.toml"
    XDG_CONFIG_HOME="$TOM/cfg" SCENARIO_OUT="$TOM/scen" timeout 150 "$SCEN" "240 list" 240 -- "$BAND fundo" >/dev/null 2>&1
    if python3 -c "
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert('L')
box = im.crop((1290, 6, 1366, 22))
dark = sum(1 for p in box.getdata() if p < 90)
sys.exit(0 if dark >= 15 and im.getpixel((700, 60)) > 200 else 1)" "$TOM/scen/frame.png" 2>/dev/null; then
        echo "ok    quadro: texto da barra escuro sobre a faixa clara"; PASS=$((PASS + 1))
    else
        echo "FALHA quadro: texto da barra escuro sobre a faixa clara"; FAIL=$((FAIL + 1))
    fi
fi
rm -rf "$TOM"
