# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== Arquivos: visualização lembrada por pasta (B21)"
# O estado (files-view.toml) fica numa pasta própria que dura a seção inteira: cada
# cenário abre o Arquivos de novo, como depois de fechar ou reiniciar. O log do app vai
# para um arquivo (os cenários mandam o dos apps para /dev/null).
# Janela de conteúdo inteiro (Tahoe) em (48, 76): botão de lista em (569, 102), de ícones em (530, 102); na lista,
# o cabeçalho "Modificado" em (555, 164).
FILES="$PWD/target/debug/mide-files"
VBASE="$(mktemp -d)"
export XDG_STATE_HOME="$VBASE/state"
VDIR="$VBASE/pastas"
mkdir -p "$VDIR/Fotos" "$VDIR/Projetos" "$VDIR/Nova"
echo a >"$VDIR/Fotos/a.jpg"
echo b >"$VDIR/Projetos/b.txt"
APPLOG="$VBASE/app.log"

# view_check "nome" "regex no log do app" "roteiro" pasta
view_check() {
    local name="$1" expected="$2" script="$3" dir="$4"
    : >"$APPLOG"
    timeout 150 "$SCEN" "$script" -- "NO_COLOR=1 $FILES '$dir' >'$APPLOG' 2>&1" >/dev/null 2>&1
    if sed 's/\x1b\[[0-9;]*m//g' "$APPLOG" | grep -Eq "$expected"; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name"; echo "      esperado: $expected"
        grep -a "visualização" "$APPLOG" | sed 's/^/      /' | tail -3
        FAIL=$((FAIL + 1))
    fi
}

view_check "sem nada guardado, abre em ícones" "visualização de $VDIR/Fotos: ícones, ordem nome crescente" "400 list" "$VDIR/Fotos"
view_check "trocar para lista guarda a escolha" "visualização guardada para $VDIR/Fotos: lista" "400 click 569 102; 440 list" "$VDIR/Fotos"
view_check "abrir de novo: a pasta volta em lista" "visualização de $VDIR/Fotos: lista, ordem nome crescente" "400 list" "$VDIR/Fotos"
view_check "pasta nunca aberta usa o padrão (a última escolha: lista)" "visualização de $VDIR/Nova: lista" "400 list" "$VDIR/Nova"
# Projetos: ordena por Modificado e depois inverte (decrescente), e volta para ícones.
view_check "ordenação guardada (Modificado, decrescente)" "visualização guardada para $VDIR/Projetos: lista, ordem modificado decrescente" "400 click 555 164; 440 click 555 164; 480 list" "$VDIR/Projetos"
view_check "abrir de novo: ordenação lembrada" "visualização de $VDIR/Projetos: lista, ordem modificado decrescente" "400 list" "$VDIR/Projetos"
view_check "Projetos volta para ícones" "visualização guardada para $VDIR/Projetos: ícones, ordem modificado decrescente" "400 click 530 102; 440 list" "$VDIR/Projetos"
view_check "Fotos continua em lista (escolha da pasta)" "visualização de $VDIR/Fotos: lista, ordem nome crescente" "400 list" "$VDIR/Fotos"
view_check "pasta nova segue o novo padrão (ícones)" "visualização de $VDIR/Nova: ícones, ordem modificado decrescente" "400 list" "$VDIR/Nova"
# Pasta apagada e criada de novo com outro nome: nada quebra; a nova segue o padrão.
rm -rf "$VDIR/Fotos"
mkdir -p "$VDIR/Fotos2"
view_check "pasta apagada não quebra (a renomeada usa o padrão)" "visualização de $VDIR/Fotos2: ícones" "400 list" "$VDIR/Fotos2"
# A pasta volta (pendrive reconectado): a escolha dela não foi apagada na ausência.
mkdir -p "$VDIR/Fotos"
view_check "pasta que sumiu e voltou continua em lista" "visualização de $VDIR/Fotos: lista, ordem nome crescente" "400 list" "$VDIR/Fotos"
# Barra no fim (como vem de lançadores) é a mesma pasta.
view_check "barra no fim guarda na mesma chave" "visualização guardada para $VDIR/Fotos2: lista" "400 click 569 102; 440 list" "$VDIR/Fotos2/"
view_check "sem a barra: a escolha volta" "visualização de $VDIR/Fotos2: lista" "400 list" "$VDIR/Fotos2"
if [ -f "$XDG_STATE_HOME/mide/files-view.toml" ] && ! ls "$XDG_STATE_HOME/mide/" | grep -q tmp; then
    echo "ok    estado em XDG_STATE_HOME/mide/files-view.toml, sem temporários"; PASS=$((PASS + 1))
else
    echo "FALHA estado em XDG_STATE_HOME/mide/files-view.toml, sem temporários"; FAIL=$((FAIL + 1))
fi
unset XDG_STATE_HOME
rm -rf "$VBASE"
