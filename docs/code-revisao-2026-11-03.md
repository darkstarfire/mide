# Code (mide-code): revisão do usuário de 2026-11-03

Apontamentos do usuário depois de usar a v0.10.66 na sessão (9 fotos), na ordem em
que vieram, com o que foi feito em cada um. Este arquivo é a lista de trabalho:
cada item só sai daqui marcado como feito, com a versão e como foi conferido.

## Apontamentos

1. **Tipo da sugestão sobrescrito no completar.** A coluna da direita da lista
   mostrava o `detail` do servidor ("function alert(mess…", "interface Blob…") no
   lugar do tipo. Ela tem de dizer sempre o que é a sugestão (palavra-chave, função,
   variável, classe...); o detalhe vai para o painel de documentação.
2. **A assinatura (e a documentação) não some.** O painel de ajuda de assinatura do
   JavaScript continuou aberto ao clicar em outras áreas e ao trocar para um arquivo
   PHP. Bug grave. A documentação deve aparecer ao parar o ponteiro sobre um símbolo
   que tenha documentação; a assinatura só enquanto o cursor está dentro da chamada,
   no arquivo dela.
3. **Avisos do servidor no fim da linha atrapalham.** O texto do erro vinha logo
   depois do código e ficava embaixo do completar. Tem de ficar alinhado à direita do
   editor.
4. **Documentação à esquerda do completar.** Com espaço à direita, o painel ia para a
   esquerda e cobria o texto que se digita. Ele fica sempre à direita; sem espaço,
   estreita e se ajusta ao tamanho da janela.
5. **Painéis por cima deixam ver o que está atrás.** O texto da árvore e da barra de
   status aparecia por cima do painel de documentação. Os painéis vêm por cima de
   tudo (se forem transparentes, com o vidro dos menus do mide; isso pode ficar para
   depois).
6. **Configurar como o Zed, e não "usar as configurações do Zed".** O pedido era o
   Code ter um arquivo de configurações próprio, que se escreve à mão como o
   `settings.json` do VS Code e do Zed, além das opções gráficas; e um sistema de
   extensões que instala as extensões do Zed. O interruptor "Usar as configurações do
   Zed" não era isso.
7. **Arquivo de servidores incompleto e Preferências compridas.** O botão abria um
   arquivo com poucas opções: ele tem de trazer todas as opções que já vêm por padrão,
   para ficar fácil de editar. As Preferências têm de ter seções separadas por tipo,
   escolhidas numa barra lateral, como nas IDEs grandes (Eclipse, VS Code, Zed,
   IntelliJ), no mesmo visual das Configurações do mide, só com as opções do Code.
8. **Painel de símbolos sem redimensionar.** Todo painel do Code tem de poder mudar de
   tamanho arrastando a borda, como a gaveta dos arquivos.
9. **Digitar e nada acontecer; o texto andou para a esquerda.** Com o completar e a
   documentação abertos, a digitação parecia travar e, numa hora, o texto rolou para
   a esquerda e sumiu da vista.

Sobre o resto, o usuário aprovou: as abas, o visual, os temas do editor e os temas de
ícones. Pergunta: "como vamos instalar [temas e ícones] se você não fez o sistema de
plugins compatível com o Zed?" (a loja existia, mas só por Ctrl+Shift+X: ninguém a
achava).

## Processo e resultado

### v0.10.67: itens 1, 2, 3, 4, 5, 8 e 9

- **1 (feito):** a coluna da direita do completar mostra sempre o tipo, traduzido
  (Função, Variável, Classe, Palavra-chave, Trecho...). O detalhe do servidor (a
  assinatura) vai inteiro, quebrando a linha, para o alto do painel de documentação
  (`ui.code_spans`). Os links do markdown aparecem só com o nome ("MDN Reference").
- **2 (feito):** a ajuda de assinatura guarda a aba e o ponto onde foi pedida e some
  ao trocar de aba, ao clicar, com Esc, ao sair da linha ou ao voltar para antes do
  parêntese. A dica do ponteiro e a do Ctrl+I somem ao trocar de aba. A documentação
  continua aparecendo ao parar o ponteiro 0,7 s sobre um símbolo.
- **3 e 9 (feito):** causa do item 9 achada: com o cursor no fim de uma linha com
  aviso, a área de texto rolava para a esquerda para mostrar o aviso inteiro (um
  comportamento feito para as notas da Calculadora), e o texto digitado saía da
  vista. Agora os avisos do Code ficam encostados à borda direita do editor
  (`Decorations::trailing_right`, novo no mide-ui), cortados com "…" e sem rolar.
- **4 (feito):** a documentação vai sempre à direita da lista; sem espaço, estreita
  (até 160) e, se ainda faltar, a lista anda para a esquerda.
- **5 (feito):** os painéis do editor (assinatura, dica do ponteiro, completar) são
  desenhados por último, depois da barra de status e do resto, e nada mais aparece
  por cima deles. O vidro dos menus nesses painéis fica para depois.
- **8 (feito):** o painel de símbolos tem a divisória do kit, arrastável como a da
  gaveta e a do terminal; a largura fica guardada (`outline_width`).
- Conferido por captura com o servidor de teste: tipo à direita, documentação à
  direita com a assinatura no alto, aviso comprido à direita sem rolar e a divisória
  arrastada de 270 para 424.
