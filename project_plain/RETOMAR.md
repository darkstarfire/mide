# Retomar o port X11 (ponto de parada)

## Estado do código
- `mide` (branch `port-x11`): enviado ao GitHub até `40dd0a2`. Commits locais depois disso: `a420e1c` (atalhos de teclas especiais), `c165861` (canal de controle lista e foca janelas no X11). Ver `git log origin/port-x11..HEAD`.
- `mide-ui` (branch `port-x11`): enviado até `80e92ed`. O `mide` fixa esse commit em `Cargo.toml`.
- Alterações Wayland antigas em `src/compositor/` continuam sem commit (não são do port X11).

## Pacote de teste
- Último gerado: `target/mide-x11-pacote/mide-x11-0.10.72-x86_64-5.txz` (build 5).
- Instalação no notebook: `sudo installpkg <arquivo>`; sessão `mide_x11` no SDDM.

## Teste em andamento (notebook, sessão mide_x11)
- Roteiro: ver a mensagem de teste (monitores, semáforos, arrasto, teclado, Alt+Tab, dock, cantos, foco, cruzamento de monitores).
- Coleta de logs: `data/mide-x11-coleta` gera um `.tar.gz` para enviar.
- Resultados esperados do teste: anotar por janela e por monitor.

## Pendências conhecidas
1. Barra de título do compositor escondida atrás de apps X11 comuns (posicionar o app `TITLE_HEIGHT` abaixo, ajustar geometria, maximizar, tela cheia e arrasto).
2. Teclado no `mide-terminal` não aceitava foco (em investigação; o teste em Xvfb ainda não concluiu).
3. Foco segue o mouse (efeito de remover o grab; decidir se o foco deve ser por clique).
4. Janela azul e barra estilo janela comum no print das 07:14 (sem causa confirmada).
5. Arrasto sob KWin não verificado (o mesmo grab implícito explica o problema).
6. Dock com auto-ocultar não testada.
7. Versão e tag por microetapa (`CLAUDE.md`) ainda não feitas.

## Como retomar
- Abrir esta conversa e pedir para continuar pelo `project_plain/RETOMAR.md`.
- No terminal: `cd ~/Downloads/src/mide/mide && git status && git log --oneline -8`.
