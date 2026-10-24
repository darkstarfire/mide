# Mapa do visual Tahoe no mide-ui

**Regra forte (decisão do usuário, 2026-10-24).** O mide segue o visual e o
comportamento do macOS mais recente, o macOS 26 Tahoe, com o material "Liquid Glass".
Este documento é o mapa do que o mide-ui precisa ter para qualquer app do mide parecer
um app do Tahoe: os valores de base (tokens), a anatomia das janelas, o catálogo
completo de componentes (com o que já existe, o que ajustar e o que criar) e os
padrões de tela. **Em conflito com o `docs/design.md`, vale este documento**; o
`design.md` continua valendo para efeitos, movimento e textos.

Só inspiração: nenhuma imagem, ícone, fonte ou código da Apple. As medidas abaixo são
do mide, escolhidas para dar o mesmo efeito visual, e ficam no código
(`src/kit/theme.rs`, `src/decorations.rs`, `src/motion.rs`). Referências do usuário: o
Human Interface Guidelines (developer.apple.com/design/human-interface-guidelines/designing-for-macos),
o artigo do Eclectic Light sobre o Tahoe
(eclecticlight.co/2026/05/31/last-week-on-my-mac-razzle-and-dazzle/) e as capturas de
tela do Finder e dos Ajustes do Sistema que ele mandou.

Unidade: **pt** (ponto lógico do mide-ui; em escala 1, um pixel).

---

## 1. Ideias do Tahoe que guiam tudo

1. **Duas camadas: conteúdo e navegação.** O conteúdo (arquivos, texto, fotos,
   opções) é opaco e ocupa a janela inteira, de borda a borda. Por cima dele flutua a
   camada de navegação e controles: barra lateral, grupos de botões da barra de
   ferramentas, campo de busca, abas, menus, popovers. **O vidro é só da camada de
   navegação**, nunca do conteúdo.
2. **Tudo flutua e é arredondado.** A barra lateral não encosta nas bordas: é um painel
   de vidro solto dentro da janela. Botões da barra de ferramentas ficam em cápsulas.
   Não há linhas separando a barra de ferramentas do conteúdo: o conteúdo passa por
   baixo e some num desfoque suave (efeito de borda de rolagem).
3. **Cantos concêntricos.** Um elemento dentro de outro tem raio = raio de fora menos
   a distância até a borda. A janela de raio 24 com a barra lateral a 8 pt da borda
   dá à barra lateral raio 16, e a seleção dentro dela, a 8 pt, raio 8.
4. **Cápsulas.** Botões, campos de busca, controles segmentados, selos e seleções de
   barra lateral têm a forma de cápsula ou cantos bem grandes. Quadrado de canto
   pequeno quase não existe mais.
5. **Vidro que reage.** O vidro tem borda de luz (reflexo fino em cima e à esquerda),
   fica mais claro sob o ponteiro e "brilha" onde é apertado. Controles que mudam de
   estado deslizam (a pílula do controle segmentado anda até a opção nova).
6. **Mais ar.** Controles maiores, linhas mais altas, margens maiores que no Big Sur.
7. **Ícones com a cor de destaque.** Na barra lateral do Finder e nos menus, os
   símbolos ganham cor; nos menus, cada item comum tem um símbolo à esquerda.

---

## 2. Tokens (valores de base)

`código: src/kit/theme.rs` (o `Theme` ganha todos estes campos)

### 2.1 Raios

| Token | Valor | Onde |
|---|---|---|
| `radius_window` | **24** | janela com barra lateral ou barra de ferramentas |
| `radius_window_plain` | **16** | janela simples (sem barra de ferramentas: diálogos pequenos, Calculadora) |
| `radius_sidebar` | **16** | barra lateral flutuante (24 − 8 de recuo) |
| `radius_panel` | **18** | popover, menu suspenso grande, Visualização Rápida |
| `radius_menu` | **12** | menus (contexto, barra superior, listas suspensas) |
| `radius_card` | **14** | cartões dos Ajustes, grupos de opções, painel de prévia |
| `radius_alert` | **24** | alertas e diálogos |
| `radius_item` | **8** | seleção dentro de menus e da barra lateral, linhas de lista |
| `radius_tile` | **10** | destaque de um ícone selecionado na grade |
| cápsula | altura ÷ 2 | botões, campos de busca, segmentado, selos, etiquetas |

### 2.2 Espaço e tamanhos

