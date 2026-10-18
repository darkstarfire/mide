# Recursos e bibliotecas do mide

Lista de crates, protocolos e serviços para o projeto, organizada a partir do `CONTEXT.md`. Confira as versões atuais no crates.io antes de instalar.

---

## 1. Núcleo do compositor (Wayland e hardware)

| Crate | Para quê |
|---|---|
| `smithay` | Framework para construir compositors. Traz backends DRM/KMS, EGL/GLES, input, sessão e protocolos Wayland. É a base do projeto. |
| `wayland-server`, `wayland-client`, `wayland-protocols` | Bindings oficiais do Wayland em Rust, para protocolos padrão e de extensão. |
| `wayland-protocols-wlr` | Protocolos do wlroots: layer-shell (barra, dock, desktop), output-management (monitores) e screencopy. |
| `smithay-client-toolkit` (SCTK) | Clientes do DE: barra superior, dock, launcher, desktop e notificações como janelas layer-shell. |
| `calloop` | Event loop usado pelo Smithay. |
| `input` | Bindings do libinput para teclado, mouse e touchpad (gestos e configurações). |
| `drm`, `gbm` | DRM/KMS com atomic modesetting, page flip sincronizado (evita screen tearing) e buffers da GPU. |
| `khronos-egl`, `gl` ou `glow` | EGL e OpenGL ES para shaders de blur, transparência, sombras e efeito lâmpada. |
| `xkbcommon` | Layouts de teclado e atalhos. |
| `xcursor` | Carrega o tema do cursor. |
| `x11rb` | Xwayland e apps X11 legados. |

---

## 2. Renderização e texto (visual estilo macOS)

| Crate | Para quê |
|---|---|
| `tiny-skia` | Renderização 2D vetorial em CPU. Bordas, sombras e ícones simples. |
| `vello` | Renderização vetorial em GPU, para interfaces mais pesadas. |
| `skia-safe` | Bindings do Skia, se quiser o motor de desenho de navegadores. Mais pesado de compilar. |
| `cosmic-text` | Layout e quebra de texto com fontes do sistema. |
| `rustybuzz`, `fontdb`, `ttf-parser` | Shaping e carregamento de fontes, se preferir camadas menores. |
| `resvg` | Renderização de SVG para ícones. |
| `image` | PNG, JPG, wallpapers e thumbnails. |
| `lyon` | Tesselação de caminhos vetoriais, útil para widgets e para o efeito lâmpada. |
| `palette` | Conversão de cores e variações de cor do tema. |

---

## 3. Toolkit de UI (barra, dock, launcher, configurações, arquivos)

| Crate | Para quê |
|---|---|
| `iced` | Toolkit em Rust com arquitetura Elm. Alta produtividade. Usado pelo COSMIC. |
| `slint` | Toolkit declarativo com linguagem própria. Bom para painéis e telas de configuração. |
| `egui` | Simples, mas com aparência pouco nativa. Use só em protótipos. |

Para o visual macOS, comece com `iced` ou `slint` e customize o tema. Se quiser controle total, desenhe os widgets sobre `tiny-skia` ou `vello`.

---

## 4. Animação e efeitos visuais

- `keyframe` ou `interpolation`, se quiser crates prontos.
- Implementação própria é o mais indicado: timer por frame, curvas de easing (cubic-bezier, spring) e sincronização com o `frame callback` do Wayland.
- **Movimentação suave:** animações de janelas, workspaces, dock e launcher.
- **Efeito lâmpada ao minimizar e restaurar:** deformação de malha em shader GLES (`glow`), com malha gerada por `lyon` se preciso.
- **Transparência e blur:** pass de blur (dual-kawase ou gaussiano) em framebuffer separado, com cache para não recalcular a cada frame.
- **Desligar efeitos:** cada efeito precisa de uma flag na configuração, lida pelo compositor em tempo real.

---

## 5. Integração com o sistema (D-Bus e serviços)

| Crate ou serviço | Para quê |
|---|---|
| `zbus` | Cliente e servidor D-Bus. Base de quase toda integração abaixo. |
| `ashpd` | Cliente das APIs de `xdg-desktop-portal`. Também pode ser usado para implementar backends de portal. |
| `pipewire` | Captura de tela, áudio e volume. |
| `bluer` | Cliente BlueZ para Bluetooth (ou `zbus` direto no `org.bluez`). |
| NetworkManager via `zbus` | Wi-Fi e rede (`org.freedesktop.NetworkManager`). |
| UPower via `zbus` | Bateria no tray e na barra superior. |
| logind via `zbus` | Suspender, desligar, reiniciar e troca de sessão. |
| systemd-localed e timedated via `zbus` | Idioma, layout de teclado e fuso horário. |
| `freedesktop-desktop-entry` | Leitura dos `.desktop` dos apps, usada pela dock e pelo launcher. |
| `freedesktop-icons` | Ícones do tema do sistema. |
| `xdg` | Diretórios XDG (config, cache, dados, Downloads). |
| `trash` | Lixeira segundo a especificação freedesktop (listar, restaurar, esvaziar). |
| `notify` | Observar `~/Downloads`, arquivos de configuração e a pasta do desktop. |
| `sysinfo`, `procfs` | Bateria, CPU, memória, rede e informações do sistema para a janela "Sobre este Mac". |
| `udev` | Hardware (monitores, dispositivos de entrada, Bluetooth) para a janela de especificações. |

