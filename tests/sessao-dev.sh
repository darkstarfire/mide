#!/usr/bin/env bash
# Instala (ou remove) a entrada "mide (desenvolvimento)" na tela de login, apontando
# para este projeto: escolher ela no login roda o data/mide-session daqui, com os
# binários de target/ (compile antes com `cargo build`).
#
# Uso: sudo tests/sessao-dev.sh            (instala)
#      sudo tests/sessao-dev.sh remover    (remove)
set -eu
cd "$(dirname "$0")/.."
DEST=/usr/share/wayland-sessions/mide-dev.desktop
if [ "${1:-}" = "remover" ]; then
    rm -f "$DEST"
    echo "removido: $DEST"
    exit 0
fi
if [ "$(id -u)" != 0 ]; then
    echo "rode com sudo: sudo $0" >&2
    exit 1
fi
mkdir -p "$(dirname "$DEST")"
cat > "$DEST" <<ENTRY
[Desktop Entry]
Name=mide (desenvolvimento)
Comment=mide deste projeto: $PWD
Exec=$PWD/data/mide-session
Type=Application
DesktopNames=mide
ENTRY
echo "instalado: $DEST"
echo "Saia da sessão e escolha \"mide (desenvolvimento)\" na tela de login."
echo "Log da sessão: ~/.local/state/mide/sessao.log"
