# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== Seletor de arquivos do mide pelo portal (B25)"
# O "app" do cenário pede ao backend do portal (org.freedesktop.impl.portal.desktop.mide,
# do mide-portal) como o xdg-desktop-portal faria, e a resposta do gdbus vai para um
# arquivo. Barramento D-Bus próprio (scenario-dbus.sh): nada toca o barramento da sessão.
# Janela do seletor (720x500) no meio da área útil, em (323, 129), com a barra lateral
# (Favoritos, Locais) à esquerda: na pasta de teste, Sub em (583, 237), a.txt (585, 275),
# b.md (585, 313), c.png (585, 351) e "nota fiscal ç.txt" (618, 389); botão principal
# em (923, 565). Salvar tem o nome em cima (a lista desce 8 px) e o interruptor
# "Só leitura" em (998, 523).
# Barra lateral igual em qualquer máquina: as pastas do usuário (que entram nos
# Favoritos quando existem) apontam para a pasta pessoal e não aparecem; ficam a pasta
# pessoal, a Área de trabalho do teste e o Computador. Com as pastas de verdade (oito
# lugares), a barra lateral fica mais alta que a lista e empurra para baixo o interruptor
# e os botões do Salvar.
cat >"$XDG_CONFIG_HOME/user-dirs.dirs" <<'EOF'
XDG_DESKTOP_DIR="$HOME"
XDG_DOWNLOAD_DIR="$HOME"
XDG_DOCUMENTS_DIR="$HOME"
XDG_MUSIC_DIR="$HOME"
XDG_PICTURES_DIR="$HOME"
XDG_VIDEOS_DIR="$HOME"
EOF
PDIR="$(mktemp -d)"
PASTA="$PDIR/pasta"
mkdir -p "$PASTA/Sub"
echo a >"$PASTA/a.txt"
echo b >"$PASTA/b.md"
echo n >"$PASTA/nota fiscal ç.txt"
echo c >"$PASTA/c.png"
RESPOSTA="$PDIR/resposta.txt"
URI="file://$PASTA"
OUT="${SCENARIO_OUT:-/tmp/mide-scenario}"
# Trecho dos scripts dos pedidos: espera a dock aparecer no log do compositor (mais meio
# segundo para a zona dela valer).
ESPERA_DOCK="for i in \$(seq 100); do grep -aq \"camada 'mide-dock' criada\" '$OUT/mide.log' 2>/dev/null && break; sleep 0.2; done; sleep 0.5"

# pedido "método" "opções a{sv} no formato do gdbus" [espera em s antes do Close]
pedido() {
    local close=""
    if [ -n "${3:-}" ]; then
        close="sleep $3; gdbus call --session -d org.freedesktop.impl.portal.desktop.mide -o /org/freedesktop/portal/desktop/request/1_1/t1 -m org.freedesktop.impl.portal.Request.Close >/dev/null 2>&1"
    fi
    # Espera a dock: a zona exclusiva dela muda a área útil onde o seletor é centrado
    # (com a máquina carregada, o seletor abria antes da dock e ficava 34 px mais baixo).
    # Depois, espera o mide-portal pegar o nome (com a máquina carregada ele demora).
    cat >"$PDIR/pedido.sh" <<EOF
$ESPERA_DOCK
for i in \$(seq 50); do
    gdbus call --session -d org.freedesktop.DBus -o /org/freedesktop/DBus -m org.freedesktop.DBus.GetNameOwner org.freedesktop.impl.portal.desktop.mide >/dev/null 2>&1 && break
    sleep 0.2
done
gdbus call --session -t 100 -d org.freedesktop.impl.portal.desktop.mide -o /org/freedesktop/portal/desktop \\
    -m org.freedesktop.impl.portal.FileChooser.$1 /org/freedesktop/portal/desktop/request/1_1/t1 teste "" "Escolher" "$2" >"$RESPOSTA" 2>&1 &
$close
wait
EOF
}