### Protocolos de tray, menus e notificações

- **Tray (lado direito da barra):** StatusNotifierItem via D-Bus (`org.kde.StatusNotifierWatcher`). Implemente o watcher no próprio DE e aceite ícones de apps como volume, Wi-Fi e Bluetooth.
- **Menus globais (menus dos apps na barra superior):** `com.canonical.AppMenu.Registrar` e `com.canonical.dbusmenu`, via `zbus`. Assim, os apps exportam o menu e a barra o exibe.
- **Notificações (estilo dunst, integradas ao visual):** o DE precisa ser o servidor `org.freedesktop.Notifications`, implementado com `zbus`. `notify-rust` é só cliente, então não serve aqui.

---

## 6. Base geral

- `tokio` ou `async-std`, se precisar de async fora do event loop principal.
- `serde` com `toml` ou `serde_json`, para configuração e para persistir a dock, os grupos do launcher e os atalhos.
- `clap`, para CLI e ferramentas de debug.
- `tracing` e `tracing-subscriber`, para logs estruturados.
- `nix` e `libc`, para chamadas de sistema específicas.
- `anyhow` ou `thiserror`, para tratamento de erros.
- `fluent` ou `fluent-bundle`, para a localização de idioma definida nas configurações.

---

## 7. Mapeamento por requisito do CONTEXT.md

### Barra superior
- Menu Apple à esquerda com "Sobre este Mac" (`sysinfo`, `procfs`, `udev`) e ações de sistema (logind).
- Menu do app focado: menus globais via `com.canonical.dbusmenu` (ver seção 5).
- Lado direito: tray (StatusNotifierItem), relógio, controle de volume (`pipewire`), Wi-Fi, Bluetooth e bateria.
- Layer-shell com `anchor` no topo e área exclusiva reservada.

### Dock
- Ícones fixos a partir de `.desktop` (`freedesktop-desktop-entry`), persistidos em `toml`.
- Arrastar para reordenar e para fixar e desafixar: eventos de ponteiro no SCTK.
- Ícones fixos do launcher, lixeira e Downloads ficam na dock, na ordem pedida (launcher, Finder, configurações, lixeira, Downloads).

### Launcher em tela cheia
- Overlay layer-shell em tela cheia.
- Campo de busca com fuzzy matching: `nucleo` ou `fuzzy-matcher`.
- Agrupamento automático por categoria do `.desktop` (`Categories=`: Office, System, etc.) e pastas criadas pelo usuário, persistidas em `toml`.
- Grupos fixos "Ferramentas do sistema" e "Escritório".

### Gerenciador de arquivos (estilo Finder)
- Listagem e operações com `std::fs` e `walkdir`.
- Lixeira com `trash`.
- Abertura de arquivos por tipo MIME (`mime`, `mime_guess`) com `ashpd` (OpenURI) ou `xdg-open`.
- Miniaturas (`image`, com cache seguindo a spec de thumbnails do freedesktop).
- Observação de pastas com `notify`.

### Lixeira e Downloads (dock)
- Lixeira: menu com "Esvaziar lixeira" e "Abrir no gerenciador de arquivos" (`trash`).
- Downloads: pasta de `xdg` observada com `notify`, com pop-up em lista ao clicar.

### Notificações
- Servidor D-Bus (`zbus`) e renderização como popups layer-shell com o tema do DE.
- Regras e tempo de exibição em `toml`, com opções parecidas com as do dunst.