| Token | Valor | Onde |
|---|---|---|
| grade | 4 | tudo é múltiplo de 4 |
| `inset_float` | **8** | recuo da barra lateral e de painéis flutuantes até a borda da janela |
| `padding` | **20** | margem da página de conteúdo |
| `spacing` | **8** | entre controles numa linha; 12 entre blocos |
| `toolbar_height` | **52** | barra de ferramentas (sem linha embaixo) |
| `control_height` | mini 16, pequeno 22, **normal 28**, grande 34, extra 40 | botões, campos, segmentado, listas suspensas |
| `toolbar_item` | **36** de altura | cápsulas da barra de ferramentas (ícone 18) |
| `row_height` | **28** barra lateral, **30** lista, **44** linha de cartão dos Ajustes | |
| `sidebar_width` | **220** (mín. 180, máx. 320) | arrastável |
| `inspector_width` | **270** | painel à direita (prévia, inspetor) |
| luzes da janela | diâmetro **14**, centro a centro **20**, primeira a **20** da borda esquerda da barra lateral e **20** do topo da janela | |
| ícone de barra lateral | **18** | símbolo na cor de destaque (Finder) ou ícone colorido em quadrado de 22 com raio 6 (Ajustes) |

### 2.3 Tipografia

Fonte sem serifa do sistema (fontconfig); o mide não traz fonte da Apple.

| Estilo | Tamanho / peso | Uso |
|---|---|---|
| `large_title` | 26 bold | título grande de página (Ajustes) |
| `title1` | 22 semibold | |
| `title2` | 17 semibold | título de seção, título de alerta |
| `title3` | 15 semibold | título da janela na barra de ferramentas |
| `headline` | 13 semibold | rótulos de destaque, títulos de grupo |
| `body` | **13** regular | texto e controles |
| `callout` | 12 | explicações |
| `subhead` | 11 semibold, secundário | títulos de seção da barra lateral ("Favoritos") |
| `footnote` | 10 | barra de status, legendas |

### 2.4 Cores (papéis)

Todas por papel no `Theme` (claro e escuro); app nenhum escreve cor fixa.

| Papel | Claro | Escuro |
|---|---|---|
| fundo do conteúdo | `#FFFFFF` (listas, Finder) / `#F5F5F7` (Ajustes) | `#1E1E1E` / `#232325` |
| texto: 1º, 2º, 3º, 4º | tinta a 85%, 50%, 26%, 10% | |
| linha divisória | tinta a 10% | |
| preenchimento de controle | tinta a 6% (normal), 10% (sob o ponteiro), 14% (apertado) | |
| destaque | escolhido em Aparência: azul `#0A84FF`-ish, roxo, rosa, vermelho, laranja, amarelo, verde, grafite | |
| seleção com foco | destaque | |
| seleção sem foco | tinta a 10% | |
| perigo | vermelho do sistema | |
| etiquetas | vermelho, laranja, amarelo, verde, azul, roxo, cinza | |

### 2.5 Materiais (vidro)

| Material | Onde | Receita |
|---|---|---|
| `glass_regular` | barra lateral, cápsulas da barra de ferramentas, menus, popovers, alertas | desfoque do que está atrás (compositor) + véu claro 70% (escuro 60%) + borda de luz 1 pt (branco 50% em cima, 15% embaixo) + sombra suave |
| `glass_clear` | controles sobre fotos e vídeo (Visualização Rápida, galeria) | desfoque leve, véu 25%, borda de luz mais forte |
| `scroll_edge` | topo do conteúdo sob a barra de ferramentas, base sob a barra de status | o conteúdo some num degradê para o fundo em 52 pt (com desfoque leve, feito no próprio app) |
| opaco | vidro desligado em Efeitos | véu 96–100%, mesma forma |

Dentro de uma janela, o vidro mostra o **papel de parede e as janelas de trás** (o
compositor desfoca a região: `app:<app_id>` no canal de controle) e, sob as cápsulas
da barra de ferramentas, **o próprio conteúdo** que passa por baixo.

### 2.6 Sombras

| Token | Valor |
|---|---|
| janela em foco | raio 40, 16 para baixo, preto 32% (+ contorno 1 pt preto 12%) |
| janela sem foco | raio 24, 8 para baixo, preto 16% |
| menu, popover | raio 24, 8 para baixo, preto 22% |
| cápsula de vidro, barra lateral | raio 8, 2 para baixo, preto 8% |

### 2.7 Movimento

