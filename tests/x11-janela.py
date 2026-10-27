#!/usr/bin/env python3
# App X11 de teste (A27, seção restaurar-arrastando da bateria): janela "X11 CSD"
# 500x300 sem decoração (dicas do Motif), com a barra desenhada por ela, como o VS Code
# e o Eclipse pelo XWayland. Apertar nos 40 px de cima pede ao gerenciador de janelas
# o arrasto (_NET_WM_MOVERESIZE), que o xwayland-satellite repassa ao mide como
# xdg_toplevel.move; apertar a 6 px da borda direita pede o redimensionamento.
# Uso: x11-janela.py [max]   (max: abre maximizada, pelo _NET_WM_STATE, como o
# Chromium/Electron: propriedade antes de mapear e pedido ao gerenciador depois)
# Precisa do python3-xlib. Sai sozinho em 90 s.
import sys
import time

from Xlib import X, Xatom, display, protocol

d = display.Display()
screen = d.screen()
root = screen.root
win = root.create_window(
    0, 0, 500, 300, 0, screen.root_depth, X.InputOutput, X.CopyFromParent,
    background_pixel=0x2060E0,
    event_mask=X.ExposureMask | X.ButtonPressMask | X.StructureNotifyMask,
)
win.set_wm_name("X11 CSD")
win.change_property(d.intern_atom("_NET_WM_NAME"), d.intern_atom("UTF8_STRING"), 8, b"X11 CSD")
win.set_wm_class("x11-janela", "X11-janela")
# Sem decoração do gerenciador: a barra é do app.
motif = d.intern_atom("_MOTIF_WM_HINTS")
win.change_property(motif, motif, 32, [2, 0, 0, 0, 0])
maximize = "max" in sys.argv[1:]
net_state = d.intern_atom("_NET_WM_STATE")
max_atoms = [d.intern_atom("_NET_WM_STATE_MAXIMIZED_VERT"), d.intern_atom("_NET_WM_STATE_MAXIMIZED_HORZ")]
if maximize:
    win.change_property(net_state, Xatom.ATOM, 32, max_atoms)
win.map()
d.flush()
if maximize:
    time.sleep(0.5)
    msg = protocol.event.ClientMessage(window=win, client_type=net_state, data=(32, [1, max_atoms[0], max_atoms[1], 1, 0]))
    root.send_event(msg, event_mask=X.SubstructureRedirectMask | X.SubstructureNotifyMask)
    d.flush()

moveresize = d.intern_atom("_NET_WM_MOVERESIZE")
width = 500
deadline = time.time() + 90
while time.time() < deadline:
    while d.pending_events():
        ev = d.next_event()
        if ev.type == X.ConfigureNotify:
            width = ev.width
        elif ev.type == X.ButtonPress and ev.detail == 1:
            if ev.event_y < 40:
                direction = 8  # _NET_WM_MOVERESIZE_MOVE
            elif ev.event_x >= width - 6:
                direction = 3  # _NET_WM_MOVERESIZE_SIZE_RIGHT
            else:
                continue
            # Como os toolkits: solta a pega do X e pede ao gerenciador.
            d.ungrab_pointer(X.CurrentTime)
            msg = protocol.event.ClientMessage(
                window=win, client_type=moveresize,
                data=(32, [ev.root_x, ev.root_y, direction, 1, 1]),
            )
            root.send_event(msg, event_mask=X.SubstructureRedirectMask | X.SubstructureNotifyMask)
            d.flush()
            print("arrasto pedido" if direction == 8 else "redimensionar pedido", flush=True)
    time.sleep(0.01)
