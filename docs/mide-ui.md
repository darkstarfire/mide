# mide-ui: crie apps com a interface do mide

> **Antes de criar uma tela, leia o guia de identidade visual: [`design.md`](design.md).** Ele define cores, vidro, estados, menus, diálogos e as animações padrão (durações e curvas) que todo app do mide segue.

O mide-ui é o framework para criar apps com a mesma cara e o mesmo comportamento das
Configurações do Sistema do mide: cartões brancos, interruptores, trilhos, barra
lateral, barra de rolagem fina e a barra de título com os três botões. Você escreve
em Rust, descreve a tela numa função e o framework cuida da janela, do mouse, do
teclado e do desenho.

## O pacote e o `mide-new`

O mide-ui é um projeto próprio (github.com/fabioaacarneiro/mide-ui), sem o
compositor: um app depende só dele.

```toml
[dependencies]
mide-ui = { git = "https://github.com/fabioaacarneiro/mide-ui" }
```

O jeito mais rápido de começar é o `mide-new`, que cria um projeto pronto: `Cargo.toml`,
`src/main.rs` com uma janela do Tahoe (barra lateral de vidro com seções, barra de
ferramentas, cartões, campo de texto, lista, interruptor e menus na barra superior), o
`.desktop` e um README com como instalar.

```sh
mide-new "Minhas Notas"            # cria ./minhas-notas
mide-new "Minhas Notas" --dir ~/Projetos
mide-new "Minhas Notas" --local    # usa o mide-ui da pasta ao lado do mide (desenvolvimento)
```

Dentro do mide, os apps usam o mesmo framework por `mide::kit` (repassado pelo
pacote `mide`); fora dele, por `mide_ui::kit`.

## Primeiro app

```rust
use mide_ui::kit::{run, App, Ui, Window};

#[derive(Default)]
struct Contador {
    cliques: u32,
}

impl App for Contador {
    fn view(&mut self, ui: &mut Ui) {
        ui.page(|ui| {
            ui.title("Contador");
            ui.label(&format!("Você clicou {} vezes.", self.cliques));
            if ui.primary_button("Clicar").clicked() {
                self.cliques += 1;
            }
        });
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(Window::new("Contador").size(360, 240), Contador::default())
}
```

Rode dentro de uma sessão do mide (o app é uma janela Wayland comum).

## Como funciona

O `view` é chamado a cada quadro e **descreve a tela inteira**. Não há eventos para
ligar nem objetos de tela para guardar: o estado é o seu `struct`, e os componentes
dizem o que aconteceu com eles naquele quadro.

```rust
if ui.button("Salvar").clicked() {
    self.salvar();
}
ui.switch("Notificações", &mut self.notificacoes);   // troca o bool sozinho
```

O quadro é redesenhado quando há mouse, teclado ou uma mudança. Quando um clique
muda o estado, o framework desenha de novo na hora, para a tela já mostrar o valor
novo.

## Componentes

| Componente | O que faz | Resposta útil |
|---|---|---|
| `ui.title("Texto")` | Título grande em negrito | — |
| `ui.heading("Grupo")` | Nome pequeno e cinza acima de um cartão | — |
| `ui.label("Texto")` | Texto comum, quebra em linhas | — |
| `ui.note("Texto")` | Texto menor e cinza (explicações) | — |
| `ui.button("Texto")` | Botão comum | `.clicked()` |
| `ui.primary_button("Texto")` | Botão de destaque (ação principal) | `.clicked()` |
| `ui.switch("Texto", &mut bool)` | Linha com interruptor | `.changed()` |
| `ui.slider("Texto", &mut f32, min, max)` | Linha com trilho | `.changed()` |
| `ui.text_input(&mut String, "dica")` | Campo de texto de uma linha | `.changed()`, `.submitted()` (Enter) |
| `ui.list_item("Texto", "detalhe")` | Item de lista clicável | `.clicked()` |
| `ui.choice_item("Texto", "detalhe", marcado, ativo)` | Item de uma lista de escolha: o marcado numa pílula azul, o desativado em cinza e sem clique | `.clicked()` |
| `ui.icon("nome-do-icone", 24)` | Ícone do tema do usuário | — |
| `ui.separator()` | Linha fina | — |
| `ui.space(12.0)` | Espaço vazio | — |

