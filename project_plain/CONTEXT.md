# Projeto: mide (Mac Inspired Desktop Environment)

--
## fluxo de trabalho do LLM(agente de IA que vai assumir o projeto)
deve-se seguir nesta ordem
1. ler o CONTEXT.md para saber o que fazer
2. ler RECURSES.md para saber o que precisa de recursos
3. ler INSTALATION.md para saber como deve ser instalado o DE depois de pronto
4. deve registrar no IMPLEMENTED.md em linguagem natural humana o que foi feito
5. deve adicionar anotações quando necessário em ANNOTATIONS_OF_AGENT.md de coisas relevantes durante o processo de construção do projeto
6. antes de começar cada passo a ser implementado no projeto, revisar todos arquivos nessa ordem antes de começar o próximo passo, cada passo deve estar alinhado com como foi estruturado e definidos nos arquivos md dentro da pasta /project_plain na raiz do projeto
--

---
## Repositório e origem
- Repositório público: https://github.com/fabioaacarneiro/mide (branch `master`). Versionar ao fim de cada passo, com os testes passando.
- O mide é criado do zero: sem engenharia reversa de nada da Apple/macOS e sem usar código, imagens, ícones ou fontes deles. O macOS é só inspiração visual; o mide não é concorrente nem cópia. Manter isso no README e em qualquer texto público.
- Testes e prints na resolução 1366x768 (a mais comum no Brasil), além dos 800x600 da bateria de interface.
- **Tudo configurável pela interface** (decisão do usuário, 2026-10-19): o usuário nunca precisa editar arquivo de configuração. Toda opção (atalhos inclusive) tem tela nas Configurações ou no próprio app, e nenhum texto da interface manda "editar o arquivo". Arquivo de configuração é detalhe interno: o mide pode usá-lo se for necessário, alimentado pela interface, sem o usuário precisar saber dele.
- **Sem sessão X11** (decisão do usuário, 2026-10-19): o mide é um compositor Wayland e não terá backend nem modo para rodar como sessão X11, para não ter de reescrever e manter tudo em dobro. **Apps X11 dentro do mide, pelo XWayland, são desejados** (A14). Rodar o backend winit dentro de uma sessão X11 (desenvolvimento) não é suporte a X11 e não exige código nosso.
- **Arquitetura do compositor (camadas):** o Smithay cuida da base (protocolos, backends, renderizador) e não é reimplementado; o código do mide fica separado dele: cena em `render.rs`, política de janelas em `wm.rs`, entrada em `input.rs`. Backends (`backend_winit.rs`, futuro DRM) só criam a saída, tratam eventos e apresentam o quadro da cena.
---

---
## Propostas
ter um visual moderno, limpo, inspirado no macos e construído no wayland, performático com os recursos necessários para ser um ambiente desktop completo e ao mesmo tempo minimalista para uso geral

**Foco atual (decisão do usuário, 2026-10-19):** terminar o mide como desktop completo para qualquer usuário. Ferramentas de público específico (como as de desenvolvedor) ficam para depois, feitas com o framework de apps (mide-ui) como produtos à parte. Prioridade sugerida: A13 (rodar sem outro desktop por baixo), A14 (XWayland), melhorias do mide-ui que servem a todos os apps (D1), depois o que falta das fases B e C.
---