### Configurações do sistema
- **Monitores:** `wlr-output-management` (`wayland-protocols-wlr`) e backend DRM do Smithay.
- **Mouse e touchpad:** configurações do libinput via `input` e Smithay.
- **Teclado:** `xkbcommon` para layouts e atalhos. Atalhos padrão para DE, janelas e apps, editáveis e com atalhos personalizados que executam binário ou script.
- **Efeitos visuais:** flags de configuração lidas pelo compositor.
- **Papel de parede:** `image` e textura GLES, com regra por monitor.
- **Ícones e pastas da área de trabalho:** layer-shell em background, `freedesktop-icons`, arrastar e soltar e menu de contexto.
- **Bluetooth:** `bluer` ou `zbus` com `org.bluez`.
- **Wi-Fi:** NetworkManager via `zbus`.
- **Apps padrão:** leitura e escrita de `~/.config/mimeapps.list` (`mime`, `xdg`).
- **Tema claro, escuro e cores mac:** `palette`, tema em `toml` e propagação para apps pelo portal `org.freedesktop.appearance` (`ashpd`).
- **Localidade:** idioma via `fluent` e região, fuso e teclado via systemd-localed e timedated.
- **Inicialização do sistema:** `.desktop` em `~/.config/autostart` e a tela de configuração que gera esses arquivos.

---

## 8. Itens não descritos no CONTEXT.md, mas necessários

### Sessão e segurança
- **Login e sessão:** o DE é iniciado por um gerenciador de login como `greetd`. Você provê a entrada `.desktop` de sessão Wayland.
- **Bloqueio de tela:** protocolo `ext-session-lock-v1`, suportado pelo Smithay. Obrigatório antes de qualquer uso público.
- **Inatividade:** protocolo `ext-idle-notify-v1` para bloquear a tela e suspender após um tempo.
- **Permissões de apps:** portais (`ashpd`) para acesso a arquivos, câmera, captura e notificações, para que apps sandboxados funcionem.
- **Clipboard e arrastar entre apps:** `wl_data_device` e `primary_selection`, tratados pelo Smithay.

### Entrada e acessibilidade
- **Método de entrada (IME):** `zwp_text_input_v3` e `input-method-v2`, para idiomas com composição de texto.
- **Atalhos globais e gestos:** libinput para gestos de trackpad, com despacho para o compositor.
- **Acessibilidade:** leitor de tela via AT-SPI (`atspi`, `zbus`), ampliação e contraste alto. Previsto desde o início, mesmo que implementado depois.

### Monitores e desempenho
- **Múltiplos monitores e HiDPI:** escala fracionária (`fractional-scale-v1`) e layouts por monitor.
- **VRR e sincronização:** page flip atômico com `drm`, para evitar screen tearing (o "screenthearing" do CONTEXT).
- **Medição:** métricas de frame time e uso de GPU em modo debug, para validar o custo dos efeitos.
- **Modo de economia:** desligar blur e animações automaticamente com bateria baixa (`upower`).

### Ferramentas e qualidade
- **Testes:** `cargo test` para lógica, e um backend headless ou nested (winit) para testes de integração.
- **Simulação sem hardware:** backend nested (`smithay` winit) para desenvolver sem sair de outra sessão.
- **Logs e diagnóstico:** `tracing` com saída para journald e um modo de inspeção de protocolos.
- **Empacotamento:** arquivo `.desktop` de sessão, serviços systemd de usuário e pacote Debian ou Flatpak para instalação.
- **Segurança de dependências:** `cargo-audit` e `cargo-deny` no CI.

---

## 9. Protocolos Wayland necessários

| Protocolo | Uso |
|---|---|
| `wlr-layer-shell` | Barra, dock, desktop, launcher e notificações. |
| `xdg-shell` | Janelas de aplicativos. |
| `xdg-decoration` | Decidir entre decoração do servidor e do cliente, estilo mac. |
| `wlr-output-management` | Configuração de monitores. |
| `ext-session-lock-v1` | Bloqueio de tela. |
| `ext-idle-notify-v1` | Inatividade. |
| `fractional-scale-v1` | Escala fracionária. |
| `zwlr-screencopy` ou `ext-image-copy-capture` | Captura de tela. |
| `zwp_text_input_v3` e `input-method-v2` | Método de entrada. |
| `xdg-foreign` e `foreign-toplevel-list` | Dock e gerenciador de janelas (lista de janelas abertas). |
| `xdg-activation-v1` | Ativação de janelas ao abrir apps pela dock. |

---

## 10. Dependências de sistema (Ubuntu/Debian)

```bash
sudo apt install build-essential pkg-config \
  libwayland-dev libxkbcommon-dev libinput-dev libseat-dev \
  libudev-dev libgbm-dev libdrm-dev libegl-dev libgles-dev \
  libdbus-1-dev libfontconfig-dev libpipewire-0.3-dev \
  libpixman-1-dev
```

---

## 11. Projetos de referência

- **anvil:** compositor de exemplo do Smithay. Ponto de partida para entender a API.
- **niri:** compositor em Rust, com layout em rolagem e efeitos visuais.
- **cosmic-comp:** compositor do COSMIC, com integração completa de DE.
- **pinnacle:** compositor em Rust configurável em Lua ou Rust.
