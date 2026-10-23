# Identidade visual do mide

Este guia reúne as regras de aparência e de comportamento que já valem no mide. Ele
serve para três coisas:

1. **Apps novos do mide** (e da comunidade, com o mide-ui) seguem estas regras.
2. **O framework (mide-ui)** usa estes valores como padrão: um componente novo nasce
   certo, sem ajuste fino depois.
3. **Revisão:** antes de publicar uma tela nova, confira a lista do fim.

O mide se inspira no visual do macOS (sem copiar nada dele): claro, limpo, com vidro,
cantos arredondados, movimento curto e suave. Bonito, mas sempre leve.

Valores marcados com `código:` estão no código e devem ser lidos de lá (o tema do
mide-ui em `src/kit/theme.rs`, as durações e curvas em `src/motion.rs`). Se mudar aqui,
mude lá, e vice-versa.

---

## 1. Princípios

- **Claro e calmo.** Fundo quase branco, cartões brancos, texto quase preto, um azul de
  destaque. Cor forte só para o que importa (seleção, ação principal, perigo).
- **Profundidade com vidro e sombra suave**, nunca com bordas pesadas. Linhas
  divisórias são finas e quase transparentes.
- **Movimento curto, com propósito.** Toda animação explica de onde algo vem ou para
  onde vai (a pasta cresce do ícone, a janela escorre para a dock). Nada passa de meio
  segundo, exceto a lâmpada.
- **Leve antes de bonito.** Nenhum efeito pode custar quadros: desenhar só quando algo
  muda, guardar o que dá para guardar (desfoque, imagens prontas) e medir.
- **Quem usa decide.** Todo efeito e toda animação têm um interruptor em
  Configurações → Efeitos. Desligado, a interface continua correta e completa (só sem
  o efeito).
- **Coerência acima de criatividade local.** Se um padrão já existe (destaque, menu,
  diálogo), use o mesmo. Um app que "faz diferente" quebra a identidade.

---

## 2. Cores

`código: src/kit/theme.rs (Theme::default)`

| Papel | Valor | Uso |
|---|---|---|
| Fundo da janela | `#F6F6F8` | área de conteúdo |
| Barra lateral | `#E8E8EC` | barra lateral opaca (sem vidro) |
| Cartão | `#FFFFFF` | grupos de opções, listas, painéis |
| Linha divisória | preto a 9% (`0,0,0,24`) | entre linhas de um cartão |
| Texto | `#18181C` | texto principal e ícones "symbolic" |
| Texto secundário | `#82828A` | explicações, datas, títulos de grupo, ícones desativados |
| Destaque | `#0A6EEB` | seleção, botão principal, interruptor ligado, trilhos |
| Sobre o destaque | `#FFFFFF` | texto e ícones em cima do azul |
| Controle | preto a 6% (`0,0,0,16`) | fundo de botões e o **destaque ao passar o mouse** |
| Pressionado | preto a 11% (`0,0,0,28`) | fundo de um controle apertado |
| Perigo | vermelho (`#D22828` no texto) | ações que apagam ("Mover para a Lixeira") |

Botões da janela (barra de título): fechar `#FF5E57`, minimizar `#FFBD2E`, maximizar
`#28C840` (`código: src/decorations.rs`).

**A corrigir:** a barra superior usa um azul próprio (`#0A78F0`) nos interruptores e
trilhos da central de controle. O certo é o destaque do tema (`#0A6EEB`).

**Tema escuro:** ainda não existe. Por isso **nunca escreva cor fixa num app**: use os
papéis do tema (`t.text`, `t.accent`...). Ícones "symbolic" são pintados com a cor do
texto (não com a cor do arquivo do ícone), para acompanharem o tema escuro sozinhos.

---

## 3. Tipografia

`código: src/kit/theme.rs`, `src/ui/text.rs`

- Fonte sem serifa do sistema (pelo fontconfig); monoespaçada no Editor de Texto.
- Tamanhos: texto **13,5 px**; pequeno **12 px** (explicações, colunas, rótulos de
  ícones); título de página **20 px semibold**; títulos de grupo pequenos, semibold, na
  cor secundária.
