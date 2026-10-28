# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== atalhos de janela (como no GNOME)"
# Dois Terminais: a de cima (80, 108) tem o foco. Alt+Tab escolhe a outra ao soltar o Alt.
check "Alt+Tab foca a outra janela" "alternador: janela [0-9]+ escolhida" "500 combo alt+tab; 560 list" "$TERMINAL" "$TERMINAL"
check "Super+← põe a janela na metade esquerda" "$FOOT posição=\(0, 28\) visível=683x673" "500 combo super+left; 560 list" "$TERMINAL"
check "Super+→ põe a janela na metade direita" "$FOOT posição=\(683, 28\) visível=683x673" "500 combo super+right; 560 list" "$TERMINAL"
check "Super+↑ maximiza" "$FOOT posição=\(0, 28\) visível=1366x673" "500 combo super+up; 560 list" "$TERMINAL"
check "Super+↓ restaura" "$FOOT posição=\(48, 76\) visível=700x500" "500 combo super+up; 560 combo super+down; 640 list" "$TERMINAL"
check "Super+H minimiza" "janela minimizada" "500 combo super+h; 560 list" "$TERMINAL"
# Teclas multimídia (A23): do mide, com qualquer app em foco (nos testes, os comandos
# só vão para o log).
check "tecla de volume aumenta 5%" "sistema \\(teste\\): wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%\\+" "500 key volumeup; 540 list" "$TERMINAL"
check "tecla de mudo" "sistema \\(teste\\): wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle" "500 key mute; 540 list" "$TERMINAL"
check "tecla de brilho" "sistema \\(teste\\): brightnessctl -c backlight -n 1% set 5%-" "500 key brightnessdown; 540 list" "$TERMINAL"
check "tecla tocar/pausar vai ao player (MPRIS)" "sistema \\(teste\\): mpris PlayPause" "500 key playpause; 540 list" "$TERMINAL"
check "Alt+F4 fecha a janela" "fechar janela pedido pelo atalho" "500 combo alt+f4; 560 list" "$TERMINAL"
# Repetição de tecla segurada: 600 ms de espera por padrão (pedido do usuário); a
# escolha das Configurações ([keyboard] repeat_delay) vale ao recarregar.
check "repetição de tecla: espera de 600 ms por padrão" "teclado: repetição depois de 600 ms, 25 por segundo" "300 list"
mkdir -p "$XDG_CONFIG_HOME/mide"
printf '[keyboard]\nrepeat_delay = 800\n' >> "$XDG_CONFIG_HOME/mide/config.toml"
check "repetição de tecla: espera escolhida vale" "teclado: repetição depois de 800 ms" "300 list"
sed -i '/repeat_delay = 800/d' "$XDG_CONFIG_HOME/mide/config.toml"
