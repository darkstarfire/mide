# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== instalar AppImage arrastando para Aplicativos (E9)"
# O AppImage é falso: um programa C mínimo (compilado aqui, com a marca AI do tipo 2
# no ELF) que chama um script; o script responde a --appimage-extract como o runtime de
# verdade (cria squashfs-root/ com o .desktop e o ícone) e não faz mais nada. A pasta
# de apps (MIDE_APPS_DIR), os dados (XDG_DATA_HOME: atalhos, ícones, Lixeira) e o
# registro de confiança são da seção; nada vai para o HOME real. Abrir um app só
# registra no log (MIDE_OPEN_DRY_RUN do scenario.sh).
# Coordenadas: a janela do instalador nasce em (48, 76): ícone do app em (210, 234),
# pasta em (446, 234), botões embaixo em y=400 (Cancelar/Fechar ~ (450, 400),
# Instalar/Substituir/Abrir ~ (548, 400)). Arquivos em (48, 76): o ícone em (345, 190);
# o diálogo "Instalar ou abrir" tem Instalar em (582, 307).
if ! command -v cc >/dev/null; then
    echo "ok    (pulado: sem compilador C para o AppImage falso)"; PASS=$((PASS + 1))
    return 0 2>/dev/null || exit 0
fi
INSTALL="$PWD/target/debug/mide-install"
FILES="$PWD/target/debug/mide-files"
IBASE="$(mktemp -d)"
# HOME e cache da seção: o AppImage no mesmo disco da "pasta pessoal" (senão o /tmp,
# tmpfs, seria "outro disco" e o instalador perguntaria antes), e a pasta temporária
# da extração ($XDG_CACHE_HOME/mide) fora do HOME real.
OLD_HOME="$HOME"
export XDG_DATA_HOME="$IBASE/data" MIDE_APPS_DIR="$IBASE/Aplicativos" HOME="$IBASE/home" XDG_CACHE_HOME="$IBASE/cache"
mkdir -p "$XDG_DATA_HOME" "$IBASE/src" "$IBASE/fake" "$HOME"
APPLOG="$IBASE/app.log"
DESK="$XDG_DATA_HOME/applications/mide-appimage-org.exemplo.Editor.desktop"
INSTALLED="$MIDE_APPS_DIR/Editor Bonito.AppImage"

# Runtime falso: só extrai (a versão vem de FAKE_VERSION, para testar a substituição).
cat >"$IBASE/fake/runtime.sh" <<'EOF'
#!/bin/sh
[ "$1" = --appimage-extract ] || { echo "app falso rodou: $*"; exit 0; }
mkdir -p squashfs-root
case "$2" in
'*.desktop') printf '[Desktop Entry]\nType=Application\nName=Editor Bonito\nIcon=editor\nExec=AppRun %%F\nCategories=Office;\nX-AppImage-Version=%s\n' "${FAKE_VERSION:-1.0}" >squashfs-root/org.exemplo.Editor.desktop ;;
'editor.*') printf '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" rx="14" fill="#e8453c"/></svg>' >squashfs-root/editor.svg ;;
esac
EOF
cat >"$IBASE/fake/fake.c" <<EOF
#include <unistd.h>
int main(int c, char **v) { char *a[] = {"sh", "$IBASE/fake/runtime.sh", c > 1 ? v[1] : 0, c > 2 ? v[2] : 0, 0}; execv("/bin/sh", a); return 127; }
EOF
APP="$IBASE/src/Editor-1.0-x86_64.AppImage"
cc -o "$APP" "$IBASE/fake/fake.c"
printf 'AI\002' | dd of="$APP" bs=1 seek=8 conv=notrunc 2>/dev/null

