#!/bin/bash
# Gera a ISO do mios com o live-build (Debian 13 por baixo). Precisa de root (o
# live-build monta o chroot), de rede e dos pacotes .deb já gerados em target/deb:
#   packaging/debian/build-deb.sh        (mide)
#   packaging/mios-release/build-deb.sh  (mios-release)
#
# Uso: sudo iso/build.sh      → target/iso/mios-<versão>-amd64.iso (e o .sha256)
set -euo pipefail

cd "$(dirname "$0")/.."
projeto="$PWD"
versao="$(cat iso/VERSION)"
trabalho="$projeto/target/iso/work"
[ "$(id -u)" -eq 0 ] || { echo "run as root (the live-build mounts the chroot)" >&2; exit 1; }
ls target/deb/mide_*.deb target/deb/mios-release_*.deb >/dev/null || {
    echo "build the packages first: packaging/debian/build-deb.sh and packaging/mios-release/build-deb.sh" >&2
    exit 1
}

# Uma pasta de trabalho nova a cada vez (o cache dos pacotes do Debian fica).
if [ -d "$trabalho" ]; then
    (cd "$trabalho" && lb clean --purge >/dev/null 2>&1) || true
    find "$trabalho" -mindepth 1 -maxdepth 1 ! -name cache -exec rm -rf {} +
fi
mkdir -p "$trabalho"
cp -a iso/live/. "$trabalho/"

# Pacotes do mios (o mais novo de cada).
install -d "$trabalho/config/packages.chroot"
cp "$(ls -t target/deb/mide_*.deb | head -1)" "$(ls -t target/deb/mios-release_*.deb | head -1)" "$trabalho/config/packages.chroot/"
# O mios-release também na lista (sem ela, os .deb locais só ficam disponíveis).
printf 'mide\nmios-release\n' >"$trabalho/config/package-lists/mios-local.list.chroot"

# Tema do GRUB da própria ISO (o mesmo do sistema), com as fontes geradas.
tema="$trabalho/config/bootloaders/grub-pc/themes/mios"
install -d "$tema"
cp data/mios/grub/mios/* "$tema/"
for t in 14 16 18; do
    grub-mkfont -s "$t" -o "$tema/noto-sans-$t.pf2" data/fonts/NotoSans-Regular.ttf
done
grub-mkfont -s 18 -o "$tema/noto-sans-bold-18.pf2" data/fonts/NotoSans-Bold.ttf

cd "$trabalho"
lb config
lb build 2>&1 | tee build.log

iso="$(ls -t ./*.iso | head -1)"
destino="$projeto/target/iso/mios-$versao-amd64.iso"
mv "$iso" "$destino"
(cd "$(dirname "$destino")" && sha256sum "$(basename "$destino")" >"$(basename "$destino").sha256")
echo "$destino"
