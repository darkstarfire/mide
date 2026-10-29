#!/usr/bin/env python3
"""App GTK 3 de teste do menu global no Wayland (gtk-shell): uma janela com barra de
menus ("Arquivo": Abrir, Salvar; "Ajuda": Sobre) e uma opção marcável ("Exibir" →
"Mostrar régua"). Cada ação grava o nome dela em GTK_TESTE_ARQUIVO. Com
GTK_TESTE_BARRA, grava ali, 2 s depois de abrir, se o compositor disse que mostra a
barra de menus (`gtk-shell-shows-menubar`: "barra=True") ou não ("barra=False")."""

import os
import sys

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gio, GLib, Gtk  # noqa: E402

OUT = os.environ.get("GTK_TESTE_ARQUIVO")


def record(text):
    if OUT:
        with open(OUT, "w") as f:
            f.write(text)


class App(Gtk.Application):
    def __init__(self):
        super().__init__(application_id="org.mide.GtkMenuTeste", flags=Gio.ApplicationFlags.NON_UNIQUE)

    def do_startup(self):
        Gtk.Application.do_startup(self)
        for name in ("abrir", "salvar", "sobre"):
            action = Gio.SimpleAction.new(name, None)
            action.connect("activate", lambda a, p, n=name: record(n))
            self.add_action(action)
        regua = Gio.SimpleAction.new_stateful("regua", None, GLib.Variant.new_boolean(True))
        regua.connect("activate", lambda a, p: (a.set_state(GLib.Variant.new_boolean(not a.get_state().get_boolean())), record("regua")))
        self.add_action(regua)
        menubar = Gio.Menu()
        arquivo = Gio.Menu()
        arquivo.append("_Abrir", "app.abrir")
        secao = Gio.Menu()
        secao.append("_Salvar", "app.salvar")
        arquivo.append_section(None, secao)
        exibir = Gio.Menu()
        exibir.append("Mostrar _régua", "app.regua")
        ajuda = Gio.Menu()
        ajuda.append("_Sobre", "app.sobre")
        menubar.append_submenu("_Arquivo", arquivo)
        menubar.append_submenu("_Exibir", exibir)
        menubar.append_submenu("A_juda", ajuda)
        self.set_menubar(menubar)
        self.set_accels_for_action("app.salvar", ["<Primary>s"])

    def do_activate(self):
        window = Gtk.ApplicationWindow(application=self, title="Menu GTK de teste")
        window.set_default_size(420, 240)
        window.show_all()
        barra = os.environ.get("GTK_TESTE_BARRA")
        if barra:
            def report():
                shows = Gtk.Settings.get_default().get_property("gtk-shell-shows-menubar")
                with open(barra, "w") as f:
                    f.write(f"barra={shows}")
                return False
            GLib.timeout_add(2000, report)


sys.exit(App().run([]))