| `ui.tile(Tile::Icon("folder"), "Nome", selecionado, 64)` | Ícone (ou `Tile::Image(&pixmap)`) com o nome embaixo, como no Finder | `.clicked()`, `.double_clicked()` |
| `ui.list_row("folder", "Nome", &[("ontem", 170.0)], selecionada)` | Linha de lista com ícone e colunas à direita | `.clicked()`, `.double_clicked()` |
| `ui.icon_button("go-previous", "‹", ativo)` | Botão pequeno com ícone do tema (texto de reserva) | `.clicked()` |
| `ui.icon_toggle("view-list-symbolic", "☰", marcado)` | Botão de ícone que fica marcado | `.clicked()` |
| `ui.strong("Texto")` | Texto em negrito numa linha só (corta com "…") | — |

| `ui.scroll_area(240.0, \|ui\| ...)` | Área de altura fixa que rola (listas dentro de diálogos) | — |
| `ui.scroll_area_showing(240.0, Some((topo, base)), \|ui\| ...)` | A mesma, rolando o mínimo para a faixa (do topo do conteúdo) ficar à vista: o item marcado pelo teclado | — |
| `ui.reveal(resposta.rect)` | Pede que a área que rola em volta (página ou `scroll_area`) role o mínimo para o item ficar à vista (chamar uma vez: o arquivo pedido ao abrir o Arquivos) | — |
| `ui.dropdown("Texto", &opções, &mut índice)` | Linha com o valor escolhido; o clique abre a lista | `.changed()` |

### Controles do Tahoe (`docs/tahoe.md`)

| Componente | O que faz | Resposta útil |
|---|---|---|
| `ui.button_styled("Texto", ButtonStyle::Prominent)` | Botão em cápsula: `Prominent` (azul), `Bordered` (comum), `Plain` (só texto), `Glass` (vidro), `Destructive` (vermelho). `button` e `primary_button` também são cápsulas | `.clicked()` |
| `ui.segmented(&["A", "B"], &mut índice, Some(largura))` | Controle segmentado: a pílula desliza até a opção escolhida | `.changed()` |
| `ui.checkbox("Texto", &mut bool)` | Caixa de seleção com ✓ | `.changed()` |
| `ui.radio("Texto", marcado)` | Botão de opção; quem chama troca a escolha no clique | `.clicked()` |
| `ui.stepper(&mut i64, min, max)` | Número com ▲▼ em cápsula | `.changed()` |
| `ui.progress_bar(Some(0.6))` / `ui.progress_bar(None)` | Barra de progresso (com `None`, uma faixa que anda) | — |
| `ui.badge("3")` | Selo vermelho com número ou texto curto | — |
| `ui.tag_dots(&[TAG_COLORS[0]])` | Bolinhas de etiqueta (cores do Finder em `TAG_COLORS`) | — |
| `ui.toolbar_buttons(&[(ícone, reserva, marcado, ativo)])` | Grupo de botões numa cápsula de vidro (barra de ferramentas) | índice clicado |
| `ui.toolbar_title("Título", largura_máx)` | Título da janela na barra de ferramentas (encolhe com "…") | — |
| `ui.search_field(&mut String, "Buscar")` | Busca em cápsula, com lupa e ✕ | `.changed()` |
| `ui.window_controls(x, y)` | Os três botões da janela (só em janela de conteúdo inteiro; a barra lateral e a barra de ferramentas já põem) | — |
| `ui.window_drag_area(rect)` | O vazio do retângulo arrasta a janela; duplo clique maximiza (chamar depois dos controles da área) | — |
| `ui.remaining_width()` | Largura que sobra na linha atual (para encolher um título) | — |
| `ui.tab_bar(&títulos, escolhida)` | Barra de abas em cápsula, × ao passar o ponteiro e + no fim | `TabAction::Select/Close/New` |
| `ui.tooltip(&resposta, "texto")` | Dica que aparece depois de o ponteiro parar 0,8 s sobre o componente | — |
| `ui.popover(&mut aberto, ui.to_window(resposta.rect), largura, \|ui\| ...)` | Painel de vidro com ponta, preso a um botão; fecha com clique fora ou Esc (chamar no fim do `view`) | — |
| `ui.to_window(rect)` | Retângulo de uma área (que rola) nas coordenadas da janela | — |

