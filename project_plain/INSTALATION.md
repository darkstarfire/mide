# Como deve ser para instalar o compositor

- deve ter um script bash que deve ser rodado com sudo
- deve analisar o linux, se é fedora, ubuntu, debian, arch
- deve instalar as dependências sozinho para compilar o projeto
- deve compilar o projeto e perguntar se quer instalar
- instalar se confirmado

## Dependências de execução já definidas

- **Obrigatórias, porque sustentam recursos que o desktop promete** (decisão do usuário em 2026-10-21: sem elas, parece que o mide não funciona). O instalador instala sempre, e o `data/mide-conferir` acusa FALTA se não estiverem:
  - `xdg-desktop-portal-wlr` + PipeWire: gravar e compartilhar a tela pelos apps (OBS, navegador em videochamada) (A19).
  - `ffmpeg`: gravar a tela em vídeo, com áudio, pela ferramenta de captura do mide (A21). Só codifica e salva; a captura das imagens e do som é do mide.

- `Xwayland` (pacote `xwayland`) e `xwayland-satellite`, para abrir apps X11 dentro do mide (A14). O satellite pode vir do pacote da distribuição, quando houver, ou de `cargo install --git https://github.com/Supreeeme/xwayland-satellite`. O instalador deve colocar o satellite no PATH (ou ao lado do binário do mide); sem ele, o mide funciona, só sem apps X11.
- Tema de cursores (ex.: `adwaita-icon-theme`), para o cursor do mide no console (A13); sem tema, o mide desenha uma seta própria.

- **Bibliotecas do sistema usadas pelos binários** (medido com `ldd` em 2026-10-20): `libinput`, `libseat`, `libudev`/`libgudev`, `libgbm` + `libdrm` (Mesa), `libEGL`/GLES (Mesa, carregado na hora), `libxkbcommon`, `libsystemd`, `glib` e as que elas puxam (`libevdev`, `libmtdev`, `libwacom`). As **dependências do Rust (crates) ficam dentro dos binários**: quem só instala não precisa de Rust nem de `cargo`.
- **Programas usados em tempo de execução:** D-Bus (sessão), `systemd --user`, `xdg-desktop-portal` + `xdg-desktop-portal-gtk` (configurações do sistema para os apps GTK), `xdg-desktop-portal-wlr` + PipeWire (gravar e compartilhar a tela: OBS, navegador em videochamada), `dbus-update-activation-environment`, fontes do sistema (fontconfig), tema de ícones do mide (`mide-icons`, fork do WhiteSur: o `mide-atualizar` baixa e instala em `~/.local/share/icons`; precisa de `git` e rede na primeira vez); recomendados: `qt5-gtk-platformtheme` e `qt6-gtk-platformtheme` (apps Qt 5 e Qt 6 seguem o tema de ícones e o claro/escuro do mide); opcionais: `foot` (terminal), `nmcli`, `bluetoothctl`, `wpctl`, `loginctl`.
- Para compilar o `xwayland-satellite` no Debian/Ubuntu: `libxcb-cursor-dev` e `clang` (além de `libxcb1-dev` e `pkg-config`).

## Estratégia de instalação (sem compilar na máquina do usuário)

- O script de instalação baixa os **binários prontos** (release do GitHub, por arquitetura) e instala com o gerenciador da distribuição só as bibliotecas e programas de execução acima (são pacotes comuns de qualquer desktop: em Debian/Ubuntu, algo como `libinput10 libseat1 libgbm1 libegl1 libxkbcommon0 xdg-desktop-portal-gtk dbus-user-session`). Compilar do código fica como opção para desenvolvimento (aí sim com Rust e os pacotes `-dev`).
- Instala: binários em `/usr/local/bin` (ou `/usr/bin` no pacote), dados em `/usr/share/mide` (`applications`, `xdg-desktop-portal/mide-portals.conf`), a sessão em `/usr/share/wayland-sessions/mide.desktop` e o `mide-session`.
- Nada novo para instalar nesta etapa (vidro das pastas do launcher e guia de identidade visual são do próprio mide). Para rodar `tests/ui.sh`: `gnome-calculator`, `foot`, `xclock` (opcional) e Python 3 com Pillow.
- **Tela de bloqueio:** instalar `data/pam/mide-lock` em `/etc/pam.d/mide-lock` (o `mide-lock` confere a senha por ele; sem o arquivo, usa o serviço `login`). Biblioteca de execução: `libpam0g` (já vem em qualquer sistema com login).

