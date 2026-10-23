# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== senha de administrador: mide-askpass, mide-sudo e agente do polkit (C7)"
# Nada aqui usa o sudo, o pkexec nem o polkit de verdade, e nenhuma senha de verdade:
# o mide-sudo usa o sudo falso (tests/fake/sudo, por MIDE_SUDO_BIN), e o agente usa um
# polkitd falso (examples/polkit-fake.rs) num barramento de teste e o helper falso
# (tests/fake/polkit-agent-helper). As duas variáveis só valem em compilação de
# desenvolvimento. A senha de teste não pode aparecer em nenhum log (conferido no fim).
PW=teste-c7-123
C7="$(mktemp -d)"
# Roteiros gerados pela seção (fora de $C7, que é limpa a cada cenário).
C7S="$(mktemp -d)"
OUT="${SCENARIO_OUT:-/tmp/mide-scenario}"
BIN="$PWD/target/debug"
FAKE_SUDO="$PWD/tests/fake/sudo"
FAKE_HELPER="$PWD/tests/fake/polkit-agent-helper"
FAKE_POLKITD="$BIN/examples/polkit-fake"
# Só o xhost falso no PATH dos cenários do mide-sudo (nunca mexe numa tela de verdade).
mkdir -p "$C7S/xbin" && ln -sf "$PWD/tests/fake/xhost" "$C7S/xbin/xhost"
XHOST_ENV="PATH='$C7S/xbin':\$PATH FAKE_XHOST_STATE='$C7/xhost-root' FAKE_XHOST_LOG='$C7/xhost.log'"

