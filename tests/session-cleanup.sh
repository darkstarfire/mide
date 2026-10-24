#!/bin/bash
# Testa o fim do `data/mide-session` sem tela e sem mexer no systemd de verdade: o
# script roda numa pasta temporária com um "mide" falso ao lado e systemctl, pkill,
# dconf e dbus-run-session falsos no PATH (cada um anota o que recebeu).
#
#   1. o mide sai sozinho: o código de saída dele passa adiante e a limpeza roda;
#   2. a tela de login manda SIGTERM ao script: o mide recebe o sinal, fecha, e a
#      limpeza roda;
#   3. sem barramento de sessão (`dbus-run-session`): a limpeza roda também.
set -u
cd "$(dirname "$0")/.."
root="$(mktemp -d "${TMPDIR:-/tmp}/mide-session-cleanup.XXXXXX")"
trap 'pkill -KILL -f "^/bin/sh $root/bin/mide\$"; rm -rf "$root"' EXIT
fails=0
ok() { echo "  ok: $1"; }
falha() { echo "  FALHOU: $1"; fails=$((fails + 1)); }

mkdir -p "$root/bin" "$root/fake" "$root/home"
cp data/mide-session "$root/bin/mide-session"
log="$root/chamadas"
for prog in systemctl pkill dconf; do
    printf '#!/bin/sh\necho "%s $*" >> "%s"\n[ "%s" = dconf ] && exit 1\nexit 0\n' "$prog" "$log" "$prog" > "$root/fake/$prog"
done
printf '#!/bin/sh\necho "dbus-run-session $*" >> "%s"\nshift\nDBUS_SESSION_BUS_ADDRESS=unix:path=/nao/existe exec "$@"\n' "$log" > "$root/fake/dbus-run-session"
# O "mide": anota o início, sai com $FAKE_STATUS ou espera um SIGTERM.
cat > "$root/bin/mide" <<EOF
#!/bin/sh
echo "mide inicio" >> "$log"
trap 'echo "mide TERM" >> "$log"; exit 0' TERM
if [ -n "\${FAKE_STATUS:-}" ]; then exit "\$FAKE_STATUS"; fi
while :; do sleep 0.1; done
EOF
chmod +x "$root"/fake/* "$root/bin/mide" "$root/bin/mide-session"

# Com `exec` (dentro de um subshell), o processo de fundo é o próprio mide-session:
# o sinal do teste chega nele.
run_env() {
    exec env -i PATH="$root/fake:/usr/bin:/bin" HOME="$root/home" XDG_CONFIG_HOME="$root/home/.config" \
        XDG_STATE_HOME="$root/home/.local/state" XDG_DATA_HOME="$root/home/.local/share" "$@"
}
limpou() {
    grep -q "systemctl --user unset-environment XDG_DESKTOP_PORTAL_DIR" "$log" \
        && grep -q "systemctl --user --no-block stop xdg-desktop-portal.service" "$log" \
        && grep -q "pkill -TERM -u $(id -u) -f \[m\]ide-portal --portal-only" "$log"
}

echo "1. o mide sai sozinho"
: > "$log"
(run_env DBUS_SESSION_BUS_ADDRESS=unix:path=/nao/existe FAKE_STATUS=3 "$root/bin/mide-session")
status=$?
[ "$status" = 3 ] && ok "código de saída do mide (3)" || falha "código de saída $status (esperado 3)"
limpou && ok "limpeza rodou" || { falha "limpeza não rodou"; cat "$log"; }

echo "2. SIGTERM da tela de login"
: > "$log"
(run_env DBUS_SESSION_BUS_ADDRESS=unix:path=/nao/existe "$root/bin/mide-session") &
spid=$!
for _ in $(seq 50); do grep -q "mide inicio" "$log" 2>/dev/null && break; sleep 0.1; done
kill -TERM "$spid"
for _ in $(seq 50); do kill -0 "$spid" 2>/dev/null || break; sleep 0.1; done
if kill -0 "$spid" 2>/dev/null; then
    falha "o script não saiu depois do SIGTERM"
    kill -KILL "$spid"
fi
grep -q "mide TERM" "$log" && ok "o mide recebeu o SIGTERM" || falha "o mide não recebeu o SIGTERM"
limpou && ok "limpeza rodou depois do sinal" || { falha "limpeza não rodou depois do sinal"; cat "$log"; }
pgrep -f "^/bin/sh $root/bin/mide\$" >/dev/null && falha "o mide ficou órfão" || ok "nenhum mide órfão"

echo "3. sem barramento de sessão"
: > "$log"
(run_env FAKE_STATUS=0 "$root/bin/mide-session")
grep -q "dbus-run-session -- $root/bin/mide-session" "$log" && ok "abriu um barramento próprio" || falha "não usou o dbus-run-session"
limpou && ok "limpeza rodou dentro do dbus-run-session" || { falha "limpeza não rodou"; cat "$log"; }

if [ "$fails" -gt 0 ]; then
    echo "session-cleanup: $fails falha(s)"
    exit 1
fi
echo "session-cleanup: tudo certo"