# portal_check "nome" "regex na resposta do portal" "roteiro" "regex no log (opcional)"
# (resposta SEM_RESPOSTA: o pedido continua aberto, sem resposta do portal; várias
# regex separadas por "&&" valem em qualquer ordem, como as chaves do a{sv})
portal_check() {
    local name="$1" expected="$2" script="$3" in_log="${4:-}"
    : >"$RESPOSTA"
    MIDE_PORTAL_FORCE=1 timeout 150 tests/scenario-dbus.sh "$script" -- "sh '$PDIR/pedido.sh'" >/dev/null 2>&1
    local log
    log="$(sed 's/\x1b\[[0-9;]*m//g' "$OUT/mide.log")"
    local answered
    if [ "$expected" = SEM_RESPOSTA ]; then
        ! grep -q uint32 "$RESPOSTA" && answered=1
    else
        answered=1
        local pat
        while IFS= read -r pat; do
            grep -Eq "$pat" "$RESPOSTA" || answered=""
        done <<<"${expected//&&/$'\n'}"
    fi
    if [ -n "${answered:-}" ] && { [ -z "$in_log" ] || echo "$log" | grep -Eq "$in_log"; }; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name"; echo "      esperado: $expected ${in_log:+/ no log: $in_log}"
        sed 's/^/      resposta: /' "$RESPOSTA" | tail -2
        echo "$log" | grep -E "seletor|autoteste: janela" | sed 's/^/      /' | tail -4
        FAIL=$((FAIL + 1))
    fi
}

ABRIR="'current_folder': <b'$PASTA'>"
pedido OpenFile "{'multiple': <true>, $ABRIR}"
portal_check "Abrir vários: Ctrl+clique e Abrir devolve os dois (resposta 0)" \
    "^\(uint32 0, \{'uris': <\['$URI/a.txt', '$URI/b.md'\]>\}\)" \
    "400 click 585 275; 420 keydown ctrl; 430 click 585 313; 440 keyup ctrl; 470 click 923 565; 520 list" \
    "seletor: escolhido $PASTA/b.md"
portal_check "janela do seletor no meio da tela, com o título do pedido" "uint32 0" \
    "400 click 585 275; 410 list; 470 click 923 565; 520 list" 'janela "Escolher" .*posição=\(323, 129\)'
portal_check "nome com espaço e acento volta em percent-encoding" \
    "'uris': <\['$URI/nota%20fiscal%20%C3%A7.txt'\]>" \
    "400 click 618 389; 470 click 923 565; 520 list"
portal_check "Esc cancela (resposta 1) e fecha a janela" "^\(uint32 1, @a\{sv\} \{\}\)" \
    "400 key esc; 520 list" "seletor: cancelado"
portal_check "Shift+clique escolhe o intervalo" "'uris': <\['$URI/a.txt', '$URI/b.md', '$URI/c.png', '$URI/nota%20fiscal%20%C3%A7.txt'\]>" \
    "400 click 585 275; 420 keydown shift; 430 click 618 389; 440 keyup shift; 470 click 923 565; 520 list"
pedido OpenFile "{$ABRIR}"
portal_check "sem multiple, Ctrl+clique fica com um só" "^\(uint32 0, \{'uris': <\['$URI/b.md'\]>\}\)" \
    "400 click 585 275; 420 keydown ctrl; 430 click 585 313; 440 keyup ctrl; 470 click 923 565; 520 list"
pedido OpenFile "{'directory': <true>, 'accept_label': <'_Escolher pasta'>, $ABRIR}"
portal_check "directory: escolhe a pasta Sub" "^\(uint32 0, \{'uris': <\['$URI/Sub'\]>\}\)" \
    "400 click 583 237; 470 click 923 565; 520 list"
pedido OpenFile "{'filters': <[('Textos', [(uint32 0, '*.txt')]), ('Imagens', [(uint32 1, 'image/*')])]>, 'current_filter': <('Imagens', [(uint32 1, 'image/*')])>, $ABRIR}"
# Filtro Imagens: só Sub e c.png (na segunda linha).
portal_check "filtro por tipo esconde os textos e volta em current_filter" \
    "'uris': <\['$URI/c.png'\]>&&'current_filter': <\('Imagens', \[\(uint32 1, 'image/\*'\)\]\)>" \
    "400 click 585 275; 470 click 923 565; 520 list"
SALVAR="'current_folder': <b'$PASTA'>, 'choices': <[('ro', 'Só leitura', @a(ss) [], 'false')]>"
pedido SaveFile "{'current_name': <'novo nome.txt'>, $SALVAR}"
portal_check "Salvar: Enter no nome, opção marcada volta em choices" \
    "^\(uint32 0, &&'uris': <\['$URI/novo%20nome.txt'\]>&&'choices': <\[\('ro', 'true'\)\]>&&'writable': <true>" \
    "400 click 998 523; 450 key enter; 520 list"
pedido SaveFile "{'current_name': <'a.txt'>, 'current_folder': <b'$PASTA'>}"
portal_check "Salvar por cima pergunta antes (Enter, Enter substitui)" "'uris': <\['$URI/a.txt'\]>" \
    "400 key enter; 440 key enter; 520 list"
portal_check "Salvar por cima: Esc no aviso volta ao nome (sem resposta)" SEM_RESPOSTA \
    "400 key enter; 440 key esc; 520 list" 'janela "Escolher"'
