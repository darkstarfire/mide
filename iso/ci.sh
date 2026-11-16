#!/bin/bash
# Roda dentro de um contêiner debian:trixie com --privileged (o CI do GitHub chama):
# gera os pacotes .deb (packaging/ci-debs.sh) e a ISO do mios.
set -euo pipefail
cd "$(dirname "$0")/.."
packaging/ci-debs.sh
iso/build.sh
