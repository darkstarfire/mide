#!/usr/bin/env bash
# Teste do instalador (install.sh), sem root e sem mexer no sistema: a simulação de
# cada distribuição mostra os comandos certos, e uma instalação de verdade numa pasta
# (--destdir) põe os arquivos nos lugares certos, atualiza apagando o que sobrou da
# versão anterior e remove tudo. Usa os programas que já estão em target/release.
#
# Uso: tests/install-test.sh
set -u
cd "$(dirname "$0")/.."
[ -x target/release/mide ] || { echo "compile antes: cargo build --release"; exit 1; }
PASS=0
FAIL=0
ok() { echo "ok    $1"; PASS=$((PASS + 1)); }
falhou() { echo "FALHA $1"; FAIL=$((FAIL + 1)); }
confere() { if grep -qE -- "$2" <<< "$3"; then ok "$1"; else falhou "$1 (esperado: $2)"; fi; }

base="$(mktemp -d)"
trap 'rm -rf "$base" target/install' EXIT
roda() { LANG=pt_BR.UTF-8 ./install.sh "$@" 2>&1; }

echo "== simulação por distribuição"
s="$(roda --install --dry-run --yes --no-build --no-icons --distro debian)"
confere "debian: apt-get com as bibliotecas" "apt-get install -y build-essential .*libseat-dev" "$s"
confere "debian: satellite compilado (bibliotecas do xcb)" "libxcb-cursor-dev" "$s"
confere "debian: cópia para / com tar" "tar -C / -x --no-same-owner" "$s"
s="$(roda --install --dry-run --yes --no-build --no-icons --distro fedora)"
confere "fedora: dnf pulando o que não existe" "dnf install -y --skip-unavailable .*libseat-devel" "$s"
s="$(roda --install --dry-run --yes --no-build --no-icons --distro arch)"
confere "arch: pacman com o satellite pronto" "pacman -S --needed --noconfirm .*xwayland-satellite" "$s"
s="$(roda --install --dry-run --yes --no-build --no-icons --distro slackware --login)"
confere "slackware: seatd e portal wlr pelo SBo" "SlackBuilds.org: seatd .*" "$s"
confere "slackware: pacote .txz com o makepkg" "makepkg -l y -c y .*mide-[0-9.]+-[a-z0-9_]+-1_mide.txz" "$s"
confere "slackware: em /usr" "programas em /usr/bin \(como pacote" "$s"
confere "slackware: greetd pelo SBo com a tela de login" "SlackBuilds.org: greetd" "$s"
confere "slackware: elogind avisado (sem systemd)" "pkill -HUP -x elogind-daemon" "$s"
s="$(printf '11\n' | roda)"
confere "menu: sair não faz nada" "11\) Sair" "$s"
confere "menu: temas do boot" "Temas do boot do mios" "$s"
confere "menu: ISO" "ISO do mios" "$s"

echo "== comandos (simulação)"
s="$(roda build --dry-run --yes)"
confere "build: limpa a pasta de debug" "rm -rf target/debug" "$s"
confere "build: no máximo 4 núcleos, prioridade baixa" "nice -n 19 cargo build --release --locked -j [1-4]$" "$s"
s="$(CARGO_BUILD_JOBS=7 roda build --dry-run --yes)"
confere "build: CARGO_BUILD_JOBS escolhe os núcleos" "-j 7$" "$s"
s="$(roda deps --dry-run --yes --distro fedora)"
confere "deps: só os pacotes" "dnf install -y --skip-unavailable .*libseat-devel" "$s"
s="$(roda login remove --dry-run --yes)"
confere "login remove: volta à tela de antes" "mide-login-install --uninstall" "$s"
s="$(roda boot-theme remove --dry-run --yes)"
confere "boot-theme remove: volta aos temas de antes" "install-boot-theme --uninstall" "$s"
s="$(MIDE_VERSION_SUFFIX=+teste roda deb --dry-run --yes --no-build --no-deps)"
confere "deb: pacote do mide com o sufixo" "MIDE_VERSION_SUFFIX=\+teste .*packaging/debian/build-deb.sh --no-build" "$s"
confere "deb: pacote do mios-release" "packaging/mios-release/build-deb.sh" "$s"
s="$(roda deb --dry-run --yes --no-build --no-deps --distro fedora 2>&1)"
confere "deb: só no Debian/Ubuntu" "gerados no Debian/Ubuntu" "$s"
if grep -q '^VERSION_CODENAME=trixie' /etc/os-release 2>/dev/null; then
    s="$(MIDE_VERSION_SUFFIX= roda iso --dry-run --yes --no-build --no-deps)"
    confere "iso: live-build do sistema no Debian 13" "iso/build.sh$" "$s"
    confere "iso: o chroot sai e a ISO volta ao usuário" "lb clean .*chown -R" "$s"
