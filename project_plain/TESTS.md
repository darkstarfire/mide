# Baterias de teste por etapa

Cada etapa concluída tem testes automáticos (rodados pelo agente) e uma checklist manual (para o usuário, no desktop). Marque cada item ao testar.

## Como rodar

```bash
cd ~/Projetos/projeto_de
cargo test                 # testes unitários (sem janela)
tests/smoke.sh             # teste de fumaça com cliente real (precisa de sessão gráfica)
```

Teste manual, com log detalhado:
```bash
RUST_LOG=mide=debug cargo run
# em outro terminal:
WAYLAND_DISPLAY=mide-0 GDK_BACKEND=wayland gnome-calculator
```

## A1, A2: socket e renderização

- Automático: `tests/smoke.sh` confere socket, globais e janela criada.
- Manual:
  - [V] A janela "mide" abre na sessão e não fica vazia.
  - [V] A calculadora aparece dentro da janela, na orientação normal.

## A3, A4, A5: teclado, ponteiro e foco

- Automático: nenhum (exige entrada real).
- Manual:
  - [V] Clicar numa tecla da calculadora com o mouse funciona.
  - [V] Digitar `2+2=` no teclado mostra `4`.
  - [V] Com duas calculadoras, clicar em uma traz ela para frente e ela recebe as teclas (log: `foco do teclado: nova janela ativa`).

## A6: arrastar janelas

- Manual:
  - [V] Arrastar pela barra da calculadora move a janela junto com o cursor.
  - [V] Soltar o botão encerra o arrasto.

## A8: maximizar, restaurar e fechar

- Manual:
  - [V] O botão de maximizar ocupa a tela; de novo, restaura a posição anterior.
  - [V - funcionou o Ctrl+q] `super+q` com a calculadora focada fecha a janela.
  - [V] Minimizar só registra no log (`minimizar pedido, ainda não implementado`). Esperado até a dock existir.

## A9: atalhos e comandos

- Automático: `cargo test` (módulo `config::tests`).
- Manual:
  - [F] `ctrl+alt+t` abre um terminal.
  - [F] Um arquivo `~/.config/mide/config.toml` com `"super+e" = "launch:gnome-calculator"` faz `super+e` abrir a calculadora (reinicie o mide).

## Popups (menus)

- Manual:
  - [V] O botão ☰ da calculadora abre o menu suspenso por cima da janela.
  - [V] Clicar numa opção do menu funciona.
  - [V] Clicar fora do menu fecha ele.

## Cascata de janelas

- Automático: `cargo test` (`state::tests`).
- Manual:
  - [F] Duas janelas novas não nascem uma sobre a outra.

## Notas do teste (feitas depois da sua marcação)

- **`super+e` (F):** o seu `config.toml` tinha os atalhos soltos no topo, e o parser só lia a seção `[keybindings]`. Os atalhos eram ignorados em silêncio. Agora os dois formatos são aceitos. Na inicialização, o log mostra `3 atalho(s) configurado(s)` para o seu arquivo. Refaça o teste de `super+e`. Se ainda não funcionar, a causa pode ser o próprio desktop, que pode capturar a tecla Super antes do mide. Nesse caso, me avise.
- **`ctrl+alt+t` (F):** o gnome-terminal reaproveita o servidor do desktop, então a janela abria fora do mide. O padrão agora é `foot`, que roda nativamente em Wayland. Para ele existir, instale com `sudo apt install foot`. Sem o foot, o padrão cai no `x-terminal-emulator`, que pode continuar abrindo fora do mide.
- **`super+q` (V, mas não era esse atalho):** você testou `Ctrl+Q`. Esse atalho é da própria calculadora, então o fechamento não veio do mide. O teste de `super+q` continua em aberto.
- **Cascata de janelas (F):** a cascata desloca cada janela nova em 32 px, então elas se sobrepõem parcialmente, de propósito. Se você esperava janelas sem sobreposição nenhuma, me diga o comportamento que prefere, e eu ajusto.

## Notas do teste 2 (atalho com Shift)

