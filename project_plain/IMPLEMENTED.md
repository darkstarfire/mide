# O que já foi implementado

## Etapa 1: esqueleto do projeto Rust

- Criado o projeto Cargo `mide` na raiz do repositório, como aplicação binária (edição 2024).
- Adicionado `.gitignore` ignorando a pasta `target/`.
- Instalado o Rust pelo rustup (versão 1.99.1 estável), porque não havia cargo na máquina.
- Verificado: `cargo build` e `cargo run` funcionam.

## Etapa 2: compositor Wayland mínimo (aceita clientes)

- Adicionado o Smithay 0.7.0 com as features de Wayland, desktop, renderização GL, winit, DRM, libinput, libseat e udev. Ainda não são usadas todas, mas já ficam prontas para as próximas etapas.
- Adicionados `tracing` e `tracing-subscriber` para os logs.
- Verificado que as bibliotecas de sistema da seção 10 do `RECURSES.md` estão instaladas (wayland, xkbcommon, libinput, libseat, libudev, gbm, libdrm, EGL, GLES, dbus, pipewire, pixman e fontconfig).
- `src/main.rs`: cria o display Wayland, abre o socket `mide-0` e roda o event loop (calloop). A cada 16 ms envia os eventos pendentes aos clientes.
- `src/state.rs`: estado global do compositor e os handlers dos protocolos mínimos: `wl_compositor`, `xdg_wm_base` (janelas e popups), `wl_shm`, `wl_seat` e `wl_data_device`.
- Teste feito: um cliente Wayland cru conectou em `mide-0`, recebeu a lista de globais e desconectou. Os logs confirmaram a conexão e a desconexão.
- Ainda não há renderização nem entrada de teclado ou mouse. Uma janela de cliente pode ser criada, mas não aparece na tela.

## Etapa 3: renderização no backend winit

- `src/backend_winit.rs`: cria uma janela winit (1280x800) dentro da sessão atual, registra a saída `mide-winit` (`wl_output`) e desenha a cada frame com `render_output` e `OutputDamageTracker`. Envia os frame callbacks aos clientes e entrega o `submit` ao backend.
- `src/state.rs`: agora há um `Space<Window>`. Cada toplevel novo entra mapeado em cascata (48 px + 32 px por janela). O commit atualiza a janela, e fechar a janela remove ela do espaço.
- Implementado `OutputHandler` e `delegate_output!`, necessários para o `wl_output`.
- Teste feito: o compositor abriu a janela winit sem erros nem panics. Um app GTK real (`gnome-calculator`) conectou em `mide-0`, criou uma janela (log `nova janela criada`) e fechou (log `janela fechada`).
- Limitação: a conferência visual do conteúdo da janela não foi feita por mim, porque não tenho acesso à tela. Os logs confirmam o fluxo, mas não a aparência final.

## Etapas A3, A4 e A5: teclado, ponteiro e foco por clique

- `src/input.rs`: recebe os eventos do winit e os repassa ao seat. Teclado com keymap padrão (xkbcommon). Ponteiro com movimento, botões e rolagem, enviados à superfície sob o cursor.
- Clique com o botão esquerdo traz a janela para frente, dá a ela o foco do teclado e marca só ela como ativa (configure com o estado `Activated`).
- `src/state.rs`: `State::new` agora retorna `Result`, porque criar o teclado pode falhar. Guarda também a posição do cursor.
- Correção visual: a janela aparecia de cabeça para baixo, porque o GL do winit tem a origem embaixo. A saída agora usa `Transform::Flipped180`.
- Limitação: nenhum evento de entrada real foi injetado, porque não há ferramenta instalada (ydotool, wtype, xdotool) e não tenho permissão para instalar. O código compila e o compositor roda sem panics com um cliente GTK conectado. Falta confirmar teclado, clique e foco com um teste real.

## Ajustes do teste no desktop: título e redesenho

- A janela do winit agora tem o título "mide" e tamanho mínimo de 640x400 (`init_from_attributes`).
- O pedido de redesenho é feito logo na criação e depois de cada evento de entrada, para o winit não deixar a janela vazia.
- Log `frame: N janela(s) mapeada(s)` em nível debug, para ver cada frame com `RUST_LOG=mide=debug`.
- Barra de título, botões e duplo clique para maximizar são do desktop hospedeiro, não do mide. Isso só muda quando a barra de título própria do mide entrar (A7).

## Pedidos de janela: arrastar, maximizar e restaurar

- Confirmado no teste: a calculadora aparece dentro do mide e responde a mouse e teclado (`10×2` resultou em `20`).
- `src/state.rs`: tratados os pedidos xdg-shell que a calculadora envia pelos próprios botões. `move_request` inicia o arrasto. `maximize_request` ocupa a área da saída e guarda a posição anterior. `unmaximize_request` restaura essa posição.
- `src/input.rs`: durante o arrasto, a janela segue o cursor e o cliente não recebe o movimento. Soltar o botão esquerdo encerra o arrasto.
- Pendente: minimizar só registra no log, porque a dock (B4) ainda não existe para restaurar a janela. Redimensionar pela borda também fica para depois.
- Pendente: barra de título do próprio mide (A7). Hoje a barra vem do app, como o GTK faz.

## Popups: menus suspensos (botão ☰ da calculadora)

- O mide agora rastreia popups (`PopupManager`), posiciona cada um pela geometria pedida pelo cliente e os desenha por cima das janelas.
- Cliques vão primeiro aos popups, antes das janelas. Clicar fora de um popup fecha todos.
- Pendente: o popup não captura o teclado nem fecha sozinho ao perder o foco (grab de popup).

## Atalhos de teclado (A9 parcial) e fechar janela (A8 parcial)

- `src/config.rs`: lê `~/.config/mide/config.toml` (ou `$XDG_CONFIG_HOME/mide/config.toml`) na seção `[keybindings]`. Sem arquivo, usa os atalhos padrão.
- Padrões: `super+q` fecha a janela com foco, e `ctrl+alt+t` abre `x-terminal-emulator`.
- Ações: `close_window` e `launch:<comando>` (binário ou script, executado pelo shell com `WAYLAND_DISPLAY=mide-0`).
- Atalho casado é interceptado: a tecla não chega ao app.
- Pendente: editor de atalhos e a lista completa de atalhos de desktop, janelas e apps.

## Testes automatizados

- `cargo test`: 11 testes unitários, para os atalhos (parsing, modificadores, substituição de padrões, erros) e para a cascata de janelas.
- `tests/smoke.sh`: sobe o compositor, confere o socket, os globais Wayland, a criação da janela com uma calculadora real e a ausência de panics. Passou nesta máquina (7 de 7).
- As baterias por etapa, incluindo a checklist manual, estão em `project_plain/TESTS.md`.

## Correções a partir do teste manual (TESTS.md)

- Configuração: atalhos soltos no topo do `config.toml` agora são aceitos, além da seção `[keybindings]`. Antes eram ignorados em silêncio.
- Terminal padrão (`ctrl+alt+t`): troca para `foot`, com `x-terminal-emulator` como reserva. O gnome-terminal reaproveitava o servidor do desktop e abria a janela fora do mide.
- Teste unitário novo para o formato de atalho no topo do arquivo. Total: 12 testes.

## Atalhos principais: terminal e buscador de programas

- Padrões novos: `ctrl+shift+t` abre o terminal (`foot`, com `x-terminal-emulator` como reserva) e `ctrl+shift+space` abre o buscador de programas (`fuzzel`, depois `wofi` e `rofi`).
- Escolhidos porque não conflitam com os atalhos do desktop hospedeiro.
- O buscador é externo por enquanto. O launcher próprio do mide é a etapa B6.
- Testes: 13 unitários passando, incluindo os dois atalhos principais.

## Layer-shell (A10 parcial): buscador de programas

- `wlr-layer-shell` implementado. Superfícies de camada são mapeadas na saída e desenhadas acima das janelas (camadas Top e Overlay).
- O ponteiro chega às camadas antes das janelas. A camada que pede foco de teclado recebe o foco ao abrir.
- Confirmado com o `fuzzel`: ele cria a camada `launcher` (Top) sem erro. Antes falhava com `no layer shell interface`.
- Pendente: camadas de fundo (Background e Bottom) e a zona exclusiva da barra, que reserva espaço para as janelas.
- O `super+q` não chega ao mide: a tecla Super é capturada pelo desktop. Adicionado um log de debug (`tecla: ...`) para diagnosticar. Adicionado `ctrl+shift+q` como alternativa no config.

## Correção do foco de camadas (buscador não recebia teclas)

- Causa: o foco de teclado era dado na criação da camada, antes de o cliente ter feito o primeiro commit. Agora o foco é dado no primeiro commit, quando a camada pede foco de teclado.
- Clicar numa camada que aceita foco também dá o foco a ela.
- Quando a camada fecha (ex.: Esc no buscador), o foco volta para a janela do topo.
- Teclado virtual (`zwp_virtual_keyboard_v1`) habilitado, para ferramentas como o `wtype` digitarem nos testes automáticos.
- Confirmado: ao abrir o `fuzzel`, o log mostra `foco do teclado: camada`. Digitação e Esc ainda precisam de teste com uma ferramenta de injeção de teclas (`wtype`).

## Digitação no buscador (correção do frame callback)

- Causa: o buscador recebia as teclas, mas não redesenhava. O redesenho depende do frame callback do compositor, e o mide só enviava esse callback às janelas, não às camadas nem aos popups.
- Correção: frame callbacks agora vão também para as camadas e para os popups.
- Teste automático: `tests/smoke.sh` digita `abc` com o `wtype` no buscador e confere que ele recebe 6 eventos de tecla e faz commits depois delas. Passou (9 de 9).

## A7 parcial: barra de título do mide (decoração de servidor)

- `src/decorations.rs`: barra de 30 px acima do conteúdo, com botões fechar (vermelho), minimizar (amarelo) e maximizar (verde). A cor muda com o foco.
- O protocolo `xdg-decoration` foi implementado: o mide responde sempre com decoração de servidor. Apps que pedem essa decoração (ex.: `foot`) recebem a barra do mide.
- Clicar nos botões fecha, minimiza (só registra no log) ou maximiza. Arrastar a barra move a janela. Duplo clique alterna a maximização.
- Maximizar agora deixa o conteúdo abaixo da barra, para ela não sair da tela.
- Limitação: apps que sempre desenham a própria barra (GTK4 e libadwaita, como a calculadora) não pedem decoração de servidor. Para esses, a barra do mide não aparece.
- Pendente: texto do título. Vai precisar de uma biblioteca de fontes (ex.: `cosmic-text`).
- Testes: 17 unitários (inclui a geometria da barra e os botões). `tests/smoke.sh`: 9 de 9.

## Correção: botões da barra de título invisíveis

- Causa: o Smithay desenha a lista de elementos da frente para trás. O fundo da barra vinha antes dos botões e os cobria.
- Correção: botões primeiro, fundo depois. Barras das janelas do topo e camadas do topo também vêm primeiro na lista.
- Verificado no log: a barra é desenhada em 477 frames durante o teste com `foot`, sem pânico.

## Cantos arredondados e botões redondos

- `src/decorations.rs`: dois shaders de pixel compilados na inicialização. Um desenha retângulos arredondados (botões circulares e fundo da barra). O outro é uma máscara de canto, que pinta com a cor do fundo do desktop a região fora do arco.
- Os quatro cantos da janela (barra + conteúdo) são arredondados com raio de 12 px. Janela maximizada fica sem arredondamento.
- Botões são círculos de 12 px.
- Limitação: a máscara usa a cor sólida do fundo do desktop. Atrás de um canto arredondado aparece essa cor, e não o que está por trás de verdade. Não há sombra.
- Compilação dos shaders e desenho dos frames verificados no log, sem erro. A aparência final ainda depende de você conferir.

## Visual mais próximo do macOS

- Janela do winit sem moldura do host por padrão. A barra do próprio mide passa a ser a única. Para voltar à moldura do desktop, use `MIDE_WINIT_DECORATIONS=1`.
- Paleta clara da barra, como no macOS. Semáforo colorido com foco (vermelho, amarelo e verde) e botões cinza sem foco. Linha fina separando a barra do conteúdo.
- Raio dos cantos ajustado para 10 px.
- Atalho `ctrl+shift+escape` encerra o compositor (ação `quit`). Como a moldura do host foi removida, esse é o jeito de fechar o mide sem o terminal.
- Testes: 18 unitários. `tests/smoke.sh`: 9 de 9.
- Ainda falta para ficar parecido com o macOS: título centralizado (precisa de texto), barra de menus superior (B1), dock (B4), sombra das janelas e papel de parede.

## Correção: cantos arredondados não apareciam

- Causa: as máscaras dos cantos eram desenhadas depois da barra e do conteúdo na lista de render, que vai da frente para trás. Assim, ficavam escondidas.
- Correção: máscaras antes da barra na lista. Verificado no log: a decoração é desenhada em 417 frames com o `foot`, sem pânico.

## Profundidade: sombra, borda e brilho da barra

- Sombra suave em volta da janela, deslocada 8 px para baixo, com margem de 24 px. Ela só pinta fora do retângulo real, então não escurece o conteúdo. Mais forte quando a janela tem foco.
- Traço de 1 px seguindo os cantos arredondados, com opacidade maior quando a janela tem foco.
- Barra com brilho na metade superior e linha clara no topo, para dar volume.
- Verificado: a decoração é desenhada em 419 frames com o `foot`, sem panics e sem erro de shader.

## Botões com hover, anel e sombra limpa

- Hover no grupo de botões: ao passar o ponteiro sobre eles, aparecem × (fechar), − (minimizar) e + (maximizar), como no macOS. Só com a janela focada.
- Cada botão tem um anel fino escuro em volta do círculo, para parecer sólido.
- Sombra: alpha abaixo de 0,02 é cortado. Isso remove o padrão pontilhado da borda, que era ruído de quantização de 8 bits sobre o fundo escuro.
- Testes: 19 unitários (novo: hover do grupo). Decoração desenhada em 419 frames com o `foot`, sem panics.

## Hover por botão e apresentação da sombra

- Hover só no botão sob o ponteiro: o símbolo (× − +) aparece apenas nele, e não nos três ao mesmo tempo.
- Apresentação: o quadro agora é apresentado inteiro (`submit(None)`). Antes, só a região "danificada" era enviada, e o buffer de trás podia conter pixels antigos, o que causava o padrão pontilhado.
- Ferramenta de depuração: `MIDE_DUMP_FRAME=/caminho` salva o frame 120 como RGBA cru, para medir pixels. Os valores crus da sombra são um gradiente suave, sem pontilhado. O dump sai de cabeça para baixo por causa do `Flipped180`.
- Testes: 19 unitários (inclui o hover por botão). Decoração desenhada em 418 frames, sem panics.

## Sombra limpa, canto contínuo e animação ao maximizar

- Pontinhos brancos: medidos no quadro salvo, a sombra tinha pixels com alfa 170 em um contorno pontilhado. A causa é a mistura de camadas translúcidas sobre o fundo escuro. A sombra agora escreve cor opaca, com o escurecimento do fundo já embutido. Os pontos caíram de 1049 para 8 pixels.
- Canto: a máscara do canto calcula a mesma sombra da janela, em vez de pintar cor de fundo chapada. Assim a curva da sombra continua no canto inferior.
- Animação ao maximizar: 220 ms, desaceleração cúbica. A posição e o tamanho da moldura são interpolados, e o conteúdo do app recebe zoom. Agora as janelas são desenhadas pelo próprio mide, na ordem de z, em vez de pelo `Space`.
- Limitação: a sombra é opaca. Se outra janela estiver logo atrás, a margem de 24 px dela aparece com a cor do fundo, e não com a janela de trás. Corrigir isso exige um pipeline de sombra com fundo real.
- Limitação: só maximizar é animado. Restaurar ainda não.
- Verificado: 19 testes unitários, e o compositor roda sem panics com o `foot`. A animação não pode ser disparada daqui, porque precisa de clique.

## Redimensionar pelas bordas e cantos

- Faixa de 6 px nas bordas e cantos da janela (barra + conteúdo). Pressionar e arrastar muda o tamanho. Borda esquerda e superior também movem a janela, para o lado oposto ficar fixo.
- Tamanho mínimo de 200x120. Janela maximizada não redimensiona.
- Só a borda superior, sem canto, fica para o arrasto da barra de título.
- Testes: 21 unitários (inclui detecção de borda, canto e borda superior).
- Pendente: o cursor não muda de forma sobre a borda (ainda não há cursor próprio). Redimensionar o conteúdo é só pelo tamanho configurado, sem animação.

## Animação ao restaurar e cursor de redimensionamento

- Restaurar (desmaximizar) agora também é animado. O mide guarda o tamanho antes de maximizar e o pede de volta ao app, para a animação saber o destino.
- Cursor: sobre uma borda, a forma do ponteiro muda (horizontal, vertical ou diagonal, conforme a borda ou canto). Sobre a barra e o conteúdo, volta ao padrão. Enquanto redimensiona, a forma se mantém.
- Testes: 21 unitários. Compositor roda sem panics com o `foot`.
- Limitação: o tamanho que o app devolve pode diferir do pedido. Nesse caso a animação termina no tamanho real, com um pequeno salto.

## Estado do plano ao fechar esta etapa

- A1 a A6: concluídas (socket, renderização, teclado, ponteiro, foco, arrasto).
- A7: barra de título com botões, hover por botão, sombra, cantos arredondados, redimensionar por borda e cursor. Falta só o título em texto.
- A8: parcial. Minimizar depende da dock (B4).
- A9: parcial. Motor de atalhos e `config.toml` prontos; falta o editor e o conjunto completo.
- A10: parcial. Camadas Top e Overlay com foco e ponteiro; faltam camadas de fundo e zona exclusiva.
- A11: parcial. Sombra e animação ao maximizar e restaurar; faltam blur, transparência e efeito lâmpada.
- Próximo passo recomendado: título em texto da barra (A7), com `cosmic-text`, que já está em `RECURSES.md`.

## Título da janela em texto (completa A7)

- `src/title.rs`: o título do app é composto com `cosmic-text` (fontes do sistema, SemiBold 13 px), rasterizado para RGBA premultiplicado e enviado à GPU como buffer de memória. Fica em cache por título e foco.
- Centralizado na barra. Cinza-escuro com foco, cinza-claro sem foco, como no macOS.
- Título vem de `XdgToplevelSurfaceData` (o que o app define). Sem título, a barra fica sem texto.
- Verificado: o quadro salvo mostra "foot" centralizado. `tests/smoke.sh` e 21 unitários passam.
- Limitação: títulos longos não são cortados com reticências ainda, e o texto não se ajusta ao espaço entre os botões e a borda.

## Título longo com reticências

- O título agora é cortado para caber entre os botões e a borda direita da barra (70 px reservados em cada ponta). Quando não cabe, termina com "…".
- O corte é feito por medição: o texto perde caracteres do fim até caber. O resultado fica em cache por título, foco e largura.
- Verificado: um título longo aparece cortado em "te…", sem invadir os botões. 21 unitários passam, e o compositor roda sem panics.

## B4 (esqueleto): dock como cliente separado

- Novo programa `mide-dock` (`src/bin/mide-dock.rs`). É um cliente Wayland com janela de camada na parte de baixo da tela, feito com `smithay-client-toolkit` 0.21.
- Desenho com `tiny-skia`: faixa clara e translúcida com cantos arredondados, e os ícones dos apps fixados.
- Apps fixados em `~/.config/mide/dock.toml` (`pinned = ["org.codeberg.dnkl.foot", ...]`). Padrão: foot e calculadora. Ids são o nome do arquivo `.desktop`.
- Ícones lidos do tema (só PNG). Sem ícone, um bloco neutro.
- Clique com botão esquerdo abre o app (campos `%` do Exec são removidos).
- Mudança no compositor: o mide agora envia a configuração inicial das camadas no primeiro commit. Antes, a dock nunca desenhava.
- Verificado: a dock aparece embaixo, centralizada, com o ícone do foot. 21 unitários e tests/smoke.sh passam.
- Pendente: reordenar, fixar e desafixar arrastando; ícones SVG (exige resvg com a mesma versão de tiny-skia); lixeira e Downloads (B5); iniciar a dock sozinha com o mide.

## Auditoria visual e correções (efeitos, janelas, controles e dock)

Análise feita olhando quadros salvos do compositor (`MIDE_DUMP_FRAME`), com duas janelas sobrepostas e a dock.

- **Sombra e máscaras de canto tapavam a janela de baixo.** Estavam pintadas com cor opaca e apareciam por cima da janela de trás. Agora cada sombra e máscara recebe até quatro retângulos das janelas de baixo e não pinta sobre eles. A janela de trás aparece inteira.
- **Desenho em duas passadas:** primeiro calcula a moldura de cada janela, de cima para baixo; depois desenha, com as janelas de baixo disponíveis para a sombra.
- **Minimizar:** antes só registrava no log. Agora a janela encolhe até o centro inferior da tela, perde opacidade e sai do espaço. Restaurar (`ctrl+shift+m`) faz a animação de volta, com a moldura. Testado com um teste temporário, depois removido.
- **Ícone SVG na dock:** a calculadora aparecia como bloco cinza. Agora o SVG é renderizado com `resvg`.
- **`cargo run` com dois programas:** definido `default-run = "mide"`.
- **Dock iniciada pelo mide:** antes era preciso rodar `mide-dock` à parte.

Limitações conhecidas, que continuam:
- Só as quatro janelas mais próximas de baixo entram nos buracos da sombra. Com mais de quatro sobrepostas, a quinta leva sombra por cima.
- Restaurar minimizadas só pelo atalho; a dock ainda não lista nem restaura janelas.
- Cliques na dock e na barra não foram testados, porque não há como enviar ponteiro daqui.
- Blur, transparência e efeito lâmpada (A11) ainda não existem.
- A dock não tem sombra própria.

## Correção: janelas de apps com decoração do cliente (GTK) e deslocamento da geometria

- Causa 1: a janela da calculadora tem um deslocamento de geometria (sombra desenhada pelo próprio app). A moldura era posicionada na origem da superfície, sem esse deslocamento, e aparecia deslocada em relação ao conteúdo.
- Correção 1: a moldura e a barra se alinham à área visível (`visible_origin`). A origem da superfície continua onde o Smithay a coloca.
- Causa 2: o mide desenhava a própria barra também em apps que desenham a sua (calculadora), e isso criava dois cabeçalhos.
- Correção 2: a barra do mide só existe em janelas que pediram decoração de servidor (ex.: foot). Nas demais, a moldura é só em volta do conteúdo, com cantos e sombra, e a barra, o título e o arrasto por barra não aparecem.
- Limpeza: removido o bloco de depuração de listagem de janelas.
- Verificado no quadro: foot com barra e calculadora com cabeçalho próprio, sem sobreposição. 21 unitários e a bateria de fumaça passam.

## Autoteste de interface e correções encontradas por ele

Para testar sem depender do usuário, o mide ganhou um autoteste de ponteiro (`src/selftest.rs`): a variável `MIDE_SELFTEST` define um roteiro de passos `frame ação x y` (move, press, release, click, list, restore). O tratamento de mouse foi separado do formato de evento do winit, então os cliques simulados passam pelo mesmo código dos reais.

- `tests/scenario.sh`: sobe o mide com um roteiro, abre apps e mostra o log (e, opcionalmente, salva um quadro).
- `tests/ui.sh`: 20 cenários com resultado conferido (posição, tamanho, fechamento, popups, ordem das janelas, dock). Passam todos.

Bugs encontrados e corrigidos com o autoteste:
- **Clique deslocado na calculadora (X só fechava clicando acima e à esquerda).** No `Space` do Smithay, a posição da janela é o canto da área visível, e a superfície fica em posição menos o deslocamento da geometria. A superfície era desenhada sem esse desconto. Corrigido: desenho na mesma origem que o clique usa.
- **Janelas nascendo uma em cima da outra.** O GTK pede "desmaximizar" ao abrir, e o mide movia a janela para (48, 48). Agora desmaximizar só age em janela maximizada. A cascata voltou a funcionar.
- **Botão amarelo da barra não minimizava.** Ainda chamava o código antigo. Corrigido.
- **Arrasto pelo cabeçalho do app perdia o começo do movimento.** O arrasto agora é medido a partir do ponto onde o botão foi pressionado.
- **App sem barra do mide maximizava 30 px mais baixo.** O tamanho maximizado agora usa a altura real da barra (0 para apps com cabeçalho próprio).
- **Erro de um cliente derrubava o compositor.** O despacho de mensagens agora registra o erro e continua.

## Dock conversando com o compositor

- Canal de controle `$XDG_RUNTIME_DIR/mide-0.ctl` (`src/control.rs`): `list` devolve as janelas com app, título e estado; `activate <ids>` restaura a minimizada ou foca a aberta.
- A dock agora: foca o app já aberto em vez de abrir outro; restaura a janela minimizada do app; mostra um ponto embaixo dos apps abertos (consulta a cada 0,8 s).
- A dock deixou de redesenhar a 60 quadros por segundo: só desenha quando algo muda.

## Efeitos desligáveis

- Seção `[effects]` no `config.toml`, com `shadows`, `rounded_corners` e `animations` (todas `true` por padrão). Verificado: com tudo desligado, minimizar é instantâneo e não há sombra nem cantos.

## B4 concluída: dock completa

- Itens: os fixados (ordem salva em `~/.config/mide/dock.toml`) e, depois de um separador fino, os apps abertos que não estão fixados.
- Clique esquerdo (ao soltar): foca ou restaura o app; se não estiver aberto, abre.
- Arrastar um ícone fixado: reordena e salva ao soltar. Um arrasto não abre o app.
- Clique direito: fixa um app aberto ou desafixa um fixado, e salva na hora. Desafixado e fechado, ele sai da dock.
- A dock muda de largura sozinha conforme o número de itens.
- O autoteste ganhou o clique direito (`rclick`), e `tests/ui.sh` tem 4 cenários novos de dock, rodados com configuração temporária (não mexem no arquivo do usuário). Total: 24 cenários, todos passando.

## Sombras combinadas (corrige sombra apagada entre janelas sobrepostas)

- Antes, cada janela desenhava a própria sombra com cor opaca, e a sombra de uma janela apagava a sombra da janela de baixo perto dos cantos.
- Agora há um único fundo do desktop desenhado atrás de tudo (`desktop_element`), que calcula num shader a sombra combinada das 8 janelas de cima. As sombras se somam e nenhuma apaga outra.
- As máscaras dos cantos usam a mesma conta, então o canto continua a sombra combinada sem entalhe.
- Limitação: a sombra de uma janela não escurece a janela de baixo, só o desktop.

## Efeito lâmpada (genie)

- `src/genie.rs`: durante minimizar e restaurar, o conteúdo da janela é renderizado numa textura fora da tela e desenhado com um shader que deforma cada linha. A base afunila e desce primeiro em direção ao centro inferior (a dock); o topo vai depois.
- Duração de 420 ms (maximizar continua com 220 ms).
- Restaurar faz o caminho inverso, saindo da dock.
- Desligável com `lamp = false` em `[effects]` (volta à animação de escala).
- Verificado no quadro salvo no meio da animação: formato de funil e conteúdo na orientação certa.

## Incidente: configuração real da dock alterada por teste

- O `tests/ui.sh` usava `XDG_CONFIG_HOME=... check`, e no bash isso não exporta a variável para os programas chamados pela função. Os testes de fixar e reordenar gravaram `~/.config/mide/dock.toml` do usuário (o arquivo não existia antes).
- Corrigido: o arquivo criado por engano foi removido (a dock voltou aos padrões), a variável agora é exportada só durante a seção da dock, e a bateria falha se a configuração real mudar (trava por impressão digital do diretório).
- Total da bateria: 25 verificações, todas passando.

## Correções da rodada: ponta branca, dock, lâmpada, saída redimensionada e Ctrl+clique

- **Ponta branca no canto (bug):** a máscara de canto não pintava sobre janelas de baixo, e o canto quadrado da barra aparecia. Agora o conteúdo da janela é recortado de verdade no desenho (`src/clip.rs`): um shader de textura troca o padrão do Smithay (`override_default_tex_program`) e zera os pixels fora da moldura arredondada. A barra usa o mesmo recorte no shader dela. As máscaras de canto foram removidas. O canto mostra o que está atrás (desktop com sombra ou outra janela).
- **Dock com visual do macOS:** raio de 18 px (antes era uma pílula), vidro claro translúcido com gradiente, borda fina mais escura em cima e mais clara embaixo, brilho interno de 1 px, margens mais justas e ponto de app aberto menor.
- **Efeito lâmpada vai até o ícone do app:** a dock informa ao compositor onde está cada ícone (`icons` no canal de controle), sempre que muda. Ao minimizar e restaurar, o funil vai e volta do ícone do app naquele momento. Sem ícone, vai ao centro inferior.
- **Efeito lâmpada mais bem feito:** a janela inteira entra na textura (barra, título, botões e conteúdo), com cantos arredondados no próprio shader; curva de aceleração e desaceleração suave (antes desacelerava logo no começo e ficava brusco).
- **Janela do mide maximizada no host:** ao mudar o tamanho da saída, as camadas são reorganizadas (a dock volta ao centro inferior) e as janelas maximizadas acompanham a nova área.
- **Ctrl+clique na dock:** abre uma instância nova mesmo com o app aberto. A dock pergunta ao compositor as teclas pressionadas (`mods` no canal de controle).
- Autoteste: passos `ctrl on/off` e `resize L A`. `tests/ui.sh`: 28 verificações, todas passando.
- Limitação mantida: a sombra de uma janela ainda não escurece a janela de baixo, só o desktop (fica como aperfeiçoamento).

## Efeito lâmpada refeito

- O alvo já era o ícone certo (confirmado no log, também depois de redimensionar a saída), mas o desenho cortava o funil no meio da tela.
- Causa: o elemento de textura usava como origem o tamanho do destino (a área até a dock), maior que a textura da janela. As coordenadas passavam de 1 e o funil era cortado e distorcido. Agora a origem é a textura inteira.
- Deformação refeita em duas fases, como no macOS: (1) a base estica até o ícone e o funil se forma, com a largura de cada linha indo da largura da janela até a do ícone conforme a altura; (2) o topo desce e a janela escorre pelo funil para dentro do ícone.
- Verificado numa sequência de 5 quadros com a saída em 1280x800 (`MIDE_WINIT_SIZE=1280x800`): o funil encosta no ícone do app e a janela entra nele.
- Nova variável de teste `MIDE_WINIT_SIZE=LxA`, que abre a janela do mide já nesse tamanho.

## Barra superior (B1)

- Novo cliente `mide-bar` (`src/bin/mide-bar.rs`), aberto pelo compositor junto com a dock. É uma camada no topo, com a largura da tela, 28 px de altura e zona exclusiva: janelas novas nascem abaixo dela e maximizar ocupa só a área livre (entre a barra e o fundo).
- À esquerda, a marca do mide e o nome do app em foco (em negrito), lido do compositor pelo canal de controle e traduzido pelo nome do `.desktop`.
- À direita, o relógio (ex.: "sáb 17 out  17:27") e, se houver bateria, a porcentagem.
- Clique na marca abre o menu do sistema (camada sobreposta que fecha ao clicar fora): "Sobre este computador", "Configurações do sistema…" (desativado até B7) e "Encerrar sessão…" (encerra o mide). Itens com destaque ao passar o mouse.
- "Sobre este computador" abre `mide-about`: janela com a barra do mide mostrando versão, sistema, kernel, processador, memória, vídeo, nome do computador e tempo ligado.
- Código de desenho compartilhado entre os clientes em `src/ui/` (formas, gradientes e texto com cosmic-text).
- Janela nova recebe o foco na hora em que abre.
- Ainda não: reiniciar e desligar (não dá para testar com segurança aqui), menus do app (B2) e área de status (B3).

## Papel de parede (A12)

- Nova seção no `config.toml`:
  ```toml
  [desktop]
  wallpaper = "~/Imagens/fundo.jpg"
  ```
  Aceita PNG e JPEG. A imagem cobre a tela inteira sem distorcer (corta as sobras no centro) e se ajusta quando a saída muda de tamanho.
- Sem imagem, ou se ela não abrir, o fundo é um gradiente próprio do mide (azul profundo, violeta e um brilho quente embaixo).
- As sombras das janelas continuam sobre o papel de parede: o fundo é uma textura desenhada por um shader que também aplica a sombra combinada (`src/wallpaper.rs`).

## Janelas sempre abaixo da barra superior

- Se a barra começa depois de uma janela já aberta, a janela desce para baixo dela, e as maximizadas se ajustam à área livre.
- Arrastar uma janela para cima para na barra, como no macOS.

## Dock: medidas finais (ajustadas com o usuário)

- Ícones de 44 px, 3 px entre ícones, 7 px até a borda, 3 px a mais embaixo para o ponto de app aberto (altura 61 px), cantos de 15 px.
- Cada ícone tem a margem transparente cortada e a parte visível ajustada aos 44 px, centralizada. Antes, ícones com margem própria (como o da calculadora) deixavam um lado da dock com mais espaço que o outro.
- O separador entre fixados e abertos tem 8 px de respiro de cada lado.
- Clicar no vão entre dois ícones conta para o ícone mais próximo.
- Efeito lâmpada com 450 ms (antes 420 ms).

## Testes

- O compositor aceita `MIDE_SOCKET` (padrão `mide-0`). Os testes usam `mide-test` e `mide-smoke` e encerram só os processos com esse socket no ambiente: uma sessão do mide aberta pelo usuário não é mais fechada nem atrapalha os testes.
- Um `mide-about` de um teste anterior podia abrir no compositor do teste seguinte; agora a limpeza também o pega.
- `tests/ui.sh` roda tudo com uma configuração temporária. Antes, um clique numa janela de teste (clique direito na dock) gravou o `dock.toml` real do usuário.

## Launcher em tela cheia (B6)

- Novo programa `mide-launcher`, no estilo do Launchpad: cobre a tela com um véu escuro translúcido sobre o papel de parede e mostra todos os apps instalados numa grade (6x3 em 800x600, até 7x5 em telas grandes), com o nome embaixo de cada ícone.
- Busca no topo: é só digitar. Não diferencia maiúsculas nem acentos ("configuracoes" acha "Configurações"), procura também no nome genérico e nas palavras-chave do app, e põe primeiro quem começa com o texto digitado.
- Grupos por categoria abaixo da busca (Todos, Internet, Escritório, Gráficos, Multimídia, Desenvolvimento, Jogos...). Só aparecem os grupos que têm apps.
- Teclado: Enter abre o primeiro resultado (ou o escolhido com as setas), Esc limpa a busca e, vazia, fecha; Page Up/Down trocam de página.
- Mouse: clique num app abre e fecha o launcher; clique no vazio fecha; rolagem e os pontos embaixo trocam de página. Os pontos ficam acima da dock.
- Apps marcados para terminal abrem no foot.
- `ctrl+shift+space` abre e, com ele aberto, fecha (nova ação `toggle_launcher`, padrão no lugar do fuzzel).
- Desempenho: abre em cerca de 165 ms. Os ícones carregam numa thread separada e aparecem conforme ficam prontos; várias teclas seguidas geram um quadro só.
- O modo de desenvolvimento agora compila as dependências otimizadas (`[profile.dev.package."*"]`): abrir o launcher caiu de 5 s para menos de 0,2 s, e a dock, a barra e o compositor também ficaram mais rápidos.
- Código compartilhado novo: `src/apps.rs` (lista de apps, grupos, busca), `src/icons.rs` (ícones do tema, usados pela dock e pelo launcher) e `src/control_client.rs` (canal de controle).
- Autoteste: passos `key`, `type` e `combo`, que passam pelo mesmo caminho de uma tecla de verdade (inclusive os atalhos).
- Efeito lâmpada com 650 ms.

## Cliques na dock e na barra não vazam para a janela de trás

- Antes, clicar na dock ou na barra por cima de uma janela também focava essa janela e, perto da borda dela, podia começar um redimensionamento. Agora barra, dock e launcher ficam com o clique só para eles.
- Uma janela no meio da animação de minimizar não recebe mais cliques. Clicar no ícone dela na dock durante a animação já traz a janela de volta (antes o clique era perdido e a janela sumia mesmo assim).

## Downloads e Lixeira na dock (B5)