`código: src/motion.rs`. Molas curtas: resposta 0,35 s, amortecimento 0,85.

- apertar: o controle encolhe para 97% e o vidro clareia no ponto do clique;
- seleção do segmentado e de abas: a pílula desliza (mola);
- menus e popovers: nascem de 96% com esmaecer em 150 ms, a partir do ponto de origem;
- barra lateral escondendo: desliza para a esquerda; o conteúdo cresce junto;
- tudo com interruptor em Configurações → Efeitos (regra do mide).

### 2.8 Foco do teclado

Anel na cor de destaque a 50%, 3 pt, colado à forma do controle (cápsula segue
cápsula). Tab percorre os controles; Espaço aperta; setas mudam segmentado e listas.

---

## 3. Anatomia da janela

```
╭──────────────────────────────────────────────────────────────╮  raio 24
│ ╭────────────────╮  ‹ ›  Título        (▦ ☰ ▥ ⊞) (⋯)  (🔍 Busca) │  barra de ferramentas 52, sem fundo
│ │ ● ● ●          │                                              │
│ │ Favoritos      │   conteúdo opaco, de borda a borda,          │
│ │ ▣ Recentes     │   passando por baixo da barra de ferramentas │
│ │ ▣ Aplicativos  │   (some no degradê do topo)                  │
│ │ ...            │                                              │
│ │ Locais         │                                              │
│ │ Etiquetas      │                                              │
│ ╰────────────────╯  caminho › pasta          12 itens, 200 GB   │  barra de status (opcional)
╰──────────────────────────────────────────────────────────────╯
   barra lateral flutuante: recuo 8, raio 16, vidro, luzes dentro dela
```

- **Sem barra de título do compositor** nos apps do mide-ui ("janela de conteúdo
  inteiro", `Window::full_size_content`). O compositor continua com: cantos (raio
  pedido pelo app, `window_style app:<app_id> radius=24` no canal de controle; 16 nos
  apps de terceiros), contorno, sombra, redimensionar pelas bordas, lâmpada e
  maximizar. As **luzes** (fechar/minimizar/maximizar) são desenhadas pelo mide-ui
  (`ui.window_controls`), dentro da barra lateral ou no começo da barra de ferramentas:
  cinza sem foco, com os símbolos ao passar o ponteiro sobre o grupo.
- **Arrastar a janela:** pelo vazio da barra de ferramentas e da barra lateral (o app
  chama o `move` do xdg com o serial do clique); duplo clique no vazio maximiza.
- **Título:** na barra de ferramentas, à esquerda, depois dos botões de navegação
  (`title3`); sem título centralizado.
- **Janela sem barra lateral** (Editor de Texto, Terminal): as luzes no canto superior
  esquerdo da barra de ferramentas (ou da faixa de 40 pt no topo, se não houver
  barra de ferramentas).
- **Apps de terceiros** (GTK, Qt, X11) continuam com a barra do compositor, que passa a
  ter o raio 16 e as luzes do Tahoe.

---

## 4. Catálogo de componentes

Status: **existe** (já no mide-ui, no visual certo), **ajustar** (existe, mudar para o
Tahoe), **criar** (não existe), **só no app** (feito dentro de um app; passar para o kit
para outros apps usarem). Situação de 2026-10-24.

### 4.1 Estrutura da janela

| Componente | Status | Como é no Tahoe |
|---|---|---|
| Janela de conteúdo inteiro (`Window::full_size_content`, `fixed_size`) | existe | sem barra de título; cantos 24; luzes onde o app pedir (compositor + kit) |
| Barra lateral flutuante (`glass_sidebar`) | existe (faltam recolher seção, arrastar largura, esconder) | recuo 8, raio 16, vidro, luzes dentro, seções com título `subhead`, recolher seção, arrastar largura, esconder (⌃⌘S) |
| Barra de ferramentas (`toolbar`, `toolbar_title`) | existe | 52, sem fundo e sem linha; título à esquerda; grupos à direita |
| Grupo de botões em cápsula (`toolbar_buttons`) | existe | cápsula de vidro com 1–4 ícones; separador fino entre eles; o selecionado ganha pílula interna |
| Botão de navegação ‹ › | existe (por `toolbar_buttons`) | par em cápsula |
| Campo de busca em cápsula (`search_field`) | existe (falta recolher para só a lupa) | cápsula 36, lupa, texto, ✕ para limpar; pode recolher para só a lupa quando falta espaço |
| Efeito de borda de rolagem | criar | conteúdo some em degradê sob a barra de ferramentas e a de status |
| Painel à direita (inspetor/prévia) | criar | 270, opaco com divisor, ou flutuante (Visualização Rápida) |
| Divisão arrastável (split) | criar | divisor invisível de 8 pt com cursor de redimensionar |
| Barra de abas (`tab_bar`) | existe | abas em cápsulas dentro de uma faixa de vidro; + no fim; arrastar para reordenar; × ao passar o mouse |
| Barra de status / caminho (`status_bar`, `path_bar`) | existe (falta o degradê da borda de rolagem) | `footnote`, sem linha dura, dentro do degradê |
| Folha (sheet) | criar | painel que desce do topo da janela, raio 24, vidro, conteúdo atrás escurece |