- `ctrl+shift+e` não casava porque o Shift transformava a tecla em `E`. Corrigido. Para testar: reinicie o mide (a config só é lida na inicialização) e aperte `ctrl+shift+e`. A calculadora deve abrir.
- **Confirmado pelo usuário:** `ctrl+shift+e` abre a calculadora pelo `config.toml` (após a correção do Shift).

## Atalhos principais (A9)

- Automático: `cargo test` (`atalhos_principais_de_terminal_e_buscador`).
- Manual:
  - [ ] `ctrl+shift+t` abre um terminal. Precisa de `foot` instalado (`sudo apt install foot`).
  - [ ] `ctrl+shift+space` abre o buscador de programas. Precisa de `fuzzel` instalado (`sudo apt install fuzzel`).

## Notas do teste 3 (buscador e super+q)

- `ctrl+shift+t` abriu o terminal (sem confirmar se abriu dentro do mide).
- `ctrl+shift+space` não abriu: o `fuzzel` precisava do protocolo de camadas, que o mide não tinha. Implementado. Refaça o teste.
- `super+q` não funciona: a tecla Super provavelmente é capturada pelo desktop. Use `ctrl+shift+q`, adicionado ao config. Para diagnosticar, rode com `RUST_LOG=mide=debug`: cada tecla aparece como `tecla: ...`. Se a tecla Super não aparecer, o desktop a está capturando.

## Notas do teste 4 (buscador não recebia teclas)

- Corrigido o foco de camadas. Confirmado no log: ao abrir o `fuzzel`, ele recebe o foco.
- Para testar digitação e Esc automaticamente, instale `wtype` (`sudo apt install wtype`). Com ele, o agente consegue digitar no mide e conferir no log (`tecla: ...`) e a ação de cada atalho.
- Manual, depois da correção: `ctrl+shift+space`, digite algo, aperte Esc. O buscador deve receber as letras e fechar.

## Notas do teste 5 (atalho de fechar e digitação)

- `super+q` removido: o desktop captura essa tecla. O padrão agora é `ctrl+shift+q`.
- O buscador abre e recebe o foco, mas não digitou. Para diagnosticar: rode `RUST_LOG=mide=debug cargo run`, abra o buscador com `ctrl+shift+space`, digite `abc` e envie as linhas do log que contêm `tecla:` e `foco`.
- **Confirmado pelo usuário:** `ctrl+shift+q` fecha a janela com foco.

## Diagnóstico da digitação no buscador (sem input real)

- Confirmado com `WAYLAND_DEBUG=1 fuzzel`: o buscador recebe `wl_keyboard.keymap`, `wl_keyboard.enter` e `wl_keyboard.modifiers`. O foco de teclado chega a ele.
- Como a calculadora digita normalmente pelo mesmo caminho (teste A3), falta confirmar se as teclas chegam ao mide enquanto o buscador está aberto. Só o log `tecla:` do teste manual responde isso.
- Pendente: instalar `wtype` (`sudo apt install wtype`) para testar a digitação automaticamente.

## Notas do teste 6 (digitação no buscador): resolvido

- Causa: frame callback não era enviado às camadas. O buscador recebia as teclas, mas não redesenhava.
- Corrigido e testado automaticamente com `wtype`: `tests/smoke.sh` passa com 9 de 9 testes.
- Manual: `ctrl+shift+space`, digite algo, aperte Esc. Ainda preciso que você confirme visualmente.
- **Confirmado pelo usuário:** o buscador (`ctrl+shift+space`) recebe a digitação e funciona.

## A7: barra de título do mide

- Automático: `cargo test` (módulo `decorations::tests`).
- Manual, com o `foot` (ele pede decoração de servidor):
  - [ ] Aparece uma barra acima da janela, com três botões coloridos.
  - [ ] Clicar no vermelho fecha a janela.
  - [ ] Clicar no verde maximiza; de novo, restaura.
  - [ ] Arrastar pela barra move a janela.
  - [ ] Duplo clique na barra maximiza.
- Esperado: a calculadora (GTK4) continua com a barra própria, porque ela não pede decoração de servidor.

## Notas do teste 7 (botões da barra)

- Os botões não apareciam porque o fundo cobria eles. Corrigido: o log mostra a barra desenhada em todos os frames com o `foot`. Refaça o teste da barra (botões, arrasto, duplo clique).