A seleção das listas, da árvore e da grade é uma pílula de cantos 8 na cor de destaque,
e fica cinza quando a janela perde o foco (como no Mac).

Todas as respostas também trazem `.hovered()`, `.double_clicked()`,
`.secondary_clicked()` (botão direito) e o lugar ocupado (`rect`).

## Menu de contexto

Qualquer componente ganha um menu com o botão direito. O menu fica por cima de tudo,
fecha ao escolher, ao clicar fora ou com Esc:

```rust
let r = ui.tile(Tile::Icon("text-x-generic"), "notas.txt", false, 64);
ui.context_menu(&r, |m| {
    if m.item("Abrir") { self.abrir(); }
    if m.item("Renomear…") { self.renomear(); }
    m.separator();
    if m.danger("Mover para a Lixeira") { self.apagar(); }   // texto em vermelho
});
```

Para o menu do "vazio" (fundo de uma pasta), use `ui.background(ui.region())` depois
dos itens: ele só reage onde não há componente.

## Teclado, foco e título

- `ui.input().key(Key::Delete)`, `ui.input().shortcut("n")` (Ctrl+N).
- `ui.focus_next_input()`: o próximo `text_input` já começa com o foco e o texto todo
  selecionado (digitar substitui, como ao renomear no Finder). `ui.typing()` diz se
  há um campo com foco (para não tratar as teclas duas vezes).
- `ui.set_title("Downloads")`: troca o título da janela.
- `ui.request_tick()`: pede outro quadro em 40 ms, para mostrar trabalho feito em
  segundo plano (por exemplo, miniaturas que chegam de outra thread).

## Layout

Os componentes se empilham de cima para baixo, ocupando a largura disponível.

- **`ui.row(|ui| ...)`**: põe os itens lado a lado (botões numa mesma linha).
- **`ui.card(|ui| ...)`**: agrupa num cartão branco; os itens dentro ganham linhas finas
  entre si, como nas Configurações.
- **`ui.page(|ui| ...)`**: área com margem que rola quando o conteúdo não cabe, com a
  barra de rolagem fina (que aparece só quando precisa e pode ser arrastada).
- **`ui.wrap(|ui| ...)`**: lado a lado, quebrando linha quando a largura acaba (grade
  de ícones).
- **`ui.toolbar(|ui| ...)`**: faixa de ferramentas no topo, com linha fina embaixo.
  Dentro dela, `ui.push_right(largura)` manda o resto para a direita e
  `ui.sized(largura, |ui| ...)` fixa a largura do próximo item (um campo de busca).
- **`ui.sidebar(&secoes, atual, |ui, secao| ...)`**: barra lateral com seções (nome e
  ícone do tema) e o conteúdo da seção à direita, numa página que rola. Devolve a seção
  escolhida:

```rust
let secoes = [("Geral", "preferences-system"), ("Sobre", "help-about")];
self.secao = ui.sidebar(&secoes, self.secao, |ui, secao| match secao {
    0 => self.geral(ui),
    _ => self.sobre(ui),
});
```

## Janela

```rust
Window::new("Meu App")      // título; o id vira "meu-app"
    .app_id("org.exemplo.MeuApp")   // id para a dock e o launcher
    .size(760, 520)
    .min_size(480, 320)
    .theme(Theme::default())
```

