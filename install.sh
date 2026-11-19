#!/usr/bin/env bash
# Ferramenta de instalação do mide e do mios (C3 e C6): interativa, para Debian/Ubuntu,
# Fedora, Arch/Manjaro e Slackware. Instala as dependências pelo gerenciador de pacotes
# da distribuição, compila o mide (como o usuário, nunca como root), monta os arquivos
# numa pasta à parte e só então pergunta antes de copiar para o sistema. Também remove
# o que instalou, atualiza a sessão aberta, troca a tela de login e os temas do boot
# (GRUB e Plymouth), gera os pacotes .deb e a ISO do mios e liga a ISO numa máquina
# virtual.
#
#   ./install.sh                 menu com tudo
#   ./install.sh --install       instala direto (ainda pergunta a distribuição e as partes)
#   ./install.sh --remove        remove do sistema
#   ./install.sh build|deps|update|login|boot-theme|deb|iso|iso-test   (veja --help)
#   ./install.sh --help          todas as opções
#
# Onde fica cada coisa:
#   Debian/Ubuntu, Fedora, Arch: programas em /usr/local (fora do gerenciador de
#     pacotes), com a lista do que foi copiado em /var/lib/mide/installed-files (é
#     por ela que a remoção apaga só o que é do mide).
#   Slackware: um pacote de verdade (mide-VERSÃO-ARQ-1_mide.txz, com o makepkg), em
#     /usr, instalado com o upgradepkg e removido com o removepkg. As dependências que
#     não vêm no Slackware (seatd, xdg-desktop-portal-wlr, greetd) são compiladas pelos
#     SlackBuilds do SlackBuilds.org.
# As configurações de cada pessoa (~/.config/mide, ~/.local/state/mide) nunca são
# tocadas, nem na remoção.
set -euo pipefail

PROJETO="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
cd "$PROJETO"
VERSAO="$(sed -nE '0,/^version = "([^"]+)"/s//\1/p' Cargo.toml)"
ARQ="$(uname -m)"
RUST_MINIMO=1.85
MANIFESTO=/var/lib/mide/installed-files
SBO_VERSAO=15.0
SATELLITE_GIT=https://github.com/Supreeeme/xwayland-satellite
ICONES_GIT=https://github.com/fabioaacarneiro/mide-icons.git
# Repositório apt do mide/mios (pacotes prontos, assinados; o CI publica a cada versão).
APT_URL="${MIDE_APT_URL:-https://github.com/fabioaacarneiro/mide/releases/download/apt}"
APT_FONTE=/etc/apt/sources.list.d/mios.sources
APT_CHAVE=/usr/share/keyrings/mios-archive-keyring.gpg
# O que a instalação pelo apt pôs além do pacote (fonte do apt, chave, tampa), para remover.
APT_EXTRAS=/var/lib/mide/apt-extras

# ---------------------------------------------------------------------------- textos
# Inglês como base e português do Brasil quando o idioma da sessão é português (I1).
case "${LC_ALL:-${LC_MESSAGES:-${LANG:-}}}" in pt*) PT=1 ;; *) PT= ;; esac
t() { if [ -n "$PT" ]; then printf '%s' "$2"; else printf '%s' "$1"; fi; }

if [ -t 1 ]; then N=$'\e[1m'; A=$'\e[34m'; V=$'\e[32m'; R=$'\e[31m'; Z=$'\e[0m'; else N= A= V= R= Z=; fi
titulo() { printf '\n%s==> %s%s\n' "$N$A" "$*" "$Z"; }
info() { printf '    %s\n' "$*"; }
aviso() { printf '    %s%s%s\n' "$N" "$(t "warning: " "aviso: ")$*" "$Z" >&2; }
falha() { printf '%s%s%s\n' "$N$R" "$(t "error: " "erro: ")$*" "$Z" >&2; exit 1; }

# ---------------------------------------------------------------------------- opções
ACAO=
DISTRO=
SIM=           # --yes: aceita as respostas padrão sem perguntar
SIMULAR=       # --dry-run: só mostra os comandos que mudariam o sistema
DESTDIR=       # --destdir: instala numa pasta (testes e empacotadores), sem root
COMPILAR=1
DEPENDENCIAS=1
PREFIXO=
X11=
ICONES=
LOGIN=
ENERGIA=
BOOT=          # temas do boot (GRUB e Plymouth) do mios
FONTE=         # apt (pacote pronto do repositório do mide) ou codigo (compilar aqui)
TIRAR=         # "remove" depois de login/boot-theme
TESTE_BIOS=    # iso-test bios: modo legado em vez de UEFI
TESTE_DISCO=   # iso-test disk: liga só pelo disco virtual (o sistema instalado)
# Núcleos para compilar: metade da máquina e no máximo 4, com prioridade baixa (o
# computador continua usável durante a compilação); CARGO_BUILD_JOBS escolhe outro número.
JOBS="${CARGO_BUILD_JOBS:-$(( $(nproc 2>/dev/null || echo 2) / 2 ))}"
[ -n "${CARGO_BUILD_JOBS:-}" ] || { [ "$JOBS" -ge 1 ] || JOBS=1; [ "$JOBS" -le 4 ] || JOBS=4; }

ajuda() {
    cat <<EOF
$(t "mide installer" "Instalador do mide") $VERSAO

$(t "Usage" "Uso"): ./install.sh [$(t "options" "opções")]

  --install              $(t "install (skip the first menu)" "instalar (sem o primeiro menu)")
  --remove               $(t "remove the mide from the system" "remover o mide do sistema")
  --distro NAME          debian | fedora | arch | slackware ($(t "default: detected" "padrão: a detectada"))
  --yes, -y              $(t "accept the default answers" "aceitar as respostas padrão")
  --dry-run              $(t "only show what would be done" "só mostrar o que seria feito")
  --no-build             $(t "use the programs already in target/release" "usar os programas que já estão em target/release")
  --no-deps              $(t "do not install packages" "não instalar pacotes")
  --prefix DIR           $(t "default: /usr/local (Slackware: /usr)" "padrão: /usr/local (Slackware: /usr)")
  --destdir DIR          $(t "install into DIR instead of / (no root)" "instalar em DIR em vez de / (sem root)")
  --x11 | --no-x11       $(t "X11 apps (xwayland-satellite)" "apps X11 (xwayland-satellite)")
  --icons | --no-icons   $(t "mide icon theme" "tema de ícones do mide")
  --login | --no-login   $(t "mide login screen (greetd)" "tela de login do mide (greetd)")
  --power | --no-power   $(t "lid and power key handled by the mide" "tampa e botão de ligar com o mide")
  --boot-theme | --no-boot-theme   $(t "mios GRUB and Plymouth themes" "temas do GRUB e do Plymouth do mios")
  --apt | --source       $(t "Debian/Ubuntu: ready package from the mide apt repository (updates through Software Update) or build here" "Debian/Ubuntu: pacote pronto do repositório apt do mide (atualiza pela Atualização de Software) ou compilar aqui")

$(t "Commands (also in the menu)" "Comandos (também no menu)"):
  build                  $(t "only build (target/release)" "só compilar (target/release)")
  deps                   $(t "only install what is needed to build" "só instalar o que falta para compilar")
  update                 $(t "update the open mide session from this folder (developers)" "atualizar a sessão do mide aberta a partir desta pasta (desenvolvimento)")
  login [remove]         $(t "mide login screen (greetd), or go back to the previous one" "tela de login do mide (greetd), ou voltar à de antes")
  boot-theme [remove]    $(t "mios GRUB and Plymouth themes, or go back" "temas do GRUB e do Plymouth do mios, ou voltar")
  deb                    $(t ".deb packages of the mide and mios-release (target/deb)" "pacotes .deb do mide e do mios-release (target/deb)")
  iso                    $(t "mios ISO (target/iso)" "ISO do mios (target/iso)")
  iso-test [bios] [disk] $(t "start the ISO in a virtual machine (QEMU)" "ligar a ISO numa máquina virtual (QEMU)")

$(t "Builds use $JOBS cores (CARGO_BUILD_JOBS=N to change)." "As compilações usam $JOBS núcleos (CARGO_BUILD_JOBS=N para mudar).")
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --install | --instalar) ACAO=instalar ;;
        --remove | --uninstall | --remover) ACAO=remover ;;
        --distro) DISTRO="${2:-}"; shift ;;
        --yes | -y) SIM=1 ;;
        --dry-run | --simular) SIMULAR=1 ;;
        --no-build) COMPILAR= ;;
        --no-deps) DEPENDENCIAS= ;;
        --prefix) PREFIXO="${2:-}"; shift ;;
        --destdir) DESTDIR="$(readlink -m "${2:-}")"; shift ;;
        --x11) X11=s ;; --no-x11) X11=n ;;
        --icons) ICONES=s ;; --no-icons) ICONES=n ;;
        --login) LOGIN=s ;; --no-login) LOGIN=n ;;
        --power) ENERGIA=s ;; --no-power) ENERGIA=n ;;
        --boot-theme) BOOT=s ;; --no-boot-theme) BOOT=n ;;
        --apt) FONTE=apt ;; --source | --build-here) FONTE=codigo ;;
        build | compilar) ACAO=compilar ;;
        deps | dependencies | dependencias) ACAO=dependencias ;;
        update | atualizar) ACAO=sessao ;;
        login) ACAO=login ;;
        boot-theme | boot | grub | plymouth) ACAO=boot ;;
        deb | packages | pacotes) ACAO=pacotes ;;
        iso) ACAO=iso ;;
        iso-test | iso-teste) ACAO=iso-teste ;;
        remove | remover) if [ "$ACAO" = login ] || [ "$ACAO" = boot ]; then TIRAR=1; else ACAO=remover; fi ;;
        bios) TESTE_BIOS=1 ;;
        disk | disco) TESTE_DISCO=1 ;;
        help | ajuda | --help | -h) ajuda; exit 0 ;;
        *) ajuda >&2; falha "$(t "unknown option" "opção desconhecida"): $1" ;;
    esac
    shift
done
# Sem terminal, as respostas vêm da entrada (uma por linha); quando ela acaba, vale a
# resposta padrão de cada pergunta.