pedido SaveFiles "{'files': <[b'um.png', b'dois.png']>, 'current_folder': <b'$PASTA'>}"
portal_check "Salvar vários: a pasta escolhida com cada nome" \
    "'uris': <\['$URI/Sub/um.png', '$URI/Sub/dois.png'\]>" "400 click 583 237; 470 click 923 565; 520 list"
pedido OpenFile "{$ABRIR}" 4
portal_check "Close do pedido fecha o seletor (resposta 1)" "uint32 1" "700 list" "seletor: pedido fechado pelo app"
if sed 's/\x1b\[[0-9;]*m//g' "$OUT/mide.log" | grep -q 'janela "Escolher"'; then
    echo "FALHA Close: a janela do seletor continuou aberta"; FAIL=$((FAIL + 1))
else
    echo "ok    Close: a janela do seletor sumiu"; PASS=$((PASS + 1))
fi

# De ponta a ponta, com o xdg-desktop-portal de verdade (o da distribuição), num
# barramento sem ativação de serviços (nada do sistema sobe sozinho: nem o portal de
# documentos nem o do GTK). Um cliente que fica no barramento (como um app) pede
# org.freedesktop.portal.FileChooser.OpenFile e espera o sinal Response. O portal acha
# o mide.portal pela pasta de links (XDG_DESKTOP_PORTAL_DIR), como na sessão.
XDP=/usr/libexec/xdg-desktop-portal
if [ -x "$XDP" ] && python3 -c 'import gi' 2>/dev/null; then
    mkdir -p "$PDIR/portals" "$PDIR/services"
    ln -s "$PWD/data/xdg-desktop-portal/portals/mide.portal" "$PDIR/portals/"
    for f in /usr/share/xdg-desktop-portal/portals/*.portal; do [ -e "$f" ] && ln -sf "$f" "$PDIR/portals/"; done
    cat >"$PDIR/bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><servicedir>$PDIR/services</servicedir>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>
EOF
    cat >"$PDIR/cliente.py" <<'PY'
import sys
from gi.repository import Gio, GLib
folder, out = sys.argv[1], sys.argv[2]
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
loop = GLib.MainLoop()
sender = bus.get_unique_name()[1:].replace(".", "_")
handle = f"/org/freedesktop/portal/desktop/request/{sender}/t9"
def on_response(conn, name, path, iface, signal, params):
    with open(out, "w") as f:
        f.write(str(params.unpack()) + "\n")
    loop.quit()
bus.signal_subscribe("org.freedesktop.portal.Desktop", "org.freedesktop.portal.Request", "Response", handle, None, Gio.DBusSignalFlags.NO_MATCH_RULE, on_response)
opts = {"handle_token": GLib.Variant("s", "t9"), "current_folder": GLib.Variant("ay", folder.encode() + b"\0")}
bus.call_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser", "OpenFile",
              GLib.Variant("(ssa{sv})", ("", "Abrir pelo portal", opts)), None, Gio.DBusCallFlags.NONE, -1, None)
GLib.timeout_add_seconds(60, loop.quit)
loop.run()
PY
    cat >"$PDIR/app.sh" <<EOF
env XDG_CURRENT_DESKTOP=mide XDG_DATA_DIRS=$PWD/data:/usr/share XDG_DESKTOP_PORTAL_DIR=$PDIR/portals $XDP -r >/dev/null 2>&1 &
sleep 2
$ESPERA_DOCK
python3 -I '$PDIR/cliente.py' '$PASTA' '$RESPOSTA'
EOF
    : >"$RESPOSTA"
    MIDE_PORTAL_FORCE=1 SCENARIO_GRACE=3 timeout 150 dbus-run-session --config-file="$PDIR/bus.conf" -- tests/scenario.sh \
        "600 list; 620 click 585 275; 660 click 923 565; 720 list" -- "sh '$PDIR/app.sh'" >/dev/null 2>&1
    if grep -q "^(0, {'uris': \['$URI/a.txt'\]})" "$RESPOSTA" && sed 's/\x1b\[[0-9;]*m//g' "$OUT/mide.log" | grep -q 'janela "Abrir pelo portal"'; then
        echo "ok    xdg-desktop-portal de verdade usa o seletor do mide (Response 0 com o arquivo)"; PASS=$((PASS + 1))
    else
        echo "FALHA xdg-desktop-portal de verdade usa o seletor do mide"; sed 's/^/      resposta: /' "$RESPOSTA"; FAIL=$((FAIL + 1))
    fi
else
    echo "pulo  xdg-desktop-portal de verdade (sem $XDP ou sem o python3-gi)"
fi
rm -rf "$PDIR"