**Janela do Tahoe:** `.full_size_content()` tira a barra de título do compositor e
deixa os cantos com 24. Com `ui.glass_sidebar()` + `ui.sidebar_area(...)` a barra
lateral vira um painel de vidro flutuante com os botões da janela dentro; sem barra
lateral, `ui.toolbar(...)` põe os botões no começo. O vazio das duas arrasta a janela.
O exemplo `mide-ui-demo` é a galeria de tudo isso.

Para o app aparecer no launcher e poder ser fixado na dock, instale um arquivo
`.desktop` com o mesmo id em `StartupWMClass`.

## Camadas e painéis de vidro

Um app pode ser uma **camada** por cima de tudo (como o buscador do mide), em vez de
janela: transparente, na tela inteira e com o teclado só para ela.

```rust
run(Window::new("Buscar").overlay("meu-painel"), MeuPainel::default())
```

Dentro dela, `ui.glass_panel` desenha um painel com o vidro do mide (o compositor
desfoca o que está atrás e aplica o vidro líquido) e a altura acompanha o conteúdo:

```rust
let (_, _, w, h) = ui.region();
ui.glass_panel((w - 600.0) / 2.0, h * 0.2, 600.0, 18.0, |ui| {
    ui.text_input(&mut self.busca, "Buscar");
    for (i, r) in self.resultados.iter().enumerate() {
        if ui.result_row(&r.icone, &r.titulo, &r.tipo, i == self.escolhido).clicked() {
            ui.close();   // fecha o app
        }
    }
});
```

Ícones são carregados em segundo plano: o componente aparece sem o ícone por um
instante e o ícone surge assim que fica pronto. Para já ter os ícones na primeira
vez, carregue numa thread e entregue com `ui.provide_icon(nome, tamanho, imagem)`.

### Camada no canto (avisos que somem sozinhos)

Com `.corner("nome")`, o app é uma camada pequena, do tamanho de `.size(l, a)`, no canto
de baixo à direita da tela principal, acima da dock, e **sem o teclado** (não tira o
foco de quem está digitando). É a miniatura que aparece depois de uma captura de tela.
O app diz onde recebe cliques com `ui.input_area(retângulo)` a cada quadro; o resto da
camada deixa os cliques passarem.

```rust
run(Window::new("Aviso").corner("meu-aviso").size(280, 300), MeuAviso::default())
```

Um menu de contexto numa camada pequena precisa de duas ajudas: `ui.menu_open()` diz se
há um menu aberto (e o retângulo dele), para a área de cliques crescer enquanto ele
aparece, e `ui.close_menu()` fecha o menu (o clique fora da camada não chega ao app;
feche quando o ponteiro sai dela).

### Uma camada em cada tela

Com `.on_every_output()` (junto com `.overlay`), a camada cobre todas as telas, e não
só a principal (a captura de tela, que escurece todas):

```rust
run(Window::new("Telas").overlay("minhas-telas").on_every_output().fixed_scale(), App::default())
```

O mesmo `view` é chamado uma vez por tela em cada quadro:

- `ui.screen()` é a tela deste passe (`Screen`: nome, origem no desktop, tamanho em
  pontos, escala e se é a principal); `ui.screens()` lista todas, primeiro a da
  superfície principal do app (a tela principal de quando o app abriu: trocar a
  principal com o app aberto não muda a camada de lugar).
- `ui.mirror()` é `true` nos passes das outras telas. **Estado só no passe com
  `!ui.mirror()`**: teclas, cliques e mudanças do app ficam lá; os outros passes só
  desenham (o que eles pedem — fechar, vidro, título — é descartado).
- `ui.pointer_global()` dá o ponteiro em coordenadas do desktop, em qualquer tela; um
  arrasto que começa numa tela e termina na outra continua chegando ao app.
- Só a camada da principal recebe o teclado. As outras desenham em escala 1 (o
  compositor amplia nas telas com escala) e se chamam `<namespace>@<TELA>`.
- Monitores que entram e saem ganham e perdem a camada com o app aberto. Se sai (ou é desligada)
  a tela da superfície principal, o app fecha: a camada dela ficaria invisível com o
  teclado.

