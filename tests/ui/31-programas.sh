# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== programas com duplo clique: .desktop, AppImage, executáveis e scripts (E8)"
# Nada roda de verdade: o scenario.sh exporta MIDE_OPEN_DRY_RUN e o mide só registra
# "executar (teste): <comando> (pasta <pasta>)". Os programas são falsos (scripts e um
# AppImage que é só o começo de um ELF). XDG_DATA_HOME próprio da seção (o registro de
# confiança dura entre os cenários) e Downloads numa pasta de teste (user-dirs.dirs na
# configuração temporária da seção).
# Arquivos em (48, 76) com um item só na pasta: o ícone em (345, 190); o diálogo de
# um script tem "Cancelar" em (420, 291), "Abrir no editor" em (539, 291) e "Executar"
# em (655, 291); o de um AppImage vindo da internet, "Abrir" em (667, 308).
# HOME da seção na mesma pasta temporária dos programas: um programa fora do disco da
# pasta pessoal (o /tmp é tmpfs) perguntaria "num pendrive ou em outro disco".
FILES="$PWD/target/debug/mide-files"
PBASE="$(mktemp -d)"
OLD_HOME="$HOME"
export XDG_DATA_HOME="$PBASE/data" HOME="$PBASE/home"
mkdir -p "$XDG_DATA_HOME" "$PBASE/Downloads" "$HOME"
printf 'XDG_DOWNLOAD_DIR="%s"\n' "$PBASE/Downloads" >"$XDG_CONFIG_HOME/user-dirs.dirs"
APPLOG="$PBASE/app.log"
REG="$XDG_DATA_HOME/mide/programas-confiaveis"

# Uma pasta com um item só (o ícone em (345, 190)).
one() { local d="$PBASE/$1"; rm -rf "$d"; mkdir -p "$d"; echo "$d"; }
fake_elf() { printf '\177ELF\002\001\001\000AI\002\000\000\000\000\000' >"$1"; }

# prog_check "nome" "regex no log do app (vários: separados por ' @@ ')" "regex que NÃO pode aparecer (ou '')" "roteiro" pasta
prog_check() {
    local name="$1" expected="$2" absent="$3" script="$4" dir="$5"
    : >"$APPLOG"
    timeout 150 "$SCEN" "$script" -- "RUST_LOG=mide=info,mide_files=info NO_COLOR=1 $FILES '$dir' >'$APPLOG' 2>&1" >/dev/null 2>&1
    local log
    log="$(sed 's/\x1b\[[0-9;]*m//g' "$APPLOG")"
    # Vários padrões separados por " @@ ": todos têm de aparecer.
    local ok=1 pattern
    while IFS= read -r pattern; do
        echo "$log" | grep -Eq "$pattern" || ok=0
    done < <(echo "$expected" | sed 's/ @@ /\n/g')
    if [ "$ok" = 1 ] && { [ -z "$absent" ] || ! echo "$log" | grep -Eq "$absent"; }; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name"; echo "      esperado: $expected${absent:+ (sem: $absent)}"
        echo "$log" | grep -aE "programs|executar|abrir" | sed 's/^/      /' | tail -4
        FAIL=$((FAIL + 1))
    fi
}
DOUBLE="400 click 345 190; 406 click 345 190"

# Atalho .desktop executável numa pasta comum (como num pendrive ou num zip extraído):
# a permissão não basta. Até confiar, mostra o nome do arquivo e pergunta; o Enter
# logo depois de o diálogo aparecer não confirma; Abrir confia e roda o comando do
# Exec (aspas, %F fora) na pasta do Path=. Depois, mostra o app e abre sem perguntar.
D="$(one atalho)"
printf '[Desktop Entry]\nType=Application\nName=Meu Editor\nIcon=accessories-text-editor\nExec="/opt/Meu Editor/editor" --novo %%F\nPath=/tmp\n' >"$D/editor.desktop"
chmod +x "$D/editor.desktop"
prog_check "atalho não confiável mostra o nome do arquivo" '' 'atalho .*/editor.desktop: "Meu Editor"' "400 list" "$D"
prog_check "atalho executável não confiável pergunta antes" 'perguntar antes de abrir .*/editor.desktop: Este atalho está em .* e ainda não foi marcado como confiável' "executar \(teste\)" "$DOUBLE; 460 list" "$D"
prog_check "Enter logo que o diálogo aparece não confirma" 'editor.desktop: Enter ignorado' "executar \(teste\)|confiado" "$DOUBLE; 412 key enter; 470 list" "$D"
prog_check "Abrir confia no atalho e roda o app do Exec" 'confiado: .*/editor.desktop @@ executar \(teste\): /opt/Meu Editor/editor --novo \(pasta /tmp\)' "" "$DOUBLE; 460 key enter; 500 list" "$D"
prog_check "atalho confiado aparece com o nome do app" 'atalho .*/editor.desktop: "Meu Editor" \(ícone accessories-text-editor\)' "" "400 list" "$D"
prog_check "lista: o atalho também mostra o app" 'atalho .*/editor.desktop: "Meu Editor"' "" "400 click 694 106; 440 list" "$D"
prog_check "duplo clique no .desktop confiado abre o app do Exec" 'executar \(teste\): /opt/Meu Editor/editor --novo \(pasta /tmp\)' "perguntar" "$DOUBLE; 460 list" "$D"
prog_check "Enter no .desktop escolhido abre o app" 'executar \(teste\): /opt/Meu Editor/editor --novo' "perguntar" "400 click 345 190; 430 key enter; 460 list" "$D"

