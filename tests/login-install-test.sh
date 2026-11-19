#!/usr/bin/env bash
# Teste da tela de login (data/mide-login-install) sem root e sem mexer no sistema: o
# script roda como root de mentira (unshare -rm), com as pastas que ele mexe trocadas por
# pastas de teste (overlay em /etc, /usr/share e /usr/bin; mount --bind em /var/lib e
# /usr/local: o overlay recusa pastas com outras montagens dentro, como as do docker) e com systemctl, dpkg-query, getent, install e chown falsos (o systemctl
# anota o que foi pedido). Nada sai do namespace: as pastas de teste ficam em mktemp.
#
# Uso: tests/login-install-test.sh
set -u
cd "$(dirname "$0")/.."
PROJETO="$PWD"
if ! unshare -rm true 2>/dev/null; then
    echo "pulo  tela de login (sem user namespaces: unshare -rm)"
    exit 0
fi
PASS=0
FAIL=0
ok() { echo "ok    $1"; PASS=$((PASS + 1)); }
falhou() { echo "FALHA $1"; FAIL=$((FAIL + 1)); }
confere() { if grep -qE -- "$2" <<< "$3"; then ok "$1"; else falhou "$1 (esperado: $2)"; fi; }
nao_tem() { if grep -qE -- "$2" <<< "$3"; then falhou "$1 (não esperado: $2)"; else ok "$1"; fi; }

base="$(mktemp -d)"
trap 'chmod -R u+rwx "$base" 2>/dev/null; rm -rf "$base"' EXIT

# Programas falsos: o systemctl anota em $LOG e diz "loaded" só para os serviços de
# $CARREGADOS; o dpkg-query diz que o pacote mide está instalado com SISTEMA_FALSO=1.
falsos="$base/falsos"
mkdir -p "$falsos"
cat > "$falsos/systemctl" <<'EOF'
#!/bin/sh
echo "$*" >> "$LOG"
if [ "$1" = show ]; then
    for s in $CARREGADOS; do [ "$5" = "$s.service" ] && { echo loaded; exit 0; }; done
    echo not-found
fi
exit 0
EOF
cat > "$falsos/dpkg-query" <<'EOF'
#!/bin/sh
for a in "$@"; do last="$a"; done
[ "${SISTEMA_FALSO:-}" = 1 ] && [ "$last" = mide ] && { echo "install ok installed"; exit 0; }
exit 1
EOF
cat > "$falsos/getent" <<'EOF'
#!/bin/sh
[ "$1 $2" = "passwd _greetd" ] && { echo "_greetd:x:997:997::/var/lib/greetd:/bin/false"; exit 0; }
exec /usr/bin/getent "$@"
EOF
# install e chown: sem trocar o dono (o root de mentira não pode dar arquivos a outro usuário).
cat > "$falsos/install" <<'EOF'
#!/bin/bash
args=()
while [ $# -gt 0 ]; do
    case "$1" in -o | -g) shift ;; *) args+=("$1") ;; esac
    shift
