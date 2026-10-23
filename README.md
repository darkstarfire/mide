# mide

**Em construção.**

O mide é um ambiente de desktop para Linux, moderno e leve, feito em Rust sobre
Wayland. Ele reúne o compositor e os componentes do dia a dia: dock, barra superior,
launcher, buscador, notificações, Configurações e um gerenciador de arquivos.

**Objetivo:** oferecer um desktop bonito, simples de usar e rápido, com visual limpo
e efeitos de vidro, que funcione bem tanto em computadores comuns quanto em máquinas
modestas, e que seja fácil de usar com mouse, teclado ou tela de toque.

**Para que serve:** para usar o computador no dia a dia (abrir apps, organizar
janelas e arquivos, ajustar o sistema) num ambiente coeso, em que todas as partes
seguem o mesmo estilo e o mesmo comportamento.

**Apps que precisam de administrador:** num atalho `.desktop` (ou num script sem
terminal), use o `mide-sudo`, que pede a senha numa janela do mide e roda o programa
como root (dentro de um terminal, é o `sudo` de sempre):

```ini
Exec=mide-sudo --name XAMPP /opt/lampp/manager-linux-x64.run
```

O `pkexec` e os apps que pedem permissão pelo polkit usam a mesma janela na sessão do
mide. O programa roda pelo X11 (XWayland), sem acesso ao Wayland nem à sessão D-Bus do
usuário; use só com programas confiáveis.

---

O mide é criado do zero. Não há engenharia reversa de nenhum software da Apple ou do
macOS, e o projeto não usa código, imagens, ícones ou fontes deles. O visual do macOS
serve apenas como inspiração; o mide não é concorrente nem cópia do macOS. "Apple" e
"macOS" são marcas da Apple Inc., sem relação com este projeto.

Os ícones vêm do [mide-icons](https://github.com/fabioaacarneiro/mide-icons), um fork
do [WhiteSur icon theme](https://github.com/vinceliuice/WhiteSur-icon-theme) (de Vince
Liuice, GPL-3.0) sem o logo da Apple e com desenhos próprios no lugar dos que recriavam
ícones de apps da Apple. O `data/mide-atualizar` instala o tema.
