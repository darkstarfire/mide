# Como deve ser para instalar o compositor

- deve ter um script bash que deve ser rodado com sudo
- deve analisar o linux, se é fedora, ubuntu, debian, arch
- deve instalar as dependências sozinho para compilar o projeto
- deve compilar o projeto e perguntar se quer instalar
- instalar se confirmado

## Dependências de execução já definidas

- `Xwayland` (pacote `xwayland`) e `xwayland-satellite`, para abrir apps X11 dentro do mide (A14). O satellite pode vir do pacote da distribuição, quando houver, ou de `cargo install --git https://github.com/Supreeeme/xwayland-satellite`. O instalador deve colocar o satellite no PATH (ou ao lado do binário do mide); sem ele, o mide funciona, só sem apps X11.
- Tema de cursores (ex.: `adwaita-icon-theme`), para o cursor do mide no console (A13); sem tema, o mide desenha uma seta própria.