fi

if command -v dpkg >/dev/null; then
    echo "== pacote pronto (repositório apt de teste)"
    apt="$base/apt"
    mkdir -p "$apt"
    printf 'Package: mide\nVersion: 9.9\nDepends: libc6 (>= 2.0), libgcc-s1\n\n' > "$apt/Packages"
    s="$(MIDE_APT_URL="file://$apt" roda --install --dry-run --yes --distro debian)"
    confere "apt: oferecido e escolhido por padrão" "> 1" "$s"
    confere "apt: chave e fonte do repositório" "install -D -m644 .*/etc/apt/sources.list.d/mios.sources" "$s"
    confere "apt: instala o pacote mide" "apt-get install -y mide$" "$s"
    # Arquivos do data/mide-update (instalação de desenvolvimento) numa pasta pessoal de teste:
    # saem os que apontam para a pasta do projeto e o tema de ícones dele; o resto fica.
    casa="$base/casa"
    mkdir -p "$casa/.local/share/dbus-1/services" "$casa/.config/xdg-desktop-portal-wlr" "$casa/.local/share/icons/mide" \
        "$casa/.local/share/icons/hicolor/scalable/apps" "$casa/.local/share/mide/fonts"
    printf '[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.mide\nExec=/projeto/target/release/mide-portal --portal-only\n' \
        > "$casa/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.mide.service"
    printf '[screencast]\nchooser_type=simple\nchooser_cmd=/projeto/target/release/mide-screencast-picker\n' > "$casa/.config/xdg-desktop-portal-wlr/mide"
    echo 11f8484 > "$casa/.local/share/icons/mide/.versao-mide-icons"
    touch "$casa/.local/share/icons/hicolor/scalable/apps/mide-files.svg"
    s="$(HOME="$casa" XDG_DATA_HOME='' XDG_CONFIG_HOME='' MIDE_APT_URL="file://$apt" roda --install --apt --dry-run --yes --distro debian)"
    confere "apt: avisa dos arquivos de desenvolvimento na pasta pessoal" "saem da pasta pessoal" "$s"
    confere "apt: tira o portal da pasta do projeto" "rm -rf $casa/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.mide.service$" "$s"
    confere "apt: tira o seletor de tela da pasta do projeto" "rm -rf $casa/.config/xdg-desktop-portal-wlr/mide$" "$s"
    confere "apt: tira o tema de ícones do mide-update" "rm -rf $casa/.local/share/icons/mide$" "$s"
    confere "apt: tira a cópia do ícone no hicolor" "rm -rf $casa/.local/share/icons/hicolor/scalable/apps/mide-files.svg$" "$s"
    if grep -q "rm -rf $casa/.local/share/mide" <<< "$s"; then falhou "apt: fontes e dados do mide ficam"; else ok "apt: fontes e dados do mide ficam"; fi
    printf '[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.mide\nExec=/usr/bin/mide-portal --portal-only\n' \
        > "$casa/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.mide.service"
    rm -rf "$casa/.local/share/icons/mide"
    s="$(HOME="$casa" XDG_DATA_HOME='' XDG_CONFIG_HOME='' MIDE_APT_URL="file://$apt" roda --install --apt --dry-run --yes --distro debian)"
    if grep -q "dbus-1/services" <<< "$s"; then falhou "apt: arquivo que não aponta para o projeto fica"; else ok "apt: arquivo que não aponta para o projeto fica"; fi
    printf 'Package: mide\nVersion: 9.9\nDepends: libc6 (>= 99.0)\n\n' > "$apt/Packages"
    s="$(MIDE_APT_URL="file://$apt" roda --install --dry-run --yes --distro debian)"
    confere "apt: glibc antiga demais compila" "cargo build --release --locked" "$s"
    s="$(MIDE_APT_URL="file://$apt/nada" roda --install --dry-run --yes --distro debian --apt)"
    confere "apt: --apt sem repositório para com aviso" "não serve a este sistema" "$s"
    s="$(HOME="$casa" XDG_DATA_HOME='' XDG_CONFIG_HOME='' MIDE_APT_URL="file://$apt" roda --install --dry-run --yes --distro debian --no-build --no-icons)"
    confere "apt: --no-build usa o compilado" "tar -C / -x" "$s"
    confere "compilado: também tira os arquivos de desenvolvimento" "rm -rf $casa/.config/xdg-desktop-portal-wlr/mide$" "$s"
