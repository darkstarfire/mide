# mide

**Em construção.**

O mide é um ambiente de desktop para Linux, moderno e leve, feito em Rust sobre
Wayland. Ele reúne o compositor e os componentes do dia a dia: dock, barra superior,
launcher, buscador, notificações, Configurações e um gerenciador de arquivos.

**Objetivo:** oferecer um desktop bonito, simples de usar e rápido, com visual limpo
e efeitos de vidro, que funcione bem tanto em computadores comuns quanto em máquinas
modestas, e que seja fácil de usar com mouse, teclado ou tela de toque.

**Para que serve:** para usar o computador no dia a dia (abrir apps, organizar
janelas e arquivos, ajustar o sistema) num ambiente coeso, em que todas as partes
seguem o mesmo estilo e o mesmo comportamento.

**Apps que precisam de administrador:** num atalho `.desktop` (ou num script sem
terminal), use o `mide-sudo`, que pede a senha numa janela do mide e roda o programa
como root (dentro de um terminal, é o `sudo` de sempre):

```ini
Exec=mide-sudo --name XAMPP /opt/lampp/manager-linux-x64.run
```

O `pkexec` e os apps que pedem permissão pelo polkit usam a mesma janela na sessão do
mide. O programa roda pelo X11 (XWayland), sem acesso ao Wayland nem à sessão D-Bus do
usuário; use só com programas confiáveis.

## Instalação manual (para testar)

O instalador automático ainda não existe. Para testar agora, compile o projeto e entre
na sessão do mide pela tela de login. Funciona em Debian, Ubuntu, Zorin, Mint, Fedora e
Arch, com placa de vídeo Intel ou AMD (drivers Mesa); a placa da NVIDIA com o driver
proprietário ainda não foi testada.

**1. Rust** (o compilador da linguagem do mide):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

**2. Pacotes do sistema** (as bibliotecas para compilar e os programas que o mide usa):

```sh
# Debian, Ubuntu, Zorin, Mint
sudo apt install build-essential git pkg-config libinput-dev libseat-dev libudev-dev \
  libgbm-dev libdrm-dev libxkbcommon-dev libpam0g-dev libwayland-dev libegl-dev \
  libglib2.0-dev libgudev-1.0-dev xwayland xdg-desktop-portal xdg-desktop-portal-gtk \
  xdg-desktop-portal-wlr pipewire wireplumber ffmpeg curl adwaita-icon-theme \
  dbus-user-session x11-xserver-utils

# Fedora
sudo dnf install gcc git pkgconf-pkg-config libinput-devel libseat-devel systemd-devel \
  mesa-libgbm-devel libdrm-devel libxkbcommon-devel pam-devel wayland-devel \
  mesa-libEGL-devel glib2-devel libgudev-devel xorg-x11-server-Xwayland \
  xdg-desktop-portal xdg-desktop-portal-gtk xdg-desktop-portal-wlr pipewire wireplumber \
  ffmpeg curl adwaita-icon-theme xhost

# Arch
sudo pacman -S base-devel git libinput seatd mesa libdrm libxkbcommon pam wayland glib2 \
  libgudev xorg-xwayland xdg-desktop-portal xdg-desktop-portal-gtk xdg-desktop-portal-wlr \
  pipewire wireplumber ffmpeg curl adwaita-icon-theme xorg-xhost
```

Os nomes foram conferidos no Debian e no Ubuntu; no Fedora e no Arch podem variar um
pouco (o `cargo build` diz qual biblioteca falta).

**3. Apps X11 dentro do mide** (opcional; sem isso, só os apps Wayland abrem):

```sh
sudo apt install libxcb-cursor-dev clang   # Debian/Ubuntu
cargo install --git https://github.com/Supreeeme/xwayland-satellite
```

**4. Baixar e compilar o mide** (leva alguns minutos):

```sh
git clone https://github.com/fabioaacarneiro/mide.git
cd mide
cargo build --release
```

**5. Pôr o mide na tela de login** (a entrada "mide (desenvolvimento)" aponta para esta
pasta) e o arquivo da tela de bloqueio:

```sh
sudo tests/session-dev.sh
sudo install -m644 data/pam/mide-lock /etc/pam.d/mide-lock
```

**6. Entrar:** saia da sessão atual e escolha "mide (desenvolvimento)" na tela de login
(no GDM, na engrenagem; no LightDM e no SDDM, no menu de sessões). Já dentro do mide,
abra o Terminal (Ctrl+Shift+T) e rode uma vez, na pasta do projeto:

```sh
data/mide-update --all
```

Ele instala o tema de ícones do mide e reinicia o compositor. Depois, saia e entre de
novo. Para conferir o que falta: `data/mide-check`.

**Atualizar** para a versão mais nova: `git pull` e, dentro do mide, `data/mide-update
--all`. **Testar sem sair da sessão atual:** `MIDE_BACKEND=winit target/release/mide`
abre o mide numa janela. **Se algo der errado:** o log fica em
`~/.local/state/mide/session.log`. **Remover:** `sudo tests/session-dev.sh remover` e
apagar a pasta do projeto (as configurações ficam em `~/.config/mide`).

---

O mide é criado do zero. Não há engenharia reversa de nenhum software da Apple ou do
macOS, e o projeto não usa código, imagens, ícones ou fontes deles. O visual do macOS
serve apenas como inspiração; o mide não é concorrente nem cópia do macOS. "Apple" e
"macOS" são marcas da Apple Inc., sem relação com este projeto.

Os ícones vêm do [mide-icons](https://github.com/fabioaacarneiro/mide-icons), um fork
do [WhiteSur icon theme](https://github.com/vinceliuice/WhiteSur-icon-theme) (de Vince
Liuice, GPL-3.0) sem o logo da Apple e com desenhos próprios no lugar dos que recriavam
ícones de apps da Apple. O `data/mide-update` instala o tema.