# ----------------------------------------------------------------------- perguntas
# pergunta "texto" padrão(s|n) → 0 para sim
pergunta() {
    local padrao="$2" resposta dica
    dica="$( [ "$padrao" = s ] && t "[Y/n]" "[S/n]" || t "[y/N]" "[s/N]")"
    if [ -n "$SIM" ]; then
        printf '    %s %s %s\n' "$1" "$dica" "$( [ "$padrao" = s ] && t yes sim || t no não)"
    else
        read -r -p "    $1 $dica " resposta || resposta=
        case "$resposta" in [sSyY]*) padrao=s ;; [nN]*) padrao=n ;; esac
    fi
    [ "$padrao" = s ]
}

# escolha "pergunta" padrão opção1 opção2... → número escolhido em $ESCOLHA
escolha() {
    local texto="$1" padrao="$2" i=1 resposta
    shift 2
    printf '\n%s%s%s\n' "$N" "$texto" "$Z"
    for opcao in "$@"; do printf '    %d) %s\n' "$i" "$opcao"; i=$((i + 1)); done
    while :; do
        if [ -n "$SIM" ]; then resposta="$padrao"; printf '    > %s\n' "$resposta"; else
            read -r -p "    $(t "Choose" "Escolha") [$padrao]: " resposta || resposta=
            resposta="${resposta:-$padrao}"
        fi
        if [[ "$resposta" =~ ^[0-9]+$ ]] && [ "$resposta" -ge 1 ] && [ "$resposta" -le $# ]; then
            ESCOLHA="$resposta"
            return
        fi
        printf '    %s\n' "$(t "Type a number from the list." "Digite um número da lista.")"
    done
}

# ----------------------------------------------------------------- quem faz o quê
# Os pacotes e a cópia para o sistema precisam de root; compilar não (o Rust e a
# pasta target/ ficam com o usuário). Funciona rodando como usuário (o sudo pede a
# senha uma vez) ou com sudo (a compilação volta para quem chamou o sudo).
if [ "$(id -u)" = 0 ]; then
    USUARIO="${SUDO_USER:-root}"
    UHOME="$(getent passwd "$USUARIO" | cut -d: -f6)"
else
    USUARIO="$(id -un)"
    UHOME="$HOME"
fi
UHOME="${UHOME:-$HOME}"

mostra() { printf '    %s$ %s%s\n' "$A" "$*" "$Z"; }

# Comando como root (sudo quando preciso). Em --dry-run, só mostra.
root() {
    if [ -n "$SIMULAR" ]; then mostra "$*"; return 0; fi
    if [ "$(id -u)" = 0 ]; then
        "$@"
    elif [ -n "${SUDO_ASKPASS:-}" ] && [ ! -t 0 ]; then
        # Sem terminal (o app Atualização de Software): a senha numa janela.
        sudo -A "$@"
    else
        sudo "$@"
    fi
}

# Comando como o usuário, com o Rust do usuário (~/.cargo/bin) no PATH.
usuario() {
    local caminho="$UHOME/.cargo/bin:$PATH"
    if [ "$(id -u)" = 0 ] && [ "$USUARIO" != root ]; then
        if command -v runuser >/dev/null; then
            runuser -u "$USUARIO" -- env HOME="$UHOME" USER="$USUARIO" PATH="$caminho" "$@"
        else
            sudo -u "$USUARIO" env HOME="$UHOME" USER="$USUARIO" PATH="$caminho" "$@"
        fi
    else
        env PATH="$caminho" "$@"
    fi
}

# Como `usuario`, mas em --dry-run só mostra (compilar, baixar).
usuario_muda() {
    if [ -n "$SIMULAR" ]; then mostra "$*"; return 0; fi
    usuario "$@"
}

# ---------------------------------------------------------------- distribuições
detectar_distro() {
    if [ -f /etc/slackware-version ]; then echo slackware; return; fi
    local id="" like=""
    if [ -f /etc/os-release ]; then
        id="$(. /etc/os-release && echo "${ID:-}")"
        like="$(. /etc/os-release && echo "${ID_LIKE:-}")"
    fi
    for d in $id $like; do
        case "$d" in
            debian | ubuntu | linuxmint | pop | zorin | elementary) echo debian; return ;;
            fedora | rhel | centos | rocky | almalinux | nobara) echo fedora; return ;;
            arch | manjaro | endeavouros | garuda | cachyos) echo arch; return ;;
            slackware) echo slackware; return ;;
        esac
    done
    echo ""
}

nome_distro() {
    case "$1" in
        debian) echo "Debian / Ubuntu (apt)" ;;
        fedora) echo "Fedora (dnf)" ;;
        arch) echo "Arch / Manjaro (pacman)" ;;
        slackware) echo "Slackware (slackpkg + SlackBuilds.org)" ;;
    esac
}

escolher_distro() {
    local detectada lista=(debian fedora arch slackware) rotulos=() padrao=1 i=1
    detectada="$(detectar_distro)"
    if [ -n "$DISTRO" ]; then
        case "$DISTRO" in debian | fedora | arch | slackware) ;; *) falha "$(t "unknown distribution" "distribuição desconhecida"): $DISTRO" ;; esac
        info "$(t "Distribution" "Distribuição"): $(nome_distro "$DISTRO")"
        return
    fi
    for d in "${lista[@]}"; do
        if [ "$d" = "$detectada" ]; then
            rotulos+=("$(nome_distro "$d")  ← $(t "detected" "detectada")")
            padrao=$i
        else
            rotulos+=("$(nome_distro "$d")")
        fi
        i=$((i + 1))
    done
    [ -n "$detectada" ] || aviso "$(t "could not detect the distribution; choose the closest one" "não deu para detectar a distribuição; escolha a mais parecida")"
    escolha "$(t "Which system is this?" "Qual é o sistema?")" "$padrao" "${rotulos[@]}"
    DISTRO="${lista[$((ESCOLHA - 1))]}"
}

# ------------------------------------------------------------------- pacotes
# Bibliotecas para compilar, programas que o mide usa ao rodar e as partes opcionais.
# Os nomes são os do README e do INSTALATION.md (conferidos no Debian 13).
pacotes_compilar() {
    case "$DISTRO" in
        debian) echo build-essential cmake git curl pkg-config libinput-dev libseat-dev libudev-dev libgbm-dev \
            libdrm-dev libxkbcommon-dev libpam0g-dev libwayland-dev libegl-dev libglib2.0-dev libgudev-1.0-dev ;;
        fedora) echo gcc gcc-c++ cmake git curl pkgconf-pkg-config libinput-devel libseat-devel systemd-devel mesa-libgbm-devel \
            libdrm-devel libxkbcommon-devel pam-devel wayland-devel mesa-libEGL-devel glib2-devel libgudev-devel ;;
        arch) echo base-devel cmake git curl libinput seatd mesa libdrm libxkbcommon pam wayland glib2 libgudev ;;
        # Instalação completa do Slackware: as bibliotecas já trazem os cabeçalhos.
        slackware) echo gcc make cmake pkg-config git curl libinput libxkbcommon mesa libdrm wayland glib2 libgudev \
            eudev elogind pam ;;
    esac
}

pacotes_rodar() {
    case "$DISTRO" in
        debian) echo xwayland xdg-desktop-portal xdg-desktop-portal-gtk xdg-desktop-portal-wlr pipewire wireplumber \
            ffmpeg curl adwaita-icon-theme udisks2 util-linux dbus-user-session x11-xserver-utils polkitd sudo \
            qt5-gtk-platformtheme qt6-gtk-platformtheme xdg-user-dirs fontconfig ;;
        fedora) echo xorg-x11-server-Xwayland xdg-desktop-portal xdg-desktop-portal-gtk xdg-desktop-portal-wlr \
            pipewire wireplumber ffmpeg-free curl adwaita-icon-theme xhost udisks2 util-linux polkit sudo \
            xdg-user-dirs fontconfig ;;
        arch) echo xorg-xwayland xdg-desktop-portal xdg-desktop-portal-gtk xdg-desktop-portal-wlr pipewire \
            wireplumber ffmpeg adwaita-icon-theme xorg-xhost udisks2 util-linux polkit sudo xdg-user-dirs fontconfig ;;
        slackware) echo xorg-server-xwayland xdg-desktop-portal xdg-desktop-portal-gtk pipewire ffmpeg udisks2 \
            util-linux dbus xhost polkit sudo xdg-user-dirs fontconfig adwaita-icon-theme ;;
    esac
}

# Para compilar o xwayland-satellite (no Arch, e no Fedora quando existir, vem pronto).
pacotes_x11() {
    case "$DISTRO" in
        debian) echo libxcb-cursor-dev libxcb1-dev clang ;;
        fedora) echo xwayland-satellite xcb-util-cursor-devel libxcb-devel clang ;;
        arch) echo xwayland-satellite ;;
        slackware) echo xcb-util-cursor libxcb llvm ;;
    esac
}

pacotes_login() {
    case "$DISTRO" in
        debian | fedora | arch) echo greetd ;;
        slackware) echo "" ;; # SlackBuilds.org (abaixo)
    esac
}

# Fica só com os pacotes que o repositório conhece (nomes mudam entre versões: um
# nome que falta não pode parar a instalação inteira).
disponiveis() {
    local p
    for p in "$@"; do
        case "$DISTRO" in
            debian) apt-cache show "$p" >/dev/null 2>&1 && echo "$p" ;;
            arch) { pacman -Si "$p" || pacman -Sg "$p"; } >/dev/null 2>&1 && echo "$p" ;;
            *) echo "$p" ;;
        esac
    done
}

# Pacotes do Slackware instalados (/var/lib/pkgtools/packages no 15.0, /var/log/packages antes).
slack_instalado() {
    ls /var/lib/pkgtools/packages /var/log/packages 2>/dev/null | grep -qE "^$1-[^-]+-[^-]+-[^-]+$"
}