- No fim da dock, depois de um separador próprio, ficam **Downloads** e **Lixeira**, com os ícones do tema do usuário (ZorinBlue-Dark aqui, com Adwaita como reserva).
- **Downloads**: clique abre um painel acima do ícone com os 8 itens mais recentes da pasta (lida de `user-dirs.dirs`), com ícone pelo tipo de arquivo, nome cortado com reticências e há quanto tempo chegou ("agora", "há 5 min", "ontem"). Clique num item abre com o app padrão; "Abrir no Gerenciador de Arquivos" abre a pasta. Clique direito: "Abrir pasta Downloads".
- **Lixeira**: o ícone muda de vazia para cheia sozinho (conferido a cada 0,8 s). Clique abre a Lixeira no gerenciador de arquivos. Clique direito: "Abrir" e "Esvaziar Lixeira…", que pede confirmação ("N itens serão apagados para sempre. Não dá para desfazer.") com "Esvaziar Lixeira" em vermelho e "Cancelar".
- Painéis com o visual dos menus do macOS: claros, translúcidos, cantos arredondados, sombra suave e destaque azul sob o mouse. Clique fora ou no mesmo ícone de novo fecha.
- Código novo: `src/places.rs` (pasta de Downloads, itens recentes, Lixeira, abrir arquivos) e `load_themed_icon` em `src/icons.rs`.
- Proteção nos testes: cada sessão de teste usa uma Lixeira própria (`XDG_DATA_HOME` temporário) e abrir arquivos só vai para o log (`MIDE_OPEN_DRY_RUN`).

## Rodada de ajustes pedidos pelo usuário (launcher, dock, barra, Sobre)

- **Launcher no estilo do Launchpad:** apps soltos na grade, sem a fileira de categorias. No fim ficam as pastas **Utilitários**, **Sistema** e **Outros**, como no macOS: quadrado claro translúcido com miniaturas 3x3 dos apps de dentro e o nome em negrito. Clique na pasta abre um painel no meio da tela com o nome em cima e os apps dela (com páginas, se não couberem); clique fora ou Esc fecha a pasta. Ícones maiores, que crescem com a tela (67 px em 800x600, até 96 px), véu mais leve para o papel de parede aparecer e busca arredondada. Os ícones carregam em 4 threads (todos em cerca de 0,7 s), primeiro os da primeira página e as miniaturas das pastas.
- **Launchpad na dock:** primeiro ícone da dock (quadrado claro com a grade colorida 3x3), abre e fecha o launcher.
- **Clique direito na dock sempre abre um menu de contexto:** "Abrir"/"Nova janela", "Manter na Dock"/"Remover da Dock" e, com o app aberto, "Ocultar" e "Encerrar". Antes o clique direito desafixava direto.
- **Menu de contexto nos arquivos de Downloads** (clique direito num item da pilha): "Abrir", "Mostrar na pasta", "Copiar caminho" (precisa do `wl-copy`, pacote `wl-clipboard`) e "Mover para a Lixeira" (pela especificação do freedesktop, dá para restaurar pelo gerenciador de arquivos).
- **Menus do app na barra superior (B2, parte 1):** o nome do app em foco é um menu ("Ocultar", "Encerrar") e ao lado ficam **Arquivo** ("Nova janela", "Fechar janela"), **Editar** (Desfazer, Refazer, Recortar, Copiar, Colar, Selecionar tudo, com os atalhos à direita) e **Janela** ("Minimizar", "Zoom"). Valem para qualquer app: o compositor faz a ação na janela ou envia o atalho de teclado a ela. Com um menu aberto, passar o mouse pelos títulos troca de menu.
- **Janela "Sobre este computador" redimensionável:** sem tamanho fixo (mínimo 360x300), conteúdo centralizado e textos longos cortados com reticências. Tamanho inicial maior para o processador caber.
- Novos comandos do canal de controle: `close <ids>`, `hide <ids>`, `window close|minimize|zoom`, `keys <atalho>` e `launcher`.

## Notificações (B9)

- Novo programa `mide-notifications`, iniciado pelo compositor junto com a dock e a barra. É o servidor padrão de notificações do Linux (`org.freedesktop.Notifications` no D-Bus): qualquer app que manda notificação (navegador, e-mail, `notify-send`) aparece no mide.
- Cada notificação vira um **banner no estilo do macOS** no canto superior direito, logo abaixo da barra: claro, translúcido, cantos de 16 px e sombra suave, com o ícone do app (pelo nome no tema ou por arquivo), o título em negrito, "agora" à direita e o texto em até duas linhas (com reticências).
- Os banners se empilham, o mais novo em cima, até 4. Somem sozinhos depois de 5 s; os críticos ficam até o usuário fechar; com o mouse em cima, o banner espera.
- Com o mouse em cima aparece o **×** no canto para fechar. Clique no banner aciona a ação padrão do app (quando ele tem uma). Ações extras viram **botões** no banner (ex.: "Abrir", "Ignorar"), e o app recebe a ação escolhida pelo D-Bus.
- Texto com marcação simples (`<b>`, `<i>`, links) é mostrado como texto puro.
- Rodando o mide em janela dentro do GNOME, o GNOME já é o servidor de notificações: o do mide percebe e sai sem fazer nada. Numa sessão própria do mide, ele assume.
- Código: `src/notify.rs` (regras de tempo, texto e ações, com testes) e `src/bin/mide-notifications.rs` (servidor D-Bus com `zbus` e os banners). Nova dependência: `zbus`.
- Testes: `tests/scenario-dbus.sh` roda o cenário num D-Bus próprio (`dbus-run-session`); 5 verificações novas, inclusive o app recebendo a ação.

## Rodada de ajustes 2 (pedido do usuário com 6 fotos)

### Janela "Sobre" mais leve e apps sem `.desktop` na dock
- O texto (`src/ui/text.rs`) guarda as larguras já medidas: medir texto é caro e as interfaces medem os mesmos textos a cada quadro. Vale para todos os programas do mide.
- A janela "Sobre" desenha uma vez só depois de tratar os eventos pendentes: redimensionar gera muitos `configure` seguidos, e antes cada um virava um desenho.
- Janelas de apps sem arquivo `.desktop` (como a "Sobre") agora aparecem na dock com o título da janela e um ícone do tema (o de computador, para as janelas do mide). Antes, minimizada, a janela sumia sem ícone para trazer de volta. Sem `.desktop`, o menu do ícone só tem Ocultar e Encerrar (não dá para abrir outra nem manter na dock).

### Launcher com a dock e a barra por cima
- Com o launcher aberto, a dock, a barra, os menus delas e as notificações ficam por cima do fundo escurecido, como no macOS (o compositor reordena as camadas no desenho e no clique: `front_layers` em `src/state.rs`).
- A barra fica transparente, com o texto branco, enquanto o launcher está aberto (ela pergunta ao compositor com `launcher state`).
- Clicar num app, em Downloads ou na Lixeira da dock fecha o launcher (`launcher close`).
- A busca desceu (fica abaixo da barra, como na referência) e agora diz "Buscar Aplicativos".
- Rolar a roda do mouse para baixo vai para a página da esquerda; para cima, para a da direita. Clique do meio vai para a próxima página e, da última, volta para a primeira.

### Downloads e Lixeira com menu no item
- Clique direito num arquivo da lista de Downloads abre o menu dele **no ponto do clique, por cima da lista**, sem fechar a lista (Abrir, Mostrar na pasta, Copiar caminho, Mover para a Lixeira). Clique fora do menu fecha só o menu.
- Clique na Lixeira agora mostra a **lista dos itens apagados** (os 8 mais recentes, com há quanto tempo foram apagados), "Abrir no Gerenciador de Arquivos" e "Esvaziar Lixeira…". Clique direito num item apagado: **Restaurar** (volta para a pasta de onde veio; se já existir um com o mesmo nome, ganha " 2") ou **Apagar permanentemente**.
- Clique direito no ícone da Lixeira continua com Abrir e Esvaziar Lixeira….
- Por segurança, um item cujo `.trashinfo` não tem caminho absoluto não é restaurado.

### Escolher a janela pelo menu da dock
- Cada janela recebe um número fixo quando abre. Com mais de uma janela do app, o menu do clique direito lista as janelas (pelo título) no topo; escolher uma traz essa janela para frente (restaurando, se estava minimizada).
- Novos comandos do canal de controle: `windows` (lista com número, app, título e estado) e `focus <número>`.

### Prévias das janelas na dock (como no GNOME)
- Com o ponteiro parado ~0,45 s sobre o ícone de um app aberto, aparece acima da dock um painel claro com a miniatura de cada janela do app e o título embaixo. Janelas com a barra do mide ganham uma barra em miniatura, para parecer a janela de verdade. Passando de um ícone para outro com as prévias abertas, elas trocam mais rápido (0,12 s).
- A miniatura sob o ponteiro fica destacada e mostra o **×**: clicar no × fecha aquela janela; clicar na miniatura traz a janela para frente (restaurando se estava minimizada).
- O painel some quando o ponteiro sai da dock e do painel, ou com um clique em outro lugar. Qualquer clique na dock também esconde as prévias.
- Quem desenha é o compositor (`src/preview.rs`), porque só ele tem o conteúdo das janelas. A dock pede com `preview <x do ícone> <números das janelas>` e esconde com `preview hide`.

### Launcher: arrastar, criar pastas e renomear
- Segurar e arrastar um app (ou pasta) no launcher: o ícone segue o ponteiro e o lugar dele fica vazio.
  - Soltar **no meio de outro app** (o alvo ganha um quadro claro) cria uma **pasta** com os dois, como no macOS e no GNOME. O nome é o grupo dos dois apps quando é o mesmo (ex.: "Jogos"); senão, "Pasta".
  - Soltar **sobre uma pasta** põe o app nela.
  - Soltar **entre itens** ou depois do último muda a ordem.
  - Arrastando até a **borda da tela**, a página troca (no máximo uma vez a cada 0,7 s).
- Com uma pasta aberta: arrastar um app **para fora do painel** tira o app da pasta (ele fica logo depois dela); arrastar dentro do painel muda a ordem. Pasta que fica com um app só deixa de existir.
- **Renomear pasta:** clique no nome em cima do painel; ele vira um campo com cursor. Enter (ou clicar fora) salva, Esc cancela.
- A organização fica salva em `~/.config/mide/launcher.toml`. Ao abrir, apps desinstalados saem e apps novos entram no fim.
- Código: `src/launcher_layout.rs` (regras de pastas, ordem e arquivo, com testes).

### Calendário no relógio
- Clique no relógio da barra abre o calendário, no estilo do macOS: a data de hoje por extenso ("sábado, 17 de outubro"), o mês ("outubro de 2026") com os botões ‹ Hoje ›, os dias da semana (D S T Q Q S S, começando no domingo) e o dia de hoje num círculo azul. Dias de outros meses aparecem apagados.
- Código: `src/calendar.rs` (grade do mês, troca de mês e textos em português, com testes).

### Ícones de som, Wi-Fi, Bluetooth e bateria na barra (B3, parte 1)
- À esquerda do relógio: **som** (alto-falante com ondas pelo volume, ou × no mudo), **Wi-Fi** (leque aceso pela força do sinal, riscado quando desligado), **Bluetooth** (mais apagado quando desligado) e a **bateria** (quando o computador tem). Cada ícone só aparece se o sistema tiver aquilo.
- Clique num ícone abre um painel no estilo da Central de Controle do macOS:
  - **Som:** trilho de volume (clicar ou arrastar), interruptor "Sem som" e "Configurações de som…". Mexer no volume tira o mudo.
  - **Wi-Fi:** interruptor, redes vistas (sinal, cadeado nas redes com senha, ✓ e negrito na conectada), clique numa rede conecta (redes salvas ou abertas) e "Configurações de Wi-Fi…".
  - **Bluetooth:** interruptor, aparelhos pareados (azul e "Conectado" quando conectado), clique conecta ou desconecta, e "Configurações de Bluetooth…".
  - **Bateria:** nível, se está carregando e "Configurações de bateria…".
- Com um painel aberto, passar o mouse pelos ícones troca de painel, como nos menus.
- O estado vem de `wpctl` (som), `nmcli` (Wi-Fi) e `bluetoothctl` (Bluetooth), lidos numa thread a cada 2 s (ou na hora, depois de uma ação), para a barra nunca travar. As configurações abrem o `gnome-control-center`.
- Código: `src/system.rs` (leitura e ações, com testes da leitura).
- Nos testes, `MIDE_SYSTEM_DRY_RUN` simula o estado e as ações só vão para o log: nenhum teste mexe no som, no Wi-Fi ou no Bluetooth de verdade.

## Configurações do Sistema (B7, parte 1)

- Novo programa `mide-settings`, no estilo dos Ajustes do Sistema do macOS: barra lateral cinza com as seções (ícones do tema do usuário sobre quadradinhos coloridos) e, à direita, o título e cartões brancos arredondados com as opções. A janela redimensiona e o conteúdo comprido rola com a roda do mouse.
- Seções:
  - **Wi-Fi:** interruptor e lista de redes (sinal, com senha, conectada); clique numa rede conecta.
  - **Bluetooth:** interruptor e aparelhos pareados; clique conecta ou desconecta.
  - **Som:** volume de saída (trilho com porcentagem, clicar ou arrastar) e "Sem som".
  - **Papel de parede:** miniaturas do gradiente do mide e das imagens de `/usr/share/backgrounds` (e `~/.local/share/backgrounds`), carregadas numa thread. Clique aplica **na hora**.
  - **Efeitos:** interruptores de sombras, cantos arredondados, animações e efeito lâmpada, valendo **na hora**.
  - **Atalhos de teclado:** os atalhos em uso, com a ação em português e as teclas ("Ctrl ⇧ Espaço").
  - **Sobre:** as especificações da máquina e a versão do mide.
- Abre pelo menu do mide ("Configurações do sistema…", que antes ficava desativado) e pelos links "Configurações de Wi-Fi/Bluetooth/som…" dos painéis da barra, já na seção certa (`mide-settings wifi`, `sound`...). A bateria ainda abre as configurações do GNOME.
- **Gravar sem perder comentários:** as escolhas vão para o `~/.config/mide/config.toml` com `toml_edit`, que mantém os comentários e a formatação do usuário (`src/settings_file.rs`, com testes). Efeito ligado é o padrão: ao religar, a chave é apagada em vez de gravar `true`.
- **Valer na hora:** novo comando `reload` no canal de controle; o compositor relê o `config.toml` (atalhos, efeitos e papel de parede) e o papel de parede troca no quadro seguinte. Novo comando `bindings` lista os atalhos em uso.
- Código compartilhado novo: `src/backdrop.rs` (ajuste de imagem à tela e gradiente padrão, usado pelo compositor e pelas miniaturas) e `src/sysinfo.rs` (especificações da máquina, usado pelo "Sobre" e pelas Configurações).
- Arquivos `.desktop` dos programas do mide em `data/applications/` (Configurações do Sistema e Sobre), para o instalador copiar; com eles, as Configurações aparecem no launcher. Até lá, a dock mostra o ícone de configurações do tema para a janela delas.

## Rodada de ajustes 3 (lentidão, Configurações, roda do mouse, animações)

### Lentidão (volume e mide maximizado)
- O próprio código do mide agora compila com otimização leve em desenvolvimento (`[profile.dev] opt-level = 1`): os laços de desenho em tela cheia (menus da barra e da dock, launcher) ficaram várias vezes mais rápidos. Os ícones do launcher carregam em ~0,27 s (antes ~0,74 s).
- A barra e a dock desenham uma vez por lote de eventos (antes, cada movimento do mouse redesenhava o menu inteiro, e arrastar o volume virava dezenas de desenhos de tela cheia por segundo).
- Com a janela do mide mudando de tamanho (maximizar), o papel de parede é esticado e só é refeito no tamanho novo quando o tamanho para de mudar por 0,15 s (antes era refeito a cada passo do redimensionamento).

### Configurações sempre à mão
- Os programas abertos pelo mide recebem a pasta de dados do mide em `XDG_DATA_DIRS` e a pasta dos programas do mide no `PATH` (`child_env` em `src/main.rs`). Assim os `.desktop` do mide (em `data/applications/` no desenvolvimento, `share/mide` instalado, ou `MIDE_DATA_DIR`) aparecem no launcher e na dock, e "Manter na Dock" funciona para as Configurações.
- As Configurações do Sistema vêm fixadas na dock por padrão, como no macOS (para quem ainda não tem `dock.toml`; quem já tem fixa com clique direito → Manter na Dock).
- Ícones de app que só existem no tema do usuário (como o das Configurações) agora são encontrados: a busca de ícone de app tenta o tema padrão e depois o tema do usuário.

### Roda do mouse no launcher
- Causa: a roda do mouse chegava aos programas só como "passos" (v120), sem o valor contínuo que muitos programas leem; o launcher lia só o valor contínuo e não via nada. O compositor agora manda os dois (15 px por passo), e o launcher lê os passos.
- Rolar para baixo vai para a página da direita; para cima, para a da esquerda (um passo, uma página).

### Animações do launcher
- **Abrir:** esmaece de transparente para opaco e chega levemente ampliado (1,06 → 1,0) em 0,26 s, como o Launchpad, começando no primeiro quadro desenhado. **Fechar:** esmaece em 0,16 s (também ao abrir um app pelo launcher).
- **Carrossel ao trocar de página** (roda, botão do meio, Page Up/Down, pontos, borda ao arrastar): a página atual desliza para um lado e a nova entra pelo outro, em 0,32 s com desaceleração suave. As duas páginas são desenhadas uma vez no começo e depois só deslizam.
- **Pasta nova:** ao soltar um app sobre outro, a pasta surge crescendo com um pequeno efeito de mola (0,34 s).
- Os quadros da animação saem a cada ~16 ms só enquanto algo anima; parado, o launcher não redesenha.

### Efeito lâmpada ao abrir pela dock
- Abrir um app pela dock (clique ou Ctrl+clique) faz a primeira janela dele sair do ícone com o efeito lâmpada (o mesmo de restaurar). Abrir pelo launcher ou pelo atalho não tem o efeito.
- A dock avisa o compositor (`launching <ids>`); quando a primeira janela daquele app chega com conteúdo, a animação começa (avisos expiram em 15 s; cada janela é conferida uma vez).

### Autoteste
- Passos novos: `wheel down` / `wheel up` (roda do mouse) e `mclick x y` (botão do meio).

### Dock menor e configurável
- Ícones da dock passaram de 44 para 40 px (padrão).
- Nova seção **Dock** nas Configurações do Sistema: trilho "Tamanho dos ícones" de 32 a 64 px (Pequeno/Grande), com uma prévia da dock no tamanho escolhido. Grava `[dock] icon_size` no `config.toml` (sem a chave quando volta ao padrão de 40 px) ao soltar o trilho.
- A dock confere o `config.toml` junto com a atualização dos apps abertos (a cada 0,8 s): com o tamanho mudado, recarrega os ícones no tamanho novo e se ajusta sozinha, sem reiniciar.
- Clique direito no ícone do Launchpad: "Abrir o Launchpad" e "Ajustes da Dock…" (abre as Configurações na seção Dock), como o menu da dock no macOS.
- `settings_file` agora grava e lê números inteiros (com teste).

## Rodada de ajustes 4 (efeitos, desfoque, papéis de parede)

### Bugs corrigidos
- **Ligar e desligar efeitos não funcionava:** o `config.toml` do usuário tinha uma seção `[dock]` vazia (criada pelas Configurações), e o leitor do compositor tratava qualquer seção desconhecida como atalho e recusava o arquivo inteiro: o `reload` falhava e, ao reiniciar, o mide também recusaria o arquivo. Agora o compositor ignora seções de outros programas (teste com o caso exato), e as Configurações não deixam mais seções vazias quando uma opção volta ao padrão.
- **Pasta com mais de uma página:** a roda trocava de página sem transição. Agora tem o mesmo carrossel da grade, deslizando só dentro do painel da pasta (recortado pelos cantos arredondados dele).
- **Configurações sem barra de rolagem:** barra fina no estilo do macOS na borda direita, que só aparece quando o conteúdo não cabe, engrossa com o mouse em cima e pode ser arrastada (clique no trilho pula para o ponto).

### Desfoque (blur)
- Novo módulo `src/blur.rs` no compositor, com a técnica "dual Kawase" (a do KDE e do Hyprland): reduções pela metade e ampliações de volta, misturando vizinhos. Barato na GPU.
- **Launcher:** o fundo é o papel de parede ativo, bem desfocado (5 passadas), por baixo do véu, como o Launchpad. Calculado uma vez por papel de parede e tamanho de tela.
- **Dock:** desfoca o que estiver atrás dela (janelas e papel de parede) a cada quadro (3 passadas), recortado pelos cantos arredondados da dock: efeito de vidro fosco. Medido: o detalhe atrás da dock cai de 29 para 10 (desvio dos pixels).
- **Configurações → Efeitos:** interruptores "Desfoque na dock" e "Desfoque no launcher" (`[effects] dock_blur` e `launcher_blur`), valendo na hora.

### Papéis de parede do usuário
- A lista lê `/usr/share/backgrounds` e `~/.local/share/backgrounds` (até 60 imagens), em ordem alfabética.
- Último quadro da grade: **"+ Adicionar imagem…"**, que abre um seletor no visual do mide (folha por cima das Configurações): lugares (Imagens, Downloads, Pasta pessoal), botão "‹" para subir de pasta, pastas e miniaturas das imagens PNG/JPEG (carregadas numa thread), rolagem e Cancelar.
- Escolher uma imagem copia ela para `~/.local/share/backgrounds` (nome repetido ganha " 2") e já aplica como papel de parede.
- Imagens adicionadas mostram um **×** com o mouse em cima: tira da lista (apaga só a cópia; se era o papel de parede atual, volta para o gradiente do mide).

## Framework mide-ui (D1, parte 1)

- Novo módulo `mide::kit` (`src/kit/`): o jeito de criar apps com a interface nativa do mide, pensado para a comunidade.
- Estilo "imediato" (como o egui): o app implementa `App::view(&mut self, ui)` descrevendo a tela a cada quadro; os componentes desenham a si mesmos e respondem se foram clicados ou mudaram. Sem eventos para ligar: o estado é o próprio `struct` do app.
- Componentes: título, grupo, texto, nota, botão, botão de destaque, interruptor, trilho, campo de texto (foco, digitação, apagar, Enter), item de lista, ícone do tema, separador e espaço.
- Layout: coluna (padrão), `row` (lado a lado), `card` (cartão branco com linhas entre os itens), `page` (área com margem que rola, com a barra fina do macOS) e `sidebar` (barra lateral com seções, igual às Configurações).
- Tema (`Theme`): cores, cantos, fontes e espaçamentos num lugar só; o padrão é o tema claro do mide.
- Janela (`Window`) e execução (`run`): abre a janela com a barra do mide, cuida de mouse, roda, teclado e desenho, e redesenha só quando algo muda (e de novo quando um clique muda o estado).
- Exemplo `examples/mide-ui-demo.rs` (barra lateral, controles, lista de tarefas) e guia em português `docs/mide-ui.md`.
- Testes sem tela (desenho numa imagem em memória e mouse simulado): clique, interruptor, campo de texto com Enter e layout em coluna/linha.

## Launcher rápido (animações sem lentidão)

Medido em 1920x1080: cada quadro do launcher levava de 70 a 240 ms (4 a 14 quadros por segundo). Causas e correções:
- **Véu (gradiente de tela cheia): 63 ms por quadro.** O gradiente genérico do tiny-skia é lento em tela cheia; agora o véu é preenchido linha a linha (uma cor por linha, `fill_vertical_fast`), ~1 ms.
- **Abrir e fechar redesenhavam e ampliavam a tela inteira na CPU a cada quadro.** Agora quem anima é o compositor, na GPU: o launcher desenha uma vez e o compositor aplica o esmaecer e o zoom (1,06 → 1,0 ao abrir em 0,26 s; esmaecer e afastar de leve ao fechar em 0,17 s). O fundo desfocado esmaece junto. Fechar (Esc, clique no vazio, abrir um app, atalho, dock) passa pelo compositor, que esmaece e só depois fecha a camada.
- **Carrossel:** no começo da troca, a página nova é desenhada uma vez já no formato da tela (a antiga é o quadro que já estava na tela); cada quadro só copia as linhas deslocadas das duas, sem desenhar nada: **1,2–2,7 ms por quadro** (antes ~70 ms). Vale também dentro das pastas.
- **Ícones reduzidos a cada quadro:** agora cada ícone é reduzido uma vez para o tamanho da grade (assim que chega da thread de carregamento) e guardado.
- **Primeira troca de página:** com o launcher parado e os ícones carregados, as outras páginas são desenhadas uma vez fora da tela, para os nomes e as miniaturas já estarem prontos: a primeira troca caiu de 67 ms para ~10 ms.
- Conversão de cor para a tela mais rápida (4 bytes de uma vez) e otimização do código do mide em desenvolvimento (`opt-level = 2`).
- Resultado: quadro parado de 70 ms para 6–8 ms; carrossel a ~1,5 ms por quadro.

## Seleção no launcher com o vidro da dock
- O destaque do app selecionado (setas ou mouse) agora cobre o ícone **e o nome**, com o mesmo vidro da dock: preenchimento claro translúcido, borda mais escura em cima e mais clara embaixo e o brilho interno. O desenho do vidro virou uma função só (`mide::ui::draw::glass`), usada pela dock e pelo launcher.

## Arquivos do mide (B8) — gerenciador de arquivos estilo Finder

- Novo app `mide-files` ("Arquivos"), o primeiro app de verdade feito com o mide-ui. Atalho `data/applications/mide-files.desktop` (ícone do gerenciador de arquivos do tema; aparece no launcher e pode ser fixado na dock).
- **Barra lateral:** Favoritos (Pasta pessoal, Área de trabalho, Documentos, Downloads, Imagens, Músicas, Vídeos, pelas pastas do `user-dirs.dirs`, só as que existem) e Lugares (Computador = `/`, Lixeira). A pasta aberta fica marcada.
- **Barra de ferramentas:** voltar e avançar (histórico), nome da pasta, ícones ou lista (o modo escolhido fica marcado) e busca (filtra enquanto digita, sem diferenciar maiúsculas e acentos). Na Lixeira, o botão "Esvaziar".
- **Grade de ícones** (ícones do tema por tipo de arquivo; imagens com miniatura carregada em segundo plano, sem travar a janela) e **lista** (ícone, nome, data em português e tamanho).
- Clique seleciona (vidro do mide atrás do ícone e o nome numa pílula azul, como no Finder); duplo clique abre (pasta entra, arquivo abre no app padrão); clique no vazio tira a seleção.
- **Menu de contexto** (botão direito): no item, "Abrir", "Renomear…", "Copiar caminho" e "Mover para a Lixeira"; no vazio da pasta, "Nova pasta", "Mostrar/Esconder arquivos ocultos" e "Copiar caminho da pasta". Na Lixeira: "Restaurar" e "Apagar para sempre" (no vazio, "Esvaziar a Lixeira").
- **Renomear e Nova pasta:** a pasta nova já entra pedindo o nome; o campo vem com o nome todo selecionado (digitar substitui), Enter confirma, Esc cancela. Nome repetido, vazio ou com "/" mostra o erro.
- **Teclado:** Enter abre, Delete manda para a Lixeira (na Lixeira, apaga), Backspace sobe uma pasta, setas mudam a seleção, Ctrl+N nova pasta, Ctrl+H ocultos, Ctrl+F busca, Ctrl+R renomeia.
- O título da janela acompanha a pasta aberta (aparece na barra do mide).
- `mide-files [pasta]` abre a pasta pedida; `mide-files trash:///` abre a Lixeira.
- Pastas e a Lixeira abertas pela dock (Downloads, Lixeira, "Mostrar na pasta") agora abrem no Arquivos; se ele não estiver instalado, continua o app padrão do sistema.
- Lógica sem interface em `mide::files` (listar com pastas primeiro e ordem alfabética sem acentos, filtrar, tamanho e data em português, nome livre para "Nova pasta", criar, validar nome e renomear sem sobrescrever, favoritos).

## mide-ui (D1, parte 2): o que o Arquivos precisou

- **Menu de contexto:** `ui.context_menu(&resposta, |m| { m.item(..), m.separator(), m.danger(..) })`. Abre com o botão direito no componente, desenhado numa camada por cima de tudo (com sombra), abre para o outro lado se não couber, bloqueia cliques no que está embaixo, fecha ao escolher, ao clicar fora ou com Esc.
- **Respostas:** `double_clicked()` (dois cliques em até 450 ms no mesmo componente), `secondary_clicked()` (botão direito) e `id`.
- **Fundo:** `ui.background(ui.region())` reage ao clique e ao botão direito só onde nenhum componente está (menu do vazio da pasta).
- **Grade que quebra linha:** `ui.wrap(|ui| ...)`; **ladrilho** `ui.tile(Tile::Icon(nome) | Tile::Image(&pixmap), "nome", selecionado, tamanho)`.
- **Linha de lista** `ui.list_row(ícone, nome, &[(coluna, largura)], selecionada)`.
- **Barra de ferramentas** `ui.toolbar(|ui| ...)`, botões de ícone `ui.icon_button` e `ui.icon_toggle` (marcado), `ui.strong` (texto numa linha), `ui.push_right(largura)` (alinha o resto à direita) e `ui.sized(largura, ...)`.
- **Barra lateral com títulos de grupo** (item sem ícone) e `ui.sidebar_area` (conteúdo sem margem nem rolagem, para uma barra de ferramentas fixa em cima de uma `page`).
- **Campo de texto:** `ui.focus_next_input()` dá o foco ao próximo campo com o texto todo selecionado (digitar substitui; setas tiram a seleção); `ui.typing()` diz se há um campo com foco.
- **Teclas e atalhos:** Delete e setas em `Key`; `ui.input().shortcut("n")` para Ctrl+letra; botão direito.
- **Janela:** `ui.set_title(..)` troca o título; `ui.request_tick()` pede outro quadro em 40 ms (para trabalho em segundo plano, como miniaturas).
- O autoteste do compositor ganhou a tecla `delete`.

## Vidro líquido na dock (estilo Liquid Glass), leve

- **Shader novo no compositor** (`src/blur.rs`, composto da dock), numa passada só, a partir da distância até a borda arredondada (calculada no shader, sem texturas extras):
  - **Refração:** perto da borda, a imagem de trás é puxada para dentro como numa lente; a borda parece vidro grosso e curvo (as pontas da dock "entortam" o que está atrás).
  - **Tom adaptativo:** um pouco mais de saturação e de luz, mais forte sobre fundos escuros, para os ícones continuarem legíveis.
  - **Reflexo:** linha fina e clara na borda, mais forte do lado da luz (em cima, à esquerda), e brilho suave na faixa curva.
- Com o vidro líquido, o desfoque fica mais leve (uma passada a menos): o fundo aparece mais nítido e distorcido, como no Liquid Glass, e custa menos.
- A dock (cliente) desenha só um véu claro leve (`mide::ui::draw::glass_veil`); a borda e o reflexo vêm do compositor. Sem vidro líquido (ou sem desfoque), volta o vidro clássico (`glass`).
- **Configurações → Efeitos → "Vidro líquido"** (`[effects] liquid_glass`, ligado por padrão). Depende do "Desfoque na dock".

## Desfoque da dock mais leve

- **Desfoque só quando o fundo muda:** cada quadro calcula uma assinatura do que está atrás da dock (posição, tamanho, opacidade e conteúdo novo de cada janela); se for igual à do quadro anterior, reaproveita o desfoque pronto. Com a tela parada: **1 desfoque em 429 quadros** (antes, um por quadro). Durante animações (lâmpada, maximizar, launcher) o desfoque é refeito a cada quadro.
- **Texturas reaproveitadas:** o fundo da dock e as passadas do desfoque usam texturas guardadas entre quadros (antes, uma textura do tamanho da tela era criada a cada quadro), e só a região da dock é limpa.

## Medição de desempenho e resposta (a cada mudança)

- **`tests/perf.sh`** roda cenários típicos numa sessão de teste isolada (desktop parado, dock com menu, launcher com troca de página e busca, Arquivos navegando) e mostra mediana, p95 e máximo de: quadro do compositor (CPU), quadro da dock, menu da dock, quadro do launcher, quadro dos apps do mide-ui e **resposta** (do clique, tecla ou roda até o app enviar o quadro seguinte). Mostra também quantas vezes o vidro da dock foi refeito.
- O compositor marca cada entrada com o cliente que a recebeu e registra `resposta (apertar|soltar|tecla|roda): N ms` no próximo commit desse cliente.
- Logs de tempo (nível debug): `quadro do compositor`, `quadro da dock`, `menu da dock desenhado`, `quadro do app` (mide-ui), `quadro do launcher`.

## Rodada de desempenho (medido antes → depois)

- **mide-ui (Arquivos):** quadro de 17 ms (mediana) / 30 ms (p95) → 5 ms; resposta ao clique de 17 → 5,6 ms (p95 19,7 → 9 ms). Bancada sem tela (tela do Arquivos com 60 ícones): `view` de 6,3 → 1,0 ms.
  - Páginas e cartões desenhavam numa imagem de 4000 px de altura por quadro (~9,6 MB) e colavam com mistura de transparência; agora a imagem cobre só a parte visível, e a página copia os pixels de/para a janela (sem misturar). Página vazia: 4,8 → 0,19 ms. Também corrige conteúdo que sumia depois de 4000 px (pastas grandes).
  - Ícones e miniaturas colados com `mide::ui::draw::blit` (mistura própria, pixel opaco copiado, transparente pulado), em vez do `draw_pixmap` do tiny-skia: ~60 → ~13 µs por ícone de 64 px. Teste confere que o resultado é igual.
  - **Cache de texto** (`mide::ui::text`): cada texto (conteúdo, tamanho, peso, cor) é composto uma vez e depois só colado. Vale para todos os clientes (dock, barra, launcher, Configurações, apps). 20 rótulos: 0,86 → 0,12 ms.
  - Ícones do tema compartilhados no cache (sem cópia por desenho).
- **Launcher:** quadro de 5,5 → 3,2 ms (cache de texto).
- **Menu da dock:** primeira abertura de 32 → 3 ms. As fontes eram carregadas só no primeiro menu (~60 ms); agora carregam numa thread logo que a dock abre, já com os textos comuns compostos. O menu (camada de tela cheia, para o clique fora fechar) desenha e converte só a caixa dos painéis e avisa o compositor só da área que mudou.
- **Vidro da dock:** com a tela parada, refeito 1 vez em ~550 quadros; com o launcher aberto, de 237 → 31 vezes (só nas animações e trocas de página). Quadro do compositor: ~0,25 ms de CPU.

## Correção: sessão travando no primeiro quadro da dock

- Em ~25% das sessões, o compositor travava no terceiro quadro (o primeiro com a dock): com a dock ainda minúscula, duas passadas do desfoque tinham o mesmo tamanho (1x1) e, com as texturas reaproveitadas, usavam a mesma textura, então a GPU lia e escrevia na mesma imagem e o driver travava. Agora cada passada tem a sua textura e áreas menores que 16 px não são desfocadas. 20 de 20 sessões seguidas sem travar. Era a causa das falhas aleatórias recentes do `ui.sh`.

## Papel de parede padrão novo (formas marcadas)

- O gradiente simples deu lugar a **faixas curvas sobrepostas** sobre um céu azul-noite, do índigo ao pêssego, passando por roxo, magenta e coral (inspirado nos fundos do macOS, com desenho próprio). Cada faixa faz sombra suave na de trás, tem um brilho fino na borda e clareia perto dela (volume, como seda dobrada). Bordas sem serrilhado e pontilhado leve contra faixas no degradê.
- As formas usam a altura da tela como medida (não esticam em telas largas). Gerado uma vez por tamanho de tela, em todos os núcleos: 1920x1080 em ~77 ms. Vale para o desktop, o fundo do launcher e a miniatura nas Configurações.

## Testes em 1366x768 (resolução mais comum no Brasil)

- O mide aninhado (`cargo run`) abre em **1366x768** por padrão (`MIDE_WINIT_SIZE=LxA` muda).
- Nova bateria **`tests/res1366.sh`**: barra na largura toda, dock centralizada embaixo, launcher em tela cheia, clique na dock abre o app, janela nova abaixo da barra, maximizar ocupando a área útil, Arquivos e Configurações cabendo na tela, e capturas de tela (desktop, launcher, Arquivos, Configurações, menu da dock) em `/tmp/mide-1366` para revisão.
- `tests/perf.sh` mede em 1366x768. `tests/ui.sh` segue em 800x600 (coordenadas fixas), agora explícito no `scenario.sh`.
- Janelas de teste com tamanho fixo (`MIDE_WINIT_FIXED`): o GNOME do anfitrião (com "auto-maximizar") maximizava a janela de 1366x768 ao abrir, e as coordenadas não valiam.