As funções de geometria (`screen_at`, `union`, `to_main`...) ficam em
`mide::kit::screens`. O exemplo `examples/` do mide-ui (e `examples/kit-telas.rs` do mide)` — `kit-telas.rs` mostra tudo isso.

## Área de texto (várias linhas)

`ui.text_area(&mut doc, altura)` é o editor do Editor de Texto: números de linha, fonte
de largura fixa, cursor, seleção com mouse e teclado, desfazer (Ctrl+Z) e refazer
(Ctrl+Shift+Z, Ctrl+Y), área de transferência (Ctrl+A/C/X/V, também com Shift) e
rolagem; só as linhas à vista são desenhadas. Os outros atalhos com Ctrl (salvar,
buscar...) ficam para o app. O documento (`TextDoc`) guarda o texto, o cursor e o
histórico; `version` aumenta a cada mudança (começa em 0 em cada `TextDoc::new`) e
`can_undo()`/`can_redo()` servem ao menu Editar. Quando o app muda o texto por conta
própria, use `type_text` ou chame `before_change` antes: senão a mudança não entra no
desfazer e o `version` não muda.

`ui.text_area_with(&mut doc, altura, TextAreaLook { .. })` muda a aparência: sem os
números de linha, em fonte proporcional, outro tamanho, uma dica com o documento vazio
(cortada com "…" se não couber) e decorações, pedidas a cada quadro depois das teclas
(nunca atrasam um quadro). As decorações ficam fora do texto (cursor, seleção e clique
não mudam):

- `colors`: trechos coloridos;
- `underlines`: trechos sublinhados com pontilhado vermelho (erros);
- `trailing`: um texto depois do fim de uma linha (o resultado das notas da
  Calculadora), 4 pt depois do fim; vários na mesma linha vêm na ordem da lista. Com o
  cursor no fim dessa linha, a área rola para ele ficar à vista; o que não cabe até a
  borda termina com "…".

Índices em bytes do texto; trechos fora do texto ou no meio de um caractere são
ignorados. A resposta traz, no quadro de um clique (esquerdo ou direito), a posição do
texto sob o ponteiro: guarde a linha no clique direito para o menu de contexto usar
depois (com o menu aberto, o ponteiro está em cima dele e a posição não vem):

```rust
use mide_ui::kit::{Decorations, TextAreaLook, TextDoc};
use mide_ui::kit::textarea::{line_of, line_starts};

let destaque = ui.theme().accent;
let mut decorar = |doc: &TextDoc| Decorations {
    colors: vec![(0..5, destaque)],                  // "total" em azul
    underlines: vec![],
    trailing: vec![(0, "1.200".into(), destaque)],   // depois do fim da linha 0
};
let r = ui.text_area_with(&mut self.doc, 300.0, TextAreaLook {
    line_numbers: false,
    sans: true,
    size: Some(15.0),
    placeholder: "Digite uma conta e termine com =",
    decorate: Some(&mut decorar),
});
if r.response.secondary_clicked {
    self.linha_do_menu = r.pointer_index.map(|i| line_of(&line_starts(&self.doc.text), i));
}
ui.context_menu(&r.response, |m| {
    if m.item("Copiar o resultado") { /* usa self.linha_do_menu */ }
});
```

## Diálogos e seletor de arquivos

`ui.modal(largura, |ui| ...)` escurece a janela e mostra um painel no meio; só o que
está dentro dele reage. Para o caso comum há o aviso pronto, que devolve o botão
clicado (o último é o principal; Esc conta como o primeiro e Enter como o último):

```rust
if self.confirmar {
    match ui.alert("Apagar o arquivo?", "Não dá para desfazer.", &["Cancelar", "Apagar"]) {
        Some(1) => { self.apagar(); self.confirmar = false; }
        Some(_) => self.confirmar = false,
        None => {}
    }
}
```

Numa janela estreita (a Calculadora básica, 250 pt), os botões do aviso encolhem para
caber; estreitos demais, ficam um embaixo do outro, com o principal em cima.

Para montar um diálogo próprio: `ui.dialog_buttons(&["Cancelar", "OK"])` põe os botões à
direita, cada um do tamanho do rótulo, com o último em azul (devolve o clicado);
`ui.error_note("Senha incorreta")` escreve um erro pequeno em vermelho;
`ui.password_field(&mut senha, "Senha", erro)` é o campo de senha com a cara dos campos
do tema (bolinhas, sempre com o teclado, sem copiar; Enter responde `submitted`, Esc e
Ctrl+U apagam; `erro` pinta a borda de vermelho). O `ui.password_input` é o da tela de
bloqueio, claro sobre o papel de parede. O `mide-askpass` usa os três.

Para escolher numa lista longa, em grupos e com busca (as 140 unidades da Calculadora,
em 15 categorias), há o seletor de lista (`ListPicker`): sem busca, mostra o grupo do item
atual, com "‹ Todos os grupos" no topo; com busca (sem acento nem maiúscula, todas as
palavras, também pelo nome do grupo), uma lista só. ↑/↓ andam (pulando os itens
desativados), Enter escolhe o marcado (sem busca, o item atual; com busca, o melhor
resultado: a palavra exata, depois o começo do nome), Esc e "Cancelar" fecham:

```rust
use mide_ui::kit::{ListPicker, PickGroup, PickItem, Picked};

