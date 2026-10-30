# Como deve ser para instalar o compositor

- deve ter um script bash que deve ser rodado com sudo
- deve analisar o linux, se é fedora, ubuntu, debian, arch
- deve instalar as dependências sozinho para compilar o projeto
- deve compilar o projeto e perguntar se quer instalar
- instalar se confirmado

## Dependências de execução já definidas

- **Obrigatórias, porque sustentam recursos que o desktop promete** (decisão do usuário em 2026-10-21: sem elas, parece que o mide não funciona). O instalador instala sempre, e o `data/mide-check` acusa FALTA se não estiverem:
  - `xdg-desktop-portal-wlr` + PipeWire: gravar e compartilhar a tela pelos apps (OBS, navegador em videochamada) (A19).
  - `curl`: buscar as cotações das moedas do Banco Central na Calculadora (só quando a pessoa usa moeda e deixa buscar na internet). Sem ele, a conversão de moedas usa a última cotação guardada ou fica sem cotação; o resto funciona. O Ubuntu Desktop pode não trazer: o instalador confere.
  - `ffmpeg`: gravar a tela em vídeo, com áudio, pela ferramenta de captura do mide (A21). Só codifica e salva; a captura das imagens e do som é do mide.

- `Xwayland` (pacote `xwayland`) e `xwayland-satellite`, para abrir apps X11 dentro do mide (A14). O satellite pode vir do pacote da distribuição, quando houver, ou de `cargo install --git https://github.com/Supreeeme/xwayland-satellite`. O instalador deve colocar o satellite no PATH (ou ao lado do binário do mide); sem ele, o mide funciona, só sem apps X11.
- Tema de cursores (ex.: `adwaita-icon-theme`), para o cursor do mide no console (A13); sem tema, o mide desenha uma seta própria.

- **Bibliotecas do sistema usadas pelos binários** (medido com `ldd` em 2026-10-20): `libinput`, `libseat`, `libudev`/`libgudev`, `libgbm` + `libdrm` (Mesa), `libEGL`/GLES (Mesa, carregado na hora), `libxkbcommon`, `libsystemd`, `glib` e as que elas puxam (`libevdev`, `libmtdev`, `libwacom`). As **dependências do Rust (crates) ficam dentro dos binários**: quem só instala não precisa de Rust nem de `cargo`.
- **Programas usados em tempo de execução:** D-Bus (sessão), `systemd --user`, `xdg-desktop-portal` + `xdg-desktop-portal-gtk` (configurações do sistema para os apps GTK), `xdg-desktop-portal-wlr` + PipeWire (gravar e compartilhar a tela: OBS, navegador em videochamada), `dbus-update-activation-environment`, `lsblk` (`util-linux`, detectar volumes removíveis ainda desmontados na Mesa), `udisksctl` (`udisks2`, montar/ejetar volumes com a autorização do sistema), fontes do sistema (fontconfig), tema de ícones do mide (`mide-icons`, fork do WhiteSur: o `mide-update` baixa e instala em `~/.local/share/icons`; precisa de `git` e rede na primeira vez); recomendados: `qt5-gtk-platformtheme` e `qt6-gtk-platformtheme` (apps Qt 5 e Qt 6 seguem o tema de ícones e o claro/escuro do mide); opcionais: `foot` (terminal), `nmcli`, `bluetoothctl`, `wpctl`, `loginctl`.
- Para compilar o `xwayland-satellite` no Debian/Ubuntu: `libxcb-cursor-dev` e `clang` (além de `libxcb1-dev` e `pkg-config`).

## Estratégia de instalação (sem compilar na máquina do usuário)

- O script de instalação baixa os **binários prontos** (release do GitHub, por arquitetura) e instala com o gerenciador da distribuição só as bibliotecas e programas de execução acima (são pacotes comuns de qualquer desktop: em Debian/Ubuntu, algo como `libinput10 libseat1 libgbm1 libegl1 libxkbcommon0 xdg-desktop-portal-gtk dbus-user-session`). Compilar do código fica como opção para desenvolvimento (aí sim com Rust e os pacotes `-dev`).
- Instala: binários em `/usr/local/bin` (ou `/usr/bin` no pacote), dados em `/usr/share/mide` (`applications`, `xdg-desktop-portal/mide-portals.conf`), a sessão em `/usr/share/wayland-sessions/mide.desktop` e o `mide-session`.
- **Seletor de arquivos do mide nos apps (B25):** instalar `data/xdg-desktop-portal/portals/mide.portal` em `/usr/share/xdg-desktop-portal/portals/mide.portal` (o `xdg-desktop-portal` só lê os `.portal` dali; tem `UseIn=mide` e o `x-cinnamon-portals.conf` escolhe xapp/gtk, então não muda o Cinnamon nem o GNOME). Sem ele (desenvolvimento, sem sudo), a sessão do mide monta uma pasta de links e publica `XDG_DESKTOP_PORTAL_DIR`. Programas novos: `mide-portal` e `mide-chooser`, junto com os outros binários. A ativação do backend pelo D-Bus vai em `/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.mide.service` (`Exec=/usr/bin/mide-portal --portal-only`; só é chamada com `XDG_CURRENT_DESKTOP=mide`). **Nunca** instalar arquivo de ativação para `org.freedesktop.FileManager1`: valeria em todos os desktops; o compositor pede esse nome só na sessão do mide.
- **Apps padrão:** nada é gravado no `mimeapps.list` geral nem em `/usr/share/applications/mimeapps.list`; o mide usa `~/.config/mide-mimeapps.list` (criado ao entrar na sessão) e `/usr/share/mide/applications/mide-mimeapps.list` (reserva), lidos só com `XDG_CURRENT_DESKTOP=mide`.
- Nada novo para instalar nesta etapa (vidro das pastas do launcher e guia de identidade visual são do próprio mide). Para rodar `tests/ui.sh`: `gnome-calculator`, `foot`, `xclock` (opcional) e Python 3 com Pillow.
- **Tela de bloqueio:** instalar `data/pam/mide-lock` em `/etc/pam.d/mide-lock` (o `mide-lock` confere a senha por ele; sem o arquivo, usa o serviço `login`). Biblioteca de execução: `libpam0g` (já vem em qualquer sistema com login).

