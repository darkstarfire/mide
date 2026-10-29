# HiDPI no Chromebook: relatório e plano

Para quem for continuar este trabalho (agente ou pessoa). Escrito em 2026-10-29, depois da v0.10.28.

## 1. O que o usuário viu

Hardware: Chromebook com painel de 13,3" e resolução física **2256 × 1504** (o usuário chama de 2K), escala do mide em **150%** (`eDP-1`, 60 Hz). Em escala 1,5, a área lógica é ~1504 × 1003.

Sintomas, com as fotos do usuário:

1. **Texto espaçado e desfocado** na barra de título das janelas (ex.: "Erro na compilação do mi…", com letras separadas por espaços irregulares) e no título do VS Code.
2. **Texto espaçado nas notificações** ("Captura  salva", "Captura  copiada"): o espaçamento entre letras varia, parecendo justificado.
3. **Cursor do mouse muito pequeno** em 150%.
4. Interface de Configurações (Monitores) em 150% aparece nítida; o problema está concentrado na **barra de título desenhada pelo compositor** e nas **notificações** (e provavelmente barra e dock, a conferir).

## 2. Hipóteses (a confirmar, não são fatos)

- **Texto (1 e 2):** o texto da barra de título e das notificações é desenhado com tamanho em pixels inteiros ou com posições arredondadas para pixel inteiro na escala 1,5. Cada glifo é arredondado para o pixel mais próximo, o que gera espaçamento irregular. Quando o texto é desenhado num buffer em escala 1 e depois esticado pelo compositor, fica desfocado. Ver `src/compositor/title.rs` (usa `cosmic-text`, como `src/documents/richtext/`) e o caminho de escala do mide-ui (`ui.scale`, `draw::scale()`, `src/compositor/clip.rs:100`).
- **Cursor (3):** `src/compositor/cursor.rs:74` lê `XCURSOR_SIZE` e cai em **24** por padrão. Esse tamanho não é multiplicado pela escala da tela, então em 150% o cursor fica com tamanho lógico de 24 (≈ 16 pt visuais em vez de ~24). Confirmar com `echo $XCURSOR_SIZE` na sessão e com o log `plano de cursor`.
- **Fonte de ícones e texto do sistema:** a fonte do sistema no Chromebook pode estar com hinting diferente; conferir com `fc-match sans` e com o texto do Configurações (que está nítido) como referência.

## 3. Como verificar antes de mexer

Na sessão do Chromebook (dentro do mide):

```
echo "$XCURSOR_SIZE $XCURSOR_THEME"
grep -i "plano de cursor\|escala\|scale" ~/.local/state/mide/session.log | tail -20
mide-logs --search "escala"
```

Para reproduzir sem o Chromebook, num terminal de teste:

```
MIDE_SCALE=1.5 target/debug/mide ...   # ou a configuração [monitors.outputs."HEADLESS-1"] scale = 1.5
```

Conferir na captura (Print) de uma janela com título longo e notificação; comparar com Configurações (referência nítida).

## 4. Plano de correção (ordem sugerida)

**Etapa H1. Cursor em escala fracionária (correção pequena, de maior impacto visível)**
- Ao montar o cursor (`src/compositor/cursor.rs`), multiplicar `XCURSOR_SIZE` pela escala da saída onde o ponteiro está (`ui.scale` do output), arredondando para o tamanho disponível do tema (24, 32, 48, 64...).
- Respeitar `XCURSOR_SIZE` explícito se o usuário definiu, mas na sessão do mide o padrão deve acompanhar a escala.
- Teste: cenário de cursor com `MIDE_SCALE=1.5`; checar que o tamanho pedido ao tema é 36 ou 48 (1,5 × 24 = 36 → 48 no tema, se o tema não tiver 36).
- Aceite: cursor visualmente do mesmo tamanho relativo aos ícones da barra no Chromebook.

**Etapa H2. Texto da barra de título nítido e com espaçamento regular**
- Desenhar o texto da barra de título (`src/compositor/title.rs`) em escala real (`tamanho_pt × escala`) e não em escala 1 esticada.
- Usar posições de glifo em pixel físico (arredondar o início do texto, não cada glifo) e o mesmo `cosmic-text` com `Metrics` em pixels físicos, como o mide-ui já faz nos apps.
- Verificar se o problema é o *letter-spacing* (kerning/hinting) ou o arredondamento por glifo; comparar com o texto do mide-ui (Configurações) no mesmo quadro.
- Aceite: título "Erro na compilação do mide" sem espaços irregulares em 1,25, 1,5 e 2,0; foto de referência antes/depois.

**Etapa H3. Notificações**
- Mesmo caminho do H2 (`src/bin/mide-notifications.rs` usa mide-ui; conferir se o problema é o mesmo ou se é o `letter-spacing` de uma fonte específica). Testar a fonte padrão do sistema do Chromebook.

**Etapa H4. Auditoria de escala em todos os textos e ícones**
- Barra superior, dock, launcher, Arquivos, Terminal (que ainda aparece ampliado pelo compositor, segundo `TESTS.md`) e os botões de título desenhados pelo compositor (fechar, minimizar, maximizar).
- Cada item: captura em 1,25, 1,5 e 2,0 e conferência de nitidez, conforme a regra de HiDPI do `project_plain/CONTEXT.md`.

**Etapa H5. Teste com o hardware real**
- No Chromebook: `data/mide-update --all`, sair e entrar na sessão, conferir H1–H4 e a foto de cada um.
- O que não é possível testar aqui: a GPU Intel do Chromebook e o painel 2256 × 1504 real. Testes headless valem, mas o aceite final é no equipamento.

## 5. Pontos abertos

- Confirmar a resolução exata (2256 × 1504 pelo painel do monitor, não 2560 × 1440).
- Confirmar se a fonte do sistema no Chromebook é a mesma que no PC do usuário (`fc-match sans`).
- Decidir se o compositor desenha o próprio texto ou se os títulos passam a ser desenhados pelo mide-ui no cliente (decoração do cliente), o que resolve H2 de vez mas muda a arquitetura do A7.

## 6. Relação com o que já existe

- Regra de HiDPI: `project_plain/CONTEXT.md` (escala 1,25/1,5/2, nada em pixel fixo).
- Problema já registrado lá: "a barra superior perdeu definição em 150% no 2K do usuário"; este documento acrescenta título de janela, notificações e cursor.
- Escala fracionária dos apps: A15 e A14 em `project_plain/IMPLEMENTED.md`.
