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

## Rodada de ajustes — automáticos

- `tests/ui.sh` (agora leva ~30 min; rodar sem limite de tempo):
  - Barra: relógio abre o calendário; "Formato da hora" liga os segundos; central de controle abre; Não perturbe; engrenagem abre as Configurações; Repouso e Reiniciar (com confirmação) simulados; Cancelar não reinicia.
  - Teclas no estilo Mac: Command+Espaço abre o buscador.
  - Configurações: ocultar a dock automaticamente (a dock esconde e grava `autohide`); busca "teclas" leva à opção e grava `mac_style`; coordenadas novas para papel de parede, efeitos e tamanho da dock.
  - Arquivos: setinha abre a pasta na lista; Novo arquivo com a extensão digitada; Comprimir cria o .zip; Ctrl+D duplica; Ctrl+C e Ctrl+V copiam para outra pasta.
  - Maximizar agora para acima da dock (800x477 na saída de 800x600).
- `cargo test`: `mide::files` (tipos, novo arquivo, duplicar, colar, tamanho de pasta, zip), relógio da barra, brilho do `brightnessctl`, busca das Configurações.
- O autoteste ganhou a tecla `.` (para digitar nomes com extensão).

## Rodada de ajustes — manual

- Launcher: fechar não pisca mais os ícones.
- Arquivos em lista: setinhas, linhas alternadas, ordenar pelo cabeçalho; botão direito numa pasta → Comprimir; num .zip → Extrair aqui; no vazio → Novo arquivo.
- Dock: maximizar uma janela para acima da dock; Configurações → Dock → ocultar automaticamente; levar o ponteiro à borda de baixo.
- Barra: clicar no relógio → "Formato da hora…"; central de controle; menu do mide → Reiniciar… → Cancelar.
- Configurações: digitar na busca; Teclado → Teclas no estilo Mac; Command+C/V nos apps.

## Ajustes da dock e da barra — automáticos

- `tests/ui.sh`: ocultar inteligente (sem janela, a dock fica; com o foot por cima, esconde); ocultar pelas Configurações esconde com a janela maximizada; maximizar com 2 px de respiro (800x475); confirmação de reiniciar no meio da tela.
- `tests/perf.sh previas`: prévias abertas e ponteiro numa miniatura.

## Ajustes da dock e da barra — manual

- Configurações → Dock → ocultar: sem janela sobre a dock ela fica; maximizar uma janela a faz sumir; ponteiro na borda de baixo a traz de volta.
- Duas janelas do mesmo app: ponteiro no ícone da dock e depois numa miniatura: vidro claro em volta e o × escuro no canto.

## Central de notificações — automáticos

- `tests/ui.sh` (barramento D-Bus de teste): notificação vai para o histórico; "Limpar" no painel do relógio apaga o histórico; com Não perturbe, o banner não aparece e a notificação fica no histórico.
- `cargo test`: formato do histórico (`historico_guarda_e_le_de_volta`).

## Central de notificações — manual

- `notify-send "Teste" "Olá"` algumas vezes; clicar no relógio: as notificações aparecem embaixo do calendário; "Limpar" apaga.
- Ligar o Não perturbe na central de controle, mandar uma notificação: não aparece banner, mas ela está no relógio.

## Diálogo do Arquivos — automáticos

- `tests/ui.sh`: Esc no diálogo não cria nada; Cancelar não cria nada; Criar sem mudar o nome cria "Nova pasta" e depois "Nova pasta (1)"; com nome digitado cria com ele.
- `cargo test`: `nomes_numerados_e_criar_com_nome`, `pixels_premultiplicados_validos`.

## Área de trabalho (B10) — automáticos

- `tests/ui.sh`: camada de fundo aparece; duplo clique no ícone abre o arquivo (simulado); Delete manda para a Lixeira; menu do fundo → Nova pasta.
- `cargo test --bin mide-desktop`: ícones em colunas a partir da direita, dentro da área útil.
- `tests/perf.sh desktop`: com a área de trabalho, o vidro da dock continua refeito só ao aparecer (2 vezes).

## Área de trabalho (B10) — manual

- Pôr arquivos na pasta "Área de trabalho": aparecem à direita; duplo clique abre; botão direito no vazio → Nova pasta (diálogo); arrastar janelas por cima: os ícones ficam atrás.

## Menus, vidro na janela e criar dentro da pasta — automáticos

- `tests/ui.sh`: coordenadas dos menus do Arquivos recalibradas (menu embaixo do item); "Nova pasta aqui dentro" e "Novo arquivo aqui dentro" criam dentro de "Projetos"; "clique fora fecha o menu" agora clica onde estava "Mover para a Lixeira"; a barra lateral do Arquivos aparece com vidro (`app:mide-files:N#0`).

## Menus, vidro na janela e criar dentro da pasta — manual

- Arquivos sobre o papel de parede: a barra lateral deixa ver o fundo desfocado; desligar "Vidro em barras e menus" nas Configurações: fica opaca em até 1 s.
- Trocar o papel de parede: a imagem nova esmaece sobre a antiga e o vidro da dock acompanha (não fica a imagem antiga).
- Abrir e fechar o launcher: a barra superior esmaece junto. Fechar uma janela: ela encolhe um pouco e some.

## Itens da barra superior (B12) — automáticos

- `tests/ui.sh`: Configurações → Barra superior → desligar "Configurações" grava `gear = false` e a barra passa a mostrar só `sound wifi bluetooth battery control`.