## Correção: testes e apps congelando com a janela do mide escondida

- O mide aninhado só desenhava quando o anfitrião pedia (`Redraw`). Com a janela coberta ou escondida (por exemplo, o usuário trabalhando enquanto os testes rodam), o anfitrião para de pedir, e o autoteste e os apps (que esperam o aviso de quadro) congelavam. Era a outra causa das falhas aleatórias do `ui.sh`.
- Agora um relógio de reserva, depois de 100 ms sem pedido do anfitrião, avança o autoteste, termina animações e libera os apps (callbacks de quadro), sem apresentar na tela escondida. Em 10 sessões seguidas, todas completaram o roteiro, inclusive as 4 em que o anfitrião parou de pedir quadros.

## Vidro em barras, menus e notificações (regiões de vidro)

- **Regiões de vidro no compositor:** qualquer camada pede vidro (desfoque e vidro líquido) atrás de retângulos dela pelo canal de controle: `glass <camada> raio:x,y,w,h ...` (sem regiões, limpa). O compositor desenha cada região logo atrás da camada, de trás para a frente (o vidro de um menu vê a dock já com o vidro dela), com cache e texturas próprias por região: refeito só quando o que está atrás muda.
- O caso especial da dock virou a primeira região (`mide-dock`). Agora também têm vidro:
  - **barra superior** (a faixa inteira; véu claro mais leve, o texto escuro continua legível; sem vidro com o launcher aberto);
  - **menus da barra** (sistema, app, calendário, som, Wi-Fi, Bluetooth);
  - **menus da dock** (apps, Downloads, Lixeira, submenus);
  - **banners de notificação**.
- Painéis translúcidos apagam a própria sombra por baixo (`mide::ui::draw::clear_rounded`), senão o vidro ficaria acinzentado.
- No lado do cliente: `mide::control_client::Glass` (guarda o último pedido e só avisa quando muda) e `panel_glass()`; `mide::settings_file::effect(chave)` lê os efeitos com cache pela data do arquivo.
- **Configurações → Efeitos → "Vidro em barras e menus"** (`[effects] panel_glass`, ligado por padrão). A força do vidro líquido segue a chave "Vidro líquido".
- Desempenho: compositor ~0,27 ms por quadro (antes ~0,25); com a tela parada, cada região é refeita só ao aparecer.
- Próximo uso: o buscador nativo (B11) e `ui.glass_panel` no mide-ui vão pedir vidro do mesmo jeito.

## Buscador do mide (B11), estilo Spotlight, sobre vidro líquido

- Novo app **`mide-search`**, feito com o mide-ui: barra flutuante centralizada (a 20% do topo) sobre o vidro do mide, aberta com **`ctrl+space`** (ação `toggle_search` no `config.toml`, ou `search` no canal de controle). Substitui o buscador de terceiros.
- Encontra, na ordem:
  - **contas** (`2+3*4`, `(3,5*4)/2`, `2^10`; Enter copia o resultado);
  - **apps** (os que têm uma palavra começando pela busca vêm primeiro);
  - **seções das Configurações** (pelo nome e por palavras-chave: "volume" acha Som);
  - apps que só casam no meio da palavra;
  - **arquivos e pastas** das pastas pessoais (palavra começando pela busca, como o Spotlight).
- Até 8 resultados com ícone, título e detalhe (tipo ou pasta). Setas escolhem, Enter abre, Esc fecha, o atalho de novo fecha, clique fora fecha, e abrir um resultado fecha o buscador.
- O índice de arquivos (até 4 níveis nas pastas pessoais, 20 mil itens, sem ocultos) e os ícones dos apps são carregados em segundo plano: a barra abre na hora.
- Lógica sem interface em `mide::search` (contas, números em português, índice e busca de arquivos, seções das Configurações), com testes.
- Desempenho (1366x768): quadro de 7,7 ms (p95 19 ms) e resposta à tecla de 8,3 ms (p95 15 ms). Antes das otimizações abaixo: 37 ms (p95 108 ms) e 23 ms.

## mide-ui (D1, parte 3)

- **`Window::overlay(nome)`**: o app vira uma camada transparente por cima de tudo, na tela inteira e com o teclado só para ela (buscador, painéis).
- **`ui.glass_panel(x, y, largura, raio, |ui| ...)`**: painel com o vidro do mide (desfoque e vidro líquido desenhados pelo compositor atrás dele, em camadas), sombra e véu claro; a altura acompanha o conteúdo.
- **`ui.result_row(ícone, título, detalhe, selecionada)`**: linha de resultado com ícone de 28 px (nome do tema, de app ou caminho).
- **`ui.close()`** fecha o app; **`ui.provide_icon(...)`** entrega ícones carregados pelo app.
- **Ícones assíncronos:** os componentes pedem o ícone a uma thread e desenham sem ele até ficar pronto (carregar um SVG leva milissegundos). Vale para o Arquivos também.
- Desenho rápido de painéis: `mide::ui::draw::paint_rounded_solid` (véu escrito linha a linha, só a borda mistura) e `soft_shadow` (sombra calculada nos pixels da faixa de fora). Painel de vidro com 8 linhas: 25 → 3,4 ms.

## Rodada de ajustes do usuário (launcher, Arquivos, dock, barra, teclado, Configurações)

### Launcher
- **Sem piscada ao fechar:** quando o esmaecer terminava, o launcher voltava a aparecer inteiro por alguns quadros até a camada sumir. Agora fica invisível até sair.

### Arquivos
- **Lista igual ao Finder:** setinha nas pastas para abrir dentro da própria lista (também com → e ←), recuo por nível, **linhas alternadas**, cabeçalho com **Nome, Modificado, Tamanho e Tipo** que ordena ao clicar (de novo, inverte), tamanho das pastas calculado em segundo plano ("…" até ficar pronto), tipo em português ("Imagem PNG", "Documento PDF", "Pasta"...).
- **Barra lateral com ícones simples** (de uma cor só, na cor de destaque, como no Finder).
- **Menu de contexto mais completo:** Abrir, Abrir no Terminal (pastas), Renomear…, Duplicar, **Comprimir** (ou **Extrair aqui** em .zip), Copiar, Copiar caminho, Mover para a Lixeira. No vazio da pasta: Nova pasta, **Novo arquivo**, Colar (com algo copiado), Abrir no Terminal, ocultos e Copiar caminho da pasta.
- **Novo arquivo:** cria "Novo arquivo.txt" com o nome todo selecionado; digitar "notas.md" já define a extensão.
- **Compactar e extrair .zip nativos** (biblioteca `zip`, em Rust, sem programa externo), numa thread; caminhos maliciosos no .zip são ignorados.
- Atalhos: Ctrl+C copia, Ctrl+V cola (com número se o nome existir), Ctrl+D duplica.
- Lógica em `mide::files` (tipo, nomes livres, criar, duplicar, copiar, tamanho de pasta com limite, compactar, extrair), com testes.

### Dock
- **Janelas maximizadas respeitam a dock:** a dock reserva o espaço dela (zona exclusiva), como no macOS.
- **Ocultar e mostrar automaticamente** (Configurações → Dock, `[dock] autohide`): a dock desliza para fora deixando uma faixa invisível de 2 px; o ponteiro na borda de baixo a faz subir, e ela desce ao sair (não esconde com um menu aberto). As janelas usam a tela toda.
- **Menus com a ponta** apontando para o ícone de onde vieram. Corrigido também o vidro dos menus da dock, que era pedido em coordenadas erradas.

### Barra superior
- **Relógio só com a hora** por padrão. No calendário, **"Formato da hora…"** liga dia da semana, dia e mês, ano e segundos (`[clock]`).
- **Central de controle** logo à esquerda do relógio: Wi-Fi e Bluetooth, **Não perturbe** (`[notifications] dnd`; as notificações não urgentes não aparecem), **brilho da tela** (com `brightnessctl` e tela com luz de fundo), som e "Configurações do Sistema…".
- **Engrenagem** que abre as Configurações.
- **Menu do mide como o do Mac:** Sobre este computador, Configurações do Sistema…, Forçar encerrar o app, Repouso, Reiniciar…, Desligar…, Bloquear tela, Encerrar sessão…. Reiniciar, desligar e encerrar a sessão pedem confirmação num painel; nos testes nada é executado (só registrado).
- **Forçar encerrar** (comando `force_quit` no canal de controle) termina o processo do app em foco.
- Todos os menus e painéis da barra com a **ponta** apontando para o título ou ícone; painéis de vidro um pouco mais transparentes.

### Teclado
- **Teclas no estilo Mac** (Configurações → Teclado, `[keyboard] mac_style`): *(mapeamento refeito em 2026-10-20, veja "Correções" no fim)* o Alt é o ⌘ Command, o Super é o ⌥ Option e o Ctrl continua Ctrl; ⌘C, ⌘V, ⌘Espaço abre o buscador. Desligado, tudo como no PC. A lista de atalhos mostra ⌘ e ⌥ com a opção ligada.

### Configurações (agora no mide-ui)
- O app foi refeito com o mide-ui (mesmo visual, menos código) e ganhou **busca** no topo da barra lateral: acha opções pelo nome ou por palavras-chave ("volume", "esconder", "teclas") e leva à seção.
- Novas opções: "Ocultar e mostrar a Dock automaticamente", seção **Teclado** com "Teclas no estilo Mac".
- Papéis de parede em ladrilhos retangulares; "Remover da lista" pelo botão direito nas imagens adicionadas.

### mide-ui
- `switch_detail` (interruptor com explicação), `slider` avisa ao soltar (`submitted`), `sidebar_search` (busca e ícones em quadradinho colorido), `tile_wide`, `list_header` e `tree_row` (lista em árvore), ícones "symbolic" pintados na cor de destaque, `ui.wake_after` (quadro agendado, sem redesenhar à toa) e menus de contexto com a ponta no ponto do clique.

### Transparência
- Barra, menus, notificações, central de controle e buscador um pouco mais transparentes sobre o vidro.

## Ajustes: dock inteligente, ícone da central, confirmações e prévias

- **Ocultar inteligente** (como no GNOME): com "ocultar automaticamente" ligado, a dock só some quando há uma janela onde ela fica (ou uma janela maximizada); sem janela por cima, fica à mostra. O compositor responde `dock_covered` (`yes`/`no`) e a dock pergunta a cada 250 ms.
- **Respiro de 2 px** entre a janela maximizada e a dock (zona reservada = altura da dock + 2). O espaço reservado agora acompanha a mudança do tamanho dos ícones.
- **Ícone da central de controle** igual ao do macOS: em cima, pílula vazada com a bolinha cheia encostada na ponta esquerda; embaixo, pílula cheia com a bolinha vazada na ponta direita.
- **Confirmações** (reiniciar, desligar, encerrar a sessão) no centro da tela, na horizontal e na vertical.
- **Prévias da dock:** o destaque da miniatura sob o ponteiro virou vidro claro (o mesmo estilo da seleção do launcher), sem os pontinhos brancos (o fundo escuro antigo deixava aparecer as bordas claras das letras do título). Desliga junto com "Vidro em barras e menus" (fica um véu branco simples).
- **Botão de fechar nas prévias** como no GNOME: círculo escuro com × branco no canto da miniatura sob o ponteiro (vermelho com o ponteiro em cima).
- Desempenho: compositor ~0,42 ms por quadro com as prévias abertas (`tests/perf.sh previas`).
- Plano: registrada a etapa futura **D2** (apps, widgets e extensões com o mide-ui, incluindo itens novos na central de controle).

## Central de notificações (B9)

- O servidor de notificações guarda cada notificação recebida num histórico (as 50 mais novas, em `$XDG_RUNTIME_DIR/<socket>-notificacoes`, separado por sessão), inclusive as silenciadas pelo **Não perturbe**.
- Clicar no relógio abre o calendário e, embaixo, a **central de notificações**: as mais recentes em cartões (app, hora, título e a primeira linha do texto), quantas couberem na tela (até 5, com "e mais N"), e **Limpar**.
- Lógica do histórico em `mide::notify` (gravar, ler, limpar e o formato da linha, com escape), com teste.

## Correções: pontinhos no launcher e diálogo de criar no Arquivos

- **Sem pontinhos brancos ao fechar o launcher:** os ícones eram reduzidos com filtro bicúbico, que "passa do ponto" nas bordas transparentes e gera pixels com cor maior que a opacidade. Quando o compositor esmaece o launcher, esses pixels clareiam demais. Agora o filtro é bilinear e os pixels são conferidos (`mide::ui::draw::fix_premultiplied`).
- **Diálogo (modal) para Nova pasta, Novo arquivo e Renomear** no Arquivos: a janela escurece e um painel branco aparece no meio, com o nome já selecionado, **Cancelar** e **Criar**. Nada é criado antes de "Criar": Cancelar ou Esc não criam nada. Sem mudar o nome, cria "Nova pasta", "Nova pasta (1)", "Nova pasta (2)"... (e "Novo arquivo.txt", "Novo arquivo (1).txt"); com outro nome, cria exatamente com ele, sem sobrescrever (nome repetido mostra o erro no diálogo).
- **mide-ui:** `ui.modal(largura, |ui| ...)` (escurece a janela, painel centralizado, só o conteúdo do modal reage). Painéis de vidro ficam opacos em janelas comuns (o vidro desfocado de verdade só existe em camadas).
- `mide::files`: `numbered_name` ("Nova pasta (1)") e `create_named` (cria pasta ou arquivo sem sobrescrever), com testes.
- Plano: registrada a etapa **B12** (escolher nas Configurações os itens da barra superior).

## Área de trabalho com ícones (B10)

- Novo app **`mide-desktop`** (mide-ui), iniciado pelo compositor: os arquivos e pastas da pasta "Área de trabalho" (do `user-dirs.dirs`) aparecem como ícones sobre o papel de parede, em colunas de cima para baixo a partir do canto direito, como no macOS, com o nome em branco com sombra.
- Clique seleciona (vidro e nome em pílula azul), duplo clique abre (pastas no Arquivos). Botão direito no ícone: Abrir, Renomear…, Duplicar, Copiar caminho, Mover para a Lixeira. No vazio: Nova pasta, Novo arquivo, Abrir no Terminal, Mostrar no Arquivos, Mudar papel de parede…. Criar e renomear usam o mesmo diálogo (modal) do Arquivos. Teclado depois de clicar: Enter abre, Delete manda para a Lixeira, Ctrl+R renomeia.
- A pasta é conferida a cada 2 s (mudanças feitas pelo Arquivos ou pelo terminal aparecem sozinhas).
- **Compositor: camadas de fundo** (`Bottom` e `Background`, pendência do A10) desenhadas entre as janelas e o papel de parede, e recebendo ponteiro e teclado (sob demanda) quando não há janela por cima; clicar nelas fecha popups abertos. Novo comando `workarea` (área útil sem barra e dock).
- **mide-ui:** `Window::desktop(nome)` (camada de fundo), `ui.at(x, y, largura, ...)` (posição livre), `Theme::tile_text` (nome claro com sombra) e **quadros iguais não são reenviados** ao compositor (um redesenho agendado sem mudança não faz o compositor refazer o vidro de quem está na frente).
- Testes usam uma pasta de área de trabalho temporária (`MIDE_DESKTOP_DIR`), nunca a do usuário.

## Ajustes: menus, vidro na janela, papel de parede e transições

- **Ponta dos menus de contexto embaixo do item** (no Arquivos e na área de trabalho): o menu nasce logo abaixo do nome do item, com a ponta encostada nele, sem a linha da borda cortando a ponta. Sem espaço embaixo, abre para cima com a ponta no topo do item.
- **Criar dentro da pasta:** clique direito numa pasta (na grade ou na lista) mostra "Nova pasta aqui dentro" e "Novo arquivo aqui dentro"; o diálogo cria dentro dela e, na lista, a pasta se abre para mostrar o item novo.
- **Volume e brilho na central de controle:** a parte cheia da barra virou azul do mide (antes, cinza claro sobre o vidro, quase invisível).
- **Vidro da dock acompanha o papel de parede:** o papel de parede tem uma "geração" que entra na assinatura do desfoque; trocar a imagem refaz o vidro. A imagem nova é lida numa thread (sem travar quadros) e entra esmaecendo sobre a antiga em 450 ms; o vidro acompanha a transição.
- **Barra lateral de vidro no Arquivos** (acesso rápido), com a mesma transparência dos menus da barra: o compositor ganhou **vidro em janelas comuns** (`glass app:<app_id> ...` no canal de controle; a região fica logo atrás do conteúdo da janela, com os cantos coincidindo com os da janela). Com "Vidro em barras e menus" desligado nas Configurações, a barra lateral fica opaca (a janela confere a opção uma vez por segundo, sem redesenhar). Os ícones da barra lateral agora têm a cor dos ícones da barra superior (escuros; brancos no item escolhido). mide-ui: `ui.glass_sidebar()`.
- **Barra superior esmaece junto com o launcher** ao abrir e fechar (antes, trocava de uma vez).
- **Fechar janela com transição:** o compositor fotografa a janela ao pedir o fechamento e, quando ela some, a foto encolhe 6% e esmaece em 180 ms.
- **Ícones do Arquivos aparecem logo:** o pedido de ícone em segundo plano agenda quadros até o ícone chegar (antes, os ícones das pastas só apareciam ao mexer o mouse).

## Itens da barra superior (B12)

- Nova seção **Barra superior** nas Configurações (também pela busca: "barra", "ícones", "bateria"...): interruptores para Configurações (engrenagem), Som, Wi-Fi, Bluetooth, Bateria e Central de controle. O relógio fica sempre.
- Grava em `[bar]` no `config.toml` (só o que está escondido, como `gear = false`); a barra relê a cada meio segundo (o arquivo só é lido de novo quando muda) e redesenha; se o painel de um item escondido estiver aberto, ele fecha.
- `mide::settings_file`: `BAR_ITEMS`, `bar_item` e `flag` (opção ligada/desligada com padrão e cache pela data do arquivo; `effect` passou a usar ela).

## Compositor parado não desenha

- Antes, o compositor pedia um quadro novo ao anfitrião depois de cada quadro: ~60 quadros/s mesmo sem nada mudando (~4% de um núcleo com a sessão parada).
- Agora desenha só quando algo muda: pedido de um cliente (commit, janela nova ou fechada), comando no canal de controle (as consultas `list`, `dock_covered`, `launcher state` etc. não contam), entrada (mouse, teclado) ou algo em movimento (animações, launcher, papel de parede trocando, janela fechando). `State::wake` marca e acorda o laço na hora por um `ping` do calloop (sem esperar relógio).
- O relógio de reserva (anfitrião sem pedir quadros) só age com algo pendente; parado, confere a cada 100 ms.
- Com autoteste (testes), continua desenhando sem parar, porque os roteiros contam quadros. Roteiro vazio (`tests/scenario.sh ""`) é uma sessão sem autoteste, para medir o consumo parado.
- Medido (1366x768, Arquivos aberto, 10 s parado): compositor de 60 para **0–2 quadros**; sessão inteira de **4,4% para 1,0%** de um núcleo; RAM total ~158 MB (compositor ~99 MB).

## Separação da cena, da política de janelas e do backend (preparo do A13)

- Refatoração sem mudança de comportamento, para o backend DRM (A13) reaproveitar tudo sem duplicar código.
- **`src/render.rs` (cena):** o que estava em `backend_winit.rs` e não era do winit foi para cá: `MideRenderElement`, montagem das janelas, decorações, efeito lâmpada, fotos de fechar, camadas, vidro, papel de parede, `send_frames` e títulos. `Scene` guarda os recursos de GPU (shaders, lâmpada, desfoque, papel de parede, títulos); `Scene::render` desenha num alvo qualquer do `GlesRenderer`, e `Scene::moving` diz se algo está em movimento.
- **`src/backend_winit.rs`** ficou só com a janela do anfitrião, a saída, os relógios, os eventos e a apresentação (`bind`, `Scene::render`, `submit`, `send_frames`). De 892 para 222 linhas.
- **`src/wm.rs` (política de janelas):** janela nova em cascata (`map_new_window`), fechar (`request_close`, `window_closed`), arrastar (`begin_move`), maximizar e restaurar (`maximize`, `unmaximize`), minimizar e restaurar minimizada, abrir saindo da dock, alvo na dock, área útil e ajuste à área. Os handlers do `XdgShellHandler` em `state.rs` só chamam essas funções.
- **Cursor sem tipo do winit:** `input::CursorShape` (padrão e quatro formas de redimensionar) no lugar do `CursorIcon` do winit em `State`; o backend winit traduz. No DRM, o compositor vai desenhar a imagem correspondente.
- **Entrada genérica:** `input::handle_event` aceita eventos de qualquer `InputBackend` do Smithay (winit hoje, libinput no A13).
- Testes: `cargo test` (102) passando, build sem avisos.

## Editor de atalhos nas Configurações (A9)

- Regra nova do projeto: tudo é configurável pela interface; o usuário não edita arquivo. A tela Teclado deixou de mandar "editar o config.toml".
- **Configurações → Teclado → Atalhos:** clicar num atalho abre um diálogo com a combinação atual e **Gravar…**, **Remover** e **Cancelar**. Gravar: o usuário aperta a combinação nova e ela vale na hora. Combinação de outra ação: aviso "… já é usado por …" com **Substituir** ou **Gravar outra**. Sem Ctrl, Alt ou Super a combinação é recusada (tomaria uma tecla de digitar); F1–F24 podem ficar sozinhas.
- **Novo atalho…:** escolhe uma ação do mide (buscador, launcher, fechar janela, restaurar minimizada, encerrar) ou, digitando na busca, um app instalado; depois grava a combinação. Atalhos de app aparecem com o nome do app ("Abrir Foot").
- **Restaurar os padrões:** tira a seção `[keybindings]` (`settings_file::clear_section`).
- **Gravação pelo compositor:** `shortcuts capture` no canal de controle faz a próxima combinação apertada ser guardada no formato que o compositor casa (`config::combo_name`, mesma tecla sem Shift e mesma troca do modo Mac), sem disparar atalhos nem chegar ao app; Esc sozinho cancela; `shortcuts captured` devolve `waiting`, a combinação, `cancel` ou `none`; expira em 20 s (o teclado nunca fica preso). As Configurações perguntam a cada 80 ms enquanto gravam.
- **Desligar um atalho:** a ação `none` remove a combinação, inclusive um atalho padrão (trocar a combinação de um atalho grava a antiga como `none` e a nova com a ação).
- Diálogo desenhado fora da área de conteúdo (`ui.modal` no `view`), centralizado na janela.
- Testes: 5 unitários novos (107 no total) e 6 verificações novas no `tests/ui.sh` (gravar, vale no compositor, antiga desligada, gravando não dispara atalho, conflito pede confirmação, restaurar padrões).

## Backend DRM (A13): o mide direto na tela

- **`src/backend_udev.rs`:** sessão pelo libseat (acesso à placa de vídeo e à entrada sem root), placa principal do assento, DRM/KMS com o primeiro monitor conectado no modo preferido, GBM + EGL + `GlesRenderer` (o mesmo renderizador do winit, então a cena é a mesma) e `GbmBufferedSurface` com formato opaco (XRGB8888) primeiro. O quadro é entregue com o ponto de sincronização da GPU e o próximo só sai depois do vblank; parado, não desenha (`state.wake` acorda pelo `ping`).
- **Entrada pelo libinput:** mouse e touchpad por deslocamento (ponteiro preso na tela), teclado, rolagem; touchpads com toque para clicar ligado.
- **Cursor desenhado pelo mide** (`src/cursor.rs`): tema do sistema (`XCURSOR_THEME`, `XCURSOR_SIZE`, padrão `default` 24 px, com nomes antigos de reserva), seta própria se não houver tema, superfície do app quando ele pede (com hotspot), e as formas de redimensionar do mide por cima. A cena desenha o cursor só quando o backend pede (`Scene::with_cursor`); no winit, o anfitrião mostra o dele.
- **Troca de console:** Ctrl+Alt+F1…F12 (`XF86Switch_VT_n` do xkb) troca pela sessão; ao sair, tela e entrada são liberadas; ao voltar, retomadas e redesenhadas.
- **Layout do teclado do sistema:** o assento lê `/etc/default/keyboard` (ex.: `XKBLAYOUT="br"`), a não ser que `XKB_DEFAULT_LAYOUT` esteja definido. Vale também no winit.
- **Escolha do backend:** `MIDE_BACKEND=drm` ou `winit`; sem ele, DRM quando não há `WAYLAND_DISPLAY` nem `DISPLAY` (iniciado de um console de texto).
- `Scene::render` passou a devolver o ponto de sincronização (`SyncPoint`) do quadro.
- Limites: uma placa e uma tela, sem hotplug de monitor, cursor desenhado na cena (sem plano de hardware), tela redesenhada inteira a cada quadro.

## Ajustes do teste do usuário: menu de contexto, cantos ao fechar e desempenho

- **Menu de contexto sem espaço em cima nem embaixo do item** (no Arquivos, item no meio de uma lista longa): antes, o menu subia até o topo da janela, cobria o item e a ponta apontava para outra linha. Agora abre **ao lado do clique** (à direita, ou à esquerda perto da borda), centralizado na altura do clique e dentro da janela, com a ponta na lateral apontando para o item (`ui::draw::menu_arrow_side`). Com espaço, continua embaixo ou em cima. A regra ficou numa função pura (`menu_place`) com testes.
- **Cantos vivos ao fechar janela:** a foto que esmaece na transição de fechar era desenhada como textura retangular. Agora passa pelo mesmo recorte arredondado das janelas (`Clipped` sobre a textura), com o raio acompanhando a escala da animação.
- **Lentidão ao arrastar o Arquivos (investigação):** medido na GPU real do usuário (Intel Alder Lake, sessão Cinnamon/X11): o desenho de cada quadro do arraste leva 0,7 ms (mediana) e 1,9 ms (p95), com o vidro refeito a cada quadro. O compositor não é o gargalo. No modo janela, cada quadro espera ~16 ms o driver/anfitrião liberar a apresentação, e o Cinnamon ainda compõe a janela do mide no quadro seguinte: são 2 a 3 quadros entre o mouse e a tela, o que dá a sensação de arraste "pesado". No backend DRM (A13) essa camada do anfitrião não existe. No llvmpipe (sem GPU), o vidro é o efeito mais caro (p95 28 ms contra 10 ms sem vidro), relevante para máquinas sem aceleração.
- **Medição de GPU:** `MIDE_GPU_TIMING=1` espera a GPU terminar o quadro e registra "quadro na GPU"; o `tests/perf.sh` passou a usar e mostrar.

## XWayland (A14): apps X11 dentro do mide

- **`src/xwayland.rs`:** na partida, o mide inicia o `xwayland-satellite` (procurado ao lado do executável, no PATH e em `~/.cargo/bin`) no primeiro display X livre (`:1`, `:2`...; o `:0` costuma ser de outra sessão), conectado ao socket Wayland do mide. O satellite roda o Xwayland e entrega cada janela X11 como janela comum (xdg-shell): barra de título, sombra, dock e barra superior funcionam igual. O compositor continua sem código X11.
- **`DISPLAY` para os apps do mide:** `child_env` passa o display do XWayland a tudo o que o mide abre (dock, launcher, atalhos, Arquivos). Um app X11 aberto pelo mide aparece dentro dele, e não no desktop por baixo (modo janela).
- **Protocolos novos no compositor:** `wp_viewporter` (o satellite exige) e `xdg-output` (posição lógica das saídas).
- O display fica gravado em `$XDG_RUNTIME_DIR/<socket>.xdisplay` (para ferramentas e testes); um arquivo velho é apagado na partida. O satellite é encerrado com o mide; se o mide morrer, ele perde a conexão e sai sozinho.
- `MIDE_XWAYLAND=0` desliga; os cenários de teste desligam por padrão e o teste de apps X11 liga.
- Sem `xwayland-satellite` ou `Xwayland` instalados, o log avisa e o mide segue sem apps X11.

## Campo de texto completo, área de transferência e Editor de Texto (D1 e E2)

- **Campo de texto (`ui.text_input`):** cursor em qualquer posição (setas, Home, End, clique), seleção (Shift+setas, arrastar, duplo clique na palavra, Ctrl+A), Ctrl+setas e Ctrl+Backspace/Delete por palavra, digitar e colar no lugar da seleção, rolagem horizontal para o cursor ficar à vista e texto recortado dentro do campo. A lógica fica em `kit::textedit` (pura, com testes).
- **Área de transferência entre apps:** Ctrl+C, Ctrl+X e Ctrl+V nos campos usam o `wl_data_device` do Wayland (o compositor já tinha o `DataDeviceState`). Copiar oferece o texto (`text/plain;charset=utf-8`, `UTF8_STRING`...); colar lê a seleção no laço de eventos, sem travar (o dono do texto pode ser o próprio app). Logs de depuração "área de transferência: …".
- **Teclas com os modificadores do momento:** cada tecla guarda o Ctrl e o Shift de quando foi apertada (`Input::key_events`, `shortcut_shift`). Antes valia o estado no fim do quadro, e uma combinação rápida (Ctrl+End apertado e solto no mesmo quadro) virava só End.
- **Área de texto (`ui.text_area`, `kit::textarea::TextDoc`):** várias linhas com números, fonte de largura fixa (`Text::draw_mono`), linha atual destacada, setas para cima e para baixo mantendo a coluna, Page Up/Down, Ctrl+Home/End, recuo automático no Enter, Tab com 4 espaços, desfazer/refazer (Ctrl+Z, Ctrl+Shift+Z, Ctrl+Y; digitação em sequência vira um passo só), rolagem pela roda e indicador de rolagem. Só as linhas à vista são desenhadas.
- **Editor de Texto (`mide-text`, E2):** um arquivo por janela; abre UTF-8 (recusa binário e arquivos acima de 20 MB), guarda se o arquivo usava CRLF e volta assim ao salvar; salvar escreve um temporário na mesma pasta, mantém as permissões e troca de uma vez; "Salvar como…" (Ctrl+Shift+S) num diálogo com o caminho; buscar e substituir (Ctrl+F), com anterior/próxima, substituir e substituir tudo; título com "•" e "editado" na barra quando há alterações; fechar com alterações pergunta (Salvar, Não salvar, Cancelar) pelo novo `App::close_requested`. `.desktop` próprio (aparece no launcher).
- **Arquivos de texto abrem no editor do mide:** `places::open` manda pastas para o Arquivos e arquivos de texto (`files::is_text`: extensão conhecida ou, sem extensão, conteúdo UTF-8 sem bytes nulos) para o `mide-text`; o resto continua com o app padrão do sistema.
- **Correção do compositor (foco roubado):** a cada redesenho, uma camada que aceita teclado pegava o foco. A área de trabalho (que só quer o teclado quando clicada) tirava o foco da janela recém-aberta, e o que se digitava ia para ela. Agora só camadas que exigem o teclado (launcher, buscador, menus) pegam o foco sozinhas no commit.

## Sessão de login (C2)

- **`data/wayland-sessions/mide.desktop`:** entrada "mide" para a tela de login (LightDM, GDM, greetd leem `/usr/share/wayland-sessions`). O instalador (C3) a copia.
- **`data/mide-session`:** o que a tela de login executa. Define `XDG_SESSION_TYPE=wayland`, `XDG_CURRENT_DESKTOP=mide`, `MIDE_SESSION=1`, `MIDE_BACKEND=drm` e as variáveis que fazem apps (Firefox, Qt, Electron, SDL, Java) preferirem Wayland; acha o `mide` ao lado do script, em `target/` (desenvolvimento) ou no PATH; abre um barramento de sessão próprio se a tela de login não abriu; grava o log em `~/.local/state/mide/session.log` (o anterior fica em `session.log.anterior`).
- **`src/session.rs`:** com `MIDE_SESSION=1`, o mide publica `WAYLAND_DISPLAY`, `DISPLAY` (XWayland), `XDG_CURRENT_DESKTOP` e `XDG_SESSION_TYPE` para os serviços do D-Bus e do systemd do usuário (`dbus-update-activation-environment --systemd`) e inicia os apps de inicialização automática (`~/.config/autostart` e `/etc/xdg/autostart`), com as regras do padrão: arquivo do usuário substitui o do sistema, `Hidden`, `OnlyShowIn`/`NotShowIn` (o mide se chama `mide`), `TryExec` e `X-GNOME-Autostart-enabled`. Rodando numa janela (desenvolvimento), não mexe na sessão de quem o abriu.
- **B1 conferido:** Reiniciar, Desligar e Encerrar sessão já estavam na barra com confirmação; Encerrar sessão encerra o compositor, o que numa sessão de login volta à tela de login. "Bloquear tela" chama `loginctl lock-session`, que só funciona com a tela de bloqueio (C1).
- **`tests/session-dev.sh`:** `sudo tests/session-dev.sh` instala "mide (desenvolvimento)" na tela de login apontando para o projeto; `sudo tests/session-dev.sh remover` tira.

## Correção: desenho invertido no backend DRM

- **Bug:** na sessão de verdade (DRM), o topo das janelas sumia, a barra de título ficava escura e transparente e o vidro aparecia fora do lugar. Causa: o recorte das janelas (`clip.rs`) e o da barra de título (`decorations.rs`) convertem a posição do pixel (`gl_FragCoord`) supondo a saída invertida do winit (`Transform::Flipped180`); no DRM a saída é `Normal`, e o recorte saía espelhado na vertical. O desfoque do vidro também desenhava fora da tela com a orientação do winit fixa.
- **Correção:** a orientação passou a ser do backend (`clip::set_flipped`, `clip::output_transform`): o winit inverte, o DRM não. Os shaders usam `output_height > 0` como "invertido"; o desenho fora da tela do vidro e a região de origem do desfoque seguem a mesma orientação.
- **Simulação do DRM no winit** (`MIDE_SIM_DRM=1`): a cena é desenhada sem inversão numa textura (como o buffer da tela no DRM) e copiada para a janela. Permite conferir e testar a orientação do DRM sem sair da sessão gráfica.

## Correções da primeira sessão de verdade em casa, backend sem tela e atualizar sem deslogar

- **Submenus fora da tela (Chrome "Histórico"):** o mide posicionava popups só pela geometria pedida, sem as regras do protocolo para caber na tela. Agora aplica o `constraint_adjustment` do posicionador (virar para o outro lado, deslizar, redimensionar) com a tela como limite, ao criar e ao reposicionar popups.
- **Vidro de cabeça para baixo no DRM:** o shader de composição do vidro desvirava a imagem sempre (certo só no winit). Agora segue a orientação da saída (`flip_y`).
- **Backend sem tela (`MIDE_BACKEND=headless`):** o mide desenha na GPU pelo render node (`/dev/dri/renderD128`), sem sessão, monitor nem janela, com a orientação do DRM. Os testes (`tests/scenario.sh`) usam ele por padrão: nenhuma janela de teste aparece na tela de quem usa o computador. O quadro salvo (`MIDE_DUMP_FRAME`) sai sempre na mesma ordem, em qualquer backend.
- **Mide de teste não age como sessão:** o `MIDE_SESSION` é lido e tirado do ambiente na partida (os apps abertos dentro da sessão não o herdam) e o `scenario.sh` o limpa. Antes, um mide de teste aberto de um terminal da sessão publicava o socket de teste no D-Bus/systemd do usuário e reabria os apps automáticos.
- **Atualizar sem deslogar (A16):** comandos `restart shell` e `restart all` no canal de controle, o programa `mide-ctl` (manda comandos ao mide) e o script `data/mide-update` (recompila em release e aplica; `--tudo` reinicia também o compositor). O compositor reinicia no mesmo processo (a sessão de login continua) e os apps automáticos que seguem vivos não são abertos de novo.
- Plano: registrados A15 (vários monitores e Configurações → Monitores), A16 e B13 (captura de tela).

## Grande rodada: monitores, captura de tela, menu global, animações e ajustes da dock

