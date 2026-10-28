#!/usr/bin/env python3
"""Player falso de MPRIS para os testes do Tocando Agora da barra.

Publica `org.mpris.MediaPlayer2.miteste` no barramento da sessão (o de teste, pelo
tests/scenario-dbus.sh) com uma música tocando, e anota no arquivo de
`MPRIS_TESTE_ARQUIVO` (uma linha por comando) os comandos que recebe da barra:
PlayPause, Next, Previous, Stop, SetPosition, Shuffle e LoopStatus.
"""
import os
import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

XML = """
<node>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/>
    <method name="Quit"/>
    <property name="Identity" type="s" access="read"/>
    <property name="DesktopEntry" type="s" access="read"/>
    <property name="CanQuit" type="b" access="read"/>
    <property name="CanRaise" type="b" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/>
    <method name="Previous"/>
    <method name="Pause"/>
    <method name="PlayPause"/>
    <method name="Stop"/>
    <method name="Play"/>
    <method name="Seek"><arg type="x" name="Offset" direction="in"/></method>
    <method name="SetPosition"><arg type="o" direction="in"/><arg type="x" direction="in"/></method>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="LoopStatus" type="s" access="readwrite"/>
    <property name="Shuffle" type="b" access="readwrite"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <property name="Volume" type="d" access="readwrite"/>
    <property name="Position" type="x" access="read"/>
    <property name="CanGoNext" type="b" access="read"/>
    <property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/>
    <property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/>
    <property name="CanControl" type="b" access="read"/>
  </interface>
</node>
"""

LOG = os.environ.get("MPRIS_TESTE_ARQUIVO")
state = {"status": "Playing", "shuffle": False, "loop": "None"}


def note(what):
    if LOG:
        with open(LOG, "a") as f:
            f.write(what + "\n")


def metadata():
    return {
        "mpris:trackid": GLib.Variant("o", "/org/mide/teste/faixa1"),
        "mpris:length": GLib.Variant("x", 230_000_000),
        "xesam:title": GLib.Variant("s", "Canção de Teste"),
        "xesam:artist": GLib.Variant("as", ["Banda do mide"]),
        "xesam:album": GLib.Variant("s", "Álbum Falso"),
    }


def get_property(_conn, _sender, _path, iface, prop):
    if iface == "org.mpris.MediaPlayer2":
        values = {"Identity": GLib.Variant("s", "Player de teste"), "DesktopEntry": GLib.Variant("s", "mide-teste"),
                  "CanQuit": GLib.Variant("b", False), "CanRaise": GLib.Variant("b", False)}
        return values.get(prop)
    values = {
        "PlaybackStatus": GLib.Variant("s", state["status"]),
        "LoopStatus": GLib.Variant("s", state["loop"]),
        "Shuffle": GLib.Variant("b", state["shuffle"]),
        "Metadata": GLib.Variant("a{sv}", metadata()),
        "Volume": GLib.Variant("d", 1.0),
        "Position": GLib.Variant("x", 93_000_000),
        "CanGoNext": GLib.Variant("b", True),
        "CanGoPrevious": GLib.Variant("b", True),
        "CanPlay": GLib.Variant("b", True),
        "CanPause": GLib.Variant("b", True),
        "CanSeek": GLib.Variant("b", True),
        "CanControl": GLib.Variant("b", True),
    }
    return values.get(prop)


def changed(conn, props):
    conn.emit_signal(None, "/org/mpris/MediaPlayer2", "org.freedesktop.DBus.Properties", "PropertiesChanged",
                     GLib.Variant("(sa{sv}as)", ("org.mpris.MediaPlayer2.Player", props, [])))


def set_property(conn, _sender, _path, _iface, prop, value):
    if prop == "Shuffle":
        state["shuffle"] = value.unpack()
        note(f"Shuffle {state['shuffle']}")
        changed(conn, {"Shuffle": value})
    elif prop == "LoopStatus":
        state["loop"] = value.unpack()
        note(f"LoopStatus {state['loop']}")
        changed(conn, {"LoopStatus": value})
    return True


def method_call(conn, _sender, _path, _iface, method, params, invocation):
    note(method)
    if method == "PlayPause":
        state["status"] = "Paused" if state["status"] == "Playing" else "Playing"
        changed(conn, {"PlaybackStatus": GLib.Variant("s", state["status"])})
    invocation.return_value(None)


def on_bus(conn, _name):
    node = Gio.DBusNodeInfo.new_for_xml(XML)
    for iface in node.interfaces:
        conn.register_object("/org/mpris/MediaPlayer2", iface, method_call, get_property, set_property)


Gio.bus_own_name(Gio.BusType.SESSION, "org.mpris.MediaPlayer2.miteste", Gio.BusNameOwnerFlags.NONE, on_bus, None, None)
loop = GLib.MainLoop()
GLib.timeout_add_seconds(int(sys.argv[1]) if len(sys.argv) > 1 else 60, loop.quit)
loop.run()