### 4.2 Controles

| Componente | Status | Como é no Tahoe |
|---|---|---|
| Botão (`button_styled`, `ButtonStyle`) | existe (faltam os tamanhos mini a extra) | cápsula; tamanhos mini a extra; estilos: **com destaque** (azul, texto branco), **com borda** (preenchimento de controle), **sem borda**, **vidro**, **perigo** |
| Botão de ícone (`icon_button`/`icon_toggle`) | ajustar | círculo/cápsula 28–36; ligado = pílula com destaque |
| Interruptor (`switch`) | existe | trilho 38x22 cápsula, botão branco com sombra que estica ao apertar |
| Caixa de seleção (`checkbox`) | existe (falta o misto) | quadrado 16 raio 5; ligado = destaque com ✓ branco; misto (–) |
| Botão de opção (`radio`) | existe | círculo 16; ligado = destaque com ponto branco |
| Controle segmentado (`segmented`) | existe | cápsula com 2–6 opções (texto ou ícone); pílula de vidro que desliza até a escolhida |
| Controle deslizante (`slider`) | existe (faltam as marcas) | trilho 4 cápsula com a parte preenchida no destaque; botão em cápsula de vidro 22x16 que alonga ao arrastar; marcas opcionais |
| Passo (`stepper`) | existe | par ▲▼ em cápsula vertical, com campo numérico |
| Lista suspensa (`dropdown`) | ajustar | cápsula com ⌃⌄ à direita; abre um menu com ✓ no item atual |
| Menu de botão (pull-down) | criar | cápsula com ⌄; abre menu de ações |
| Campo de texto (`text_input`) | ajustar | raio 8, preenchimento de controle, anel de foco; rótulo à esquerda em formulários |
| Campo de senha | existe/ajustar | idem, com 👁 |
| Campo de etiquetas (tokens) | criar | itens em cápsula dentro do campo (filtros de busca, destinatários) |
| Combo (campo + lista) | criar | |
| Seletor de cor | criar | poço de cor redondo; painel com roda, controles e cores salvas |
| Seletor de data e hora | criar | campo com partes editáveis + calendário em popover |
| Barra de progresso (`progress_bar`) | existe (falta a indeterminada) | trilho 6 cápsula, destaque; indeterminada com faixa andando |
| Indicador giratório | criar | 12 raios em gradiente, ou anel |
| Indicador de nível | criar | barra segmentada (bateria, avaliação ★) |
| Botão de ajuda (?) | criar | círculo 22 com "?" |
| Selo (`badge`) | existe | cápsula vermelha com número (dock, abas, barra lateral) |
| Bolinha de etiqueta (`tag_dots`, `tag_toggle`) | existe | círculo 10–12 da cor da etiqueta; várias se sobrepondo |

### 4.3 Conteúdo

| Componente | Status | Como é no Tahoe |
|---|---|---|
| Lista em colunas / tabela (`list_row`, `list_header`) | existe (faltam colunas redimensionáveis e escolhíveis) | linhas 30, listras quase invisíveis, seleção em pílula raio 8 com recuo de 8 nas laterais, cabeçalhos com ⌃⌄ da ordem, colunas redimensionáveis e escolhíveis (clique direito no cabeçalho) |
| Árvore (`tree_row`, `disclosure`) | existe | setinha que gira (mola) |
| Grade de ícones (`tile`) | existe | seleção: fundo arredondado raio 10 atrás do ícone + nome em cápsula azul; ícones de 16 a 512 |
| Colunas (navegação em colunas) | só no Arquivos (passar para o kit) | colunas com divisor, setinha › nas pastas, seleção em pílula |
| Galeria | criar | imagem grande no meio, tira de miniaturas embaixo, informações e ações rápidas à direita |
| Formulário agrupado (Ajustes) | ajustar `card` (faltam linhas 44 e texto de ajuda embaixo) | cartão raio 14, linhas 44 com rótulo à esquerda e controle à direita, divisores recuados, texto de ajuda embaixo do cartão |
| Cabeçalho de página dos Ajustes | criar | ícone grande, título, descrição e link |
| Lista de barra lateral com ícones coloridos | existe (Configurações) | ícone em quadrado colorido 22 raio 6 + texto (Ajustes) |
| Estado vazio | criar | símbolo grande cinza, título e texto no centro |
| Avatar | criar | círculo com foto ou iniciais |
| Prévia de imagem (`image_centered`) | existe | |
| Área de texto (`text_area`) | existe | |

