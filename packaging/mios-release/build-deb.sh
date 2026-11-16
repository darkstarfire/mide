#!/bin/bash
# Gera o pacote mios-release: a identidade do mios (Mac Inspired Operational System)
# por cima dos pacotes do Debian 13.
#
#   - /usr/lib/os-release, /etc/issue e /etc/issue.net do mios, desviados com o
#     dpkg-divert (as atualizações do base-files não os trocam de volta);
#   - /etc/lsb-release e GRUB_DISTRIBUTOR=mios (o nome no menu do GRUB);
#   - os temas do GRUB e do Plymouth (data/mios), já com as fontes .pf2;
#   - a configuração padrão do mide para usuários novos (/etc/skel): o papel de parede
#     automático do mios;
#   - o repositório apt do mios (chave e fonte), para as atualizações do mide.
#
# Uso: packaging/mios-release/build-deb.sh [--out DIR]   (padrão: target/deb)
# Precisa do grub-mkfont (grub-common) e do dpkg-deb.
set -euo pipefail

cd "$(dirname "$0")/../.."
saida=target/deb
while [ $# -gt 0 ]; do
    case "$1" in
        --out) saida="$2"; shift 2 ;;
        -h | --help) sed -n '2,15p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

versao="$(cat iso/VERSION)"
# Versão do pacote: a do mios mais a do mide (sobe a cada versão, e o apt atualiza a
# identidade, os temas e a fonte do apt junto com o mide).
pacote_versao="$versao+$(sed -nE 's/^version = "([^"]+)"/\1/p' Cargo.toml | head -1)"
raiz="$(readlink -m target/deb/mios-release-root)"
rm -rf "$raiz"
mkdir -p "$saida" "$raiz/DEBIAN" "$raiz/usr/lib/mios" "$raiz/etc/default/grub.d" "$raiz/etc/skel/.config/mide"

# Identidade: os arquivos do mios ficam em /usr/lib/mios e o postinst os põe no lugar
# dos do Debian (desviados para *.debian).
cat >"$raiz/usr/lib/mios/os-release" <<EOF
PRETTY_NAME="mios $versao"
NAME="mios"
VERSION_ID="$versao"
VERSION="$versao"
VERSION_CODENAME=trixie
ID=mios
ID_LIKE=debian
HOME_URL="https://github.com/fabioaacarneiro/mide"
SUPPORT_URL="https://github.com/fabioaacarneiro/mide/issues"
BUG_REPORT_URL="https://github.com/fabioaacarneiro/mide/issues"
LOGO=mios
EOF
printf 'mios %s \\n \\l\n\n' "$versao" >"$raiz/usr/lib/mios/issue"
printf 'mios %s\n' "$versao" >"$raiz/usr/lib/mios/issue.net"
cat >"$raiz/etc/lsb-release" <<EOF
DISTRIB_ID=mios
DISTRIB_RELEASE=$versao
DISTRIB_CODENAME=trixie
DISTRIB_DESCRIPTION="mios $versao"
EOF
echo 'GRUB_DISTRIBUTOR="mios"' >"$raiz/etc/default/grub.d/mios.cfg"

# Temas do boot, com as fontes do GRUB já geradas.
tema="$raiz/boot/grub/themes/mios"
install -d "$tema" "$raiz/usr/share/plymouth/themes/mios"
cp data/mios/grub/mios/* "$tema/"
for t in 14 16 18; do
    grub-mkfont -s "$t" -o "$tema/noto-sans-$t.pf2" data/fonts/NotoSans-Regular.ttf
done
grub-mkfont -s 18 -o "$tema/noto-sans-bold-18.pf2" data/fonts/NotoSans-Bold.ttf
cp data/mios/plymouth/mios/* "$raiz/usr/share/plymouth/themes/mios/"
echo 'GRUB_THEME="/boot/grub/themes/mios/theme.txt"' >>"$raiz/etc/default/grub.d/mios.cfg"
echo 'GRUB_GFXMODE="1920x1080,1366x768,auto"' >>"$raiz/etc/default/grub.d/mios.cfg"

# Repositório apt do mios (as atualizações do mide): a chave pública e a fonte, em
# formato deb822. É um repositório "plano" nos Releases do GitHub (a release "apt").
install -d "$raiz/usr/share/keyrings" "$raiz/etc/apt/sources.list.d"
install -m644 data/mios/apt/mios-archive-keyring.gpg "$raiz/usr/share/keyrings/"
cat >"$raiz/etc/apt/sources.list.d/mios.sources" <<'EOF'
Types: deb
URIs: https://github.com/fabioaacarneiro/mide/releases/download/apt/
Suites: ./
Signed-By: /usr/share/keyrings/mios-archive-keyring.gpg
EOF

# Usuários novos: o papel de parede automático do mios (claro e escuro).
cat >"$raiz/etc/skel/.config/mide/config.toml" <<'EOF'
[desktop]
wallpaper = "/usr/share/mide/backgrounds/mios-light.jpg"
wallpaper_dark = "/usr/share/mide/backgrounds/mios-dark.jpg"
EOF

cat >"$raiz/DEBIAN/control" <<EOF
Package: mios-release
Version: $pacote_versao
Architecture: all
Maintainer: mide <https://github.com/fabioaacarneiro/mide>
Section: admin
Priority: optional
Depends: base-files, mide
Recommends: plymouth, plymouth-themes, grub-common
Description: mios identity, boot themes and defaults
 Name and version of mios (Mac Inspired Operational System), the GRUB and Plymouth
 themes, and the default mide settings for new users.
EOF
cat >"$raiz/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
# Os arquivos do Debian vão para *.debian; os do mios ficam no lugar.
desviar() {
    dpkg-divert --package mios-release --add --rename --divert "$1.debian" "$1"
    rm -f "$1"
    cp "$2" "$1"
}
if [ "$1" = configure ]; then
    desviar /usr/lib/os-release /usr/lib/mios/os-release
    desviar /etc/issue /usr/lib/mios/issue
    desviar /etc/issue.net /usr/lib/mios/issue.net
    if command -v plymouth-set-default-theme >/dev/null 2>&1; then
        plymouth-set-default-theme mios || true
    fi
    if command -v update-grub >/dev/null 2>&1 && [ -e /boot/grub/grub.cfg ]; then
        update-grub || true
    fi
    if command -v update-initramfs >/dev/null 2>&1 && ls /boot/initrd.img-* >/dev/null 2>&1; then
        update-initramfs -u || true
    fi
fi
exit 0
EOF
cat >"$raiz/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e
# Remoção: os arquivos do Debian voltam.
if [ "$1" = remove ] || [ "$1" = purge ]; then
    for f in /usr/lib/os-release /etc/issue /etc/issue.net; do
        rm -f "$f"
        dpkg-divert --package mios-release --remove --rename --divert "$f.debian" "$f"
    done
fi
exit 0
EOF
chmod 755 "$raiz/DEBIAN/postinst" "$raiz/DEBIAN/postrm"
# Arquivos de configuração (o dpkg pergunta antes de trocar um que o usuário mudou).
printf '%s\n' /etc/lsb-release /etc/default/grub.d/mios.cfg /etc/skel/.config/mide/config.toml /etc/apt/sources.list.d/mios.sources >"$raiz/DEBIAN/conffiles"
echo "Installed-Size: $(du -sk --exclude=DEBIAN "$raiz" | cut -f1)" >>"$raiz/DEBIAN/control"

pacote="$saida/mios-release_${pacote_versao}_all.deb"
# Sem escrita para o grupo e os outros (o umask de quem gera não importa).
chmod -R go-w "$raiz"
dpkg-deb --root-owner-group --build -Zxz "$raiz" "$pacote"
echo "$pacote"
