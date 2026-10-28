# Funções da Calculadora usadas pelas seções 16-calculadora-*.sh.
# A Calculadora lembra o modo em [calculator] do config.toml: cada teste usa uma
# configuração própria (um modo lembrado mudaria o tamanho dela nos outros testes) e o
# log dela (RUST_LOG=mide_calculator=debug, no stdout) registra a tela ("calculadora:
# tela 14"). Janela em (48, 76).
# Teclas redondas do visual do Tahoe (24/10), com a coluna c e a linha r a partir de 0:
# - Básica (300x490): x = 92 + 70,5c, y = 240 + 70,5r (linha 0: ⌫ AC % ÷; linha 1:
#   7 8 9 ×; linha 4: ± 0 , =). Com RPN (300x548), a fileira da pilha (x↔y R↓ R↑ LASTx)
#   vem antes e as linhas ficam em y = 237 + 68,7r.
# - Científica (680x470): x = 90 + 66,2c, y = 239 + 66,2r (linha 0: ( ) mc m+ m− mr;
#   linha 1: 2nd x² x³ xʸ eˣ 10ˣ; linha 3: x! sen cos tg e EE; o teclado da Básica nas
#   colunas 6 a 9).
# - Programador (520x600): bases HEX/DEC/OCT/BIN em y=131, x = 88 + 58i; bits em
#   y=271 (63–32) e y=315 (31–0), bit n de 31 a 0 em x = 62 + 15,375(31 − n) + 7,7.
# - Barra: "Visualizar" em x=232; no menu, Básica y=55, Científica 79, Programador 103,
#   Notas matemáticas 127, Conversor de unidades 208.
# - Notas (720x480): a nota começa em x=276, y=145 (linhas de 23 pt); botão da Lixeira
#   em (738, 102).
CDIR="$(mktemp -d)"
# calc_run "roteiro" [opções de [calculator]]: abre a Calculadora (com CALC_SCEN, outro
# cenário, como o do barramento D-Bus de teste). As notas e a Lixeira ficam em
# $CDIR/data (XDG_DATA_HOME só da Calculadora), apagada a cada execução (com
# CALC_KEEP_DATA=1, fica a da execução anterior). As cotações vêm do curl falso
# (tests/data/bcb/curl, com os boletins de 20 e 21/10/2026; nada vai à internet), com
# o cache em $CDIR/data/cache e os pedidos anotados em $CDIR/data/curl/chamadas; a hora
# fica em 21/10/2026 15h (o boletim do dia já saiu). CALC_NO_CURL=1 tira o curl falso.
calc_run() {
    local script="$1" saved="${2:-}"
    rm -rf "$CDIR/cfg" "$CDIR/calc.log"
    [ -z "${CALC_KEEP_DATA:-}" ] && rm -rf "$CDIR/data"
    mkdir -p "$CDIR/cfg/mide" "$CDIR/data/curl"
    [ -n "$saved" ] && printf '[calculator]\n%b\n' "$saved" > "$CDIR/cfg/mide/config.toml"
    local curl="MIDE_CURL='$PWD/tests/data/bcb/curl'"
    [ -n "${CALC_NO_CURL:-}" ] && curl=""
    # A Calculadora só abre depois da barra: aberta antes, a janela nasce em (48, 48), sem
    # a faixa da barra, e os cliques caem 28 pt abaixo das teclas (acontece com a bateria
    # em paralelo, com a máquina carregada). Espera no máximo 15 s.
    local bar="for i in \$(seq 150); do grep -aqs \"camada .mide-bar. criada\" \"\${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log\" && break; sleep 0.1; done;"
    XDG_CONFIG_HOME="$CDIR/cfg" timeout 150 "${CALC_SCEN:-$SCEN}" "$script" -- "$bar RUST_LOG=mide_calculator=debug XDG_DATA_HOME='$CDIR/data' XDG_CACHE_HOME='$CDIR/data/cache' MIDE_CURL_DIR='$CDIR/data/curl' MIDE_RATES_NOW=2026-10-21T15:00:00-03:00 $curl $CALCULATOR >'$CDIR/calc.log' 2>&1" > "$CDIR/scen.log" 2>&1
}
# calc_calls "nome" N: o curl falso recebeu N pedidos na última execução.
calc_calls() {
    local got
    got="$(cat "$CDIR/data/curl/chamadas" 2>/dev/null | wc -l)"
    if [ "$got" = "$2" ]; then echo "ok    $1"; PASS=$((PASS + 1)); else echo "FALHA $1 (esperado $2 pedido(s), obtido $got)"; FAIL=$((FAIL + 1)); fi
}
# calc_screen "nome" "tela esperada" "roteiro" [opções]: confere a última tela.
calc_screen() {
    calc_run "$3" "${4:-}"
    local got
    got="$(grep -a "calculadora: tela " "$CDIR/calc.log" | tail -1 | sed 's/.*calculadora: tela //')"
    if [ "$got" = "$2" ]; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1 (esperado \"$2\", obtido \"$got\")"; FAIL=$((FAIL + 1))
    fi
}
# calc_window "nome" "regex da janela no autoteste" "roteiro" [opções].
calc_window() {
    calc_run "$3" "${4:-}"
    if grep -a "autoteste: janela" "$CDIR/scen.log" | tail -1 | grep -Eq "$2"; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1"; echo "      esperado: $2"
        grep -a "autoteste: janela" "$CDIR/scen.log" | tail -1 | sed 's/^/      /'; FAIL=$((FAIL + 1))
    fi
}
# Conversor de unidades (E3b, etapa 2): o visor ganha 64 pt e as teclas descem; o ⇅
# fica em (77, 187). O log registra "calculadora: conversão 100 °F = 37,7777777778 °C".
# calc_convert "nome" "conversão esperada" "roteiro" [opções]: a última conversão.
calc_convert() {
    calc_run "$3" "${4:-}"
    local got
    got="$(grep -a "calculadora: conversão " "$CDIR/calc.log" | tail -1 | sed 's/.*calculadora: conversão //')"
    if [ "$got" = "$2" ]; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1 (esperado \"$2\", obtido \"$got\")"; FAIL=$((FAIL + 1))
    fi
}
# calc_log "nome" "regex" e calc_config "nome" "regex": conferem o log e o config.toml da
# última execução (várias conferências de uma vez só).
calc_log() {
    if grep -aEq "$2" "$CDIR/calc.log"; then echo "ok    $1"; PASS=$((PASS + 1)); else echo "FALHA $1 (sem \"$2\" no log)"; FAIL=$((FAIL + 1)); fi
}
calc_config() {
    if grep -Eq "$2" "$CDIR/cfg/mide/config.toml" 2>/dev/null; then echo "ok    $1"; PASS=$((PASS + 1)); else echo "FALHA $1 (sem \"$2\" no config.toml)"; FAIL=$((FAIL + 1)); fi
}
# Notas matemáticas (E3b, etapa 2): o log registra o que muda em cada linha
# ("calculadora: notas: linha 1 = 360" ou "...linha 4: erro: ...").
# notes_line "nome" N "resultado": a última linha "notas: linha N" da última execução
# (" = 360" ou ": erro: ..."); várias conferências de uma vez só.
notes_line() {
    local got
    got="$(grep -a "calculadora: notas: linha $2[ :]" "$CDIR/calc.log" | tail -1 | sed "s/.*calculadora: notas: linha $2//")"
    if [ "$got" = "$3" ]; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1 (esperado \"$3\", obtido \"$got\")"; FAIL=$((FAIL + 1))
    fi
}
# calc_file "nome" "caminho dentro de $CDIR/data (com *)" "conteúdo": confere um arquivo
# das notas ou da Lixeira.
calc_file() {
    local got
    got="$(cat "$CDIR"/data/$2 2>/dev/null)"
    if [ "$got" = "$3" ]; then
        echo "ok    $1"; PASS=$((PASS + 1))
    else
        echo "FALHA $1 (esperado \"$3\", obtido \"$got\")"; FAIL=$((FAIL + 1))
    fi
}
