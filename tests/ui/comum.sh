# Base das seções da bateria (tests/ui.sh roda cada tests/ui/NN-*.sh com isto antes,
# na raiz do projeto, cada seção com o próprio socket e a própria pasta de saída).
#
# Bateria de regressão da interface: janelas, controles, popups, barra e dock.
#
# Usa tests/scenario.sh (autoteste por MIDE_SELFTEST) e confere o log de cada
# cenário. Usa o Terminal e a Calculadora do mide (a gnome-calculator, se
# instalada, testa também as janelas com cabeçalho do próprio app).
#
# Coordenadas (saída 1366x768, a tela mais comum no Brasil; barra superior de 28 px):
# - A primeira janela nasce em (48, 76) e a segunda em (80, 108).
# - Janela de abertura (tamanho fixo, ou pequena e pedindo "sem decoração") nasce no
#   meio da tela do ponteiro, e diálogo com pai, centrado sobre a pai (A26, seção 34).
#   "Sobre este computador" (330x610, tamanho fixo) fica em (518, 59).
# - Barra do mide numa janela em (48, 76): moldura em y=46, botões no centro
#   y=61: fechar x=63, minimizar x=83, maximizar x=103.
# - Calculadora (360x509, cabeçalho próprio, configurações padrão do GTK: só o X) em
#   (48, 76): X em (384, 99), menu (345, 99); duplo clique no cabeçalho (300, 99)
#   maximiza; borda direita em x=408.
# - Maximizada: área útil de y=28 até 2 px acima da dock (y=701).
# - Dock padrão (ícones de 40 px) em (541, 703) 285x57: Launchpad em (568, 728),
#   Terminal em (611, 728), Calculadora em (654, 728), Configurações em (697, 728),
#   Downloads em (756, 728) e Lixeira em (799, 728).
# - Menu de contexto de um app aberto (clique direito): "Manter/Remover da Dock" em
#   y=609, "Ocultar" em y=644 e "Encerrar" em y=670. App fechado: "Remover" em y=670.
# - Barra superior: marca em (25, 14); menu: "Sobre este computador" em (100, 49).
#   À direita: relógio (1331, 14), central de controle (1293, 14), Bluetooth (1257, 14),
#   Wi-Fi (1221, 14), som (1185, 14) e engrenagem (1149, 14).
set -u

SCEN=tests/scenario.sh
export MIDE_WINIT_SIZE=1366x768
PASS=0
FAIL=0

# Todas as sessões de teste usam uma configuração temporária e vazia: nada que
# aconteça numa janela de teste (nem um clique de quem estiver usando o PC)
# grava na configuração real.
TESTCFG="$(mktemp -d)"
export XDG_CONFIG_HOME="$TESTCFG"
# As pastas do usuário (Documentos, Imagens...) vêm do xdg-user-dirs, como na sessão.
[ -f "$HOME/.config/user-dirs.dirs" ] && cp "$HOME/.config/user-dirs.dirs" "$TESTCFG/"
trap 'rm -rf "$TESTCFG"' EXIT

# check "nome" "regex esperado no log" "roteiro" app...
check() {
    local name="$1" expected="$2" script="$3"
    shift 3
    local log
    log="$(timeout 150 "$SCEN" "$script" -- "$@" 2>&1)"
    if echo "$log" | grep -Eq "$expected"; then
        echo "ok    $name"
        PASS=$((PASS + 1))
    else
        echo "FALHA $name"
        echo "      esperado: $expected"
        echo "$log" | grep -E "autoteste: (janela|camada)|janela (fechada|minimizada|restaurada)" | sed 's/^/      /' | tail -4
        FAIL=$((FAIL + 1))
    fi
}

# check_last "nome" "regex" "roteiro" app...: a última janela listada (a do topo) casa.
check_last() {
    local name="$1" expected="$2" script="$3"
    shift 3
    local last
    last="$(timeout 150 "$SCEN" "$script" -- "$@" 2>&1 | grep "autoteste: janela" | tail -1)"
    if echo "$last" | grep -Eq "$expected"; then
        echo "ok    $name"
        PASS=$((PASS + 1))
    else
        echo "FALHA $name"
        echo "      topo: $last"
        FAIL=$((FAIL + 1))
    fi
}

# check_count "nome" "regex" quantidade "roteiro" app...: quantas linhas de listagem casam.
check_count() {
    local name="$1" pattern="$2" want="$3" script="$4"
    shift 4
    local got
    got="$(timeout 150 "$SCEN" "$script" -- "$@" 2>&1 | grep -E "autoteste: (janela|camada|tela)" | grep -Ec "$pattern")"
    if [ "$got" = "$want" ]; then
        echo "ok    $name"
        PASS=$((PASS + 1))
    else
        echo "FALHA $name (esperado $want, obtido $got)"
        FAIL=$((FAIL + 1))
    fi
}

TERMINAL="$PWD/target/debug/mide-terminal --title Terminal"
CALCULATOR="$PWD/target/debug/mide-calculator"
FOOT='janela "Terminal" popups=0'
CALC='janela "Calculadora"'
CALC_OPEN='janela "Calculadora"'