## Itens da barra superior (B12) — manual

- Configurações → Barra superior: desligar Som e Bluetooth; os ícones somem da barra em meio segundo e os outros se ajeitam; religar traz de volta.

## Compositor parado — medição

- Sessão sem autoteste (`tests/scenario.sh "" -- "target/debug/mide-files /usr/share"`), 10 s parado: quadros do compositor e CPU por processo (`/proc/<pid>/stat`). Resultado: 0–2 quadros, 1,0% de um núcleo no total.
- Mesma sessão, `launcher` pelo canal de controle: o compositor desenha a animação (23 quadros ao abrir, 30 ao fechar) e volta a parar.

## Refatoração da cena e da política de janelas — automáticos

- `tests/ui.sh` (153 verificações) rodado antes e depois da refatoração, no Xvfb (`xvfb-run -a -s "-screen 0 1920x1080x24" tests/ui.sh`), numa máquina sem `foot` e `gnome-calculator`: resultado idêntico item por item (82 ok, as mesmas 71 falhas de ambiente). O Xvfb roda a bateria sem abrir janelas na tela de quem está usando o PC.

## Editor de atalhos (A9) — automáticos

- `cargo test`: `none` desliga um atalho padrão; `combo_name` gera o nome que volta a ser lido; combinações sem Ctrl/Alt/Super recusadas; atalho de app mostra o nome do app; limpar seção mantém o resto.
- `tests/ui.sh` (Configurações): gravar Ctrl+Alt+K para "Fechar a janela" grava `"ctrl+alt+k" = "close_window"` e `"ctrl+shift+q" = "none"`; Ctrl+Alt+K fecha a janela; Ctrl+Shift+Q não fecha mais; gravando Ctrl+Espaço o buscador não abre e nada é gravado sem confirmar; "Restaurar os padrões" tira `[keybindings]`.

## Editor de atalhos (A9) — manual

- [ ] Configurações → Teclado: clicar em "Fechar a janela" → Gravar… → apertar Ctrl+Alt+K: a lista mostra "Ctrl Alt K" e o atalho fecha janelas.
- [ ] Gravar uma combinação usada por outro atalho (ex.: Ctrl+Espaço): aparece o aviso com Substituir e Gravar outra.
- [ ] Gravar só uma letra (sem Ctrl/Alt/Super): aparece o aviso para usar um modificador.
- [ ] Novo atalho… → digitar o nome de um app → escolher → Gravar… → apertar a combinação: o atalho abre o app.
- [ ] Esc durante a gravação cancela; fechar as Configurações no meio não prende o teclado (expira em 20 s).
- [ ] Restaurar os padrões volta tudo ao original.

## Backend DRM (A13) — manual (num console de texto)

Preparação: compilar antes, na sessão gráfica (`cargo build`). Depois, Ctrl+Alt+F3, entrar com o usuário e:

```bash
cd ~/Proyectos/Fabio/mide
timeout 120 ./target/debug/mide 2> /tmp/mide-drm.log
```

O `timeout 120` encerra o mide sozinho em 2 minutos no primeiro teste (rede de segurança). Para voltar à sessão gráfica: o console dela (`loginctl show-session $XDG_SESSION_ID -p VTNr` na sessão gráfica; no Debian do trabalho, com LightDM, é Ctrl+Alt+F7).

- [ ] A tela mostra o papel de parede, a barra e a dock, na resolução nativa do monitor.
- [ ] O cursor aparece e acompanha o mouse/touchpad; não sai da tela.
- [ ] Clicar na dock e no menu do mide funciona; toque no touchpad clica.
- [ ] Ctrl+Espaço abre o buscador e o teclado digita com acentos do layout do sistema (ABNT2: `ç`, `´`+`a`).
- [ ] Abrir as Configurações; arrastar a janela; o cursor muda nas bordas (redimensionar).
- [ ] Ctrl+Alt+F2 (ou outro console) sai do mide; voltar para o console do mide (Ctrl+Alt+F3) mostra a tela de novo, com mouse e teclado funcionando.
- [ ] Ctrl+Shift+Esc encerra o mide e devolve o console de texto.
- [ ] Se algo falhar, mandar o `/tmp/mide-drm.log`.

## Backend DRM (A13) — automáticos (no winit)

- `cargo test`: 109 (cursor: nomes do tema e reservas; teclado do sistema lido de `/etc/default/keyboard`).
- `tests/ui.sh` no Xvfb depois do A13: os mesmos resultados da referência mais os 6 do editor de atalhos (87 ok); a única diferença ("tamanho dos ícones muda a dock") passou ao repetir (instável, ver anotações). O DRM em si só roda num console de texto: teste manual acima.

## Menu ao lado e cantos ao fechar — automáticos

- `cargo test` (`kit::ui`): menu embaixo quando cabe, em cima quando não cabe embaixo, ao lado (direita, ou esquerda perto da borda) quando não cabe em nenhum, sempre dentro da janela.
- `tests/ui.sh` (área de trabalho, vidro e Arquivos): mesmos resultados de antes.
- Conferido em quadro salvo: a foto da janela fechando mantém os cantos arredondados; o menu de um item no meio da lista abre à direita do clique com a ponta na lateral.

## Menu ao lado e cantos ao fechar — manual

- [ ] Arquivos com uma pasta de muitos itens, janela baixa: clique direito num item do meio; o menu abre ao lado, sem cobrir o item, com a ponta apontando para ele. Perto da borda direita da janela, abre à esquerda.
- [ ] Fechar uma janela: ela encolhe e esmaece com os cantos arredondados (sem cantos vivos).

