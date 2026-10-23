# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== apps X11 (XWayland, precisa de xwayland-satellite, Xwayland e xclock)"
if command -v xclock >/dev/null && { command -v xwayland-satellite >/dev/null || [ -x "$HOME/.cargo/bin/xwayland-satellite" ]; }; then
    # O app X11 usa o display do XWayland deste mide (gravado ao lado do socket).
    XAPP="sh -c 'sleep 2; DISPLAY=\$(cat \"\$XDG_RUNTIME_DIR/\$MIDE_SOCKET.xdisplay\") exec xclock -geometry 300x300'"
    # O xclock abre ~3 s depois do mide (espera do app e partida do Xwayland): os
    # passos começam no quadro 2000.
    MIDE_XWAYLAND=1 SCENARIO_WAIT=42 SCENARIO_TIMEOUT=60 check "app X11 (xclock) abre como janela do mide" 'janela "xclock" .*barra=30' "2000 list" "$XAPP"
    # Aparece na lista de antes do clique (1 vez) e some depois dele.
    MIDE_XWAYLAND=1 SCENARIO_WAIT=42 SCENARIO_TIMEOUT=60 check_count "fechar pela barra do mide fecha o app X11" 'janela "xclock"' 1 "2000 list; 2020 click 63 61; 2100 list" "$XAPP"
else
    echo "pulo  apps X11 (falta xclock ou xwayland-satellite)"
fi
