# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== menu Janela como o do Mac"
# Menu Janela do Arquivos (publicado por ele, num barramento de teste) em x=375; linhas:
# Minimizar y=55, Preencher 79, Centralizar 103, Metade esquerda 159, Canto inferior
# direito 337; com duas telas, "Mover para" em y=531. Área útil: (0, 28) 1366x673.
FILES_APP="$PWD/target/debug/mide-files"
SCEN=tests/scenario-dbus.sh check "Janela > Preencher ocupa a área útil" 'janela "Pasta pessoal" popups=0 posição=\(0, 28\) visível=1366x673' "500 click 375 14; 540 click 400 79; 620 list" "$FILES_APP"
SCEN=tests/scenario-dbus.sh check "Janela > Centralizar" 'janela "Pasta pessoal" popups=0 posição=\(233, 104\) visível=900x520' "500 click 375 14; 540 click 430 103; 620 list" "$FILES_APP"
SCEN=tests/scenario-dbus.sh check "Janela > Mover e redimensionar > Metade esquerda" 'janela "Pasta pessoal" popups=0 posição=\(0, 28\) visível=683x673' "500 click 375 14; 540 click 430 159; 620 list" "$FILES_APP"
SCEN=tests/scenario-dbus.sh check "Janela > Canto inferior direito" 'janela "Pasta pessoal" popups=0 posição=\(683, ' "500 click 375 14; 540 click 430 337; 620 list" "$FILES_APP"
MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080 SCEN=tests/scenario-dbus.sh check "Janela > Mover para a outra tela" 'janela "Pasta pessoal" popups=0 posição=\(1433, 77\)' "500 click 375 14; 540 click 430 531; 620 list" "$FILES_APP"
