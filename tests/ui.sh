#!/usr/bin/env bash
# Bateria de regressão da interface: janelas, controles, popups, barra e dock.
#
# Usa tests/scenario.sh (autoteste por MIDE_SELFTEST) e confere o log de cada
# cenário. Precisa de sessão gráfica, foot e gnome-calculator.
#
# Coordenadas (saída 800x600, barra superior de 28 px):
# - A primeira janela nasce em (48, 76) e a segunda em (80, 108).
# - Barra do mide numa janela em (48, 76): moldura em y=46, botões no centro
#   y=61: fechar x=63, minimizar x=83, maximizar x=103.
# - Calculadora (378 px, cabeçalho próprio) em (48, 76): X em (402, 99),
#   maximizar (364, 99), minimizar (317, 99), menu (289, 99).
# - Maximizada: área útil começa em y=28.
# - Dock padrão (ícones de 40 px) em (258, 535) 285x57: Launchpad em (285, 560),
#   foot em (328, 560), calculadora em (371, 560), Configurações em (414, 560),
#   Downloads em (473, 560) e Lixeira em (516, 560).
#   Com separador (calculadora aberta e não fixada): (261, 531) 278x61, calculadora em (400, 560).
# - Menu de contexto de um app aberto (clique direito): "Manter/Remover da Dock" em
#   y=441, "Ocultar" em y=476 e "Encerrar" em y=502. App fechado: "Remover" em y=502.
# - Barra superior: marca em (25, 14); menu: "Sobre este computador" em (100, 49).
set -u

cd "$(dirname "$0")/.."
SCEN=tests/scenario.sh
export SCENARIO_WAIT="${SCENARIO_WAIT:-10}"
PASS=0
FAIL=0

