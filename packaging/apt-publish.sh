#!/bin/bash
# Monta e assina o repositório apt "plano" do mios a partir dos .deb de uma pasta e,
# com --upload, publica tudo na release "apt" do GitHub (o CI chama, a cada versão).
#
# Uso: packaging/apt-publish.sh DIR [--upload]
#   DIR tem os .deb; ali ficam também Packages, Packages.gz, Release, InRelease e
#   Release.gpg. A chave de assinatura vem do GPG de quem roda (no CI, importada do
#   segredo MIOS_APT_GPG_KEY); a impressão digital esperada está em
#   data/mios/apt/FINGERPRINT.
#
# Nas máquinas: deb [signed-by=/usr/share/keyrings/mios-archive-keyring.gpg]
#   https://github.com/fabioaacarneiro/mide/releases/download/apt/ ./
set -euo pipefail

dir="$1"
upload="${2:-}"
cd "$(dirname "$0")/.."
chave="$(cat data/mios/apt/FINGERPRINT)"

cd "$dir"
# O Release não pode estar na pasta enquanto o apt-ftparchive a varre (ele se listaria).
rm -f Release InRelease Release.gpg
tmp="$(mktemp)"
dpkg-scanpackages --multiversion . /dev/null >Packages
gzip -9nkf Packages
apt-ftparchive \
    -o APT::FTPArchive::Release::Origin=mios \
    -o APT::FTPArchive::Release::Label=mios \
    -o APT::FTPArchive::Release::Suite=stable \
    -o APT::FTPArchive::Release::Description="mios packages (mide)" \
    release . >"$tmp"
mv "$tmp" Release
chmod 644 Release Packages Packages.gz
gpg --batch --yes --local-user "$chave" --clearsign -o InRelease Release
gpg --batch --yes --local-user "$chave" -abs -o Release.gpg Release
chmod 644 InRelease Release.gpg
echo "repositório apt: $(grep -c '^Package:' Packages) pacote(s) em $dir"

if [ "$upload" = --upload ]; then
    repo="${GITHUB_REPOSITORY:-fabioaacarneiro/mide}"
    # A release "apt" existe uma vez só; os arquivos dela são trocados a cada versão.
    if ! gh release view apt --repo "$repo" >/dev/null 2>&1; then
        gh release create apt --repo "$repo" --title "apt repository" --latest=false \
            --notes "Repositório apt do mios (as atualizações do mide). Não baixe à mão: o mios já vem com ele configurado."
    fi
    # Tira os .deb antigos que não estão mais no índice.
    for antigo in $(gh release view apt --repo "$repo" --json assets -q '.assets[].name' | grep '\.deb$' || true); do
        [ -f "$antigo" ] || gh release delete-asset apt "$antigo" --repo "$repo" --yes
    done
    gh release upload apt --repo "$repo" --clobber ./*.deb Packages Packages.gz Release InRelease Release.gpg
fi