## Notas do teste 8 (cantos e botões)

- Cantos arredondados e botões redondos implementados com shaders. Testar com o `foot`: os cantos superiores e inferiores devem estar arredondados, e os botões devem ser círculos.
- Se a forma aparecer errada (ex.: quadrada, ou cortada), mande uma foto da janela.

## Notas do teste 9 (visual mac)

- Sem moldura do Openbox/desktop: a barra do mide é a única. Confira se a janela aparece sem a moldura externa.
- Cores claras e semáforo: vermelho, amarelo e verde quando a janela tem foco; cinza quando não tem.
- Sair do mide: `ctrl+shift+escape` deve encerrar o compositor.

## Notas do teste 10 (cantos)

- Os cantos continuavam quadrados por causa da ordem de desenho. Corrigido. Refaça o teste: os quatro cantos da janela devem estar arredondados, com a barra clara no topo.

## Notas do teste 11 (profundidade)

- Confira se a janela parece flutuar: sombra suave embaixo, traço fino na borda e brilho no topo da barra. A sombra deve ser mais forte com o foco.
- Se a sombra cobrir o conteúdo, ou se aparecer um quadrado escuro em volta, mande uma foto.

## Notas do teste 12 (hover e sombra)

- Passe o ponteiro sobre os três botões: devem aparecer ×, − e + (com a janela focada).
- A sombra não deve mais ter pontilhado na borda.
- Se algo ainda estiver errado (símbolo fora do centro, sombra cortada), mande uma foto.

## Notas do teste 13 (hover por botão e sombra)

- Passe o ponteiro sobre um botão: só ele deve mostrar o símbolo.
- A sombra deve estar limpa, sem o pontilhado da borda. Se o pontilhado continuar, mande uma foto e diga em qual lado (ex.: só embaixo, ou em volta toda).

## Notas do teste 14 (sombra, canto e maximizar)

- Sombra: sem pontinhos brancos no contorno.
- Canto inferior: a sombra deve continuar a curva do canto, sem entalhe.
- Maximizar (botão verde ou duplo clique na barra): a janela deve crescer suavemente, em cerca de 0,2 s, e não pular. Restaurar ainda é instantâneo.
- Se a sombra cobrir uma janela que está atrás, isso é a limitação conhecida. Mande uma foto.

## Notas do teste 15 (redimensionar e pontinhos)

- Redimensionar: passe o ponteiro na borda ou canto da janela e arraste. A janela deve mudar de tamanho; pela esquerda ou pelo topo, ela também anda. A barra só arrasta a janela, não redimensiona.
- Pontinhos brancos: continuam nos botões. Isso é esperado no modo janela. Confirme depois na sessão real (etapa A13).

## Notas do teste 16 (restaurar e cursor)

- Restaurar: clique no botão verde ou no duplo clique da barra com a janela maximizada. A janela deve voltar ao tamanho anterior com a mesma animação suave.
- Cursor: passe o ponteiro sobre a borda ou canto da janela. A forma deve mudar para seta dupla (horizontal, vertical ou diagonal). Fora das bordas, volta à seta normal.
- Se a forma do cursor não mudar, avise: o host pode ignorar a troca de cursor.

## Estado dos testes ao fechar a etapa

- Automáticos: `cargo test` (21 unitários) e `tests/smoke.sh` (9 de 9 na última execução).
- Manuais aprovados pelo usuário: abrir e ver a janela, teclado e foco, arrasto, buscador com digitação, `ctrl+shift+q`, `ctrl+shift+e`, maximizar e restaurar, redimensionar pela borda, cursor de redimensionamento.
- Manuais pendentes: título em texto (ainda não existe) e os itens da seção do terminal `ctrl+shift+t` com `foot` (confirmar que abre dentro do mide).

## Notas do teste 17 (título)

- Confira se o nome da janela (ex.: "foot") aparece centralizado na barra, em cinza-escuro com foco e cinza-claro sem foco.
- Títulos muito longos ainda não são cortados: se passarem dos botões, mande uma foto.

## Notas do teste 18 (título longo)