# ELF com nome de documento ou de biblioteca abre como documento, nunca roda.
D="$(one disfarce)"
cp /bin/true "$D/foto.jpg"
chmod +x "$D/foto.jpg"
prog_check "ELF chamado foto.jpg abre como documento" "abrir \(teste\): $D/foto.jpg" "perguntar|executar" "$DOUBLE; 460 list" "$D"
D="$(one biblioteca)"
cp /bin/true "$D/libexemplo.so.1"
chmod +x "$D/libexemplo.so.1"
prog_check "biblioteca .so abre como documento" "abrir \(teste\): $D/libexemplo.so.1" "perguntar|executar" "$DOUBLE; 460 list" "$D"

# Atalho que é um FIFO: a pasta abre sem travar (não é lido).
D="$(one fifo)"
mkfifo "$D/x.desktop"
prog_check "atalho que é um FIFO não trava a pasta (abre como arquivo)" "abrir \(teste\): $D/x.desktop" "atalho .*x.desktop|perguntar" "$DOUBLE; 460 list" "$D"


# Atalho sem permissão de execução: pergunta; Abrir (Enter) marca como executável,
# lembra e roda.
D="$(one atalho-novo)"
printf '[Desktop Entry]\nType=Application\nName=Jogo\nExec=jogo\n' >"$D/jogo.desktop"
chmod 644 "$D/jogo.desktop"
prog_check ".desktop sem permissão pergunta antes" 'perguntar antes de abrir .*/jogo.desktop: Este programa veio de .* e não está marcado como executável' "executar \(teste\)" "$DOUBLE; 460 list" "$D"
prog_check "Abrir no diálogo marca, lembra e roda" 'confiado: .*/jogo.desktop @@ executar \(teste\): jogo' "" "$DOUBLE; 460 key enter; 500 list" "$D"
if [ -x "$D/jogo.desktop" ] && grep -q "jogo.desktop" "$REG" 2>/dev/null; then
    echo "ok    o atalho confiado ficou executável e no registro"; PASS=$((PASS + 1))
else
    echo "FALHA o atalho confiado ficou executável e no registro"; FAIL=$((FAIL + 1))
fi

# AppImage em Downloads (mesmo executável): pergunta; Esc cancela e nada roda.
D="$PBASE/Downloads"
fake_elf "$D/Editor.AppImage"
chmod +x "$D/Editor.AppImage"
prog_check "AppImage em Downloads pergunta; Esc cancela e nada roda" 'veio da internet \(está em Downloads\) @@ abrir .*Editor.AppImage: cancelado' "executar \(teste\)" "$DOUBLE; 460 key esc; 500 list" "$D"
prog_check "AppImage da internet: Enter não confirma" "Editor.AppImage: Enter ignorado" "executar \(teste\)|confiado" "$DOUBLE; 460 key enter; 500 list" "$D"
prog_check "AppImage confirmado (clique em Abrir) roda na pasta dele" "executar \(teste\): $D/Editor.AppImage \(pasta $D\)" "" "$DOUBLE; 460 click 667 308; 500 list" "$D"
# Confiado: não avisa mais da origem, só oferece Instalar ou Abrir (E9); Enter abre.
prog_check "AppImage confiado não avisa mais (só Instalar ou Abrir)" "perguntar antes de abrir .*Editor.AppImage: instalar ou abrir @@ executar \(teste\): $D/Editor.AppImage" "veio da internet" "$DOUBLE; 460 key enter; 500 list" "$D"
# Mudou (outra versão com o mesmo nome): a confiança acaba.
printf 'x' >>"$D/Editor.AppImage"
prog_check "AppImage trocado pergunta de novo" "perguntar antes de abrir .*Editor.AppImage" "executar \(teste\)" "$DOUBLE; 460 list" "$D"
rm -f "$D/Editor.AppImage"

