# Registros e diagnóstico do MIDE

Os processos do MIDE escrevem no journal do sistema (stderr) e em um arquivo compartilhado, com rotação e permissões privadas:

```text
~/.local/state/mide/logs/mide.log
```

Se `XDG_STATE_HOME` estiver definido, o arquivo fica em `$XDG_STATE_HOME/mide/logs/mide.log`. `MIDE_LOG_FILE` permite apontar para outro arquivo, principalmente em testes.

## Consultar

```sh
cargo build --release --bin mide-logs # compila o consultor de logs
target/release/mide-logs              # últimas 300 linhas
target/release/mide-logs --errors     # avisos, erros, pânicos e referências a travamentos/watchdog
target/release/mide-logs --search drm # busca literal, sem diferenciar maiúsculas/minúsculas
target/release/mide-logs --all        # arquivo atual e rotações, em ordem cronológica
journalctl --user -f      # eventos recentes encaminhados ao journal
```

Cada linha inclui horário, nível, processo, alvo, thread e posição no código quando disponível. Os processos registram início/fim e versão. O compositor registra os efeitos ativos ao iniciar; Configurações registra cada alteração de efeito. Para reproduzir um congelamento, anote o horário aproximado e rode `mide-logs --search` com `watchdog`, `drm`, `gpu`, `panico` ou o nome do efeito.

O arquivo tem até 8 MiB por segmento, mais três rotações, é compartilhado entre processos com escrita serializada e fica acessível somente ao usuário (`0700` na pasta, `0600` nos arquivos). Avisos e erros são sincronizados em disco. Se não for possível criar o arquivo, os eventos continuam no journal/stderr.

## Privacidade

O logger não despeja variáveis de ambiente, senhas, texto de documentos nem mensagens de pânico (que podem conter dados privados). O hook de pânico registra localização e backtrace. Eventos específicos de uma operação ainda devem evitar registrar conteúdo fornecido pelo usuário.

## Limites do diagnóstico

Um log persistente ajuda a localizar o último evento antes de uma parada, mas não substitui coleta do kernel, métricas de GPU nem captura de um travamento que impede o processo de registrar a recuperação. Para falhas de driver, compare o horário com `journalctl -k` e `coredumpctl`; para investigar o Chromebook, mantenha o arquivo após cada efeito isolado e registre modelo da GPU, escala, monitor e passos de reprodução.