## XWayland (A14) — automáticos

- `cargo test`: display X livre pula sockets e travas ocupados.
- `tests/ui.sh` (bloco "apps X11"): `xclock` aberto pelo display do XWayland do mide vira janela do mide com a barra de título; fechar pelo botão da barra fecha o app X11. Pulado se faltar `xclock` ou `xwayland-satellite`.

## XWayland (A14) — manual

- [ ] `cargo run` (ou no console, A13): o log mostra `XWayland: apps X11 no display :N`.
- [ ] Abrir um app X11 pelo mide (ex.: atalho novo nas Configurações para `xterm`, ou o terminal pelo menu do Arquivos quando ele for X11): abre dentro do mide, com a barra do mide, e digita normalmente.
- [ ] Um app X11 grande (IDE da JetBrains sem Wayland ativado, ou outro só-X11) abre, aceita teclado, mouse e copiar/colar com apps Wayland.

## Campo de texto, área de transferência e Editor de Texto — automáticos

- `cargo test`: `kit::textedit` (digitar no meio, acentos, seleção com Shift, palavras, tudo selecionado, duplo clique, cursor fora do texto), `kit::textarea` (linhas, coluna por largura, desfazer juntando a digitação, recuo), `files::is_text`, `mide-text` (salvar mantém CRLF, binário não abre, buscar e substituir com desfazer).
- `tests/ui.sh` (editor de texto): editar no fim e salvar com Ctrl+S; copiar num editor e colar em outro (Wayland) e salvar; título com "•" quando alterado; fechar com alteração pergunta (a janela continua).

## Campo de texto, área de transferência e Editor de Texto — manual

- [ ] Configurações → busca: clicar no meio do texto põe o cursor ali; Shift+setas seleciona; Ctrl+A, Ctrl+C e colar em outro app (ex.: um terminal Wayland).
- [ ] Digitar acentos no campo e no editor (ABNT2: `´`+`a`, `~`+`a`, `ç`).
- [ ] Arquivos: duplo clique num `.txt`/`.md`/`.rs` abre no Editor de Texto; editar, Ctrl+S, conferir o arquivo; Ctrl+Z desfaz.
- [ ] Editor: Ctrl+F busca, Substituir tudo; fechar com alteração pergunta; Não salvar fecha sem gravar.
- [ ] Abrir um app e digitar logo em seguida: as teclas vão para ele (não para a área de trabalho).

## Sessão de login (C2) — automáticos

- `cargo test` (`session::tests`): regras de autostart (substituição pelo arquivo do usuário, `Hidden`, `OnlyShowIn`, `NotShowIn`, `TryExec`, `X-GNOME-Autostart-enabled`, códigos `%U` tirados do `Exec`).

## Sessão de login (C2) — manual

Preparação: `cargo build` e `sudo tests/sessao-dev.sh`. Encerrar a sessão atual (Cinnamon) e, na tela de login, escolher "mide (desenvolvimento)" no seletor de sessão.

- [ ] Entra direto no mide, na resolução do monitor, com barra, dock e papel de parede.
- [ ] Mouse, teclado (ABNT2) e touchpad funcionam; apps abrem pela dock e pelo launcher.
- [ ] Apps de inicialização automática do sistema sobem (ex.: o applet de rede ou o gnome-keyring, conforme a distribuição): conferir em `~/.local/state/mide/sessao.log` as linhas "sessão: iniciando".
- [ ] Notificações de outros apps chegam (o ambiente foi publicado para o D-Bus).
- [ ] Menu do mide → Encerrar sessão volta à tela de login.
- [ ] Se algo falhar: Ctrl+Alt+F3, entrar no console e mandar o `~/.local/state/mide/sessao.log`. Para tirar a entrada: `sudo tests/sessao-dev.sh remover`.

## Bateria completa depois de A14, campo de texto, editor e foco

- `tests/ui.sh` no Xvfb: todas as verificações antigas com o mesmo resultado da referência (as falhas restantes são de ambiente: sem `foot`, `gnome-calculator` e o papel de parede do Zorin); 10 novas passando (editor de atalhos e editor de texto). Os testes de XWayland passaram a começar no quadro 2000 (o xclock abre ~3 s depois do mide) e o de fechar exige que a janela apareça antes do clique e suma depois.

## Orientação do DRM — automáticos

- `tests/res1366.sh`: com `MIDE_SIM_DRM=1`, o Arquivos aparece inteiro, com a barra de título e a barra de ferramentas claras (foto em `/tmp/mide-1366/arquivos-drm.png`). Com o recorte antigo, essa verificação falha (barra escura e topo da janela sumindo).

## Orientação do DRM — manual

- Entrar na sessão "mide (desenvolvimento)" (ou `data/mide-session` num console): janelas inteiras, cantos arredondados, barra de título clara, vidro da dock e da barra lateral do Arquivos mostrando o que está atrás deles.

## Backend sem tela e atualizar sem deslogar — automáticos

- Bateria inteira no backend headless (`tests/scenario.sh` usa ele por padrão), rodada de dentro da sessão do mide: `ui.sh` 157 ok e 6 falhas (todas da Calculadora, ambiente: ver anotações), `res1366.sh` 9 ok, testes unitários ok.
- `mide-ctl restart shell` num mide headless: barra, dock, área de trabalho e notificações trocam de processo e o Arquivos continua aberto; `mide-ctl restart all`: o compositor reinicia no mesmo pid e volta a responder.