### Vários monitores (A15)
- **Compositor:** o backend DRM liga **todas as telas conectadas**, cada uma com o seu buffer, o seu vblank e o seu modo. Conectar e desconectar um cabo refaz as telas (udev). A cena é desenhada por tela, com a posição dela (`crate::outputs`): papel de parede por tela, janelas, popups e cursor relativos à tela, camadas na tela delas. O ponteiro passa de uma tela para a outra sem cair nos vãos.
- **Tela principal:** barra, dock, área de trabalho e launcher ficam nela; se ela muda (nas Configurações ou porque a tela saiu), eles são reabertos lá. Janelas que ficam fora de qualquer tela voltam para a principal.
- **Escala por tela** (100%, 125%, 150%, 175%, 200%): a cena é desenhada na resolução física com tudo multiplicado pela escala (recortes, cantos, vidro, sombras, títulos, prévias, cursor); os apps recebem a escala da tela (GTK, Qt, Chrome e VS Code desenham nítidos). A interface do próprio mide (mide-ui) ainda é ampliada (fica um pouco menos nítida em escalas fracionárias): desenhar o mide-ui na escala da tela fica como próximo passo.
- **`[monitors]` no config.toml:** `primary` e, por tela, `mode` (`1920x1080@60`), `scale`, `x`/`y` e `enabled`. O arranjo começa sempre em (0, 0).
- **Configurações → Monitores:** arranjo das telas em escala (arrastar; ao soltar, a tela encosta na vizinha, como no macOS; a principal tem a faixa da barra), tela ligada/desligada, "Usar como tela principal", Resolução, Frequência e Escala. "Aplicar" grava e o compositor muda na hora; "Manter estas configurações?" com contagem: sem confirmar em 15 s, volta ao que era.
- **Canal de controle:** `outputs` (lista para as Configurações) e `outputs apply`.
- **Backend sem tela com várias telas virtuais** (`MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080`), para testar tudo isso sem monitores.
- **mide-ui:** `ui.dropdown` (lista de escolha) e `ui.arrangement` (arranjo de telas arrastável), `ui.paint` (desenho livre).

### Captura de tela (B13)
- App **`mide-screenshot`** (tecla **Print**, ou "Captura de Tela" no launcher): a tela congela por cima de tudo. Modos **Seleção** (arrastar), **Janela** (clicar numa janela) e **Tela inteira**. Dentro da área, desenha **seta, círculo ou quadrado** em **vermelho, branco, preto, verde, amarelo ou azul**. **Copiar** (Enter ou Ctrl+C) põe a imagem na área de transferência; **Salvar** (Ctrl+S) grava em Imagens/Capturas de tela; Esc cancela.
- Compositor: `screenshot <arquivo>` (o próximo quadro da tela principal, sem o cursor), `copy-file <tipo> <arquivo>` (imagem na área de transferência, guardada pelo compositor) e `window_rects` (janelas com a moldura, para o modo Janela).

### Menu global dos apps na barra (B2)
- A barra é o **`com.canonical.AppMenu.Registrar`** e lê os menus pelo **dbusmenu**: quando o app em foco publicou a barra de menus, os títulos depois do nome do app são os dele (submenus com recuo, marcados com ✓, atalhos à direita) e o clique chega ao app. O app em foco é achado pelo processo que registrou (o nome dele bate com o `app_id` da janela). Chamadas aos apps desistem em 300 ms (a barra nunca trava).
- Quem publica: apps que exportam o menu global pelo D-Bus — VS Code, Chrome, Firefox e LibreOffice **rodando pelo XWayland** (no Wayland puro eles não publicam), apps Qt/GTK com o módulo appmenu. Sem menu publicado, ficam os menus do mide (Arquivo, Editar, Janela).

### Animações (B14), cada uma com interruptor em Efeitos
- **Animação dos menus** (`menu_animation`): menus da barra, da dock, dos apps (popups) e os de contexto do mide-ui entram esmaecendo e descendo 6 px em 120 ms; o vidro atrás deles esmaece junto.
- **Transições suaves** (`transitions`, antes "Transições no Arquivos"): itens da pasta que abre na lista aparecem em cascata; trocar entre ícones e lista esmaece o conteúdo novo.
- mide-ui: quadros no ritmo da tela (16 ms) enquanto algo anima, 40 ms parado.

### Dock (B16) e área de transferência
- **Clique no app em foco minimiza**; com várias janelas, mostra as prévias para escolher (como o "minimize or previews" do dash-to-dock). Interruptor em Configurações → Dock.
- **Downloads e Lixeira:** depois de excluir, restaurar ou apagar um item, a lista continua aberta e atualizada; fecha com clique fora ou **Esc** (o painel da dock recebe o teclado enquanto aberto).
- **"Copiar caminho" copia de verdade** (dock, Arquivos, área de trabalho, resultado da calculadora na busca): antes dependia do programa `wl-copy` (não instalado); agora o **compositor guarda o texto** na área de transferência (`copy <texto>`), e ele continua lá mesmo que o app que copiou feche.
- **Prévias com o Launchpad aberto:** escolher uma janela fecha o launcher.

### Sessão e testes
- Sessão do mide: `xdg-desktop-portal` configurado para o desktop "mide" (`data/xdg-desktop-portal/mide-portals.conf`) e reiniciado na entrada: os apps GTK4 recebem as configurações do sistema (botões da janela, tema, fontes, cursor).
- `tests/ui.sh` passou para **1366x768** (todas as coordenadas recalibradas), com testes novos de monitores, escala, captura de tela, menu global, dock e cópia entre apps; app de teste `examples/mide-menu-teste`.

## Correções depois do primeiro uso (monitores, reinício, captura)

- **Configurações → Monitores sem modo atual:** o compositor lia o modo da tela na superfície GBM, que vinha zerado (`0x0@0`): a Frequência ficava vazia e as telas não tinham tamanho no arranjo (não dava para arrastar). Agora o modo vem da saída Wayland. Modo que não pode ser aplicado deixa a tela no modo anterior.
- **`restart all` travava:** depois de publicar o ambiente, o mide esperava o systemd reiniciar o portal, e o portal, ao subir, esperava o mide. Agora o reinício do portal é pedido sem esperar (`--no-block`).
- **Captura aberta pelo launcher saía com o launcher na imagem:** espera o launcher fechar antes de capturar.
- **Captura mais leve:** a imagem escurecida é feita uma vez; cada quadro só copia bytes (antes misturava a tela inteira a cada movimento do mouse). Medido em 1920x1080: 14–19 ms por quadro. Mostra o tamanho da área escolhida e o painel não corta mais o botão Copiar.

## Monitores do tamanho de verdade, confirmação em janelinha, pasta do launcher com transição e lâmpada na tela certa

- **Arranjo em milímetros:** as telas aparecem do tamanho físico. Vale o que o monitor informa quando é plausível; valores que são só proporção (como `160x90 mm`) são ignorados e o tamanho é estimado pela resolução. Nova opção **"Tamanho da tela"** (Automático, 13,3" a 43") para corrigir monitores que não informam (`diagonal` no `[monitors.outputs."NOME"]`). As telas são desenhadas a partir de quem encosta em quem (densidades diferentes ficam coladas no desenho) e, ao soltar, o encaixe em milímetros vira a posição lógica (`physical_layout`, `snap_physical`, com testes).
- **"Manter estas configurações?" em janelinha (modal)**, por cima da janela inteira, como os diálogos do Arquivos.
- **Pasta do launcher abre e fecha com transição** (cresce do ícone até o lugar, esmaecendo; fecha voltando ao ícone), como o launcher abrindo. Preparada uma vez (grade escurecida e o painel sobre fundo transparente); cada quadro leva 3,5–4,7 ms em 1366x768.
- **"Transições suaves"** em Efeitos (`[effects] transitions`): agrupa a pasta do launcher e as transições do Arquivos (antes "Transições no Arquivos").
- **Lâmpada na tela certa com a tela principal à direita:** a posição do ícone na dock não somava a posição da tela da dock; minimizar e abrir pela dock saíam da outra tela. Corrigido também o alvo sem ícone e o `workarea` (agora relativo à tela principal; com a principal à direita, os ícones da área de trabalho sairiam da camada).

## Atalhos de janela (como no GNOME), Alt+Tab, setas do Arquivos, nomes sem diferenciar maiúsculas e prévias com transição

- **Atalhos novos (padrão, mudam nas Configurações):** Alt+Tab e Super+Tab (alternador de janelas: painel com as miniaturas no meio da tela, cada Tab avança, Shift volta, soltar o Alt escolhe, Esc cancela; o mouse também escolhe), Super+↑ maximizar, Super+↓ restaurar, Alt+F10 maximizar/restaurar, Super+H minimizar, Super+← e Super+→ metade esquerda/direita da tela (estado "tiled" para os apps), Alt+F4 fechar, Super+D mostrar a área de trabalho, Super+A launcher, Super+L bloquear a tela. Maximizar e as metades usam a tela onde a janela está (com várias telas).
- **Setas do Arquivos** (voltar/avançar) como os outros ícones da barra de ferramentas: ícones "symbolic" na cor do texto (desativadas, na cor secundária); acompanham o tema escuro quando ele existir.
- **Nomes sem diferenciar maiúsculas** (como no macOS): com "Nova Pasta" na pasta, a sugestão é "Nova pasta (1)", e criar "nova pasta" avisa que já existe "Nova Pasta".
- **Prévias da dock e o alternador entram esmaecendo** (sobem 8 px em 160 ms), no grupo "Transições suaves".
- **Cursor nas prévias:** em cima do painel das prévias o ponteiro é do mide (seta), e a janela de trás não recebe o movimento (antes aparecia o cursor de texto do terminal).

## Refinos do launcher, vidro depois da lâmpada e terminal no estilo Mac

- **Pasta do launcher:** durante a transição, os ícones da grade somem aos poucos no fundo escurecido, no mesmo ritmo do painel crescendo (antes a grade sumia de uma vez no fim). Ao fechar, o contrário. Cada quadro mistura duas imagens prontas (grade e fundo sem a grade).
- **Bolinhas das páginas paradas:** no carrossel (launcher e pastas), só o conteúdo desliza; a linha das bolinhas fica parada, já com a página de destino acesa.
- **Vidro da janela depois da lâmpada:** o vidro (desfoque atrás da barra lateral do Arquivos) entra esmaecendo em 220 ms quando a animação termina, em vez de aparecer de uma vez (`Genie::glass_since`).
- **Teclas no estilo Mac num terminal** (foot, kitty, GNOME Terminal, Konsole e outros): o Ctrl continua Ctrl (Ctrl+C interrompe, como no Terminal do Mac) e ⌘+C/⌘+V (a tecla Windows) copiam e colam (viram Ctrl+Shift+C/V). Antes, com o estilo Mac, copiar e colar no terminal exigia Windows+Shift+C/V.

## Lista de atalhos em grupos com busca, destaque padrão nas listas e Mouse e trackpad (B7)

- **Configurações → Teclado → atalhos:** em grupos (Janelas, Launcher e busca, Apps, Sistema), com **busca pelo nome** (sem diferenciar acentos; também acha pela combinação). Nomes legíveis para as ações novas ("Alternar janelas", "Janela na metade esquerda"...) e teclas com nomes melhores (Tab, ↑ ↓ ← →, Print Screen).
- **Destaque ao passar o mouse nas listas do mide-ui** (`list_item`): a mesma pílula cinza arredondada da barra lateral (antes, um retângulo colado nas linhas, fora do padrão).
- **Configurações → Mouse e trackpad** (`[input]` no config): velocidade do ponteiro, aceleração do mouse, rolagem natural no mouse e no trackpad (ligada no trackpad por padrão, como no Mac) e toque para clicar. O backend DRM aplica pelo libinput em cada aparelho conectado e reaplica na hora quando muda (`reload` → `input_waker`).
- `settings_file`: valores decimais (`Value::Float`).

## Identidade visual, pastas do launcher em vidro líquido e correções da bateria

- **Guia de identidade visual (`docs/design.md`):** cores, tipografia, formas, vidro, estados de interação, componentes do mide-ui, menus, diálogos, movimento (durações e curvas) e lista de revisão. Seção 9, **catálogo de efeitos**: superfícies (S1–S8), tipos de efeito (Surgir, Zoom de cena, Expandir da origem, Lâmpada, Interpolar geometria, Deslizar, Cascata, Esmaecer cruzado, Encolher e sumir, Mola, Imediato), ações (A1–A16), onde cada efeito acontece hoje com o interruptor, lacunas e regras para decidir. Regra para os apps novos e padrão do mide-ui.
- **Pastas do launcher em vidro líquido** (como no macOS): o ícone da pasta e o painel aberto pedem vidro ao compositor (desfoque, refração na borda e reflexo, o mesmo da dock), e o vidro acompanha as transições (some com a grade, cresce com o painel, desliza com as páginas, segue o zoom e o esmaecer do launcher). Interruptor "Vidro líquido" (desligado, o visual antigo).
- **Fundo do launcher mais leve:** desfoque com 3 passadas (antes 5) e véu mais claro; o papel de parede continua reconhecível atrás.
- **Vidro com opacidade por região** no canal de controle (`glass <camada> raio@opacidade:x,y,w,h`), e o envio do vidro pelos clientes numa thread própria (o desenho não espera o compositor; só o pedido mais recente vai).
- **Carrossel e pasta nova do launcher com interruptor** ("Transições suaves"); antes não tinham opção.
- **Colar num app recém-aberto:** a janela nova ganhava o foco antes de criar o canal da área de transferência, e a seleção oferecida se perdia (colar não funcionava até trocar de janela). Agora a seleção é oferecida de novo no primeiro quadro da janela em foco.
- **Controle deslizante do mide-ui:** no quadro em que o botão é solto, o valor também vem da posição do ponteiro (a velocidade do ponteiro em Mouse e trackpad não era gravada ao soltar).

## Terminal e Calculadora do mide (sem foot e sem gnome-calculator)

- **Terminal (`mide-terminal`, E1), com emulador próprio escrito do zero** (`src/terminal/`, sem crate de terminal; o iTerm2 serviu de pesquisa para a arquitetura, sem cópia de código): `parser` (máquina de estados do VT500: texto UTF-8, controles, ESC, CSI com subparâmetros, OSC; DCS/APC ignorados), `screen` (grade principal e alternativa, histórico de 10 mil linhas com números absolutos estáveis, **reflow** ao mudar a largura como no iTerm2, região de rolagem, tabulações, inserir/apagar caracteres e linhas, cores de 16, 256 e RGB, negrito claro, esmaecido, invertido, sublinhado simples e duplo, riscado, caracteres largos e acentos combinados, desenho de linhas do DEC, cursor em bloco/barra/sublinhado, respostas a DA, DSR, DECRQM, XTVERSION e tamanho da janela, título e pilha de títulos, cores pelo OSC 4/10/11/12/104, copiar pelo OSC 52, pasta atual pelo OSC 7, marcas de prompt pelo OSC 133, relatórios de mouse X10 e SGR), `keys` (teclas e colar → sequências do xterm), `pty` (pseudoterminal com `rustix`, shell em sessão nova, leitura e escrita em threads) e `width` (largura dos caracteres). A janela desenha as células com o mide-ui: seleção arrastando (duplo clique: palavra, atravessando quebras; triplo: linha), Ctrl+Shift+C/V copiar e colar (⌘C/⌘V no estilo Mac), Ctrl+Shift+K limpar, Ctrl+Shift+↑/↓ prompt anterior/seguinte, Shift+PgUp/PgDn/Home/End no histórico, roda do mouse no histórico (setas nos programas de tela cheia), mouse para vim/htop (Shift seleciona mesmo assim), barra de posição ao ler o histórico, menu de contexto, título do programa (ou `--title`), `-e`, `-D` e `--working-directory=`. Fecha quando o shell termina.
- **Calculadora (`mide-calculator`, E3):** teclas redondas sobre fundo escuro, operações na cor de destaque do mide; quatro operações, %, ±, `=` repetido (2 + 3 = = dá 8), AC/C, número com vírgula e ponto de milhar, notação científica quando não cabe, "Erro" na divisão por zero. Teclado: algarismos, `+ - * /`, vírgula ou ponto, Enter, Backspace, Esc; Ctrl+C copia o resultado e Ctrl+V cola um número ("1.234,5" ou "1234.5").
- **No lugar do foot e da gnome-calculator:** `ctrl+shift+t`, "Abrir no Terminal" do Arquivos e apps de terminal do launcher abrem o Terminal do mide; a dock padrão fixa Terminal, Calculadora e Configurações. `.desktop` novos em `data/applications`.
- **mide-ui:** `Input::presses` (cada tecla crua, com o keysym, o texto e Ctrl/Shift/Alt), `Input::alt`, `Ui::waker()` (acorda a janela de outra thread, por exemplo quando chega saída do programa), `Ui::paint_text` (desenho livre com o texto do mide-ui), `Ui::copy` e `Ui::request_paste`.
- **Autoteste:** `type` e `key` aceitam algarismos e `, / - =`.

## Terminal do mide: abas, painéis, busca e perfis (inspirado no iTerm2)

- **Abas:** barra de abas quando há mais de uma (título do programa em primeiro plano ou o pedido por ele, ponto azul quando uma aba em segundo plano recebe saída, × ao passar o mouse, + para nova aba). ⌘T/Ctrl+Shift+T (com o mide-terminal em foco, o atalho global de abrir terminal vira nova aba), ⌘1…9/Alt+1…9, Ctrl+PgUp/PgDn, Ctrl+Tab, ⌘⇧[ e ⌘⇧].
- **Painéis divididos:** ⌘D/Ctrl+Shift+D lado a lado, ⌘⇧D/Ctrl+Shift+E em cima e embaixo, divisor arrastável, ⌘⌥ setas para o painel vizinho, ⌘[ ⌘] em sequência, ⌘⇧Enter amplia o painel (e volta), painéis sem foco levemente apagados. Abas e painéis novos abrem na pasta do painel em foco (lida do `/proc`, sem depender do shell). Árvore de painéis em `src/terminal/layout.rs`.
- **Busca no histórico:** ⌘F/Ctrl+Shift+F abre a barra de busca; sem diferenciar maiúsculas nem acentos e atravessando as quebras automáticas; todas as ocorrências em amarelo e a atual em laranja, "3 de 12", Enter/⌘G próxima, Shift+Enter/⌘⇧G anterior, Esc ou "Concluído" fecha.
- **Negrito e itálico de verdade** na fonte de largura fixa (`Text::draw_mono_styled`).
- **Perfis e Preferências** (⌘, ou Ctrl+Shift+, ou o menu de contexto): janela própria (`mide-terminal --preferences`) com os perfis na barra lateral; tema de cores (Claro, Escuro, Grafite, Oceano, com amostra), tamanho da fonte, cursor (bloco, barra, sublinhado) e piscar, negrito em cor clara, aviso visual do sino, tamanho do histórico, pasta das abas novas, comando ao abrir; novo, duplicar, apagar e usar como padrão. Grava em `~/.config/mide/terminal.toml` e os terminais abertos aplicam em até um segundo. `--profile nome` abre com outro perfil.
- **Fonte:** ⌘+/⌘−/⌘0 (Ctrl+Shift+=/−/0).
- **Fechar com programas rodando** (janela, aba ou painel): pergunta antes, com o nome dos programas (como o iTerm2).
- **Sino visual:** o painel pisca quando o programa toca o sino.
- **Compositor:** com o mide-terminal em foco, o ⌘ chega ao app (ele trata os atalhos do Mac) e o atalho que abre um terminal vira nova aba.
- **mide-ui:** `Input::logo`/`KeyPress::logo` (tecla Super/⌘) e `Ui::custom` (desenho livre num espaço reservado no fluxo).

## Menus do app na barra para os apps do mide-ui, e teclas presas no winit

- **mide-ui publica os menus do app na barra superior** (`src/kit/menubar.rs`): o app descreve os menus em `App::menu_bar` (grupos, itens com ação, atalho, ativo e marcado, separadores e submenus) e o mide-ui publica pelo dbusmenu e registra no registrar da barra (de novo quando a barra reinicia). O clique chega ao app em `ui.input().menu`. O menu "Janela" (Minimizar, Zoom) vem pronto (`window_menu`) e o mide-ui trata sozinho.
- **Terminal com menus de verdade na barra:** Shell (Nova janela, Nova aba, Dividir, Fechar painel/janela), Editar (Copiar, Colar, Selecionar tudo, Buscar, ocorrências, Limpar), Visualizar (fonte, Ampliar painel, prompts, início/fim do histórico), Perfis (nova aba com cada perfil, Preferências…) e Janela (abas e painéis, lista das abas). O menu do botão direito usa as mesmas ações e os mesmos nomes. Atalhos aparecem no estilo do teclado em uso.
- **Atalhos nos menus da barra mais legíveis:** com ⌘ no estilo do macOS (⌘⇧D), e PgUp/PgDn, Enter e setas.
- **Teclas presas no mide em janela (winit):** quando a janela do mide perde o foco no desktop de baixo (Alt+Tab do Cinnamon), o mide solta todas as teclas que achava apertadas. Antes o Alt ficava "preso" e cada letra chegava ao terminal como Alt+letra: o bash tocava o sino (o painel piscava) e nada aparecia.

## Terminal: vidro, cores editáveis, mais temas, abas arrastáveis e sessão restaurada

- **Transparência e vidro por perfil:** opacidade do fundo (30–100%) e "Vidro" (o compositor desfoca o que está atrás, como os painéis do mide). Um vidro só cobre a área dos painéis (vários deixavam emendas). Com o vidro desligado nas Configurações, o fundo fica só translúcido. Novo no mide-ui: `Ui::glass_behind(rect, raio)`.
- **Cores editáveis:** nas Preferências, qualquer cor do perfil (texto, fundo, cursor, seleção e as 16 básicas) em `#rrggbb`, com amostra, por cima do tema escolhido; "Usar as cores do tema" desfaz. Gravadas em `[profiles.colors]`.
- **Mais temas prontos:** Papel, Floresta, Meia-noite e Alto contraste (8 no total).
- **Abas arrastáveis:** arrastar uma aba para os lados muda a ordem.
- **Reabrir abas e painéis** (Preferências → Sessão, vale para todos os perfis): ao fechar a janela, as abas, a divisão dos painéis e a pasta e o perfil de cada painel ficam em `~/.local/state/mide/terminal-session.toml`; o Terminal aberto sem argumentos volta assim (um shell novo em cada pasta). Módulo `src/terminal/session.rs`.
- **Acabamentos:** painéis sem foco apagados com um véu da cor do fundo (antes um cinza que clareava os temas escuros); barra de abas escura com temas escuros; o aviso visual do sino respeita o perfil.
- **Abrir endereços com Ctrl/⌘+clique** (como no iTerm2): com a tecla segurada, o endereço sob o ponteiro (`https://`, `http://`, `file://`, `ftp://`, `mailto:`, mesmo quebrado em duas linhas, sem a pontuação do fim da frase) fica sublinhado e o clique abre com o app padrão (`places::open`). No botão direito sobre um endereço: "Abrir endereço" e "Copiar endereço". Detecção em `Terminal::url_at`.
- **Autoteste:** `keydown <tecla>` e `keyup <tecla>` seguram e soltam uma tecla de verdade (o `ctrl on` só vale para o compositor).

## Terminal: integração do shell, gatilhos e imagens

- **Integração do shell sem instalar nada** (`src/terminal/integration.rs`, opção por perfil, ligada por padrão): o bash abre com `--rcfile` de um script do mide (que carrega `/etc/bash.bashrc` e `~/.bashrc` antes, como o bash faria) e o zsh com um `ZDOTDIR` do mide (que volta ao do usuário e registra ganchos `precmd`/`preexec`). O script avisa o começo de cada prompt, o começo do comando e o fim com o código de saída (OSC 133 A/C/D) e a pasta atual (OSC 7). Testado com um bash de verdade num PTY.
- **Marcas dos prompts** na margem esquerda: azul quando o comando deu certo, vermelha quando falhou, cinza no prompt atual. Ctrl+Shift+↑/↓ (⌘⇧↑/↓) pulam entre elas.
- **Aviso de comando demorado:** um comando que levou mais que o escolhido nas Preferências (padrão 10 s; 5 s, 30 s, 1 min ou nunca) e terminou com a janela sem foco, ou numa aba de trás, gera uma notificação do mide ("Comando terminado" ou "Comando falhou (código N)", com o comando e o tempo). Novo: `notify::send`, e o mide-ui sabe se a janela tem o foco (`Input::focused`; ao perder o foco, os modificadores voltam a soltos).
- **Gatilhos por perfil:** texto (sem diferenciar maiúsculas nem acentos) que, quando aparece na saída, é destacado em amarelo, vira notificação (até 3 por leitura) ou faz o painel piscar. Conferidos só nas linhas que já terminaram. Preferências → Gatilhos.
- **Imagens no terminal** com o protocolo do iTerm2 (`OSC 1337;File=…;inline=1`): PNG e JPEG, tamanho automático ou em células, `px` ou `%`, mantendo a proporção e sem passar da largura; a imagem rola com o texto, some com `clear` e quando sai do histórico (até 32 guardadas). A integração do shell traz um `imgcat` (quando o sistema não tem um).

## Terminal: janela de atalho, tmux e imagens Sixel e kitty

- **Terminal de atalho (F12, como o "hotkey window" do iTerm2 e o Guake):** `mide-terminal --hotkey` desce do topo, logo abaixo da barra, com a altura escolhida nas Preferências (padrão 45% da área útil), por cima das janelas e com o teclado só para ele; o mesmo atalho esconde e mostra de novo sem perder abas nem histórico. O segundo `--hotkey` só manda "alternar" pelo socket `$XDG_RUNTIME_DIR/mide/terminal-atalho-<display>.sock` e sai. Atalho padrão F12 (Configurações → Atalhos: "Mostrar ou esconder o terminal de atalho").
- **mide-ui:** `Window::dropdown` (camada de cima com o teclado só para ela, na área útil), `Ui::input_area` (só a parte desenhada recebe cliques; o resto passa para trás) e `Ui::set_hidden` (esconder e mostrar sem fechar o app).
- **Compositor:** uma camada que se esconde (commit sem imagem) volta a esperar a configuração inicial; antes, depois de escondida ela não voltava. Só o atalho do terminal "puro" (`mide-terminal`) vira nova aba com o terminal em foco (o F12 continua global).
- **Integração com o tmux (`tmux -CC`, como no iTerm2):** o terminal reconhece o modo de controle do tmux (DCS 1000p) e cada janela do tmux vira uma aba e cada painel, um painel; a saída vem pelo `%output`, o que se digita vai como `send-keys -H`, o tamanho da área é informado com `refresh-client -C`, e o layout do tmux (`%layout-change`, `list-windows`) vira a divisão dos painéis com as mesmas proporções. Nova aba, dividir e fechar num painel do tmux viram `new-window`, `split-window` e `kill-pane`; fechar a aba fecha a janela no tmux. O painel que roda o tmux mostra um aviso, não aceita digitação e desanexa com Esc (ou Shell → Desanexar do tmux). Protocolo em `src/terminal/tmux.rs`.
- **Imagens Sixel e do kitty** (`src/terminal/graphics.rs`): Sixel com paleta RGB e HLS, repetição e faixas (o terminal anuncia o Sixel na identificação, `CSI ? 62;4;22 c`); kitty com transmissão em partes, PNG, RGB e RGBA, arquivo (`t=f`/`t=t`), mostrar, guardar e mostrar de novo (`a=T/t/p`), apagar (`a=d`), consulta (`a=q`) e respostas (`q`). As três formas de imagem (iTerm2, Sixel, kitty) usam o mesmo posicionamento e rolam com o texto. O decodificador passou a entregar DCS em pedaços (`dcs_start`/`dcs_put`/`dcs_end`) e APC inteiro.

## Terminal: perfil padrão de verdade, fonte, placeholders do kitty (imagens no Neovim)

- **Painéis seguem o perfil padrão:** um painel aberto sem perfil escolhido segue o perfil marcado como padrão, mesmo quando o padrão muda ou é editado depois (antes ele ficava preso ao perfil com que nasceu: mudar opacidade no novo padrão não mudava o terminal aberto). Quem abriu com "Nova aba com …" ou `--profile` continua preso ao escolhido.
- **Fonte do terminal:** Preferências → Aparência → Fonte, com as famílias de largura fixa instaladas (ou a padrão do sistema), além do tamanho. Novo no texto do mide-ui: `Text::set_mono_family` e `Text::monospace_families`.
- **Amostra da cor com borda** (uma cor branca não sumia no cartão branco).
- **Placeholders Unicode do protocolo do kitty** (`U=1`): o programa manda a imagem uma vez e escreve U+10EEEE com acentos (linha, coluna, byte alto do número) em cada célula; o terminal desenha ali o pedaço certo da imagem, na proporção, e ela rola, divide e funciona dentro do tmux como texto. É como o Neovim mostra imagens (`snacks.nvim`, `image.nvim`). Tabela dos 297 acentos do protocolo em `graphics.rs`; os acentos têm largura zero.
- **Neovim com `snacks.nvim`:** ele só tenta imagens em terminais que conhece (kitty, ghostty, wezterm, pelo XTVERSION). Testado com `SNACKS_KITTY=1`: a imagem aparece. Na configuração do usuário: `if vim.env.TERM_PROGRAM == "mide" then vim.env.SNACKS_KITTY = "1" end` (o certo é o snacks conhecer o mide-terminal; ver anotações).
- **Letras em itálico e negrito cortadas** (com temas que pintam o fundo de toda célula, como no Neovim): o fundo de cada célula era pintado depois da letra anterior e apagava o pedaço dela que passa da célula ("nada" virava "naaa", "NORMAL" virava "NCRMAL"). Agora o painel pinta todos os fundos primeiro e todas as letras depois.

## Bloqueio de tela (C1)

- **Compositor (`src/lock.rs`):** atende o protocolo `ext-session-lock-v1`. Com a tela bloqueada, a cena desenha só a tela de bloqueio de cada saída (sem ela, um fundo escuro: nada de trás aparece nem por um quadro), o teclado vai só para a primeira tela de bloqueio do programa e o mouse só alcança a tela de bloqueio da saída onde está; nenhum atalho do mide funciona, só trocar de console (Ctrl+Alt+Fn). Se o programa de bloqueio fechar ou cair, a tela continua bloqueada e o mide abre outro.
- **Quem bloqueia:** Super+L (agora abre o `mide-lock`, antes chamava o `loginctl` sem ninguém atender), o logind (`loginctl lock-session` e antes de suspender, sinais `Lock` e `PrepareForSleep`) e o tempo sem uso (`[lock] idle_minutes`, padrão 5) — esses dois só na sessão de verdade (DRM); num mide em janela quem bloqueia é o desktop de baixo (`MIDE_LOCK_IDLE=1` liga mesmo assim).
- **App `mide-lock` (mide-ui):** papel de parede desfocado e escurecido, relógio e data, a inicial e o nome de quem usa, campo de senha (novo no mide-ui: `Ui::password_input`, com bolinhas e sem copiar); a senha é conferida pelo PAM numa thread (interface própria com a `libpam.so.0`, sem crate e sem o pacote de desenvolvimento), com o serviço `mide-lock` (arquivo em `data/pam/mide-lock`) ou o `login`; senha errada mostra "Senha incorreta" e o campo balança. Novo no mide-ui: `Window::lock` (tela de bloqueio em todas as saídas; a interface na primeira) e `Ui::unlock`.
- **Configurações → Bloqueio de tela:** bloquear após (nunca, 1, 2, 5, 10, 15, 30 minutos), bloquear ao suspender e "Bloquear agora".

## Bloqueio de tela: apagar a tela, ociosidade e todos os monitores

- **Apagar a tela (`[lock] blank_minutes`, padrão 10):** depois do tempo sem uso (ou 1 minuto com a tela bloqueada), o DRM desliga os monitores pela propriedade DPMS dos conectores e para de desenhar; qualquer tecla, clique ou movimento liga de novo e recomeça os quadros. No mide em janela e sem tela, "apagada" é preto. Configurações → Bloqueio de tela → "Apagar a tela após".
- **`ext-idle-notify-v1`:** programas (como o swayidle) sabem quando o usuário está ocioso.
- **`idle-inhibit`:** um programa que pede (vídeo tocando no navegador ou no mpv) impede bloquear e apagar enquanto a superfície dele existe.
- **Tela de bloqueio em todos os monitores:** o mide-ui desenha a mesma tela do app em cada monitor (sem entrada nos outros; as teclas vão para o primeiro).
- **Testes de tempo sem uso:** `MIDE_IDLE_UNIT=seconds` conta os minutos da configuração como segundos; `MIDE_LOCK_IDLE=1` liga o bloqueio por tempo fora da sessão de verdade.

## Otimizações do DRM (A17)

- **Só o que mudou é redesenhado:** a cena usa a idade do buffer (`next_buffer` do GBM; no backend de teste, 1 depois do primeiro quadro) e o rastreador de dano do Smithay. Para isso funcionar, os elementos do compositor mantêm a mesma identidade entre quadros: as decorações (`decorations::stable`: o n-ésimo elemento de cada programa no quadro de cada tela reaproveita o anterior e só conta como mudado se a área ou os uniformes mudarem), o vidro (identidade nova só quando o desfoque é refeito) e o papel de parede (identidade nova quando a textura ou as sombras mudam). O `TextureShaderElement` do Smithay cria um `Id` novo a cada criação; `clip::Keyed` guarda o `Id` escolhido. Parado, o dano é 0%; mexer uma janela redesenha só a área dela.
- **Áreas mudadas para o kernel (FB_DAMAGE_CLIPS):** nos monitores externos vão só os retângulos que mudaram; na tela do notebook (eDP/LVDS/DSI) vai a tela inteira, porque com PSR2 (Intel) ela atualizaria só uma faixa.
- **Cursor no plano de hardware:** a imagem do tema vai para um buffer do tamanho do plano do cursor (`drmGetCap` CURSOR_WIDTH/HEIGHT) com `set_cursor2` e `move_cursor`; cursores de app (superfície), imagens grandes demais ou um driver que recusa voltam ao cursor desenhado na cena. Mexer o mouse sem mudar a forma do cursor, sem arrastar e fora das barras de título (onde os botões acendem) só move o plano, sem quadro novo.
- **Para voltar atrás na sessão:** `MIDE_SW_CURSOR=1` (cursor desenhado na cena) e `MIDE_FULL_DAMAGE=1` (tela inteira a cada quadro).
- **Medido (`tests/perf.sh`, todos os cenários, 1366x768, backend de teste; mediana por quadro do compositor, `MIDE_FULL_DAMAGE=1` → atual):** desktop, dock, launcher e buscador: CPU 0,9 → 0,26 ms e GPU 2,2 → 0,32 ms; prévias: CPU 1,42 → 0,50 e GPU 3,62 → 0,56 ms; arquivos: CPU 1,21 → 0,43 e GPU 3,09 → 0,49 ms. Os máximos (~50–70 ms) são o primeiro quadro, iguais nos dois modos. Os quadros dos apps não mudam (são do cliente).

## Correções (2026-10-20, bugs vistos na sessão de verdade)

