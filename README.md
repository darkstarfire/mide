# mide

**mide** é um ambiente de desktop (DE) moderno e leve para Linux, escrito em Rust
sobre Wayland. O compositor usa o [Smithay](https://github.com/Smithay/smithay), e os
componentes da área de trabalho (dock, barra superior, launcher, notificações,
Configurações e Arquivos) são clientes Wayland próprios.

> ### Sobre a origem do projeto
>
> - O mide é **criado do zero**. Todo o código, os shaders e os componentes visuais
>   foram escritos para este projeto.
> - **Não há engenharia reversa** de nenhum software, sistema ou recurso da Apple ou
>   do macOS, e o projeto não usa código, imagens, ícones, fontes ou outros arquivos
>   deles.
> - O visual do macOS é **apenas uma inspiração** de estilo (vidro translúcido, dock,
>   launcher em tela cheia), reinterpretada com implementação própria.
> - O mide **não é um concorrente** do macOS e **não tem o objetivo de ser uma cópia**
>   dele. É um desktop livre para Linux, com identidade própria.
> - "Apple" e "macOS" são marcas da Apple Inc. O mide não tem nenhuma relação com a
>   Apple, nem é patrocinado ou aprovado por ela.

## O que já tem

- **Compositor** Wayland com janelas de cantos arredondados, sombras, animações de
  maximizar e minimizar ("efeito lâmpada") e barra de título própria.
- **Dock** com apps fixados e abertos, prévias das janelas, menus de contexto,
  Downloads e Lixeira, e fundo de **vidro líquido** desenhado na GPU (desfoque,
  refração nas bordas e reflexo).
- **Barra superior** translúcida com menu do app, relógio com calendário, som, Wi-Fi
  e Bluetooth; menus e notificações também sobre vidro.
- **Launcher** em tela cheia com busca, páginas, pastas e arrastar para organizar.
- **Buscador** (`ctrl+space`): apps, ajustes, arquivos e contas, sobre vidro.
- **Notificações** (`org.freedesktop.Notifications`).
- **Configurações** (Wi-Fi, Bluetooth, som, papel de parede, dock, efeitos, atalhos).
- **Arquivos**, gerenciador de arquivos com grade e lista, miniaturas, busca, menus de
  contexto e Lixeira.
- **mide-ui**, framework para criar apps com a interface do mide
  ([guia](docs/mide-ui.md)).

O projeto está em desenvolvimento ativo. O plano em etapas e o estado de cada uma
ficam em [`project_plain/CONTEXT.md`](project_plain/CONTEXT.md).

## Rodar

Por enquanto o mide roda **aninhado** numa sessão gráfica existente (janela winit):

```sh
cargo run
```

Dentro dele, abra apps Wayland com o socket do mide, por exemplo:

```sh
WAYLAND_DISPLAY=mide-0 foot
```

As dependências de sistema e os detalhes de instalação estão em
[`project_plain/INSTALATION.md`](project_plain/INSTALATION.md).

## Testes

```sh
cargo test          # testes de unidade
tests/ui.sh         # bateria de interface (sessão de teste isolada)
tests/smoke.sh      # teste rápido de ponta a ponta
tests/perf.sh       # tempos de quadro e de resposta
```

Os testes usam um socket, uma configuração e uma Lixeira temporários: não mexem na
sua sessão nem nos seus arquivos.

## Criar um app com o mide-ui

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

Veja o [guia do mide-ui](docs/mide-ui.md) e o exemplo `cargo run --example mide-ui-demo`.

---

## English summary

**mide** is a modern, lightweight Wayland desktop environment for Linux, written in
Rust (Smithay compositor plus its own dock, top bar, launcher, notifications, settings
and file manager).

- mide is **built from scratch**. It involves **no reverse engineering** of any Apple
  or macOS software, and it uses none of their code, images, icons or fonts.
- macOS is used **only as visual inspiration**. mide is **not a competitor** to macOS
  and **does not aim to be a copy** of it.
- Apple and macOS are trademarks of Apple Inc. mide is not affiliated with, sponsored
  or endorsed by Apple.