- Abra um app com título longo (ex.: `foot -T "texto muito longo..."`). O título deve terminar em "…" antes da borda direita, sem passar dos botões.

## B4: dock (esqueleto)

- Subir o mide e depois a dock dentro dele: `WAYLAND_DISPLAY=mide-0 ./target/debug/mide-dock`.
- Confira: a faixa aparece embaixo, com o ícone do foot. A calculadora aparece como bloco cinza (ícone SVG ainda não suportado).
- Clicar no ícone do foot deve abrir um terminal. O clique ainda não foi testado (precisa de clique real).

## Auditoria visual (rodada atual)

- Duas janelas sobrepostas: a de trás deve aparecer inteira, sem faixa escura cobrindo o conteúdo. A sombra só escurece o fundo do desktop.
- Minimizar pelo botão amarelo: a janela encolhe até o centro inferior e some. Restaurar com `ctrl+shift+m`: ela volta com a animação.
- Dock: ícones do foot e da calculadora aparecem, sem blocos cinza.
- Clicar no ícone do foot na dock deve abrir um terminal (ainda não testado).
- Itens que ainda vão mal ou não existem: sombra da dock, restaurar pela dock, blur, transparência e efeito lâmpada.

## Auditoria: janelas com decoração do cliente

- Abrir a calculadora e o foot juntos: o foot tem a barra do mide (com título e botões); a calculadora usa o próprio cabeçalho, sem barra extra e sem deslocamento.
- Arrastar a calculadora pelo próprio cabeçalho e redimensionar pelas bordas (a moldura e os cantos devem acompanhar o conteúdo).

## Clique para fechar a calculadora (pendente)

- Rodar com `RUST_LOG=mide=debug cargo run`. Clicar no X da calculadora e mandar as linhas com `clique`, `janela fechada` e `redimension` do terminal. Assim dá para ver se o clique chega ao app e se a janela foi realmente destruída.

## Bateria automática de interface

```bash
cargo build
tests/ui.sh        # 20 cenários de janelas, controles, popups e dock
tests/smoke.sh     # protocolo, digitação no buscador, ausência de panics
cargo test         # 22 testes unitários
```

Para um roteiro próprio: `tests/scenario.sh "400 list; 410 click 103 33; 450 list" -- foot`.

Manual (o que o autoteste não cobre):
- Aparência geral (cores, sombra, cantos) num monitor real.
- Atalhos de teclado reais (o autoteste só simula ponteiro).
- Cursor de redimensionamento sobre as bordas.
- Efeitos desligados: criar `[effects]` no `~/.config/mide/config.toml` com `shadows = false`, `rounded_corners = false` ou `animations = false`.

## Dock (B4) — manual

- Abrir um app que não está fixado: ele aparece na dock depois de um separador, com o ponto embaixo.
- Clique direito num ícone: fixa ou desafixa (o arquivo `~/.config/mide/dock.toml` é atualizado).
- Arrastar um ícone fixado para o lado: muda a ordem, que fica salva.

## Efeito lâmpada e sombras — manual

- Minimizar uma janela (botão amarelo): ela vira um funil que desce até a dock. Restaurar (clique no ícone da dock ou `ctrl+shift+m`): o funil sai da dock e vira a janela.
- Duas janelas sobrepostas: a sombra de cada uma continua inteira no desktop, sem faixas apagadas perto dos cantos.
- `lamp = false` em `[effects]`: minimizar volta a ser uma escala com transparência.

## Rodada: canto, dock, lâmpada e Ctrl+clique — manual

- Duas janelas sobrepostas: o canto arredondado da janela da frente mostra a janela de trás, sem ponta branca.
- Dock: cantos de 18 px, vidro translúcido, borda mais escura em cima e mais clara embaixo.
- Minimizar: o funil vai até o ícone do app na dock (inclusive depois de reordenar a dock). Restaurar: sai do ícone.
- Maximizar a janela do mide no seu desktop: a dock vai para o centro inferior na hora.
- Ctrl+clique num ícone de app aberto: abre outra janela do app.

## Barra superior (B1) — manual

