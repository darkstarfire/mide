# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== notificações (barramento D-Bus de teste)"
# O servidor do mide atende num barramento próprio (tests/scenario-dbus.sh). A
# camada fica em (1002, 28): banner em x=1012..1356; com botões, "Abrir" em (1238, 121);
# o × aparece com o mouse em cima, no canto em (1016, 42).
NOTE1="notify-send -a Calculadora 'Download concluído' 'Arquivo pronto'"
NOTE2="notify-send -a Terminal -A ok=Abrir -A no=Ignorar 'Atualizações' 'Há 12 atualizações'"
SCEN_PLAIN="$SCEN"
SCEN=tests/scenario-dbus.sh
check "notificação aparece abaixo da barra, à direita" 'camada "mide-notifications" em \(1002, 28\) 364x94' "250 list" "$NOTE1"
check "notificação some sozinha depois de 5 s" "notificação 1 expirou" "700 list" "$NOTE1"
# Sons (A22; nos testes, só registrados no log): notificação toca; com os efeitos
# sonoros desligados, não.
SONS="$(mktemp -d)"
mkdir -p "$SONS/mide"
timeout 150 "$SCEN" "250 list" -- "$NOTE1" >/dev/null 2>&1
if grep -aq "som (teste): message-new-instant" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log"; then
    echo "ok    som de notificação"; PASS=$((PASS + 1))
else
    echo "FALHA som de notificação"; FAIL=$((FAIL + 1))
fi
printf '[sounds]\nenabled = false\n' > "$SONS/mide/config.toml"
XDG_CONFIG_HOME="$SONS" timeout 150 "$SCEN" "250 list" -- "$NOTE1" >/dev/null 2>&1
if grep -aq "notificação 1: Download" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log" && ! grep -aq "som (teste)" "${SCENARIO_OUT:-/tmp/mide-scenario}/mide.log"; then
    echo "ok    efeitos sonoros desligados: notificação sem som"; PASS=$((PASS + 1))
else
    echo "FALHA efeitos sonoros desligados: notificação sem som"; FAIL=$((FAIL + 1))
fi
rm -rf "$SONS"
check "× fecha a notificação" "notificação 1 fechada" "250 move 1166 80; 260 click 1016 42; 300 list" "$NOTE1"
check "botão de ação aciona o app" "ação ok da notificação 1" "250 click 1238 121; 300 list" "$NOTE2"
ACAO="$(mktemp)"
timeout 150 "$SCEN" "250 click 1238 121; 300 list" -- "$NOTE2 > $ACAO" >/dev/null 2>&1
if [ "$(cat "$ACAO")" = "ok" ]; then
    echo "ok    app recebe a ação pelo D-Bus"; PASS=$((PASS + 1))
else
    echo "FALHA app recebe a ação pelo D-Bus (recebeu: '$(cat "$ACAO")')"; FAIL=$((FAIL + 1))
fi
rm -f "$ACAO"
# Central de notificações: histórico em $XDG_RUNTIME_DIR/<socket de teste>-notificacoes.
HIST="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/${MIDE_TEST_SOCKET:-mide-test}-notificacoes"
rm -f "$HIST"
timeout 150 "$SCEN" "250 list" -- "$NOTE1" >/dev/null 2>&1
if grep -q "Download concluído" "$HIST" 2>/dev/null; then
    echo "ok    notificação vai para o histórico"; PASS=$((PASS + 1))
else
    echo "FALHA notificação vai para o histórico"; FAIL=$((FAIL + 1))
fi
# Painel do relógio (x 1078..1358): "Limpar" da central em (1321, 364).
timeout 150 "$SCEN" "300 click 1331 14; 340 click 1321 364; 380 list" -- "$NOTE1" >/dev/null 2>&1
if [ ! -s "$HIST" ]; then
    echo "ok    Limpar apaga o histórico de notificações"; PASS=$((PASS + 1))
else
    echo "FALHA Limpar apaga o histórico de notificações"; FAIL=$((FAIL + 1))
fi
# Não perturbe: o banner não aparece, mas a notificação fica no histórico.
NCFG="$(mktemp -d)"
mkdir -p "$NCFG/mide"
printf '[notifications]\ndnd = true\n' >"$NCFG/mide/config.toml"
rm -f "$HIST"
NLOG="$(XDG_CONFIG_HOME="$NCFG" timeout 150 "$SCEN" "250 list" -- "$NOTE1" 2>&1)"
if ! echo "$NLOG" | grep -q 'camada "mide-notifications"' && grep -q "Download concluído" "$HIST" 2>/dev/null; then
    echo "ok    Não perturbe silencia o banner e guarda no histórico"; PASS=$((PASS + 1))
else
    echo "FALHA Não perturbe silencia o banner e guarda no histórico"; FAIL=$((FAIL + 1))
fi
rm -rf "$NCFG" "$HIST"
SCEN="$SCEN_PLAIN"