---
## caracteristicas visuais
- visual inspirado no mac contendo:
    - barra superior com menu igual o do mac 
        - a esquerda que você clica tem as opções que o mac tem como tbm abrir uma janela para especificações do sistema, versão, hardware, e demais informações
        - as janelas devem ter o menu delas nessa barra superior como no mac ao invés de mostrar nas janelas como é no gnome
        - a direita deve ter os recursos inspirados no mac, mesmo recursos, basta usar ele como referencia
    - uma dock em baixo que tem recursos para fixar um app(lançador.desktop), reorganizar com clicar e arrastar, desfixar
    - sistema de notificação como o dunst, mas integrado ao visual do desktop environment
    - na dock precia de app padrões da esquerda para direita:
        - app launcher
            - campo de filtro por texto
            - abre em tela cheia com a mesma visão de apps que o mac
            - posso agrupar em "pastas" como o mac/gnome
            - deve ter um grupo para ferramentas do sistema
            - deve ter grupo para escritório
            - deve ser inteligente para que organize em grupos pela categoria do app, como menus normais no linux faz, mas com visual mac
        - app para gerenciador de arquivos como o finder
        - app de configuração que abre o menu de configurações do sistema
            - as configurações do sistema deve ter um visual simples, e trazer recursos necessários para o usuário:
                - configuração do monitor
                - configuração do mouse
                - configuração dos efeitos visuais
                - configuração do papel de parede
                - configuração do touchpad
                - configuração do teclado
                    - precisa ter atalhos padrões para:
                        - o desktop enviromnt
                        - janelas
                        - apps
                        - precisa poder editar os atalhos anteriores
                        - precisa poder criar atalhos personalizados:
                            - campo de nome
                            - campo de comando podendo ser um binário ou script
                            - atalho do teclado
                - configuração do bluetooth
                - configuração do wifi
                - configuração de apps default
                - configuração de tema:
                    - claro
                    - escuro
                    - variação de cores iguais ao mac
                - localidade:
                    - linguagem do sistema
                    - localização do sistema
                - configuração do que vai iniciar no sistema:
                    - campo de nome
                    - campo de comando podendo ser um binário ou script
                    - atalho do teclado
        - ícone de lixeira
            - deve mostrar opções como limpar lixeira ou abrir em gerenciador de arquivos
            - ao clicar deve mostrar a mesma visualização que o icone das ultimas coisas baixadas
        - ícone das últimas coisas baixadas na pasta download
            - quando clicado, precisa apareccer uma lista dos itens baixados, como no mac
---

---
## Efeitos visuais
- movimentação suave dos elementos visuais, corrigindo screenthearing
- efeito lampada ao minimizar e restaurar janelas minimizadas
- transparencia
- blur na transparencia
---

---
## Configurações de caracteristicas visuais
- pode ter opções de desligar um ou mais efeitos visuais em configurações
- precisa ter um tray no lado direito da barra superior que podemos adicionar ícones que são serviços que sobem com o sistema como ícone de volume, e outros apps que naturalmente aparecem ali
- além dos ícons de apps padrões da dock, podemos adicionar mais como no dock do gnome e no mac
- trocar o papel de parede do desktop
- poder adicionar ícones e criar pastas no desktop
---

---
## App padrões que devem vir com o DE
- gerenciador de arquivos estilo finder, leve e com as funcionalidades necessárias
- lançador de aplicativos em tela cheia, com gerenciamento de grupos de apps
- configurações do sistema
- dock de aplicativos
- barra superior
- apps que compõem o ambiente desktop
- terminal nativo e próprio do mide, inspirado no iTerm2, com menus gráficos para configurar tudo (sem editar arquivo à mão): abas, divisão de painéis, perfis (cores, fonte, transparência/vidro), busca no histórico. Substitui o `foot` como terminal padrão (`ctrl+shift+t`)
- editor de texto nativo e mínimo: abrir arquivos, fazer ajustes simples e salvar (o editor de programação completo é um produto à parte, ver "Produtos à parte")
---

---
## Plano em etapas micro

Cada micro etapa é pequena, verificável e registrada em `IMPLEMENTED.md` ao terminar. Marque o status conforme avança: [x] feito, [ ] pendente.

Decisão de arquitetura: o compositor (`mide`) cuida de Wayland, janelas, input e efeitos. Barra, dock, launcher, configurações, arquivos e notificações são clientes separados, que usam o protocolo layer-shell. Isso segue o que está em `RECURSES.md` e mantém cada parte testável sozinha.