## Atualizar sem deslogar — manual

- Na sessão: `data/mide-atualizar` (barra, dock... trocam; janelas ficam) e `data/mide-atualizar --tudo` (compositor reinicia; volta sem passar pelo login; os apps automáticos como o nm-applet voltam, os que seguiam abertos não duplicam).
- Chrome: menu ⋮ → Histórico: o submenu abre para a esquerda, dentro da tela.
- Arquivos: a barra lateral de vidro mostra o papel de parede na posição certa.

## Monitores, captura, menu global, animações e dock — automáticos (`tests/ui.sh`, 1366x768)

- Monitores (telas virtuais): duas telas lado a lado; barra na principal escolhida no config; tela desligada fica de fora; Configurações → Monitores: tornar principal e manter (barra vai para a outra tela, `primary` gravado); sem confirmar, volta sozinho em 15 s; escala 125% numa tela de 1920x1080 dá 1536x864 pontos.
- Captura de tela: Print, arrastar uma área e Ctrl+S grava 400x250 em Imagens/Capturas de tela (pasta temporária); Enter copia a imagem (`image/png`).
- Menu global: com `examples/mide-menu-teste` (barramento de teste), a barra mostra "Projeto, Ajuda" e o clique em Projeto → Compilar chega ao app.
- Dock: clique no app em foco minimiza; de novo restaura; com duas janelas mostra as prévias; lista de Downloads e da Lixeira continua aberta depois de agir num item; Esc fecha a lista; "Copiar caminho" da dock cola no Editor de Texto (Ctrl+V).
- Prévias com o launcher aberto: escolher fecha o launcher.
- `cargo test`: arranjo das telas (`outputs`), leitura da lista de telas e gravação de `[monitors]` (`displays`), encaixe da tela arrastada, leitura do dbusmenu e casamento do processo com o app (`appmenu`), recorte da captura com as marcações.

## Monitores, captura, menu global, animações e dock — manual (na sessão)

- Configurações → Monitores com os dois monitores: os dois acendem; arrastar a segunda tela para o outro lado; trocar a principal (barra e dock mudam); trocar resolução/frequência/escala e "Manter"; "Reverter" ou esperar 15 s volta ao anterior; desconectar um cabo com o mide aberto.
- Print: seleção, janela, tela inteira; desenhar seta/círculo/quadrado nas 6 cores; colar a imagem num app.
- VS Code pelo XWayland (com o `xwayland-satellite`): a barra mostra os menus dele.
- Efeitos: desligar "Animação dos menus" e "Transições suaves" e conferir que somem.

## Atalhos de janela — automáticos (`tests/ui.sh`)

- Alt+Tab foca a outra janela; Super+← e Super+→ deixam a janela na metade (683x643 em 1366x768); Super+↑ maximiza; Super+↓ restaura; Super+H minimiza; Alt+F4 fecha.
- `cargo test`: nomes sem diferenciar maiúsculas (`nomes_numerados_e_criar_com_nome`).

## Atalhos e prévias — manual

- Na sessão: Alt+Tab com várias janelas (segurar o Alt e apertar Tab várias vezes; Shift volta; Esc cancela); Super+D; Super+L.
- Passar o mouse num ícone da dock com janelas abertas: as prévias entram esmaecendo; com o mouse em cima delas, o cursor é a seta.

## Refinos do launcher e terminal — manual

- Launchpad: abrir e fechar uma pasta; a grade some e volta junto com o painel. Trocar de página (roda, arrastar): só os ícones deslizam, as bolinhas ficam paradas.
- Arquivos aberto pela dock: depois da lâmpada, o vidro da barra lateral aparece suave.
- Estilo Mac ligado, no foot: ⌘C copia, ⌘V cola, Ctrl+C interrompe um comando.

## Atalhos em grupos e Mouse e trackpad — automáticos

- `tests/ui.sh`: editar atalho usando a busca ("fechar" → "Fechar a janela" (Ctrl ⇧ Q) → gravar Ctrl+Alt+K); combinação já usada não dispara; "Restaurar os padrões" (com a busca vazia de resultados); Mouse e trackpad gravam `mouse_natural_scroll`, `tap_to_click` e `pointer_speed` no config.
- `cargo test`: seção `[input]` (`mouse_e_trackpad_no_config`).

## Mouse e trackpad — manual (na sessão)

- Mudar a velocidade e ver o ponteiro mudar na hora; desligar o toque para clicar no trackpad do notebook; rolagem natural no mouse.

## Identidade visual, vidro nas pastas e correções — automáticos

- `tests/scenario.sh`: binários de `target/debug` na frente do `PATH` (a tecla Print abria o mide-screenshot instalado na sessão de quem roda os testes); apps GTK com configurações padrão (`GDK_DEBUG=no-portals ADW_DISABLE_PORTAL=1 GSETTINGS_BACKEND=memory`: a Calculadora não pega tema e botões do usuário); linhas da área de transferência na saída.
- `tests/ui.sh`: Calculadora no layout padrão do GTK (só o X; maximizar com duplo clique no cabeçalho; minimizar com Super+H nos testes da dock); testes de vidro com o nome da tela no cache (`HEADLESS-1:mide-bar#0`); tela principal na origem quando o config não dá posições.
- `cargo test`: `parse_glass` com opacidade.
- Medido (headless, 1366x768, build de depuração, máquina com carga 5): quadro da pasta do launcher 11,8 ms de mediana com o vidro líquido e 11,3 ms sem (o vidro custa ~0,5 ms no app); carrossel 1,6 ms; compositor na GPU 1,2 ms de mediana, p95 3,6 ms.

