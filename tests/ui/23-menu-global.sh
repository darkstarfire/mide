# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== menu do app na barra (menu global, barramento D-Bus de teste)"
# App de teste (examples/mide-menu-teste) que publica "Projeto" e "Ajuda" pelo dbusmenu.
MENU_APP="$PWD/target/debug/examples/mide-menu-teste"
# "Projeto" na barra em (186, 14); no menu, "Compilar" é a primeira linha (y=49).
MENU_X=186
if [ -x "$MENU_APP" ]; then
    SCEN_PLAIN="$SCEN"
    SCEN=tests/scenario-dbus.sh
    RUST_LOG="mide=debug,mide_bar=info" check "a barra mostra os menus do app em foco" "menu do app mide-menu-teste: Projeto, Ajuda" "500 list" "$MENU_APP"
    CLICK="$(mktemp)"
    MENU_TESTE_ARQUIVO="$CLICK" timeout 150 "$SCEN" "500 click $MENU_X 14; 540 click $MENU_X 49; 580 list" -- "$MENU_APP" >/dev/null 2>&1
    if [ "$(cat "$CLICK")" = "11" ]; then
        echo "ok    clique no menu do app chega ao app (Projeto → Compilar)"; PASS=$((PASS + 1))
    else
        echo "FALHA clique no menu do app chega ao app (recebeu: '$(cat "$CLICK")')"; FAIL=$((FAIL + 1))
    fi
    rm -f "$CLICK"
    SCEN="$SCEN_PLAIN"
else
    echo "pulo  menu do app (rode cargo build --examples)"
fi
