# mide — instruções para o Claude Code

Leia primeiro `project_plain/CONTEXT.md` (o plano e as regras do projeto, o "DNA" do
agente) e siga o fluxo de documentação de lá: depois de cada passo, atualizar
`project_plain/IMPLEMENTED.md`, `TESTS.md`, `CONTEXT.md` e `ANNOTATIONS_OF_AGENT.md`.
As anotações têm as lições já aprendidas: leia antes de mexer numa área.

@project_plain/CONTEXT.md

## Regras do usuário (valem em qualquer PC)

- **Português do Brasil** nas respostas, no código (comentários) e nos commits.
- **Nada copiado da Apple/macOS:** o macOS é só inspiração (sem código, imagens, ícones
  ou fontes deles).
- **"Nativo" é do zero:** componentes do mide marcados como nativos (terminal,
  calculadora...) têm o núcleo escrito aqui. Não usar um crate que faz o trabalho
  principal (como `alacritty_terminal`, `vte`, `numbat`) sem perguntar. Projetos de
  terceiros apontados pelo usuário (iTerm2, Calculadora do Windows...) são só pesquisa,
  nunca código copiado.
- **Não mexer em projetos de terceiros:** só ler/clonar. Mudanças só em repositórios do
  usuário (`fabioaacarneiro/*`), por fork. Num clone de terceiros, o remoto original
  vira `upstream` com push inválido
  (`git remote set-url --push upstream NAO_ENVIAR_PROJETO_ORIGINAL`).
- **O usuário é desenvolvedor Go aprendendo Rust:** ao explicar código ou erros,
  compare com Go (trait ≈ interface, `?` ≈ `if err != nil`, `Option` ≈ nil, canais) e
  explique posse/empréstimo (ownership) quando aparecer.
- **Commit e push depois de cada passo terminado**, com os testes do passo passando.
- **Foco em implementar; testes objetivos** (pedido do usuário em 2026-10-23: "estamos
  evoluindo muito lentamente"): em cada passo, `cargo test`, `clippy` e só as seções
  da bateria que o passo afeta (`tests/ui.sh captura dock`), mais os cenários novos do
  passo. A bateria inteira (paralela, minutos) só antes de juntar trabalhos grandes no
  master ou quando a mudança mexe em algo comum a tudo (compositor, mide-ui).

## Testes

- `cargo test`, `cargo clippy --all-targets` e a bateria `tests/ui.sh`: seções em
  `tests/ui/NN-*.sh` rodando em paralelo (`-j 6` por padrão), cada uma com socket,
  saída e configuração próprios; `tests/ui.sh -l` lista, `tests/ui.sh dock` roda só as
  seções com "dock" no nome. Seção nova para uma área nova; seções grandes divididas
  (a mais longa define o tempo total). O cenário acaba quando o compositor registra
  "autoteste: roteiro concluído" (mais `SCENARIO_GRACE`, 1,5 s); `SCENARIO_WAIT` só
  em cenário que espera algo depois do roteiro.
- Enquanto a bateria roda: nada de `cargo build` nem de editar `tests/ui.sh` ou
  `tests/ui/*.sh` (o bash lê os scripts aos poucos e os cenários usam os binários de
  `target/debug`).
- Não rodar o nvim com a configuração do usuário nos testes (`nvim --clean -u ...`).
- Não tocar na configuração real (`~/.config/mide`); a bateria confere isso no fim.
- `pkill -f "texto"` casa com o próprio comando que contém o texto: use
  `pkill -f "[t]exto"` ou o PID.

## Atualizar e testar na sessão de verdade (outro PC)

O passo a passo está em `project_plain/TESTS.md`, seção "Atualizar e testar num PC
com a sessão do mide". O script `data/mide-conferir` mostra o estado da instalação
(binários, tema de ícones, `mide-session`, erros no log).