# Marca do navegador (user.xdg.origin.url): o site aparece no diálogo.
D="$(one baixado)"
fake_elf "$D/ferramenta"
chmod +x "$D/ferramenta"
if python3 -c "import os,sys; os.setxattr(sys.argv[1], 'user.xdg.origin.url', b'https://exemplo.com/ferramenta')" "$D/ferramenta" 2>/dev/null; then
    prog_check "marca do navegador: o diálogo diz o site" 'Este programa veio de exemplo.com; abra só se confiar' "executar \(teste\)" "$DOUBLE; 460 list" "$D"
else
    echo "ok    (pulado: o sistema de arquivos de teste não aceita xattr de usuário)"; PASS=$((PASS + 1))
fi

# Executável sem permissão: Abrir dá a permissão (chmod +x) e roda.
D="$(one binario)"
fake_elf "$D/programa"
chmod 644 "$D/programa"
prog_check "executável sem permissão: Abrir marca e roda" "executar \(teste\): $D/programa \(pasta $D\)" "" "$DOUBLE; 460 key enter; 500 list" "$D"
if [ -x "$D/programa" ]; then
    echo "ok    o executável confirmado ganhou a permissão de execução"; PASS=$((PASS + 1))
else
    echo "FALHA o executável confirmado ganhou a permissão de execução"; FAIL=$((FAIL + 1))
fi

# Script: sempre pergunta Executar ou Abrir no editor.
D="$(one script)"
printf '#!/bin/sh\necho oi\n' >"$D/ola.sh"
chmod +x "$D/ola.sh"
prog_check "script: Enter não executa" "ola.sh: Enter ignorado" "executar \(teste\)" "$DOUBLE; 460 key enter; 500 list" "$D"
prog_check "script: Executar roda na pasta dele" "perguntar antes de abrir .*ola.sh: script @@ executar \(teste\): $D/ola.sh \(pasta $D\)" "" "$DOUBLE; 460 click 655 291; 500 list" "$D"
prog_check "script: Abrir no editor não executa" "abrir $D/ola.sh no editor @@ abrir \(teste\): $D/ola.sh" "executar \(teste\)" "$DOUBLE; 460 click 539 291; 500 list" "$D"
prog_check "script: Cancelar não faz nada" "abrir $D/ola.sh: cancelado" "executar \(teste\)|abrir \(teste\)" "$DOUBLE; 460 click 420 291; 500 list" "$D"
# Documento comum continua abrindo no app padrão, sem diálogo.
D="$(one documento)"
echo oi >"$D/notas.txt"
prog_check "documento comum abre no app padrão, sem perguntar" "abrir \(teste\): $D/notas.txt" "perguntar|executar" "$DOUBLE; 460 list" "$D"

# Área de trabalho: o atalho pergunta na primeira vez; confiado, aparece com o nome do
# app e abre com duplo clique (o ícone na coluna da direita, em (1298, 76)).
DESK="$(one desk)"
printf '[Desktop Entry]\nType=Application\nName=Planilhas\nExec=planilhas --nova\n' >"$DESK/planilhas.desktop"
chmod +x "$DESK/planilhas.desktop"
export MIDE_DESKTOP_DIR="$DESK"
DLOG="$(timeout 150 "$SCEN" "300 list" 2>&1)"
if echo "$DLOG" | grep -q "autoteste: camada" && ! echo "$DLOG" | grep -Eq 'atalho .*/planilhas.desktop: "Planilhas"'; then
    echo "ok    área de trabalho: atalho não confiável sem o nome do app"; PASS=$((PASS + 1))
else
    echo "FALHA área de trabalho: atalho não confiável sem o nome do app"; FAIL=$((FAIL + 1))
fi
check "área de trabalho: atalho não confiável pergunta; Abrir confia e roda" "confiado: .*/planilhas.desktop" "300 click 1298 76; 306 click 1298 76; 360 key enter; 400 list"
check "área de trabalho: atalho com o nome do app" 'atalho .*/planilhas.desktop: "Planilhas"' "300 list"
check "área de trabalho: duplo clique no atalho abre o app" "executar \(teste\): planilhas --nova \(pasta $HOME\)" "300 click 1298 76; 306 click 1298 76; 340 list"
DESK="$(one desk2)"
fake_elf "$DESK/App.AppImage"
chmod 644 "$DESK/App.AppImage"
export MIDE_DESKTOP_DIR="$DESK"
check "área de trabalho: AppImage sem permissão pergunta antes" "perguntar antes de abrir .*App.AppImage" "300 click 1298 76; 306 click 1298 76; 340 list"
unset MIDE_DESKTOP_DIR XDG_DATA_HOME
export HOME="$OLD_HOME"
rm -f "$XDG_CONFIG_HOME/user-dirs.dirs"
rm -rf "$PBASE"
