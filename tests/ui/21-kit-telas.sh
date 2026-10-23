# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== mide-ui em todas as telas (exemplo kit-telas, telas virtuais)"
# examples/kit-telas (`Window::on_every_output`): uma camada escurecida em cada tela; o
# app registra no log os cliques e arrastos em coordenadas globais. HEADLESS-2 fica em
# (1366, 0), à direita da principal. Arrastar de uma tela para a outra prova a pega
# implícita do compositor (o ponteiro continua com a superfície onde o botão desceu).
KIT_TELAS="$PWD/target/debug/examples/kit-telas"
if [ -x "$KIT_TELAS" ]; then
    KT="$(mktemp -d)"
    mkdir -p "$KT/cfg/mide"
    # kit_telas "roteiro" "config.toml" "comando extra": roda o exemplo; log do app em
    # $KT/app.log, do compositor em $KT/scen.txt, uma captura de cada tela em $KT/t-*.png.
    kit_telas() {
        printf '%b' "$2" > "$KT/cfg/mide/config.toml"
        rm -f "$KT"/app.log "$KT"/t-*.png
        XDG_CONFIG_HOME="$KT/cfg" SCENARIO_OUT="$KT/scen" MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT="${KT_WAIT:-14}" timeout 150 "$SCEN" "$1" \
            -- "RUST_LOG=kit_telas=info,mide=info $KIT_TELAS > $KT/app.log 2>&1" "sh -c 'sleep 5; mide-ctl screenshot all $KT/t > /dev/null; ${3:-true}'" > "$KT/scen.txt" 2>&1
    }
    # kt_check "nome" "regex" [arquivo]: o log (do app, por padrão) casa.
    kt_check() {
        if grep -aEq "$2" "${3:-$KT/app.log}"; then
            echo "ok    $1"; PASS=$((PASS + 1))
        else
            echo "FALHA $1 (esperado: $2)"; FAIL=$((FAIL + 1))
        fi
    }
    # Sem o app: as telas como estão, para comparar o escurecido.
    XDG_CONFIG_HOME="$KT/cfg" SCENARIO_OUT="$KT/scen" MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT=10 timeout 150 "$SCEN" "400 list" \
        -- "sh -c 'sleep 5; mide-ctl screenshot all $KT/base > /dev/null'" >/dev/null 2>&1
    kit_telas "400 click 1600 300; 450 press 1300 300; 460 move 1400 300; 480 release 1500 300; 520 press 1600 300; 530 move 1400 300; 540 release 1266 300; 580 click 200 200; 620 list" ""
    kt_check "kit em todas as telas: camada na segunda tela" 'camada "kit-telas@HEADLESS-2" em \(1366, 0\) 1920x1080' "$KT/scen.txt"
    kt_check "kit em todas as telas: o passe da segunda tela sabe qual é" "desenhou a tela HEADLESS-2 em \(1366, 0\) 1920x1080 \(passe extra\)"
    kt_check "kit em todas as telas: clique na segunda tela em coordenadas globais" "clique global \(1600, 300\) na tela HEADLESS-2"
    kt_check "kit em todas as telas: arrasto da principal para a segunda" "arrasto global de \(1300, 300\) a \(1500, 300\), soltou na tela HEADLESS-2"
    kt_check "kit em todas as telas: arrasto da segunda para a principal" "arrasto global de \(1600, 300\) a \(1266, 300\), soltou na tela HEADLESS-1"
    kt_check "kit em todas as telas: clique na principal" "clique global \(200, 200\) na tela HEADLESS-1"
    if python3 -c "
import sys
from PIL import Image
d = sys.argv[1]
for n in ('HEADLESS-1', 'HEADLESS-2'):
    a = Image.open(d + '/base-' + n + '.png').convert('RGB').getpixel((700, 400))
    b = Image.open(d + '/t-' + n + '.png').convert('RGB').getpixel((700, 400))
    if not sum(b) < sum(a) * 0.8: sys.exit('%s não escureceu: %s %s' % (n, a, b))