- Texto que não cabe é cortado com reticências (`…`), nunca quebrado no meio de um
  controle.
- Teclas nos atalhos: `Ctrl`, `Alt`, `Super`, `⇧`, `Tab`, `↑ ↓ ← →`, `Espaço`, `Esc`,
  `Enter`, `Print Screen`; no estilo Mac, `⌘` e `⌥`.

---

## 4. Forma e espaço

`código: src/kit/theme.rs`, `src/decorations.rs`

| Medida | Valor |
|---|---|
| Canto de janela | 10 px |
| Canto de painel, cartão, menu | 10–14 px (menus 10, painéis de vidro 14) |
| Canto de controle (botão, campo, destaque) | 7 px |
| Barra de título das janelas | 30 px |
| Barra superior | 28 px |
| Altura de linha (listas, interruptores) | 32 px |
| Espaçamento entre componentes | 10 px |
| Margem da página | 24 px |
| Margem interna de painel de vidro | 10 px |
| Ícones na barra lateral | 18 px (symbolic) ou 20 px |
| Ícones nos botões da barra de ferramentas | 16 px, botão de 32x28 |

Sombras são suaves e largas (raio ~16 px, deslocadas ~6 px para baixo, preto a ~24%),
nunca uma linha dura. A janela em foco tem sombra mais forte que as outras.

---

## 5. Vidro (materiais)

O vidro é a marca mais forte do mide. Há três níveis, sempre com o desfoque feito pelo
compositor atrás da área (o app só pede a região: `ui.glass_panel`,
`ui.glass_sidebar`, ou `glass` no canal de controle).

| Nível | Onde | Como |
|---|---|---|
| **Vidro líquido** | dock e pastas do launcher (ícone e painel aberto) | desfoque + refração na borda, reflexo suave vindo de cima à esquerda, borda clara bem fina |
| **Vidro de painel** | barra superior, menus, notificações, central de controle, prévias, buscador, barra lateral do Arquivos | desfoque + véu claro (`248,248,250` a 70%: alfa 178) e contorno branco fino |
| **Opaco** | quando o vidro está desligado, e em janelas comuns fora das áreas de vidro | mesmo véu a 96–100% (alfa 246 em camadas, 255 em janelas) |

