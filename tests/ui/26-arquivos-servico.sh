# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== Arquivos como gerenciador de arquivos padrão: FileManager1 (B25)"
# O mide-portal pede o nome org.freedesktop.FileManager1 só na sessão do mide; aqui, com
# MIDE_PORTAL_FORCE=1 num barramento D-Bus próprio (scenario-dbus.sh). O "app" do cenário
# faz o pedido como um navegador ("Mostrar na pasta"). O Arquivos aberto pelo serviço
# escreve no log do compositor (mide.log): "selecionado ...", "informações de ...".
FDIR="$(mktemp -d)"
mkdir -p "$FDIR/Fotos de férias" "$FDIR/Outra" "$FDIR/Muitos"
echo f >"$FDIR/Fotos de férias/praia 1.jpg"
echo f >"$FDIR/Fotos de férias/praia 2.jpg"
echo o >"$FDIR/Outra/nota.txt"
for i in $(seq -w 1 80); do echo "$i" >"$FDIR/Muitos/arquivo $i.txt"; done
# Nome em Latin-1 (não é UTF-8): "é.txt" com o byte E9.
mkdir -p "$FDIR/Latin" && echo l >"$FDIR/Latin/$(printf '\351').txt"
FOTOS="file://$FDIR/Fotos%20de%20f%C3%A9rias"
OUT="${SCENARIO_OUT:-/tmp/mide-scenario}"
APPLOG="$FDIR/app.log"

# fm "Método" "lista de endereços no formato do gdbus": comando do app do cenário.
fm() {
    # Espera o mide-portal pegar o nome (com a máquina carregada ele demora).
    local wait_name="for i in \$(seq 50); do gdbus call --session -d org.freedesktop.DBus -o /org/freedesktop/DBus -m org.freedesktop.DBus.GetNameOwner org.freedesktop.FileManager1 >/dev/null 2>&1 && break; sleep 0.2; done"
    echo "$wait_name; gdbus call --session -d org.freedesktop.FileManager1 -o /org/freedesktop/FileManager1 -m org.freedesktop.FileManager1.$1 \"$2\" '' >'$APPLOG' 2>&1"
}

# fm_check "nome" "regex no mide.log" "roteiro" "app" [regex que NÃO pode aparecer]
fm_check() {
    local name="$1" expected="$2" script="$3" app="$4" absent="${5:-}"
    : >"$APPLOG"
    MIDE_PORTAL_FORCE=1 timeout 150 tests/scenario-dbus.sh "$script" -- "$app" >/dev/null 2>&1
    local log
    log="$(sed 's/\x1b\[[0-9;]*m//g' "$OUT/mide.log")"
    if echo "$log" | grep -Eq "$expected" && { [ -z "$absent" ] || ! echo "$log" | grep -Eq "$absent"; }; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name"; echo "      esperado: $expected ${absent:+/ sem: $absent}"
        echo "$log" | grep -E "filemanager1|mide_files|autoteste: janela" | sed 's/^/      /' | tail -4
        sed 's/^/      app: /' "$APPLOG" | tail -2
        FAIL=$((FAIL + 1))
    fi
}

fm_check "ShowItems abre a pasta do item com ele escolhido" \
    "selecionado $FDIR/Fotos de férias/praia 1.jpg" "400 list" "$(fm ShowItems "['$FOTOS/praia%201.jpg']")"
fm_check "ShowItems: janela do Arquivos na pasta do item" \
    'janela "Fotos de férias"' "400 list" "$(fm ShowItems "['$FOTOS/praia%201.jpg']")"
fm_check "ShowItems com dois itens da mesma pasta: uma janela, os dois escolhidos" \
    "selecionado $FDIR/Fotos de férias/praia 2.jpg" "400 list" "$(fm ShowItems "['$FOTOS/praia%201.jpg', '$FOTOS/praia%202.jpg']")"
check_items_windows() {
    : >"$APPLOG"
    local got
    got="$(MIDE_PORTAL_FORCE=1 timeout 150 tests/scenario-dbus.sh "400 list" -- "$(fm ShowItems "['$FOTOS/praia%201.jpg', 'file://$FDIR/Outra/nota.txt']")" 2>&1 | grep -c "autoteste: janela")"
    if [ "$got" = 2 ]; then
        echo "ok    ShowItems de duas pastas: uma janela por pasta"; PASS=$((PASS + 1))
    else
        echo "FALHA ShowItems de duas pastas: uma janela por pasta (janelas: $got)"; FAIL=$((FAIL + 1))
    fi
}
check_items_windows
fm_check "ShowFolders abre a pasta, sem escolher nada" \
    "visualização de $FDIR/Outra:" "400 list" "$(fm ShowFolders "['file://$FDIR/Outra']")" "selecionado"
fm_check "ShowFolders com trash:/// abre a Lixeira" \
    'janela "Lixeira"' "400 list" "$(fm ShowFolders "['trash:///']")"
fm_check "ShowItemProperties mostra as informações do item" \
    "informações de $FDIR/Outra/nota.txt" "400 list" "$(fm ShowItemProperties "['file://$FDIR/Outra/nota.txt']")"
fm_check "endereço que não é de arquivo é recusado com aviso" \
    "endereço que o Arquivos não abre: sftp://servidor/x" "400 list" "$(fm ShowItems "['sftp://servidor/x']")" 'janela "'
fm_check "item no fim de uma pasta longa: a lista rola até ele" \
    "rolagem até o item pedido: 0 → [1-9]" "400 list" "$(fm ShowItems "['file://$FDIR/Muitos/arquivo%2080.txt']")"
fm_check "ShowItems com nome que não é UTF-8: o Arquivos abre com ele escolhido" \
    "selecionado $FDIR/Latin/.+\\.txt" "400 list" "$(fm ShowItems "['file://$FDIR/Latin/%E9.txt']")" "panicked"

# Sem MIDE_PORTAL_FORCE (fora da sessão do mide), o nome não é pedido: o
# "Mostrar na pasta" do desktop de baixo (Cinnamon) não muda.
: >"$APPLOG"
timeout 150 tests/scenario-dbus.sh "400 list" -- \
    "sleep 2; gdbus call --session -d org.freedesktop.DBus -o /org/freedesktop/DBus -m org.freedesktop.DBus.GetNameOwner org.freedesktop.FileManager1 >'$APPLOG' 2>&1" >/dev/null 2>&1
if grep -q "NameHasNoOwner" "$APPLOG" && ! grep -aq "mide-portal iniciado" "$OUT/mide.log"; then
    echo "ok    fora da sessão do mide, ninguém do mide pede o FileManager1"; PASS=$((PASS + 1))
else
    echo "FALHA fora da sessão do mide, ninguém do mide pede o FileManager1"; sed 's/^/      /' "$APPLOG"; FAIL=$((FAIL + 1))
fi

# Obter informações pelo teclado (Ctrl+I) no item escolhido ao abrir.
: >"$APPLOG"
timeout 150 tests/scenario.sh "400 combo ctrl+i; 440 list" -- \
    "NO_COLOR=1 $PWD/target/debug/mide-files --select '$FDIR/Outra/nota.txt' >'$APPLOG' 2>&1" >/dev/null 2>&1
if grep -q "informações de $FDIR/Outra/nota.txt" "$APPLOG"; then
    echo "ok    Ctrl+I mostra as informações do item escolhido"; PASS=$((PASS + 1))
else
    echo "FALHA Ctrl+I mostra as informações do item escolhido"; tail -3 "$APPLOG" | sed 's/^/      /'; FAIL=$((FAIL + 1))
fi
rm -rf "$FDIR"
