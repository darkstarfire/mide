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
echo "Para entrar no mide pela tela de login, saia da sessão atual (fecha os apps abertos)"
echo "e escolha \"mide (desenvolvimento)\"."
echo "Para testar sem sair da sessão atual: Ctrl+Alt+F3, entre no console e rode"
echo "  timeout 300 $PWD/data/mide-session"
echo "(Ctrl+Alt+F7, ou o console da sua sessão, volta para ela.)"
echo "Log da sessão: ~/.local/state/mide/sessao.log"
