# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== dock"
check "clique no ícone abre o app" "nova janela criada" "400 click 611 728"
# App em foco: o clique minimiza (como o "minimize or previews" do dash-to-dock); outro
# clique traz de volta, sem abrir outra janela.
check "clique no ícone do app em foco minimiza" "em foco: minimizado pela dock" "400 click 611 728; 480 list" "$TERMINAL"
check_count "clique de novo restaura, sem abrir outro" "janela \"Terminal\"" 1 "400 click 611 728; 460 click 611 728; 540 list" "$TERMINAL"
check "app em foco com duas janelas: o clique mostra as prévias" "em foco com 2 janelas: prévias" "400 click 611 728; 480 list" "$TERMINAL" "$TERMINAL"
# A instância nova (aberta pela dock, sem --title) tem o nome do shell no título.
check_count "ctrl+clique abre nova instância" 'janela "[^"]+"' 2 "390 ctrl on; 400 click 611 728; 480 list" "$TERMINAL"
check "clique no ícone restaura minimizada" "janela restaurada" "400 combo super+h; 440 click 654 728" "$CALCULATOR"
check "Launchpad na dock abre o launcher" 'camada "mide-launcher"' "400 click 568 728; 470 list"
check "abrir pela dock sai do ícone (lâmpada)" "abrindo pela dock: mide-calculator" "400 click 654 728; 700 list"
check "Configurações fixadas na dock abrem" 'janela "Configurações do Sistema"' "400 click 697 728; 560 list"
check "menu do app: Ocultar" "janela minimizada" "400 rclick 611 728; 440 click 611 644" "$TERMINAL"
check "menu do app: Encerrar" "janela fechada" "400 rclick 611 728; 440 click 611 670" "$TERMINAL"
check "menu do app lista as janelas e foca a escolhida" "controle: janela [0-9]+ focada" "400 rclick 611 728; 440 click 611 513; 480 list" "$TERMINAL" "$TERMINAL"
check "prévias ao parar o mouse no ícone" "prévias: 2 janela" "400 move 623 728; 405 move 611 728; 470 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
check "clique na prévia escolhe a janela" "prévia: janela [0-9]+ escolhida" "400 move 623 728; 405 move 611 728; 470 move 520 598; 480 click 503 588; 520 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
# Com o Launchpad aberto, escolher uma janela nas prévias fecha o launcher.
check_count "prévia com o launcher aberto: escolher fecha o launcher" 'camada "mide-launcher"' 0 "400 combo ctrl+shift+space; 460 move 623 728; 465 move 611 728; 530 move 520 598; 540 click 503 588; 600 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
check "× da prévia fecha a janela" "prévia: fechar janela" "400 move 623 728; 405 move 611 728; 470 move 520 598; 475 move 600 533; 480 click 600 533; 540 list" "$TERMINAL" "$CALCULATOR" "$TERMINAL"
check "clique direito abre o menu do app" "menu de Terminal aberto" "400 rclick 611 728; 470 list" "$TERMINAL"
check "clique no ícone durante a animação de minimizar restaura" "janela restaurada" "400 combo super+h; 420 click 654 728" "$CALCULATOR"

# Tema de ícones do mide (fork do WhiteSur): instalado numa pasta de dados de teste a
# partir do clone (o do mide-update ou o do lado deste projeto); sem clone, pulado.
ICONSRC=""
for d in "${XDG_CACHE_HOME:-$HOME/.cache}/mide/mide-icons" "$PWD/../mide-icons"; do
    if [ -x "$d/install.sh" ]; then ICONSRC="$d"; break; fi
done
if [ -n "$ICONSRC" ]; then
    IDATA="$(mktemp -d)"
    (cd "$ICONSRC" && ./install.sh -n mide -d "$IDATA/icons" >/dev/null 2>&1)
    XDG_DATA_HOME="$IDATA" RUST_LOG="mide=debug,mide_dock=info" check "dock: ícones dos apps do mide vêm do tema do mide" "ícone mide-terminal: tema do mide" "400 list"
    rm -rf "$IDATA"
else
    echo "pulado: dock com o tema do mide (sem o clone do mide-icons)"
fi