- A barra aparece no topo, com a marca do mide à esquerda e o relógio à direita.
- O nome do app em foco muda ao clicar em outra janela.
- Maximizar uma janela: ela fica entre a barra e o fundo da tela, sem cobrir a barra.
- Clique na marca: abre o menu. Clique fora: fecha.
- "Sobre este computador": abre uma janela com as especificações da máquina.
- "Encerrar sessão…": fecha o mide.
- Automático: `tests/ui.sh` agora tem 34 verificações (barra, menu, "Sobre", e as posições novas das janelas abaixo da barra), todas passando.

## Papel de parede, barra e dock — manual

- Sem configurar nada: o fundo é um gradiente azul e violeta com brilho quente embaixo à direita.
- Com `[desktop] wallpaper = "/usr/share/backgrounds/<alguma imagem>.jpg"` no `config.toml`: a imagem cobre a tela sem distorcer, e as janelas têm sombra sobre ela.
- Maximizar a janela do mide no seu desktop: o papel de parede se ajusta ao tamanho novo.
- Arrastar uma janela para cima: a barra de título para logo abaixo da barra superior.
- Dock: ícones de 44 px mais próximos entre si, mesmo espaço nas duas bordas, ponto de app aberto centralizado no espaço de baixo, cantos de 15 px.
- Automático: 27 testes unitários; `tests/ui.sh` com 35 verificações (todas passaram; uma vez a verificação "dock volta ao centro inferior" falhou sem gerar listagem e passou 3 de 3 vezes ao repetir).
- Efeito lâmpada: dura 450 ms.

## Launcher (B6) — manual

- `ctrl+shift+space`: o launcher cobre a tela com os apps em grade. De novo `ctrl+shift+space`: fecha.
- Digitar "calc": aparecem só a Calculadora (e outros com "calc"); Enter abre a Calculadora.
- Digitar com acento ou sem ("configuracoes"): acha os apps de configuração.
- Esc com texto: limpa a busca. Esc sem texto: fecha.
- Clicar num grupo (ex.: "Internet"): mostra só os apps dele. "Todos" volta.
- Rolar o mouse ou clicar nos pontos embaixo: troca de página.
- Clicar no vazio: fecha.
- Automático: 6 verificações novas em `tests/ui.sh` (abrir, Esc, atalho de novo, clique no vazio, buscar + Enter, buscar + clique), mais "clique no ícone durante a animação de minimizar restaura".
- Seu `config.toml` foi atualizado: `ctrl+shift+space` agora é `toggle_launcher` (antes era o fuzzel).
- Minimizar e, logo em seguida (antes de a animação acabar), clicar no ícone na dock: a janela volta.
- Clicar na dock por cima de uma janela que não está em foco: a janela de trás não ganha foco nem é redimensionada.

## Downloads e Lixeira (B5) — manual

- No fim da dock, depois de um separador: pasta Downloads e Lixeira.
- Clique em Downloads: painel com os arquivos mais recentes da sua pasta Downloads. Clique num arquivo: abre. "Abrir no Gerenciador de Arquivos": abre a pasta.
- Mandar algo para a Lixeira (no gerenciador de arquivos): o ícone da Lixeira fica cheio em menos de 1 s.
- Clique na Lixeira: abre a Lixeira no gerenciador de arquivos.
- Clique direito na Lixeira → "Esvaziar Lixeira…": aparece a confirmação. "Cancelar" não apaga nada. "Esvaziar Lixeira" apaga para sempre (cuidado: é a sua Lixeira de verdade).
- Clique fora do painel: fecha.
- Automático: 8 verificações novas em `tests/ui.sh`, com pastas temporárias (a sua Lixeira e a sua pasta Downloads nunca são tocadas).

## Rodada: launcher, dock, barra e Sobre — manual

- Launcher (`ctrl+shift+space` ou o primeiro ícone da dock): apps soltos com ícones grandes; na última página, as pastas Utilitários, Sistema e Outros com miniaturas. Clique numa pasta: abre o painel dela; clique fora ou Esc: fecha a pasta.
- Clique direito num app da dock: menu com Abrir/Nova janela, Manter/Remover da Dock, Ocultar e Encerrar (os dois últimos com o app aberto).
- Clique direito num arquivo do painel de Downloads: Abrir, Mostrar na pasta, Copiar caminho, Mover para a Lixeira.
- Barra superior com um app em foco: nome do app (negrito), Arquivo, Editar, Janela. Editar → Copiar/Colar funcionam no app (pelo atalho Ctrl+C/Ctrl+V; no terminal, o atalho dele é outro). Janela → Zoom maximiza.
- "Sobre este computador": arrastar a borda ou o canto redimensiona.
- Automático: `tests/ui.sh` com 60 verificações.

