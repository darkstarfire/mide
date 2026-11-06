#!/usr/bin/env bash
# Instalador do mide (C3 e C6): interativo, para Debian/Ubuntu, Fedora, Arch/Manjaro e
# Slackware. Instala as dependências pelo gerenciador de pacotes da distribuição,
# compila o mide (como o usuário, nunca como root), monta os arquivos numa pasta à
# parte e só então pergunta antes de copiar para o sistema. Também remove o que
# instalou.
#
#   ./install.sh                 menu: instalar, remover ou sair
#   ./install.sh --install       instala direto (ainda pergunta a distribuição e as partes)
#   ./install.sh --remove        remove do sistema
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
        --help | -h) ajuda; exit 0 ;;
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
else
    USUARIO="$(id -un)"
fi
UHOME="$(getent passwd "$USUARIO" | cut -d: -f6)"
UHOME="${UHOME:-$HOME}"

mostra() { printf '    %s$ %s%s\n' "$A" "$*" "$Z"; }

# Comando como root (sudo quando preciso). Em --dry-run, só mostra.
root() {
    if [ -n "$SIMULAR" ]; then mostra "$*"; return 0; fi
    if [ "$(id -u)" = 0 ]; then "$@"; else sudo "$@"; fi
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
        debian) echo build-essential git curl pkg-config libinput-dev libseat-dev libudev-dev libgbm-dev \
            libdrm-dev libxkbcommon-dev libpam0g-dev libwayland-dev libegl-dev libglib2.0-dev libgudev-1.0-dev ;;
        fedora) echo gcc git curl pkgconf-pkg-config libinput-devel libseat-devel systemd-devel mesa-libgbm-devel \
            libdrm-devel libxkbcommon-devel pam-devel wayland-devel mesa-libEGL-devel glib2-devel libgudev-devel ;;
        arch) echo base-devel git curl libinput seatd mesa libdrm libxkbcommon pam wayland glib2 libgudev ;;
        # Instalação completa do Slackware: as bibliotecas já trazem os cabeçalhos.
        slackware) echo gcc make pkg-config git curl libinput libxkbcommon mesa libdrm wayland glib2 libgudev \
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
    usuario_muda nice -n 10 cargo build --release --locked
    # Pronto pelo gerenciador de pacotes (Arch, Fedora novo): não precisa compilar.
    if [ "$X11" = s ] && [ ! -x /usr/bin/xwayland-satellite ]; then
        titulo "$(t "Building xwayland-satellite (X11 apps)" "Compilando o xwayland-satellite (apps X11)")"
        usuario_muda nice -n 10 cargo install --locked --git "$SATELLITE_GIT" --root "$PROJETO/target/install/satellite"
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
    # Programas: o compositor procura os auxiliares (barra, dock...) ao lado dele.
    for f in target/release/mide target/release/mide-*; do
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
    (cd "$fonte" && ./install.sh -n mide -d "$destino" >/dev/null 2>&1)
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

# ------------------------------------------------------------------------ instalar
instalar() {
    escolher_distro
    [ -n "$PREFIXO" ] || { if [ "$DISTRO" = slackware ]; then PREFIXO=/usr; else PREFIXO=/usr/local; fi; }
    [ -n "$DESTDIR" ] && [ -z "$LOGIN" ] && LOGIN=n

    printf '\n%s%s%s\n' "$N" "$(t "What to install" "O que instalar")" "$Z"
    info "$(t "mide: compositor, bar, dock, launcher, apps and the login-screen session (always)" "mide: compositor, barra, dock, launcher, apps e a sessão na tela de login (sempre)")"
    [ -n "$X11" ] || { pergunta "$(t "X11 apps inside the mide (xwayland-satellite)?" "Apps X11 dentro do mide (xwayland-satellite)?")" s && X11=s || X11=n; }
    [ -n "$ICONES" ] || { pergunta "$(t "The mide icon theme (needs internet)?" "O tema de ícones do mide (precisa de internet)?")" s && ICONES=s || ICONES=n; }
    [ -n "$ENERGIA" ] || { pergunta "$(t "Let the mide handle the lid and the power key (it asks before sleeping)?" "Deixar a tampa e o botão de ligar com o mide (ele pergunta antes de dormir)?")" s && ENERGIA=s || ENERGIA=n; }
    [ -n "$LOGIN" ] || { pergunta "$(t "Replace the login screen with the mide one (greetd)? You can go back with --remove" "Trocar a tela de login pela do mide (greetd)? Dá para voltar com --remove")" n && LOGIN=s || LOGIN=n; }

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

    if [ "$ENERGIA" = s ] && [ -z "$DESTDIR" ]; then
        if [ "$DISTRO" = slackware ] || ! command -v systemctl >/dev/null; then root pkill -HUP -x elogind-daemon || true
        else root systemctl kill -s HUP systemd-logind || true; fi
    fi
    [ "$LOGIN" = s ] && [ -z "$DESTDIR" ] && instalar_login
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
    if [ -z "$pacote_slack" ] && [ ! -f "$lista" ] && ! login_instalado; then
        info "$(t "The mide is not installed on this system (nothing to remove)." "O mide não está instalado neste sistema (nada para remover).")"
        exit 0
    fi
    titulo "$(t "Remove the mide from the system" "Remover o mide do sistema")"
    [ -n "$pacote_slack" ] && info "$(t "Slackware package" "pacote do Slackware"): $pacote_slack"
    [ -f "$lista" ] && info "$(wc -l < "$lista") $(t "files listed in" "arquivos listados em") $lista"
    login_instalado && info "$(t "the mide login screen (the previous one comes back)" "a tela de login do mide (a de antes volta)")"
    info "$(t "Your settings (~/.config/mide) and files stay." "As suas configurações (~/.config/mide) e os seus arquivos ficam.")"
    pergunta "$(t "Remove?" "Remover?")" s || exit 0

    if login_instalado && [ -z "$DESTDIR" ]; then
        titulo "$(t "Login screen" "Tela de login")"
        root bash "$PROJETO/data/mide-login-install" --uninstall
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
printf '%s%s%s\n' "$N" "mide $VERSAO — $(t "installer" "instalador")" "$Z"
[ -n "$SIMULAR" ] && info "$(t "dry run: nothing will be changed" "simulação: nada será mudado")"
if [ "$(id -u)" != 0 ] && [ -z "$DESTDIR" ] && [ -z "$SIMULAR" ] && ! command -v sudo >/dev/null; then
    falha "$(t "run as root (no sudo found)" "rode como root (não há sudo)")"
fi
if [ -z "$ACAO" ]; then
    escolha "$(t "What do you want to do?" "O que você quer fazer?")" 1 \
        "$(t "Install the mide on the system (or update it)" "Instalar o mide no sistema (ou atualizar)")" \
        "$(t "Remove the mide from the system" "Remover o mide do sistema")" \
        "$(t "Quit" "Sair")"
    case "$ESCOLHA" in 1) ACAO=instalar ;; 2) ACAO=remover ;; *) exit 0 ;; esac
fi
case "$ACAO" in
    instalar) instalar ;;
    remover) remover ;;
esac
