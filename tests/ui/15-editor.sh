# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== editor de texto (mide-text)"
TDIR="$(mktemp -d)"
TEXT_APP="$PWD/target/debug/mide-text"
printf 'fn main() {\n    println!("olá");\n}\n' > "$TDIR/teste.rs"
# Abre com o foco no texto: Ctrl+End vai ao fim, Enter e digitar, Ctrl+S salva.
timeout 150 "$SCEN" "400 list; 420 combo ctrl+end; 440 key enter; 450 type editado; 470 combo ctrl+s; 520 list" -- "$TEXT_APP '$TDIR/teste.rs'" >/dev/null 2>&1
if [ "$(tail -n 1 "$TDIR/teste.rs")" = "editado" ] && grep -q 'println!("olá");' "$TDIR/teste.rs"; then
    echo "ok    editor: editar no fim e salvar com Ctrl+S"; PASS=$((PASS + 1))
else
    echo "FALHA editor: editar no fim e salvar com Ctrl+S"; FAIL=$((FAIL + 1))
fi
# Copiar num editor e colar em outro (área de transferência do Wayland). O primeiro
# fica em (48, 76) e o segundo em (80, 108), por cima: barra do primeiro em (300, 61),
# texto do segundo visível em (925, 400).
printf 'copiar isto' > "$TDIR/a.txt"; : > "$TDIR/b.txt"
# O segundo abre ~3 s depois do primeiro: os passos começam no quadro 2000 (o
# autoteste conta quadros, e sem GPU eles passam rápido).
SCENARIO_WAIT=40 timeout 150 "$SCEN" "2000 list; 2020 click 300 61; 2040 combo ctrl+a; 2050 combo ctrl+c; 2070 click 925 400; 2090 combo ctrl+v; 2120 combo ctrl+s; 2180 list" -- "$TEXT_APP '$TDIR/a.txt'" "sleep 1; $TEXT_APP '$TDIR/b.txt'" >/dev/null 2>&1
if [ "$(cat "$TDIR/b.txt")" = "copiar isto" ]; then
    echo "ok    copiar num app e colar em outro"; PASS=$((PASS + 1))
else
    echo "FALHA copiar num app e colar em outro"; FAIL=$((FAIL + 1))
fi
# Teclas mortas (layout ABNT2 do sistema de teste): ´ + a = á e ~ + o = õ, também com
# o idioma "C" (sessão iniciada do console, sem LANG).
: > "$TDIR/acentos.txt"
LANG=C LC_ALL= timeout 150 "$SCEN" "400 click 400 300; 420 key leftbrace; 430 key a; 440 key apostrophe; 450 key o; 480 combo ctrl+s; 530 list" -- "$TEXT_APP '$TDIR/acentos.txt'" >/dev/null 2>&1
if [ "$(cat "$TDIR/acentos.txt")" = "áõ" ]; then
    echo "ok    teclas mortas: ´ + a = á e ~ + o = õ (idioma C)"; PASS=$((PASS + 1))
elif ! grep -q 'XKBLAYOUT="br"' /etc/default/keyboard 2>/dev/null; then
    echo "pulo  teclas mortas (o teclado do sistema não é ABNT2)"
else
    echo "FALHA teclas mortas ($(cat "$TDIR/acentos.txt"))"; FAIL=$((FAIL + 1))
fi
# Abrir… (Ctrl+O) pelo seletor de arquivos: pasta com Sub/, b.md e nota.txt; duplo
# clique em "nota.txt" (190, 443) abre outra janela (a atual já tem documento).
mkdir -p "$TDIR/abrir/Sub"; printf 'outro\n' > "$TDIR/abrir/b.md"; printf 'nota\n' > "$TDIR/abrir/nota.txt"
check "editor: Abrir… pelo seletor de arquivos" 'janela "nota.txt — Editor de Texto"' "400 click 400 300; 420 combo ctrl+o; 480 click 190 443; 485 click 190 443; 600 list" "$TEXT_APP '$TDIR/abrir/b.md'"
# Salvar sem título: o seletor abre em Documentos (pasta pessoal de teste) com o nome
# sugerido; Ctrl+A, "novo" e Enter salvam lá.
mkdir -p "$TDIR/home/Documentos"
timeout 150 "$SCEN" "400 click 400 300; 420 type ola; 440 combo ctrl+s; 480 combo ctrl+a; 490 type novo; 500 key enter; 560 list" -- "HOME='$TDIR/home' $TEXT_APP" >/dev/null 2>&1
if [ "$(cat "$TDIR/home/Documentos/novo" 2>/dev/null)" = "ola" ]; then
    echo "ok    editor: Salvar como… pelo seletor de arquivos"; PASS=$((PASS + 1))
else
    echo "FALHA editor: Salvar como… pelo seletor de arquivos"; FAIL=$((FAIL + 1))
fi
check "editor: alteração não salva marca o título" 'janela "• teste.rs' "400 type x; 460 list" "$TEXT_APP '$TDIR/teste.rs'"
check "fechar com alteração pergunta antes (janela continua)" 'janela "• teste.rs' "400 type x; 430 click 63 61; 480 list" "$TEXT_APP '$TDIR/teste.rs'"
rm -rf "$TDIR"