### Fase A: compositor (núcleo)
- [x] A0. Esqueleto do projeto Rust
- [x] A1. Servidor Wayland aceita clientes no socket `mide-0`
- [x] A2. Renderização em janela winit com `Space` e frame callbacks
- [x] A3. Teclado: `wl_keyboard`, keymap com xkbcommon, envio de teclas ao foco
- [x] A4. Ponteiro: movimento, botões e rolagem para a superfície sob o cursor
- [x] A5. Foco por clique: traz a janela para frente e marca como ativa
- [x] A6. Mover janelas arrastando (provisório, sem barra de título ainda)
- [x] A7. (feito: barra com botões, título em texto, hover, sombra, cantos, redimensionar pela borda, cursor) Decorações de servidor: barra de título estilo mac com botões
- [x] A8. Fechar, minimizar e restaurar janelas (pela barra, pelo botão do app, pelo atalho e pela dock), com animação
- [~] A9. (feito: motor de atalhos e editor de atalhos nas Configurações → Teclado — mudar a combinação gravando as teclas apertadas, criar (ação do mide ou abrir um app), remover e restaurar os padrões, com aviso de conflito, valendo na hora; falta o conjunto completo de atalhos padrão) Atalhos globais lidos de `toml`, com os padrões de desktop, janelas e apps
- [~] A10. (parcial: camadas Top e Overlay desenhadas e com ponteiro, zona exclusiva da barra respeitada ao abrir e maximizar janelas; camadas de fundo também, desde o B10) Layer-shell (`wlr-layer-shell`), para barra, dock e launcher
- [~] A11. (feito: sombra combinada de todas as janelas, cantos, animações de maximizar e restaurar, efeito lâmpada ao minimizar e restaurar, e `[effects]` no config com `shadows`, `rounded_corners`, `animations` e `lamp`; feito também o desfoque da dock e do fundo do launcher, com interruptores, e o vidro líquido na dock (refração, reflexo e tom, com desfoque refeito só quando o fundo muda); feito também o vidro em barra superior, menus e notificações, por regiões de vidro pedidas pelas camadas (`glass` no canal de controle); feito `ui.glass_panel` no mide-ui (em camadas); feito vidro em janelas comuns por regiões (barra lateral do Arquivos), papel de parede trocando com esmaecer e fechar janela com transição; falta transparência de janelas inteiras) Efeitos: animação suave, transparência, blur e efeito lâmpada, com flag para desligar cada um
- [x] A12. (feito: `[desktop] wallpaper` em PNG ou JPEG cobrindo a tela, gradiente próprio por padrão, sombras das janelas sobre ele, escolha nas Configurações com imagens do usuário) Papel de parede, lido de configuração
- [ ] A13. Backend DRM/udev com libseat, para rodar sem outro compositor por baixo
- [ ] A14. Apps X11 dentro do mide pelo XWayland. Preferência: `xwayland-satellite` (como o niri), que roda o XWayland como cliente Wayland comum e mantém o compositor sem código X11; alternativa: o recurso `xwayland` do Smithay (gerenciador de janelas X11 dentro do compositor), se o satellite não bastar. Validar com um app só-X11 (ex.: IDE da JetBrains sem Wayland ativado)