## Notificações (B9) — manual

- Em janela dentro do GNOME, as notificações continuam sendo as do GNOME (é esperado). Para ver as do mide sem sair do GNOME: `dbus-run-session -- cargo run` e, num terminal aberto dentro do mide, `notify-send "Olá" "Teste do mide"`.
- O banner aparece no canto superior direito, abaixo da barra, e some em 5 s.
- Passar o mouse: o banner fica parado e aparece o × no canto.
- `notify-send -A ok=Abrir -A no=Ignorar "Atualizações" "Há 12"`: dois botões; clicar em "Abrir" faz o `notify-send` mostrar `ok`.
- Várias notificações seguidas: empilham, a mais nova em cima.

## Rodada de ajustes 2 — manual

- **Launcher:** a dock continua visível e clara por cima do fundo escurecido; a barra fica transparente com texto branco; a busca diz "Buscar Aplicativos" e fica abaixo da barra.
- **Launcher, roda e botão do meio:** rolar para baixo vai para a página da esquerda, para cima para a da direita; clique do meio vai para a próxima e, da última, volta para a primeira.
- **Launcher, arrastar:** segurar um app e soltar sobre outro cria uma pasta; soltar sobre uma pasta põe dentro; soltar entre itens reordena; levar até a borda troca de página. Clicar no nome da pasta aberta deixa renomear (Enter salva). Arrastar um app para fora do painel da pasta tira ele dela. Fechar e abrir o launcher: continua igual (salvo em `~/.config/mide/launcher.toml`).
- **Downloads:** clique direito num arquivo da lista abre o menu no ponto do clique, sem fechar a lista.
- **Lixeira:** clique mostra os itens apagados; clique direito num item: Restaurar (volta para a pasta de origem) ou Apagar permanentemente.
- **Dock com várias janelas:** clique direito no ícone lista as janelas; escolher uma traz ela para frente. Parar o mouse sobre o ícone de um app aberto mostra as miniaturas das janelas acima da dock; clique numa escolhe, o × fecha.
- **Janela "Sobre":** redimensiona sem travar; minimizada, aparece na dock (ícone de computador) e volta com um clique.
- **Relógio:** clique abre o calendário do mês, com o dia de hoje marcado e ‹ Hoje › para trocar de mês.
- **Som, Wi-Fi, Bluetooth:** ícones à esquerda do relógio; cada clique abre o painel. Atenção: aqui as ações são de verdade (volume, ligar/desligar Wi-Fi e Bluetooth, conectar).
- Automático: `tests/ui.sh` com 79 verificações (com som, Wi-Fi, Bluetooth, Lixeira e abrir arquivos simulados).

## Configurações do Sistema (B7) — manual

- Menu do mide → "Configurações do sistema…": abre a janela com a barra lateral (Wi-Fi, Bluetooth, Som, Papel de parede, Efeitos, Atalhos de teclado, Sobre).
- Papel de parede: clicar numa miniatura troca o fundo na hora. O `config.toml` ganha `[desktop] wallpaper = ...` e continua com os seus comentários.
- Efeitos: desligar "Sombras das janelas" tira a sombra na hora; religar volta.
- Som: arrastar o trilho muda o volume de verdade; "Sem som" silencia.
- Wi-Fi / Bluetooth: interruptores e listas iguais aos dos painéis da barra (ações de verdade).
- Atalhos de teclado: lista os atalhos em uso.
- Pelos painéis da barra, "Configurações de Wi-Fi…" etc. abrem direto na seção.
- Automático: 4 verificações novas em `tests/ui.sh` (com configuração temporária).

## Rodada de ajustes 3 — manual

