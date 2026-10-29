# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== menu global com o LibreOffice (Wayland e X11) e a opção de desligar"
# O LibreOffice publica a barra de menus pelo org.gtk.Menus (com o VCL do GTK 3): no
# Wayland pelo gtk_shell1, no X11 (xwayland-satellite) nas propriedades _GTK_* da janela
# X. São uns 80 grupos de menu com submenu-action: o caso que deixava o menu Arquivo
# vazio na barra. Perfil do LibreOffice numa pasta temporária (nunca o do usuário).
# Barramento sem serviços ativáveis (veja a seção "buscador": o GDK pede o portal de
# configurações e ficaria 25 s parado).
if ! command -v soffice >/dev/null; then
    echo "pulo  menu global do LibreOffice (LibreOffice não instalado)"
elif ! python3 -c "import gi; gi.require_version('Gtk', '3.0'); from gi.repository import Gtk" 2>/dev/null; then
    echo "pulo  menu global do LibreOffice (sem python3-gi)"
else
    LOT="$(mktemp -d)"
    mkdir -p "$LOT/services" "$LOT/cfg-off/mide"
    cat >"$LOT/bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><servicedir>$LOT/services</servicedir>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>
EOF
    # cenário NOME ROTEIRO app...: cada um com o próprio perfil do LibreOffice.
    lo_scenario() {
        local name="$1" script="$2"
        shift 2
        MIDE_PRIVATE_BUS=1 SCENARIO_OUT="$LOT/$name" SCENARIO_TIMEOUT=70 SCENARIO_MAX=70 RUST_LOG="mide=info,mide_bar=info" \
            timeout 150 dbus-run-session --config-file="$LOT/bus.conf" -- tests/scenario.sh "$script" -- "$@" >/dev/null 2>&1
    }
    LO="soffice --norestore --nologo"
    # "Arquivo" do Writer na barra em (213, 14), depois do nome do app.
    # 1) Wayland: menus na barra e o menu Arquivo cheio (Novo › Documento de texto...).
    #    O app GTK de teste confere que o mide anuncia a barra de menus (barra=True).
    lo_scenario wayland "900 click 213 14; 960 list" \
        "GTK_TESTE_BARRA=$LOT/barra-on.txt python3 $PWD/tests/apps/gtk-menu-teste.py" \
        "SAL_USE_VCLPLUGIN=gtk3 $LO -env:UserInstallation=file://$LOT/perfil-w --writer"
    if grep -aqE "menu do app libreoffice-writer: (Arquivo|File), (Editar|Edit), (Exibir|View)" "$LOT/wayland/mide.log" \
        && grep -aqE "menu (Arquivo|File) de libreoffice-writer: ([2-9][0-9]|[0-9]{3}) itens" "$LOT/wayland/mide.log"; then
        echo "ok    LibreOffice no Wayland: menus na barra e o Arquivo com os itens"; PASS=$((PASS + 1))
    else
        echo "FALHA LibreOffice no Wayland: menus na barra e o Arquivo com os itens"; FAIL=$((FAIL + 1))
        grep -a "menu" "$LOT/wayland/mide.log" | sed 's/\x1b\[[0-9;]*m//g' | tail -5
    fi
    if [ "$(cat "$LOT/barra-on.txt" 2>/dev/null)" = "barra=True" ]; then
        echo "ok    menus ligados: o mide anuncia a barra de menus aos apps GTK"; PASS=$((PASS + 1))
    else
        echo "FALHA menus ligados: o mide anuncia a barra de menus aos apps GTK ('$(cat "$LOT/barra-on.txt" 2>/dev/null)')"; FAIL=$((FAIL + 1))
    fi
    # 2) X11 pelo xwayland-satellite: o menu vem das propriedades da janela X.
    if command -v xwayland-satellite >/dev/null || [ -x "$HOME/.cargo/bin/xwayland-satellite" ]; then
        MIDE_XWAYLAND=1 lo_scenario x11 "1200 click 213 14; 1260 list" \
            "sh -c 'sleep 2; DISPLAY=\$(cat \"\$XDG_RUNTIME_DIR/\$MIDE_SOCKET.xdisplay\") SAL_USE_VCLPLUGIN=gtk3 GDK_BACKEND=x11 exec $LO -env:UserInstallation=file://$LOT/perfil-x --writer'"
        if grep -aqE "menu do app libreoffice-writer: (Arquivo|File), (Editar|Edit), (Exibir|View)" "$LOT/x11/mide.log" \
            && grep -aqE "menu (Arquivo|File) de libreoffice-writer: ([2-9][0-9]|[0-9]{3}) itens" "$LOT/x11/mide.log"; then
            echo "ok    LibreOffice no X11 (satellite): menus na barra e o Arquivo com os itens"; PASS=$((PASS + 1))
        else
            echo "FALHA LibreOffice no X11 (satellite): menus na barra e o Arquivo com os itens"; FAIL=$((FAIL + 1))
            grep -a "menu" "$LOT/x11/mide.log" | sed 's/\x1b\[[0-9;]*m//g' | tail -5
        fi
    else
        echo "pulo  LibreOffice no X11 (falta o xwayland-satellite)"
    fi
    # 3) Opção desligada ([bar] app_menus = false): o mide não anuncia a barra de menus
    #    (o app GTK fica com a dele), a barra não toma o nome do registrar (o LibreOffice
    #    no X11 só esconde o menu quando ele existe) e nenhum menu de app vai à barra.
    printf '[bar]\napp_menus = false\n' >"$LOT/cfg-off/mide/config.toml"
    XDG_CONFIG_HOME="$LOT/cfg-off" lo_scenario off "900 list" \
        "GTK_TESTE_BARRA=$LOT/barra-off.txt python3 $PWD/tests/apps/gtk-menu-teste.py" \
        "SAL_USE_VCLPLUGIN=gtk3 $LO -env:UserInstallation=file://$LOT/perfil-o --writer" \
        "sh -c 'sleep 6; gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner com.canonical.AppMenu.Registrar >$LOT/registrar-off.txt'"
    if [ "$(cat "$LOT/barra-off.txt" 2>/dev/null)" = "barra=False" ] \
        && grep -q "false" "$LOT/registrar-off.txt" 2>/dev/null \
        && grep -aq "camada 'mide-bar' criada" "$LOT/off/mide.log" \
        && ! grep -aq "mide_bar.*menu do app " "$LOT/off/mide.log"; then
        echo "ok    menus desligados: o app fica com a barra de menus dele (sem registrar e sem gtk-shell)"; PASS=$((PASS + 1))
    else
        echo "FALHA menus desligados: o app fica com a barra de menus dele ('$(cat "$LOT/barra-off.txt" 2>/dev/null)', registrar '$(cat "$LOT/registrar-off.txt" 2>/dev/null)')"; FAIL=$((FAIL + 1))
    fi
    rm -rf "$LOT"
fi
