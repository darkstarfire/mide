# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== bloqueio de tela (mide-lock)"
# Senha só dos testes (vale apenas nas compilações de desenvolvimento).
export MIDE_LOCK_TEST_PASSWORD=teste
SCENARIO_WAIT=16 check "Super+L bloqueia a tela" "autoteste: tela bloqueada \(1 tela" "430 combo super+l; 650 list" "$TERMINAL"
SCENARIO_WAIT=16 check "senha errada mantém bloqueada" "autoteste: tela bloqueada" "430 combo super+l; 620 type errada; 622 key enter; 760 list" "$TERMINAL"
SCENARIO_WAIT=16 check "senha certa desbloqueia" "autoteste: tela livre, desbloqueios: 1" "430 combo super+l; 620 type teste; 622 key enter; 760 list" "$TERMINAL"
unset MIDE_LOCK_TEST_PASSWORD
# Dois monitores: uma tela de bloqueio em cada.
MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT=16 check "bloqueio cobre os dois monitores" "autoteste: tela bloqueada \(2 tela" "400 combo super+l; 650 list"
# Com dois monitores, a resposta da senha não pode se perder na cópia da tela de
# bloqueio do outro monitor (antes, a senha certa às vezes não desbloqueava).
MIDE_LOCK_TEST_PASSWORD=teste MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCENARIO_WAIT=16 check "senha certa desbloqueia com dois monitores" "autoteste: tela livre, desbloqueios: 1" "400 combo super+l; 620 type teste; 622 key enter; 760 list"
# Botão de ligar: o mide pergunta antes de desligar (o logind não desliga direto); com a
# tela bloqueada, não abre nada por cima dela.
check "botão de ligar pergunta antes de desligar" 'camada "mide-bar-menu"' "400 key power; 460 list"
MIDE_LOCK_TEST_PASSWORD=teste SCENARIO_WAIT=16 check_count "botão de ligar com a tela bloqueada não abre a confirmação" 'camada "mide-bar-menu"' 0 "400 combo super+l; 600 key power; 680 list"
# Tempo sem uso (contado em segundos nos testes): apaga e acende ao mexer.
IDLECFG="$(mktemp -d)"
mkdir -p "$IDLECFG/mide"
printf '[lock]\nidle_minutes = 0\nblank_minutes = 3\n' > "$IDLECFG/mide/config.toml"
OLD_IDLE_CFG="$XDG_CONFIG_HOME"
export XDG_CONFIG_HOME="$IDLECFG" MIDE_LOCK_IDLE=1 MIDE_IDLE_UNIT=seconds
check "tela apaga depois do tempo sem uso" "autoteste: tela apagada" "200 move 500 400; 600 list"
check_count "mexer o mouse acende a tela" "autoteste: tela apagada" 0 "200 move 500 400; 600 move 520 420; 610 list"
printf '[lock]\nidle_minutes = 3\nblank_minutes = 0\n' > "$IDLECFG/mide/config.toml"
SCENARIO_WAIT=16 check "bloqueia sozinho depois do tempo sem uso" "autoteste: tela bloqueada" "200 move 500 400; 700 list"
# Energia: suspender depois do tempo sem uso (simulado: só registra o pedido ao logind)
# e o botão de ligar configurado para suspender.
printf '[lock]\nidle_minutes = 0\nblank_minutes = 0\n[power]\nsuspend_minutes = 3\n' > "$IDLECFG/mide/config.toml"
check "Energia: suspende depois do tempo sem uso" "sistema \(teste\): logind Suspend" "200 move 500 400; 600 list"
printf '[lock]\nidle_minutes = 0\nblank_minutes = 0\n[power]\nbutton = "suspend"\n' > "$IDLECFG/mide/config.toml"
check "Energia: botão de ligar configurado para suspender" "sistema \(teste\): logind Suspend" "400 key power; 460 list"
export XDG_CONFIG_HOME="$OLD_IDLE_CFG"
unset MIDE_LOCK_IDLE MIDE_IDLE_UNIT
rm -rf "$IDLECFG"