# Trava: a bateria não pode alterar a configuração real do usuário.
REAL_CFG="${XDG_CONFIG_HOME:-$HOME/.config}/mide"
cfg_fingerprint() { (cd "$REAL_CFG" 2>/dev/null && ls -A | sort && cat ./* 2>/dev/null) | md5sum; }
CFG_BEFORE="$(cfg_fingerprint)"

# Todas as sessões de teste usam uma configuração temporária e vazia: nada que
# aconteça numa janela de teste (nem um clique de quem estiver usando o PC)
# grava na configuração real.
TESTCFG="$(mktemp -d)"
export XDG_CONFIG_HOME="$TESTCFG"
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
    got="$(timeout 150 "$SCEN" "$script" -- "$@" 2>&1 | grep -E "autoteste: (janela|camada)" | grep -Ec "$pattern")"
    if [ "$got" = "$want" ]; then
        echo "ok    $name"
        PASS=$((PASS + 1))
    else
        echo "FALHA $name (esperado $want, obtido $got)"
        FAIL=$((FAIL + 1))
    fi
}

FOOT='janela "foot" popups=0'
CALC='janela "Calculadora"'
CALC_OPEN='janela "Calculadora"'

echo "== barra superior"
check "barra no topo, largura total" 'camada "mide-bar" em \(0, 0\) 800x28' "400 list" foot
check "janela nova nasce abaixo da barra" "$FOOT posição=\(48, 76\)" "400 list" foot
check "menu abre ao clicar na marca" 'camada "mide-bar-menu"' "400 click 25 14; 430 list"
check_count "menu fecha ao clicar fora" 'camada "mide-bar-menu"' 0 "400 click 25 14; 430 click 600 400; 460 list"
check "arrastar para cima para na barra" "$FOOT posição=\(48, 58\)" "400 press 400 61; 405 move 400 30; 410 move 400 5; 415 release 400 5; 440 list" foot
check "menu Editar abre pela barra" "menu aberto: Edit" "400 click 180 14; 440 list" foot
check "Janela > Zoom maximiza" "$FOOT posição=\(0, 58\) visível=800x542" "400 click 238 14; 430 click 250 73; 480 list" foot
check "app > Encerrar fecha o app" "janela fechada" "400 click 65 14; 430 click 90 82" foot
check "janela Sobre redimensiona" 'janela "Sobre este computador" .*visível=580x400' "400 click 25 14; 430 click 100 49; 520 list; 530 press 587 435; 535 move 607 455; 540 move 627 475; 545 release 627 475; 580 list"
check "relógio abre o calendário" "menu aberto: Calendar" "400 click 733 14; 440 list"
check "Wi-Fi: interruptor desliga (simulado)" "sistema \(teste\): nmcli radio wifi off" "400 click 623 14; 440 click 735 55; 470 list"
check "som: trilho muda o volume (simulado)" "sistema \(teste\): wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.5" "400 click 587 14; 440 click 600 90; 470 list"
check "Bluetooth: clique desconecta o fone (simulado)" "sistema \(teste\): bluetoothctl disconnect" "400 click 659 14; 440 click 650 93; 470 list"
check "Sobre minimizada aparece na dock e volta" "janela restaurada" "400 click 25 14; 430 click 100 49; 520 click 83 61; 600 click 443 560; 700 list"
check "Sobre este computador abre" 'janela "Sobre este computador"' "400 click 25 14; 430 click 100 49; 520 list"

echo "== launcher (ctrl+shift+space)"
# Grade em 800x600: ícones de 67 px, 6 colunas a partir de x=79, 3 linhas a partir
# de y=130; o primeiro item fica em (132, 171) e o segundo em (239, 171). Abaixo da
# grade (400, 495) é vazio.
LAUNCH="400 combo ctrl+shift+space"
check "atalho abre o launcher" 'camada "mide-launcher" em \(0, 0\) 800x600' "$LAUNCH; 460 list"
check_count "Esc fecha" 'camada "mide-launcher"' 0 "$LAUNCH; 450 key esc; 480 list"
check_count "atalho de novo fecha" 'camada "mide-launcher"' 0 "$LAUNCH; 450 combo ctrl+shift+space; 480 list"
check_count "clique no vazio fecha" 'camada "mide-launcher"' 0 "$LAUNCH; 450 click 400 495; 480 list"
check "buscar e Enter abre o app" "$CALC_OPEN" "$LAUNCH; 440 type calc; 450 key enter; 600 list"
check "buscar e clicar abre o app" "$CALC_OPEN" "$LAUNCH; 440 type calc; 450 click 132 171; 600 list"

check "roda para baixo vai para a página da direita" "página 0 → 1" "$LAUNCH; 450 move 400 300; 460 wheel down; 500 list"
check "roda para cima volta para a página da esquerda" "página 1 → 0" "$LAUNCH; 450 move 400 300; 460 wheel down; 500 wheel up; 540 list"
check "clique do meio vai para a próxima página" "página 0 → 1" "$LAUNCH; 450 mclick 400 300; 490 list"
# Pasta Utilitários (2 páginas) na página 2 da grade, em (346, 397).
check "roda dentro da pasta troca de página (carrossel)" "página da pasta 0 → 1" "$LAUNCH; 440 key pagedown; 470 click 346 397; 500 move 400 300; 510 wheel down; 550 list"
check "Configurações aparecem no launcher" 'janela "Configurações do Sistema"' "$LAUNCH; 440 type configuracoes_do_sistema; 450 key enter; 560 list"
check_count "clicar num app da dock fecha o launcher" 'camada "mide-launcher"' 0 "$LAUNCH; 460 click 371 560; 520 list"

echo "== launcher: arrastar, pastas e renomear (configuração própria)"
LCFG="$(mktemp -d)"
OLD_LCFG="$XDG_CONFIG_HOME"
export XDG_CONFIG_HOME="$LCFG"
DRAG="460 press 132 171; 465 move 150 171; 470 move 200 171; 475 move 239 171; 480 release 239 171"
check "arrastar app sobre app cria uma pasta" "pasta .*\(2 apps\)" "$LAUNCH; $DRAG; 520 list"
check "pasta fica salva" "em [0-9]+ lugar\(es\) e 4 pasta" "$LAUNCH; 460 list"
check "clicar no nome da pasta e renomear" "pasta renomeada para gravar" "$LAUNCH; 460 click 132 171; 490 click 400 192; 510 key backspace; 511 key backspace; 512 key backspace; 513 key backspace; 514 key backspace; 520 type gravar; 530 key enter; 560 list"
check "arrastar para fora tira o app da pasta" "saiu da pasta gravar" "$LAUNCH; 460 click 132 171; 490 press 346 290; 495 move 300 350; 500 move 200 450; 505 move 100 500; 510 release 100 500; 540 list"
export XDG_CONFIG_HOME="$OLD_LCFG"
rm -rf "$LCFG"

echo "== notificações (barramento D-Bus de teste)"
# O servidor do mide atende num barramento próprio (tests/scenario-dbus.sh). A
# camada fica em (436, 28): banner em x=446..790; com botões, "Abrir" em (672, 121);
# o × aparece com o mouse em cima, no canto em (450, 42).
NOTE1="notify-send -a Calculadora 'Download concluído' 'Arquivo pronto'"
NOTE2="notify-send -a Terminal -A ok=Abrir -A no=Ignorar 'Atualizações' 'Há 12 atualizações'"
SCEN_PLAIN="$SCEN"
SCEN=tests/scenario-dbus.sh
check "notificação aparece abaixo da barra, à direita" 'camada "mide-notifications" em \(436, 28\) 364x94' "250 list" "$NOTE1"
check "notificação some sozinha depois de 5 s" "notificação 1 expirou" "700 list" "$NOTE1"
check "× fecha a notificação" "notificação 1 fechada" "250 move 600 80; 260 click 450 42; 300 list" "$NOTE1"
check "botão de ação aciona o app" "ação ok da notificação 1" "250 click 672 121; 300 list" "$NOTE2"
ACAO="$(mktemp)"
timeout 150 "$SCEN" "250 click 672 121; 300 list" -- "$NOTE2 > $ACAO" >/dev/null 2>&1
if [ "$(cat "$ACAO")" = "ok" ]; then
    echo "ok    app recebe a ação pelo D-Bus"; PASS=$((PASS + 1))
else
    echo "FALHA app recebe a ação pelo D-Bus (recebeu: '$(cat "$ACAO")')"; FAIL=$((FAIL + 1))
fi
rm -f "$ACAO"
SCEN="$SCEN_PLAIN"

echo "== janela com barra do mide (foot)"
check "maximizar pelo verde (abaixo da barra)" "$FOOT posição=\(0, 58\) visível=800x542" "400 click 103 61; 450 list" foot
check "restaurar pelo verde" "$FOOT posição=\(48, 76\) visível=700x500" "400 click 103 61; 450 click 55 43; 500 list" foot
check "duplo clique maximiza" "$FOOT posição=\(0, 58\)" "400 click 400 61; 404 click 400 61; 450 list" foot
check "arrastar pela barra" "$FOOT posição=\(148, 176\)" "400 press 400 61; 405 move 450 111; 410 move 500 161; 415 release 500 161; 430 list" foot
check "redimensionar pelo canto" "$FOOT posição=\(48, 76\) visível=650x450" "400 press 745 573; 405 move 720 548; 410 move 695 523; 415 release 695 523; 450 list" foot
check "redimensionar pela esquerda" "$FOOT posição=\(88, 76\) visível=660x500" "400 press 50 330; 405 move 70 330; 410 move 90 330; 415 release 90 330; 450 list" foot
check "fechar pelo vermelho" "janela fechada" "400 click 63 61" foot
check "minimizar pelo amarelo" "janela minimizada" "400 click 83 61" foot
check "restaurar minimizada" "janela restaurada" "400 click 83 61; 440 restore" foot

echo "== janela com cabeçalho do app (calculadora)"
check "fechar pelo X do app" "janela fechada" "400 click 402 99" gnome-calculator
check "arrastar pelo cabeçalho do app" "$CALC popups=0 posição=\(108, 116\)" "400 press 200 98; 405 move 230 118; 410 move 260 138; 415 release 260 138; 450 list" gnome-calculator
check "maximizar pelo botão do app" "$CALC popups=0 posição=\(0, 28\) visível=800x572" "400 click 364 99; 460 list" gnome-calculator
check "abrir menu do app" "$CALC popups=1" "400 click 289 99; 440 list" gnome-calculator
check "fechar menu clicando fora" "$CALC popups=0" "400 click 289 99; 430 click 700 400; 460 list" gnome-calculator
check "redimensionar pela borda" "$CALC .*visível=418x500" "400 press 423 330; 405 move 443 330; 410 move 463 330; 415 release 463 330; 460 list" gnome-calculator

echo "== várias janelas"
check "cascata (segunda em 80,108)" "$CALC popups=0 posição=\(80, 108\)" "400 list" foot gnome-calculator
check_last "clique traz para frente (topo = foot)" "janela \"foot\"" "400 click 700 300; 440 list" foot gnome-calculator

echo "== saída redimensionada (janela do mide maximizada no host)"
check "dock volta ao centro inferior" 'camada "mide-dock" em \(458, 735\) 285x57' "400 resize 1200 800; 420 list" foot
check "barra acompanha a largura" 'camada "mide-bar" em \(0, 0\) 1200x28' "400 resize 1200 800; 420 list" foot
check "maximizada acompanha a nova área" "$FOOT posição=\(0, 58\) visível=1200x742" "400 click 103 61; 440 resize 1200 800; 460 list" foot

echo "== dock"
check "clique no ícone abre o app" "nova janela criada" "400 click 328 560"
check_count "clique no ícone de app aberto não abre outro" "janela \"foot\"" 1 "400 click 328 560; 480 list" foot
check_count "ctrl+clique abre nova instância" "janela \"foot\"" 2 "390 ctrl on; 400 click 328 560; 480 list" foot
check "clique no ícone restaura minimizada" "janela restaurada" "400 click 317 99; 440 click 371 560" gnome-calculator
check "Launchpad na dock abre o launcher" 'camada "mide-launcher"' "400 click 285 560; 470 list"
check "abrir pela dock sai do ícone (lâmpada)" "abrindo pela dock: org.gnome.calculator" "400 click 371 560; 700 list"
check "Configurações fixadas na dock abrem" 'janela "Configurações do Sistema"' "400 click 414 560; 560 list"
check "menu do app: Ocultar" "janela minimizada" "400 rclick 328 560; 440 click 328 476" foot
check "menu do app: Encerrar" "janela fechada" "400 rclick 328 560; 440 click 328 502" foot
check "menu do app lista as janelas e foca a escolhida" "controle: janela [0-9]+ focada" "400 rclick 328 560; 440 click 328 345; 480 list" foot foot
check "prévias ao parar o mouse no ícone" "prévias: 2 janela" "400 move 340 560; 405 move 328 560; 470 list" foot gnome-calculator foot
check "clique na prévia escolhe a janela" "prévia: janela [0-9]+ escolhida" "400 move 340 560; 405 move 328 560; 470 move 237 430; 480 click 220 420; 520 list" foot gnome-calculator foot
check "× da prévia fecha a janela" "prévia: fechar janela" "400 move 340 560; 405 move 328 560; 470 move 237 430; 475 move 317 365; 480 click 317 365; 540 list" foot gnome-calculator foot
check "clique direito abre o menu do app" "menu de Foot aberto" "400 rclick 328 560; 470 list" foot
check "clique no ícone durante a animação de minimizar restaura" "janela restaurada" "400 click 317 99; 420 click 371 560" gnome-calculator

echo "== dock: Downloads e Lixeira (pastas temporárias)"
# Downloads de teste com 3 itens (o mais novo é "projeto") e uma Lixeira de teste.
# Nada aqui toca as pastas reais: o abrir só vai para o log (MIDE_OPEN_DRY_RUN).
PLACES="$(mktemp -d)"
mkdir -p "$PLACES/dl/projeto"
echo x > "$PLACES/dl/antigo.txt"
echo x > "$PLACES/dl/foto.jpg"
touch -d "3 hours ago" "$PLACES/dl/antigo.txt"
touch -d "1 hour ago" "$PLACES/dl/foto.jpg"
printf 'XDG_DOWNLOAD_DIR="%s"\n' "$PLACES/dl" > "$TESTCFG/user-dirs.dirs"
fill_trash() {
    rm -rf "$PLACES/data" "$PLACES/orig"
    mkdir -p "$PLACES/data/Trash/files" "$PLACES/data/Trash/info" "$PLACES/orig"
    for f in a.txt b.txt; do
        echo x > "$PLACES/data/Trash/files/$f"
        printf '[Trash Info]\nPath=%s\nDeletionDate=2026-01-01T10:00:00\n' "$PLACES/orig/$f" > "$PLACES/data/Trash/info/$f.trashinfo"
    done
}
# Painel de Downloads (320x177) acima do ícone: linhas em y=395 (projeto) e 429
# (foto.jpg). O menu de um arquivo abre no ponto do clique direito, por cima da
# lista: clicando em (455, 395), "Copiar caminho" fica em (520, 494); em (455, 429),
# "Mover para a Lixeira" fica em (520, 563). Menu do ícone da Lixeira: "Esvaziar
# Lixeira…" em (502, 502). Confirmação: "Esvaziar Lixeira" em (502, 476) e
# "Cancelar" em (502, 502). Lista da Lixeira com 2 itens: o primeiro (a.txt) em
# y=403; menu dele em (504, 403): "Restaurar" em (580, 450), "Apagar
# permanentemente" em (580, 485).
check "Downloads mostra os recentes" "pilha Downloads aberta: 3 item" "400 click 473 560; 470 list"
check "clique num download abre o item" "abrir \(teste\): .*/dl/projeto" "400 click 473 560; 440 click 473 395"
check "Abrir no Gerenciador de Arquivos" "abrir \(teste\): .*/dl$" "400 click 473 560; 440 click 473 502"
check_count "clique fora fecha o painel" 'camada "mide-dock-menu"' 0 "400 click 473 560; 440 click 100 200; 470 list"
check "menu do arquivo abre sobre a lista" "menu do arquivo projeto aberto" "400 click 473 560; 440 rclick 473 395; 470 list"
check_count "lista continua aberta com o menu do arquivo" 'camada "mide-dock-menu"' 1 "400 click 473 560; 440 rclick 473 395; 470 list"
check "menu de arquivo: Copiar caminho" "copiar \(teste\): .*/dl/projeto" "400 click 473 560; 440 rclick 473 395; 480 click 538 494"
check "menu de arquivo: Mover para a Lixeira" "movido para a lixeira: foto.jpg" "400 click 473 560; 440 rclick 473 429; 480 click 538 563"
OLD_DATA="${XDG_DATA_HOME-}"
export XDG_DATA_HOME="$PLACES/data"
fill_trash
check "clique na Lixeira mostra os itens apagados" "lista da Lixeira aberta: 2 item" "400 click 516 560; 440 list"
check "item da Lixeira: Restaurar" "restaurado da lixeira: .*/orig/a.txt" "400 click 516 560; 440 rclick 516 403; 480 click 604 450; 520 list"
fill_trash
check "item da Lixeira: Apagar permanentemente" "apagado permanentemente: a.txt" "400 click 516 560; 440 rclick 516 403; 480 click 604 485; 520 list"
fill_trash
check "Lixeira: esvaziar com confirmação" "lixeira esvaziada: 2 item" "400 rclick 516 560; 440 click 516 502; 480 click 516 476; 520 list"
if [ -z "$(ls -A "$PLACES/data/Trash/files")" ]; then
    echo "ok    Lixeira: arquivos apagados de verdade"; PASS=$((PASS + 1))
else
    echo "FALHA Lixeira: arquivos continuam lá"; FAIL=$((FAIL + 1))
fi
fill_trash
timeout 150 "$SCEN" "400 rclick 516 560; 440 click 516 502; 480 click 516 502; 520 list" -- >/dev/null 2>&1
if [ "$(ls "$PLACES/data/Trash/files" | wc -l)" = 2 ]; then
    echo "ok    Lixeira: cancelar não apaga"; PASS=$((PASS + 1))
else
    echo "FALHA Lixeira: cancelar apagou arquivos"; FAIL=$((FAIL + 1))
fi
if [ -n "$OLD_DATA" ]; then export XDG_DATA_HOME="$OLD_DATA"; else unset XDG_DATA_HOME; fi
rm -f "$TESTCFG/user-dirs.dirs"
rm -rf "$PLACES"

echo "== dock: fixar, desafixar e reordenar (configuração temporária)"
DOCKCFG="$(mktemp -d)"
mkdir -p "$DOCKCFG/mide"
dock_pins() { printf 'pinned = [%s]\n' "$1" > "$DOCKCFG/mide/dock.toml"; }
FOOT_ID='"org.codeberg.dnkl.foot"'
CALC_ID='"org.gnome.Calculator"'
# Atenção: "VAR=x funcao" não exporta VAR para os programas que a função chama.
# Por isso a variável é exportada só nesta seção e restaurada no fim.
OLD_XDG_CONFIG_HOME="${XDG_CONFIG_HOME-}"
export XDG_CONFIG_HOME="$DOCKCFG"
dock_pins "$FOOT_ID"
check "app aberto não fixado entra na dock" 'camada "mide-dock" em \(271, 535\) 258x57' "500 list" gnome-calculator
dock_pins "$FOOT_ID"
check "menu do app: Manter na Dock" "dock salva: org.codeberg.dnkl.foot, org.gnome.Calculator" "500 rclick 400 560; 540 click 400 445" gnome-calculator
dock_pins "$FOOT_ID, $CALC_ID"
check "menu do app: Remover da Dock" "dock salva: org.gnome.Calculator$" "300 rclick 349 560; 340 click 349 506"
dock_pins "$FOOT_ID, $CALC_ID"
check "arrastar reordena" "dock salva: org.gnome.Calculator, org.codeberg.dnkl.foot" "300 press 349 560; 305 move 360 560; 310 move 375 560; 315 move 392 560; 320 release 392 560"
if [ -n "$OLD_XDG_CONFIG_HOME" ]; then export XDG_CONFIG_HOME="$OLD_XDG_CONFIG_HOME"; else unset XDG_CONFIG_HOME; fi
rm -rf "$DOCKCFG"

echo "== Configurações do Sistema (tela 1280x800, configuração própria)"
check "menu do mide abre as Configurações" "sistema \(teste\): mide-settings" "400 click 25 14; 430 click 100 82; 460 list"
SCFG="$(mktemp -d)"
mkdir -p "$SCFG/mide"
printf '# Meu config\n[keybindings]\n"ctrl+shift+t" = "launch:foot" # terminal\n' > "$SCFG/mide/config.toml"
OLD_SCFG="$XDG_CONFIG_HOME"
export XDG_CONFIG_HOME="$SCFG" MIDE_WINIT_SIZE=1280x800
SETTINGS="$PWD/target/debug/mide-settings"
# Janela em (48, 76): miniatura "Beach" em (562, 192); interruptor de Sombras em (805, 166).
timeout 150 "$SCEN" "400 click 562 192; 460 list" -- "$SETTINGS wallpaper" >/dev/null 2>&1
if grep -q 'wallpaper = "/usr/share/backgrounds/Beach' "$SCFG/mide/config.toml" && grep -q "# terminal" "$SCFG/mide/config.toml"; then
    echo "ok    escolher papel de parede grava (e mantém os comentários)"; PASS=$((PASS + 1))
else
    echo "FALHA escolher papel de parede grava"; FAIL=$((FAIL + 1))
fi
check "compositor usa o papel de parede gravado" "papel de parede: /usr/share/backgrounds/Beach" "400 list; 420 list" "$SETTINGS wallpaper"
timeout 150 "$SCEN" "400 click 805 166; 460 list" -- "$SETTINGS effects" >/dev/null 2>&1
if grep -q "shadows = false" "$SCFG/mide/config.toml"; then
    echo "ok    desligar sombras grava no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA desligar sombras grava no config.toml"; FAIL=$((FAIL + 1))
fi
# Efeitos: "Desfoque na dock" em (805, 374) e "Desfoque no launcher" em (805, 426).
timeout 150 "$SCEN" "400 click 805 374; 440 click 805 426; 480 list" -- "$SETTINGS effects" >/dev/null 2>&1
if grep -q "dock_blur = false" "$SCFG/mide/config.toml" && grep -q "launcher_blur = false" "$SCFG/mide/config.toml"; then
    echo "ok    interruptores de desfoque gravam no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA interruptores de desfoque gravam no config.toml"; FAIL=$((FAIL + 1))
fi
# O teste anterior desligou o desfoque da dock; o vidro líquido depende dele.
sed -i '/dock_blur = false/d' "$SCFG/mide/config.toml"
# "Vidro líquido" em (805, 478): grava no config.toml e a dock troca para o vidro clássico.
check "Efeitos → Vidro líquido desligado: a dock volta ao vidro clássico" "vidro líquido na dock: desligado" "400 click 805 478; 480 list" "$SETTINGS effects"
if grep -q "liquid_glass = false" "$SCFG/mide/config.toml"; then
    echo "ok    interruptor do vidro líquido grava no config.toml"; PASS=$((PASS + 1))
else
    echo "FALHA interruptor do vidro líquido grava no config.toml"; FAIL=$((FAIL + 1))
fi
# Seção Dock: arrastar o trilho até o fim grava 64 px e a dock cresce sozinha.
check "Configurações → Dock: tamanho dos ícones muda a dock" 'camada "mide-dock" em \([0-9]+, [0-9]+\) [0-9]+x81' "400 list; 420 press 520 162; 425 move 600 162; 430 move 700 162; 435 move 790 162; 440 release 790 162; 520 list" "$SETTINGS dock"
export XDG_CONFIG_HOME="$OLD_SCFG"
unset MIDE_WINIT_SIZE
rm -rf "$SCFG"

echo "== buscador (ctrl+space)"
SEARCH="400 combo ctrl+space"
check "ctrl+space abre o buscador em tela cheia" 'camada "mide-search" em \(0, 0\) 800x600' "$SEARCH; 460 list"
check "buscador sobre vidro" 'vidro: desfoque refeito \(mide-search#0\)' "$SEARCH; 440 type calc; 480 list"
check "buscar e Enter abre o app" "nova janela criada" "$SEARCH; 440 type calc; 470 key enter; 600 list"
check_count "Esc fecha o buscador" 'camada "mide-search"' 0 "$SEARCH; 450 key esc; 490 list"
check_count "o atalho de novo fecha o buscador" 'camada "mide-search"' 0 "$SEARCH; 450 combo ctrl+space; 490 list"
check_count "clique fora do painel fecha o buscador" 'camada "mide-search"' 0 "$SEARCH; 450 click 60 560; 490 list"
check_count "abrir um app pelo buscador fecha o buscador" 'camada "mide-search"' 0 "$SEARCH; 440 type calc; 470 key enter; 600 list"

echo "== vidro em barras e menus"
check "barra superior sobre vidro" 'vidro: desfoque refeito \(mide-bar#0\)' "300 list"
check "menu da barra sobre vidro" 'vidro: desfoque refeito \(mide-bar-menu#0\)' "300 click 25 14; 360 list"
check "menu da dock sobre vidro" 'vidro: desfoque refeito \(mide-dock-menu#0\)' "300 rclick 516 560; 360 list"
GCFG="$(mktemp -d)"
mkdir -p "$GCFG/mide"
printf '[effects]\npanel_glass = false\n' >"$GCFG/mide/config.toml"
GLOG="$(XDG_CONFIG_HOME="$GCFG" timeout 150 "$SCEN" "300 click 25 14; 360 list" 2>&1)"
if ! echo "$GLOG" | grep -q 'camada "mide-bar"' || echo "$GLOG" | grep -q "vidro: desfoque refeito (mide-bar"; then
    echo "FALHA sem vidro na barra e nos menus com o efeito desligado"; FAIL=$((FAIL + 1))
else
    echo "ok    sem vidro na barra e nos menus com o efeito desligado"; PASS=$((PASS + 1))
fi
rm -rf "$GCFG"

# Arquivos (mide-files) numa pasta de exemplo recriada a cada cenário. Janela em
# (48, 76): grade com Documentos antigos (345, 190), Fotos (463, 190), Projetos
# (581, 190); na 2ª linha "por do sol.png" (345, 310), praia.jpg (463, 310) e
# relatorio.pdf (581, 310). Botão de lista em (694, 106); Lixeira na barra lateral
# em (103, 435). Na lista, as linhas começam em y=165 e descem de 38 em 38.
FILES="$PWD/target/debug/mide-files"
FDIR="$(mktemp -d)/mide-arquivos-teste"
TRASH="${SCENARIO_OUT:-/tmp/mide-scenario}/data/Trash/files"
fixture() {
    rm -rf "$FDIR"
    mkdir -p "$FDIR/Documentos antigos" "$FDIR/Fotos" "$FDIR/Projetos"
    echo oi >"$FDIR/notas.txt"
    echo x >"$FDIR/relatorio.pdf"
    echo y >"$FDIR/música.mp3"
    cp /usr/share/pixmaps/debian-logo.png "$FDIR/praia.jpg" 2>/dev/null || echo z >"$FDIR/praia.jpg"
    echo w >"$FDIR/por do sol.png"
}
# file_check "nome" "teste do shell" "roteiro": roda o Arquivos e confere o disco.
file_check() {
    local name="$1" test="$2" script="$3"
    fixture
    timeout 150 "$SCEN" "$script" -- "$FILES '$FDIR'" >/dev/null 2>&1
    if eval "$test"; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name"; FAIL=$((FAIL + 1))
    fi
}
fixture
check "Arquivos abre a pasta pedida (título = pasta)" 'janela "mide-arquivos-teste"' "400 list" "$FILES '$FDIR'"
check "duplo clique numa pasta entra nela" 'janela "Fotos"' "400 click 463 190; 406 click 463 190; 440 list" "$FILES '$FDIR'"
check "Backspace volta para a pasta de cima" 'janela "mide-arquivos-teste"' "400 click 463 190; 406 click 463 190; 440 key backspace; 470 list" "$FILES '$FDIR'"
check "voltar da barra de ferramentas" 'janela "mide-arquivos-teste"' "400 click 463 190; 406 click 463 190; 440 click 296 106; 470 list" "$FILES '$FDIR'"
check "Lixeira na barra lateral" 'janela "Lixeira"' "400 click 103 435; 440 list" "$FILES '$FDIR'"
file_check "Delete manda o item selecionado para a Lixeira" '[ ! -e "$FDIR/relatorio.pdf" ] && [ -e "$TRASH/relatorio.pdf" ]' "400 click 581 310; 430 key delete; 470 list"
file_check "menu do fundo → Nova pasta, já pedindo o nome" '[ -d "$FDIR/arquivados" ] && [ ! -e "$FDIR/Nova pasta" ]' "400 rclick 600 450; 430 click 650 468; 470 type arquivados; 490 key enter; 530 list"
file_check "Esc cancela o nome e a pasta nova fica" '[ -d "$FDIR/Nova pasta" ]' "400 rclick 600 450; 430 click 650 468; 470 type xyz; 490 key esc; 530 list"
file_check "lista + menu do item → Mover para a Lixeira" '[ ! -e "$FDIR/notas.txt" ] && [ -e "$TRASH/notas.txt" ]' "400 click 694 106; 430 rclick 500 317; 460 click 550 422; 500 list"
file_check "menu do item → Renomear" '[ -e "$FDIR/imagens" ] && [ ! -e "$FDIR/Fotos" ]' "400 rclick 463 190; 430 click 515 234; 470 key backspace; 471 key backspace; 472 key backspace; 473 key backspace; 474 key backspace; 480 type imagens; 490 key enter; 530 list"
file_check "clique fora fecha o menu sem fazer nada" '[ -e "$FDIR/relatorio.pdf" ] && [ -e "$FDIR/notas.txt" ]' "400 rclick 581 310; 430 click 700 470; 460 click 630 415; 500 list"
rm -rf "$(dirname "$FDIR")"

if [ "$(cfg_fingerprint)" != "$CFG_BEFORE" ]; then
    echo "FALHA a bateria alterou $REAL_CFG"
    FAIL=$((FAIL + 1))
else
    echo "ok    configuração real intacta"
    PASS=$((PASS + 1))
fi

echo "resultado: $PASS ok, $FAIL falha(s)"
[ "$FAIL" = 0 ]