" "$KT"; then
        echo "ok    kit em todas as telas: as duas telas escurecidas"; PASS=$((PASS + 1))
    else
        echo "FALHA kit em todas as telas: as duas telas escurecidas"; FAIL=$((FAIL + 1))
    fi
    # Principal à direita (HEADLESS-1 em (1920, 0), HEADLESS-2 em (0, 0)): a superfície
    # do app começa em 1920 e a outra tela fica à esquerda, com coordenadas menores.
    kit_telas "400 click 2000 200; 430 click 300 300; 450 press 1800 300; 460 move 1900 300; 480 release 2100 300; 520 press 2100 300; 530 move 1900 300; 540 release 1800 300; 600 key esc; 640 list" \
        '[monitors]\nprimary = "HEADLESS-1"\n[monitors.outputs."HEADLESS-2"]\nx = 0\ny = 0\n[monitors.outputs."HEADLESS-1"]\nx = 1920\ny = 0\n'
    kt_check "kit com a principal à direita: clique na principal" "clique global \(2000, 200\) na tela HEADLESS-1"
    kt_check "kit com a principal à direita: clique na outra tela" "clique global \(300, 300\) na tela HEADLESS-2"
    kt_check "kit com a principal à direita: arrasto da outra para a principal" "arrasto global de \(1800, 300\) a \(2100, 300\), soltou na tela HEADLESS-1"
    kt_check "kit com a principal à direita: arrasto da principal para a outra" "arrasto global de \(2100, 300\) a \(1800, 300\), soltou na tela HEADLESS-2"
    if grep -q 'autoteste: camada "kit-telas' "$KT/scen.txt"; then
        echo "FALHA kit em todas as telas: Esc fecha (teclado na camada principal)"; FAIL=$((FAIL + 1))
    else
        echo "ok    kit em todas as telas: Esc fecha (teclado na camada principal)"; PASS=$((PASS + 1))
    fi
    # Espelhando, as duas telas ocupam o mesmo lugar: uma camada só.
    kit_telas "400 list" '[monitors]\nmirror = true\n'
    if grep -aq "telas do app: HEADLESS-1 em (0, 0) 1366x768 principal" "$KT/app.log" && ! grep -aq "criada" "$KT/app.log"; then
        echo "ok    kit em todas as telas: espelhando, uma camada só"; PASS=$((PASS + 1))
    else
        echo "FALHA kit em todas as telas: espelhando, uma camada só"; FAIL=$((FAIL + 1))
    fi
    # A segunda tela é desligada com o app aberto e volta: a camada dela sai e volta, e
    # o app continua (só a camada principal fechada encerra o app).
    printf '[monitors.outputs."HEADLESS-2"]\nenabled = false\n' > "$KT/desligada.toml"
    : > "$KT/ligada.toml"
    KT_WAIT=20 kit_telas "900 click 1600 300; 1000 list" "" \
        "cp $KT/desligada.toml $KT/cfg/mide/config.toml; mide-ctl outputs apply > /dev/null; sleep 3; cp $KT/ligada.toml $KT/cfg/mide/config.toml; mide-ctl outputs apply > /dev/null"
    kt_check "kit em todas as telas: monitor desligado tira a camada dele" "tela HEADLESS-2 saiu: camada fechada"
    kt_check "kit em todas as telas: monitor ligado de novo ganha a camada" "clique global \(1600, 300\) na tela HEADLESS-2"
    kt_check "kit em todas as telas: o app continua aberto" 'camada "kit-telas@HEADLESS-2" em \(1366, 0\)' "$KT/scen.txt"
    # A principal muda com o app aberto: o compositor não muda a camada principal de
    # lugar (continua em HEADLESS-1, agora em (1920, 0)); a extra fica na HEADLESS-2,
    # que vai para (0, 0) e recebe o clique.
    printf '[monitors]\nprimary = "HEADLESS-2"\n' > "$KT/p2.toml"
    KT_WAIT=20 kit_telas "900 click 600 300; 1000 list" "" "cp $KT/p2.toml $KT/cfg/mide/config.toml; mide-ctl outputs apply > /dev/null"
    kt_check "kit com a principal trocada: a camada principal fica onde estava" 'camada "kit-telas" em \(1920, 0\) 1366x768' "$KT/scen.txt"
    kt_check "kit com a principal trocada: a nova principal tem a camada extra" 'camada "kit-telas@HEADLESS-2" em \(0, 0\) 1920x1080' "$KT/scen.txt"
    kt_check "kit com a principal trocada: clique na nova principal chega ao app" "clique global \(600, 300\) na tela HEADLESS-2"
    # A tela da camada principal é desligada com o app aberto: o app fecha (a camada
    # ficaria invisível segurando o teclado).
    printf '[monitors.outputs."HEADLESS-1"]\nenabled = false\n' > "$KT/d1.toml"
    rm -f "$KT/vivo"
    KT_WAIT=20 kit_telas "1000 list" "" "cp $KT/d1.toml $KT/cfg/mide/config.toml; mide-ctl outputs apply > /dev/null; sleep 4; pgrep -x kit-telas > /dev/null && touch $KT/vivo"
    kt_check "kit com a principal desligada: o app percebe" "a tela HEADLESS-1 da camada principal saiu"
    if [ -e "$KT/vivo" ] || grep -aq 'autoteste: camada "kit-telas' "$KT/scen.txt"; then
        echo "FALHA kit com a principal desligada: o app fecha"; FAIL=$((FAIL + 1))
    else
        echo "ok    kit com a principal desligada: o app fecha"; PASS=$((PASS + 1))
    fi
    rm -rf "$KT"
else
    echo "pulo  kit em todas as telas (rode cargo build --examples)"
fi