- Arrastar o volume (barra ou Configurações): acompanha o mouse sem engasgar.
- Maximizar a janela do mide no seu desktop: sem travar; o papel de parede se ajusta logo depois.
- Configurações do Sistema: aparecem no launcher (buscar "configurações") e, se você ainda não tem `dock.toml`, fixadas na dock. Com `dock.toml`: abrir as Configurações, clique direito no ícone → "Manter na Dock".
- Launcher: rolar para baixo vai para a página da direita com o carrossel deslizando; para cima volta. Clique do meio avança e, da última, volta para a primeira.
- Launcher: abre esmaecendo com uma leve aproximação; fecha esmaecendo.
- Arrastar um app sobre outro: a pasta surge crescendo.
- Clicar num app fechado da dock: a janela sai do ícone com o efeito lâmpada. Pelo launcher, abre sem o efeito.
- Dock: ícones um pouco menores (40 px).
- Configurações → Dock: arrastar "Tamanho dos ícones" muda a dock em menos de 1 s, sem reiniciar. A prévia mostra o tamanho escolhido.
- Clique direito no ícone do Launchpad: "Ajustes da Dock…" abre as Configurações na seção Dock.

## Rodada de ajustes 4 — manual

- Configurações → Efeitos: desligar e religar cada efeito muda na hora (sombras, cantos, animações, lâmpada, desfoque na dock, desfoque no launcher).
- Dock por cima de uma janela ou do papel de parede: aparece desfocado atrás dela (vidro fosco).
- Launcher: o fundo é o papel de parede atual desfocado.
- Pasta com mais de uma página (ex.: Utilitários): a roda troca de página deslizando dentro do painel.
- Configurações com conteúdo longo (Papel de parede): barra de rolagem à direita, que pode ser arrastada.
- Papel de parede → "+ Adicionar imagem…": escolher uma imagem da pasta Imagens; ela aparece na lista, selecionada e aplicada. Com o mouse em cima dela, o × tira da lista.

## Framework mide-ui — manual

- Dentro de uma sessão do mide: `cargo run --example mide-ui-demo`. A janela tem a cara das Configurações.
- Controles: os interruptores trocam; o trilho de volume acompanha o mouse; digitar um nome e apertar Enter (ou "Cumprimentar") mostra a saudação; a linha de "Estado" acompanha.
- Tarefas: clicar numa tarefa marca/desmarca; digitar uma nova e Enter adiciona; "Limpar concluídas" remove as feitas.
- Automático: 4 testes do mide-ui em `cargo test`.

## Launcher rápido — manual

- Abrir o launcher: aparece esmaecendo e se aproximando, sem engasgar. Fechar (Esc, clique no vazio): esmaece suave.
- Rolar entre páginas várias vezes seguidas: o carrossel desliza liso, inclusive na primeira troca e dentro das pastas.
- Passar o mouse pelos apps: o destaque segue sem atraso e cobre o ícone e o nome, com o mesmo vidro da dock.

## Arquivos (B8) — automáticos

- `cargo test`: 5 testes de `mide::files` (ordem com pastas primeiro e sem acentos, ocultos, filtro, tamanhos e datas, nome livre de "Nova pasta", renomear sem sobrescrever) e 4 novos do mide-ui (grade que quebra linha, duplo clique, menu de contexto abre/escolhe/fecha ao clicar fora, foco com tudo selecionado).
- `tests/ui.sh`, 11 novas verificações numa pasta de exemplo recriada a cada cenário (a Lixeira é a do teste, em `XDG_DATA_HOME` temporário): título da janela é a pasta; duplo clique entra numa pasta; Backspace e o botão voltar sobem; Lixeira na barra lateral; Delete manda para a Lixeira; menu do vazio → Nova pasta já pedindo o nome; Esc cancela o nome; lista + menu do item → Mover para a Lixeira; menu do item → Renomear; clique fora fecha o menu sem fazer nada.

## Arquivos (B8) — manual