## Vidro nas pastas — manual (na sessão)

- Abrir o Launchpad: o papel de parede aparece com desfoque leve; as pastas têm vidro com borda clara. Abrir uma pasta: o painel cresce em vidro; trocar de página: o vidro das pastas desliza junto. Desligar "Vidro líquido" em Efeitos: pastas no visual antigo.
- Copiar caminho (Downloads da dock) e abrir um app do mide em seguida: Ctrl+V cola de primeira.

## Terminal e Calculadora do mide — automáticos

- `cargo test`: decodificador VT500 (UTF-8, CSI com subparâmetros, OSC com BEL e ST, cancelamento), tela (quebra automática, histórico, cursor e apagar, região de rolagem, cores 16/256/RGB, largos e combinados, tela alternativa, respostas DA/DSR/DECRQM/OSC 11, título, OSC 52/7/133, reflow ao mudar largura e altura, seleção por palavra atravessando quebras, mouse SGR/X10, desenho de linhas do DEC, busca sem acentos), teclas e colar, PTY real (`stty size`, `tty`), larguras, temas, painéis (dividir, fechar, áreas, divisores, vizinhos), perfis (campos faltando, nomes repetidos, limites), atalhos do Mac e do Linux, contas da calculadora.
- `tests/ui.sh` usa o Terminal (`--title Terminal`) e a Calculadora do mide em todos os testes de janela, dock e prévias; os de cabeçalho do app (CSD) só com a gnome-calculator instalada.

## Terminal do mide — manual (na sessão)

- Abrir pela dock e pelo Ctrl+Shift+T (com o terminal em foco, vira nova aba); `ls --color`, `vim`, `top`, `less` (roda do mouse vira setas), `htop` com o mouse.
- ⌘D e ⌘⇧D, arrastar o divisor, ⌘⌥ setas, ⌘⇧Enter; ⌘T, ⌘1…9, fechar aba pelo ×; ⌘F e buscar; ⌘, e trocar tema/fonte/cursor (o terminal aberto muda em até 1 s); ⌘+/⌘−.
- Selecionar com duplo e triplo clique, ⌘C/⌘V no estilo Mac, Ctrl+C interrompe.
- Fechar com `vim` aberto: pergunta antes.

## Menus do app na barra — automáticos

- `cargo test`: números estáveis e ações do dbusmenu publicado pelo mide-ui, a barra lê o que o mide-ui publica (títulos, atalho, separador), atalhos com símbolos do Mac, ações dos menus do Terminal vão e voltam.
- `tests/ui.sh`: menus padrão do mide com a Calculadora (Editar, Janela > Zoom); num barramento de teste, a barra mostra Shell, Editar, Visualizar, Perfis e Janela do Terminal, e Janela > Zoom pelo menu do app maximiza.

## Menus do app e teclas presas — manual

- Mide numa janela do X11: Alt+Tab para outra janela e voltar; digitar no Terminal escreve normalmente.
- Terminal em foco: os menus da barra são os dele; Shell → Dividir lado a lado divide; Perfis → Nova aba com outro perfil.

## Terminal: vidro, cores e sessão — automáticos

- `cargo test`: cores trocadas por cima do tema (e limpeza de chaves e valores inválidos ao ler), opacidade com limite, sessão guardada e remontada (painéis, pastas, perfis) e painel que não abre some.

## Terminal: vidro, cores e sessão — manual

- Preferências → Transparência: opacidade 70% e Vidro: o papel de parede aparece desfocado atrás do texto, sem emendas entre painéis.
- Preferências → Cores: trocar o Fundo para `#102030`; o terminal aberto muda em até 1 s; "Usar as cores do tema" volta.
- Arrastar uma aba para o outro lado.
- Ligar "Reabrir abas e painéis", dividir, abrir outra aba, fechar a janela e abrir o Terminal de novo pela dock.
- `cargo test`: endereços sob a célula (parênteses equilibrados, pontuação final, quebrado em duas linhas, `mailto:`).
- Manual: `echo https://github.com/fabioaacarneiro/mide`, segurar Ctrl (ou ⌘) e passar o mouse: sublinha; clicar abre no navegador; botão direito em cima: "Abrir endereço" e "Copiar endereço".

## Terminal: integração do shell, gatilhos e imagens — automáticos

- `cargo test`: bash de verdade com o script de integração num PTY (marca A, C e D;1 depois de `false`, `imgcat` disponível); marcas com código de saída, D sem C não conta; busca por faixa de linhas (gatilhos); imagem PNG pelo `OSC 1337` (posição, tamanho automático e em células, cursor abaixo, sem `inline=1` não aparece, `clear` tira); duração por extenso.

## Terminal: integração do shell, gatilhos e imagens — manual

- No bash (e no zsh, se tiver): `false`, `ls`: marcas vermelha e azul na margem; ⌘⇧↑ volta ao prompt anterior.
- `sleep 12`, trocar para outra janela: ao terminar chega a notificação "Comando terminado".
- Preferências → Gatilhos: "erro" → Destacar; `echo deu erro`: "erro" em amarelo.
- `imgcat` com uma foto: a imagem aparece e rola com o texto; `clear` tira.

## Terminal de atalho, tmux e imagens — automáticos

