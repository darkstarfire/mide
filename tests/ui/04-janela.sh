# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
# Terminal de conteúdo inteiro (Tahoe): a barra e as luzes são do app, desenhadas pelo
# mide-ui; luzes em (69, 95), (89, 95) e (109, 95) com a janela em (48, 76).
echo "== janela de conteúdo inteiro (Terminal)"
check "maximizar pelo verde (abaixo da barra)" "$FOOT posição=\(0, 28\) visível=1366x673" "400 click 109 95; 450 list" "$TERMINAL"
check "restaurar pelo verde" "$FOOT posição=\(48, 76\) visível=700x500" "400 click 109 95; 450 click 61 47; 500 list" "$TERMINAL"
check "duplo clique maximiza" "$FOOT posição=\(0, 28\)" "400 click 400 95; 404 click 400 95; 450 list" "$TERMINAL"
check "arrastar pela barra" "$FOOT posição=\(148, 176\)" "400 press 400 95; 405 move 450 145; 410 move 500 195; 415 release 500 195; 430 list" "$TERMINAL"
check "redimensionar pelo canto" "$FOOT posição=\(48, 76\) visível=650x450" "400 press 745 573; 405 move 720 548; 410 move 695 523; 415 release 695 523; 450 list" "$TERMINAL"
check "redimensionar pela esquerda" "$FOOT posição=\(88, 76\) visível=660x500" "400 press 50 330; 405 move 70 330; 410 move 90 330; 415 release 90 330; 450 list" "$TERMINAL"
check "fechar pelo vermelho" "janela fechada" "400 click 69 95" "$TERMINAL"
# O app fecha sozinho (aqui, o processo termina): a foto dele esmaece mesmo assim (A20).
SCENARIO_WAIT=12 check "app que fecha sozinho também sai com transição" "janela fechando com transição" "900 list" "sh -c '$CALCULATOR & sleep 4; kill \$!'"
check "minimizar pelo amarelo" "janela minimizada" "400 click 89 95" "$TERMINAL"
check "restaurar minimizada" "janela restaurada" "400 click 89 95; 440 restore" "$TERMINAL"
# Mostrar a mesa: clique no papel de parede afasta a janela; Esc a traz de volta.
check "clique no papel de parede mostra a mesa" "mesa à mostra: 1 janela" "400 click 1100 450; 450 list" "$TERMINAL"
check "Esc traz as janelas de volta" "$FOOT posição=\(48, 76\)" "400 click 1100 450; 440 key esc; 500 list" "$TERMINAL"

# Cabeçalho desenhado pelo próprio app (GTK): só com a gnome-calculator instalada.
if command -v gnome-calculator >/dev/null; then
    echo "== janela com cabeçalho do app (calculadora)"
    check "fechar pelo X do app" "janela fechada" "400 click 384 99" gnome-calculator
    check "arrastar pelo cabeçalho do app" "$CALC popups=0 posição=\(108, 116\)" "400 press 200 98; 405 move 230 118; 410 move 260 138; 415 release 260 138; 450 list" gnome-calculator
    check "maximizar com duplo clique no cabeçalho do app" "$CALC popups=0 posição=\(0, 28\) visível=1366x673" "400 click 300 99; 405 click 300 99; 460 list" gnome-calculator
    check "abrir menu do app" "$CALC popups=1" "400 click 345 99; 440 list" gnome-calculator
    check "fechar menu clicando fora" "$CALC popups=0" "400 click 345 99; 430 click 700 400; 460 list" gnome-calculator
    check "redimensionar pela borda" "$CALC .*visível=400x509" "400 press 405 330; 405 move 425 330; 410 move 445 330; 415 release 445 330; 460 list" gnome-calculator
else
    echo "pulo  janela com cabeçalho do app (gnome-calculator ausente)"
fi