instalar_pacotes() {
    local todos=("$@") ok=() faltam=() p
    [ ${#todos[@]} -gt 0 ] || return 0
    case "$DISTRO" in
        debian)
            root apt-get update
            if [ -n "$SIMULAR" ]; then ok=("${todos[@]}"); else mapfile -t ok < <(disponiveis "${todos[@]}"); fi
            for p in "${todos[@]}"; do [[ " ${ok[*]} " == *" $p "* ]] || faltam+=("$p"); done
            root env DEBIAN_FRONTEND=noninteractive apt-get install -y "${ok[@]}"
            ;;
        fedora)
            # dnf5 (Fedora 41+) e dnf4 escrevem "pular o que não existe" de jeitos diferentes.
            if [ -n "$SIMULAR" ]; then root dnf install -y --skip-unavailable "${todos[@]}"; else
                root dnf install -y --skip-unavailable "${todos[@]}" 2>/dev/null ||
                    root dnf install -y --setopt=strict=0 "${todos[@]}"
            fi
            ;;
        arch)
            if [ -n "$SIMULAR" ]; then ok=("${todos[@]}"); else mapfile -t ok < <(disponiveis "${todos[@]}"); fi
            for p in "${todos[@]}"; do [[ " ${ok[*]} " == *" $p "* ]] || faltam+=("$p"); done
            root pacman -S --needed --noconfirm "${ok[@]}"
            ;;
        slackware)
            for p in "${todos[@]}"; do slack_instalado "$p" || faltam+=("$p"); done
            if [ ${#faltam[@]} -eq 0 ]; then
                info "$(t "all present (full Slackware install)" "todos presentes (instalação completa do Slackware)")"
            elif command -v slackpkg >/dev/null || [ -n "$SIMULAR" ]; then
                info "$(t "missing from the official set" "faltam do conjunto oficial"): ${faltam[*]}"
                root slackpkg -batch=on -default_answer=y install "${faltam[@]}"
                faltam=()
            fi
            ;;
    esac
    [ ${#faltam[@]} -eq 0 ] || aviso "$(t "not found in the repositories (skipped)" "não encontrados nos repositórios (pulados)"): ${faltam[*]}"
}

# -------------------------------------------------------- SlackBuilds.org (C6)
# O que não vem no Slackware é compilado pelo SlackBuild oficial do SBo, com as
# dependências (REQUIRES) antes, e instalado como pacote (upgradepkg). Lista do SBo:
# SLACKBUILDS.TXT (local, versão e requisitos de cada um).
SBO_DIR=
sbo_lista() {
    if [ -z "$SBO_DIR" ]; then
        SBO_DIR="$PROJETO/target/install/sbo"
        mkdir -p "$SBO_DIR"
    fi
    [ -s "$SBO_DIR/SLACKBUILDS.TXT" ] ||
        curl -fsSL "https://slackbuilds.org/slackbuilds/$SBO_VERSAO/SLACKBUILDS.TXT" -o "$SBO_DIR/SLACKBUILDS.TXT"
}

# campo do SLACKBUILDS.TXT de um pacote: sbo_campo nome LOCATION
sbo_campo() {
    awk -v nome="$1" -v campo="SLACKBUILD $2:" '
        $0 == "SLACKBUILD NAME: " nome { dentro = 1; next }
        /^SLACKBUILD NAME:/ { dentro = 0 }
        dentro && index($0, campo) == 1 { sub(campo " *", ""); print; exit }' "$SBO_DIR/SLACKBUILDS.TXT"
}

sbo_instalar() {
    local nome="$1" local_ req dep pasta url md5s urls i
    slack_instalado "$nome" && { info "$nome: $(t "already installed" "já instalado")"; return 0; }
    if [ -n "$SIMULAR" ]; then mostra "SlackBuilds.org: $nome ($(t "with its REQUIRES" "com os REQUIRES dele"))"; return 0; fi
    sbo_lista
    local_="$(sbo_campo "$nome" LOCATION)"
    [ -n "$local_" ] || falha "$(t "not in SlackBuilds.org" "não está no SlackBuilds.org") $SBO_VERSAO: $nome"
    req="$(sbo_campo "$nome" REQUIRES)"
    for dep in $req; do
        [ "$dep" = "%README%" ] && continue
        sbo_instalar "$dep"
    done
    titulo "SlackBuilds.org: $nome"
    pasta="$SBO_DIR/$nome"
    rm -rf "$pasta"
    curl -fsSL "https://slackbuilds.org/slackbuilds/$SBO_VERSAO/${local_#./}.tar.gz" | tar -xz -C "$SBO_DIR"
    # Fontes do .info (as de 64 bits quando há), conferidas pelo MD5 do SBo.
    # shellcheck disable=SC1090
    (
        . "$pasta/$nome.info"
        urls="$DOWNLOAD"; md5s="$MD5SUM"
        if [ "$ARQ" = x86_64 ] && [ -n "${DOWNLOAD_x86_64:-}" ]; then urls="$DOWNLOAD_x86_64"; md5s="$MD5SUM_x86_64"; fi
        set -- $md5s
        cd "$pasta"
        for url in $urls; do
            curl -fL --retry 3 -o "$(basename "$url")" "$url"
            echo "$1  $(basename "$url")" | md5sum -c --quiet - || falha "MD5: $(basename "$url")"
            shift
        done
    )
    (cd "$pasta" && root env OUTPUT="$SBO_DIR/pacotes" sh "./$nome.SlackBuild")
    root upgradepkg --install-new --reinstall "$SBO_DIR"/pacotes/"$nome"-*.t?z
}

# ------------------------------------------------------------------------ Rust
versao_maior_igual() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -1)" = "$2" ]; }

garantir_rust() {
    local v
    v="$(usuario cargo --version 2>/dev/null | awk '{print $2}')" || v=
    if [ -n "$v" ] && versao_maior_igual "$v" "$RUST_MINIMO"; then
        info "Rust $v ($(t "ok" "ok"))"
        return
    fi
    if [ -n "$v" ]; then
        info "$(t "Rust $v is too old (the mide needs $RUST_MINIMO or newer)" "Rust $v é antigo demais (o mide pede $RUST_MINIMO ou mais novo)")"
    fi
    if usuario sh -c 'command -v rustup' >/dev/null 2>&1; then
        usuario_muda rustup update stable
        usuario_muda rustup default stable
        return
    fi
    info "$(t "Installing Rust with rustup for $USUARIO (in $UHOME/.cargo, without touching the system)" "Instalando o Rust pelo rustup para $USUARIO (em $UHOME/.cargo, sem mexer no sistema)")"
    usuario_muda sh -c "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path"
}

# -------------------------------------------------------------------- compilar
compilar() {
    titulo "$(t "Building the mide (a few minutes; runs as $USUARIO)" "Compilando o mide (alguns minutos; roda como $USUARIO)")"
    info "$(t "$JOBS cores (CARGO_BUILD_JOBS=N to change)" "$JOBS núcleos (CARGO_BUILD_JOBS=N para mudar)")"
    usuario_muda nice -n 19 cargo build --release --locked -j "$JOBS"
    # Pronto pelo gerenciador de pacotes (Arch, Fedora novo): não precisa compilar.
    if [ "$X11" = s ] && [ ! -x /usr/bin/xwayland-satellite ]; then
        titulo "$(t "Building xwayland-satellite (X11 apps)" "Compilando o xwayland-satellite (apps X11)")"
        usuario_muda nice -n 19 cargo install --locked -j "$JOBS" --git "$SATELLITE_GIT" --root "$PROJETO/target/install/satellite"
    fi
}

# ------------------------------------------------------------- montar os arquivos
# Monta em $PALCO tudo o que vai para o sistema, nos caminhos finais (como num
# pacote). Nada aqui precisa de root.
PALCO="$PROJETO/target/install/root"

montar() {
    local P="$PREFIXO" f
    titulo "$(t "Preparing the files" "Preparando os arquivos") ($PALCO)"
    [ -x target/release/mide ] || falha "$(t "target/release/mide not found: build first" "target/release/mide não existe: compile antes")"
    rm -rf "$PALCO"
    install -d "$PALCO$P/bin" "$PALCO$P/share/mide" "$PALCO/usr/share/wayland-sessions" \
        "$PALCO/usr/share/xdg-desktop-portal/portals" "$PALCO/usr/share/dbus-1/services" \
        "$PALCO/etc/pam.d" "$PALCO/etc/xdg/xdg-desktop-portal-wlr" "$PALCO/var/lib/mide"
    # Programas: o compositor procura os auxiliares (barra, dock...) ao lado dele. Os
    # do mios (instalador e boas-vindas) vão junto: só aparecem no mios (os atalhos e o
    # início automático deles vêm do mios-release e da imagem do live).
    for f in target/release/mide target/release/mide-* target/release/mios-*; do
        case "$f" in *.d | *.rlib) continue ;; esac
        [ -f "$f" ] && [ -x "$f" ] && install -m755 "$f" "$PALCO$P/bin/"
    done
    install -m755 data/mide-session "$PALCO$P/bin/mide-session"
    if [ -x target/install/satellite/bin/xwayland-satellite ] && [ "$X11" = s ]; then
        install -m755 target/install/satellite/bin/xwayland-satellite "$PALCO$P/bin/"
    fi
    # Dados: os .desktop dos apps, os portais e a fonte da interface (o compositor
    # acha `share/mide` ao lado de `bin`).
    cp -r data/applications data/xdg-desktop-portal "$PALCO$P/share/mide/"
    install -d "$PALCO$P/share/mide/fonts"
    install -m644 data/fonts/*.ttf data/fonts/OFL.txt "$PALCO$P/share/mide/fonts/"
    # Início automático da sessão do mide (a verificação de atualizações em segundo plano).
    install -d "$PALCO/etc/xdg/autostart"
    install -m644 data/autostart/*.desktop "$PALCO/etc/xdg/autostart/"
    # Papéis de parede do mios (Configurações → Papel de parede).
    install -d "$PALCO$P/share/mide/backgrounds"
    install -m644 data/mios/backgrounds/*.jpg "$PALCO$P/share/mide/backgrounds/"
    echo "$VERSAO" > "$PALCO$P/share/mide/version"
    # A entrada "mide" das telas de login (GDM, LightDM, SDDM, greetd).
    sed "s|^Exec=.*|Exec=$P/bin/mide-session|" data/wayland-sessions/mide.desktop \
        > "$PALCO/usr/share/wayland-sessions/mide.desktop"
    # Seletor de arquivos do mide nos apps (B25): o xdg-desktop-portal só lê os .portal
    # de /usr/share. Vale só na sessão do mide (UseIn=mide). Nunca um arquivo de ativação
    # para o FileManager1: valeria em todos os desktops.
    install -m644 data/xdg-desktop-portal/portals/mide.portal "$PALCO/usr/share/xdg-desktop-portal/portals/"
    install -m644 data/xdg-desktop-portal/mide-portals.conf "$PALCO/usr/share/xdg-desktop-portal/"
    printf '[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.mide\nExec=%s --portal-only\n' "$P/bin/mide-portal" \
        > "$PALCO/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.mide.service"
    # Gravar a tela (OBS...): o portal wlr escolhe o monitor pela janela do mide. O nome
    # do arquivo é o do desktop: só vale na sessão do mide.
    printf '[screencast]\nchooser_type=simple\nchooser_cmd=%s\n' "$P/bin/mide-screencast-picker" \
        > "$PALCO/etc/xdg/xdg-desktop-portal-wlr/mide"
    # Senha da tela de bloqueio.
    install -m644 data/pam/mide-lock "$PALCO/etc/pam.d/mide-lock"
    # Tampa e botão de ligar com o mide (ele pergunta o que fazer).
    if [ "$ENERGIA" = s ]; then
        local logind=/etc/systemd/logind.conf.d
        [ "$DISTRO" = slackware ] && logind=/etc/elogind/logind.conf.d
        install -d "$PALCO$logind"
        install -m644 data/systemd/60-mide-sem-suspensao.conf "$PALCO$logind/"
    fi
    [ "$ICONES" = s ] && montar_icones
    # As escolhas desta instalação: a atualização pelo app Atualização de Software
    # (compilando a versão nova) repete as mesmas.
    printf 'DISTRO=%s\nPREFIXO=%s\nX11=%s\nICONES=%s\nENERGIA=%s\nLOGIN=%s\n' \
        "$DISTRO" "$P" "${X11:-n}" "${ICONES:-n}" "${ENERGIA:-n}" "${LOGIN:-n}" > "$PALCO/var/lib/mide/install-options"
    # A lista do que foi instalado (para remover e atualizar), dentro do próprio palco.
    (cd "$PALCO" && find . \( -type f -o -type l \) ! -path ./var/lib/mide/installed-files | sed 's|^\.||' | sort) \
        > "$PALCO$MANIFESTO"
    info "$(wc -l < "$PALCO$MANIFESTO") $(t "files" "arquivos")"
}

# Tema de ícones do mide (fork do WhiteSur, na versão de data/icons.version), para
# todos os usuários.
montar_icones() {
    local versao fonte destino="$PALCO$PREFIXO/share/icons" nome
    versao="$(cat data/icons.version)"
    fonte="$PROJETO/target/install/mide-icons"
    if [ ! -d "$fonte/.git" ]; then
        git clone -q "$ICONES_GIT" "$fonte" || { aviso "$(t "icons not downloaded (no network?)" "ícones não baixados (sem rede?)")"; return 0; }
    fi
    git -C "$fonte" fetch -q origin || true
    git -C "$fonte" checkout -q "$versao"
    install -d "$destino"
    (cd "$fonte" && ./install.sh -n mide -d "$destino" >/dev/null 2>&1) ||
        { aviso "$(t "icon theme not installed (missing gtk-update-icon-cache?)" "tema de ícones não instalado (falta o gtk-update-icon-cache?)")"; return 0; }
    echo "$versao" > "$destino/mide/.versao-mide-icons"
    # Os apps do mide também no hicolor: aparecem em qualquer tema.
    install -d "$destino/hicolor/scalable/apps"
    for nome in mide-terminal mide-text mide-files mide-settings mide-screenshot mide-calculator mide-about; do
        [ -e "$destino/mide/apps/scalable/$nome.svg" ] && cp -L "$destino/mide/apps/scalable/$nome.svg" "$destino/hicolor/scalable/apps/"
    done
    info "$(t "icon theme" "tema de ícones") ($(echo "$versao" | cut -c1-8))"
}

# ----------------------------------------------------------- copiar para o sistema
# Debian, Fedora, Arch: tar do palco para / (dono root, sem mexer nas permissões das
# pastas que já existem, sem trocar links de pasta como /lib), e os arquivos de uma
# instalação anterior que não existem mais são apagados.
copiar_para_sistema() {
    local destino="${DESTDIR:-/}" antigo="${DESTDIR}$MANIFESTO" sobras
    titulo "$(t "Copying to" "Copiando para") $destino"
    sobras="$(mktemp)"
    if [ -f "$antigo" ]; then comm -23 <(sort "$antigo") "$PALCO$MANIFESTO" > "$sobras"; fi
    if [ -n "$SIMULAR" ]; then
        mostra "tar -C $PALCO -c . | tar -C $destino -x --no-same-owner --no-overwrite-dir --keep-directory-symlink"
    elif [ -n "$DESTDIR" ]; then
        mkdir -p "$DESTDIR"
        tar -C "$PALCO" -cf - . | tar -C "$DESTDIR" -xf - --no-same-owner --no-overwrite-dir --keep-directory-symlink
    else
        tar -C "$PALCO" --owner=0 --group=0 -cf - . | root tar -C / -xf - --no-same-owner --no-overwrite-dir --keep-directory-symlink
    fi
    if [ -s "$sobras" ]; then
        info "$(t "removing files from the previous version" "apagando arquivos da versão anterior"): $(wc -l < "$sobras")"
        apagar_lista "$sobras"
    fi
    rm -f "$sobras"
}

# Apaga os arquivos de uma lista (caminhos absolutos) e as pastas do mide que ficarem vazias.
apagar_lista() {
    local lista="$1" destino="${DESTDIR:-}"
    if [ -n "$SIMULAR" ]; then mostra "rm -f <$(wc -l < "$lista") $(t "files" "arquivos")>"; return; fi
    if [ -n "$DESTDIR" ]; then
        sed "s|^|$destino|" "$lista" | xargs -r -d '\n' rm -f --
    else
        sed "s|^|$destino|" "$lista" | root xargs -r -d '\n' rm -f --
    fi
    local pasta
    for pasta in "$PREFIXO/share/mide" "$PREFIXO/share/icons/mide" "$PREFIXO/share/icons/mide-dark" \
        "$PREFIXO/share/icons/mide-light" /etc/xdg/xdg-desktop-portal-wlr /var/lib/mide; do
        [ -d "$destino$pasta" ] || continue
        if [ -n "$DESTDIR" ]; then find "$destino$pasta" -depth -type d -empty -delete; else root find "$pasta" -depth -type d -empty -delete; fi
    done
}

# Slackware: um pacote de verdade, com o makepkg (a lista do pacote é a do pkgtools).
empacotar_slackware() {
    local pacote="$PROJETO/target/install/mide-$VERSAO-$ARQ-1_mide.txz"
    titulo "$(t "Slackware package" "Pacote do Slackware"): $(basename "$pacote")"
    install -d "$PALCO/install"
    cat > "$PALCO/install/slack-desc" <<'EOF'
    |-----handy-ruler------------------------------------------------------|
mide: mide (Mac Inspired Desktop Environment)
mide:
mide: A Wayland desktop environment written from scratch in Rust: compositor,
mide: top bar, dock, launcher, file manager, settings, terminal, editor and
mide: the other apps of a complete desktop, with a look inspired by macOS.
mide:
mide: Enter it from the login screen ("mide" session).
mide:
mide: https://github.com/fabioaacarneiro/mide
mide:
mide:
EOF
    cat > "$PALCO/install/doinst.sh" <<EOF
if [ -x /usr/bin/gtk-update-icon-cache ] && [ -d $PREFIXO/share/icons/mide ]; then
  /usr/bin/gtk-update-icon-cache -q -t $PREFIXO/share/icons/mide >/dev/null 2>&1
fi
EOF
    # O makepkg guarda a própria lista; a do instalador não vai no pacote.
    rm -f "$PALCO$MANIFESTO"
    if [ -n "$SIMULAR" ]; then
        mostra "cd $PALCO && makepkg -l y -c y $pacote"
        mostra "upgradepkg --install-new --reinstall $pacote"
        return
    fi
    (cd "$PALCO" && root /sbin/makepkg -l y -c y "$pacote")
    root /sbin/upgradepkg --install-new --reinstall "$pacote"
}

# ------------------------------------------------------------------ tela de login
login_instalado() { [ -d "${DESTDIR}/usr/local/lib/mide-login" ]; }

preparar_login_slackware() {
    # Pedido do README do greetd no SBo: o usuário do greeter antes de instalar.
    if ! getent group greeter >/dev/null; then root groupadd -g 381 greeter; fi
    if ! getent passwd greeter >/dev/null; then
        root useradd -d /var/lib/greeter -u 381 -g greeter -G video -s /bin/false greeter
    fi
    sbo_instalar greetd
}

instalar_login() {
    titulo "$(t "mide login screen (greetd)" "Tela de login do mide (greetd)")"
    [ "$DISTRO" = slackware ] && preparar_login_slackware
    root bash "$PROJETO/data/mide-login-install"
}

# --------------------------------------------------------- temas do boot (mios)
# GRUB e Plymouth do mios (data/mios/install-boot-theme). Só no GRUB do jeito do
# Debian/Ubuntu (update-grub); no mios quem cuida deles é o pacote mios-release.
# Programas de administração ficam em /usr/sbin, fora do PATH de quem não é root.
tem_programa() { command -v "$1" >/dev/null 2>&1 || [ -x "/usr/sbin/$1" ] || [ -x "/sbin/$1" ]; }
boot_tema_possivel() { [ -z "$DESTDIR" ] && tem_programa update-grub && ! dpkg -s mios-release >/dev/null 2>&1; }
boot_tema_instalado() { [ -d "${DESTDIR}/boot/grub/themes/mios" ] && ! dpkg -s mios-release >/dev/null 2>&1; }

instalar_tema_boot() {
    titulo "$(t "mios boot themes (GRUB and Plymouth)" "Temas do boot do mios (GRUB e Plymouth)")"
    if dpkg -s mios-release >/dev/null 2>&1; then
        info "$(t "this is a mios system: the mios-release package already has them" "este sistema é o mios: o pacote mios-release já os tem")"
        return 0
    fi
    tem_programa update-grub ||
        falha "$(t "update-grub not found: for now the boot themes are installed only on Debian/Ubuntu-like systems with GRUB" "update-grub não encontrado: por enquanto os temas do boot só são instalados em sistemas do tipo Debian/Ubuntu com o GRUB")"
    local faltam=() splash=()
    tem_programa grub-mkfont || faltam+=(grub-common)
    if ! tem_programa plymouth-set-default-theme; then
        if pergunta "$(t "Plymouth (the boot animation) is not installed. Install it?" "O Plymouth (a animação do boot) não está instalado. Instalar?")" s; then
            faltam+=(plymouth plymouth-themes)
        fi
    fi
    if [ ${#faltam[@]} -gt 0 ]; then
        DISTRO="${DISTRO:-debian}"
        instalar_pacotes "${faltam[@]}"
    fi
    # Sem "splash" na linha do kernel, o Plymouth não aparece.
    if ! grep -qE '^GRUB_CMDLINE_LINUX_DEFAULT=.*splash' /etc/default/grub 2>/dev/null; then
        pergunta "$(t "Show the boot animation instead of the text messages (adds \"quiet splash\" to the kernel line)?" "Mostrar a animação no boot em vez das mensagens em texto (põe \"quiet splash\" na linha do kernel)?")" s &&
            splash=(--splash)
    fi
    root bash "$PROJETO/data/mios/install-boot-theme" "${splash[@]}"
    info "$(t "Go back with: ./install.sh boot-theme remove" "Para voltar: ./install.sh boot-theme remove")"
}

remover_tema_boot() {
    titulo "$(t "Removing the mios boot themes" "Tirando os temas do boot do mios")"
    root bash "$PROJETO/data/mios/install-boot-theme" --uninstall
}

# ---------------------------------------------------------------- só compilar
# Apaga o que as compilações anteriores deixaram (o disco já encheu com 400 GB de
# target/): a pasta de debug, os intermediários incrementais e as cópias antigas do
# próprio mide e do mide-ui (o cargo guarda uma por versão e nunca apaga). As
# dependências já compiladas ficam: nada é recompilado do zero.
so_compilar() {
    titulo "$(t "Cleaning old builds" "Limpando compilações antigas") ($(du -sh target 2>/dev/null | cut -f1))"
    usuario_muda rm -rf target/debug target/release/incremental
    usuario_muda cargo clean --release -p mide -p mide-ui >/dev/null 2>&1 || true
    titulo "Rust"
    garantir_rust
    compilar
    info "$(t "Programs in" "Programas em") $PROJETO/target/release ($(du -sh target 2>/dev/null | cut -f1) $(t "used by the build" "usados pela compilação"))"
}

so_dependencias() {
    escolher_distro
    local pacotes=()
    read -ra pacotes <<< "$(pacotes_compilar) $(pacotes_rodar) $(pacotes_x11)"
    titulo "$(t "Packages" "Pacotes")"
    instalar_pacotes "${pacotes[@]}"
    if [ "$DISTRO" = slackware ]; then sbo_instalar seatd; sbo_instalar xdg-desktop-portal-wlr; fi
    titulo "Rust"
    garantir_rust
}

# ------------------------------------------------- atualizar a sessão aberta
# Para quem desenvolve: recompila e troca os programas da sessão do mide aberta, a
# partir desta pasta (data/mide-update). Dentro da sessão, reinicia também o
# compositor e a tela de login (--all).
atualizar_sessao() {
    [ "$(id -u)" != 0 ] || falha "$(t "run without sudo (it asks for the password when needed)" "rode sem sudo (ele pede a senha quando precisa)")"
    # Com o pacote do repositório, a sessão usa os programas dele: compilar aqui não a muda.
    if pacote_instalado mide; then
        falha "$(t "the mide on this computer is the package from the mide repository: new versions arrive through the Software Update app. To try the mide of this folder, build it (./install.sh build) and choose the \"mide (desenvolvimento)\" session on the login screen (tests/session-dev.sh adds it); to go back to the development install: ./install.sh --remove and then ./install.sh update" "o mide deste computador é o pacote do repositório do mide: as versões novas chegam pelo app Atualização de Software. Para testar o mide desta pasta, compile (./install.sh build) e escolha a sessão \"mide (desenvolvimento)\" na tela de login (o tests/session-dev.sh cria a entrada); para voltar à instalação de desenvolvimento: ./install.sh --remove e depois ./install.sh update")"
    fi
    local opcao=()
    case ":${XDG_CURRENT_DESKTOP:-}:" in
        *:mide:*)
            opcao=(--all)
            info "$(t "Inside the mide session: the compositor restarts at the end (open app windows close; save your work)." "Dentro da sessão do mide: o compositor reinicia no fim (as janelas dos apps fecham; salve o que estiver aberto).")"
            pergunta "$(t "Continue?" "Continuar?")" s || exit 0
            ;;
        *) info "$(t "Outside the mide session: the programs are rebuilt and take effect the next time you log in." "Fora da sessão do mide: os programas são recompilados e valem ao entrar de novo na sessão.")" ;;
    esac
    titulo "data/mide-update ${opcao[*]}"
    usuario_muda env CARGO_BUILD_JOBS="$JOBS" nice -n 19 "$PROJETO/data/mide-update" "${opcao[@]}"
}

# --------------------------------------------------------------- pacotes .deb
# Montagens fora de uma versão marcada (tag) levam o commit na versão, como no CI:
# ficam acima dos pacotes publicados com o mesmo número.
sufixo_versao() {
    if [ -n "${MIDE_VERSION_SUFFIX+x}" ]; then echo "$MIDE_VERSION_SUFFIX"; return; fi
    if usuario git -C "$PROJETO" describe --exact-match --tags HEAD >/dev/null 2>&1 &&
        [ -z "$(usuario git -C "$PROJETO" status --porcelain --untracked-files=no 2>/dev/null)" ]; then
        echo ""
    else
        echo "+git$(usuario git -C "$PROJETO" rev-parse --short=7 HEAD 2>/dev/null || echo local)"
    fi
}

fazer_pacotes() {
    DISTRO="${DISTRO:-$(detectar_distro)}"
    [ "$DISTRO" = debian ] ||
        falha "$(t "the .deb packages are built on Debian/Ubuntu (for the ISO elsewhere, use docker or podman: ./install.sh iso)" "os pacotes .deb são gerados no Debian/Ubuntu (para a ISO em outro sistema, use o docker ou o podman: ./install.sh iso)")"
    local sufixo pacotes=() extra=()
    sufixo="$(sufixo_versao)"
    titulo "$(t ".deb packages" "Pacotes .deb"): mide $VERSAO$sufixo, mios-release"
    if [ -n "$DEPENDENCIAS" ]; then
        read -ra pacotes <<< "$(pacotes_compilar) $(pacotes_x11) dpkg-dev grub-common"
        instalar_pacotes "${pacotes[@]}"
    fi
    garantir_rust
    [ -n "$COMPILAR" ] || extra=(--no-build)
    usuario_muda env MIDE_VERSION_SUFFIX="$sufixo" CARGO_BUILD_JOBS="$JOBS" nice -n 19 "$PROJETO/packaging/debian/build-deb.sh" "${extra[@]}"
    usuario_muda env MIDE_VERSION_SUFFIX="$sufixo" "$PROJETO/packaging/mios-release/build-deb.sh"
    titulo "$(t "Done" "Pronto")"
    [ -n "$SIMULAR" ] || ls -1 "$PROJETO"/target/deb/*"$VERSAO$sufixo"*.deb 2>/dev/null | sed 's/^/    /'
}

# ------------------------------------------------------------------------ ISO
# No Debian 13 (o que está por baixo do mios), com o live-build do sistema; nos outros,
# dentro de um contêiner debian:trixie (docker ou podman), como no CI.
host_trixie() { [ "$(. /etc/os-release 2>/dev/null && echo "${VERSION_CODENAME:-}")" = trixie ]; }

fazer_iso() {
    [ "$ARQ" = x86_64 ] || falha "$(t "the ISO is x86_64 (amd64) only" "a ISO é só para x86_64 (amd64)")"
    local versao_iso destino
    versao_iso="$(cat iso/VERSION)"
    destino="$PROJETO/target/iso/mios-$versao_iso-amd64.iso"
    # Os pacotes da ISO levam sempre o commit na versão (também numa tag): com o número
    # igual ao do repositório apt do mios, o apt da montagem troca o pacote local pelo
    # da internet e para ("downgrade").
    export MIDE_VERSION_SUFFIX="${MIDE_VERSION_SUFFIX-+git$(usuario git -C "$PROJETO" rev-parse --short=7 HEAD 2>/dev/null || echo local)}"
    if host_trixie; then
        DISTRO=debian
        titulo "$(t "mios ISO (Debian 13 live-build; asks for the sudo password; 20 to 60 minutes, downloads about 1.5 GB)" "ISO do mios (live-build do Debian 13; pede a senha do sudo; de 20 a 60 minutos, baixa uns 1,5 GB)")"
        [ -z "$DEPENDENCIAS" ] || instalar_pacotes live-build debootstrap squashfs-tools xorriso mtools dosfstools \
            grub-pc-bin grub-efi-amd64-bin grub-efi-amd64-signed shim-signed grub-common dpkg-dev
        fazer_pacotes
        titulo "live-build"
        root "$PROJETO/iso/build.sh"
        # O chroot da montagem (alguns GB, de root) sai; o cache dos pacotes fica para a
        # próxima vez, com a ISO, de volta para o usuário.
        root sh -c "cd '$PROJETO/target/iso/work' && lb clean >/dev/null 2>&1; chown -R '$USUARIO:' '$PROJETO/target/iso'"
    else
        local motor=""
        if command -v docker >/dev/null; then motor=docker; elif command -v podman >/dev/null; then motor=podman; fi
        [ -n "$motor" ] || falha "$(t "outside Debian 13 the ISO is built in a container: install docker or podman" "fora do Debian 13 a ISO é montada num contêiner: instale o docker ou o podman")"
        titulo "$(t "mios ISO in a debian:trixie container ($motor; 30 to 90 minutes)" "ISO do mios num contêiner debian:trixie ($motor; de 30 a 90 minutos)")"
        local uid gid sufixo
        uid="$(id -u "$USUARIO")"; gid="$(id -g "$USUARIO")"
        sufixo="$(sufixo_versao)"
        usuario_muda mkdir -p "$PROJETO/.ci-cache/cargo"
        root "$motor" run --rm --privileged -v "$PROJETO:/src" -w /src -v "$PROJETO/.ci-cache/cargo:/root/.cargo" \
            -e MIDE_VERSION_SUFFIX="$sufixo" -e CARGO_BUILD_JOBS="$JOBS" debian:trixie \
            bash -c "iso/ci.sh; s=\$?; (cd target/iso/work && lb clean >/dev/null 2>&1); chown -R $uid:$gid target .ci-cache; exit \$s"
    fi
    titulo "$(t "ISO ready" "ISO pronta")"
    info "$destino"
    info "$(t "Try it in a virtual machine" "Para testar numa máquina virtual"): ./install.sh iso-test"
    info "$(t "Write it to a USB stick (ERASES the stick): the Disks app, or" "Gravar num pendrive (APAGA o pendrive): o app Discos, ou") sudo dd if=$destino of=/dev/sdX bs=4M status=progress oflag=sync"
}

# Liga a ISO mais nova numa máquina virtual (QEMU), com um disco virtual de 32 GB para
# experimentar o instalador. "bios": modo legado; "disk": só o disco (o sistema instalado).
testar_iso() {
    local iso disco firmware=() kvm=() midia=() tela
    iso="$(ls -t "$PROJETO"/target/iso/mios-*.iso 2>/dev/null | head -1)"
    [ -n "$iso" ] || [ -n "$TESTE_DISCO" ] || falha "$(t "no ISO in target/iso: run ./install.sh iso (or download one from the GitHub CI)" "nenhuma ISO em target/iso: rode ./install.sh iso (ou baixe uma do CI do GitHub)")"
    if ! command -v qemu-system-x86_64 >/dev/null || ! command -v qemu-img >/dev/null; then
        DISTRO="${DISTRO:-$(detectar_distro)}"
        case "$DISTRO" in
            debian) instalar_pacotes qemu-system-x86 qemu-utils ovmf ;;
            fedora) instalar_pacotes qemu-system-x86 qemu-img edk2-ovmf ;;
            arch) instalar_pacotes qemu-desktop edk2-ovmf ;;
            *) falha "$(t "install QEMU (qemu-system-x86_64) and OVMF" "instale o QEMU (qemu-system-x86_64) e o OVMF")" ;;
        esac
    fi
    [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ] || falha "$(t "the virtual machine opens a window: run it inside a graphical session" "a máquina virtual abre uma janela: rode dentro de uma sessão gráfica")"
    disco="$PROJETO/target/iso/test-disk.qcow2"
    usuario_muda mkdir -p "$PROJETO/target/iso"
    if [ -f "$disco" ] && [ -z "$TESTE_DISCO" ] &&
        pergunta "$(t "Start with a blank virtual disk (erases the previous test install)?" "Começar com o disco virtual em branco (apaga a instalação de teste anterior)?")" n; then
        rm -f "$disco" "$PROJETO/target/iso/ovmf-vars.fd"
    fi
    [ -f "$disco" ] || usuario_muda qemu-img create -f qcow2 "$disco" 32G >/dev/null
    if [ -z "$TESTE_BIOS" ]; then
        local codigo vars
        for codigo in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/edk2/x64/OVMF_CODE.4m.fd \
            /usr/share/edk2/ovmf/OVMF_CODE.fd /usr/share/ovmf/OVMF.fd; do
            [ -f "$codigo" ] || continue
            vars="$PROJETO/target/iso/ovmf-vars.fd"
            if [ ! -f "$vars" ] && [ -z "$SIMULAR" ]; then
                cp "$(dirname "$codigo")/$(basename "$codigo" | sed 's/CODE/VARS/')" "$vars" 2>/dev/null || true
            fi
            if { [ -f "$vars" ] || [ -n "$SIMULAR" ]; } && [ "$codigo" != /usr/share/ovmf/OVMF.fd ]; then
                firmware=(-drive "if=pflash,format=raw,readonly=on,file=$codigo" -drive "if=pflash,format=raw,file=$vars")
            else
                firmware=(-bios "$codigo")
            fi
            break
        done
        [ ${#firmware[@]} -gt 0 ] || aviso "$(t "OVMF (UEFI) not found: starting in legacy BIOS mode" "OVMF (UEFI) não encontrado: ligando no modo BIOS legado")"
    fi
    [ -w /dev/kvm ] && kvm=(-enable-kvm -cpu host)
    [ -n "$TESTE_DISCO" ] || midia=(-drive "file=$iso,if=none,id=cd,media=cdrom,readonly=on" -device ide-cd,drive=cd,bootindex=0)
    tela=gtk
    titulo "$(t "Starting the virtual machine (close the window to turn it off)" "Ligando a máquina virtual (feche a janela para desligar)")"
    info "${iso:-$(t "virtual disk only" "só o disco virtual")}$( [ -n "$TESTE_BIOS" ] && echo " — BIOS")"
    usuario_muda qemu-system-x86_64 "${kvm[@]}" -m 4096 -smp 2 "${firmware[@]}" \
        -drive "file=$disco,if=none,id=hd,format=qcow2" -device virtio-blk-pci,drive=hd,bootindex=1 \
        "${midia[@]}" -vga virtio -display "$tela" -nic user,model=virtio-net-pci
}

# ------------------------------------------------------------- submenus
menu_login() {
    if [ -z "$TIRAR" ] && [ "$ACAO" = menu-login ]; then
        escolha "$(t "mide login screen (greetd)" "Tela de login do mide (greetd)")" 1 \
            "$(t "Install it (or update it)" "Instalar (ou atualizar)")" \
            "$(t "Go back to the previous login screen" "Voltar à tela de login de antes")" \
            "$(t "Back" "Voltar")"
        case "$ESCOLHA" in 2) TIRAR=1 ;; 3) return 0 ;; esac
    fi
    if [ -n "$TIRAR" ]; then
        titulo "$(t "Login screen" "Tela de login")"
        root bash "$PROJETO/data/mide-login-install" --uninstall
        return
    fi
    DISTRO="${DISTRO:-$(detectar_distro)}"
    if [ ! -x target/release/mide-login ]; then
        titulo "Rust"; garantir_rust; compilar
    fi
    instalar_login
}

menu_boot() {
    if [ -z "$TIRAR" ] && [ "$ACAO" = menu-boot ]; then
        escolha "$(t "mios boot themes (GRUB and Plymouth)" "Temas do boot do mios (GRUB e Plymouth)")" 1 \
            "$(t "Install them" "Instalar")" \
            "$(t "Go back to the previous ones" "Voltar aos de antes")" \
            "$(t "Back" "Voltar")"
        case "$ESCOLHA" in 2) TIRAR=1 ;; 3) return 0 ;; esac
    fi
    if [ -n "$TIRAR" ]; then remover_tema_boot; else instalar_tema_boot; fi
}

# ------------------------------------------------------- pacote pronto (apt)
# No Debian e nos derivados (Ubuntu...), o mide vem pronto do repositório apt do mide,
# o mesmo do mios: nada a compilar, e as versões novas chegam pelo app Atualização de
# Software (PackageKit), como qualquer pacote.
pacote_instalado() { [ "$(dpkg-query -W -f='${Status}' "$1" 2>/dev/null)" = "install ok installed" ]; }

# O repositório serve a este sistema? Precisa de rede e de uma glibc pelo menos igual à
# que o pacote publicado pede.
apt_possivel() {
    [ "$DISTRO" = debian ] && [ "$ARQ" = x86_64 ] && [ -z "$DESTDIR" ] && [ -n "$COMPILAR" ] || return 1
    command -v apt-get >/dev/null && command -v dpkg-query >/dev/null || return 1
    local lista pedida tem
    lista="$(curl -fsSL --max-time 20 "$APT_URL/Packages" 2>/dev/null)" || return 1
    grep -q '^Package: mide$' <<< "$lista" || return 1
    pedida="$(printf '%s\n' "$lista" | awk 'BEGIN { RS = "" } /^Package: mide\n/' |
        sed -nE 's/^Depends:.*libc6 \(>= ([^)]+)\).*/\1/p' | head -1)"
    # Pacotes publicados antes da dependência com versão: feitos no Debian 13.
    pedida="${pedida:-2.41}"
    tem="$(dpkg-query -W -f='${Version}' libc6 2>/dev/null)" || return 1
    dpkg --compare-versions "${tem%%-*}" ge "$pedida"
}

# Arquivos que o data/mide-update (instalação de desenvolvimento) deixa na pasta pessoal:
# o portal e o seletor de tela apontando para os programas da pasta do projeto, e o tema
# de ícones na versão daquela compilação. Com o mide instalado no sistema, eles passariam
# na frente dos do sistema. Só os criados por ele, conferidos pelo conteúdo ou pela marca.
restos_desenvolvimento() {
    local dados="${XDG_DATA_HOME:-$UHOME/.local/share}" config="${XDG_CONFIG_HOME:-$UHOME/.config}" nome
    [ "$(id -u)" = 0 ] && { dados="$UHOME/.local/share"; config="$UHOME/.config"; }
    local servico="$dados/dbus-1/services/org.freedesktop.impl.portal.desktop.mide.service"
    grep -qs '^Exec=.*/target/[a-z]*/mide-portal' "$servico" && echo "$servico"
    grep -qs '^chooser_cmd=.*/target/[a-z]*/mide-screencast-picker' "$config/xdg-desktop-portal-wlr/mide" && echo "$config/xdg-desktop-portal-wlr/mide"
    if [ -f "$dados/icons/mide/.versao-mide-icons" ]; then
        echo "$dados/icons/mide"
        for nome in mide-terminal mide-text mide-files mide-settings mide-screenshot mide-calculator mide-about; do
            [ -f "$dados/icons/hicolor/scalable/apps/$nome.svg" ] && echo "$dados/icons/hicolor/scalable/apps/$nome.svg"
        done
    fi
    return 0
}

tirar_restos_desenvolvimento() {
    local restos f
    restos="$(restos_desenvolvimento)"
    [ -n "$restos" ] || return 0
    titulo "$(t "Development files in your home folder" "Arquivos da instalação de desenvolvimento na pasta pessoal")"
    while IFS= read -r f; do
        usuario_muda rm -rf "$f"
    done <<< "$restos"
    [ -n "$SIMULAR" ] || usuario gtk-update-icon-cache -q -t "${XDG_DATA_HOME:-$UHOME/.local/share}/icons/hicolor" 2>/dev/null || true
}

instalar_apt() {
    printf '\n%s%s%s\n' "$N" "$(t "What to install" "O que instalar")" "$Z"
    info "$(t "the mide package from the mide apt repository (in /usr; apps X11 and the icon theme included)" "o pacote mide do repositório apt do mide (em /usr; apps X11 e tema de ícones incluídos)")"
    [ -n "$ENERGIA" ] || { pergunta "$(t "Let the mide handle the lid and the power key (it asks before sleeping)?" "Deixar a tampa e o botão de ligar com o mide (ele pergunta antes de dormir)?")" s && ENERGIA=s || ENERGIA=n; }
    # Quem já usa a tela de login do mide continua com ela (resposta padrão "sim").
    [ -n "$LOGIN" ] || { pergunta "$(t "Replace the login screen with the mide one (greetd)? You can go back with --remove" "Trocar a tela de login pela do mide (greetd)? Dá para voltar com --remove")" "$(login_instalado && echo s || echo n)" && LOGIN=s || LOGIN=n; }
    boot_tema_possivel || BOOT=n
    [ -n "$BOOT" ] || { pergunta "$(t "Also the mios boot screens (GRUB menu and Plymouth animation)? You can go back with --remove" "Também as telas do boot do mios (menu do GRUB e animação do Plymouth)? Dá para voltar com --remove")" n && BOOT=s || BOOT=n; }

    # Outra instalação do mide (compilada ou de desenvolvimento) sai antes: os programas
    # dela em /usr/local viriam antes dos do pacote no PATH.
    local compilada="" desenvolvimento=""
    [ -f "$MANIFESTO" ] && compilada=1
    login_instalado && [ -z "$compilada" ] && ! pacote_instalado mide && desenvolvimento=1

    printf '\n%s%s%s\n' "$N" "$(t "Summary" "Resumo")" "$Z"
    info "$(t "repository" "repositório"): $APT_URL"
    info "$(t "updates: the Software Update app (or sudo apt upgrade)" "atualizações: o app Atualização de Software (ou sudo apt upgrade)")"
    [ -n "$compilada" ] && info "$(t "the mide built before (in /usr/local) is removed first" "o mide compilado antes (em /usr/local) sai antes")"
    if [ -n "$desenvolvimento" ] && [ "$LOGIN" = s ]; then
        info "$(t "the development install gives way to the package; the mide login screen stays, with the package programs" "a instalação de desenvolvimento dá lugar ao pacote; a tela de login do mide continua, com os programas do pacote")"
    elif [ -n "$desenvolvimento" ]; then
        info "$(t "the development install and the mide login screen are removed first" "a instalação de desenvolvimento e a tela de login do mide saem antes")"
    fi
    [ -n "$(restos_desenvolvimento)" ] && info "$(t "files of the development install in your home folder are removed (portal and screen picker of the project folder, icons of that build)" "saem da pasta pessoal os arquivos da instalação de desenvolvimento (portal e seletor de tela da pasta do projeto, ícones daquela compilação)")"
    info "$(t "Your settings (~/.config/mide) stay." "As suas configurações (~/.config/mide) ficam.")"
    pergunta "$(t "Continue?" "Continuar?")" s || exit 0

    # Mantendo a tela de login do mide, o greetd não é desligado em momento nenhum: no fim
    # ela passa a usar os programas do pacote (e as sobras da cópia de desenvolvimento saem).
    # Antes, ela saía e voltava, e o computador ficava sem tela de login se o gerenciador
    # de antes já tivesse sido desinstalado ou se algo falhasse no meio.
    if login_instalado && [ "$LOGIN" != s ]; then
        titulo "$(t "mide login screen (removed)" "Tela de login do mide (sai)")"
        root bash "$PROJETO/data/mide-login-install" --uninstall
    fi
    if [ -n "$compilada" ]; then
        titulo "$(t "Removing the mide built before" "Tirando o mide compilado antes")"
        local copia
        PREFIXO="$(sed -n 's|^\(.*\)/bin/mide$|\1|p' "$MANIFESTO" | head -1)"
        copia="$(mktemp)"
        cp "$MANIFESTO" "$copia"
        echo "$MANIFESTO" >> "$copia"
        apagar_lista "$copia"
        rm -f "$copia"
    fi

    titulo "$(t "mide apt repository" "Repositório apt do mide")"
    local extras=()
    if [ ! -f "$APT_CHAVE" ]; then
        root install -D -m644 "$PROJETO/data/mios/apt/mios-archive-keyring.gpg" "$APT_CHAVE"
        extras+=("$APT_CHAVE")
    fi
    if [ ! -f "$APT_FONTE" ]; then
        local fonte
        fonte="$(mktemp)"
        printf 'Types: deb\nURIs: %s/\nSuites: ./\nSigned-By: %s\n' "$APT_URL" "$APT_CHAVE" > "$fonte"
        root install -D -m644 "$fonte" "$APT_FONTE"
        rm -f "$fonte"
        extras+=("$APT_FONTE")
    fi
    root apt-get update
    titulo "$(t "Installing the mide package" "Instalando o pacote do mide")"
    root env DEBIAN_FRONTEND=noninteractive apt-get install -y mide
    tirar_restos_desenvolvimento
    if [ "$ENERGIA" = s ]; then
        root install -D -m644 "$PROJETO/data/systemd/60-mide-sem-suspensao.conf" /etc/systemd/logind.conf.d/60-mide-sem-suspensao.conf
        extras+=(/etc/systemd/logind.conf.d/60-mide-sem-suspensao.conf)
        root systemctl kill -s HUP systemd-logind || true
    fi
    if [ ${#extras[@]} -gt 0 ]; then
        printf '%s\n' "${extras[@]}" | root sh -c "mkdir -p '$(dirname "$APT_EXTRAS")' && cat >> '$APT_EXTRAS'"
    fi
    [ "$LOGIN" = s ] && instalar_login
    [ "$BOOT" = s ] && instalar_tema_boot

    titulo "$(t "Done" "Pronto")"
    info "$(t "Log out and choose the \"mide\" session on the login screen." "Saia da sessão e escolha a sessão \"mide\" na tela de login.")"
    info "$(t "New versions arrive through the Software Update app." "As versões novas chegam pelo app Atualização de Software.")"
    info "$(t "Remove it with" "Para remover"): ./install.sh --remove"
}

# ------------------------------------------------------------------------ instalar
instalar() {
    escolher_distro
    [ -n "$PREFIXO" ] || { if [ "$DISTRO" = slackware ]; then PREFIXO=/usr; else PREFIXO=/usr/local; fi; }
    [ -n "$DESTDIR" ] && [ -z "$LOGIN" ] && LOGIN=n
    # Debian e derivados: pacote pronto (recomendado) ou compilar aqui.
    if [ -z "$FONTE" ]; then
        FONTE=codigo
        if apt_possivel; then
            escolha "$(t "How to install?" "Como instalar?")" 1 \
                "$(t "Ready package from the mide repository (recommended: updates arrive through Software Update)" "Pacote pronto do repositório do mide (recomendado: as atualizações chegam pela Atualização de Software)")" \
                "$(t "Build here (slower; to develop or try changes)" "Compilar aqui (mais demorado; para desenvolver ou testar mudanças)")"
            [ "$ESCOLHA" = 1 ] && FONTE=apt
        fi
    elif [ "$FONTE" = apt ] && ! apt_possivel; then
        falha "$(t "the mide apt repository does not serve this system (Debian/Ubuntu amd64 with a recent enough glibc, network needed); use --source" "o repositório apt do mide não serve a este sistema (Debian/Ubuntu amd64 com glibc nova o bastante, e rede); use --source")"
    fi
    if [ "$FONTE" = apt ]; then
        instalar_apt
        return
    fi
    boot_tema_possivel || BOOT=n

    printf '\n%s%s%s\n' "$N" "$(t "What to install" "O que instalar")" "$Z"
    info "$(t "mide: compositor, bar, dock, launcher, apps and the login-screen session (always)" "mide: compositor, barra, dock, launcher, apps e a sessão na tela de login (sempre)")"
    [ -n "$X11" ] || { pergunta "$(t "X11 apps inside the mide (xwayland-satellite)?" "Apps X11 dentro do mide (xwayland-satellite)?")" s && X11=s || X11=n; }
    [ -n "$ICONES" ] || { pergunta "$(t "The mide icon theme (needs internet)?" "O tema de ícones do mide (precisa de internet)?")" s && ICONES=s || ICONES=n; }
    [ -n "$ENERGIA" ] || { pergunta "$(t "Let the mide handle the lid and the power key (it asks before sleeping)?" "Deixar a tampa e o botão de ligar com o mide (ele pergunta antes de dormir)?")" s && ENERGIA=s || ENERGIA=n; }
    [ -n "$LOGIN" ] || { pergunta "$(t "Replace the login screen with the mide one (greetd)? You can go back with --remove" "Trocar a tela de login pela do mide (greetd)? Dá para voltar com --remove")" "$(login_instalado && echo s || echo n)" && LOGIN=s || LOGIN=n; }
    [ -n "$BOOT" ] || { pergunta "$(t "Also the mios boot screens (GRUB menu and Plymouth animation)? You can go back with --remove" "Também as telas do boot do mios (menu do GRUB e animação do Plymouth)? Dá para voltar com --remove")" n && BOOT=s || BOOT=n; }

    local pacotes=()
    read -ra pacotes <<< "$(pacotes_compilar) $(pacotes_rodar)"
    [ "$X11" = s ] && read -ra extra <<< "$(pacotes_x11)" && pacotes+=("${extra[@]}")
    [ "$LOGIN" = s ] && [ -n "$(pacotes_login)" ] && pacotes+=("$(pacotes_login)")

    printf '\n%s%s%s\n' "$N" "$(t "Summary" "Resumo")" "$Z"
    info "$(t "system" "sistema"): $(nome_distro "$DISTRO")"
    info "$(t "programs in" "programas em") ${DESTDIR}$PREFIXO/bin$( [ "$DISTRO" = slackware ] && t " (as a Slackware package)" " (como pacote do Slackware)")"
    info "$(t "user that builds" "usuário que compila"): $USUARIO"
    [ -n "$DEPENDENCIAS" ] && info "$(t "packages" "pacotes"): ${pacotes[*]}"
    [ "$DISTRO" = slackware ] && [ -n "$DEPENDENCIAS" ] &&
        info "SlackBuilds.org: seatd xdg-desktop-portal-wlr$( [ "$LOGIN" = s ] && echo " greetd")"
    pergunta "$(t "Continue?" "Continuar?")" s || exit 0

    if [ -n "$DEPENDENCIAS" ]; then
        titulo "$(t "Packages" "Pacotes")"
        instalar_pacotes "${pacotes[@]}"
        if [ "$DISTRO" = slackware ]; then
            sbo_instalar seatd
            sbo_instalar xdg-desktop-portal-wlr
        fi
    fi
    if [ -n "$COMPILAR" ]; then
        titulo "Rust"
        garantir_rust
        compilar
    fi
    [ -n "$SIMULAR" ] && [ ! -x target/release/mide ] && { info "$(t "(dry run: nothing built)" "(simulação: nada compilado)")"; return; }
    montar

    printf '\n'
    pergunta "$(t "The mide $VERSAO is ready. Install it on the system now?" "O mide $VERSAO está pronto. Instalar no sistema agora?")" s || {
        info "$(t "Nothing was copied. The files are in" "Nada foi copiado. Os arquivos estão em") $PALCO"
        exit 0
    }
    if [ "$DISTRO" = slackware ] && [ -z "$DESTDIR" ]; then empacotar_slackware; else copiar_para_sistema; fi
    [ -z "$DESTDIR" ] && tirar_restos_desenvolvimento

    if [ "$ENERGIA" = s ] && [ -z "$DESTDIR" ]; then
        if [ "$DISTRO" = slackware ] || ! command -v systemctl >/dev/null; then root pkill -HUP -x elogind-daemon || true
        else root systemctl kill -s HUP systemd-logind || true; fi
    fi
    [ "$LOGIN" = s ] && [ -z "$DESTDIR" ] && instalar_login
    [ "$BOOT" = s ] && [ -z "$DESTDIR" ] && instalar_tema_boot
    rm -rf "$PALCO"

    titulo "$(t "Done" "Pronto")"
    info "$(t "Log out and choose the \"mide\" session on the login screen." "Saia da sessão e escolha a sessão \"mide\" na tela de login.")"
    if [ "$DISTRO" = slackware ] && [ "$LOGIN" != s ]; then
        info "$(t "Without a login screen (runlevel 3), start it from the console with: mide-session" "Sem tela de login (runlevel 3), entre pelo console com: mide-session")"
    fi
    info "$(t "Remove it with" "Para remover"): ./install.sh --remove"
}

# ------------------------------------------------------------------------- remover
remover() {
    [ -n "$DISTRO" ] || DISTRO="$(detectar_distro)"
    local pacote_slack="" lista
    if [ -z "$DESTDIR" ]; then
        pacote_slack="$(ls /var/lib/pkgtools/packages /var/log/packages 2>/dev/null | grep -E '^mide-[0-9][^-]*-[^-]+-[^-]+$' | head -1 || true)"
    fi
    lista="${DESTDIR}$MANIFESTO"
    # Pacote do apt instalado por aqui (no mios, o mide é parte do sistema e fica).
    local pacote_apt=""
    [ -z "$DESTDIR" ] && pacote_instalado mide && ! pacote_instalado mios-release && pacote_apt=1
    if [ -z "$pacote_slack" ] && [ -z "$pacote_apt" ] && [ ! -f "$lista" ] && ! login_instalado && ! boot_tema_instalado; then
        info "$(t "The mide is not installed on this system (nothing to remove)." "O mide não está instalado neste sistema (nada para remover).")"
        exit 0
    fi
    titulo "$(t "Remove the mide from the system" "Remover o mide do sistema")"
    [ -n "$pacote_slack" ] && info "$(t "Slackware package" "pacote do Slackware"): $pacote_slack"
    [ -n "$pacote_apt" ] && info "$(t "the mide package (apt) and its repository" "o pacote mide (apt) e o repositório dele")"
    [ -f "$lista" ] && info "$(wc -l < "$lista") $(t "files listed in" "arquivos listados em") $lista"
    login_instalado && info "$(t "the mide login screen (the previous one comes back)" "a tela de login do mide (a de antes volta)")"
    boot_tema_instalado && info "$(t "the mios boot themes (the previous ones come back)" "os temas do boot do mios (os de antes voltam)")"
    info "$(t "Your settings (~/.config/mide) and files stay." "As suas configurações (~/.config/mide) e os seus arquivos ficam.")"
    pergunta "$(t "Remove?" "Remover?")" s || exit 0

    if login_instalado && [ -z "$DESTDIR" ]; then
        titulo "$(t "Login screen" "Tela de login")"
        root bash "$PROJETO/data/mide-login-install" --uninstall
    fi
    if boot_tema_instalado && [ -z "$DESTDIR" ]; then
        remover_tema_boot
    fi
    if [ -n "$pacote_apt" ]; then
        titulo "apt-get remove mide"
        root env DEBIAN_FRONTEND=noninteractive apt-get remove -y mide
        if [ -f "$APT_EXTRAS" ]; then
            root sh -c "xargs -r -d '\n' rm -f -- < '$APT_EXTRAS' && rm -f '$APT_EXTRAS'"
            root systemctl kill -s HUP systemd-logind 2>/dev/null || true
        fi
    fi
    if [ -n "$pacote_slack" ]; then
        titulo "removepkg"
        root /sbin/removepkg "$pacote_slack"
    fi
    if [ -f "$lista" ]; then
        titulo "$(t "Removing files" "Apagando os arquivos")"
        PREFIXO="${PREFIXO:-$(sed -n 's|^\(.*\)/bin/mide$|\1|p' "$lista" | head -1)}"
        local copia
        copia="$(mktemp)"
        cp "$lista" "$copia"
        echo "$MANIFESTO" >> "$copia"
        apagar_lista "$copia"
        rm -f "$copia"
    fi
    titulo "$(t "Done" "Pronto")"
    info "$(t "The packages installed as dependencies stay (remove them with the package manager if you want)." "Os pacotes instalados como dependências ficam (remova pelo gerenciador de pacotes se quiser).")"
}

# --------------------------------------------------------------------------- início
printf '%s%s%s\n' "$N" "mide $VERSAO — $(t "setup tool" "ferramenta de instalação") ($(nome_distro "$(detectar_distro)" || true), $ARQ)" "$Z"
[ -n "$SIMULAR" ] && info "$(t "dry run: nothing will be changed" "simulação: nada será mudado")"
if [ "$(id -u)" != 0 ] && [ -z "$DESTDIR" ] && [ -z "$SIMULAR" ] && ! command -v sudo >/dev/null; then
    falha "$(t "run as root (no sudo found)" "rode como root (não há sudo)")"
fi
if [ -z "$ACAO" ]; then
    escolha "$(t "What do you want to do?" "O que você quer fazer?")" 1 \
        "$(t "Install the mide on the system (or update it)" "Instalar o mide no sistema (ou atualizar)")" \
        "$(t "Update the open mide session from this folder (developers)" "Atualizar a sessão do mide aberta a partir desta pasta (desenvolvimento)")" \
        "$(t "Only build (target/release)" "Só compilar (target/release)")" \
        "$(t "Only install what is needed to build" "Só instalar o que falta para compilar")" \
        "$(t "mide login screen (greetd)" "Tela de login do mide (greetd)")" \
        "$(t "mios boot themes (GRUB menu and Plymouth animation)" "Temas do boot do mios (menu do GRUB e animação do Plymouth)")" \
        "$(t ".deb packages (mide and mios-release)" "Pacotes .deb (mide e mios-release)")" \
        "$(t "mios ISO (live system with the installer)" "ISO do mios (sistema ao vivo com o instalador)")" \
        "$(t "Try the ISO in a virtual machine (QEMU)" "Testar a ISO numa máquina virtual (QEMU)")" \
        "$(t "Remove the mide from the system" "Remover o mide do sistema")" \
        "$(t "Quit" "Sair")"
    case "$ESCOLHA" in
        1) ACAO=instalar ;; 2) ACAO=sessao ;; 3) ACAO=compilar ;; 4) ACAO=dependencias ;;
        5) ACAO=menu-login ;; 6) ACAO=menu-boot ;; 7) ACAO=pacotes ;; 8) ACAO=iso ;;
        9) ACAO=iso-teste ;; 10) ACAO=remover ;; *) exit 0 ;;
    esac
fi
case "$ACAO" in
    instalar) instalar ;;
    remover) remover ;;
    sessao) atualizar_sessao ;;
    compilar) so_compilar ;;
    dependencias) so_dependencias ;;
    login | menu-login) menu_login ;;
    boot | menu-boot) menu_boot ;;
    pacotes) fazer_pacotes ;;
    iso) fazer_iso ;;
    iso-teste) testar_iso ;;
esac