- `cargo test`: DCS em pedaços e APC no decodificador; Sixel (cores, repetição, faixas, HLS); kitty (partes, mostrar, guardar, apagar, consulta, silêncio, formato desconhecido); Sixel, kitty e o fluxo do tmux pela tela; protocolo do tmux (escapes octais, avisos, respostas casadas com os comandos com avisos no meio, layout com divisões aninhadas e proporções, `send-keys` em hexadecimal); `--hotkey`.
- Manual na cópia de teste: F12 abre a camada `mide-terminal-atalho` (1366x673 abaixo da barra), F12 esconde (0x0), F12 mostra de novo; `tmux -L mide-teste -f /dev/null -CC new-session`: a janela vira aba, `echo` e `pwd` vão ao painel do tmux, Ctrl+Shift+D divide pelo tmux; Sixel 120x48 e kitty 64x24 aparecem.

## Terminal de atalho, tmux e imagens — manual (na sessão)

- F12 em qualquer lugar: o terminal desce; digitar; F12 esconde; F12 volta com o mesmo histórico. Preferências → Sessão: altura do terminal de atalho.
- `tmux -CC` (ou `tmux -CC attach`): janelas viram abas; dividir, nova aba, fechar aba; Esc no painel do tmux desanexa e as abas somem.
- `img2sixel foto.png` (pacote libsixel-bin), `kitty +kitten icat foto.png` (se tiver o kitty) ou `chafa -f sixels foto.png`.

## Perfil padrão, fonte e placeholders — automáticos e manuais

- `cargo test`: colocação virtual do kitty (`U=1`), acentos de linha/coluna, perfil vazio é o padrão.
- Manual (cópia de teste): perfil padrão com opacidade 0,6 e fonte "Courier 10 Pitch": o terminal aberto sem escolher perfil fica translúcido e com a fonte; placeholders 8x2 de uma imagem 4x2 aparecem; `SNACKS_KITTY=1 nvim --clean -u mini.lua imagem.png` mostra a imagem.
- Manual (sessão): trocar o perfil padrão nas Preferências muda os terminais abertos; Fonte → outra família; abrir uma imagem no Neovim com a linha do `SNACKS_KITTY` na configuração.

## Bloqueio de tela — automáticos

- `cargo test`: o PAM de verdade recusa uma senha errada (sem travar); usuário e serviço do PAM.
- `tests/ui.sh` (senha de teste `MIDE_LOCK_TEST_PASSWORD`, só em compilação de desenvolvimento): Super+L bloqueia (1 tela de bloqueio); senha errada mantém bloqueada; senha certa desbloqueia.
- Conferido à parte: o que se digita com a tela bloqueada não chega às janelas (o terminal de trás fica limpo); matar o `mide-lock` com a tela bloqueada: o mide abre outro e a tela não desbloqueia.

## Bloqueio de tela — manual (sessão de verdade)

- Super+L; senha errada e certa; `loginctl lock-session`; suspender e voltar (pede a senha); ficar 5 min sem usar; Configurações → Bloqueio de tela → "Bloquear agora"; com dois monitores, a outra tela fica escura.
- Instalar o serviço do PAM: `sudo cp data/pam/mide-lock /etc/pam.d/` (sem ele, o mide-lock usa o `login`).
- `tests/ui.sh`: o bloqueio cobre os dois monitores (2 telas de bloqueio); a tela apaga depois do tempo sem uso e acende ao mexer o mouse; bloqueia sozinha depois do tempo sem uso.
- Manual (sessão de verdade): os monitores desligam (DPMS) e ligam com uma tecla; um vídeo tocando no Firefox ou no mpv impede o bloqueio e o apagar.

## Otimizações do DRM (A17)

