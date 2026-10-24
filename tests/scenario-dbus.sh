#!/usr/bin/env bash
# tests/scenario.sh numa sessão D-Bus própria (dbus-run-session): o servidor de
# notificações do mide fica com o nome no barramento de teste, sem disputar com o
# do desktop onde o mide roda. Mesmos argumentos de tests/scenario.sh.
export MIDE_PRIVATE_BUS=1
exec dbus-run-session -- "$(dirname "$0")/scenario.sh" "$@"
