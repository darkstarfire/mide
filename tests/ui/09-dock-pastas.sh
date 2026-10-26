# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
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
# Painel de Downloads (320x177) acima do ícone: linhas em y=563 (projeto) e 597
# (foto.jpg). O menu de um arquivo abre por cima da lista: clicando em (756, 563),
# "Copiar caminho" fica em (821, 662); em (756, 597), "Mover para a Lixeira" fica
# em (821, 731). Menu do ícone da Lixeira: "Esvaziar Lixeira…" em (799, 670).
# Confirmação: "Esvaziar Lixeira" em (799, 644) e "Cancelar" em (799, 670). Lista da
# Lixeira com 2 itens: o primeiro (a.txt) em y=571; menu dele: "Restaurar" em
# (887, 618), "Apagar permanentemente" em (887, 653).
check "Downloads mostra os recentes" "pilha Downloads aberta: 3 item" "400 click 756 728; 470 list"
check "clique num download abre o item" "abrir \(teste\): .*/dl/projeto" "400 click 756 728; 440 click 756 563"
check "Abrir no Gerenciador de Arquivos" "abrir \(teste\): .*/dl$" "400 click 756 728; 440 click 756 670"
check_count "clique fora fecha o painel" 'camada "mide-dock-menu"' 0 "400 click 756 728; 440 click 100 200; 470 list"
check "menu do arquivo abre sobre a lista" "menu do arquivo projeto aberto" "400 click 756 728; 440 rclick 756 563; 470 list"
check_count "lista continua aberta com o menu do arquivo" 'camada "mide-dock-menu"' 1 "400 click 756 728; 440 rclick 756 563; 470 list"
check "menu de arquivo: Copiar caminho" "copiar \(teste\): .*/dl/projeto" "400 click 756 728; 440 rclick 756 563; 480 click 821 662"
check "menu de arquivo: Mover para a Lixeira" "movido para a lixeira: foto.jpg" "400 click 756 728; 440 rclick 756 597; 480 click 821 731"
# Depois de agir num item, a lista continua aberta (para agir em vários); Esc fecha.
check_count "lista de Downloads continua aberta depois de mover um item" 'camada "mide-dock-menu"' 1 "400 click 756 728; 440 rclick 756 597; 480 click 821 731; 540 list"
check_count "Esc fecha a lista de Downloads" 'camada "mide-dock-menu"' 0 "400 click 756 728; 440 key esc; 480 list"
# Copiar caminho chega de verdade a outro app: o editor de texto (aberto depois da
# cópia, para o ícone dele não mexer na dock antes dos cliques) cola
# com Ctrl+V e salva. O texto fica com o compositor.
CPY="$(mktemp)"
# Os testes de cima levaram itens para a Lixeira: com menos itens, a lista e o menu
# ficam em outro lugar. A pasta volta aos 3 itens ("projeto" o mais novo).
mkdir -p "$PLACES/dl/projeto"
echo x > "$PLACES/dl/antigo.txt"
echo x > "$PLACES/dl/foto.jpg"
touch -d "3 hours ago" "$PLACES/dl/antigo.txt"
touch -d "1 hour ago" "$PLACES/dl/foto.jpg"
touch "$PLACES/dl/projeto"
# O editor aparece por volta do quadro 900; sob carga, mais tarde (folga até o 1200).
# O roteiro conta quadros e o cenário espera por tempo: sob a carga da bateria saem
# menos quadros por segundo, então a espera e o limite vão com sobra.
SCENARIO_WAIT=45 SCENARIO_TIMEOUT=75 timeout 150 "$SCEN" "400 click 756 728; 440 rclick 756 563; 480 click 821 662; 1200 click 400 300; 1220 combo ctrl+v; 1250 combo ctrl+s; 1300 list" -- "sleep 12; $PWD/target/debug/mide-code '$CPY'" >/dev/null 2>&1
if grep -q "/dl/projeto" "$CPY"; then
    echo "ok    Copiar caminho da dock cola em outro app"; PASS=$((PASS + 1))
else
    echo "FALHA Copiar caminho da dock cola em outro app (colado: '$(cat "$CPY")')"; FAIL=$((FAIL + 1))
fi
rm -f "$CPY"
OLD_DATA="${XDG_DATA_HOME-}"
export XDG_DATA_HOME="$PLACES/data"
fill_trash
check "clique na Lixeira mostra os itens apagados" "lista da Lixeira aberta: 2 item" "400 click 799 728; 440 list"
check "item da Lixeira: Restaurar" "restaurado da lixeira: .*/orig/a.txt" "400 click 799 728; 440 rclick 799 571; 480 click 887 618; 520 list"
fill_trash
check_count "lista da Lixeira continua aberta depois de restaurar" 'camada "mide-dock-menu"' 1 "400 click 799 728; 440 rclick 799 571; 480 click 887 618; 540 list"
fill_trash
check "item da Lixeira: Apagar permanentemente" "apagado permanentemente: a.txt" "400 click 799 728; 440 rclick 799 571; 480 click 887 653; 520 list"
fill_trash
check "Lixeira: esvaziar com confirmação" "lixeira esvaziada: 2 item" "400 rclick 799 728; 440 click 799 670; 480 click 799 644; 520 list"
if [ -z "$(ls -A "$PLACES/data/Trash/files")" ]; then
    echo "ok    Lixeira: arquivos apagados de verdade"; PASS=$((PASS + 1))
else
    echo "FALHA Lixeira: arquivos continuam lá"; FAIL=$((FAIL + 1))
fi
fill_trash
timeout 150 "$SCEN" "400 rclick 799 728; 440 click 799 670; 480 click 799 670; 520 list" -- >/dev/null 2>&1
if [ "$(ls "$PLACES/data/Trash/files" | wc -l)" = 2 ]; then
    echo "ok    Lixeira: cancelar não apaga"; PASS=$((PASS + 1))
else
    echo "FALHA Lixeira: cancelar apagou arquivos"; FAIL=$((FAIL + 1))
fi
if [ -n "$OLD_DATA" ]; then export XDG_DATA_HOME="$OLD_DATA"; else unset XDG_DATA_HOME; fi
rm -f "$TESTCFG/user-dirs.dirs"
rm -rf "$PLACES"