// Rótulo, detalhe cinza e as palavras exatas do item (símbolo, apelidos).
let grupos = vec![PickGroup { name: "Comprimento".into(), items: vec![PickItem::new("Metros", "m", &["m", "metro"])] }];
self.escolher = Some(ListPicker::new("Converter de", "Todas as categorias", "Buscar unidade", grupos, (0, 0)));
// a cada quadro, por cima do resto:
if let Some(seletor) = self.escolher.as_mut() {
    match seletor.show(ui) {
        Some(Picked::Chosen { group, item }) => { self.usar(group, item); self.escolher = None; }
        Some(Picked::Cancelled) => self.escolher = None,
        None => {}
    }
}
```

O seletor de arquivos (`FileChooser`) abre por cima da janela: lugares (pasta pessoal,
Área de trabalho, Documentos, Downloads, Imagens), a pasta de cima, a lista com pastas
primeiro, duplo clique para entrar ou escolher. `FileChooser::open` escolhe um arquivo,
`FileChooser::save` pede o nome (e pergunta antes de substituir um que já existe) e
`FileChooser::folder` escolhe uma pasta; `.extensions(&["txt", "md"])` filtra. O app
guarda o seletor enquanto ele está aberto:

```rust
if let Some(seletor) = self.abrir.as_mut() {
    match seletor.show(ui) {
        Some(Choice::Chosen(caminho)) => { self.abrir_arquivo(caminho); self.abrir = None; }
        Some(Choice::Cancelled) => self.abrir = None,
        None => {}
    }
}
```

Mais opções (as do portal `FileChooser`, usadas pelo `mide-chooser`, que é o seletor dos
outros apps na sessão do mide): `.multiple(true)` deixa escolher vários com Ctrl+clique
e Shift+clique (todos em `seletor.chosen()`, depois do `Chosen` com o primeiro);
`.filters(vec![Filter { nome, padrões }], atual)` mostra a lista "Mostrar" com filtros
por nome (`Pattern::Glob("*.png")`, sem diferenciar maiúsculas) ou por tipo
(`Pattern::Mime("image/*")`), e `seletor.current_filter()` diz o escolhido;
`.choices(vec![ExtraChoice { .. }])` põe caixas de marcar ou listas embaixo
(`seletor.choice_values()`); `.accept_label("Enviar")` troca o texto do botão;
`FileChooser::save_files(título, pasta, &nomes)` escolhe a pasta onde salvar vários
arquivos (pergunta antes de substituir); `.embedded(true)` ocupa a janela inteira em vez
de abrir por cima.

## Tema

O tema segue Configurações → Aparência (claro ou escuro) e troca na hora, sem
reabrir o app. `Theme::system()` é o tema escolhido; `Theme::dark()` e
`Theme::default()` (claro) existem para quem quer um fixo. `theme.shade(alfa)` dá o
tom de contraste certo para o tema (preto no claro, branco no escuro): use-o em
linhas, trilhos e fundos sutis em vez de cores fixas.

Cores, cantos, fontes e espaçamentos ficam em `Theme`. Para mudar só a cor de
destaque:

```rust
let mut tema = Theme::default();
tema.accent = [52, 199, 89, 255];   // verde
run(Window::new("Verde").theme(tema), MeuApp::default())
```

Na barra lateral, um item sem ícone vira um título de grupo (`("Favoritos", "")`).
`ui.sidebar_area` é igual, mas entrega a área inteira à direita, sem margem nem
rolagem: dá para pôr uma `toolbar` fixa e uma `page` embaixo.

## Exemplo completo

`examples/` do mide-ui (e `examples/kit-telas.rs` do mide)` — `mide-ui-demo.rs` usa os componentes básicos: barra lateral, cartões com
interruptores e trilho, campo de texto com Enter e uma lista de tarefas.