fi

echo "== com o pacote do repositório instalado"
# dpkg-query falso dizendo que o pacote mide está instalado. O mide-update roda numa cópia
# fora do projeto: se a proteção falhasse, ele não compilaria nem apagaria nada daqui.
falso="$base/falso"
mkdir -p "$falso" "$base/projeto/data"
printf '#!/bin/sh\necho "install ok installed"\n' > "$falso/dpkg-query"
chmod +x "$falso/dpkg-query"
s="$(PATH="$falso:$PATH" XDG_CURRENT_DESKTOP='' roda update --dry-run)"
confere "update: com o pacote, explica e não compila" "pacote do repositório do mide" "$s"
cp data/mide-update "$base/projeto/data/"
s="$(PATH="$falso:$PATH" sh "$base/projeto/data/mide-update" 2>&1)"
confere "mide-update: com o pacote, explica e não compila" "pacote do repositório apt" "$s"

echo "== instalação numa pasta"
d="$base/raiz"
roda --install --yes --distro debian --no-build --no-deps --no-icons --destdir "$d" >/dev/null
for f in /usr/local/bin/mide /usr/local/bin/mide-session /usr/local/bin/mide-bar /usr/local/share/mide/applications/mide-files.desktop \
    /usr/local/share/mide/fonts/NotoSans-Regular.ttf /usr/share/wayland-sessions/mide.desktop /usr/share/xdg-desktop-portal/portals/mide.portal \
    /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.mide.service /etc/pam.d/mide-lock /etc/xdg/xdg-desktop-portal-wlr/mide \
    /var/lib/mide/installed-files; do
    if [ -e "$d$f" ]; then ok "instalado: $f"; else falhou "instalado: $f"; fi
done
confere "sessão aponta para o mide-session instalado" "^Exec=/usr/local/bin/mide-session$" "$(cat "$d/usr/share/wayland-sessions/mide.desktop")"
confere "lista com os programas" "^/usr/local/bin/mide-files$" "$(cat "$d/var/lib/mide/installed-files")"
confere "escolhas guardadas para as atualizações" "^PREFIXO=/usr/local$" "$(cat "$d/var/lib/mide/install-options")"
touch "$d/usr/local/bin/mide-velho"
echo /usr/local/bin/mide-velho >> "$d/var/lib/mide/installed-files"
sort -o "$d/var/lib/mide/installed-files" "$d/var/lib/mide/installed-files"
roda --install --yes --distro debian --no-build --no-deps --no-icons --destdir "$d" >/dev/null
if [ ! -e "$d/usr/local/bin/mide-velho" ] && [ -x "$d/usr/local/bin/mide" ]; then ok "atualizar apaga o que saiu da versão nova"; else falhou "atualizar apaga o que saiu da versão nova"; fi
s="$(roda --remove --yes --destdir "$d")"
restou="$(cd "$d" && find . \( -type f -o -type l \) | head)"
if [ -z "$restou" ]; then ok "remover apaga todos os arquivos"; else falhou "remover apaga todos os arquivos: $restou"; fi
if [ ! -d "$d/usr/local/share/mide" ]; then ok "remover apaga a pasta de dados do mide"; else falhou "remover apaga a pasta de dados do mide"; fi
confere "remover de novo: nada a fazer" "não está instalado" "$(roda --remove --yes --destdir "$d")"

echo
echo "$PASS ok, $FAIL falhas"
[ "$FAIL" = 0 ]
