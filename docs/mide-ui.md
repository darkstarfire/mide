# mide-ui: crie apps com a interface do mide

O mide-ui é o framework para criar apps com a mesma cara e o mesmo comportamento das
Configurações do Sistema do mide: cartões brancos, interruptores, trilhos, barra
lateral, barra de rolagem fina e a barra de título com os três botões. Você escreve
em Rust, descreve a tela numa função e o framework cuida da janela, do mouse, do
teclado e do desenho.

## Primeiro app

```rust
use mide::kit::{run, App, Ui, Window};

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
| `ui.icon("nome-do-icone", 24)` | Ícone do tema do usuário | — |
| `ui.separator()` | Linha fina | — |
| `ui.space(12.0)` | Espaço vazio | — |

| `ui.tile(Tile::Icon("folder"), "Nome", selecionado, 64)` | Ícone (ou `Tile::Image(&pixmap)`) com o nome embaixo, como no Finder | `.clicked()`, `.double_clicked()` |
| `ui.list_row("folder", "Nome", &[("ontem", 170.0)], selecionada)` | Linha de lista com ícone e colunas à direita | `.clicked()`, `.double_clicked()` |
| `ui.icon_button("go-previous", "‹", ativo)` | Botão pequeno com ícone do tema (texto de reserva) | `.clicked()` |
| `ui.icon_toggle("view-list-symbolic", "☰", marcado)` | Botão de ícone que fica marcado | `.clicked()` |
| `ui.strong("Texto")` | Texto em negrito numa linha só (corta com "…") | — |

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

## Tema

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

`examples/mide-ui-demo.rs` usa os componentes básicos: barra lateral, cartões com
interruptores e trilho, campo de texto com Enter e uma lista de tarefas.

O app **Arquivos** (`src/bin/mide-files.rs`) é um app completo feito só com o mide-ui:
barra lateral com grupos, barra de ferramentas, grade de ícones com miniaturas,
lista, menus de contexto, duplo clique, renomear e atalhos.

```
cargo run --example mide-ui-demo
```

## Ainda não tem (próximas versões)

- Tema escuro pronto e troca automática com o sistema.
- Campo de texto com cursor no meio, seleção e colar.
- Diálogos e seletor de arquivos como componentes.
- Vidro em janelas comuns (hoje, só em camadas).
- Seleção múltipla e arrastar e soltar entre componentes.
- Animações dos componentes (o interruptor deslizando).
- Toque na tela (gestos) e acessibilidade.
