#!/bin/bash
# Publica a ISO de target/iso nas releases do GitHub (o CI chama, com GH_TOKEN):
#   - tag mios-*: numa release própria com o nome da tag, que fica;
#   - o resto (tags v*, ou à mão): na release "iso", com o arquivo trocado pelo novo, para
#     o endereço de download não mudar.
#
# Uso: iso/publish-release.sh   (variáveis do Actions: GITHUB_REPOSITORY, GITHUB_REF_TYPE,
#                                GITHUB_REF_NAME, GITHUB_SHA)
set -euo pipefail
cd "$(dirname "$0")/.."

repo="${GITHUB_REPOSITORY:-fabioaacarneiro/mide}"
iso="$(ls -t target/iso/mios-*-amd64.iso | head -1)"
nome="$(basename "$iso")"
soma="$iso.sha256"
[ -f "$soma" ] || (cd target/iso && sha256sum "$nome" >"$nome.sha256")
# O GitHub não aceita arquivos de mais de 2 GiB numa release.
tamanho="$(stat -c %s "$iso")"
if [ "$tamanho" -ge $((2 * 1024 * 1024 * 1024)) ]; then
    echo "the ISO has $tamanho bytes: GitHub releases take at most 2 GiB per file" >&2
    exit 1
fi

mide="$(sed -nE 's/^version = "([^"]+)"/\1/p' Cargo.toml | head -1)"
mios="$(cat iso/VERSION)"
commit="${GITHUB_SHA:-$(git rev-parse HEAD)}"
data="$(date -u +%Y-%m-%d)"
notas="$(mktemp)"
cat >"$notas" <<EOF
mios $mios (Mac Inspired Operational System): a live system with the mide desktop and the
installer, on top of Debian 13. Built on $data from mide $mide (commit ${commit:0:7}).

- Download: \`$nome\` ($(numfmt --to=iec --suffix=B "$tamanho")) and check it with \`$nome.sha256\`
  (\`sha256sum -c $nome.sha256\`).
- Write it to a USB stick with the mide Disks app, or
  \`sudo dd if=$nome of=/dev/sdX bs=4M status=progress oflag=sync\` (erases the stick).
- Try it in a virtual machine: \`./install.sh iso-test\` in the mide project.
- Boots in UEFI and legacy BIOS. The live user is \`mios\` (no password).

SHA-256: \`$(cut -d' ' -f1 "$soma")\`
EOF

if [ "${GITHUB_REF_TYPE:-}" = tag ] && [[ "${GITHUB_REF_NAME:-}" == mios-* ]]; then
    tag="$GITHUB_REF_NAME"
    gh release create "$tag" --repo "$repo" --title "mios $mios ($tag)" --notes-file "$notas" \
        --verify-tag --latest=false "$iso" "$soma"
    echo "published on the $tag release"
    exit 0
fi

tag=iso
if ! gh release view "$tag" --repo "$repo" >/dev/null 2>&1; then
    gh release create "$tag" --repo "$repo" --target "$commit" --title "mios ISO (latest build)" \
        --notes-file "$notas" --latest=false
fi
# ISOs com outro nome (outra versão do mios) saem; a de mesmo nome é trocada.
for antigo in $(gh release view "$tag" --repo "$repo" --json assets -q '.assets[].name'); do
    case "$antigo" in "$nome" | "$nome.sha256") ;; *) gh release delete-asset "$tag" "$antigo" --repo "$repo" --yes ;; esac
done
gh release upload "$tag" --repo "$repo" --clobber "$iso" "$soma"
gh release edit "$tag" --repo "$repo" --notes-file "$notas"
echo "published on the $tag release: https://github.com/$repo/releases/download/$tag/$nome"