- **Clique não atravessa janelas:** a barra de título e as bordas de redimensionar só valem para a janela de cima no ponto (moldura ou área de entrada dela) e nunca sobre um popup. Antes, um clique num app com barra própria (Chrome) em cima da barra de título de uma janela de trás subia essa janela.
- **Janelas escondidas pelo app:** pelo xdg-shell, um buffer nulo esconde a janela sem fechar (o Chromium/Brave fazem isso). O mide tira a janela da tela e da lista, passa o teclado para a de cima e, quando o app refaz o começo, manda o configure inicial de novo e a janela volta onde estava. Antes, a janela escondida continuava "aberta" e, ao voltar, o app esperava o configure para sempre.
- **Launcher, páginas dentro da pasta:** no carrossel, o vidro do painel da pasta (que fica parado) ia recortado na faixa que desliza, e o compositor desenhava a borda e os cantos desse vidro nos limites dela: o painel aparecia com duas molduras durante a animação.
- **Estilo Mac refeito (pedido do usuário):** o Ctrl continua Ctrl; o **Alt (esquerdo) é o ⌘** e o **Super é o ⌥**; o Alt da direita continua AltGr (acentos e símbolos do ABNT2). Nos apps, o ⌘ chega como Ctrl (⌘C, ⌘V, ⌘L) e o ⌥ como Alt; no terminal, o ⌘ chega como Super (⌘C copia, Ctrl+C interrompe). Os atalhos do mide veem o ⌘ como Super (⌘Tab troca de janela) e também valem os com Ctrl pelo ⌘ (⌘Espaço). ⌘A, ⌘D, ⌘L e ⌘ com setas ficam com o app. **Bloquear no estilo Mac: Ctrl+⌘+Q** (atalho padrão novo `ctrl+super+q`); Ctrl+L e ⌘L não bloqueiam. O soltar de cada tecla usa a mesma troca do apertar (nada fica preso se o foco muda entre app e terminal). Antes, o estilo Mac trocava Super e Ctrl: o Ctrl+L físico bloqueava a tela e Ctrl+S/Ctrl+C da captura de tela não funcionavam.
- **Tela de bloqueio rápida:** o fundo (papel de parede desfocado e escurecido) é do compositor, na GPU, com o desfoque guardado por tela; o mide-lock desenha só relógio, avatar e campo, e os outros monitores são redesenhados uma vez por segundo (relógio). Resposta por tecla em 1920x1080: de 35–47 ms para 7–16 ms.
- **Senha certa que não desbloqueava (dois monitores):** o kit desenha a tela de bloqueio dos outros monitores com o mesmo `view`, e era ele que às vezes lia a resposta do PAM; o pedido de desbloqueio feito ali era descartado. Agora as cópias (`Ui::mirror`) não leem a resposta, e um desbloqueio pedido em qualquer uma vale.
- **Nada da sessão atrás da tela de bloqueio:** o `space::render_output` do Smithay acrescentava as camadas da tela (barra, dock, área de trabalho) por baixo dos elementos do mide; na tela bloqueada elas apareciam (antes escondidas pelo fundo opaco do mide-lock, mas visíveis nos primeiros quadros depois de bloquear). O mide desenha pelo rastreador de dano direto, só com os próprios elementos (no uso normal, as cópias ficavam atrás do papel de parede: trabalho a menos). O alfa passado era a escala da tela.
- **Tela que não acendia depois de apagar:** depois do DPMS ligar de novo, o estado do CRTC é reavaliado (`reset_state`) e o próximo quadro refaz o modo.
- **Captura de tela:** avisa por notificação quando salva (com o caminho), copia ou falha; grava em Imagens/Capturas de tela como `Captura de tela AAAA-MM-DD HH-MM-SS.png` (a pasta é criada).

## Tema de ícones do mide (B15, etapa 1)

- **Fork do WhiteSur:** `github.com/fabioaacarneiro/mide-icons` (o original, de Vince Liuice, GPL-3, fica só como fonte: nada é enviado a ele). As mudanças do mide ficam em `mide/` no fork: `build_icons.py` gera os ícones próprios (SVG à mão, com a sombra e o quadrado do próprio WhiteSur, para o mesmo acabamento), `apply.py` aplica as trocas (reaplicável depois de trazer versões novas do original) e o `README.md` lista o que mudou.
- **Sem desenhos da Apple:** o logo da Apple (`start-here`, todos os tamanhos) e a maçã do iMac viram a marca do mide (anel com ponto); o rosto do Finder vira a pasta do tema sobre um quadrado claro; a bússola do Safari, um globo; o "A" da App Store, uma sacola; a engrenagem das Preferências do Sistema, três controles deslizantes; o Terminal, uma tela com ❯ e cursor em bloco; o TextEdit, uma folha com linhas; a Captura de Tela, uma seleção tracejada com a mira; o ícone padrão de app, quatro blocos; o "A" das pastas de modelos, um documento. Os atalhos do tema seguem (Nautilus, Nemo, Dolphin, gnome-control-center, Konsole, gedit...).
- **Apps do mide com nomes próprios:** `mide-terminal`, `mide-text`, `mide-files`, `mide-settings`, `mide-screenshot`, `mide-calculator`, `mide-about` nos `.desktop`; sem o tema, caem nos nomes genéricos (`icons::generic_name`).
- **Busca:** com o tema `mide` instalado, ele vem antes do `hicolor` (os apps de terceiros ganham a versão no estilo do tema, como no GTK); depois o `hicolor` e os temas do usuário.
- **Instalação:** o `mide-update` baixa o fork na versão de `data/icons.version`, instala `mide`, `mide-light` e `mide-dark` em `~/.local/share/icons` (sem `-a` e sem `-p`) e copia os ícones dos apps do mide para o `hicolor` (aparecem em menus de outros ambientes). Só reinstala quando a versão muda; sem rede, avisa e segue.

## Tema de ícones do mide (B15, etapas 2 e 3)

- **Etapa 2, revisão completa:** as 1.446 imagens de apps do WhiteSur foram conferidas uma a uma. Ganharam desenho próprio, no mesmo acabamento, todas as recriações de apps da Apple encontradas: Calendário, Relógio, Contatos, Música (disco de vinil), Fotos (fotos em leque), Monitor de Atividade, Mapas, Lembretes, Ajuda, Dicionário, Podcasts, Acessibilidade, Captura de Imagem/Photo Booth, Utilitário de Sistema, Mail, Pré-Visualização, Utilitário de Disco, Time Machine, QuickTime, Magic Mouse, Mensagens, Gravador de Voz, Catálogo de Fontes e Tempo; as variantes da App Store, do Safari e das Preferências também. A maçã em todos os desenhos de iMac (`computer`, `video-television`, `variety`, `putty`, `xwaylandvideobridge`) vira a marca do mide, no mesmo lugar e tamanho. A lista completa fica no `mide/README.md` do fork.
- **Etapa 3, na sessão:** o `mide-session` cria um perfil do dconf da sessão (`~/.config/mide/dconf/profile`: o banco do usuário, onde tudo continua sendo gravado, e por cima um banco só de leitura que trava só `org.gnome.desktop.interface icon-theme` em `mide`). Assim os apps GTK da sessão do mide usam o tema, e o Cinnamon (ou outra sessão) não muda. `QT_QPA_PLATFORMTHEME=gtk3` faz os apps Qt 5 seguirem o GTK. O mide publica `DCONF_PROFILE` e `QT_QPA_PLATFORMTHEME` para os serviços do D-Bus e reinicia também o portal GTK.

## Correções (2026-10-21, sessão de verdade num Dell Micro 3090)

- **A tela não voltava depois de apagar:** com a tela apagada (DPMS), o desenho do DRM saía cedo sem atender o pedido de quadro (`state.redraw` ficava ligado); o `wake` só avisa quando não há pedido, e sem vblank (monitor desligado) ninguém mais desenhava: mexer no mouse ou no teclado não acendia a tela. Agora o desenho atende o pedido mesmo apagado, e acender (`activity`) acorda o desenho sempre.
- **Depois de suspender:** o aviso de "acordou" do logind (`PrepareForSleep(false)`) refaz as telas (sem quadro pendente, estado do CRTC reavaliado, buffers e cursor de novo), como depois de acender: uma troca de quadro em andamento na hora de suspender podia deixar a tela parada.
- **Botão de ligar:** na sessão, o mide segura o botão no logind (`handle-power-key`); sem isso, fora do GNOME/Cinnamon o logind desligava o computador direto e o trabalho aberto se perdia. Apertar o botão abre a confirmação "Desligar o computador agora?" da barra; com a tela bloqueada, só acende a tela.

## Gravar a tela pelos apps (A19) e atalhos de captura do Mac

- **`wlr-screencopy` v3 no compositor** (`src/screencopy.rs`): tela inteira e região, `copy` (no próximo quadro) e `copy_with_damage` (quando a tela muda; o primeiro pedido de cada cliente sai logo, para a gravação não começar preta). A imagem sai do quadro que o mide acabou de desenhar (sem desenho extra), convertida para XRGB8888 no buffer de memória compartilhada do cliente. Durante uma gravação que pede o cursor, o DRM desenha o cursor na cena (o plano de hardware não entra na imagem).
- **Portal:** `mide-portals.conf` manda `ScreenCast` e `Screenshot` ao `xdg-desktop-portal-wlr` (OBS, navegador em videochamada); o `mide-update` escreve `~/.config/xdg-desktop-portal-wlr/mide` com a janela de escolha do mide; o `data/mide-check` avisa se o pacote falta.
- **`mide-screencast-picker`:** janela "Compartilhar a tela" com os monitores (o principal já escolhido); o nome do escolhido sai na saída padrão para o portal; Cancelar recusa.
- **⌘⇧4 e ⌘⇧3, como no Mac:** ⌘⇧4 escolhe uma área e, ao soltar o mouse, salva em Imagens/Capturas de tela e copia (sem painel); ⌘⇧3 salva e copia a tela inteira sem abrir nada. Funcionam com o launcher aberto, e a imagem sai com ele. Fora do estilo Mac o ⌘ é a tecla Super; no estilo Mac, a Alt. O app Captura de Tela (Print) continua com o painel e as marcações.

## Foco e escala pedidos pelos apps (A14/A15)

- **`xdg-activation`:** um app pede para trazer uma janela para a frente (link aberto de outro app, notificação clicada, o xwayland-satellite passando o foco de uma janela X11). Pedido recente (até 10 s do token): a janela sobe, volta se estava minimizada e ganha o teclado (`src/activation.rs`).
- **`wp_fractional_scale`:** cada janela, camada e tela de bloqueio recebe a escala exata da tela onde está (1,25, 1,5...), atualizada a cada quadro só quando muda; apps GTK 4, Qt 6, Firefox e Chromium desenham nítidos em vez de esticados. (O `wp_viewporter`, que eles usam junto, já existia.)

## Transparência de janelas inteiras (A11)

- `[opacity]` no config: `inactive` (janelas sem foco, de 1 = opacas até 0,2) e a opacidade por `app_id` (`"mide-terminal" = 0.95`; multiplica a de sem foco). A janela inteira fica translúcida: conteúdo, barra de título, título, botões e contorno; a sombra fica mais leve na mesma proporção.
- O foco considerado é o do teclado (o estado "ativo" do xdg só muda quando o app confirma, e os apps do kit só confirmam quando redesenham).
- Configurações → Efeitos → Transparência: "Janelas sem foco" (0 a 60%).

## Layer-shell completo (A10)

- **Popups de camadas** (o menu de um ícone da Waybar e de outros painéis): pelo protocolo, o popup nasce sem pai e a camada o adota depois (`get_popup`). O mide espera a adoção, ajusta a posição à tela da camada, registra o popup e manda a configuração; os popups são desenhados na frente da camada, recebem o ponteiro e fecham ao clicar fora (junto com os das janelas).
- Exemplo de teste `examples/mide-layer-popup-teste.rs`: uma camada no topo (azul) com um popup laranja adotado por ela.

## Tray: ícones de apps na barra (B3)

- `src/tray.rs`: a barra é o registro `org.kde.StatusNotifierWatcher` (no barramento da sessão; com outro painel já atendendo, fica sem tray). Cada app que registra um ícone (`StatusNotifierItem`, pelo nome no barramento ou só pelo objeto, como o Ayatana) tem as propriedades lidas numa thread: título, estado (os "passivos" não aparecem), ícone pelo nome (pasta do app, depois o tema) ou a imagem mandada (ARGB32), e o menu. Sinais `NewIcon`/`NewStatus`/... releem o ícone; o app que sai do barramento perde os ícones.
- Na barra: os ícones ficam à esquerda dos do sistema (como no macOS); clique esquerdo chama `Activate` (os apps que só têm menu abrem o menu); clique direito abre o menu do app (`dbusmenu`, o mesmo leitor do menu global), com o título dele no topo; o item clicado chega ao app.
- Exemplo de teste `examples/mide-tray-teste.rs` (ícone laranja, menu "Abrir"/"Sair", tempo de vida `TRAY_TESTE_SEGUNDOS`).

## Menu global dos apps GTK e Qt no Wayland (B2)

- **GTK (`gtk_shell1`)** (`src/app_menus.rs`): o mide anuncia o `gtk_shell1` com a capacidade de menu global; o app GTK diz onde está a barra de menus dele no D-Bus (`set_dbus_properties`) e deixa de desenhar a barra dentro da janela. A barra do mide pergunta ao compositor (`appmenu`) e lê os menus pelo `org.gtk.Menus` (`src/gmenu.rs`: seções viram separadores, submenus, atalhos `<Primary>s` → `Ctrl+S`), com o estado das ações pelo `org.gtk.Actions` (desligadas em cinza, opções marcadas com ✓). O item clicado manda `Activate` ao app (`app.` no objeto do app, `win.` no da janela). O menu é relido ao abrir.
- **Qt (`org_kde_kwin_appmenu`)**: o app diz o serviço e o objeto do `dbusmenu`; a barra usa o mesmo leitor do menu global do X11 (depende do tema de plataforma do Qt publicar o menu).
- O registrar (`com.canonical.AppMenu.Registrar`) continua tendo preferência; o caminho pelo compositor entra quando o app não registrou nada nele.
- App de teste `tests/apps/gtk-menu-teste.py` (GTK 3: Arquivo/Exibir/Ajuda, uma seção e uma opção marcável).

## Tema escuro (D1, B7, B17)

- **Configurações → Aparência:** Claro ou Escuro (`[appearance] theme = "dark"` no `config.toml`).
- **mide-ui:** `Theme::dark()` e `Theme::system()`; as janelas seguem o tema do sistema e trocam na hora (verificado uma vez por segundo, o arquivo só é relido quando muda). Os componentes não têm mais cores fixas: trilhos, bordas de campos, listras, barras de rolagem, menus e ações destrutivas vêm do tema (`ink`/`shade`, `menu`, `danger`, `sidebar_glass`). A tela de bloqueio fica sempre clara (sobre o papel de parede); apps com tema próprio (`Window::theme`) não mudam.
- **Compositor:** barras de título grafite com título claro no tema escuro.
- **Barra superior:** barra escura com texto branco; menus, painéis (calendário, som, Wi-Fi, Bluetooth, central de controle, confirmações) com a paleta escura. Notificações e "Sobre este computador" também.
- **Apps GTK e Qt (só na sessão do mide):** o mide escreve `color-scheme` e `gtk-theme` (`Adwaita-dark`) no banco do dconf da sessão (`src/appearance.rs`; o mesmo perfil do tema de ícones, agora criado sempre que o dconf existe) e reinicia o portal do GTK; valem para os apps abertos depois da troca. Outras sessões não mudam.

## mide-ui: teclas mortas, listas longas e redesenho parcial (D1)

- Teclas mortas e composição (´ + a = á, ~ + o = õ) conferidas nos apps do mide-ui, também com o idioma "C" (sessão iniciada do console).
- Listas e grades longas: além de ladrilhos e linhas de arquivos, as linhas de lista (`list_item`) e os interruptores (`switch_detail`) fora da área à vista só ocupam o lugar e recebem o clique, sem desenhar.
- Redesenho parcial: cada quadro manda ao compositor só o retângulo que mudou desde o anterior (`damage_buffer` da área diferente); digitar num campo não reenvia a janela inteira.

## Diálogos e seletor de arquivos do mide-ui (D1) e "Abrir…" no Editor de Texto (E2)

- `ui.alert(título, texto, botões)`: aviso pronto por cima da janela; devolve o botão clicado (o último é o principal; Esc = primeiro, Enter = último). Painéis de diálogo seguem o tema (escuros no tema escuro).
- `ui.scroll_area(altura, ...)`: área de altura fixa que rola (a `page` agora usa a mesma base), com a barra de rolagem no tom do tema.
- `FileChooser` (`src/kit/chooser.rs`): Abrir, Salvar e Escolher pasta. Lugares (pasta pessoal, Área de trabalho, Documentos, Downloads, Imagens), pasta de cima, caminho com `~` (cortado pelo começo quando é longo), lista com pastas primeiro e tamanhos, duplo clique entra na pasta ou escolhe, Enter confirma, Esc cancela, filtro por extensões, ocultos escondidos; no modo Salvar, campo do nome com foco, clicar num arquivo usa o nome dele, nome com "/" é recusado e um arquivo que já existe pede "Substituir?".
- Editor de Texto: botão "Abrir…", Ctrl+O e menus do app na barra (Arquivo: Nova janela, Abrir…, Salvar, Salvar como…; Editar: Buscar e substituir…). Abrir num editor sem título e vazio carrega nele; com um documento aberto, abre outra janela. "Salvar como…" usa o seletor (começa na pasta do arquivo, ou em Documentos).

## mide-ui nítido em telas com escala (D1, A15)

- Os apps do mide-ui desenham na resolução real da tela: a janela pede a escala exata ao compositor (`wp_fractional_scale`, 1,25 = 150/120), desenha a imagem em pixels (tamanho em pontos × escala) e diz o tamanho em pontos pelo `wp_viewporter`. Sem a escala fracionária, usa a escala inteira da tela (`set_buffer_scale`).
- O layout continua em pontos: `mide::ui::draw` tem a escala da thread (`scale`, `tf`, `px`) e as funções de desenho (formas, sombras, vidro, colar ícones e áreas) e o texto convertem pontos em pixels; os ícones são carregados no tamanho em pixels; miniaturas do app são ampliadas (`blit_image`). Cliques e o vidro continuam em pontos.
- O Terminal e a Captura de Tela pintam pixels por conta própria e ficam em escala 1 por enquanto (`Window::fixed_scale`); a tela de bloqueio também (as outras telas copiam a imagem dela).
- Compositor: os shaders das decorações (botões da barra de título, contornos, cantos, vidro das prévias, sombras) recebiam o raio em pixels com o tamanho em pontos; com escala 2 os botões viravam losangos. Agora tudo em pontos.

## Configurações → Teclado: layout (B7)

- Lista "Layout" (o do sistema, ABNT2, EUA, EUA internacional com acentos, EUA AltGr, Reino Unido, Portugal, Espanha, América Latina, França, Alemanha, Itália), gravada em `[keyboard] layout`/`variant`; o compositor troca o mapa de teclas na hora (`reload`), sem reiniciar, e usa o mesmo ao iniciar. Vazio = `/etc/default/keyboard`.
- Listas suspensas do mide-ui ficam da largura do nome mais longo (antes cortavam em 220 px).

## Configurações: Idioma e região, Energia e Apps padrão (B7)

- **Idioma e região:** "Idioma" e "Formatos (data, hora, números e moeda)" entre os idiomas instalados que a lista sabe nomear (`locale -a`), gravados em `[locale] lang`/`formats`; o `mide-session` exporta `LANG`/`LANGUAGE` e `LC_TIME`, `LC_NUMERIC`, `LC_MONETARY`, `LC_PAPER`, `LC_MEASUREMENT` no próximo login, e a sessão passa os mesmos aos serviços do D-Bus.
- **Energia:** "Suspender o computador após" (nunca, 15/30 min, 1/2/3 h; `[power] suspend_minutes`), contado como o bloqueio (não suspende enquanto um app pede a tela acesa) e uma vez por período sem uso; "Botão de ligar" (perguntar, suspender, desligar, nada; `[power] button`).
- **Suspender, reiniciar, desligar e bloquear pelo logind no D-Bus** (`org.freedesktop.login1`): funciona com o systemd e com o elogind (Slackware, Void, Gentoo sem systemd); sem resposta, cai no `systemctl`/`loginctl`.
- **Apps padrão:** Navegador, E-mail, Pastas, Textos, Imagens, Vídeos, Músicas e PDF; a lista vem dos apps que declaram o tipo (`MimeType`) e a escolha vai para `~/.config/mimeapps.list` (`[Default Applications]`, mantendo o resto do arquivo), o mesmo do `gio open`, `xdg-open`, GNOME e Cinnamon (`src/default_apps.rs`). O mide respeita a escolha: com outro app padrão para pastas ou textos, eles não abrem mais no Arquivos/Editor do mide.
- **Barra lateral do mide-ui rola** com a roda do mouse quando os itens não cabem, e abre mostrando o item escolhido.

## Arquivos: seleção múltipla, arrastar e soltar e "Abrir com…" (B8)

- **Seleção múltipla:** Ctrl+clique junta ou tira, Shift+clique escolhe do item principal até o clicado (na ordem da tela, na grade e na lista), Ctrl+A escolhe tudo. Delete, "Mover para a Lixeira", Copiar/Ctrl+C + Colar, Duplicar e Compactar valem para todos (vários itens viram "Arquivo.zip", como no Finder); o menu mostra "N itens" e esconde o que só faz sentido para um (Renomear, Copiar caminho, Abrir no Terminal). Botão direito num item de fora da seleção escolhe só ele.
- **Arrastar e soltar:** apertar num item e andar mais de 6 pontos arrasta a seleção (ou só ele); o nome (ou "N itens") segue o ponteiro numa etiqueta azul e a pasta de destino ganha contorno; soltar move para a pasta (com Ctrl, copia), também entre discos (`files::move_into`: troca de nome, ou copia e apaga). Mensagem "N itens movido(s) para …".
- **"Abrir com…":** no menu de um arquivo, abre a lista dos apps que abrem o tipo dele (tipo pelo shared-mime-info, `default_apps::mime_for`; o padrão primeiro, marcado "padrão"); clicar abre o arquivo no app (`apps::open_with`). Os apps são carregados numa thread ao abrir o Arquivos.
- **mide-ui:** `ui.pointer()` e `ui.over(rect)` públicos (achar onde soltar algo arrastado).

## Movimento num lugar só e o azul único (B17)

- `src/motion.rs` (`mide::motion`): todas as durações da tabela do `docs/design.md` (menus 120 ms, prévias 160, fechar 180, dock 200/450, lista do Arquivos 180+25, troca de visão 200, vidro 220, mover janela 220, pasta do launcher 240, launcher 260/170, página 320, pasta nova 340, papel de parede 450, lâmpada 650) e as curvas (`ease_out` cúbica, `ease_out_soft`, `smooth`, `ease_in_out`, `ease_out_back`, `progress`). O compositor (`state.rs`, `genie.rs`, `wallpaper.rs`, `preview.rs`), a barra, a dock, o launcher, o Arquivos e os menus do mide-ui usam daqui.
- Azul de destaque único: a barra superior (menus, painéis, calendário, controles, confirmações) e os menus da dock usam a cor de destaque do tema do mide-ui (`[10, 110, 235]`; no escuro, `[30, 125, 245]`), em vez de três azuis diferentes; os menus da dock também seguem o tema escuro.

## Tema de ícones: tipos de arquivo e símbolos (B15, etapa 4)

- Revisados os 298 tipos de arquivo e 565 símbolos monocromáticos do fork com folhas de contato (`examples/mide-folha-de-icones`: desenha uma grade de SVG/PNG com o carregador do mide, que abre os SVGs do WhiteSur que o ImageMagick não abre).
- No fork (`mide/apply.py`, etapas 5 e 6; commit `11f8484`): a caixa marrom com fita do instalador de pacotes virou caixa verde-azulada lisa (pacotes, executáveis, componentes do sistema e o compactador); Apple Music, Xcode, rosto do Finder, bússola do Safari, "A" da App Store e das pastas de modelos e o pássaro do Swift deram lugar a desenhos do próprio tema (ou à sacola com seta). Ícones de apps de terceiros que usam caixas como marca ficaram.
- `data/icons.version` aponta para a versão nova (o `mide-update` instala).
- `data/mide-check`: avisa quando falta o plugin do GTK para Qt 5 ou Qt 6 (`qt5-gtk-platformtheme`, `qt6-gtk-platformtheme`); recomendados na instalação.

## Espelhar telas (A15)

- `[monitors] mirror = true` (Configurações → Monitores → "Espelhar telas", no fim da página, vale na hora): todas as telas ficam em (0, 0); as que não são a principal ganham a escala que deixa a largura lógica igual à da principal (uma tela de 1920 espelhando uma de 1366 fica em 1,41, nítida), e ao desligar voltam à escala das Configurações.
- As telas espelhadas desenham a barra, a dock, a área de trabalho e as prévias da principal (`State::layer_output`); o ponteiro e os cliques nas camadas usam a principal (`output_at`).

## Vários layouts de teclado e o seletor na barra (B18), grupos de atalhos que recolhem (B19)

- Configurações → Teclado: a lista dos layouts escolhidos (sem nenhum, aparece o do sistema), "Remover" em cada um (com mais de um) e "Adicionar layout" (o primeiro acréscimo mantém o do sistema na frente). Grava `[keyboard] layouts = "br,us:intl"` (`mide::keyboard_layouts`; o formato antigo de um layout só continua valendo).
- Compositor: o mapa de teclas tem um grupo por layout; a ação `next_layout` (Ctrl+Super+Espaço; no estilo Mac, Ctrl+⌥+Espaço, como no macOS) passa ao próximo; o canal de controle tem `layout` (o ativo) e `layout N` (trocar).
- Barra superior: com mais de um layout, a sigla do atual ("BR", "US", "US-I") num contorno, à esquerda do relógio; o clique abre a lista com o ativo marcado e "Configurações de teclado…". Interruptor "Mostrar o layout na barra superior" em Teclado (`[keyboard] show_layout`).
- Os grupos da lista de atalhos (Janelas, Launcher e busca, Apps, Sistema) recolhem e expandem pelo título, com a setinha e o número de atalhos (`ui.disclosure`, componente novo do mide-ui); o estado fica em `[settings] shortcuts_closed`; buscando, os grupos com resultado ficam abertos.
- Corrigido: "Configurações de bateria…" abria o Wi-Fi (agora abre Energia).

## Animações que faltavam (A20)

- **Fechar janela suave em todos os casos:** o compositor guarda uma foto recente de cada janela (tirada no quadro depois de um commit do app, no máximo a cada 300 ms quando ele muda sem parar). Quando o app fecha sozinho (atalho do próprio app, botão desenhado por ele, processo que termina), essa foto encolhe e esmaece, como já acontecia quando o mide pedia o fechamento.
- **Ícone pulando na dock:** ao abrir um app pela dock (com as animações ligadas e o app ainda sem janela), o ícone pula acima da dock, como no Mac, numa camada própria e transparente (`mide-dock-pulo`, mesma largura e margem da dock, sem receber cliques), até a janela aparecer (termina num pouso; no máximo 10 s). O lugar dele na dock fica vazio enquanto pula.
- **Arquivos, lista:** abrir e fechar pastas virou uma sanfona: as linhas de dentro entram (ou saem) uma a uma, com um espaço que cresce, e as de baixo deslizam junto; ao fechar, a pasta só fecha de verdade no fim. A duração acompanha o número de itens (180 a 400 ms, com a curva de entrada do `mide::motion`).

## Gravar a tela em vídeo com áudio (A21)

- **⌘⇧5** (`super+shift+5`; no estilo Mac, ⌘ é o Alt): a Captura de Tela abre no modo gravar, com um painel embaixo: **Tela inteira** ou **Área** (arrastar para escolher), **Som do sistema** e **Microfone** (lembrados em `[record]`), Cancelar e **Gravar** (ou Enter). Com uma gravação em andamento, ⌘⇧5 para.
- **Compositor** (`src/record.rs`, comandos `record start [full|x y w h] [sistema] [microfone]`, `record stop`, `record status`): depois de desenhar a tela, copia a área gravada (em pixels, com lados pares) e manda ao `ffmpeg` pela entrada, com a hora de chegada de cada quadro; com a tela parada, uma linha de execução repete o último quadro a cada 100 ms (o vídeo acompanha o tempo e o som). O cursor entra na imagem. Os primeiros 250 ms ficam de fora (o painel ainda sumindo). Fila curta: se o `ffmpeg` atrasar, quadros são pulados, sem travar o desenho.
- **ffmpeg** (dependência obrigatória): H.264 (`libx264`, `veryfast`, CRF 23, 30 quadros por segundo) e AAC; som do sistema pelo monitor da saída padrão (`@DEFAULT_MONITOR@`) e microfone pela entrada padrão (`@DEFAULT_SOURCE@`), misturados quando os dois; arquivo `Vídeos/Gravações de tela/Gravação de tela AAAA-MM-DD HH-MM-SS.mp4`. Sem o ffmpeg, o aviso diz como instalar.
- **Barra superior:** enquanto grava, o botão de parar (quadrado num círculo) com o tempo (00:12); o clique para. No fim, notificação "Gravação salva" com o caminho.
- `places::videos_dir` (pasta de vídeos do usuário, `XDG_VIDEOS_DIR`).

## Sons do sistema (A22)

- **Sons do mide, feitos do zero** (`src/sounds.rs`): sintetizados por código (senos com harmônico leve e queda exponencial, ruído filtrado com envelope), sem arquivo de som de terceiros: notificação (duas notas subindo), obturador (dois estalos), Lixeira (papel amassado), volume (toque curto), bateria fraca (notas descendo), pronto (três notas subindo), entrada na sessão (acorde aberto), aparelho conectado/desconectado e o alerta escolhido (Vidro, Gota, Sino ou Pop).
- **Tema de sons do freedesktop `mide`:** na primeira vez, vira `~/.local/share/sounds/mide` (WAV estéreo de 48 kHz, nomes da especificação: `message-new-instant`, `screen-capture`, `trash-empty`, `bell`, `dialog-warning`...), refeito quando os sons ou o alerta mudam. Na sessão do mide, o banco do dconf põe `org.gnome.desktop.sound theme-name='mide'` e `event-sounds` (apps GTK com libcanberra tocam os sons do mide).
- **Onde tocam:** notificação chegando (não no Não perturbe, nem se o app pede silêncio ou manda o próprio som); captura de tela (uma vez por captura); esvaziar a Lixeira (Arquivos e dock); ao soltar o volume na barra; sino do terminal com o sino visual desligado; bateria passando de 10%, descarregando (com um aviso); gravação de tela salva; entrada na sessão.
- **Tocar:** `pw-play` (PipeWire), ou `paplay`, no volume dos efeitos; nos testes (`MIDE_SYSTEM_DRY_RUN`), só "som (teste): nome" no log.
- **Configurações → Som → Efeitos sonoros:** "Tocar efeitos sonoros" (`[sounds] enabled`), "Volume dos efeitos" (`volume`, 70 por padrão; toca ao soltar) e "Som de alerta" (`alert`; toca a prévia ao escolher).

## Calculadora completa, etapa 1 (E3b)

