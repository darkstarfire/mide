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
- **Commit e push depois de cada passo terminado**, só com a bateria passando.

## Testes

- `cargo test`, `cargo clippy --all-targets` e a bateria `tests/ui.sh` (uns 200
  cenários, perto de uma hora). Rodar a bateria **sozinha**: nada de `cargo build` nem
  de editar `tests/ui.sh` enquanto ela roda (o bash lê o script aos poucos e os
  cenários usam os binários de `target/debug`). Cenários em paralelo só com
  `MIDE_TEST_SOCKET` e `SCENARIO_OUT` próprios.
- Não rodar o nvim com a configuração do usuário nos testes (`nvim --clean -u ...`).
- Não tocar na configuração real (`~/.config/mide`); a bateria confere isso no fim.
- `pkill -f "texto"` casa com o próprio comando que contém o texto: use
  `pkill -f "[t]exto"` ou o PID.

## Atualizar e testar na sessão de verdade (outro PC)

O passo a passo está em `project_plain/TESTS.md`, seção "Atualizar e testar num PC
com a sessão do mide". O script `data/mide-conferir` mostra o estado da instalação
(binários, tema de ícones, `mide-session`, erros no log).
