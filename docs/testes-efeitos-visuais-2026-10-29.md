# Bateria de efeitos visuais — 2026-10-29

## Resultado executivo

- A matriz automatizada de configuração cobriu os 11 efeitos: cada chave foi desligada isoladamente e o teste confirmou que só o efeito escolhido mudou. Resultado: **11/11 aprovados** (`cargo test --bin mide config::tests::cada_efeito_pode_ser_desligado_independentemente`).
- A bateria Rust completa passou: **530 aprovados, 0 falhas, 5 ignorados** (`cargo test --offline`). Os 85 testes do compositor também passaram isoladamente.
- Os testes gráficos por cenário não foram concluídos de forma válida. Sem acesso ao dispositivo gráfico nesta execução, o compositor de cenário relatou `Unable to become drm master` e `controle: ... Broken pipe`; a tentativa inicial também não conseguiu criar o barramento D-Bus privado. A bateria foi interrompida quando as seções ficaram sem avançar. Os resultados dessas tentativas não são classificados como falhas dos efeitos nem como aprovações visuais.
- **Ainda falta verificar os efeitos na GPU do Chromebook que congela.** Nenhuma simulação/headless substitui essa reprodução no equipamento e no driver Intel reais.

## Matriz por efeito

| Efeito | Chave | Implementação auditada | Evidência automatizada | Estado visual nesta bateria |
|---|---|---|---|---|
| Sombras das janelas | `shadows` | `src/compositor/render.rs` | chave isolada e sem alteração das demais; suíte completa | Pendente: cena com janelas no compositor gráfico |
| Cantos arredondados | `rounded_corners` | `src/compositor/render.rs` | chave isolada e sem alteração das demais; testes de raio do compositor | Pendente: inspeção de pixels em janela real |
| Animações de janelas | `animations` | `src/compositor/wm.rs`, `fullscreen.rs`, `state.rs` | chave isolada; suíte completa | Pendente: minimizar, restaurar, fechar, maximizar e tela cheia |
| Efeito lâmpada | `lamp` | `src/compositor/wm.rs` | chave isolada; suíte completa | Pendente: animação de minimizar/restaurar até o ícone da dock |
| Desfoque da dock | `dock_blur` | `src/compositor/render.rs` | chave isolada; a seção `11-configuracoes` tem cobertura do interruptor | Pendente: comparação visual ligado/desligado |
| Desfoque do launcher | `launcher_blur` | `src/compositor/render.rs` | chave isolada; a seção `11-configuracoes` tem cobertura do interruptor | Pendente: abrir launcher com ambos os valores |
| Vidro líquido | `liquid_glass` | `src/compositor/render.rs`, `src/compositor/blur.rs` | chave isolada; há cenário de retorno ao vidro clássico na seção `11-configuracoes` | Pendente: comparar borda/refração/reflexo na dock e nas pastas |
| Vidro de barras e painéis | `panel_glass` | `src/compositor/render.rs`, `preview.rs`, `osd.rs` | chave isolada; a seção `14-vidro` cobre barras e menus | Pendente: imagem capturada com o efeito ligado e desligado |
| Animação de menus | `menu_animation` | `src/compositor/state.rs` | chave isolada; suíte completa | Pendente: abrir/fechar menus e observar opacidade/duração |
| Transições suaves | `transitions` | `src/compositor/snap.rs`, `input.rs`, `osd.rs`; apps consultam a preferência | chave isolada; suíte completa | Pendente: dock, alternador, launcher, Arquivos e miniatura de captura |
| Cursor agitado | `shake_cursor` | `src/compositor/input.rs`, `src/compositor/shake.rs` | chave isolada; testes de intensidade, retorno e quadros irregulares passaram | Pendente: observar o cursor crescer na sessão gráfica |

“Chave isolada” significa que o parser leu a opção como falsa e manteve os outros dez efeitos no padrão verdadeiro. O teste foi adicionado em `src/compositor/config.rs`. Isso verifica armazenamento/configuração; não prova o resultado visual do renderizador.

## Cobertura já existente para repetir em ambiente gráfico

- `tests/ui/11-configuracoes.sh`: sombras, desfoques da dock e launcher e vidro líquido na interface de Configurações.
- `tests/ui/14-vidro.sh`: vidro de barras/menus e animação de crescimento/retorno do cursor agitado.
- `tests/ui/04-janela.sh`, `08-dock.sh` e `44-tela-cheia.sh`: animações, lâmpada da dock e transições relacionadas às janelas.
- Para fechar a bateria gráfica, execute `tests/ui.sh -j 1 configuracoes vidro 44-tela-cheia` numa sessão em que D-Bus privado e renderização headless estejam disponíveis. Registre capturas ligadas/desligadas para sombras, cantos, desfoques e vidro; os cenários atuais verificam mais eventos e estado do que comparação visual de pixels.

## Relato de uso no Chromebook

O usuário testou os efeitos no Chromebook, fora desta execução automatizada. O único congelamento observado ao ligar um efeito aconteceu ao ativar **Mouse crescer com o agito** (`shake_cursor`). Depois de encerrar a sessão e entrar de novo, o usuário o deixou desligado e então o ativou novamente. Nessa segunda ativação conseguiu usar o sistema por mais de 15 minutos sem novo congelamento. O carregador estava desconectado durante o teste.

Isso aponta uma associação temporal com `shake_cursor`, mas **não confirma a causa**: houve uma ocorrência na ativação e uma reativação prolongada sem repetição, sem captura do log ou reprodução controlada. A execução na bateria também é uma variável útil para comparar, não uma explicação estabelecida. Classificação: **intermitente; uma ocorrência, não reproduzida na reativação**.

O código grava a mudança em Configurações (`efeito shake_cursor: ligado/desligado`) e o compositor registra o conjunto de efeitos ao iniciar. Em MIDE v0.10.18 ou posterior, depois de reativar, confira `mide-logs --search 'efeito shake_cursor'` e `mide-logs --search 'configuração de efeitos visuais'`. Se voltar a congelar, após reiniciar recolha `mide-logs --all` e os eventos do kernel (`journalctl -k`) em torno do horário; anote se estava na bateria, o nível de carga e se o cursor foi agitado antes do travamento.

## Conclusões

Não foi encontrado erro na leitura independente dos 11 parâmetros, nos testes de renderização geométrica existentes nem na lógica unitária de cursor agitado. O Chromebook teve uma ocorrência associada ao primeiro acionamento de `shake_cursor`, seguida por mais de 15 minutos de uso depois de reativá-lo, sem repetição e sem carregador. É uma pista concreta, ainda inconclusiva. A bateria gráfica nesta máquina não terminou; comparações futuras devem capturar logs e variar uma condição por vez no Chromebook.