# inst_check "nome" "regex no log do app (vários: ' @@ ')" "regex que NÃO pode aparecer (ou '')" "roteiro" comando
inst_check() {
    local name="$1" expected="$2" absent="$3" script="$4" cmd="$5"
    : >"$APPLOG"
    timeout 150 "$SCEN" "$script" -- "RUST_LOG=mide=info,mide_install=info,mide_files=info NO_COLOR=1 $cmd >'$APPLOG' 2>&1" >/dev/null 2>&1
    local log ok=1 pattern
    log="$(sed 's/\x1b\[[0-9;]*m//g' "$APPLOG")"
    while IFS= read -r pattern; do
        echo "$log" | grep -Eq "$pattern" || ok=0
    done < <(echo "$expected" | sed 's/ @@ /\n/g')
    if [ "$ok" = 1 ] && { [ -z "$absent" ] || ! echo "$log" | grep -Eq "$absent"; }; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name"; echo "      esperado: $expected${absent:+ (sem: $absent)}"
        echo "$log" | grep -aE "instal|AppImage|perguntar|executar|remov" | sed 's/^/      /' | tail -4
        FAIL=$((FAIL + 1))
    fi
}
# file_check "nome" condição...
file_check() {
    local name="$1"
    shift
    if "$@"; then echo "ok    $name"; PASS=$((PASS + 1)); else echo "FALHA $name"; FAIL=$((FAIL + 1)); fi
}

# Arquivos: duplo clique num AppImage oferece Instalar ou Abrir; Instalar abre o
# instalador (no teste, só o log).
inst_check "duplo clique no AppImage oferece Instalar ou Abrir" "perguntar antes de abrir .*Editor-1.0-x86_64.AppImage: instalar ou abrir" "executar \(teste\)" \
    "400 click 345 190; 406 click 345 190; 460 list" "$FILES '$IBASE/src'"
inst_check "Instalar no diálogo abre o instalador" "instalar \(teste\): mide-install $APP" "executar \(teste\)" \
    "400 click 345 190; 406 click 345 190; 460 click 582 307; 500 list" "$FILES '$IBASE/src'"

# Instalador: lê o nome, a versão e o ícone de dentro e mostra o palco.
inst_check "instalador lê o nome, a versão e o ícone de dentro" 'pronto para instalar "Editor Bonito" \(versão 1.0; já instalado: não\) @@ ícone .*squashfs-root/editor.svg' "" \
    "460 list" "$INSTALL '$APP'"
check "instalador abre com o nome do app" 'janela "Instalar Editor Bonito"' "460 list" "$INSTALL $APP"
# Arrastar o ícone para fora da pasta não instala.
inst_check "soltar o ícone fora da pasta não instala" "ícone solto fora da pasta" 'instalado: "' \
    "440 press 210 234; 446 move 260 300; 452 move 300 330; 458 release 300 330; 500 list" "$INSTALL '$APP'"
file_check "nada instalado depois de soltar fora" test ! -e "$DESK"
# Arrastar até a pasta instala: o AppImage em Aplicativos (executável), o atalho com a
# marca e o ícone copiado.
inst_check "arrastar o ícone até a pasta instala" 'instalar "Editor Bonito" \(arrastado para a pasta\) @@ instalado: "Editor Bonito" em .*/Aplicativos/Editor Bonito.AppImage' "" \
    "440 press 210 234; 446 move 260 234; 452 move 350 234; 458 move 440 238; 464 release 446 234; 520 list" "$INSTALL '$APP'"
file_check "AppImage copiado e executável em Aplicativos" test -x "$INSTALLED"
file_check "atalho gerado com Exec, Icon e a marca" grep -q "^X-Mide-AppImage=$INSTALLED\$" "$DESK"
file_check "atalho aponta para o AppImage e o ícone copiados" sh -c "grep -qF 'Exec=\"$INSTALLED\" %F' '$DESK' && grep -q '^Icon=$XDG_DATA_HOME/mide/appimage-icons/org.exemplo.Editor.svg\$' '$DESK' && test -s '$XDG_DATA_HOME/mide/appimage-icons/org.exemplo.Editor.svg'"
# Abrir, no fim, roda o app instalado (pelo atalho).
inst_check "Abrir depois de instalar roda o app instalado" "executar \(teste\): $INSTALLED" "" \
    "440 key enter; 500 key enter; 540 list" "$INSTALL '$APP'"