- Dano parcial sem defeitos: o mesmo roteiro (arrastar a janela, menu de contexto, fechar o menu, mexer o mouse) com e sem `MIDE_FULL_DAMAGE=1`, quadro 460 salvo pelo `scenario.sh`: as imagens ficam iguais (só o relógio da barra difere).
- Dano medido: `RUST_LOG=mide=debug` mostra "dano: N% da tela" a cada quadro; com `MIDE_DAMAGE_DEBUG=1`, a identidade e a contagem de mudanças de cada elemento (um `Id` novo a cada quadro é dano de tela inteira). Com uma janela parada: 0%.
- `tests/perf.sh desktop` com e sem `MIDE_FULL_DAMAGE=1` (o script agora usa `LC_ALL=C`: em pt_BR o `sort` e o `awk` liam "0.85" errado).
- Manual (sessão de verdade, DRM): cursor no plano de hardware (mexer o mouse com o `MIDE_GPU_TIMING` ligado não gera quadros), forma do cursor trocando (texto, redimensionar, mão), cursor de app (Firefox, jogos), barra de título acendendo os botões, dois monitores (o cursor passa de um para o outro), apagar e acender a tela (o cursor volta), trocar de VT e voltar; sem cortes no notebook (eDP) nem no monitor externo. Se algo der errado: `MIDE_SW_CURSOR=1` e `MIDE_FULL_DAMAGE=1`.
- A bateria não depende da máquina: sem bateria na barra (`MIDE_POWER_SUPPLY_DIR`, pasta vazia), papéis de parede de teste (`MIDE_BACKGROUNDS_DIR`) e pastas do launcher contadas a partir das padrão. "Copiar caminho da dock cola em outro app" com mais folga (cliques no quadro 1200).
- `tests/ui.sh`: "clique não atravessa para a barra de título de trás" (Calculadora atrás, Terminal arrastado por cima da barra dela, clique no Terminal: ele continua no topo).
- Launcher, pasta com 2 páginas, roda do mouse: quadros 512 e 515 do carrossel com uma moldura só no painel (antes, duas).
- Manual (sessão de verdade): Brave/Chrome fecham pelo × deles, pelo Alt+F4 e pelo menu da dock; com "continuar rodando em segundo plano" ligado, a janela some e volta ao abrir de novo pelo ícone. O log da sessão fica em `~/.local/state/mide/sessao.log` ("janela escondida pelo app", "janela mostrada de novo pelo app").
- `tests/ui.sh` novos: estilo Mac (Ctrl+⌘+Q bloqueia; Ctrl+L, ⌘L e ⌥L não; ⌘Tab troca de janela; ⌘Espaço abre o buscador pela tecla Alt), senha certa desbloqueia com dois monitores, janela escondida pelo app volta quando ele mostra, captura: ⌘S salva no estilo Mac.
- Tela de bloqueio: quadro sem barra nem dock atrás (papel de parede desfocado e escurecido), também nos primeiros quadros depois de bloquear; resposta por tecla medida com `RUST_LOG=mide=debug` ("resposta (tecla)").
- Manual (sessão de verdade): bloquear com dois monitores e digitar a senha certa; deixar bloquear sozinho, esperar a tela apagar e acender com uma tecla; Ctrl+⌘+Q no estilo Mac; Print → Salvar e Copiar pelos botões, por Ctrl+S/Ctrl+C e por ⌘S/⌘C (aviso na tela).
- `tests/ui.sh` (2026-10-20): 204 verificações. "Copiar caminho da dock cola em outro app" repõe os 3 itens de Downloads antes (os testes da Lixeira levam `projeto` e `foto.jpg`; com menos itens o menu fica em outro lugar) e espera com sobra (o roteiro conta quadros, o cenário encerra por tempo).
- Ícones (B15): `cargo test --test icons` (tema do mide antes do hicolor; nomes genéricos de reserva); `tests/ui.sh` "dock: ícones dos apps do mide vêm do tema do mide" (instala o tema do clone numa pasta de teste; sem clone, pulado). No fork, conferir os ícones renderizados depois de `apply.py` (nenhum atalho quebrado: `find <tema> -xtype l`).
- Sessão (B15, etapa 3): o trecho do `mide-session` testado isolado (com `XDG_DATA_HOME` e `XDG_CONFIG_HOME` temporários): com o tema instalado, `DCONF_PROFILE` aponta para o perfil do mide, `gsettings get org.gnome.desktop.interface icon-theme` dá `'mide'`, as outras chaves vêm do usuário, e sem o perfil o valor do usuário não muda.
- Manual (sessão de verdade): depois do `mide-atualizar`, os apps GTK (Arquivos do GNOME, gedit, Configurações do GNOME) e Qt 5 mostram os ícones do mide; no Cinnamon, o tema de ícones continua o de antes.

## Atualizar e testar num PC com a sessão do mide

Para o usuário ou para um agente no outro PC (onde o mide roda como sessão de login, no DRM).

1. `git pull` no projeto e `data/mide-conferir` (mostra o que falta: binários, tema de ícones, `mide-session`, erros no log).
2. Dentro da sessão do mide, num terminal: `data/mide-atualizar --tudo`. Recompila em release, baixa o tema de ícones do fork (`fabioaacarneiro/mide-icons`, versão de `data/icons.version`; precisa de `git` e internet na primeira vez) e reinicia o compositor (as janelas abertas fecham; a sessão continua). No fim aparece `ícones do mide instalados (xxxxxxxx)`.
3. Se o `data/mide-conferir` disser que o `mide-session` instalado é uma cópia antiga: `sudo cp data/mide-session <caminho que ele mostrar>`.
4. Sair e entrar de novo na sessão do mide (o `mide-session` prepara o tema de ícones dos apps GTK e Qt). Rodar `data/mide-conferir` de novo, dentro da sessão: tudo `ok`.
5. Conferir:
   - Ícones: dock sem quadrados cinzas (Terminal, Calculadora, Configurações com os desenhos do mide); launcher com os apps no estilo do tema; nenhum logo da Apple (menu, "Sobre", computador); apps GTK (Arquivos do GNOME, gedit) com o mesmo tema; `gsettings get org.gnome.desktop.interface icon-theme` dá `'mide'` dentro do mide e o tema de antes no Cinnamon.
   - Janelas: clique num app (Chrome) por cima da barra de título de outra janela não sobe a de trás; o Brave fecha.
   - Launcher: rolar as páginas dentro de uma pasta, painel com uma moldura só.
   - Bloqueio: Super+L (no estilo Mac, Ctrl+⌘+Q; Ctrl+L e ⌘L não bloqueiam), digitar rápido, senha certa desbloqueia com um e com dois monitores, a tela apaga sozinha e acende com uma tecla.
   - Captura de tela (Print): Salvar e Copiar pelos botões, por Ctrl+S/Ctrl+C e por ⌘S/⌘C no estilo Mac; aparece o aviso; o arquivo fica em Imagens/Capturas de tela.
   - Desempenho (A17): mexer o mouse e usar o VS Code; uso de CPU do mide no `htop` (antes ~3,7% de um núcleo). Cursor de hardware: troca de forma (texto, redimensionar), passa de um monitor para o outro, volta depois de apagar a tela e de trocar de VT.
