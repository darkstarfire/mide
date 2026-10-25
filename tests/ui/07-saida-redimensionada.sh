# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== saída redimensionada (janela do mide maximizada no host)"
check "dock volta ao centro inferior" 'camada "mide-dock" em \(458, 735\) 285x57' "400 resize 1200 800; 420 list" "$TERMINAL"
check "barra acompanha a largura" 'camada "mide-bar" em \(0, 0\) 1200x28' "400 resize 1200 800; 420 list" "$TERMINAL"
check "maximizada acompanha a nova área" "$FOOT posição=\(0, 28\) visível=1200x705" "400 click 109 95; 440 resize 1200 800; 460 list" "$TERMINAL"
