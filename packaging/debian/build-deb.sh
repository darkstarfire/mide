#!/bin/bash
# Gera o pacote .deb do mide (o desktop inteiro, em /usr), para o repositório apt do
# mios e para a ISO. Usa o install.sh para montar os arquivos (o mesmo caminho da
# instalação direta), sem instalar nada no sistema.
#
# Uso: packaging/debian/build-deb.sh [--no-build] [--out DIR]
#   --no-build   usa os programas que já estão em target/release
#   --out DIR    onde fica o .deb (padrão: target/deb)
#
# Precisa das bibliotecas de compilação do Debian (o install.sh as lista) e do dpkg-deb.
set -euo pipefail

cd "$(dirname "$0")/../.."
saida=target/deb
compilar=1
while [ $# -gt 0 ]; do
    case "$1" in
        --no-build) compilar=0; shift ;;
        --out) saida="$2"; shift 2 ;;
        -h | --help) sed -n '2,10p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

versao="$(sed -nE 's/^version = "([^"]+)"/\1/p' Cargo.toml | head -1)"
# Montagens fora de uma versão (a ISO de um commit qualquer) ganham um sufixo, como
# "+git1a2b3c4": ficam acima do pacote publicado com o mesmo número, e o apt não troca
# o pacote do commit pelo da internet.
versao="$versao${MIDE_VERSION_SUFFIX:-}"
arquitetura="$(dpkg --print-architecture)"
raiz="$(readlink -m "target/deb/mide-root")"
rm -rf "$raiz"
mkdir -p "$saida" "$raiz"

opcoes=(--install --yes --distro debian --no-deps --prefix /usr --destdir "$raiz" --x11 --icons --no-login --no-power)
[ "$compilar" -eq 1 ] || opcoes+=(--no-build)
./install.sh "${opcoes[@]}" </dev/null

# A lista de arquivos do install.sh é da instalação direta; no pacote, quem sabe os
# arquivos é o dpkg.
rm -f "$raiz/var/lib/mide/installed-files" "$raiz/var/lib/mide/install-options"
rmdir "$raiz/var/lib/mide" "$raiz/var/lib" "$raiz/var" 2>/dev/null || true

# Programas que o mide usa ao rodar (os mesmos do install.sh) e as bibliotecas dos
# binários.
# A glibc mínima é a mais nova que os binários pedem (símbolos GLIBC_x.y): sem ela, o apt
# instalaria o mide num sistema mais antigo e ele não abriria.
# Os scripts (mide-session...) não são ELF: o objdump falha neles, e com o pipefail a falha
# derrubava o script inteiro, sem mensagem. Só o resultado final conta.
glibc="$( (find "$raiz/usr/bin" -type f -perm -u+x -exec objdump -T {} + 2>/dev/null || true) |
    grep -o 'GLIBC_[0-9.]*' | sed 's/^GLIBC_//' | sort -uV | tail -1 || true)"
[ -n "$glibc" ] || { echo "could not read the glibc version the programs need (objdump)" >&2; exit 1; }
depende="libc6 (>= $glibc), libgcc-s1, libinput10, libseat1, libudev1, libgbm1, libdrm2, libxkbcommon0, libpam0g, \
libwayland-client0, libwayland-server0, libegl1, libgles2, libglib2.0-0t64, libgudev-1.0-0, \
xwayland, xdg-desktop-portal, xdg-desktop-portal-gtk, xdg-desktop-portal-wlr, pipewire, wireplumber, \
ffmpeg, curl, adwaita-icon-theme, udisks2, util-linux, dbus-user-session, x11-xserver-utils, polkitd, \
sudo, xdg-user-dirs, fontconfig"

install -d "$raiz/DEBIAN"
cat >"$raiz/DEBIAN/control" <<EOF
Package: mide
Version: $versao
Architecture: $arquitetura
Maintainer: mide <https://github.com/fabioaacarneiro/mide>
Section: x11
Priority: optional
Homepage: https://github.com/fabioaacarneiro/mide
Depends: $depende
Recommends: greetd, wimtools, qt5-gtk-platformtheme, qt6-gtk-platformtheme, plymouth, plymouth-themes
Description: mide, the Mac Inspired Desktop Environment
 Wayland compositor with its own top bar, dock, launcher, file manager, settings,
 terminal, code editor, writer, calculator, resource monitor and screenshot tools,
 written from scratch in Rust with the mide-ui framework.
EOF
# Tamanho instalado (KB), para o apt mostrar.
tamanho="$(du -sk --exclude=DEBIAN "$raiz" | cut -f1)"
echo "Installed-Size: $tamanho" >>"$raiz/DEBIAN/control"

pacote="$saida/mide_${versao}_${arquitetura}.deb"
# Sem escrita para o grupo e os outros (o umask de quem gera não importa).
chmod -R go-w "$raiz"
dpkg-deb --root-owner-group --build -Zxz "$raiz" "$pacote"
echo "$pacote"
