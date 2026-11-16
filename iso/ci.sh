#!/bin/bash
# Roda dentro de um contêiner debian:trixie com --privileged (o CI do GitHub chama):
# instala o que falta, compila o mide, gera os pacotes .deb e a ISO do mios.
set -euo pipefail
cd "$(dirname "$0")/.."

export DEBIAN_FRONTEND=noninteractive
# Repositórios com contrib, non-free e non-free-firmware (firmware e microcódigo).
sed -i 's/^Components: main$/Components: main contrib non-free non-free-firmware/' /etc/apt/sources.list.d/debian.sources
apt-get update
apt-get install -y --no-install-recommends \
    build-essential git curl ca-certificates pkg-config clang \
    libinput-dev libseat-dev libudev-dev libgbm-dev libdrm-dev libxkbcommon-dev libpam0g-dev \
    libwayland-dev libegl-dev libglib2.0-dev libgudev-1.0-dev libxcb-cursor-dev libxcb1-dev \
    dpkg-dev grub-common live-build debootstrap squashfs-tools xorriso mtools dosfstools \
    grub-pc-bin grub-efi-amd64-bin grub-efi-amd64-signed shim-signed

# Rust (o mide usa a versão estável mais nova). O rustup fica dentro de ~/.cargo, a
# pasta que o CI guarda entre uma execução e outra.
export RUSTUP_HOME="$HOME/.cargo/rustup"
if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
export PATH="$HOME/.cargo/bin:$PATH"
rustup update stable

# O git do contêiner roda como root numa pasta de outro dono.
git config --global --add safe.directory "$PWD"

packaging/debian/build-deb.sh
packaging/mios-release/build-deb.sh
iso/build.sh
