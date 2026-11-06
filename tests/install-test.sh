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
s="$(printf '3\n' | roda)"
confere "menu: sair não faz nada" "3\) Sair" "$s"

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