### 4.4 Menus, popovers e diálogos

| Componente | Status | Como é no Tahoe |
|---|---|---|
| Menu de contexto (`context_menu`) | existe (faltam seções com título) | vidro raio 12, itens 24 com **símbolo à esquerda**, atalho à direita em secundário, ✓, submenus ›, seções com título, linha de **bolinhas de etiqueta**, destaque em pílula raio 8 |
| Menus da barra superior | ajustar (barra) | mesmo visual, e a barra superior **sem fundo** (só texto sobre o papel de parede), item aberto em cápsula de vidro |
| Popover (`popover`) | existe | vidro raio 18 com seta apontando para a origem, fecha ao clicar fora |
| Dica (`tooltip`) | existe | aparece após 0,8 s parado; cápsula de vidro pequena com `footnote` |
| Alerta (`alert`) | ajustar (raio 20 feito; faltam o ícone do app e os botões na largura toda) | raio 24, vidro, ícone do app no topo à esquerda, título `title2`, texto, botões em cápsula ocupando a largura (lado a lado, ou empilhados se forem 3) |
| Folha | criar | ver 4.1 |
| Visualização Rápida | só no Arquivos (passar para o kit) | painel flutuante raio 18 sobre o conteúdo escurecido, barra superior de vidro claro com título e ações |
| Seletor de arquivos (`FileChooser`) | ajustar | é um mini-Finder: mesma barra lateral, barra de ferramentas e listas |

### 4.5 Shell do sistema (fora do kit, mesmo visual)

| Parte | Tahoe |
|---|---|
| Barra superior | sem faixa de fundo; texto e ícones direto sobre o papel de parede (com sombra suave para ler); item aberto em cápsula de vidro |
| Dock | vidro líquido, cantos maiores, indicador de app aberto em ponto |
| Central de controle | módulos em cartões de vidro com cantos grandes, personalizáveis |
| Notificações | cartões de vidro raio 24, agrupadas |
| Buscador (Spotlight) | campo em cápsula grande de vidro; resultados em lista de vidro; ações rápidas |
| Launcher | grade de apps (já existe) no mesmo vidro |

---

## 5. Padrões de tela

- **Finder (Arquivos):** barra lateral flutuante (Recentes, Compartilhado, Aplicativos,
  Área de trabalho, Documentos, Downloads / Locais com discos e ejetar / Etiquetas),
  barra de ferramentas com ‹ ›, título, grupo de visualizações (ícones, lista, colunas,
  galeria), grupo (agrupar, compartilhar, etiquetas, ações), busca; conteúdo de borda
  a borda; barras de caminho e de status opcionais (Visualizar); abas; inspetor.
- **Ajustes do Sistema (Configurações):** barra lateral com busca em cima, conta do
  usuário e grupos com ícones coloridos; página com cabeçalho grande e cartões
  agrupados; ‹ › e título na barra de ferramentas.
- **Documento (Editor de Texto):** conteúdo opaco de borda a borda, barra de
  ferramentas sem fundo, luzes no canto.
- **Utilitário pequeno (Calculadora):** janela raio 16, conteúdo com as luzes por
  cima.
- **Diálogo:** alerta raio 24 centrado na janela pai (folha) ou na tela.

---

## 6. Regras para qualquer tela nova (lista de revisão)