6. Se algo der errado: log em `~/.local/state/mide/sessao.log` (o anterior em `sessao.log.anterior`). Para voltar ao comportamento antigo do A17: `MIDE_SW_CURSOR=1` (cursor desenhado) e `MIDE_FULL_DAMAGE=1` (tela inteira a cada quadro), exportados no `mide-session` antes do `exec`.
- `tests/ui.sh`: "botão de ligar pergunta antes de desligar" (`key power` abre a confirmação da barra) e "botão de ligar com a tela bloqueada não abre a confirmação".
- Manual (sessão de verdade, DRM): deixar a tela apagar sozinha (e bloquear) e acender com o mouse e com o teclado; suspender pelo menu (Repouso) e acordar; apertar o botão de ligar (aparece a confirmação, Cancelar não desliga); com a tela bloqueada, o botão só acende a tela.
- `tests/ui.sh` (captura): ⌘⇧4 salva a área ao soltar (300x200), ⌘⇧3 salva a tela inteira, ⌘⇧3 com o launcher aberto, ⌘⇧4 no estilo Mac (Alt+Shift+4); captura pelos apps com o `grim` (tela inteira e região iguais ao quadro do mide; sem o grim, pulado); "compartilhar a tela: escolher o segundo monitor" (o picker escreve `HEADLESS-2`).
- Manual (sessão de verdade, com `xdg-desktop-portal-wlr` instalado e depois do `mide-atualizar`): OBS → Fontes → Captura de tela (PipeWire): aparece a janela "Compartilhar a tela", o monitor escolhido aparece no OBS e a gravação acompanha a tela (com o cursor); navegador (Meet/Jitsi) → compartilhar a tela; `grim` e `wf-recorder` funcionam.
- `tests/ui.sh`: "escala fracionária: app GTK 4 recebe 1,25" (loupe com `WAYLAND_DEBUG`: `preferred_scale(150)` e o global `xdg_activation_v1`; sem o loupe, pulado).
- `tests/ui.sh`: "janela sem foco translúcida ([opacity] inactive)" (Terminal sem foco com o papel de parede aparecendo; Calculadora com foco opaca); `cargo test` `opacidade_das_janelas`.
- `tests/ui.sh`: "layer-shell: popup de um painel de terceiros aparece embaixo dele" (`cargo build --example mide-layer-popup-teste` antes; o `tests/ui.sh` usa `target/debug/examples/`).
- `tests/ui.sh` (tray, barramento de teste): o ícone aparece na barra, o clique ativa o app, o clique direito abre o menu e o item chega ao app, o ícone some quando o app sai.
- Manual (sessão de verdade): Discord, Steam, Nextcloud, Telegram ou o `nm-applet --indicator` mostram o ícone; clique e menu funcionam.
- `tests/ui.sh`: "menu global: app GTK no Wayland" (no barramento da sessão, com `python3-gi`: a barra mostra Arquivo, Exibir, Ajuda do `tests/apps/gtk-menu-teste.py` e "Abrir" chega ao app); `cargo test` `atalhos_do_gtk`.
- Manual (sessão de verdade): gedit/Gnome Text Editor, Inkscape e outros apps GTK com barra de menus mostram os menus na barra do mide; apps Qt com o tema do KDE (`QT_QPA_PLATFORMTHEME=kde`) também.
- `tests/ui.sh`: "Aparência: tema escuro grava e a janela muda na hora" (Configurações → Aparência → Escuro; o fundo da janela fica (30, 30, 32) sem reabrir); `cargo test` `banco_do_tema_escuro`.
- Manual (sessão de verdade): trocar para Escuro e ver barras de título, barra superior, menus, central de controle e notificações escuras; abrir um app GTK (gedit, Nautilus) e um Qt depois da troca: escuros; voltar para Claro.
- `tests/ui.sh`: "teclas mortas: ´ + a = á e ~ + o = õ (idioma C)" no mide-text (pula se o teclado do sistema não for ABNT2); `cargo test` `dano_so_onde_mudou`.
- `tests/ui.sh`: "editor: Abrir… pelo seletor de arquivos" (Ctrl+O, duplo clique em nota.txt abre outra janela) e "editor: Salvar como… pelo seletor de arquivos" (sem título, Ctrl+S, nome "novo" em Documentos de uma pasta pessoal de teste); `cargo test` `pastas_primeiro_e_filtro`.
- Manual: no Editor de Texto, Abrir… → navegar pelos lugares e pela pasta de cima, Esc cancela; Salvar como… por cima de um arquivo existente pergunta antes de substituir; tema escuro com o seletor aberto.
- Manual/cenário: `[monitors.outputs."HEADLESS-1"] scale = 1.25` (e `2.0`) num `XDG_CONFIG_HOME` de teste, abrir Configurações, Arquivos e o seletor do Editor: texto e ícones nítidos, layout igual ao de escala 1, clique no interruptor "Sombras" grava; botões da barra de título redondos a 2x. `cargo test` `dano_so_onde_mudou`.
- Manual (sessão de verdade com escala 1,25/2): apps do mide-ui nítidos; Terminal ainda ampliado pelo compositor.
- `tests/ui.sh`: "Teclado: escolher o layout troca na hora", "Teclado: layout gravado no config.toml" e "Teclado: o layout escolhido vale ao digitar" (com o layout dos EUA, a tecla do acento do ABNT2 escreve "[").
