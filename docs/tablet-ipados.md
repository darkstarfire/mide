# Modo tablet como o iPadOS

Pedido do usuário (2026-11-01): no modo tablet, o mide se comporta como o iPadOS, sem
o usuário precisar apontar cada detalhe. Este documento é a referência: o comportamento
do iPadOS (com as fontes), o que o mide já faz e o que falta, em ordem de prioridade.
Só inspiração de comportamento: nada de código, imagens ou ícones da Apple.

## Fontes

- Apple, *iPad User Guide* — gestos: <https://support.apple.com/en-au/guide/ipad/ipadab6772b8/ios>
- Apple, *iPad User Guide* — trocar de app: <https://support.apple.com/en-vn/guide/ipad/switch-between-apps-ipad619935ea/14.0/ipados>
- Apple, *Multitask on iPad with iPadOS 26*: <https://support.apple.com/en-ie/125309>
- iPadOS 26 (visão geral, janelas e barra de menus): <https://en.wikipedia.org/wiki/IPadOS_26>
- 9to5Mac, dock que aparece e some no iPadOS 26: <https://9to5mac.com/2026/02/12/ipados-26-multitasking-got-better-when-i-changed-this-one-setting/>

Pontos marcados "(conferir)" vêm da experiência geral com o iPadOS e não foram
confirmados numa fonte; conferir antes de tratar como regra.

## Comportamento do iPadOS × situação no mide

Legenda: **feito**, **parcial**, **falta**.

### Gestos da borda de baixo (o mais importante)

| iPadOS | mide |
|---|---|
| Dentro de um app, deslizar para cima um pouco a partir da borda de baixo mostra a Dock por cima do app. | **feito** (menos de 90 pt: a dock fica à mostra por 4 s; `touch.rs`, `dock_reveal_until`) |
| Deslizar para cima até o meio e soltar vai para a tela inicial (no mide: o launcher). | **feito** (borda de baixo → launcher) |
| Deslizar para cima e parar no meio abre o seletor de apps (cartões das janelas). | **feito** (borda de baixo e parar → visão das janelas) |
| Deslizar na horizontal ao longo da borda de baixo troca para o app anterior/seguinte. | **falta** |

### Seletor de apps (visão das janelas)

| iPadOS | mide |
|---|---|
| Tocar num cartão abre o app. | **feito** |
| Deslizar um cartão para cima fecha o app (conferir). | **feito** (90 pt para cima; `overview::release`) |
| Botão de fechar no cartão (pedido do usuário, canto superior direito). | **feito** (× em cada janela do mosaico) |

### Gestos de vários dedos (na tela e no trackpad)

| iPadOS | mide |
|---|---|
| Pinçar com quatro ou cinco dedos vai para a tela inicial. | **falta** (pinça) |
| Quatro dedos para cima abrem o seletor de apps. | **feito** (tela e touchpad) |
| Quatro dedos para os lados trocam de app. | **feito** no touchpad; **parcial** na tela (o código é o mesmo, falta conferir) |
| Três dedos para cima/baixo (pedido do usuário: launcher abre/fecha). | **feito** (tela e touchpad) |

### Bordas de cima

| iPadOS | mide |
|---|---|
| Canto superior direito para baixo: Central de Controle. | **feito** |
| Canto superior esquerdo/meio para baixo: Central de Notificações. | **feito** (canto esquerdo) |
| iPadOS 26: dentro de um app, deslizar do meio de cima para baixo mostra a barra de menus do app. | **falta** |

### Dock

| iPadOS | mide |
|---|---|
| Sem ampliação dos ícones (toque não tem "passar por cima"). | **feito** (`mide-dock`, `TABLET`) |
| Ícones grandes para o dedo. | **feito** (`[tablet] large_targets`) |
| Some quando o teclado na tela aparece (o teclado fica na frente de tudo). | **feito** (consulta `osk_open` ao compositor) |
| Escondida enquanto um app ocupa a tela; aparece com o deslizar curto de baixo. | **feito** com o ocultar automático ligado |

### Apps e janelas

| iPadOS | mide |
|---|---|
| Modo "Apps em tela cheia" (padrão): todo app ocupa a tela, inclusive os que abrem depois. | **feito** (ao entrar no modo e ao abrir, `wm.rs`) |
| Sem barra de título; os controles da janela aparecem pela barra de menus (iPadOS 26). | **falta** |

### Toque, ponteiro e teclado

| iPadOS | mide |
|---|---|
| O ponteiro some quando se toca a tela e volta ao mexer o trackpad/mouse (conferir). | **feito** |
| Tocar e segurar abre o menu de contexto. | **feito** (segurar = botão direito) |
| Tocar num campo de texto abre o teclado na tela. | **feito** (`mide-keyboard`) |
| Girar em todas as orientações, inclusive retrato de cabeça para baixo. | **feito** (mapeamento do Chromebook corrigido em 2026-11-01) |

## Ordem de implementação

1. ~~Seletor de apps: botão de fechar e deslizar o cartão para cima.~~ (2026-11-01)
2. ~~Dock no modo tablet: sem ampliação e escondida com o teclado na tela.~~ (2026-11-01)
3. ~~Borda de baixo: deslizar curto mostra a Dock por cima do app.~~ (2026-11-01)
4. ~~Apps em tela cheia também para os que abrem depois.~~ (já existia)
5. Deslizar na horizontal na borda de baixo troca de app.
6. Pinçar com quatro ou cinco dedos vai ao launcher.
7. Sem barra de título no modo tablet; controles e menus do app ao deslizar do meio de cima.

Cada item: testes de unidade quando der, seção da bateria (`tests/ui/`) quando o
roteiro do compositor conseguir simular o toque, e o teste do usuário no Chromebook.