1. Conteúdo opaco de borda a borda; vidro só na navegação e nos controles flutuantes.
2. Cantos concêntricos; nenhum canto pequeno (< 8) em controle visível.
3. Botões e campos de busca em cápsula; botões da barra de ferramentas em grupos.
4. Sem linhas duras entre barra de ferramentas e conteúdo (borda de rolagem).
5. Controles do tamanho do Tahoe (normal 28, barra de ferramentas 36, linhas 28/30/44).
6. Seleção: pílula de destaque com foco, cinza sem foco.
7. Menus com símbolos à esquerda e atalhos à direita.
8. Claro e escuro conferidos; nenhuma cor fixa.
9. Cada animação com interruptor em Efeitos; desligada, tudo continua correto.
10. Testado em 1366x768 com captura do quadro.

---

## 7. Ordem de construção (D3 no plano)

1. **Tokens:** os valores da seção 2 no `Theme` (claro e escuro) e cores de destaque
   escolhíveis.
2. **Janela de conteúdo inteiro:** compositor sem a barra para os apps do mide-ui,
   luzes no lugar pedido, mover/maximizar pelo app; barra dos apps de terceiros no
   raio novo.
3. **Controles:** botões em cápsula (tamanhos e estilos), segmentado, caixa de seleção,
   rádio, interruptor, deslizante, passo, listas suspensas, progresso, selos, bolinhas.
4. **Estrutura:** barra lateral flutuante, barra de ferramentas com grupos, busca em
   cápsula, borda de rolagem, divisão arrastável, inspetor, abas, folha.
5. **Menus e diálogos:** menu de contexto com símbolos e etiquetas, popover, dica,
   alerta novo.
6. **Conteúdo:** listas/tabelas, grade, galeria, formulário agrupado, estado vazio.
7. **Galeria de componentes** (`mide-ui-demo`) mostrando tudo, claro e escuro, para
   conferir por captura.
8. **Apps:** Configurações, Arquivos, Editor, Terminal, Calculadora, Captura e os
   outros passam para o visual novo; depois, a reconstrução completa do Finder (B26).

---

## 8. Visual padrão e situação dos apps (2026-10-24)

**O Tahoe é o visual padrão do mide** (decisão do usuário, 2026-10-24). Toda janela nova do
mide-ui nasce de conteúdo inteiro (`Window::full_size_content`), com as luzes desenhadas pelo
kit, barra lateral flutuante quando há navegação, barra de ferramentas sem fundo com grupos
em cápsula, e só os componentes do kit (seção 4). Nada de desenhar controle à mão dentro de
um app: o que faltar vira componente do kit primeiro, para os outros apps e o framework
reaproveitarem. Quando os apps da tabela abaixo estiverem migrados, `full_size_content`
passa a ser o padrão do `Window::new` (com uma saída para quem precisar da barra do
compositor).

| App | Situação |
|---|---|
| Arquivos (`mide-files`) | Tahoe (falta o checklist do Finder, B26) |
| Configurações (`mide-settings`) | Tahoe na janela e na barra lateral; faltam o cabeçalho de página e as linhas 44 dos cartões |
| Calculadora | Tahoe |
| Editor de Texto (`mide-text`) | Tahoe |
| Editor de Capturas | Tahoe (Marcação) |
| Sobre este computador | Tahoe |
| Terminal (`mide-terminal`) | **falta**: barra do compositor; abas e Preferências no visual antigo |
| Captura (`mide-screenshot`, painel ⌘⇧5) | **falta**: painel flutuante em vidro do Tahoe |
| Seletor de arquivos (`mide-chooser`, `FileChooser`) | **falta**: virar mini-Finder (barra lateral e barra de ferramentas do Arquivos) |
| Instalador (`mide-install`), senha (`mide-askpass`, polkit) | **falta**: janela de conteúdo inteiro e botões em cápsula |
| Escolha da tela (`mide-screencast-picker`) | **falta** |
| Launcher, busca, notificações, central de controle | parcial: vidro líquido já; faltam os cartões de raio 24 nas notificações e os módulos em cartões na central |
| Barra superior | **falta**: sem faixa de fundo (texto direto sobre o papel de parede) e item aberto em cápsula de vidro |
| Dock | Tahoe (escura, ampliação, clique) |
| Bloqueio de tela (`mide-lock`) | **falta** revisar no Tahoe |

**Componentes do kit que faltam** (seção 4, status "criar"): folha (sheet), divisão
arrastável, inspetor, borda de rolagem em degradê, menu de botão (pull-down), campo de
etiquetas, combo, seletor de cor, seletor de data e hora, indicador giratório, indicador de
nível, botão de ajuda, galeria, estado vazio, avatar, cabeçalho de página dos Ajustes. E
passar para o kit o que hoje está só no Arquivos: navegação em colunas e Visualização Rápida.