# Já instalado: oferece substituir pela versão nova; o botão (Enter) substitui.
cp "$APP" "$IBASE/src/Editor-2.0-x86_64.AppImage"
printf 'nova' >>"$IBASE/src/Editor-2.0-x86_64.AppImage"
export FAKE_VERSION=2.0
inst_check "versão nova: oferece substituir a instalada" 'já instalado: versão 1.0' "" "460 list" "$INSTALL '$IBASE/src/Editor-2.0-x86_64.AppImage'"
inst_check "Substituir troca o AppImage instalado" 'instalar "Editor Bonito" \(botão\) @@ instalado: "Editor Bonito" em .*/Aplicativos/Editor Bonito.AppImage' "" \
    "440 click 548 400; 520 list" "$INSTALL '$IBASE/src/Editor-2.0-x86_64.AppImage'"
file_check "o AppImage instalado é a versão nova (mesmo nome, atalho com 2.0)" sh -c "tail -c 4 '$INSTALLED' | grep -q nova && grep -q '^X-AppImage-Version=2.0' '$DESK' && [ \$(ls '$MIDE_APPS_DIR' | wc -l) = 1 ]"
unset FAKE_VERSION

# Launcher: o app instalado aparece; clique direito → Mover para a Lixeira pergunta
# (instalador) e Enter remove: o AppImage na Lixeira, atalho e ícone apagados.
LAUNCH="400 combo ctrl+shift+space"
check "launcher mostra o app instalado (busca)" 'resultados da busca: 1 \(primeiro: Editor Bonito\)' "$LAUNCH; 440 type bonito; 470 list"
check "clique direito no app instalado abre o menu" 'menu do app Editor Bonito \(mide-appimage-org.exemplo.Editor\)' "$LAUNCH; 440 type bonito; 470 rclick 179 210; 500 list"
# O instalador aberto pelo launcher escreve num log que não é o do compositor: a
# confirmação é conferida pela janela e o resultado, pelos arquivos.
check "Mover para a Lixeira pede confirmação" 'janela "Mover para a Lixeira"' "$LAUNCH; 440 type bonito; 470 rclick 179 210; 500 click 260 231; 640 list"
file_check "cancelado: nada foi removido" test -x "$INSTALLED" -a -e "$DESK"
check_count "confirmar (Enter) fecha a janela de confirmação" 'janela "Mover para a Lixeira"' 0 "$LAUNCH; 440 type bonito; 470 rclick 179 210; 500 click 260 231; 660 key enter; 720 list"
file_check "AppImage na Lixeira, sem atalho nem ícone" sh -c "test ! -e '$INSTALLED' && test ! -e '$DESK' && test ! -e '$XDG_DATA_HOME/mide/appimage-icons/org.exemplo.Editor.svg' && ls '$XDG_DATA_HOME/Trash/files' | grep -q 'Editor Bonito.AppImage'"
check "app removido some do launcher" 'resultados da busca: 0 ' "$LAUNCH; 440 type bonito; 470 list"

# Pastas da extração: só no cache da seção, só do usuário (0700), e as de instaladores
# que já saíram (a bateria encerra as janelas sem fechar) são apagadas pelo próximo.
file_check "pasta da extração no cache, só do usuário, sem sobras" sh -c "n=\$(ls -d '$XDG_CACHE_HOME'/mide/instalar-* 2>/dev/null | wc -l); [ \$n -le 1 ] && for d in '$XDG_CACHE_HOME'/mide/instalar-*; do [ ! -e \"\$d\" ] || [ \$(stat -c %a \"\$d\") = 700 ] || exit 1; done"
unset XDG_DATA_HOME MIDE_APPS_DIR XDG_CACHE_HOME
export HOME="$OLD_HOME"
rm -rf "$IBASE"