### Fase B: shell (clientes)
- [~] B1. (feito: barra `mide-bar` no topo com zona exclusiva, marca do mide com menu do sistema, nome do app focado, relógio, bateria e janela "Sobre este computador"; faltam Configurações (depende de B7), reiniciar/desligar e os menus do app de B2) Barra superior como cliente layer-shell, com menu Apple e relógio
- [~] B2. (feito: menus do app em foco no estilo do macOS para qualquer app — nome do app, Arquivo, Editar, Janela — com ações pelo compositor; falta ler os menus nativos dos apps por dbusmenu) Menu do app focado na barra (dbusmenu via `zbus`)
- [~] B3. (feito: ícones e painéis de som, Wi-Fi, Bluetooth e bateria, e calendário no relógio; falta o tray de apps de terceiros via StatusNotifierWatcher) Tray do lado direito (StatusNotifierWatcher via `zbus`) com volume, Wi-Fi, Bluetooth e bateria
- [x] B4. Dock: ícones de `.desktop` (PNG e SVG), abrir, focar app aberto, restaurar minimizada, ponto de app aberto, apps abertos não fixados depois de um separador, fixar e desafixar (clique direito) e arrastar para reordenar
- [x] B5. (feito: Downloads com os 8 itens mais recentes e Lixeira com menu e confirmação para esvaziar, no fim da dock) Lixeira e Downloads na dock, com menus e lista de itens recentes
- [x] B6. (feito: `mide-launcher` em tela cheia no estilo do Launchpad, busca sem acentos, pastas com miniaturas, arrastar para criar pastas e reordenar, renomear pasta, páginas por roda/botão do meio, dock e barra por cima; abre e fecha com `ctrl+shift+space` ou pelo ícone na dock) Launcher em tela cheia, com busca e grupos por categoria
- [~] B7. (feito: `mide-settings` no mide-ui, com busca, Wi-Fi, Bluetooth, Som, Papel de parede, Dock (tamanho dos ícones e ocultar automaticamente), Efeitos, Teclado (teclas no estilo Mac e atalhos) e Sobre, valendo na hora; faltam monitor, mouse, touchpad, teclado (layout), tema, localidade e apps padrão) Configurações do sistema: monitor, mouse, touchpad, teclado, tema, localidade, Wi-Fi, Bluetooth e apps padrão
- [~] B8. (feito: app `mide-files` com favoritos, Lixeira, histórico, ícones e lista em árvore no estilo do Finder (setinhas, linhas alternadas, colunas e ordenação), miniaturas, busca, menus de contexto completos, nova pasta, novo arquivo, renomear, duplicar, copiar/colar, compactar e extrair .zip, mover para a Lixeira, restaurar e teclado; faltam seleção múltipla, arrastar e soltar e "abrir com") Gerenciador de arquivos estilo Finder
- [x] B9. (feito: servidor `mide-notifications` com banners no estilo do macOS, ações, expiração, fechar, Não perturbe e a central de notificações com o histórico, no painel do relógio) Notificações: servidor `org.freedesktop.Notifications` com visual do DE
- [x] B10. (feito: `mide-desktop` com os itens da pasta Área de trabalho em colunas à direita, abrir, menus de contexto, criar e renomear com diálogo, Lixeira e teclado; pendentes: arrastar ícones e miniaturas) Ícones e pastas na área de trabalho, com menu de contexto
- [x] B11. (feito: `mide-search` com ctrl+space, apps, Configurações, arquivos e contas, sobre o vidro líquido; pendentes: prévia, grupos e histórico) Buscador de apps nativo (estilo Spotlight), recriado no mide no lugar do buscador de terceiros que está instalado: barra flutuante com o vidro líquido da dock (mesmo componente/shader), busca de apps, arquivos e ações, abrindo por atalho, para o visual ficar integrado ao DE
- [x] B12. (feito: seção "Barra superior" nas Configurações com engrenagem, som, Wi-Fi, Bluetooth, bateria e central de controle, gravando em `[bar]` e valendo em meio segundo; falta incluir os ícones do tray de terceiros quando o StatusNotifierWatcher (B3) existir) Configurações → Barra superior: escolher quais itens aparecem na barra (som, Wi-Fi, Bluetooth, bateria, central de controle, engrenagem e os ícones de apps do tray)