O app **Arquivos** (`src/bin/mide-files.rs`) é um app completo feito só com o mide-ui:
barra lateral com grupos, barra de ferramentas, grade de ícones com miniaturas,
lista, menus de contexto, duplo clique, renomear e atalhos.

```
cargo run --example mide-ui-demo
```

## Ainda não tem (próximas versões)

- Vidro em janelas comuns (hoje, só em camadas).
- Seleção múltipla e arrastar e soltar entre componentes.
- Animações dos componentes (o interruptor deslizando).
- Toque na tela (gestos) e acessibilidade.

## Compatibilidade com conceitos do Mac (plano futuro E10)

O mide-ui segue os conceitos do AppKit do macOS, para que uma camada de compatibilidade futura (rodar apps de .dmg por meio de algo como o Darling, com a parte gráfica desenhada pelo mide-ui) possa casar um com o outro. Nada de código, frameworks ou imagens da Apple: só os conceitos. Equivalências de hoje:

| mide-ui | AppKit |
|---|---|
| `Window` (`full_size_content`, `fixed_size`) | `NSWindow` (style mask: titled, fullSizeContentView, sem resizable) |
| `window_controls`, `window_drag_area` | botões padrão da janela, `isMovableByWindowBackground` |
| `toolbar`, `toolbar_buttons`, `toolbar_title`, `search_field` | `NSToolbar`, `NSToolbarItemGroup`, `NSSearchToolbarItem` |
| `glass_sidebar`, `sidebar_search` | `NSSplitViewController` com a barra lateral |
| `App::menu_bar`, `MenuGroup`, `MenuItem` | `NSMenu`, `NSMenuItem` (ações e atalhos) |
| `context_menu`, `popover`, `tooltip` | menu de contexto, `NSPopover`, tooltip |
| `alert`, `modal` | `NSAlert`, folha (sheet) |
| `FileChooser` | `NSOpenPanel`, `NSSavePanel` |
| `copy`, `request_paste`, `start_drag` | `NSPasteboard`, arrastar e soltar |
| `button_styled` (`ButtonStyle`), `switch`, `segmented`, `checkbox`, `radio`, `stepper`, `slider`, `progress_bar` | `NSButton` (bezel styles), `NSSwitch`, `NSSegmentedControl`, `NSStepper`, `NSSlider`, `NSProgressIndicator` |
| `list_row`, `tree_row`, `tile` | `NSTableView`, `NSOutlineView`, `NSCollectionView` |
| `richtext` (trechos de estilo, RTF, PDF) | `NSAttributedString`, `NSTextView`, RTF, `NSPrintOperation` |
| `columns`, `form_row`, `fixed_label` | `NSStackView`, `NSGridView` (formulário com rótulos alinhados) |

Regra: todo componente novo do kit ganha aqui a linha do equivalente no AppKit.