c7_result() {
    local name="$1"
    shift
    if "$@"; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name"; FAIL=$((FAIL + 1))
        for f in "$C7"/*.err "$C7"/*.log "$C7"/*.out; do
            [ -s "$f" ] && sed "s|^|      $(basename "$f"): |" "$f" | grep -v "$PW" | tail -4
        done
    fi
}

c7_reset() { rm -rf "${C7:?}"/*; }
mide_log() { sed 's/\x1b\[[0-9;]*m//g' "$OUT/mide.log"; }
auth_windows() { mide_log | grep -c 'autoteste: janela "Senha de administrador"'; }

# --- mide-askpass sozinho (modo SUDO_ASKPASS) -------------------------------------
askpass_app() {
    echo "MIDE_ELEVATE_TITLE=XAMPP MIDE_ELEVATE_COMMAND=/opt/lampp/manager-linux-x64.run MIDE_ASKPASS_STATE='$C7/estado' '$BIN/mide-askpass' '[sudo] password for $USER: ' >'$C7/askpass.senha' 2>'$C7/askpass.err'; echo \$? >'$C7/askpass.code'"
}
c7_reset; : >"$C7/estado"; chmod 600 "$C7/estado"
timeout 150 "$SCEN" "500 list; 520 type $PW; 522 key enter; 600 list" -- "$(askpass_app)" >/dev/null 2>&1
c7_result "askpass: janela no meio da tela, por cima e com foco" \
    sh -c "sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -q 'diálogo mide-auth no meio da tela'"
c7_result "askpass: senha certa sai na saída padrão (para o sudo) e código 0" \
    sh -c "[ \"\$(cat '$C7/askpass.senha')\" = '$PW' ] && [ \"\$(cat '$C7/askpass.code')\" = 0 ]"
c7_result "askpass: conta a tentativa no arquivo de estado" grep -qx "1" "$C7/estado"
c7_result "askpass: a janela fecha depois do OK" test "$(auth_windows)" = 1

c7_reset; : >"$C7/estado"; chmod 600 "$C7/estado"
timeout 150 "$SCEN" "500 type abc; 520 key esc; 600 list" -- "$(askpass_app)" >/dev/null 2>&1
c7_result "askpass: Esc cancela (código 1, nada na saída, estado diz cancel)" \
    sh -c "[ \"\$(cat '$C7/askpass.code')\" = 1 ] && [ ! -s '$C7/askpass.senha' ] && grep -q cancel '$C7/estado'"

c7_reset; printf '1\n' >"$C7/estado"; chmod 600 "$C7/estado"
timeout 150 "$SCEN" "500 key esc; 560 list" -- "$(askpass_app)" >/dev/null 2>&1
c7_result "askpass: da segunda chamada do sudo em diante, abre com \"Senha incorreta\"" \
    grep -q "tentativa 2: a anterior foi recusada" "$C7/askpass.err"

# --- mide-sudo com o sudo falso ----------------------------------------------------
# O comando grava o próprio ambiente: o que o programa "root" recebe.
sudo_app() {
    local extra="${1:-}"
    echo "env DISPLAY=:987 LANG=pt_BR.UTF-8 LC_MEASUREMENT=../../tmp/x $XHOST_ENV MIDE_SUDO_BIN='$FAKE_SUDO' MIDE_TEST_SUDO_PASSWORD='$PW' FAKE_SUDO_LOG='$C7/sudo.log' $extra '$BIN/mide-sudo' --name XAMPP sh -c 'env >\"$C7/ambiente-root\"' >'$C7/sudo.out' 2>'$C7/sudo.err' </dev/null; echo \$? >'$C7/sudo.code'"
}
c7_reset
timeout 150 "$SCEN" "500 type $PW; 502 key enter; 600 list" -- "$(sudo_app)" >/dev/null 2>&1
c7_result "mide-sudo: senha certa roda o comando (código 0, uma tentativa)" \
    sh -c "[ \"\$(cat '$C7/sudo.code')\" = 0 ] && [ -s '$C7/ambiente-root' ] && grep -q 'tentativas=1 modo=env askpass=1' '$C7/sudo.log'"
c7_result "mide-sudo: o programa recebe a tela X11, o idioma e o X11 forçado" \
    sh -c "grep -qx 'DISPLAY=:987' '$C7/ambiente-root' && grep -qx 'GDK_BACKEND=x11' '$C7/ambiente-root' && grep -qx 'QT_QPA_PLATFORM=xcb' '$C7/ambiente-root' && grep -qx 'LANG=pt_BR.UTF-8' '$C7/ambiente-root'"
c7_result "mide-sudo: nada do Wayland, da pasta de execução nem da sessão D-Bus vai para o root" \
    sh -c "! grep -qE '^(WAYLAND_DISPLAY|XDG_RUNTIME_DIR|DBUS_SESSION_BUS_ADDRESS|SUDO_ASKPASS|MIDE_[A-Z_]*)=|^LANGUAGE=en\$' '$C7/ambiente-root'"
c7_result "mide-sudo: idioma com caminho (LC_MEASUREMENT=../..) não vai para o root" sh -c "! grep -q '^LC_MEASUREMENT=' '$C7/ambiente-root'"
c7_result "mide-sudo: libera a tela X11 para o root só enquanto roda (xhost falso)" \
    sh -c "[ \"\$(tr '\n' ' ' <'$C7/xhost.log')\" = '+root -root ' ] && [ ! -e '$C7/xhost-root' ] && ! ls \"\${XDG_RUNTIME_DIR:-/run/user/\$(id -u)}\"/mide-sudo-x11-987-* >/dev/null 2>&1"
c7_result "mide-sudo: a janela mostra quem pede (--name)" grep -q "pedindo a senha de administrador para: sh -c" "$C7/sudo.err"
c7_result "mide-sudo: o arquivo de estado é apagado no fim" sh -c "! ls \"\${XDG_RUNTIME_DIR:-/run/user/\$(id -u)}\"/mide-askpass-* >/dev/null 2>&1"

c7_reset
timeout 150 "$SCEN" "500 type errada; 502 key enter; 640 type $PW; 642 key enter; 740 list" -- "$(sudo_app)" >/dev/null 2>&1
c7_result "mide-sudo: senha errada pede de novo, com \"Senha incorreta\", e a certa roda" \
    sh -c "grep -q 'tentativa 2: a anterior foi recusada' '$C7/sudo.err' && grep -q 'tentativas=2 modo=env' '$C7/sudo.log' && [ \"\$(cat '$C7/sudo.code')\" = 0 ]"

c7_reset
timeout 150 "$SCEN" "500 type errada; 502 key enter; 640 type errada; 642 key enter; 780 type errada; 782 key enter; 900 list" -- "$(sudo_app)" >/dev/null 2>&1
c7_result "mide-sudo: três senhas erradas saem com erro e aviso nas notificações" \
    sh -c "[ \"\$(cat '$C7/sudo.code')\" = 1 ] && [ ! -e '$C7/ambiente-root' ] && grep -q 'notificação (teste): Senha de administrador: Senha incorreta' '$C7/sudo.err'"

c7_reset
timeout 150 "$SCEN" "500 key esc; 600 list" -- "$(sudo_app)" >/dev/null 2>&1
c7_result "mide-sudo: cancelar sai com erro, sem rodar e sem aviso" \
    sh -c "[ \"\$(cat '$C7/sudo.code')\" = 1 ] && [ ! -e '$C7/ambiente-root' ] && grep -q 'cancelado na janela de senha' '$C7/sudo.err' && ! grep -q 'notificação (teste)' '$C7/sudo.err'"

c7_reset
timeout 150 "$SCEN" "500 type $PW; 502 key enter; 640 list" -- "$(sudo_app "FAKE_SUDO_NO_ENV=1 FAKE_SUDO_CACHE='$C7/credencial'")" >/dev/null 2>&1
c7_result "mide-sudo: sudoers sem o env: segunda tentativa pelo --preserve-env, sem pedir de novo como erro" \
    sh -c "grep -q 'modo=preserve-env' '$C7/sudo.log' && grep -qx 'DISPLAY=:987' '$C7/ambiente-root' && grep -qx 'GDK_BACKEND=x11' '$C7/ambiente-root' && [ \"\$(cat '$C7/sudo.code')\" = 0 ] && ! grep -q 'tentativa 2' '$C7/sudo.err'"

# Encerrado (TERM) com a janela aberta: repassa ao grupo do sudo, a janela fecha, a
# liberação da tela X11 é desfeita e o arquivo de estado apagado. (O código de saída é o
# do sudo: o sudo falso, um sh esperando o askpass, sai com 1, não com 143.)
c7_reset
timeout 150 "$SCEN" "1000 list" -- \
    "env DISPLAY=:987 $XHOST_ENV MIDE_SUDO_BIN='$FAKE_SUDO' MIDE_TEST_SUDO_PASSWORD='$PW' FAKE_SUDO_LOG='$C7/sudo.log' '$BIN/mide-sudo' --name XAMPP sh -c 'env >\"$C7/ambiente-root\"' >'$C7/sudo.out' 2>'$C7/sudo.err' </dev/null & p=\$!; sleep 5; kill -TERM \$p; wait \$p; echo \$? >'$C7/sudo.code'" >/dev/null 2>&1
c7_result "mide-sudo encerrado (TERM): a janela fecha, a tela X11 volta a ser fechada e o estado some" \
    sh -c "[ \"\$(cat '$C7/sudo.code')\" != 0 ] && [ ! -e '$C7/ambiente-root' ] && [ \"\$(tr '\n' ' ' <'$C7/xhost.log')\" = '+root -root ' ] && grep -q 'sinal 15 recebido' '$C7/sudo.err' && [ \"\$(sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -c 'janela \"Senha de administrador\"')\" = 0 ] && ! ls \"\${XDG_RUNTIME_DIR:-/run/user/\$(id -u)}\"/mide-askpass-* >/dev/null 2>&1"

# Num terminal (entrada é um pseudo-terminal), é o sudo de sempre: sem janela.
c7_reset
timeout 150 "$SCEN" "600 list" -- \
    "printf '%s\n' '$PW' | MIDE_SUDO_BIN='$FAKE_SUDO' MIDE_TEST_SUDO_PASSWORD='$PW' FAKE_SUDO_LOG='$C7/sudo.log' script -qec \"'$BIN/mide-sudo' sh -c 'echo rodou >$C7/terminal.out'\" /dev/null >/dev/null 2>&1" >/dev/null 2>&1
c7_result "mide-sudo dentro de um terminal: sudo normal, sem janela" \
    sh -c "grep -q rodou '$C7/terminal.out' && grep -q 'askpass=0' '$C7/sudo.log' && [ \"\$(sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -c 'janela \"Senha de administrador\"')\" = 0 ]"

# --- agente do polkit com o polkitd falso --------------------------------------------
# O roteiro abre um barramento de teste (dbus-run-session), o polkitd falso e o agente.
cat >"$C7S/c7-polkit.sh" <<EOF
#!/bin/sh
# \$1: arquivo de saída do polkitd falso; o resto: passos dele.
out="\$1"; shift
"$FAKE_POLKITD" "\$out" "\$@" >/dev/null 2>&1 &
fake=\$!
sleep 0.5
MIDE_POLKIT_BUS="\$DBUS_SESSION_BUS_ADDRESS" MIDE_POLKIT_HELPER="$FAKE_HELPER" MIDE_TEST_POLKIT_PASSWORD="$PW" \
    FAKE_HELPER_LOG="$C7/helper.log" XDG_SESSION_ID=teste-c7 "$BIN/mide-polkit-agent" >"$C7/agente.err" 2>&1 &
wait \$fake
EOF
chmod +x "$C7S/c7-polkit.sh"
polkit_app() { echo "dbus-run-session -- '$C7S/c7-polkit.sh' '$C7/polkitd.out' $*"; }

c7_reset
timeout 150 "$SCEN" "500 type $PW; 502 key enter; 640 list" -- "$(polkit_app begin c1)" >/dev/null 2>&1
c7_result "polkit: o agente se registra na sessão, com o caminho do mide" \
    grep -q "registrado tipo=unix-session sessão=teste-c7 caminho=/org/mide/PolicyKit1/AuthenticationAgent idioma=true fallback=Some(false)" "$C7/polkitd.out"
c7_result "polkit: senha certa na primeira: autenticação aceita e a janela fecha" \
    sh -c "grep -qx 'c1: ok' '$C7/polkitd.out' && grep -q \"usuário=\$(id -un) cookie=c1\" '$C7/helper.log' && [ \"\$(sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -c 'janela \"Senha de administrador\"')\" = 0 ]"
c7_result "polkit: a janela mostra o comando do pkexec" grep -q "pedido de autenticação: org.mide.teste.rodar" "$C7/agente.err"

c7_reset
timeout 150 "$SCEN" "500 type errada; 502 key enter; 620 list; 660 type $PW; 662 key enter; 760 list" -- "$(polkit_app begin c2)" >/dev/null 2>&1
c7_result "polkit: senha errada mantém a janela aberta e a certa é aceita" \
    sh -c "grep -qx 'c2: ok' '$C7/polkitd.out' && [ \"\$(grep -c resultado= '$C7/helper.log')\" = 2 ] && grep -q recusada '$C7/helper.log' && [ \"\$(sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -c 'janela \"Senha de administrador\"')\" = 1 ]"

c7_reset
timeout 150 "$SCEN" "500 key esc; 600 list" -- "$(polkit_app begin c3)" >/dev/null 2>&1
c7_result "polkit: cancelar na janela responde Cancelled" grep -qx "c3: erro org.freedesktop.PolicyKit1.Error.Cancelled" "$C7/polkitd.out"

c7_reset
timeout 150 "$SCEN" "700 list" -- "$(polkit_app cancel c4 3000)" >/dev/null 2>&1
c7_result "polkit: CancelAuthentication do polkit fecha a janela e responde Cancelled" \
    sh -c "grep -qx 'c4: erro org.freedesktop.PolicyKit1.Error.Cancelled' '$C7/polkitd.out' && [ \"\$(sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -c 'janela \"Senha de administrador\"')\" = 0 ]"

c7_reset
timeout 150 "$SCEN" "500 type errada; 502 key enter; 620 type errada; 622 key enter; 740 type errada; 742 key enter; 860 list" -- "$(polkit_app begin c5)" >/dev/null 2>&1
c7_result "polkit: três senhas erradas respondem Failed e fecham a janela" \
    sh -c "grep -qx 'c5: erro org.freedesktop.PolicyKit1.Error.Failed' '$C7/polkitd.out' && [ \"\$(grep -c recusada '$C7/helper.log')\" = 3 ]"

# O compositor inicia o agente (aqui forçado, com o barramento de teste); sem a sessão
# do mide nem o barramento de teste, não inicia.
c7_reset
BUS="$(dbus-daemon --session --fork --print-address=1 --print-pid=1 2>/dev/null)"
BUS_ADDR="$(echo "$BUS" | head -1)"
BUS_PID="$(echo "$BUS" | sed -n 2p)"
DBUS_SESSION_BUS_ADDRESS="$BUS_ADDR" "$FAKE_POLKITD" "$C7/polkitd.out" begin c6 >/dev/null 2>&1 &
FAKE_PID=$!
MIDE_POLKIT_AGENT_FORCE=1 MIDE_POLKIT_BUS="$BUS_ADDR" MIDE_POLKIT_HELPER="$FAKE_HELPER" MIDE_TEST_POLKIT_PASSWORD="$PW" \
    FAKE_HELPER_LOG="$C7/helper.log" XDG_SESSION_ID=teste-c7 timeout 150 "$SCEN" "500 type $PW; 502 key enter; 640 list" >/dev/null 2>&1
kill "$FAKE_PID" 2>/dev/null
[ -n "$BUS_PID" ] && kill "$BUS_PID" 2>/dev/null
c7_result "polkit: o compositor inicia o agente na sessão e ele atende o pedido" \
    sh -c "sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -q 'mide-polkit-agent iniciado' && grep -qx 'c6: ok' '$C7/polkitd.out'"
timeout 150 "$SCEN" "300 list" >/dev/null 2>&1
c7_result "polkit: fora da sessão do mide, o agente não é iniciado" sh -c "! sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -q 'mide-polkit-agent iniciado'"

# Só o polkitd pode pedir ou cancelar: outro processo no barramento é recusado.
c7_reset
timeout 150 "$SCEN" "500 type $PW; 502 key enter; 640 list" -- "$(polkit_app intruso c8 intruso-cancel c9 1500)" >/dev/null 2>&1
c7_result "polkit: pedido de quem não é o polkit é recusado, sem abrir janela nem o helper" \
    sh -c "grep -qx 'intruso c8: erro org.freedesktop.PolicyKit1.Error.NotAuthorized' '$C7/polkitd.out' && grep -q 'não veio do polkit' '$C7/agente.err' && ! grep -q 'cookie=c8' '$C7/helper.log'"
c7_result "polkit: cancelamento de quem não é o polkit é recusado e o pedido segue" \
    sh -c "grep -qx 'intruso c9-cancelamento: erro org.freedesktop.PolicyKit1.Error.NotAuthorized' '$C7/polkitd.out' && grep -qx 'c9: ok' '$C7/polkitd.out'"

# --- a senha de teste em nenhum log -------------------------------------------------
# Rodar de novo um caso com senha certa e procurar a senha em tudo o que foi gravado,
# menos a saída do askpass (que é a resposta para o sudo).
c7_reset
timeout 150 "$SCEN" "500 type $PW; 502 key enter; 600 list" -- "$(sudo_app)" >/dev/null 2>&1
timeout 150 "$SCEN" "500 type $PW; 502 key enter; 640 list" -- "$(polkit_app begin c7)" >/dev/null 2>&1
c7_result "a senha não aparece em nenhum log (compositor, mide-sudo, sudo, agente, helper)" \
    sh -c "! grep -rl -- '$PW' '$OUT/mide.log' '$C7' 2>/dev/null | grep -v '\.senha\$' | grep -q ."
rm -rf "$C7" "$C7S"