done
exec /usr/bin/install "${args[@]}"
EOF
printf '#!/bin/sh\nexit 0\n' > "$falsos/chown"
printf '#!/bin/sh\nexit 0\n' > "$falsos/greetd"
chmod +x "$falsos"/*

# Uma raiz de teste com a instalação de desenvolvimento como o mide-login-install a deixa
# (cópia em /usr/local/lib/mide-login, atalhos em /usr/local/bin, a sessão, os dados) e o
# greetd configurado para ela; o gerenciador de antes registrado é o lightdm.
preparar() {
    local r="$1"
    mkdir -p "$r"/{etc,etc-w,etc-greetd,etc-mide,varlib,share,share-w,usrbin,usrbin-w,local}
    mkdir -p "$r/local/lib/mide-login" "$r/local/bin" "$r/local/share/mide/applications" \
        "$r/local/share/mide/xdg-desktop-portal" "$r/local/share/mide/fonts" "$r/etc-mide/login" \
        "$r/varlib/mide-login" "$r/share/wayland-sessions"
    local p
    for p in mide mide-bar mide-login mide-login-install; do
        printf '#!/bin/sh\n' > "$r/local/lib/mide-login/$p"
        chmod 755 "$r/local/lib/mide-login/$p"
        ln -s "/usr/local/lib/mide-login/$p" "$r/local/bin/$p"
    done
    printf '#!/bin/sh\n' > "$r/local/bin/mide-session"
    printf '#!/bin/sh\n' > "$r/local/bin/outro-programa"
    printf '[Desktop Entry]\nName=mide\nExec=mide-session\n' > "$r/share/wayland-sessions/mide.desktop"
    printf '[default_session]\ncommand = "/usr/local/lib/mide-login/mide --login"\nuser = "_greetd"\n' > "$r/etc-greetd/config.toml"
    printf '[default_session]\ncommand = "agreety"\nuser = "_greetd"\n' > "$r/varlib/mide-login/config.toml.antes"
    echo lightdm > "$r/varlib/mide-login/gerenciador-anterior"
}

# Roda o mide-login-install com os argumentos dados, dentro do namespace com a raiz $1.
rodar() {
    local r="$1"
    shift
    LOG="$r/systemctl.log" PATH="$falsos:$PATH" unshare -rm bash -c '
        r="$1"; shift
        mount -t overlay overlay -o "lowerdir=/etc,upperdir=$r/etc,workdir=$r/etc-w" /etc &&
        mkdir -p /etc/greetd /etc/mide && mount --bind "$r/etc-greetd" /etc/greetd && mount --bind "$r/etc-mide" /etc/mide &&
        mount --bind "$r/varlib" /var/lib &&
        mount -t overlay overlay -o "lowerdir=/usr/share,upperdir=$r/share,workdir=$r/share-w" /usr/share &&
        mount -t overlay overlay -o "lowerdir=/usr/bin,upperdir=$r/usrbin,workdir=$r/usrbin-w" /usr/bin &&
        mount --bind "$r/local" /usr/local || { echo "não deu para montar a raiz de teste"; exit 1; }
        unset SUDO_USER
        bash "$PROJETO/data/mide-login-install" "$@"
    ' _ "$r" "$@" 2>&1
}
export PROJETO

echo "== remover, com o gerenciador de antes desinstalado depois"
r="$base/a"
preparar "$r"
s="$(CARREGADOS="greetd" rodar "$r" --uninstall)"
log="$(cat "$r/systemctl.log" 2>/dev/null)"
confere "o greetd continua ativo" "^enable --force greetd.service$" "$log"
nao_tem "o greetd não é desativado" "^disable greetd" "$log"
confere "avisa que o de antes não existe mais" "lightdm .*não está mais instalado" "$s"
confere "a configuração de antes do greetd volta" 'command = "agreety"' "$(cat "$r/etc-greetd/config.toml")"
if [ ! -e "$r/local/lib/mide-login" ] && [ ! -e "$r/local/bin/mide" ] && [ ! -e "$r/local/bin/mide-session" ] &&
    [ -e "$r/local/bin/outro-programa" ]; then ok "a cópia e os atalhos saem; o resto de /usr/local fica"; else falhou "a cópia e os atalhos saem; o resto de /usr/local fica"; fi

echo "== remover, com o gerenciador de antes instalado"
r="$base/b"
preparar "$r"
s="$(CARREGADOS="greetd lightdm" rodar "$r" --uninstall)"
log="$(cat "$r/systemctl.log" 2>/dev/null)"
confere "o greetd é desativado" "^disable greetd.service$" "$log"
confere "o de antes volta" "^enable --force lightdm.service$" "$log"

echo "== instalar com o mide do sistema (pacote), sobre a cópia de desenvolvimento"
r="$base/c"
preparar "$r"
for p in mide mide-login mide-bar; do printf '#!/bin/sh\n' > "$r/usrbin/$p"; chmod 755 "$r/usrbin/$p"; done
s="$(SISTEMA_FALSO=1 CARREGADOS="greetd" rodar "$r")"
log="$(cat "$r/systemctl.log" 2>/dev/null)"
confere "a tela de login usa o mide do sistema" 'MIDE_BACKEND=drm /usr/bin/mide --login"' "$(cat "$r/etc-greetd/config.toml")"
confere "o greetd é ativado" "^enable --force greetd.service$" "$log"
restou="$(ls "$r/local/lib/mide-login" 2>/dev/null | tr '\n' ' ')"
confere "na pasta da tela de login fica só o instalador" "^mide-login-install $" "$restou"
if [ ! -e "$r/local/bin/mide" ] && [ ! -L "$r/local/bin/mide-bar" ] && [ ! -e "$r/local/bin/mide-session" ] &&
    [ -e "$r/local/bin/outro-programa" ]; then ok "os atalhos e o mide-session da cópia saem; o resto fica"; else falhou "os atalhos e o mide-session da cópia saem; o resto fica"; fi
if [ ! -e "$r/local/share/mide" ]; then ok "os dados da cópia em /usr/local/share/mide saem"; else falhou "os dados da cópia em /usr/local/share/mide saem"; fi
confere "a configuração de antes do greetd continua guardada" 'command = "agreety"' "$(cat "$r/varlib/mide-login/config.toml.antes" 2>/dev/null)"
[ "$FAIL" = 0 ] || printf '%s\n' "$s" | tail -15

echo
echo "$PASS ok, $FAIL falhas"
[ "$FAIL" = 0 ]