- Abrir "Arquivos" pelo launcher (ou `mide-files`). Clicar nos Favoritos troca de pasta; voltar/avançar seguem o histórico.
- Pasta com fotos: as miniaturas aparecem aos poucos, sem travar a janela.
- Alternar ícones/lista; digitar na busca filtra na hora.
- Botão direito num arquivo e no vazio: menus do lugar do clique; clicar fora ou Esc fecha.
- Nova pasta (menu ou Ctrl+N): digitar o nome substitui "Nova pasta"; Enter confirma.
- Mover um arquivo para a Lixeira, abrir a Lixeira na barra lateral e restaurar pelo menu: ele volta para a pasta original.
- Dock → Downloads → "Abrir": abre no Arquivos.

## Vidro líquido — automáticos

- `tests/ui.sh`: Configurações → Efeitos → "Vidro líquido" desligado grava `liquid_glass = false` e a dock troca para o vidro clássico (log da dock).
- Medição (manual, com `RUST_LOG=mide::blur=trace`): contar "dock: desfoque refeito" e os quadros; parado, 1 desfoque em ~430 quadros.

## Vidro líquido — manual

- Dock sobre um papel de parede com detalhes (listras ou foto): nas pontas da dock, o que está atrás aparece curvado, como através de vidro grosso; borda com reflexo claro, mais forte em cima.
- Janela passando por trás da dock: o vidro acompanha sem atraso.
- Configurações → Efeitos → desligar "Vidro líquido": a dock volta ao vidro fosco clássico, na hora.

## Desempenho — automáticos

- `tests/perf.sh` (ou `tests/perf.sh dock|desktop|launcher|arquivos`): tempos de quadro e de resposta. Referência atual (saída 800x600): compositor ~0,25 ms; dock ~1,1 ms; menu da dock ~3 ms; launcher ~3,2 ms (p95 ~4 ms); Arquivos ~5 ms; resposta do Arquivos ~5,6 ms (p95 ~9 ms); resposta do launcher ~3,4 ms.
- Bancadas do mide-ui (sem tela): `cargo test --lib bancada -- --ignored --nocapture --test-threads 1`.
- `blit_igual_ao_draw_pixmap`: a colagem rápida dá o mesmo resultado do tiny-skia (tolerância de 2 níveis).
- Estresse de início de sessão (o travamento do desfoque): 20 sessões seguidas, todas com ~190 quadros em 3 s.

## 1366x768

- `tests/res1366.sh`: 8 verificações de layout em 1366x768 e capturas em `/tmp/mide-1366` (`RES_OUT` muda a pasta).
- Manual: `cargo run` abre em 1366x768; conferir que nada fica cortado (janelas, menus, launcher, Configurações).

## Papel de parede

- Prévia e tempo: `MIDE_PREVIA=/tmp/fundo.png cargo test --lib previa_papel -- --ignored --nocapture`.

## Vidro em barras e menus — automáticos

- `tests/ui.sh`: barra superior, menu da barra e menu da dock sobre vidro (log `vidro: desfoque refeito (<camada>#<n>)`), e sem vidro com `panel_glass = false`.
- `cargo test`: `regioes_de_vidro_sao_interpretadas` (formato do comando `glass`).

## Vidro em barras e menus — manual

- Barra superior translúcida, mostrando o papel de parede desfocado; menus da barra e da dock e notificações também.
- Janela passando atrás da barra ou de um menu aberto: o desfoque acompanha.
- Configurações → Efeitos → desligar "Vidro em barras e menus": barra e menus voltam ao fundo claro quase opaco.

## Buscador (B11) — automáticos

- `tests/ui.sh`: ctrl+space abre em tela cheia; buscador sobre vidro; buscar "calc" e Enter abre a calculadora; Esc fecha; o atalho de novo fecha; clique fora fecha; abrir um app fecha o buscador.
- `cargo test`: `mide::search` (contas, números, índice e busca por começo de palavra, seções das Configurações, pasta curta), `ui::draw` (pintura sólida igual a apagar e pintar), `toggle_search` no config.
- `tests/perf.sh buscador`: abrir, digitar letra a letra, setas e Esc.
- Bancada: `cargo test --lib bancada_painel -- --ignored --nocapture`.

## Buscador (B11) — manual

- ctrl+space: a barra aparece sobre o vidro. Digitar "calc", "volume", "2+2", o nome de um arquivo da pasta Documentos. Setas e Enter; Esc ou clique fora fecham.