- **Senha de administrador gráfica (C7):** programas novos `mide-sudo`, `mide-askpass` e `mide-polkit-agent`, junto com os outros binários (o `mide-sudo` procura o `mide-askpass` ao lado dele). Precisam do `sudo` (com o usuário no grupo `sudo` ou `wheel`) e do polkit com o `polkit-agent-helper-1` setuid de root (pacote `polkitd`/`polkit-1`; procurado em `/usr/lib/polkit-1`, `/usr/libexec`, `/usr/lib/policykit-1` e `/usr/libexec/polkit-1`); `xhost` (pacote `x11-xserver-utils`) para apps root abrirem no XWayland do mide. Nada em `/etc`: o agente se registra sozinho na sessão do mide e o `mide-sudo` usa a regra padrão do sudoers. Para apps em `.desktop` que precisam de root: `Exec=mide-sudo --name XAMPP /opt/lampp/manager-linux-x64.run` (sem terminal, a senha é pedida numa janela do mide; dentro de um terminal, é o sudo normal). **Riscos documentados:** (1) o app roda como root pelo X11 (XWayland), com os poderes de um cliente X sobre as outras janelas X11 (os apps Wayland ficam isolados); e o contrário também vale: **qualquer app X11 aberto na sessão** consegue mandar teclas ao app root (XTEST/XSendEvent) e ler o que ele mostra. Usar só com programas confiáveis e, de preferência, sem apps X11 desconhecidos abertos. (2) A janela de senha pode ser imitada: qualquer app pode abrir uma janela parecida (ou chamar `mide-sudo --name "Atualizações do sistema" ...`); o mide ainda não tem um sinal confiável na tela (fica para depois). (3) O teclado virtual (`zwp_virtual_keyboard`, usado pelo `wtype` e por teclados na tela) aceita qualquer app: um app pode digitar senhas erradas na janela e gastar as tentativas (contando no `pam_faillock`), mas não consegue ler a senha. (4) A tela X11 fica liberada para o root (`xhost +si:localuser:root`) só enquanto algum `mide-sudo` roda; se ele for morto com SIGKILL, a liberação fica até o próximo `mide-sudo` terminar ou o XWayland reiniciar. (5) Sessão com o mide de desenvolvimento (`target/debug`, quando não há release): as variáveis de teste (`MIDE_SUDO_BIN`, `MIDE_POLKIT_HELPER`, `MIDE_POLKIT_BUS`) passam a valer; só quem já controla o ambiente do compositor consegue usar, mas a instalação de verdade deve ser sempre em release. (6) Polkit novo com o `polkit-agent-helper-1` pelo socket do systemd (sem setuid): ainda não suportado pelo agente do mide (o log diz isso); o polkit 126 do Debian 13 ainda é setuid.

- **Diagnóstico:** `mide-logs` é instalado junto aos executáveis do mide e mostra o log combinado de todos os processos; o arquivo rotativo fica em `~/.local/state/mide/logs/mide.log` (ou sob `XDG_STATE_HOME`). Consulte `docs/logs-e-diagnostico.md` para busca, erros e limites.

## Tela de login do mide (greetd + mide-login, C5)

A tela de login do mide é um greeter do `greetd`: o greetd confere a senha pelo PAM do
sistema e inicia a sessão escolhida; o mide só mostra a tela (compositor em modo de
login, `mide --login`, com o `mide-login`).

Instalar (depois de compilar, de dentro do projeto):

```sh
cargo build --release
sudo data/mide-login-install
```

O script, explicando cada passo:

1. instala o pacote `greetd` se faltar (apt, dnf ou pacman);
2. copia `mide` e `mide-login` para `/usr/local/lib/mide-login` (o usuário do greeter
   não lê a pasta pessoal; rode o script de novo depois de atualizar o mide);
3. copia a fonte da interface para `/usr/local/share/mide/fonts`;
4. copia a configuração de quem rodou o sudo (telas, escala, layout de teclado, papel
   de parede) para `/etc/mide/login`, para a tela de login abrir igual;
5. escreve `/etc/greetd/config.toml` (cópia de segurança do anterior em
   `/var/lib/mide-login`), com o mesmo terminal virtual de antes;
6. desativa o gerenciador de login atual (GDM, LightDM, SDDM...) e ativa o greetd (com
   systemd; nos outros sistemas de inicialização, o script diz o que fazer).

Vale no próximo boot. Para voltar ao gerenciador de antes:

```sh
sudo data/mide-login-install --uninstall
```

Se o login falhar, entre num terminal de texto (Ctrl+Alt+F3) e rode o `--uninstall`.
As sessões listadas vêm de `/usr/share/wayland-sessions` e `/usr/share/xsessions` (as do
X11 sobem pelo `startx`); a do mide é a padrão.