- **Motor de contas próprio** (`src/calculator/`, sem crates de terceiros, com testes sem tela): o app só desenha as teclas e chama `mide::calculator`.
  - **Básico e Científico** (`engine.rs`): precedência (2 + 3 × 4 = 14) e parênteses (uma pilha por nível; "=" fecha os que ficaram abertos), multiplicação implícita antes do "(" (2 (3 + 4) = 14), "=" repetido (também depois de fechar parênteses: 2 × (3 + 4 = = repete × 7), troca de operação refazendo a precedência (2 − 3 × trocado por + dá 2 − 3 + 4), ± logo depois de uma operação começa um número negativo (5 × ± 3 = −15), % (100 + 10% = 110), C e AC.
  - **Precisão:** o número da tela é "digitado" (texto) ou "valor" (o `f64` inteiro); as contas seguintes não partem do número cortado em 12 algarismos (1 ÷ 3 = × 3 = dá 1). Somas e subtrações que deveriam dar zero e sobram 1 ou 2 algarismos de arredondamento viram zero (0,1 + 0,2 − 0,3 = 0).
  - **Funções do Científico:** x², x³, xʸ, eˣ, 10ˣ, 2ˣ, 1/x, √, ∛, ʸ√x (raiz ímpar de negativo dá o valor real), ln, log₁₀, log₂, logᵧ, x! (inteiros multiplicando; os outros pela função gama, até 170,5!), sen/cos/tg e as inversas, hiperbólicas e inversas, π, e, EE, Rand, Rad/Grau e a tecla 2nd. Em graus, o ângulo é reduzido a uma volta e os múltiplos de 90° saem exatos (sen 1800° = 0, tg 90° = Erro); em radianos, o resto do arredondamento de π vira 0 (sen π = 0) sem zerar ângulos minúsculos.
  - **Memória** (mc, m+, m−, mr), a mesma no RPN; m+ e m− fecham o número da tela.
  - **Histórico:** cada conta com a expressão ("2 + 3 × 4", "√(9)", "(3 + 4)") e o resultado; também as do RPN ("9 + 1"); até 500 contas.
  - **Programador** (`programmer.rs`): inteiros de 64 bits com sinal (complemento de dois) em HEX, DEC, OCT e BIN; AND, OR, XOR, NOR, X<<Y, X>>Y, <<, >> (aritmético, como ÷ e mod com sinal: −8 >> 1 = −4), RoL, RoR, byte⇄, ~ e ±; precedência como em C; troca de operação; "=" repetido; ± durante a digitação (−5 seguido de 3 é −53); i64::MIN ÷ −1 dá a volta (só ÷ 0 é erro); bits clicáveis; o caractere Unicode do número ("A", "ê", "😀"); colar na base da tela ("FF" em HEX, "0x1F", "0b101").
  - **RPN** (`rpn.rs`), como as calculadoras RPN clássicas: pilha X/Y/Z/T, Enter duplica X e desliga a subida da pilha; constantes, mr e números colados sobem a pilha; x↔y e R↓ religam a subida; ± durante a digitação troca o sinal (5 ± 3 = −53); EE faz parte da digitação (2 EE 3 = 2.000; ± depois do EE troca o sinal do expoente); % é Y × X ÷ 100; ⌫ fora da digitação apaga X; C apaga X e um segundo C seguido apaga a pilha ("AC"); uma conta com erro não mexe na pilha e a tecla seguinte só apaga o aviso.
  - **Números na tela** (`format.rs`): vírgula decimal, ponto de milhar, no máximo 12 algarismos (o arredondamento nunca passa disso), notação científica com os algarismos que cabem (2⁵⁰ = 1,125899907e15) e de 0,000001 para baixo; casas decimais fixas (empates arredondam para longe do zero, como numa calculadora); colar entende "1.234,5", "1.234", "1.234.567", "1234.5", "1,234.5" e "−3".
- **App** (`mide-calculator`): Visualizar → Básica, Científica e Programador (Ctrl+1/2/3; a janela muda de tamanho e já abre no tamanho do modo lembrado), "Notação polonesa reversa (RPN)", "Mostrar o histórico" (Ctrl+T, painel à direita que rola com a roda do mouse; clicar numa conta traz o valor exato), "Separador de milhar" e "Casas decimais" (Automático, 0–4, 6, 8, 10, 15); Editar → Desfazer (Ctrl+Z) e Refazer (Ctrl+Shift+Z) das contas, Copiar (sem os pontos de milhar, para colar em qualquer lugar), Colar, Copiar o histórico e Limpar o histórico. O número da tela passa de um modo para o outro (também ao ligar e desligar o RPN). Teclado e teclado numérico na ordem em que as teclas foram apertadas; no RPN, Tab e ( são x↔y e ) é R↓; no Programador, A–F, & | ^ ~ % < >. Opções em `[calculator]` (`mode`, `rpn`, `history`, `radians`, `decimals`, `thousands`).
- **Completude da etapa 1** (itens apontados pelo crítico da revisão): histórico também no Programador ("0xFF AND 0xF = 0xF", clicar traz o inteiro exato de 64 bits); 2nd troca eˣ por yˣ (3 yˣ 2 = 8); RPN com R↑ e LASTx (no Científico, 2nd nas teclas da pilha; no Básico, uma fileira a mais com x↔y, R↓, R↑ e LASTx, e a janela cresce essa altura); EE dentro do número digitado também fora do RPN (2 EE 3 mostra "2e3"; ± depois do EE troca o sinal do expoente; o expoente tem até 3 algarismos); a conta em andamento em letra pequena acima do número ("2 × (3 +"; depois do "=", a conta feita); ⌫ na tela enquanto um número é digitado (mouse e toque); base do Programador lembrada (`[calculator] base`); clique direito numa conta do histórico copia a conta ou só o resultado; atalhos F9 (±), Ctrl+R (RPN) e, no Científico, letras para as funções (s, c, t, n, l, r, q, p, e, !).
- **mide-ui:** `Ui::request_size(tamanho, mínimo)` (o app pede outro tamanho de janela; o quadro já sai no tamanho novo). O Delete do teclado numérico (NumLock desligado) vale como Delete em todos os apps. O tamanho do compositor vale sempre com a janela maximizada, em tela cheia ou lado a lado; solta, um `configure` repetido com o mesmo tamanho (o mide reenvia o estado a cada troca de foco) não desfaz o tamanho pedido pelo app, e um pedido feito com a janela presa vale quando ela fica solta.
- **Testes:** o roteiro do autoteste digita "+", "*" e "/" pelo teclado numérico (iguais em qualquer layout; no ABNT2 a tecla do "/" americano dá ";").
- Fica para a etapa 2: conversões de unidades e de moeda e o Math Notes (contas digitadas em texto).

## Calculadora, etapa 2: conversor de unidades (E3b, partes 1 a 3 do plano da etapa 2)

- **Motor de unidades** (`src/calculator/units.rs`, sem tela e sem crates): 15 categorias e 140 unidades — Comprimento, Área, Volume, Massa, Temperatura, Tempo, Velocidade, Energia, Potência, Pressão, Força, Ângulo, Dados, Transferência de dados e Consumo de combustível —, com as de costume brasileiro (alqueires paulista e mineiro, arroba de 15 kg, xícara de 240 mL, colheres, cv, BTU/h, TR, mca, kgf/cm², mmHg, psi). Os fatores saem de poucas definições oficiais (polegada 0,0254 m e libra 0,45359237 kg de 1959, gₙ, galão de 231 pol³, galão imperial, caloria termoquímica de 4,184 J e a internacional de 4,1868 J no BTU, au, parsec, ano-luz juliano); nenhum número final é digitado, e um teste confere 22 fatores contra a tabela B.8 do NIST. Temperatura afim; L/100 km recíproco (0 não tem conversão); ano de 365 dias.
- Também no motor: a busca do seletor (sem acento nem maiúscula; símbolo igual, começo do nome, o resto; "peso" acha a Massa), o reconhecimento de unidades num texto para o futuro Math Notes (`lookup` e `longest_at`: m e M, b e B, min e mi; "km/h em m/s" acha km/h; o "a" do are só depois de um número) e os pares lembrados por categoria (`Pairs`: "temperatura=f>c;comprimento=pol>cm", do mais recente ao mais antigo).
- **Conversor na Calculadora** (`src/bin/mide-calculator/`, agora uma pasta, com `converter.rs`): Visualizar → "Conversor de unidades" (Ctrl+U), na Básica e na Científica; a janela cresce 64 pt (Básica 250×444) e as teclas descem. O visor vira duas linhas — a ativa (o número da tela, branca, até 40 pt) e a outra (a conversão a cada tecla, cinza) —, com o nome da unidade e ⌄ à esquerda (o símbolo, se o nome não cabe), o ⇅ e a categoria entre elas.
  - Usar a conversão: clique na outra linha, ↑/↓ ou Converter → "Usar a outra linha". Vale como uma tecla de função: no meio da conta vira o operando ("2 + (12,5 pol → cm) = 33,75" no histórico); no RPN troca o X (o LASTx guarda o anterior) e entra no histórico na hora.
  - ⇅ ou Ctrl+Shift+U trocam as unidades de linha (o número fica: "100 °F = 37,7777777778 °C" vira "100 °C = 212 °F").
  - Escolher a unidade: clique no nome, Ctrl+L ou Converter → "Escolher a unidade…" abrem o seletor; mesma categoria muda só a linha (a unidade da outra linha troca as duas); outra categoria leva o par lembrado dela (ou o padrão).
  - Menu Converter (só com o conversor à vista): Recentes (até 5 pares) e Categoria; Editar → "Copiar a conversão" ("12,5 pol = 31,75 cm"); clique direito numa linha: copiar o número, a conversão, escolher a unidade, trocar.
  - Desfazer e refazer cobrem as trocas de unidade e o uso da outra linha. No Programador o conversor some e volta igual. Opções em `[calculator]` (`convert`, `convert_pairs`).
  - O log registra "calculadora: conversão 100 °F = 37,7777777778 °C" (a bateria confere por ele).
- **mide-ui:** seletor de lista com busca (`ListPicker`, `src/kit/picker.rs`): grupos (as categorias, em ordem alfabética), "‹ Todos os grupos", busca sem acento, ↑/↓ pulando itens desativados, Enter, Esc e "Cancelar"; a lista volta ao topo quando a busca muda e rola até o item marcado. Na busca, o Enter escolhe o melhor resultado, mesmo que a unidade atual também apareça ("metro" acha os Centímetros, mas escolhe os Metros); o clique que abriu o seletor não escolhe nada nele, e depois de trocar de grupo os itens esperam o intervalo do duplo clique. Componentes novos: `ui.choice_item` (item com a pílula azul do marcado) e `ui.scroll_area_showing` (rola até uma faixa). `ui.alert` cabe em janelas estreitas (os botões encolhem ou ficam um embaixo do outro).

## Calculadora, etapa 2: Notas matemáticas (E3b, partes 7 a 10 do plano da etapa 2)

- **Motor das notas** (`src/calculator/notes/`, sem tela e sem crates): texto livre em que a linha que termina em "=" mostra o resultado depois dele. Léxico próprio (números em português e em inglês, com ponto de milhar, vírgula decimal, expoente e fração colada; palavras com acento; "47x96" e "1h30min" separados; símbolos de moeda), parser Pratt com a precedência da calculadora e multiplicação implícita, e avaliador com as mesmas contas do teclado (`Binary::apply`, `Unary::apply`, o fatorial). Variáveis ("aluguel = 1.200", sem diferença de maiúscula e acento), funções em português e em inglês (sen, cos, raiz, ln, arred, mín, máx, mod...), porcentagem ("100 + 10% =" dá 110), graus e radianos como no teclado, prosa antes da conta ("Hoje gastei 2 × 3 =" dá 6), "x" como vezes entre números e "e" como conjunção quando é prosa. Erros com o trecho a sublinhar e mensagem em português ("“y” não é uma variável nem uma unidade", "“x” é definida mais abaixo (linha 2): defina antes de usar", "Divisão por zero"...). Resultado só no "=" do fim da linha; uma resposta já escrita ("2 + 2 = 4") não mostra nada. Bancada: 500 linhas em ~1 ms, 5.000 em ~9 ms.
- Revisão do motor (dois revisores e um corretor): 13 achados corrigidos, cada um com teste de regressão (nome desconhecido antes de + ou −, variável definida mais abaixo, função sem fechar, "e" depois de prosa, linha comprida O(n²), pilha funda...). O fatorial do teclado também passou a dar infinito (tela "Erro") acima de 171,7, em vez de NaN.
- **Gravação** (`src/calculator/notes/store.rs`): uma nota por arquivo de texto em `~/.local/share/mide/calculadora/notas` ("20261022-153012.txt"), só com o que foi digitado (os resultados são recalculados ao abrir). A gravação segura saiu do Editor de Texto para `mide::files::write_atomic` e vale para os dois; com a revisão, ela segue links simbólicos (um `~/.bashrc` que aponta para um repositório de configurações continua link e o arquivo de verdade recebe o texto), o temporário tem nome único, nasce com as permissões do original e sem seguir nem truncar nada que já exista, e é apagado se a gravação falha.
- **mide-ui:** `ui.text_area_with(doc, altura, TextAreaLook { .. })`: sem números de linha, fonte proporcional, outro tamanho, dica com o documento vazio (cortada com "…") e decorações (`Decorations`: trechos coloridos, sublinhado pontilhado dos erros e texto depois do fim de uma linha), pedidas a cada quadro já com a tecla do quadro. Com o cursor no fim de uma linha que tem texto depois do fim, a área rola para ele ficar à vista; nas outras linhas, o que não cabe termina com "…". Tabulações em trechos coloridos ficam no lugar certo. A resposta traz a posição do texto sob o ponteiro no quadro de um clique. `TextDoc::can_undo`/`can_redo` para o menu Editar. O `text_area` do Editor de Texto é o mesmo componente com a aparência e as teclas de antes.
- **Modo Notas na Calculadora** (Ctrl+4 e Visualizar → "Notas matemáticas", janela 720×480): lista das notas à esquerda (Ctrl+T, lembrada), barra com o título, "Nova nota" e "Mover para a Lixeira", a nota em letra de 15 pt e a linha de estado com o erro da linha do cursor (em vermelho) e Graus/Radianos. O resultado aparece em azul depois do "=", as variáveis num tom entre o azul e o texto, os erros com pontilhado vermelho. Segue o tema claro ou escuro, as casas decimais e o separador de milhar do Visualizar.
  - Grava 1 s depois da última tecla numa thread à parte (o disco lento não trava a janela), e na hora ao trocar de nota ou de modo e ao fechar; a nota editada sobe na lista e o título é a primeira linha; uma nota nova vazia não vira arquivo; a última nota aberta fica lembrada (`nota` em `[calculator]`).
  - Menus: Arquivo (Nova nota, Exportar com os resultados…, Mover a nota para a Lixeira) e Editar (Desfazer e Refazer da nota, Cortar, Copiar, Colar, Selecionar tudo, Copiar o resultado da linha, Copiar a nota com os resultados); no Visualizar, RPN e conversor ficam desativados e "Mostrar o histórico" vira "Mostrar a lista de notas". Novo em todos os modos: Visualizar → "Ângulos em radianos".
  - Clique direito num resultado: copiar o resultado, copiar só o número ou "Levar para a calculadora" (volta ao último modo de teclado com o número na tela).
  - Mover para a Lixeira abre a nota de baixo (ou uma nova); a nota vai para a Lixeira do sistema (dá para recuperar pelo Arquivos).
  - O log registra o que muda em cada linha ("calculadora: notas: linha 1 = 360", "...linha 4: erro: ..."), as gravações, as cópias e a Lixeira (a bateria confere por ele).
- Revisão da área de texto e do modo Notas (quatro revisores, um verificador por achado e um crítico de completude): 24 achados corrigidos, entre eles o custo de medir a posição do ponteiro a cada movimento do mouse (agora só no clique), o resultado cortado na borda, as tabulações, o Enter do "Exportar" que entrava na nota, o foco perdido ao mostrar a lista e o `fsync` dentro do quadro.
- Conferência das correções (três verificadores, um por frente, e um crítico): mais 19 ajustes. Nas notas: fechar a janela com uma nota que não gravou mostra um aviso (Cancelar, Fechar sem gravar, Tentar de novo) e o Exportar que falha também avisa; duas Calculadoras na mesma nota não se apagam (o texto daqui vira uma nota nova se o arquivo mudou fora); uma nota que não é UTF-8 abre só para ler e nunca é gravada; a Lixeira leva o que foi digitado por último; os botões da barra e a lista devolvem o foco à nota; ir do Programador às Notas e voltar não muda o número. No mide-ui: a posição do clique numa linha longa passou a ser achada por busca binária (um JSON minificado não trava mais o Editor de Texto ao clicar), o título encolhe com "…" dentro de `sized`, a roda não rola a lista atrás de um diálogo e o Enter confirma o "Substituir?" do seletor de arquivos. Na gravação segura: só segue links que o kernel também seguiria (um link plantado por outro usuário em `/tmp` dá erro de permissão), o grupo e o setuid do original continuam, e nomes longos não fazem o temporário passar do limite.
- Fica para a parte 11: unidades e moedas dentro das notas ("10 km em milhas =", "R$ 1.200 + 300 =").

## Janela restaurada que ignorava cliques e teclas (A24, bug do VS Code)

- Causa: o mide guarda as janelas minimizadas numa lista e não manda input para as que estão nela (as que estão saindo, na animação). O "minimizar tudo" (Super+D) e o botão de minimizar não conferiam se a janela já estava na lista: uma janela ainda na animação de saída, ou um Super+D repetido, entrava duas vezes. Ao restaurar saía uma entrada e a outra ficava: a janela voltava à tela, mas cliques e teclas eram ignorados (parecia congelada). Maximizar pela barra de título, que é do mide, destravava.
- Correção (`src/wm.rs`, `src/input.rs`): uma entrada só por janela, a restauração limpa as que sobraram e o Super+D deixa como estão as janelas que já estão saindo.

## Calculadora, etapa 2: moeda no conversor (E3b, partes 4 e 5 do plano da etapa 2)

- Decisões do usuário (2026-10-22): cotações do Banco Central do Brasil; busca pelo `curl` como programa externo; pergunta na primeira vez; falha de rede avisada ("Houve um problema de rede e não foi possível buscar as cotações agora").
- **Motor das moedas** (`src/calculator/currency.rs`, sem tela e sem crates): 149 moedas que o BC publica, mais o real, em português (nome, plural, símbolo, casas decimais da ISO 4217, apelidos); busca e reconhecimento ("dólares", "US$", "$", "reais", "R$", "₲"); leitura do arquivo "Cotação de todas as moedas" do fechamento PTAX escrita à mão, pela taxa de venda: tipo A = dólar ÷ paridade, tipo B = dólar × paridade, conferida contra a taxa publicada (0,5%); recusa página de erro em HTML, linhas incompletas, datas misturadas, valores não positivos ou infinitos e arquivo sem o dólar; deixa de fora o ouro (vem por grama), o DES e as unidades de conta (CLF, COU, CNH, MRO, SLL).
- **Cotações** (`src/calculator/rates.rs`): data esperada do boletim (dia útil depois das 13h30 de Brasília, em UTC−3 fixo; senão o dia útil anterior); cache em `~/.cache/mide/cotacoes` (`bcb.csv` e `estado.toml`); quando buscar (no máximo uma vez por boletim; sem boletim, de novo em 1 h; sem conexão, em 15 min; feriados lembrados sem pedir de novo); o `curl` com os argumentos em vetor, sem shell, só https, ignorando o `~/.curlrc`, com tempo e tamanho máximos; os códigos de saída viram estados (sem boletim, sem conexão, rede pedindo login, resposta estranha, curl ausente, disco); volta até 5 dias úteis em 404; o arquivo baixado vai para um temporário próprio da busca e só troca o guardado depois de validado. Revisado (8 achados corrigidos, entre eles erro 5xx do BC tratado como feriado e quedas no meio da transferência).
- **Moeda no conversor da Calculadora:** categoria Moeda, a primeira no seletor e no menu Categoria (par inicial USD → BRL, lembrado como "moeda=USD>BRL"); conversão pela cotação, com as casas da moeda quando "Casas decimais" está em Automático (real 2, iene 0). A busca só acontece quando a pessoa usa moeda, numa thread que acorda a janela (a Calculadora não trava); na primeira vez, a pergunta "Buscar cotações?" (Não buscar / Buscar), lembrada em `currency_online`. A faixa do conversor mostra "BCB · 21/10/2026" ou o estado ("Buscando cotações…", "Sem conexão · cotação de 20/10", "Cotação antiga…", "Sem cotações · Tentar de novo", "Instale o curl para buscar cotações"); um clique nela (ou Converter → "Cotações de moedas…") abre a janela com a taxa do par, o interruptor "Buscar na internet", "Atualizar" e o aviso de que a taxa é de referência (bancos cobram outra taxa e o IOF). Uma busca que falha mostra o aviso "Cotações não atualizadas" com a mensagem e a cotação que vale.
- Nos testes nada usa a internet: o `scenario.sh` marca `MIDE_NO_NETWORK=1`, e sem o `curl` falso da bateria a Calculadora não busca.
- `curl` entra no `INSTALATION.md` e no `data/mide-check` (aviso, não obrigatório).
- Ficam para depois: a reserva no Banco Central Europeu (parte 6, opcional) e as moedas e unidades dentro das notas (parte 11).

## Pastas coloridas nas listas da dock (B24)

- Causa: no tema do mide (do WhiteSur), os ícones de 16, 22 e 24 px das pastas (e de outros lugares) são desenhos só de contorno, na cor do texto; a lista de Downloads e da Lixeira pede 24 px e recebia o contorno, que parecia um símbolo ou um tema mal acabado.
- Correção (`src/icons.rs`): para nomes que não são `-symbolic`, a busca no tema usa o desenho grande (48 px ou mais, o colorido) e reduz para o tamanho pedido. Vale em todo lugar que mostra arquivos (dock, Arquivos, busca, área de trabalho, Abrir com…); os ícones `-symbolic` das barras laterais continuam de contorno, como devem.

## Teclas multimídia (A23)

- O compositor trata as teclas XF86 de qualquer teclado (o receptor USB de um teclado sem fio aparece como mais um aparelho do libinput, e as teclas chegam iguais): volume +5% e −5% (`wpctl`, no máximo 100%, com o som de confirmação do volume), mudo, microfone mudo, brilho ±5% (`brightnessctl`, sem chegar a apagar) e tocar/pausar, pausar, parar, próxima e anterior pelo MPRIS no D-Bus (no player que está tocando, ou no primeiro aberto). Volume e brilho valem também com a tela bloqueada; a mídia, não. As teclas não chegam mais ao app em foco.
- O ícone de som da barra acompanha na consulta seguinte. O aviso flutuante (OSD) veio depois, na seção "Aviso de volume e brilho (A23, OSD)".

## Calculadora, etapa 2: unidades e moedas nas notas (E3b, parte 11)

- O motor das notas ganhou quantidades com unidade e dinheiro (`src/calculator/notes/`, com o reconhecimento em `words.rs`): "10 km em milhas =" dá 6,21371192237 mi; "5 km + 300 m =" dá 5,3 km (a primeira unidade vence e um número solto entra nela: "R$ 1.200 + 300 =" dá R$ 1.500,00); "1 h 30 min em min =" e "1h30min em min =" dão 90 min; "1/2 kg em g =" dá 500 g; "25 °C em °F =" dá 77 °F e "20 °C + 5 °C =" dá 25 °C; × e ÷ por número mantêm a unidade; "10 km / 2 km =" dá 5; variáveis guardam quantidades ("d = 5 km", depois "d em m =" dá 5.000 m); "°" sozinho é o grau de ângulo. Contas que criariam outra grandeza (km ÷ h, m × m) dão "Conta entre unidades diferentes: ainda não"; somar grandezas diferentes dá erro explicado.
- Moedas: prefixos R$, US$, $, €, £, ¥ (e CA$, AU$, CN¥...), nomes, plurais e códigos depois de número ("100 usd em brl ="); a conversão usa as mesmas cotações do conversor; sem cotação, o erro diz qual falta e a data da última. Uma nota que converte moedas pede as cotações ao Banco Central (com a pergunta da primeira vez) e é recalculada quando elas chegam; a linha de estado mostra "Cotações: BCB · 21/10/2026".
- Unidade ou moeda pelo nome só vale depois de um número ou como alvo de "em/para/→": "a", "m", "min", "real" da prosa continuam prosa ("De 2 a 5 × 3 =" dá 15).
- Revisado (11 achados corrigidos, entre eles unidade depois de parênteses e variáveis, símbolos com letras, desempenho e diferença de temperaturas). Bancada: 500 linhas em ~1 ms, 5.000 em ~11 ms.

## Calculadora completa: fechamento da E3b (parte 12)

- Conferência visual em 1366×768, em escala 1,25 e nos temas claro e escuro: teclado, conversor, seletor de unidades, Moeda (faixa, pergunta, aviso de rede, janela de cotações na Básica de 250 pt) e Notas. Achado e corrigido na conferência: buscar "peso" no seletor punha todas as unidades de massa à frente das moedas (o apelido "peso" da Massa contava como palavra exata de cada unidade); agora os apelidos da categoria só entram na busca, e "peso" acha primeiro o Peso argentino.
- A E3b fica feita. Ficam para depois (registrados no plano da etapa 2): a reserva das cotações no Banco Central Europeu (opcional), o resultado no meio da frase e inserir o resultado como texto nas notas, contas que criam outra grandeza (km ÷ h), "Janela → Nova janela" e a quebra automática de linha nas notas, e o buscador do mide usando o avaliador das notas.

## Captura de tela por tela e em escala fracionária (B22, passos 1 e 2)

- Compositor: o pedido de captura virou uma lista por tela (`screenshot_requests`), e cada tela salva o próprio quadro (sem o cursor) quando desenha. O comando `screenshot` ganhou duas formas, além da de antes (`screenshot <arquivo>`, a tela principal): `screenshot tela NOME <arquivo>` (uma tela; nome errado responde "erro: tela não encontrada") e `screenshot all <prefixo>` (todas, em `<prefixo>-NOME.png`, com uma linha por tela na resposta: nome, posição e tamanho em pontos no desktop, escala, `principal|-` e o caminho). Espelhando as telas, só a principal. A leitura do comando (`parse_screenshot`) e a linha de cada tela (`screen_line`) são funções puras, com testes.
- Captura (`mide-screenshot`), ainda numa camada só (na tela principal): pede `screenshot all` e guarda cada tela (`Shot`) com a imagem em pixels (para o arquivo) e uma cópia no tamanho em pontos (para mostrar e escurecer). A seleção, as marcações e as janelas do modo Janela ficam em pontos no espaço do desktop. O arquivo sai de `compose`: cada tela que a área toca entra na maior escala entre elas, o que não é de nenhuma tela sai preto, e as marcações engrossam junto com a escala. O painel fica centrado pelo tamanho da tela em pontos.
- Corrige o bug em escala fracionária: numa tela em 1,25 a tela não escurecia (o escurecido, em pixels, não cabia na camada, em pontos) e o recorte saía do lugar errado (a área em pontos era recortada da imagem em pixels). Agora a área (300, 200)–(700, 450) numa tela de 1920x1080 em 1,25 sai com 500x313 pixels, do lugar certo.
- Compositor antigo (atualizado sem `restart all`): ele responde `ok` ao `screenshot all` (e só registra no log que não conseguiu gravar); a Captura volta ao `screenshot <arquivo>` com a posição e a escala da principal pelo comando `outputs`.
- Nitidez: o que se vê na camada numa tela de escala fracionária é a imagem reduzida e ampliada de novo pelo compositor (um pouco suave); o arquivo sai nos pixels da tela.
- Ainda não (próximos passos da B22): camadas nas outras telas (a seleção só acontece na principal), área ajustável com bolinhas, "Tela inteira" perguntando qual tela, ⌘⇧3 com várias telas e gravação de área em outra tela.

## mide-ui em todas as telas (B22, passos 3 e 4)

- Geometria das telas no kit (`src/kit/screens.rs`, sem Wayland, com testes): `Screen` (nome, origem no desktop, tamanho em pontos, escala e se é a principal) com `rect`, `contains`, `to_local` e `to_global`; `screen_at` (tela sob um ponto global, `None` num buraco), `union` (retângulo de todas), `dedupe_mirror` (telas espelhadas viram uma, fica a principal; 2 pontos de folga, porque a cópia ajustada fica com 769 pontos de altura), `primary_first`, `to_main` (ponto de uma superfície nas coordenadas da principal) e `parse_outputs` (lê o comando `outputs`).
- `Window::on_every_output()`, junto com `Window::overlay`: além da camada principal (criada como antes, que o compositor põe na tela principal, com o teclado), o app ganha uma camada em cada outra tela (`<namespace>@<TELA>`, sem teclado, para duas camadas não disputarem o foco). A tela principal vem da linha `principal` do comando `outputs` do canal de controle, casada pelo nome com a saída do Wayland (sem resposta, a primeira). Monitores que entram e saem ganham e perdem a camada com o app aberto; a camada de outra tela fechada pelo compositor não fecha o app (só a principal fecha).
- O mesmo `view` desenha cada tela: `ui.screen()` diz qual é, `ui.mirror()` é verdadeiro nos passes das outras telas e a lógica do app fica no passe da principal. O ponteiro de qualquer camada chega ao app nas coordenadas da principal (pode cair fora dela) e em coordenadas globais por `ui.pointer_global()`; `ui.screens()` lista todas, a principal primeiro. Os passes das outras telas guardam e devolvem o estado da principal (entrada, botão apertado, vidro, área de clique, fechar, título, tamanho, área de transferência), são desenhados antes da saída por "quadro igual" (a principal pode não mudar quando o ponteiro anda na outra tela) e só mandam o quadro de uma tela quando ele muda, com o dano só da parte mudada.
- As outras telas desenham em escala 1 (o compositor amplia nas telas com escala); a escala nativa por camada fica para depois.
- Corrigido no compositor (achado pelo cenário de desligar e religar a tela com o app aberto): a tela desligada ou desconectada continuava anunciada aos apps (o global `wl_output` nunca era removido, no DRM e nas telas de teste); ao religar, os apps viam duas saídas com o mesmo nome, e uma camada pedida na antiga ia parar na principal. Agora o global é escondido ao desligar e removido de vez na mudança de telas seguinte (`outputs::retire_global`/`sweep_globals`).
- Apps de hoje não mudam: sem a opção, não há camadas extras, a origem é (0, 0) e o ponteiro segue o caminho de antes; a tela de bloqueio continua com o código dela. Nenhum app usa a opção ainda (a Captura é o passo 5).
- Exemplo `examples/kit-telas.rs`: uma camada escurecida em cada tela com o nome dela; registra cliques e arrastos em coordenadas globais. Prova a pega implícita do compositor: arrastar de uma tela para a outra mantém o ponteiro com a camada onde o botão desceu, nos dois sentidos e com a principal à direita.

## Captura em todas as telas e área ajustável (B22, passos 5 e 6)

- A Captura (`mide-screenshot`) usa `Window::on_every_output`: uma camada em cada tela. O mesmo `view` desenha cada uma com a parte dela, deslocada pela origem da tela: a captura congelada daquela tela escurecida, a área escolhida clara, o contorno, as marcações e as bolinhas. O rótulo do tamanho fica na tela onde está o canto de cima à esquerda da área. Teclas, ponteiro e painel só no passe da camada principal (`!ui.mirror()`), e o painel fica só na tela dela.
- A seleção, as marcações e o modo Janela usam o ponteiro global (`ui.pointer_global()`): a área pode ser escolhida em qualquer tela e atravessar de uma para a outra, nos dois sentidos; uma janela na outra tela pode ser escolhida no modo Janela. A área fica dentro do retângulo de todas as telas; na gravação (⌘⇧5, Área), por enquanto dentro da tela principal (a gravação de área em outra tela é o passo 8).
- Área ajustável: nova ferramenta **Ajustar**, a padrão (primeira da segunda linha do painel, antes de Seta, Círculo e Quadrado). Com ela, arrastar dentro da área muda o lugar da área; nas bolinhas (4 pontas e o meio dos 4 lados, na cor de destaque do tema com anel branco), muda o tamanho, com o lado oposto parado. Passar de um lado para o outro vira a área, o mínimo é 2 pontos e nada sai das telas. Com Seta, Círculo ou Quadrado, arrastar dentro da área desenha, como antes. Mover ou mudar o tamanho não move as marcações já feitas. Clicar fora da área escolhe outra (e apaga as marcações). Na gravação (⌘⇧5, Área) a área também se ajusta (dentro, move).
- O ⌘⇧4 continua sem bolinhas: ao soltar o mouse, salva e copia (também atravessando as telas).
- Funções puras com testes: `handles`, `handle_at` (até 10 pontos do centro, pontas antes dos meios), `resize_area`, `moved`, `clamp_point` e `draw_handles`.
- Log `captura: área LxA em (X, Y) [LxA px]` a cada área escolhida, movida ou redimensionada (para a bateria) e `captura: ferramenta NOME` ao trocar de ferramenta.
- Achados e corrigidos na conferência visual: a camada principal mostrava a área do quadro anterior depois de soltar o mouse (o desenho vem antes da lógica no `view`; agora apertar e soltar pedem mais um quadro); a sombra do rótulo do tamanho escurecia as bolinhas de cima (as bolinhas passaram a ser desenhadas por último). O anel da cor escolhida no painel usa a cor de destaque do tema (antes, um azul fixo).
- Ainda não (próximos passos da B22): "Tela inteira" perguntando qual tela, ⌘⇧3 com várias telas, gravação de área em outra tela e o cenário com a segunda tela em escala 1,25.

## Tela inteira, ⌘⇧3 e gravação com vários monitores, e escalas diferentes (B22, passos 7 a 9)

- **Tela inteira com mais de uma tela** (`Mode::PickScreen`): o botão "Tela inteira" pergunta qual tela. Todas ficam escuras; a que está sob o ponteiro fica clara, com um contorno na cor de destaque e um rótulo em vidro no meio ("Clique para capturar esta tela", com o nome e o tamanho em pixels). O clique escolhe a tela (a área vira ela inteira, e Copiar/Salvar levam a imagem dela, em pixels); Esc volta para a Seleção. Com uma tela só, "Tela inteira" é ela, sem perguntar (como antes).
- **⌘⇧3 com várias telas** (decisão tomada no plano): um arquivo por tela, com o nome da tela no fim (`Captura de tela 2026-10-23 10-00-00 (HDMI-A-1).png`), e a imagem da tela principal vai para a área de transferência. Um aviso só ("Capturas salvas", N telas). Com uma tela, o nome de antes.
- **⌘⇧4** já atravessava as telas (passo 6); agora também conferido numa área toda na segunda tela.
- **⌘⇧5 (gravar) com várias telas:** a gravação é de uma tela só (o compositor grava por saída). "Área" funciona em qualquer tela e fica presa à tela onde o arrasto começou (não atravessa; mover e as bolinhas também ficam nela); o pedido vai com a área em pontos daquela tela e o nome dela: `record start 134 300 300 200 tela HEADLESS-2 sistema` (`record_target` e `record_command`). "Tela inteira" com várias telas é o mesmo `PickScreen` ("Clique para gravar esta tela"), e o clique já começa a gravar a tela escolhida (como no Mac); Gravar ou Enter sem escolher grava a principal. Ao abrir o ⌘⇧5 com várias telas, ele já está escolhendo a tela. Com uma tela, o comando sai como antes (sem `tela`).
- **Escalas diferentes:** com a segunda tela em 1,25, uma área nela sai em pixels daquela tela (200x200 pontos → 250x250), do lugar certo; atravessando, o arquivo sai na maior escala entre as telas que a área toca e a parte da tela de escala menor é ampliada (fica um pouco menos nítida que a outra). A gravação de uma área numa tela em 1,25 vai em pixels (300x200 pontos → 374x250, lados pares).
- Nitidez: a camada da Captura desenha em escala 1 (`fixed_scale`); numa tela em escala fracionária, o que se vê durante a escolha fica um pouco suave, mas o arquivo sai em pixels da tela.
- Funções puras com testes: `screen_index_at`, `screen_index_near`, `record_target`, `record_command`, `shot_file_name`; `Capture::choose_full`/`pick_screen`. No compositor, teste do pedido com `tela NOME`.
- Logs para a bateria: `captura: tela NOME escolhida`, `captura: escolha da tela cancelada`, `captura rápida da tela inteira (N tela(s), ...)`.
- Borda comum entre telas (achado da revisão): para escolher a tela, o ponto conta como da tela que começa nele (teste semiaberto `contains_half_open`, usado por `screen_index_at`, `screen_index_near` e o destaque do `PickScreen`). Antes, com o ponteiro em x=1366 (primeira coluna da segunda tela), as duas telas ficavam destacadas, o clique escolhia a primeira e a Área do ⌘⇧5 começada ali saía com largura 0. Teste `borda_comum_e_da_tela_que_comeca_nela`.
- Fica para o teste do usuário (por isso a B22 segue `[~]`): os dois monitores de verdade, com escalas diferentes.

## Terminal: grade, linhas de quadro, powerline e Nerd Fonts (E1b)

O usuário mandou fotos do nvim (Neo-tree e lualine) e do prompt do bash com quatro defeitos. Reproduzidos com uma amostra (`printf` de quadros, blocos, setas, ícones e "Projetos my-text-game" com cores e negrito) e quadros salvos, as causas foram:

- **Espaços no meio das palavras ("Proj eto s", "my-text- game"):** o texto em negrito (nomes de pasta do Neo-tree, partes da lualine) era pedido ao cosmic-text com peso 700. Numa família sem negrito (a CodeNewRoman Nerd Font Mono, por exemplo, só tem o normal), o cosmic-text procura o peso em outra fonte e acha uma proporcional (Noto Sans Bold, DejaVu Sans): o "j" dela tem 5 pixels de avanço numa célula de 9, e as letras ficam soltas na grade. Agora o terminal escolhe o rosto da própria família (`MonoFace`: o peso e o estilo mais perto que ela tem, sempre de largura normal) e imita o que faltar: negrito desenhado duas vezes, um pixel ao lado; itálico inclinado (`FAKE_ITALIC` do cosmic-text).
- **Linhas de quadro com falhas e sem se encontrar:** os caracteres de quadro vinham da fonte, com a altura do desenho dela (menor que a célula de 1,3 em) e o avanço dela (7,8 pixels numa célula de 8). Agora `src/terminal/boxdraw.rs` desenha U+2500–U+257F na célula: cada caractere vira braços (cima, direita, baixo, esquerda) finos, grossos ou duplos, em faixas calculadas do mesmo jeito em todas as células (as linhas das vizinhas caem nos mesmos pixels), com os encontros de linhas duplas certos (╔ ╬ ╦ ╢ ╒), tracejados, arcos (╭╮╯╰, um quarto de círculo ligando os meios das bordas) e diagonais.
- **Blocos com frestas e setas do powerline desalinhadas:** também vinham da fonte. Os blocos (U+2580–U+259F: metades, oitavos, quadrantes e sombreados) e o powerline (U+E0B0–U+E0BF: setas cheias e finas, meios-círculos, cantos e diagonais) agora cobrem a célula inteira, com as bordas em pixels inteiros (▀ e ▄ juntos dão exatamente a célula) e a coluna da borda reta das setas toda pintada.
- **Ícones cortados ou fora da célula:** cada célula era um texto separado, com a linha de base da fonte de cada glifo (o ícone de uma Nerd Font de reserva ficava um pixel acima das letras), sem limite de tamanho (os ícones das Nerd Fonts sem "Mono" no nome têm 1,5 a 2 células de largura) e, sem Nerd Font na família escolhida, de qualquer fonte com algo naquele ponto da área privada (Material Icons, Tabler: outro desenho). Agora `Text::draw_cell` desenha os glifos com o SwashCache na linha de base da fonte principal; ícones (área privada) vêm de uma Nerd Font instalada quando a família não tem o ícone; ícone maior que o espaço é reduzido: uma célula, ou duas quando a seguinte está livre (como nos outros terminais; a tela continua contando uma célula, como o wcwidth e o nvim), começando na própria célula e com uma folga antes do texto seguinte; nunca passa da altura da linha.
- **Grade com a largura da fonte:** a célula é o avanço da fonte arredondado (`Text::mono_cell`, também com a altura da fonte quando maior que 1,3 em), e cada letra é centrada na célula pelo avanço.
- **Fonte padrão do sistema:** sem fonte escolhida, o cosmic-text procurava sempre a "Noto Sans Mono". Agora vale a do fontconfig (`fc-match monospace`, DejaVu Sans Mono aqui), ou uma das conhecidas, ou a primeira de largura fixa (também no editor de texto, que usa a mesma fonte de largura fixa).
- **Nerd Fonts na lista das Preferências:** as que não se declaram de largura fixa (a "Cousine Nerd Font": os ícones largos tiram o aviso do arquivo) entram se "i", "M", "W" e "." tiverem a mesma largura; antes, escolhida no arquivo, era ignorada sem aviso.
- Testes: `boxdraw` (8), `ui::text` (5 novos), `width` (1), `mide-terminal` (1) e a seção nova do `tests/ui.sh` com o conferidor de pixels (`tests/data/terminal/`). Fica `[~]` até o usuário conferir com o nvim dele (Neo-tree e lualine) e o prompt do bash.
- **Confirmado pelo usuário (2026-10-23):** com Nerd Font, o terminal desenha certo (mide em janela no Debian com Cinnamon/X11).

## Apps na GPU e captura por dmabuf (A25, bug urgente)

O problema (diagnóstico na sessão do usuário): o compositor não oferecia o protocolo `zwp_linux_dmabuf_v1`, o jeito de um app Wayland entregar ao compositor uma imagem que já está na placa de vídeo. Sem ele, o Mesa não tem como usar a GPU e todo app com OpenGL/Vulkan desenhava no processador (llvmpipe): o OBS gastava ~27% de CPU parado e mostrava a captura preta, e o mesmo valia para Chrome/Electron, jogos e o XWayland.

- **`linux-dmabuf` no compositor** (`src/dmabuf.rs`): o global sai na versão 5 com o "feedback" padrão, que diz aos apps qual é a placa principal (o render node, `main_device`) e quais formatos e modificadores o renderizador do mide sabe importar. O Mesa e o portal de captura usam o `main_device` para achar a placa. Cada buffer que um app cria é importado na hora no renderizador; se a importação falha, o app recebe `failed` e pode tentar outro formato. O desenho das janelas não mudou: o Smithay já sabia desenhar buffers dmabuf, faltava só anunciar o protocolo.
- **Nos três backends:** DRM (render node da placa aberta pela sessão; a tranche de scanout fica para o A18), sem tela (o próprio render node dos testes) e winit (a placa por trás do EGL do desktop de baixo; sem ela, fica sem o protocolo, que é só desenvolvimento). Sem GPU (renderizador sem importação de dmabuf), o protocolo não é anunciado e os apps continuam por memória compartilhada, como antes. O motivo vai para o log.
- **`MIDE_NO_DMABUF=1`:** variável de diagnóstico que não anuncia o protocolo, para o caso de algum driver dar problema na sessão de verdade (não é opção do usuário).
- **Captura por dmabuf** (`src/screencopy.rs`): com o protocolo ligado, o portal (`xdg-desktop-portal-wlr`, o que o OBS e o navegador usam) passa a entregar buffers na GPU. O mide copia o quadro para eles na própria GPU (`glBlitFramebuffer`, sem passar pela memória) e espera a cópia terminar por uma cerca do EGL antes de avisar o app (sem isso o quadro podia sair rasgado ou preto). O destino de cada buffer é preparado fora do quadro, quando o app pede a cópia (o Smithay guarda e o portal reaproveita os mesmos buffers). O caminho por memória compartilhada (o `grim`) continua igual, e a leitura dos pixels só acontece quando algum pedido é por memória.
- **winit:** a cópia sai de pé também com a janela invertida do GL (destino de cabeça para baixo no blit). Achado e corrigido no caminho: no winit, qualquer leitura dos pixels do quadro (o `grim`, a gravação) deixava o contexto do EGL sem a janela e com a leitura desligada (`glReadBuffer(GL_NONE)`, do Smithay); a troca de buffers seguinte falhava ("context lost") e o mide parava de desenhar, e uma cópia por dmabuf depois disso saía preta. Agora um quadro vazio liga a janela de novo (`screencopy::reattach_window`) quando houve captura ou gravação.
- **Gravação nativa (⌘⇧5, A21):** sem mudança; continua lendo o quadro do compositor.
- **Exemplo `examples/mide-screencopy-dmabuf.rs`:** faz o que o portal faz, sem o portal nem o PipeWire: acha a placa pelo feedback, cria um buffer na GPU (linear, com um modificador escolhido ou com os que a placa prefere), pede a tela ou uma região e lê o resultado importando o buffer no EGL (como o OBS lê), e salva um PNG. `--modifiers` lista os modificadores que o mide aceita.
- **`data/mide-check`:** dentro da sessão, mostra se os apps Wayland usam a GPU (pelo `eglinfo`).
- Na máquina do trabalho (Intel Alder Lake, backend sem tela): `eglinfo` de um app Wayland passou de `llvmpipe` para `Mesa Intel(R) Graphics (ADL GT2)`, o `glxinfo` pelo XWayland também, e o `glxgears` roda a ~58 quadros/s na placa. Falta a conferência na sessão de verdade (OBS, Chrome, gravação), por isso o A25 segue `[~]`.


## Arquivos lembra a visualização (B21)

- **Por pasta, como no Finder:** quem troca uma pasta para lista (ou ícones) ou muda a ordenação (coluna e sentido, clicando no cabeçalho da lista) encontra a pasta igual na próxima vez que abri-la, também em outra janela do Arquivos e depois de reiniciar o PC. Vale ao abrir o app, ao entrar numa pasta e ao voltar/avançar.
- **Padrão das pastas nunca ajustadas:** a última escolha feita em qualquer pasta (decisão desta etapa: é o que a pessoa espera depois de mudar a visualização, e evita uma tela só para "padrão"). A Lixeira, sempre em lista, guarda só a ordenação dela e não muda o padrão. O Arquivos não tem tamanho de ícones ajustável, então não há o que guardar disso.
- **Onde fica:** `~/.local/state/mide/files-view.toml` (respeita `XDG_STATE_HOME`; é estado do app, como a sessão do Terminal, não configuração). Módulo `mide::files_view` (`ViewMemory`), separado da interface e testado sem tela.
- **Seguro e com limite:** gravado num temporário ao lado e trocado de uma vez; antes de gravar, relê o disco (duas janelas do Arquivos não apagam a escolha uma da outra). No máximo 500 pastas: as ajustadas há mais tempo saem. A existência da pasta não é conferida (correção da revisão): pendrive desconectado ou rede desmontada não apagam as escolhas, e um compartilhamento travado não congela a janela; o limite de 500 basta para não crescer. Uma pasta renomeada abre com o padrão. A chave é o caminho normalizado só no texto (absoluto, sem `.`, barra repetida ou no fim: `mide-files ~/Downloads/` e `~/Downloads` são a mesma pasta), e nomes que não são UTF-8 viram `bytes:` com `%XX`, sem perda. Campo faltando numa entrada toma o padrão; arquivo ilegível é guardado como `files-view.toml.bak` antes de gravar o novo. Coluna inválida vira Nome, sem erro.
- **Testes:** os cenários (`tests/scenario.sh`) agora usam um `XDG_STATE_HOME` próprio, limpo a cada cenário (antes o estado dos apps de teste ia para a pasta de verdade); a seção nova `tests/ui/25-arquivos-visualizacao.sh` guarda o estado entre os cenários dela.

## Apps do mide como padrão do sistema (B25)

- **Apps padrão só na sessão do mide:** a escolha das Configurações → Apps padrão agora vai para `~/.config/mide-mimeapps.list`, que o `gio open`, o `xdg-open` e os navegadores leem antes do `mimeapps.list` geral **só** com `XDG_CURRENT_DESKTOP=mide` (padrão freedesktop por desktop). Antes ia para o `mimeapps.list` geral e mudava o Cinnamon. A leitura (`default_apps::current`) segue a ordem do GLib (em cada pasta, o do mide antes do geral). Ao entrar na sessão, o mide grava no arquivo dele o Arquivos para pastas e o Editor de Texto para textos, só nos tipos ainda sem escolha (nunca troca o que a pessoa escolheu). `data/applications/mide-mimeapps.list` traz os mesmos padrões como reserva (essa pasta só entra no `XDG_DATA_DIRS` da sessão do mide). A tela de Apps padrão diz que a escolha vale só no mide.
- **Correção:** o próprio mide (`places::open`: área de trabalho, dock, busca) lia as escolhas de outros desktops no `mimeapps.list` geral e abria pastas no Dolphin e textos no TextEditor. Agora só a escolha feita no mide tira pastas e textos dos apps do mide.
- **Arquivos abre com o item escolhido:** `mide-files --select itens...` abre a pasta dos itens com eles escolhidos e a lista (ou a grade) rola até o primeiro; um arquivo passado como argumento (também `file://` com `%20` e acentos) faz o mesmo (antes caía na pasta pessoal). `mide-files --info item` mostra as informações. Componente novo no mide-ui: `ui.reveal(rect)` rola a área em volta até o item (também ao andar pelas setas e ao trocar ícones/lista).
- **Obter informações:** no menu de contexto do item e em Ctrl+I: nome, tipo (com o MIME), tamanho (de pastas, a quantidade de itens), local, data de modificação e permissões (`files::info_lines`).
- **"Mostrar na pasta" abre o Arquivos (`org.freedesktop.FileManager1`):** programa novo `mide-portal`, iniciado pelo compositor **só na sessão do mide** (ou com `MIDE_PORTAL_FORCE=1` nos testes) e reiniciado pelo `mide-ctl restart shell`. Atende `ShowFolders`, `ShowItems` (itens da mesma pasta numa janela só, com eles escolhidos) e `ShowItemProperties`; a Lixeira (`trash:`) abre a Lixeira; outros endereços (`sftp:`) são recusados com aviso; o `startup_id` vai como token de ativação. O nome é pedido em tempo de execução, sem arquivo de ativação do D-Bus (que valeria no Cinnamon): com o Nemo já no nome, pede para trocar e, se ele não deixar, fica na fila. Sai sozinho quando o compositor que o abriu termina (o nome não fica preso depois da sessão).
- **Seletor de arquivos do mide nos apps (portal `FileChooser`):** o `mide-portal` também é o backend `org.freedesktop.impl.portal.FileChooser` (nome `org.freedesktop.impl.portal.desktop.mide`, versão 4: `OpenFile`, `SaveFile`, `SaveFiles`, com o `Request.Close`). Cada pedido abre o `mide-chooser`, janela nova do mide-ui com o seletor ocupando a janela, no meio da tela (regra do compositor para `app_id=mide-chooser`). As regras ficam em `mide::portal` (sem D-Bus nem janela): opções (`multiple`, `directory`, `filters` por nome e por MIME, `current_filter`, `choices`, `current_folder`, `current_name`, `current_file`, `files`, `accept_label`) viram argumentos do `mide-chooser`, e a saída dele vira a resposta (`uris` em `file://` com percent-encoding, `choices`, `current_filter`, `writable`). O pedido roda numa thread: o barramento continua atendendo (o `Close` chega com o seletor aberto).
- **`FileChooser` do mide-ui:** vários arquivos (`.multiple`, Ctrl e Shift+clique, `chosen()`), filtros com nome em lista ("Mostrar"; padrões de nome sem diferenciar maiúsculas, com `*`, `?` e `[...]`, ou tipos MIME como `image/*`), opções extras (caixa de marcar ou lista), texto do botão, "salvar vários" (escolhe a pasta, pergunta antes de substituir) e o modo de janela inteira (`.embedded`). O `.extensions` dos apps (Editor, Calculadora) continua igual.
- **Como o portal acha o seletor do mide:** `data/xdg-desktop-portal/portals/mide.portal` (`UseIn=mide`) e, no `mide-portals.conf`, `FileChooser=mide;gtk;` (o do GTK de reserva). O `xdg-desktop-portal` só lê os `.portal` de `/usr/share/xdg-desktop-portal/portals` ou de `XDG_DESKTOP_PORTAL_DIR`: sem o arquivo instalado no sistema (precisa de sudo), a sessão monta `~/.local/share/mide/xdg-desktop-portal/portals` com links para os do sistema e o do mide, publica a variável junto com o resto do ambiente e a tira do systemd do usuário ao sair (também o `mide-session`, mesmo se o mide cair), para não valer numa sessão do Cinnamon aberta depois. O `mide-update` grava a ativação do backend (`~/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.mide.service`, `mide-portal --portal-only`, que sai depois de 10 minutos sem pedidos); só é chamada com `XDG_CURRENT_DESKTOP=mide`.
- **Apps que usam o portal:** Chrome/Chromium e apps GTK 4 usam sozinhos; o `mide-session` exporta `GTK_USE_PORTAL=1` (apps GTK 3 e o Firefox fora do Flatpak), só no ambiente da sessão (não vai para o systemd).
- **Conferência:** `data/mide-check` mostra o app de pastas e de textos na sessão do mide, se o `mide.portal` está instalado e a ativação do seletor; dentro da sessão, quem tem o `FileManager1`, se o portal acha o `mide.portal` e se o seletor está no barramento.
- **Fica para depois:** ligar o seletor à janela do app que pediu (`parent_window`: `xdg-foreign` no compositor e no mide-ui, filho por cima do pai); imagens (depende do editor da B23) e compactados (E4) como padrão.

## Miniatura da captura e Editor de Capturas (B23)

- **Miniatura no canto:** depois do ⌘⇧4, do ⌘⇧3 (com várias telas, a da tela principal) e do "Salvar" (Ctrl+S) da Captura, a imagem fica salva em Imagens/Capturas de tela, como antes, e uma miniatura aparece no canto de baixo à direita da tela principal, acima da dock, com moldura branca, cantos arredondados e sombra. Some sozinha em 5 segundos; com o ponteiro em cima (ou o menu aberto) espera mais 2 s depois que ele sai. "Copiar" da Captura (sem arquivo) não mostra miniatura. A cópia para a área de transferência continua igual.
- **Na miniatura:** clique abre a captura no editor; clique direito abre o menu (Abrir no editor, Mostrar no Arquivos, Copiar, Mover para a Lixeira); arrastar para a direita (para fora da tela) faz ela sumir na hora, com o arquivo salvo (como no Mac). Ela não pega o teclado (não tira o foco de quem está digitando) e só a área dela recebe cliques.
- **Movimento:** entra esmaecendo e deslizando 8 pontos da borda direita (160 ms) e sai do mesmo jeito (120 ms), o efeito "Surgir" do `docs/design.md` (tabelas atualizadas); o interruptor é "Transições suaves" em Configurações → Efeitos (a explicação dele cita a miniatura).
- **Uma de cada vez:** uma captura nova fecha a miniatura anterior antes de capturar (para ela não sair na imagem): o PID da miniatura fica num arquivo da sessão (`mide::markup::thumbnail_pid_file`, com o nome do socket) e a captura grava o dela ali; a miniatura confere a cada quadro e sai.
- **Editor de Capturas (`mide-screenshot-editor`):** app do mide-ui que abre um PNG ou JPEG. Recortar (botão, Ctrl+K ou R) com a área ajustável da Captura (bolinhas na cor de destaque, arrastar dentro move, fora começa outra; Enter ou o botão "Recortar" aplica, Esc cancela); Lápis (desenho livre), Seta, Quadrado e Círculo (L, S, Q, C) nas seis cores da Captura; desfazer e refazer (também o recorte: nada é apagado da imagem original, o recorte é só uma edição); Salvar (Ctrl+S, substitui o arquivo, no formato dele, gravando num temporário e trocando de uma vez), Salvar como… (seletor de arquivos do mide-ui), Copiar (Ctrl+C, pelo compositor), Mover para a Lixeira (Ctrl+Delete, fecha a janela) e Mostrar no Arquivos. Menus na barra: Arquivo, Editar, Ferramentas e Janela. Fechar com alterações pergunta se salva. Sem arquivo, abre o seletor (Imagens/Capturas de tela). A imagem cabe na janela sem ampliar; a versão reduzida para a tela só é refeita quando algo muda. O traço engrossa em imagens grandes (4 pixels até 1600 de lado).
- **"Abrir com":** `data/applications/mide-screenshot-editor.desktop` (PNG e JPEG), fora do launcher (`NoDisplay`). O "Abrir com" do Arquivos passou a listar também os apps `NoDisplay` que abrem o tipo (`apps::load_handlers`; pela especificação, eles continuam abrindo os tipos deles). Não virou o padrão das imagens (é um editor, não um visualizador).
- **Código comum:** `src/markup.rs` (lib `mide::markup`) com as cores, as marcações (agora também o desenho livre, `draw_mark`) e a área ajustável com bolinhas, usados pela Captura e pelo editor (a Captura não mudou de comportamento). mide-ui: `Window::corner` (camada pequena no canto, sem teclado), `Ui::menu_open` e `Ui::close_menu` (menu de contexto numa camada pequena).
- **Correções da revisão (2026-10-23):** em telas 2x ou fracionárias, o editor e a miniatura mostram a imagem com um pixel dela por pixel da tela (antes ficava ampliada e borrada); a miniatura apertada perto da borda direita some ao chegar à borda (antes pedia 40 pontos de arrasto que não cabiam); a captura só espera a miniatura anterior sair quando o PID do arquivo é mesmo de uma miniatura (linha de comando com `--miniatura`) e apaga o arquivo depois (antes esperava 400 ms à toa com a Captura aberta); desfazer até o estado salvo não pergunta mais "Salvar as alterações?"; "Salvar como" com nome sem extensão ou de outro formato ganha `.png`; com imagens grandes, uma marcação nova é desenhada só ela, a redução para a tela começa da imagem sem copiar, e redimensionar a janela estica a imagem reduzida anterior e refaz a boa quando o tamanho para de mudar (150 ms).
- **Ficam para depois:** arrastar a miniatura para outro app (depende do arrastar entre apps, Wayland DnD), interruptor para não mostrar a miniatura (como a opção do Mac), texto e destaque no editor, zoom no editor.

## Aviso de volume e brilho (A23, OSD)

- Ao apertar volume +/−, mudo, microfone mudo ou brilho +/−, aparece um aviso no meio, embaixo da tela onde está o ponteiro (acima da dock; espelhando as telas, em todas): painel de 260x48 pontos com cantos de 14, em vidro de painel (desfoque + véu claro a 70%, ou grafite no tema escuro; opaco com o vidro desligado) e contorno fino. Dentro: o ícone (alto-falante com 0 a 3 ondas conforme o nível e cortado no mudo; microfone, cortado quando mudo; sol), a barra de nível contínua no azul de destaque (cinza no mudo) e o valor ("55%", "Mudo"); no microfone, no lugar da barra, "Microfone desligado"/"Microfone ligado".
- Entra esmaecendo e subindo 8 px (160 ms), fica 1,5 s depois da última tecla e sai esmaecendo (200 ms) (`src/motion.rs`: `OSD_APPEAR`, `OSD_HOLD`, `OSD_FADE`); com "Transições suaves" desligado, aparece e some de uma vez. Apertos seguidos só trocam o valor do mesmo aviso (sem entrar de novo).
- **Onde é desenhado (decisão):** no próprio compositor, como o alternador de janelas (`src/osd.rs`, chamado pela cena em `render.rs`). É um elemento da cena, sem cliente: não pega teclado nem clique, não abre programa e aparece também por cima da tela de bloqueio (volume, mudo, microfone e brilho, as teclas que já valiam com a tela bloqueada). Um cliente de camada (Overlay) não apareceria com a tela bloqueada, porque nada além da tela de bloqueio é desenhado.
- O nível é o de verdade: a tecla entra na fila de uma thread só do aviso (os apertos rodam em ordem, então a última leitura é sempre a do último aperto; sem a thread, a tecla ainda vale, sem aviso), que roda o comando (agora `volume_step`, `toggle_mute`, `toggle_mic_mute` e `brightness_step` esperam o comando), lê o nível que ficou (`system::read_volume`, `read_mic_muted`, `read_brightness`) e manda ao compositor por um canal do calloop; resposta de um aperto mais antigo é ignorada. Um relógio do calloop acorda o desenho para o aviso sair (parado, o compositor não desenha). Sem nível (sem áudio, monitor sem luz de fundo), não aparece aviso. Nos testes (`MIDE_SYSTEM_DRY_RUN`), os níveis são simulados (volume 50%, brilho 70%) e mudam com as teclas.
- Ícones desenhados pelo mide com o `tiny-skia` (nada copiado), em cache por tipo, ondas, mudo, tema e escala; o texto usa o mesmo desenho dos títulos das janelas.
- **Capturas (decisão):** o aviso fica fora das capturas de tela do mide (Print, ⌘⇧3/⌘⇧4), como o cursor; nas gravações (⌘⇧5) e nas capturas pedidas pelos apps (screencopy: OBS, grim) ele aparece, como o cursor nas gravações.
- Com a tela bloqueada não há vidro atrás do aviso (o caminho do bloqueio não desfoca regiões): o véu fica opaco (96%), para o texto continuar legível.
- Os elementos do aviso têm contagem própria no cache de elementos das decorações (`decorations::set_group(GROUP_OSD)`): o aviso aparecendo ou sumindo não muda a identidade das molduras das janelas, da dock e das prévias, e o rastreador de dano não redesenha a tela toda nesses quadros.
- Log: `osd: aparece na tela NOME`, `osd: volume 55%` (ou `(mudo)`, `brilho 75%`, `microfone desligado`) e `osd: escondido`.

## Senha de administrador gráfica (C7)

- **`mide-sudo` (programa novo):** `mide-sudo [--name NOME] [--icon ÍCONE] [--gui] <programa> [argumentos]` roda o programa como root pedindo a senha numa janela do mide, para usar em `.desktop` e scripts sem terminal (`Exec=mide-sudo --name XAMPP /opt/lampp/manager-linux-x64.run`). Por baixo é o `sudo -A` com o `mide-askpass` como `SUDO_ASKPASS`: quem confere a senha é o sudo (PAM), nunca o mide. Dentro de um terminal (entrada é um terminal, sem `--gui`), é o sudo de sempre, com a pergunta no terminal. Sai com o código do programa.
- **O que o programa root recebe:** só `DISPLAY` e `XAUTHORITY` (ou o `~/.Xauthority` do usuário, se existir), o idioma (`LANG`, `LANGUAGE`, `LC_*`), `GDK_BACKEND=x11`, `QT_QPA_PLATFORM=xcb` e `NO_AT_BRIDGE=1`, passados pelo `sudo -A -- /usr/bin/env K=V... programa` (funciona com o `env_reset` padrão e a regra `%sudo ALL=(ALL:ALL) ALL`). Ficam de fora `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR` e `DBUS_SESSION_BUS_ADDRESS`: o app root abre pelo X11 (XWayland do mide), não cria arquivos de root na pasta de execução do usuário e não alcança a sessão D-Bus dele. Se o sudoers só deixa rodar o próprio programa (recusa o `/usr/bin/env`), há uma segunda tentativa com `--preserve-env=LISTA`.
- **Tela X11 para o root:** o XWayland do mide não usa Xauthority; o `mide-sudo` libera a tela para o root com `xhost +si:localuser:root` só enquanto o programa roda (se ainda não estava liberada) e desfaz no fim.
- **Quem pede:** o nome mostrado na janela vem do `--name`, senão do app aberto pelo launcher, pela busca ou pela dock (agora eles passam `MIDE_APP_NAME` e, quando há, `MIDE_APP_ICON` ao app aberto, `apps::launch_env`), senão do nome do programa.
- **Erros:** três senhas erradas ("Senha incorreta") ou usuário sem permissão de sudo viram um aviso nas notificações; cancelar sai com 1, sem aviso. As mensagens do sudo são lidas em inglês (o sudo roda com `LANGUAGE=en`; o programa recebe o `LANGUAGE` do usuário).
- **`mide-askpass` (programa novo):** a janela, no mide-ui: cadeado desenhado pelo mide (com o ícone do app no canto, quando há), título "Digite sua senha", "XAMPP quer fazer alterações no sistema.", "Digite a senha de fabio para permitir.", o comando em texto pequeno, o campo de senha (bolinhas, sem copiar) e Cancelar/OK; Enter confirma, Esc ou fechar a janela cancelam. Senha recusada: "Senha incorreta. Tente de novo." em vermelho, o campo com a borda vermelha e o conteúdo balançando. A janela (`app_id` `mide-auth`) abre no meio da tela, por cima e com o foco (o compositor centraliza como o seletor de arquivos). O sudo chama o askpass de novo a cada tentativa: a contagem fica num arquivo 0600 em `XDG_RUNTIME_DIR` criado pelo `mide-sudo` (apagado no fim; sobras de um `mide-sudo` encerrado à força são apagadas na próxima vez).
- **`mide-polkit-agent` (programa novo):** agente de autenticação do polkit da sessão (`org.freedesktop.PolicyKit1.AuthenticationAgent`, só zbus, sem a libpolkit-agent), para o `pkexec`, o PackageKit e os apps que pedem permissão. Registra-se para a sessão de login (pelo logind/elogind, ou `XDG_SESSION_ID`); a cada `BeginAuthentication` escolhe o usuário (o próprio, inclusive quando o polkit manda o grupo `sudo`), abre a janela (`mide-askpass --agent`, que fica aberta entre as tentativas) e entrega a senha ao `polkit-agent-helper-1` (setuid de root, conferido) por pipe; três erros respondem `Failed`, cancelar (na janela ou pelo `CancelAuthentication`) responde `Cancelled`. Um pedido por vez. Iniciado pelo compositor só na sessão do mide (num mide em janela dentro do Cinnamon, o agente do Cinnamon continua) e reiniciado pelo `mide-ctl restart shell`.
- **A senha:** passa só por pipes (janela → sudo ou janela → agente → helper), nunca por argumento, variável de ambiente, log ou arquivo; o askpass não gera dump de memória (`PR_SET_DUMPABLE`) e as cópias da senha nos programas do mide são apagadas (`elevate::wipe`).
- **mide-ui:** `ui.password_field` (campo de senha com a cara dos campos do tema, borda vermelha no erro), `ui.error_note` (texto pequeno em vermelho) e `ui.dialog_buttons` (botões de diálogo à direita, o último é o principal; o `ui.alert` passou a usar).
- **Código:** regras sem interface em `src/elevate.rs` (ambiente do root, linha do sudo, erros do sudo, estado das tentativas, linhas do helper do polkit, escolha da identidade, protocolo da janela com o agente).
- **Revisão de segurança (2026-10-23):** o agente do polkit só atende o próprio polkit (pedido ou cancelamento de outro processo é recusado); o idioma passado ao root tem o mesmo filtro do sudo (nada de caminhos), a tela só `:N` e o Xauthority só com caminho absoluto; a senha não deixa cópias em buffers que não são apagados (leitura e escrita direto nos pipes, campos com espaço reservado) e nem o tamanho dela vai para o log; o agente também não gera dump de memória; o cancelamento não manda sinal a processos já encerrados; a tela X11 liberada para o root é dividida entre vários `mide-sudo` abertos (só o último fecha; a sobra de um encerrado à força é recolhida pelo próximo), e encerrar o `mide-sudo` (TERM, Ctrl+C, terminal fechado) fecha a janela de senha e desfaz a liberação; forçar o agente fora da sessão só vale em desenvolvimento. Riscos que ficam documentados no `INSTALATION.md`.
- **Fica para depois:** marca do compositor na janela de senha (para diferenciar de uma imitação), `--wayland` no `mide-sudo` (app root no Wayland com uma pasta de execução própria), ligar a janela do polkit à janela do app que pediu.

## Programas com duplo clique: .desktop, AppImage, executáveis e scripts (E8)

- **No Arquivos e na área de trabalho** (duplo clique, Enter e "Abrir"): um atalho `.desktop` abre o app que descreve (comando do `Exec` separado pelas regras da especificação, com aspas e sem os códigos `%f`/`%U`; `Path=` como pasta; `Terminal=true` abre no Terminal do mide; `TryExec` que não existe avisa "não está instalado"; `Type=Link` abre o endereço). AppImages, executáveis (ELF) e scripts (`#!`, ou `.sh` sem `#!`, rodado pelo `sh`) rodam com a pasta deles como pasta de trabalho. Documentos continuam abrindo no app padrão.
- **Nome e ícone do app:** o atalho aparece com o `Name` e o `Icon` dele no lugar do nome do arquivo, nos ícones e na lista do Arquivos e na área de trabalho (ícone por nome do tema ou por caminho de arquivo). Um atalho baixado (em Downloads ou com a marca do navegador) ainda não confiado continua com o nome do arquivo: um `.desktop` baixado podia se passar por "foto.jpg".
- **Confiança:** pergunta antes de rodar o que veio da internet (atributo `user.xdg.origin.url`, gravado pelos navegadores, com o site no texto; ou estar em Downloads) ou não está marcado como executável: "Abrir "<nome>"? Este programa veio de <origem>; abra só se confiar.", com Cancelar e Abrir. Abrir marca como executável (chmod +x para quem pode ler) e guarda a confiança num registro do mide (`$XDG_DATA_HOME/mide/programas-confiaveis`, com tamanho e data do arquivo): se o arquivo muda (outra versão com o mesmo nome), pergunta de novo. Atalhos instalados nas pastas de apps (`applications/`) são confiáveis. Scripts de texto sempre perguntam: Cancelar, Abrir no editor ou Executar. O Enter ou o clique que abre o diálogo não conta como resposta.
- **Código:** `src/programs.rs` (tipo do programa, leitura do atalho, separação do `Exec`, origem, registro de confiança, comando e o diálogo `Prompt` que os dois apps usam); o Arquivos e a área de trabalho só chamam `programs::open` e desenham o `Prompt`. O tipo "AppImage" na lista virou "Programa (AppImage)".
- **Fica para depois:** "Executar no Terminal" para scripts; o E9 (Instalar AppImage arrastando) usa o mesmo módulo.

## Instalar AppImage arrastando, no estilo do Mac (E9)

- **Duplo clique num AppImage** (Arquivos e área de trabalho) fora da pasta de apps: o diálogo do E8 passa a ser "Instalar ou abrir "<arquivo>"?" com Cancelar, Instalar e Abrir (Enter abre, como antes). O aviso de origem (internet, sem permissão) continua no texto quando vale. Instalar também é confiar (marca como executável e guarda no registro) e abre o instalador. Um AppImage já na pasta de apps abre direto.
- **Instalador `mide-install`** (app do mide-ui; o E6 vai reaproveitar a janela para .deb/.rpm): título "Instalar <app>", versão, tipo e tamanho; um palco com o ícone do app à esquerda, uma seta e a pasta Aplicativos à direita, os nomes embaixo, nas cores do tema (claro/escuro). Arrastar o ícone (ele segue o ponteiro, o lugar dele fica apagado e a pasta acende em azul) e soltar na pasta instala; soltar fora não faz nada. O botão Instalar (Enter) faz o mesmo. Progresso na própria janela ("Instalando… 45%", barra na cor de destaque); no fim, "foi instalado em Aplicativos e já aparece no launcher e na busca", com Fechar e Abrir (roda o app instalado). Se o mesmo app já está instalado: "<app> 1.0 já está em Aplicativos. Arraste para substituir pela versão 2.0." e o botão vira Substituir. Aberto direto num AppImage baixado ainda não confiado, pergunta antes (ler o que tem dentro roda o AppImage). `mide-install --remove <atalho>` pergunta "Mover "<app>" para a Lixeira?".
- **Pasta de apps (decisão):** `~/Aplicativos`, visível como o "Aplicativos" do Mac, com o nome no idioma ("Aplicativos", "Aplicaciones", "Applications"); se só existe `~/Applications` (de outro instalador, como o AppImageLauncher), usa ela.
- **O que vem de dentro:** o `.desktop` e o ícone (o citado no `.desktop`, SVG antes de PNG, senão o `.DirIcon`), extraídos pelo próprio AppImage (`--appimage-extract '<padrão>'`, tipos 1 e 2) numa pasta temporária, com limite de 30 s, sem aceitar links que saem da pasta extraída. Sem nada extraído, nome e versão vêm do nome do arquivo ("Krita-5.2.2-x86_64.AppImage" → "Krita", 5.2.2) e o ícone é o genérico.
- **Instalação:** cópia para a pasta de apps por um temporário escondido (troca só no fim, executável 0755), ícone em `$XDG_DATA_HOME/mide/appimage-icons/<id>.png|svg`, atalho `$XDG_DATA_HOME/applications/mide-appimage-<id>.desktop` feito do `.desktop` de dentro (traduções, categorias, tipos de arquivo, ações), com o `Exec` (também das ações) apontando para o AppImage instalado, `Icon` para o ícone copiado, sem `TryExec`/`DBusActivatable`/`NoDisplay`/`OnlyShowIn`, e as marcas `X-Mide-AppImage=<caminho>` e `X-AppImage-Version`. O id é o nome do `.desktop` de dentro: outra versão do mesmo app substitui a instalada (o arquivo antigo vai para a Lixeira se o nome mudou); outro app com o mesmo nome ganha "Nome (1).AppImage". O launcher e a busca já leem os apps a cada vez que abrem, então o app aparece sem recarregar nada.
- **Remover:** no launcher, clique direito num app instalado pelo mide abre um menu com "Mover para a Lixeira" (em vermelho; Esc ou clique fora fecham); escolher abre a confirmação do `mide-install --remove` e fecha o launcher; confirmar põe o AppImage na Lixeira e apaga o atalho e o ícone. Apps do sistema não ganham esse menu.
- **Também:** `ui.dialog_buttons` e `ui.alert` medem a linha de botões com o espaçamento real (10): com três botões o último saía cortado na borda do diálogo. `places::mide_program` acha um programa do mide ao lado do atual. `data/applications/mide-install.desktop` (sem aparecer no launcher) para o "Abrir com".
- **Código:** `src/appimage.rs` (pasta de apps, nome pelo arquivo, leitura do AppImage, atalho gerado, instalar, remover), `src/bin/mide-install.rs`, `src/programs.rs` (Instalar no diálogo), `src/bin/mide-launcher.rs` (menu).
- **Revisão de segurança (2026-10-23):** um atalho `.desktop` fora das pastas de apps só roda e só mostra o nome e o ícone do app depois de confiado (mesmo executável: num pendrive ou num zip extraído todo arquivo tem a permissão); programas num pendrive ou em outro disco perguntam antes; um arquivo com nome de documento ou de biblioteca (`foto.jpg`, `libx.so`, `.o`) e um ELF que não é programa (biblioteca, objeto) abrem como documento, nunca rodam; um FIFO ou um arquivo enorme com nome `.desktop` não trava a janela; o Enter não confirma o diálogo de algo baixado nem de um script (só o clique), e nos outros não vale logo que o diálogo aparece nem com a tecla segurada; o nome do AppImage instalado e o `Exec` do atalho não levam nada que o shell interprete; o instalador extrai numa pasta só do usuário no cache dele, mata o grupo inteiro no limite de tempo e não roda um AppImage sem a marca (tipo 0); a pasta de apps fica gravada na primeira instalação (trocar o idioma não "desinstala" nada); remover só manda para a Lixeira um AppImage da pasta de apps.
- **Fica para depois:** ler o squashfs (gzip/zstd) sem rodar o AppImage; atualização pelo zsync/AppImageUpdate; os .tar.gz comuns do E6 na mesma pasta; animação do ícone voltando quando é solto fora da pasta; o instalador nascer no meio da tela.

## Arquivos mais perto do Finder (B26, primeira parte)

- **Seleção por retângulo:** apertar no vazio e arrastar escolhe tudo o que o retângulo toca (Ctrl ou Shift somam à seleção de antes); o retângulo é desenhado na cor de destaque.
- **Zoom dos ícones:** Ctrl+roda, Ctrl + / − / 0 e o controle deslizante na barra de status, de 32 a 256 pontos, lembrado por pasta junto com a visualização (`files-view.toml` ganhou `columns` e `icon_size`).
- **Miniaturas de verdade** (`mide::thumbnails`): imagens; arquivos de texto viram uma folha com as primeiras linhas (como os ícones do Finder); PDF, vídeo, fontes, WebP, AppImage e o que mais houver gerador no sistema (`/usr/share/thumbnailers`), com cache em `~/.cache/mide/thumbnails`. Só os itens à vista são pedidos, em duas linhas de execução.
- **Visualização em colunas:** uma coluna por pasta do caminho até o item escolhido; ← e → andam entre as colunas.
- **Painel de prévia** (botão na barra de ferramentas): miniatura grande, nome, tipo, tamanho e as informações do item.
- **Visualização Rápida** (Espaço): o item grande no meio da janela; as setas trocam de item; Esc ou Espaço fecham.
- **Rodapé do Finder:** barra de caminho clicável (Computador › … › item) e barra de status com itens, escolhidos e espaço livre.
- **mide-ui:** `path_bar`, `status_bar`, `mini_slider`, `image_centered`, `page_sized`, `scroll_area_padded` e `take_scroll`.
- **Regra forte no CONTEXT.md:** o visual de referência é o macOS Tahoe.

## Arrastar arquivos entre apps (B26, 2026-10-24)
- **Arquivos → outros apps:** arrastar itens para fora da janela vira um arrasto do Wayland com a lista de arquivos (`text/uri-list`): soltar num site no navegador (WhatsApp Web, Telegram Web), no Telegram, no Nautilus ou em outra janela do Arquivos entrega os arquivos (cópia). O ícone que segue o ponteiro é a miniatura do primeiro item, com um selo vermelho com a contagem quando são vários.
- **Outros apps → Arquivos:** arquivos soltos na janela vão para a pasta sob o ponteiro (destacada) ou para a pasta aberta; de outro app, copia; voltando do próprio Arquivos, move (com Ctrl, copia).
- **mide-ui:** `ui.start_drag(caminhos, ícone)`, `input().drop_hover`, `input().dropped` e `input().drag_ended`, valendo para qualquer app; `mide::kit::parse_uri_list` e `mide::thumbnails::file_uri`.
- **Compositor:** desenha o ícone do arrasto junto do ponteiro (com o deslocamento pedido pelo app), na frente de tudo e fora das capturas de tela.

## Janela no estilo do Tahoe (D3, etapas 2 e 4, 2026-10-24)
- **Janela de conteúdo inteiro** (`Window::full_size_content`): o app do mide-ui abre sem a barra de título do compositor e com cantos de 24; os três botões da janela são desenhados pelo próprio app (cinza sem foco, com ×, − e + ao passar o ponteiro sobre o grupo). Arrastar o vazio da barra de ferramentas ou da barra lateral move a janela; duplo clique maximiza e restaura. Apps de terceiros continuam com a barra do mide, agora com cantos de 16.
- **Barra lateral flutuante:** painel de vidro solto 8 pontos da borda, raio 16, com os botões da janela dentro, títulos de seção pequenos, linhas de 28, seleção em pílula cinza e ícones na cor de destaque (nas Configurações, com os quadradinhos coloridos, a pílula é azul).
- **Barra de ferramentas do Tahoe:** sem fundo e sem linha; botões agrupados em cápsulas (`ui.toolbar_buttons`, com o escolhido marcado por uma pílula), título à esquerda que encolhe com reticências (`ui.toolbar_title`) e busca em cápsula com lupa e ✕ (`ui.search_field`). Os grupos ficam presos à direita.
- **Arquivos** é o primeiro app no visual novo.
- Compositor: `window_style app:<app_id> radius=N` no canal de controle (cantos por app).

## Controles e menus do Tahoe (D3, etapas 3, 5 e 7, 2026-10-24)
- **Botões em cápsula** em todos os apps (`button`, `primary_button`) e estilos novos com `ui.button_styled`: principal (azul), comum, só texto, vidro e apagar (vermelho), com brilho de borda nos cheios.
- **Interruptor e deslizante** com o botão branco do Tahoe, que estica enquanto é apertado ou arrastado; trilho do deslizante mais fino.
- **Controles novos:** segmentado com a pílula deslizando até a opção (com interruptor em Efeitos → Transições), caixa de seleção, botão de opção, passo (▲▼), barra de progresso (com andamento ou indeterminada), selo vermelho e bolinhas de etiqueta nas cores do Finder.
- **Menus:** cantos de 12, sem a setinha, destaque em pílula de 8, contorno fino; itens com símbolo à esquerda e atalho à direita, item marcado (✓) e a linha das etiquetas coloridas (para o Finder). As posições das linhas não mudaram.
- **Diálogos** com cantos de 20.
- **Galeria** (`cargo run --example mide-ui-demo`): janela do Tahoe com uma página para botões, controles e indicadores.

## Configurações no visual do Tahoe (D3, etapa 8, 2026-10-24)
- As Configurações viraram janela de conteúdo inteiro, como os Ajustes do Sistema: barra lateral flutuante de vidro com os botões da janela, busca em cápsula e cada seção com um símbolo branco no quadradinho colorido; a faixa do título da página arrasta a janela. O conteúdo das páginas ficou no mesmo lugar.
- Corrigido junto: os ícones da barra lateral das Configurações não apareciam (os nomes coloridos só têm tamanhos de contorno no tema do mide, recusados desde o B24); agora são símbolos.

## Abas, dica, popover e seleção do Tahoe (D3, 2026-10-24)
- **Barra de abas** (`ui.tab_bar`): abas de largura igual numa faixa em cápsula, a escolhida em destaque, × ao passar o ponteiro e + no fim (para as abas do Finder).
- **Dica** (`ui.tooltip`): cápsula pequena embaixo do componente depois de o ponteiro parar 0,8 s.
- **Popover** (`ui.popover`): painel de vidro com ponta, preso a um botão; fecha com clique fora ou Esc. `ui.to_window` converte o retângulo de uma área que rola para a janela.
- **Seleção** das listas, da árvore e da grade em pílula de cantos 8, cinza quando a janela perde o foco, como no Mac.
- Tudo na galeria `mide-ui-demo` (página Indicadores).

## Ajustes pedidos depois do primeiro teste do Tahoe (2026-10-24)
- Botões da janela: o símbolo (×, −, +) aparece só no botão sob o ponteiro.
- Rodapé do Arquivos: barra de caminho e de status coladas, com uma linha só em cima e o conteúdo centralizado na altura.
- Configurações: símbolos no meio dos quadradinhos coloridos e barra de rolagem na barra lateral quando ela não cabe.
- Cartões: o destaque de uma linha vai de borda a borda do cartão e encosta nas linhas divisórias (o conteúdo do cartão é recortado nos cantos arredondados).
- Colunas do Finder: largura que se ajusta para caber colunas inteiras (mínimo 200), a vista desliza (animada, com interruptor em Efeitos → Transições) até a última: a pasta clicada fica no meio, a de cima à esquerda e o conteúdo à direita; Shift+roda rola para os lados. `ui.clip` no mide-ui (desenho recortado num retângulo).
- Arrasto dentro do Arquivos: a miniatura dos itens (com o selo de contagem) segue o ponteiro em qualquer visualização.
- Sombra das janelas menor e desenhada logo atrás de cada janela: cai também sobre as janelas de baixo.

## Finder e janelas do Mac, segunda leva (2026-10-24)
- **Enter renomeia no lugar** (como no Finder): o nome vira um campo com só o nome escolhido (sem a extensão); Enter confirma, Esc cancela, clicar fora confirma. Abrir: ⌘O/Ctrl+O ou ⌘↓/Ctrl+↓; ⌘↑/Ctrl+↑ sobe para a pasta de cima. `ui.focus_next_input_selecting` no mide-ui.
- **Seleção de ícone como no Finder:** fundo cinza arredondado atrás do ícone e o nome em cápsula azul (na área de trabalho continua o vidro).
- **Ícone de arquivo de texto como no Mac:** folha com a orelha dobrada, as primeiras linhas do conteúdo e a extensão do arquivo (TXT, MD, JS…) grande e em cinza por cima.
- **Etiquetas coloridas:** linha das sete cores no menu de contexto (marca/desmarca em todos os escolhidos) e bolinhas no canto do ícone ou no fim do nome. Gravadas no próprio arquivo (`user.xdg.tags`, o padrão do Linux, o mesmo do Dolphin), sem perder etiquetas de outros programas.
- **Janela Informações (⌘I)**, uma por item (`mide-files --info`, `mide::files_info`): ícone, nome, tamanho e data; etiquetas clicáveis; seções que abrem e fecham: Geral (tipo, tamanho em bytes e no disco, onde, com "Copiar como nome do caminho" no clique direito, criado, modificado), Mais informações (último acesso, dimensões de imagens, tipo MIME), Nome e extensão (renomeia), Comentários (`user.xdg.comment`), Abrir com (com "Alterar todos…"), Prévia e Compartilhamento e permissões (dono, grupo e todos, editáveis). `ui.key_value` e `ui.tag_toggle` no mide-ui.
- **Sobre este computador** refeito no mide-ui como o do Mac: janela em pé e de vidro, só com o fechar ativo, desenho próprio do computador (monitor ou notebook, pelo tipo de gabinete), nome do modelo, fabricante, tipo e ano, especificações que quebram a linha (processador, gráficos, memória arredondada como a do pente, disco de inicialização, nome do computador, sistema) e "Mais informações…" (Configurações → Sobre). `ui.window_close_only`, `ui.centered_title`, `ui.centered_note`, `ui.space_x`.
- **Miniatura da captura (⇧⌘4) arrastável:** arrastar para a esquerda, para cima ou para baixo leva o arquivo para outro app (Telegram, WhatsApp, uma janela); para a direita continua descartando.

## Janela de tamanho fixo e ajustes finos (2026-10-24)
- **Regra do componente de janela (mide-ui):** `Window::fixed_size()` deixa a janela sem redimensionar; os botões de minimizar e maximizar ficam cinza e sem ação (só o fechar funciona), e o duplo clique não maximiza. O "Sobre este computador" usa isso.
- **Compositor respeita os limites de tamanho do app** (`set_min_size`/`set_max_size`): janela com mínimo igual ao máximo não redimensiona pela borda nem maximiza; nas outras, o redimensionamento fica dentro dos limites.
- **Ícone de texto:** a sombra segue o formato da folha (a sombra retangular aparecia no canto como uma segunda orelha).
- **Configurações:** símbolos de 16 no centro exato dos quadradinhos de 22.

## Dock do Tahoe: escura, clique e ampliação (2026-10-24)
- **Variante escura:** com Configurações → Aparência em escuro, a dock ganha um véu grafite sobre o vidro, separadores e pontos de app aberto claros (muda na hora).
- **Clique visível:** o ícone apertado escurece até soltar, como no Mac.
- **Ampliação** (Configurações → Dock → Ampliação e Tamanho da ampliação, 56–128; desligada por padrão, como no Mac): os ícones perto do ponteiro crescem numa curva suave, os vizinhos se afastam e a faixa de vidro cresce junto; entra e sai animada (com o interruptor de Animações). Os ícones ampliados são carregados no tamanho grande (nítidos), numa camada própria acima da dock que não recebe cliques; os cliques seguem as posições ampliadas.

## Agitar o ponteiro para localizá-lo (2026-10-24)
- Como no Mac: quatro vaivéns rápidos do mouse (de pelo menos 30 pontos, em 0,7 s) fazem o cursor crescer até 4 vezes, com animação de crescer (180 ms), segurar enquanto agita (+0,5 s) e voltar (320 ms). A imagem vem do tema de cursores no tamanho mais próximo (nítida) e, enquanto cresce, a cena desenha o cursor no lugar do plano de cursor da placa. Interruptor em Configurações → Efeitos ("Agitar o ponteiro para localizá-lo", `[effects] shake_cursor`). Módulo `src/shake.rs`, com testes.

## Calculadora no visual do Tahoe (D3, etapa 8, 2026-10-24)
- Janela de conteúdo inteiro (cantos de 24) com os botões da janela por cima do visor (o alto do visor arrasta a janela), fundo grafite, teclas redondas: números em cinza escuro, funções em cinza mais claro com texto branco e operações em laranja; disposição do Tahoe na Básica: ⌫ AC % ÷ em cima e ± 0 , = embaixo (o ⌫ do visor fica só no Programador e no conversor); a conta em andamento em cinza logo acima do número; sinais das operações maiores. Tamanho padrão da Básica 300x490.

## Editor de Capturas como a Marcação do Mac (2026-10-24)

- Pedido do usuário: o traço feito com a caneta vira forma editável (como no Mac), com alças de seleção e de tamanho, seletor e as ferramentas da Marcação.
- **Formas editáveis (`src/annotate.rs`):** desenho livre, linha, seta, retângulo, retângulo arredondado, oval, triângulo, estrela, balão de fala e texto. Cada forma guarda borda, preenchimento, espessura, tracejado e texto, e fica editável até salvar ou copiar (só então entra nos pixels). O reconhecimento do esboço simplifica o traço e decide entre linha, seta, retângulo, triângulo e oval.
- **Ferramentas:** Selecionar (V), Esboço (S, o traço vira forma e já fica escolhido), Desenho (D, traço livre), Formas (menu com as oito) e Texto (T: clique e digite). Com a forma escolhida: arrastar move, as alças redimensionam (numa linha ou seta, as pontas), Delete apaga, setas andam 1 pixel, Esc tira a escolha.
- **Estilo:** popovers de Estilo do traço (cinco espessuras, tracejado), Cor da borda, Preenchimento (ou nenhum) e Estilo do texto (tamanho). Valem para a próxima forma e para a escolhida.
- **Mais:** girar para a esquerda e a direita, recortar, desfazer e refazer de tudo (`doc.rs` guarda o estado inteiro: imagem, formas e recorte), Copiar e Salvar na barra. A janela é de conteúdo inteiro (Tahoe), e o menu Ferramentas tem tudo isso.
- **mide-ui:** `Ui::last_rect()` dá o retângulo do último componente (âncora de popovers), e `Text::draw_px` desenha texto em pixels do canvas.

## Calculadora: correções do visual do Tahoe (2026-10-24)

- **Fechava ao trocar de modo:** no primeiro quadro depois da troca, a janela ainda tem o tamanho do modo anterior, as teclas ficavam mais estreitas que o mínimo e o `clamp` do tamanho caía (mínimo maior que o máximo). Agora o tamanho das teclas aceita a janela pequena até o compositor mudar o tamanho.
- **Luzes por cima do conteúdo:** o visor ganhou uma faixa de cima (`TITLE_H`, 36 pontos) para os botões da janela. As bases e os bits do Programador, as linhas do conversor, Rad/M, a pilha do RPN e a conta pequena ficam abaixo dela. O Programador ficou 40 pontos mais alto.
- **Luzes duplicadas nas Notas:** a barra lateral do Tahoe já desenha os botões da janela. O mide-ui agora desenha as luzes uma vez por quadro (o segundo pedido no mesmo quadro é ignorado).

## Esboço: reconhecimento mais tolerante (2026-10-24)

- Pedido do usuário: o quadrado convertia bem, o círculo às vezes falhava e a seta era muito difícil.
- **Reto:** passa a ser medido pelo desvio máximo da reta. Antes era o comprimento do caminho, que cresce com o tremor da mão e recusava quase toda seta.
- **Seta:** a haste reta até a ponta e a cabeça perto dela, saindo para o lado (uma asa basta). Também em dois traços: uma linha e, logo depois, a cabeça perto de uma ponta transformam a linha em seta.
- **Formas fechadas:** contam como fechadas também quando o traço dá a volta e passa do começo, ou deixa uma fresta pequena. A forma é escolhida pela parte da caixa que o traço cobre (retângulo ≈ 1, oval ≈ 0,79, triângulo ≈ 0,5). Estrela nova (8 a 12 pontas alternando longe e perto do centro).
- O aviso do editor ("Imagem copiada…") ganhou recuo à esquerda.

## Editor de Texto no Tahoe e menus pelo processo da janela (2026-10-24)

- **Editor de Texto (`mide-text`), como o TextEdit:**
  - janela de conteúdo inteiro, com as luzes e o nome do documento ("— Editado") na barra de ferramentas;
  - cápsulas de Abrir/Salvar e de Buscar à direita;
  - barra de busca com campos em cápsula, anterior/próxima numa cápsula, Substituir, Tudo e OK;
  - rodapé com linha, coluna e o total de linhas.
- **Menus do app na barra:** o `list` do compositor manda o processo (PID) de cada janela, como quarto campo. A barra prefere os menus registrados por esse processo. Antes valia o registro mais novo com o mesmo nome, e com dois processos do mesmo app (o Terminal comum e o de atalho, F12) a barra podia mostrar os menus genéricos ou os do outro processo. Pedido do usuário: o Terminal mostrava Arquivo/Editar/Janela e não dava para abrir as Preferências.

## Correções: menu do app depois de trocar os menus e clique que maximizava (2026-10-24)

- **Menu vazio:** os números dos itens dos menus publicados pelo mide-ui dependem da posição. Ao trocar de modo, a Calculadora muda os menus, o "Editar" ganha outro número e a barra abria um menu vazio. Agora a barra relê a raiz ao abrir um menu e o acha pelo título. Os títulos do app em foco também são relidos a cada volta da barra, então menus que entram ou saem aparecem sem trocar de janela.
- **Janela maximizava com um clique (Notas):** a área de arrastar da barra lateral e a da Calculadora se sobrepunham, e o mesmo clique contava duas vezes, como duplo clique. Agora o mide-ui trata um clique de arrastar por quadro.

## Tema claro/escuro sem esperar o ponteiro (2026-10-24)

- A barra superior só relia as cores ao se redesenhar por outro motivo (o ponteiro passando). Agora ela confere o tema a cada volta (450 ms; o arquivo só é lido quando muda) e se redesenha sozinha, com o menu aberto junto. Os apps do mide-ui (a cada segundo) e a dock já faziam isso; conferido por captura com a troca feita no meio do cenário.

## Editor de Texto completo, etapa 1a: núcleo do texto formatado (2026-10-24)

- `src/richtext/` (biblioteca, com testes):
  - `mod.rs`: documento em parágrafos com trechos de estilo (fonte, tamanho, negrito, itálico, sublinhado, tachado, cor, realce, sobrescrito/subscrito, link) e estilo de parágrafo (alinhamento, recuos, entrelinha, espaços, listas, nome do estilo, quebra de página antes), imagens num parágrafo delas e a página (tamanho, margens, cabeçalho, rodapé e numeração com "mostrar a partir da página N", como a ABNT pede). Edição: digitar, Enter, apagar intervalos entre parágrafos, aplicar estilo de texto e de parágrafo, palavras (Ctrl+setas, duplo clique), copiar e colar com formatação (`fragment`/`insert_fragment`) e contagem de palavras.
  - `layout.rs`: o cosmic-text dá forma às letras; a quebra em linhas (palavra a palavra, recuo da primeira linha, tabulações de meia polegada, justificado, centralizado, à direita), os marcadores de lista e a paginação são daqui, com cache por parágrafo (digitar num livro só refaz o parágrafo mexido). Fontes de métricas iguais para as que faltam no Linux (Times New Roman → Liberation Serif/Tinos, Arial → Liberation Sans/Arimo, Calibri → Carlito...).
  - `render.rs`: desenho de uma página em qualquer escala (seleção, realce, letras, sublinhado, tachado, imagens, marcadores, cursor, cabeçalho, rodapé e número da página).
  - `rtf.rs`: ler e gravar RTF (o formato do TextEdit e do WordPad), escrito aqui, com imagens PNG/JPEG, papel e margens; lê o RTF do TextEdit (conferido num teste).
- Falta a tela nova do Editor de Texto usando esse núcleo (etapa 1b).

## Editor de Texto completo, etapa 1b: tela do modo formatado (2026-10-24)

- **Modo formatado (`src/bin/mide-text/rich.rs`):** documento novo e arquivos .rtf abrem formatados (como no TextEdit); .txt e código continuam no modo de texto simples. Formatar → Converter em texto simples/formatado (Ctrl+Shift+T).
- **Na tela:** páginas A4 (100% = 96 pontos de tela por polegada, como no Word) com sombra, rolagem, zoom (Ctrl+roda, Ctrl+=/−/0), cursor piscando, seleção com o mouse (arrastar, duplo clique na palavra, triplo no parágrafo, Shift+clique) e rodapé com página, palavras, caracteres e zoom.
- **Teclado:** digitação, Enter, Shift+Enter (quebra de linha), Ctrl+Enter (quebra de página), Backspace/Delete (com Ctrl, palavra), setas (Ctrl: palavra; Shift: seleção), Home/End (Ctrl: documento), PageUp/Down, Tab (nas listas, desce de nível), desfazer/refazer, recortar/copiar/colar (com a formatação dentro do editor), Ctrl+B/I/U, Ctrl+L/E/R/J (alinhamento), Ctrl+Shift+./, (tamanho).
- **Barra de formatação do TextEdit (Tahoe):** estilo de parágrafo (Corpo, Título, Título 1–3, Citação no padrão ABNT, Legenda, Referência), família da fonte (com busca), tamanho, cor do texto, realce, B/I/U/S, alinhamento, entrelinha e espaço depois, listas (marcadores, travessões, números, letras) e inserir imagem (PNG/JPEG).
- **Menus:** Arquivo, Editar, Formatar (Fonte, Texto, Lista, imagem, quebra de página, Configurar página, Aplicar normas ABNT), Visualizar (zoom).
- **Configurar página (Ctrl+Shift+P):** A4/Carta, retrato/paisagem, margens em cm, cabeçalho, rodapé e numeração (posição e "a partir da página N").
- **Aplicar normas ABNT:** A4, margens 3/2/2/3 cm, Times New Roman ou Arial 12, entrelinha 1,5, recuo de 1,25 cm, justificado, citações com 4 cm e letra 10, números no canto de cima a partir da página escolhida.
- **mide-ui:** `fixed_label` e `form_row` (formulário com rótulos alinhados); `Text::font_system`, `draw_glyph` e `families`.
- **Teste:** o `type` dos cenários digita frases inteiras (espaços e maiúsculas).
- **Falta:** DOCX, ODT, PDF e imprimir (etapa 2); buscar e substituir no modo formatado; régua; tabelas; sumário e notas de rodapé.

## Menus dos apps que sumiam (2026-10-24)

- Causa: os cenários de teste rodavam no D-Bus da sessão de verdade e a barra do teste tomava o registrar dos menus. Os testes agora usam um barramento próprio. A barra recupera o nome do registrar sozinha (conferência a cada 5 s), tenta de novo as leituras que falharam (depois de 3 s) e relê os títulos do app em foco a cada 1,5 s (antes, 450 ms).

## Editor de Texto, etapa 2a: PDF e imprimir (2026-10-24)

- **`src/richtext/pdf.rs`, gravador de PDF próprio:** as letras saem nas posições do layout da tela (o que se vê é o que se imprime). Fontes TrueType e OpenType/CFF embutidas inteiras; as de coleções .ttc são separadas num arquivo próprio. Mapa para o Unicode, então copiar e buscar no leitor de PDF funciona (conferido com o `pdftotext`). Também cores, realce, sublinhado, tachado, itálico imitado, imagens (JPEG como veio, PNG com transparência), marcadores de lista, cabeçalho, rodapé, números de página e intervalo de páginas. Conteúdo compactado com o `flate2` (já vinha com o `zip`).
- **Imprimir (Ctrl+P, Arquivo → Imprimir…)**, como o do Mac: prévia da página com ‹ ›; destino "Salvar como PDF" ou uma impressora do CUPS (`lpstat`, a padrão primeiro); cópias, páginas (todas, a atual ou um intervalo) e frente e verso. Imprime mandando um PDF temporário ao `lp`. Sem impressora configurada, o diálogo avisa e oferece o PDF.
- **Arquivo → Exportar como PDF…**
- **mide-ui:** `columns` (duas colunas lado a lado, como o `NSStackView`).

## Editor de Texto, etapa 2b: DOCX e ODT (2026-10-24)

- **`src/richtext/xml.rs`:** leitor e escritor de XML mínimos, próprios.
- **`src/richtext/docx.rs` (Word):** ler e gravar. Estilos de parágrafo mapeados (Title, Heading1–3, Quote, Caption, Bibliography); fonte, tamanho, negrito, itálico, sublinhado, tachado, cor, realce, sobrescrito, subscrito; alinhamento, recuos, entrelinha, espaços, quebra de página; listas (`numbering.xml`); imagens (`wp:inline`); papel, margens, cabeçalho e rodapé com o campo PAGE. "Número a partir da página N" vira "primeira página diferente" no Word, e o N exato fica num `docVar` que o mide relê. A leitura entende estilos herdados (`basedOn`), a fonte do tema, links, campos e conteúdos (`sdt`); tabelas viram parágrafos por enquanto.
- **`src/richtext/odt.rs` (LibreOffice):** o mesmo, com estilos automáticos, listas aninhadas, espaços seguidos (`text:s`), imagens em `Pictures/`, página e cabeçalho/rodapé na página mestra, e a altura do cabeçalho somada à margem (também no modo `dynamic-spacing` do LibreOffice).
- **Conferido com o LibreOffice nos dois sentidos:** ele abre os nossos .docx e .odt (convertidos para PDF, com o texto, as listas e o realce certos) e o mide lê o que ele grava (margens 3/2/2/3 cm, estilos, listas, cores).
- **No editor:** .docx, .odt e .rtf abrem formatados; salvar usa a extensão; documento novo sugere .docx. Configurações → Apps padrão ganhou "Documentos" (o Editor de Texto é o padrão da sessão se nada foi escolhido); o `.desktop` do editor declara os tipos.

## Editor de Texto, etapa 3: buscar e substituir no modo formatado (2026-10-24)

- Ctrl+F (e o botão da lupa, e Editar → Buscar e substituir…) abre a barra abaixo da de formatação. Busca em cápsula; anterior/próxima (também Ctrl+G e Ctrl+Shift+G); "Aa" para diferenciar maiúsculas (desligado por padrão); substituir e substituir tudo, mantendo a formatação do texto trocado. Desfazer volta a substituição inteira.

## Editor de Texto, etapa 4a: sumário (2026-10-24)

- Formatar → Inserir sumário: "SUMÁRIO" centralizado e uma linha por título (Título 1, 2 e 3, com recuo por nível, nível 1 em negrito), com pontilhado até o número da página na margem direita; o texto seguinte começa numa página nova. Formatar → Atualizar sumário refaz a lista. Os números são acertados depois de quebrar o documento (até duas passadas, porque o sumário pode mudar as páginas).
- Tabulação com pontilhado (`ParaStyle::leader`) na quebra de linhas, na tela, no PDF e nos formatos: RTF `\tqr\tldot`, DOCX `w:tabs` com `w:leader="dot"` (e os estilos TOC do Word lidos como sumário), ODT `style:tab-stops` com `style:leader-text`.
- Atalhos de título como no Word: Ctrl+Alt+1, 2 e 3; Ctrl+Alt+0 volta ao corpo.
- Teste de unidade do sumário (páginas 2 e 3 e o pontilhado até a margem).

## Editor de Texto, etapa 4b: notas de rodapé (2026-10-24)

- Formatar → Inserir nota de rodapé… (Ctrl+Alt+F, como no Word): diálogo com o texto da nota; o número sobrescrito entra no texto e a nota vai para o pé da mesma página, com o traço separador, em letra 10, entrelinha simples, preto e justificado (ABNT). Numeração automática na ordem do texto. Duplo clique no número edita ou remove a nota.
- Modelo: o caractere `NOTE` no texto, com `CharStyle::note` (id), e `Doc::notes`. A paginação reserva o espaço das notas no pé da página onde a referência cai (a linha passa para a página seguinte junto com a nota, se não couber).
- Na tela, no PDF (texto copiável) e nos formatos: RTF `\footnote` com `\chftn`; DOCX `footnotes.xml` com os separadores do Word; ODT `text:note`. Conferido com o LibreOffice (ele mostra as notas dos nossos .docx e .odt) e em testes de ida e volta.
- mide-ui: o Alt de cada atalho é guardado (`shortcut_alt`, `shortcut_plain`); atalhos com pontuação (Ctrl+=, Ctrl+−, Ctrl+Shift+.) passam a chegar aos apps. Os diálogos do editor devolvem o teclado ao texto ao fechar.

## Editor de Texto, etapa 4c: tabelas (2026-10-24)

- Tabela > Inserir tabela… (Ctrl+Alt+T): linhas e colunas. Cada célula é um documento pequeno (`Cell` com um `Doc`): escrever, formatar, listas, colar e desfazer funcionam dentro dela. Ao entrar numa célula, o editor troca o documento de fora pelo da célula (`Outer`); ao sair, a célula volta e a edição dela vira um passo de desfazer do documento.
- Tab/Shift+Tab andam entre as células (depois da última, uma linha nova); ↑/↓ na primeira/última linha passam para a célula de cima/de baixo; Esc ou um clique fora saem.
- Menu Tabela: inserir linha acima/abaixo, coluna à esquerda/direita, excluir linha, coluna ou tabela, mostrar bordas.
- Layout: colunas em pontos (encolhem juntas se não couberem), altura da linha da tabela pela célula mais alta, linhas da tabela não se partem entre páginas. Desenho na tela e no PDF (fundo, texto, bordas).
- Formatos: DOCX `w:tbl` (grade, larguras, bordas, `w:shd`, `gridSpan`), ODT `table:table` (colunas repetidas, células cobertas, bordas, fundo) e RTF como o Word (`\trowd`, `\cellx`, `\clcbpat`, `\intbl`, `\cell`, `\row`). Conferido com o LibreOffice (os nossos .docx e .odt com tabela) e em testes de ida e volta. Copiar ou converter em texto simples: Tab entre as células.

## Editor de Texto, etapa 4d: revisão ortográfica (2026-10-24)

- **`src/spell.rs` (biblioteca, reaproveitável por outros apps):** verificador próprio que lê os dicionários do sistema no formato Hunspell (`/usr/share/hunspell/pt_BR.aff` e `.dic`, os mesmos do LibreOffice e do Firefox). Entende sufixos e prefixos com condições, prefixo com sufixo, sufixos em duas camadas (afixo com continuação), palavras proibidas, maiúsculas e palavras com hífen. Sugestões: trocas do dicionário (REP), letras parecidas (MAP: acentos), uma letra a mais, a menos, trocada ou de lugar, e duas palavras juntas. Palavras aprendidas em `~/.local/share/mide/spelling-words.txt`. O dicionário de português (312 mil palavras, 26 mil regras) carrega em 0,25 s, em segundo plano; conferido com palavras certas e erradas ("metodolojia" → "metodologia", "intrudução" → "introdução").
- **No editor:** ondulado vermelho nas palavras erradas (a que está sendo digitada só depois de terminar), com cache por parágrafo. Clique direito: sugestões, Aprender ortografia, Ignorar, e Recortar/Copiar/Colar. Editar → Verificar ortografia ao digitar liga e desliga. Idioma pelo `LANG` (reserva em pt_BR e en_US).

## Editor de Texto, etapa 4e: redimensionar imagens (2026-10-24)

- **Formatar → Tamanho da imagem…** permite alterar a largura em centímetros e manter a proporção ligada por padrão; ao desligá-la, largura e altura podem ser ajustadas separadamente.
- O diálogo começa com as dimensões atuais, dá foco ao campo de largura e rejeita valores inválidos. Aplicar é uma única operação de desfazer. O alinhamento já usa os comandos de parágrafo do editor.
- `Image::resize_to_width` atualiza o tamanho lógico sem recomprimir a imagem original; os gravadores de RTF, DOCX, ODT e PDF usam essas dimensões.
- Testes: proporção e rejeição de largura inválida no modelo; diálogo inicial carrega largura, altura e proporção. `cargo test --bin mide-text`.

## Editor de Texto, etapa 4f: rolagem isolada nos popovers (2026-10-24)

- Com um popover aberto, a página de trás não recebe roda nem cliques; as listas internas dos popovers de fonte e tamanho continuam roláveis. Isso também impede que escolher uma cor mova o cursor do documento.
- Causa: a página processava a entrada antes do popover desenhar seu conteúdo. O editor agora bloqueia a interação de fundo até o popover fechar.
- Verificado com `cargo test --bin mide-text` (5 testes).

## Editor de Texto, etapa 4g: face da fonte na barra (2026-10-24)

- A barra ganhou o seletor Regular, Bold, Italic e Bold Italic, sincronizado com os controles B/I. Em janelas estreitas, as opções ficam dentro do seletor de família para a barra não crescer para fora da tela.
- As quatro faces usam os campos negrito/itálico que já são salvos em RTF, DOCX e ODT.
- Testes: `cargo test --bin mide-text` (6 testes), incluindo os nomes apresentados para cada combinação.

## Editor de Texto, etapa 4h: paradas de tabulação na régua (2026-10-24)

- A régua agora mostra triângulos de tabulação além dos marcadores de recuo existentes. Clique na escala cria uma parada; arrastar move em passos de 0,25 cm; arrastar para fora da régua remove. As alterações entram no histórico de desfazer.
- `Tab` avança para a próxima parada personalizada do parágrafo; se não houver outra à frente, continua usando o intervalo padrão de 36 pt. A régua de uma célula de tabela mostra e altera as paradas daquela célula.
- As posições são lidas e gravadas em RTF, DOCX e ODT. Testes de layout e ida e volta nos três formatos passaram.
- RTF conserva tabulações alinhadas à direita separadas das paradas esquerdas; assim o líder pontilhado do sumário não vira marcador de régua.
- Os triângulos das paradas de tabulação **personalizadas** apontam para a direita, deitados; os três marcadores de recuo continuam verticais. **Atenção:** as paradas padrão (a cada 36 pt) ainda não têm triângulo na régua; é o pendente 3 da etapa 4i.

## Editor de Texto, etapa 4i: correções visuais e pendências (2026-10-24, feito pelo Copilot)

Feito e conferido em quadro de teste (visual, sem teste automatizado):
- **Folha por cima da barra de título e das cápsulas:** `ui.paint_text` pinta na janela inteira, sem recorte; ao rolar, a página (e a sombra) cobria a barra de título e a de formatação. Em `Rich::view` (`src/bin/mide-text/rich.rs`) o desenho das páginas agora guarda as faixas de pixels acima e abaixo da área do documento e as restaura no fim.
- **Clique/rolagem atravessando popovers:** `blocked` em `Rich::view` inclui `self.pop.is_some()`; com um popover aberto a página não recebe clique, seleção nem roda (as listas de Fonte e Tamanho rolam pelo `scroll_area` delas).
- **Triângulos de tabulação personalizada** deitados (ver 4h). Um patch ruim colocou esse desenho dentro de `format_bar`; já corrigido, `cargo test --bin mide-text` passa.

Ainda **não feito** (pedido do usuário em 2026-10-24, a próxima coisa a fazer):
0. **Atualização (mesma sessão, depois de comparar com o TextEdit):** os itens 1, 2 e 3 abaixo foram feitos e conferidos em quadro de teste (`/tmp/ed-shot*/frame.png`, 1366x768): ícone do Realce e da cor do texto desenhados à mão (sem o texto de reserva); campo `#RRGGBB` com amostra e botão Aplicar nos popovers de Cor do texto e Realce (`parse_hex_color`/`hex_color`, abre já com a cor atual); régua com 34 pt (números em cima, triângulos deitados embaixo), triângulos cinza nas paradas padrão (`default_tab_stops`) e azuis nas personalizadas, recuo direito para cima apontando para baixo como no Mac (o clique segue o desenho); barra de formatação com larguras pela janela (`bar_widths`) e lista dos popovers limitada ao espaço abaixo do botão (`popover_list_height`, é o que cortava a barra de rolagem). 9 testes de `cargo test --bin mide-text` passam.
   **Ainda aberto:** (a) na janela mínima (560 pt) a barra de formatação continua não cabendo: `bar_widths` só foi conferida em quadro a 1040 pt, e o teste de unidade cobre 920 pt ou mais; (b) o Aplicar do popover fica rente à borda direita; (c) arrastar os triângulos com o mouse e a cor `#RRGGBB` aplicada ao texto não foram exercitados (só vistos desenhados); (d) o desenho do recuo direito e das paradas padrão não tem teste, só quadro; (e) o usuário quer o visual do TextEdit: comparar de novo lado a lado; (f) `src/kit/chooser.rs` tem um aviso `pretty_dir` sem uso que não é desta sessão.

Lista original dos pendências:
1. **Ícone do Realce bugado:** o botão usa `toolbar_buttons` com texto de reserva `▌`, que é desenhado e, por cima, o retângulo colorido pintado em `format_bar`. Trocar o reserva por vazio e desenhar o ícone inteiro à mão (amostra do realce, ou quadriculado cinza sem realce). A letra `A` da cor do texto usa o mesmo mecanismo: conferir.
2. **Cor personalizada** nos popovers de Cor do texto e de Realce (`Pop::Color`, `Pop::Highlight`, em `Rich::popovers`): hoje só há as amostras fixas `COLORS` e `HIGHLIGHTS`. Pôr um campo `#rrggbb` com amostra (e/ou três deslizantes RGB) e aplicar com `chosen_color`/`chosen_highlight`. O mide-ui não tem seletor de cor; `parse_hex` existe só como função privada em `richtext/docx.rs`.
3. **Triângulos clássicos do TextEdit na régua:** o Mac mostra triângulos pretos deitados em cada parada padrão (a cada 0,5 in, aqui `TAB_STOP` = 36 pt em `richtext/layout.rs`) ao longo da régua. Hoje só desenhamos as personalizadas. Desenhar as padrão, em cinza escuro, depois do recuo da primeira linha e antes da margem direita; no Mac definir uma parada remove as padrão à esquerda dela (alinhar `tab_width` a isso). Imagens de referência: as que o usuário mandou (TextEdit com régua e triângulos).

Estado: nada foi commitado. O diff mistura as refatorações de `let` encadeado que o Claude deixou (docx, layout, mod, odt, pdf, render, spell, main) com as alterações desta sessão (imagem, face da fonte, tabulações, popovers, recorte).

## Configurações → Itens de início e som de mover para a Lixeira (2026-10-25)

- **Itens de início** (pedido do usuário): nova seção nas Configurações, como os "Itens de início de sessão" do Mac. Lista os apps e comandos que abrem ao entrar na sessão, no formato do freedesktop (`~/.config/autostart` e as pastas do sistema), cada um com interruptor. Os do sistema não são apagados: desligar grava no usuário um arquivo com o mesmo nome e `Hidden=true` (o compositor já dá preferência ao do usuário). "Adicionar app…" busca entre os apps instalados; "Adicionar comando…" pede nome e comando, com "Escolher…" abrindo o seletor de arquivos do mide para um script. Itens do usuário têm o botão de remover. Biblioteca em `src/login_items.rs`, com teste; `mide-settings login-items` abre direto na seção.
- **Som de papel amassado** ao mover para a Lixeira (pedido do usuário): evento `trash-move` no tema de sons, tocado pela função comum `places::move_to_trash` (vale no Arquivos, área de trabalho, dock, captura, launcher). Esvaziar a Lixeira ganhou um amassado mais longo com a queda no fundo. O mesmo som pedido várias vezes em 300 ms toca uma vez só (mover 20 arquivos não dá 20 sons). Nos testes de unidade nenhum som toca.