### Fase D: plataforma para apps
- [~] D1. (feito: `mide::kit` com componentes, layout, tema, janela e eventos, exemplo `mide-ui-demo` e guia `docs/mide-ui.md`; menu de contexto, grade, ladrilhos, lista, barra de ferramentas, duplo clique e foco com seleção; camadas (`Window::overlay`), `glass_panel`, linhas de resultado e ícones assíncronos; faltam tema escuro, campo de texto completo (cursor no meio, colar, seleção, teclas mortas e composição como `´`+`a`=`á`), área de transferência (copiar e colar entre apps), desenho só do que é visível e redesenho parcial com cache de glifos (o desenho é na CPU, com tiny-skia: necessário para listas longas e máquinas modestas), diálogos, seletor de arquivos e animações como componentes; as Configurações já foram passadas para ele) Framework mide-ui: criar apps com a interface nativa do mide
- [ ] D2. Apps, widgets e extensões com o mide-ui: a comunidade cria apps e widgets com o framework, e a central de controle da barra (Wi-Fi, Bluetooth, Não perturbe, brilho, som) pode ser estendida com novos itens

### Fase E: apps nativos (depois)
Apps do dia a dia feitos com o mide-ui, com o visual e o comportamento do resto do DE. Ficam para depois das fases A a D.
- [ ] E1. Terminal nativo (`mide-terminal`), inspirado no iTerm2: emulação de terminal sobre PTY, abas, divisão de painéis, perfis (cores, fonte, transparência/vidro), busca no histórico e menus gráficos de configuração (as preferências do terminal ficam em telas, não em arquivo). Vira o terminal padrão do `ctrl+shift+t` no lugar do `foot`. A decidir no início: escrever o interpretador de sequências de escape do zero ou usar um crate de base (ex.: `vte`), mantendo o terminal e a interface próprios
- [ ] E2. Editor de texto mínimo (`mide-text`): abrir, editar e salvar arquivos de texto, desfazer/refazer, buscar e substituir, aviso de alterações não salvas, abrir pelo Arquivos ("Abrir com")

### Planos futuros (fora do foco atual)
- Uma ISO de instalação de um Linux com o mide.
- Na instalação (ou depois), a pessoa escolhe apps extras do ecossistema do mide para instalar direto do GitHub (ex.: o editor de código P1 e outras ferramentas de público específico).

### Produtos à parte (por último, fora do mide)
Não fazem parte do DE nem do plano acima: são produtos próprios, feitos depois de o mide estar completo.
- [ ] P1. Editor de código completo, em Rust, nativo do mide (mide-ui). Decisões já tomadas: núcleo sem interface (texto em `ropey`, desfazer, seleções e múltiplos cursores, `tree-sitter`, cliente LSP próprio) testável só com testes unitários, separado do app; marcos com metas medidas: M1 abrir/editar/salvar com segurança, M2 tree-sitter, árvore de pastas, abrir por nome e busca no projeto, M3 LSP (diagnósticos, completar, ir para definição, formatar), M4 renomear, ações de código, Git na margem e terminal embutido (E1), M5 opcional depuração (DAP). Sem sistema de extensões no começo; Rust primeiro. Atenção ao LSP: posições em UTF-16 por padrão (negociar UTF-8), respostas atrasadas descartadas, servidores detectados no sistema e não embutidos. Pré-requisito: mide-ui com campo de texto completo, composição de teclas mortas, área de transferência, desenho só das linhas visíveis e cache de glifos

### Fase C: sistema
- [ ] C1. Bloqueio de tela (`ext-session-lock-v1`) e inatividade (`ext-idle-notify-v1`)
- [ ] C2. Sessão de login (entrada `.desktop` para `greetd`) e autostart
- [ ] C3. Instalador em bash descrito em `INSTALATION.md`
- [~] C4. (feito: 102 testes unitários, tests/smoke.sh, tests/ui.sh com 153 verificações de interface (800x600) e tests/res1366.sh com 8 em 1366x768; feito também `tests/perf.sh` com tempos de quadro e de resposta por cenário; falta medir o tempo de GPU) Testes automatizados e medição de desempenho (frame time e GPU)
---

---
## comportamentos mobile
- se virar o note, ele deve conseguir virar a visibilidade do DE para ajustar, como o gnome faz
- deve ser capaz de reconhecer toque na tela touchscreen se a tela for touch
- deve abrir teclado virtual se o notebook for conversivel e estiver no modo tablet
---