Regras:
- **Desligar o vidro deixa tudo opaco** (Configurações → Efeitos → "Vidro em barras e
  menus"), sem mudar o resto do layout.
- Numa janela comum, vidro só onde o design pede (a barra lateral de acesso rápido do
  Arquivos). O conteúdo principal é sempre opaco, para leitura.
- O desfoque só é refeito quando o que está atrás muda; parado, sai do cache.
- Vidro que aparece (menu abrindo, janela saindo da lâmpada) **entra esmaecendo** junto
  com o conteúdo, nunca de uma vez.

---

## 6. Estados de interação

Todo elemento clicável tem os mesmos estados, nesta ordem de força:

| Estado | Aparência |
|---|---|
| Normal | sem fundo (itens de lista, barra lateral) ou fundo "controle" (botões) |
| **Ponteiro em cima** | **pílula cinza arredondada** (cor "controle", canto 7 px, com folga de ~3 px em cima e embaixo) |
| Apertado | pílula um pouco mais escura (cor "pressionado") |
| **Selecionado** | pílula azul (destaque), texto e ícone brancos |
| Desativado | texto e ícone na cor secundária; não reage |
| Foco do teclado (campo de texto) | contorno azul de 1,5 px |

O destaque ao passar o mouse é **o mesmo** na barra lateral, nas listas
(`ui.list_item`), nos menus de contexto e nas linhas do Arquivos. Nada de retângulo reto
nem de cor própria.

Itens de grade (ícones do Arquivos, da área de trabalho, do launcher): selecionado =
vidro claro atrás do ícone e o nome numa pílula azul.

**Área ajustável** (seleção que se arrasta e muda de tamanho, como a da Captura):
contorno branco de 1,5 px e **bolinhas** nas 4 pontas e no meio dos 4 lados, de raio
5 pt, cheias na **cor de destaque** do tema (`t.accent`) com um anel branco de 1,5 px
(aparecem sobre qualquer fundo). O clique pega a bolinha até 10 pt do centro; as pontas
vencem o meio dos lados. Arrastar uma bolinha muda o tamanho (o lado oposto fica
parado, passar do outro lado vira a área); arrastar dentro da área muda o lugar. As
bolinhas somem enquanto uma área nova é escolhida ou algo é desenhado, e não aparecem
em modos rápidos que fecham ao soltar o mouse (⌘⇧4). As bolinhas ficam por cima de
tudo da área (inclusive o rótulo do tamanho).

**Escolher uma tela** (com mais de um monitor, como a "Tela inteira" da Captura e da
gravação): todas as telas escurecidas; a tela sob o ponteiro fica clara, com um
contorno de 4 px na **cor de destaque** em volta dela toda e, no meio, um rótulo em
vidro (`glass_panel`) com o que o clique faz ("Clique para capturar esta tela") e, em
nota, o nome e o tamanho em pixels. O clique escolhe; Esc volta para a Seleção (na gravação, que abre
direto na escolha, Esc cancela). Com
uma tela só, não se pergunta.

---

## 7. Componentes padrão (mide-ui)

Use sempre o componente pronto. Se faltar um, ele entra no mide-ui (com estas regras),
não fica só no app.

| Precisa de | Use |
|---|---|
| Página com título e rolagem | `ui.page`, `ui.title` |
| Grupo de opções | `ui.card` com `ui.heading` em cima |
| Liga/desliga com explicação | `ui.switch_detail` |
| Escolha numa lista | `ui.dropdown` |
| Escolha numa lista longa, em grupos e com busca | `ListPicker` (diálogo; itens com `ui.choice_item`) |
| Valor contínuo | `ui.slider` (grava ao soltar: `submitted`) |
| Linha clicável com detalhe à direita | `ui.list_item` |
| Navegação por seções | `ui.sidebar` / `ui.sidebar_search` (com busca) / `ui.glass_sidebar` |
| Barra de ferramentas | `ui.toolbar` com `ui.icon_button` / `ui.icon_toggle` (ícones `-symbolic`) |
| Grade de ícones | `ui.wrap` + `ui.tile` |
| Lista em árvore | `ui.list_header` + `ui.tree_row` |
| Menu de contexto | `ui.context_menu` |
| Pergunta ou formulário curto | `ui.modal` (janelinha) |
| Busca numa lista | `ui.text_input` no topo, filtrando enquanto digita (sem acentos: `mide::apps::fold`) |
| Texto de várias linhas | `ui.text_area` (editor, fonte fixa) / `ui.text_area_with` (fonte proporcional, dica, decorações: cores, erro com pontilhado vermelho, resultado depois do fim da linha na cor de destaque) |
| Painel por cima de tudo | `Window::overlay` + `ui.glass_panel` |

### Menus

- Abrem **embaixo do item** que os chamou (ou em cima, se não couber), com a **ponta**
  (8 px) apontando para ele. Num fundo vazio, saem do ponto do clique.
- Itens de 26 px, separadores finos, itens perigosos em vermelho no fim.
- Fecham ao escolher, ao clicar fora ou com Esc.
- Listas de ações repetidas (Downloads, Lixeira) **continuam abertas** depois de agir
  num item, para agir em vários; fecham ao clicar fora ou com Esc.

### Diálogos

- Sempre `ui.modal`: a janela escurece levemente (preto a 22%) e um painel aparece a
  ~24% da altura, centralizado, com o campo já selecionado.
- Botões: **Cancelar** à esquerda, ação principal (azul) à direita. Esc cancela, Enter
  confirma. Nada acontece antes da ação principal.
- Mudança que pode deixar a tela inutilizável (monitores) pede confirmação com
  contagem regressiva e volta sozinha (15 s).

### Nomes e textos

- Português, frases curtas, sem jargão ("Mover para a Lixeira", não "Excluir").
- Sugestões de nome: "Nova pasta", "Nova pasta (1)"…; nomes iguais sem diferenciar
  maiúsculas de minúsculas (como no macOS).
- Opções nas Configurações: rótulo + uma linha de explicação do efeito.

---

## 8. Movimento

`código: src/motion.rs` (durações e curvas num lugar só: o compositor, a barra, a dock, o launcher, o Arquivos e o mide-ui usam daqui)

### Curvas

- **Entrar e mover:** desaceleração cúbica (`1 - (1-t)³`): rápido no começo, assenta
  devagar.
- **Esmaecer de troca** (papel de parede, vidro aparecendo): suave nos dois lados
  (`t²(3-2t)`).
- **Sair:** a mesma curva ao contrário; um pouco mais curta que a entrada.

### Durações

| O quê | Duração | Movimento |
|---|---|---|
| Menu aparecendo (barra, dock, apps, contexto) | 120 ms | esmaece e desce 6 px |
| Pasta abrindo na lista do Arquivos | 180 ms (+25 ms por linha) | as linhas de dentro esmaecem de cima para baixo |
| Troca entre ícones e lista (Arquivos) | 200 ms | o conteúdo novo esmaece |
| Prévias da dock e alternador (Alt+Tab) | 160 ms | esmaecem e sobem 8 px |
| Aviso de volume e brilho (OSD) | 160 ms entrando, 1,5 s parado, 200 ms saindo | esmaece e sobe 8 px; apertos seguidos só trocam o valor |
| Miniatura da captura entrando / saindo | 160 / 120 ms | esmaece e desliza 8 px a partir da borda direita (fica 5 s) |
| Fechar janela | 180 ms | encolhe 6% e esmaece |
| Dock aparecendo/escondendo | 200 ms | desliza (espera 450 ms antes de esconder) |
| Vidro de uma janela depois de animar | 220 ms | esmaece |
| Maximizar, restaurar, metade da tela | 220 ms | posição e tamanho interpolados |
| Pasta do launcher abrindo/fechando | 240 ms | painel cresce do ícone; a grade some no mesmo ritmo |
| Launcher abrindo / fechando | 260 / 170 ms | esmaece com leve zoom |
| Página do launcher (carrossel) | 320 ms | só o conteúdo desliza; as bolinhas ficam paradas |
| Pasta nova surgindo no launcher | 340 ms | "pula" no lugar |
| Papel de parede trocando | 450 ms | a imagem nova esmaece sobre a antiga (o vidro acompanha) |
| Lâmpada (minimizar e restaurar) | 650 ms | a janela escorre até o ícone na dock (da tela certa) |

### Regras de movimento

- **Toda animação tem interruptor** em Configurações → Efeitos (hoje: Animações,
  Efeito lâmpada, Animação dos menus, Transições suaves, desfoques e vidros). Uma
  animação nova entra num grupo existente ou ganha o seu; nunca fica sem opção.
- **Origem e destino reais:** o que abre sai de onde foi clicado (ícone, item) e volta
  para lá ao fechar.
- **Nada aparece de uma vez no fim de uma transição.** O que vai sumir some junto com o
  que entra (a grade do launcher some enquanto a pasta cresce).
- **Elementos de navegação ficam parados** (bolinhas das páginas, barras); só o
  conteúdo se move.
- **Desempenho:** a animação prepara as imagens uma vez e cada quadro só combina ou
  copia (alvo: menos de 8 ms por quadro no app; o compositor ~0,3 ms). Medir com
  `tests/perf.sh` antes de publicar.
- Parado, nada é redesenhado (o compositor e os apps dormem).

---

## 9. Catálogo de efeitos: o que acontece, onde e quando

Esta seção dá nome a cada **tipo de efeito**, a cada **tipo de ação** e a cada **tipo de
superfície**, e diz qual efeito vale em cada caso. Para uma tela nova, a pergunta é
sempre: *que ação é esta, em que superfície?* A tabela 9.4 dá a resposta. Não se
inventa efeito novo para um caso que já está na tabela.

### 9.1 Tipos de superfície

| Código | Superfície | Exemplos no mide |
|---|---|---|
| **S1** | Janela de app | Arquivos, Configurações, Editor de Texto, apps GTK/Qt |
| **S2** | Painel fixo do sistema | barra superior, dock |
| **S3** | Menu (lista ancorada num item) | menus da barra, da dock e de contexto; menus nativos dos apps (popups) |
| **S4** | Painel flutuante passageiro | prévias das janelas, notificações, central de controle, Downloads e Lixeira da dock, miniatura da captura, aviso de volume e brilho |
| **S5** | Sobreposição de tela cheia | launcher, captura de tela, alternador de janelas (Alt+Tab) |
| **S6** | Diálogo modal (dentro de uma janela) | renomear, nova pasta, confirmar monitores |
| **S7** | Controle dentro de uma superfície | botão, interruptor, linha de lista, ícone da grade, item da barra lateral |
| **S8** | Fundo | papel de parede, área de trabalho |

### 9.2 Tipos de efeito

Cada efeito tem um nome, um significado (o que ele comunica) e valores fixos.

| Efeito | Significa | Como é | Duração e curva |
|---|---|---|---|
| **Imediato** | "isto reage a você" | troca de estado sem animação: pílula cinza, pílula azul, contorno | 0 ms |
| **Surgir** | "isto veio deste ponto, é passageiro" | esmaece e anda ≤ 8 px a partir da âncora (menus descem 6 px; painéis acima da dock sobem 8 px) | 120–160 ms, desaceleração |
| **Zoom de cena** | "você entrou noutro modo, que cobre tudo" | esmaece e assenta de 106% para 100%; ao sair, vai para 104% e some | 260 ms entrando, 170 ms saindo |
| **Expandir da origem** | "isto mora dentro daquele ícone" | o painel cresce de 25% a 100% saindo do ícone; o resto some no mesmo ritmo; ao fechar, volta para o ícone | 240 ms, desaceleração |
| **Lâmpada** | "a janela foi guardada ali" | a janela escorre até o ícone da dock e volta de lá | 650 ms |
| **Interpolar geometria** | "a mesma coisa, noutro tamanho ou lugar" | posição e tamanho vão do antigo ao novo | 220 ms, desaceleração |
| **Deslizar** | "ao lado há mais do mesmo nível" | o conteúdo anda inteiro para o lado; a navegação (bolinhas, barra) fica parada | 320 ms (páginas), 200 ms (dock se escondendo) |
| **Cascata** | "estes vieram de dentro daquele item" | os filhos entram um a um, esmaecendo e descendo um pouco, 25 ms de diferença entre eles | 180 ms cada |
| **Esmaecer cruzado** | "o mesmo lugar mostra outra coisa" | o novo aparece sobre o antigo | 200 ms (modo de visão), 220 ms (vidro), 450 ms (papel de parede), curva suave nos dois lados |
| **Encolher e sumir** | "isto deixou de existir" | encolhe 6% e esmaece | 180 ms |
| **Mola** | "você acabou de criar isto" | cresce de 55% passando um pouco do tamanho final e volta | 340 ms; uso raro |

Materiais (não são movimento, mas também são efeitos com interruptor):

| Material | Onde |
|---|---|
| **Vidro líquido** (desfoque + refração na borda + reflexo) | dock; pastas do launcher (ícone e painel aberto) |
| **Vidro de painel** (desfoque + véu claro) | barra superior, menus, notificações, central de controle, prévias, barra lateral do Arquivos, aviso de volume |
| **Desfoque de fundo** (leve, o papel de parede continua reconhecível) | atrás do launcher |
| **Sombra** e **cantos arredondados** | janelas |

### 9.3 Tipos de ação

| Código | Ação de quem usa | Efeito |
|---|---|---|
| **A1** | Apontar (passar o mouse) | Imediato (pílula cinza) |
| **A2** | Apertar, selecionar, ligar/desligar | Imediato (pílula mais escura, pílula azul, interruptor) |
| **A3** | Abrir algo passageiro ancorado (menu, lista, prévia) | Surgir |
| **A4** | Entrar num modo que cobre a tela | Zoom de cena |
| **A5** | Entrar num contêiner (pasta, grupo) e sair dele | Expandir da origem |
| **A6** | Ir para o lado no mesmo nível (página, aba) | Deslizar |
| **A7** | Revelar o que está dentro de um item numa lista (expandir) | Cascata (recolher: imediato) |
| **A8** | Trocar como o mesmo conteúdo é mostrado (ícones ↔ lista) | Esmaecer cruzado |
| **A9** | Mudar o tamanho ou o lugar de uma janela (maximizar, metade, restaurar) | Interpolar geometria |
| **A10** | Guardar e trazer de volta uma janela (minimizar, restaurar pela dock) | Lâmpada |
| **A11** | Fechar ou apagar | Encolher e sumir |
| **A12** | Criar algo novo no lugar (pasta, arquivo) | Mola (no launcher); nos outros, Imediato com o item já selecionado |
| **A13** | Responder a uma pergunta (diálogo) | Imediato, com o fundo escurecido |
| **A14** | Receber um aviso (notificação) | Surgir |
| **A15** | Trocar o ambiente (papel de parede, vidro aparecendo) | Esmaecer cruzado |
| **A16** | Mostrar e esconder um painel preso à borda | Deslizar |

### 9.4 Onde cada efeito acontece hoje

| Ação | Superfície | O que acontece | Interruptor (Configurações → Efeitos) |
|---|---|---|---|
| A1/A2 | S7 em qualquer superfície | pílula cinza / azul, sem animação | (sempre) |
| A3 | S3 menus da barra e da dock | Surgir, 120 ms, desce 6 px | Animação dos menus |
| A3 | S3 menus dos apps (popups) | Surgir, 120 ms | Animação dos menus |
| A3 | S4 prévias da dock | Surgir, 160 ms, sobe 8 px | Transições suaves |
| A14 | S4 miniatura da captura (canto de baixo à direita) | Surgir, 160 ms entrando e 120 ms saindo, desliza 8 px da borda direita; arrastada para a direita, segue o ponteiro e some | Transições suaves |
| A4 | S5 alternador (Alt+Tab) | Surgir, 160 ms (é passageiro e some ao soltar as teclas) | Transições suaves |
| A4 | S5 launcher | Zoom de cena, 260 / 170 ms | Animações |
| A5 | S5 pasta do launcher | Expandir da origem, 240 ms | Transições suaves |
| A6 | S5 páginas do launcher e da pasta | Deslizar, 320 ms (bolinhas paradas; o vidro das pastas desliza junto) | Transições suaves |
| A7 | S1 pastas na lista do Arquivos | Cascata, 180 ms | Transições suaves |
| A8 | S1 ícones ↔ lista no Arquivos | Esmaecer cruzado, 200 ms | Transições suaves |
| A9 | S1 maximizar, metades, restaurar | Interpolar geometria, 220 ms | Animações |
| A10 | S1 ↔ S2 minimizar e restaurar | Lâmpada, 650 ms; o vidro da janela volta esmaecendo (220 ms) | Efeito lâmpada |
| A11 | S1 fechar janela | Encolher e sumir, 180 ms | Animações |
| A12 | S5 pasta nova no launcher | Mola, 340 ms | Transições suaves |
| A12 | S1 nova pasta no Arquivos | Imediato, com o nome em edição | (sempre) |
| A13 | S6 diálogos | Imediato, fundo escurecido | (sempre) |
| A15 | S8 papel de parede | Esmaecer cruzado, 450 ms | (sempre) |
| A16 | S2 dock que se esconde | Deslizar, 200 ms, espera 450 ms antes de esconder | (opção da dock) |

**Ainda sem efeito (lacunas, entram com estas regras):**

| Ação | Superfície | Efeito a usar |
|---|---|---|
| Abrir janela | S1 | Surgir (esmaecer e crescer de 96% a 100%, 180 ms), com o interruptor Animações |
| Notificação chegando e saindo | S4 | Surgir, deslizando 8 px a partir da borda da tela, 160 ms |
| Central de controle, calendário | S4 | Surgir, 120 ms, como os menus |
| Diálogo abrindo | S6 | Surgir: o fundo escurece em 120 ms e o painel desce 6 px |
| Destaque da dock ao passar o mouse | S2 | Esmaecer cruzado curto (100 ms) |
| Captura de tela abrindo | S5 | Zoom de cena |

### 9.5 Regras para decidir

1. **Identifique a ação (A) e a superfície (S)** e use o efeito da tabela. Se a ação
   não está na tabela, ela é parecida com uma que está: use o efeito dela.
2. **Um efeito, um significado.** Deslizar é só para "mesmo nível ao lado"; nunca para
   abrir um menu. Expandir da origem é só para contêineres que têm um ícone na tela.
3. **Passageiro é curto** (≤ 160 ms); **mudança de modo é média** (220–260 ms);
   **viagem longa** (lâmpada) é a única acima de meio segundo.
4. **Sair é mais rápido que entrar** (ou igual).
5. **Feedback de ponteiro nunca espera:** hover e clique são imediatos.
6. **Uma animação de cada vez por superfície.** Se outra começar, a anterior termina
   na hora (vai para o estado final).
7. **Cada efeito tem um interruptor** da tabela 9.4. Efeito novo usa um interruptor
   existente quando o significado é o mesmo; senão, ganha o seu.
8. **Material segue a superfície:** S2, S3, S4 em vidro de painel (a dock e as pastas
   em vidro líquido); S1 opaca (vidro só na barra lateral, quando o design pede); S5
   sobre o fundo levemente desfocado.

---

## 10. Janelas e o ambiente

- Barra de título do mide com os três botões à esquerda (fechar, minimizar, maximizar);
  janelas com cabeçalho próprio (apps GTK) mantêm o deles.
- Janelas novas nascem na tela principal, abaixo da barra, em cascata.
- Maximizar respeita a barra e a dock (2 px de folga acima da dock).
- A barra e a dock ficam na tela principal; o papel de parede em todas.
- Cursor: seta do tema do sistema; em cima de painéis do mide (prévias, menus), sempre
  a seta, nunca o cursor do app de trás.

---

## 11. Ícones

- **Interface do mide** (barra lateral, barra de ferramentas, barra superior): ícones
  "symbolic" (de uma cor só), pintados com a cor do texto; brancos quando em cima do
  azul; na cor secundária quando desativados.
- **Apps e arquivos:** ícones coloridos do tema.
- **Tema de ícones próprio do mide (estilo Big Sur):** em desenvolvimento (B15). Até lá,
  o tema do sistema; ícones de terceiros só com licença livre e crédito.

---

## 12. Lista de revisão de uma tela nova

- [ ] Só cores do tema (nenhuma cor fixa); ícones de interface "symbolic".
- [ ] Componentes do mide-ui (nenhum controle feito à mão quando já existe um pronto).
- [ ] Destaque ao passar o mouse = pílula cinza; seleção = pílula azul.
- [ ] Menus com a ponta, abrindo embaixo do item; diálogos em `ui.modal`.
- [ ] Listas longas com busca no topo.
- [ ] Cada animação classificada (ação A, superfície S) e com o efeito da seção 9, as durações e curvas da tabela e o interruptor em Efeitos.
- [ ] Vidro só onde o padrão pede; com o vidro desligado, tudo opaco e correto.
- [ ] Textos curtos em português; opções com uma linha de explicação.
- [ ] Funciona em 1366x768 (a tela mais comum no Brasil) e em escala de 125%.
- [ ] Tempo de quadro medido (`tests/perf.sh`) e nada redesenhando parado.
