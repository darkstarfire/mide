# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== teclado na tela"
# O campo de texto é o do `zenity --entry` (GTK 4, text-input v3; GTK_IM_MODULE=wayland
# para não depender de outro método de entrada da máquina). Com "Mostrar o teclado na
# tela ao digitar" ligado, o teclado sobe acima da dock: a camada fica em (0, 361)
# 1366x340 (a barra de atalhos em cima das teclas) e as teclas (centro, na tela) ficam em: o (1070, 498), l (1070, 554),
# a (185, 554), x (461, 611), apagar (1153, 611), Enter (1059, 667), esconder
# (1185, 667). Segurar o "a" abre as variações; a primeira (á) fica em (60, 495).
if command -v zenity >/dev/null; then
    OSK="$(mktemp -d)"
    mkdir -p "$OSK/sempre/mide" "$OSK/tablet/mide"
    printf '[keyboard]\non_screen = true\n' > "$OSK/sempre/mide/config.toml"
    : > "$OSK/tablet/mide/config.toml"
    osk_run() { # osk_run nome config roteiro [quadro]
        rm -f "$OSK/$1.txt"
        XDG_CONFIG_HOME="$OSK/$2" SCENARIO_OUT="$OSK/$1-out" timeout 150 "$SCEN" "$3" ${4:-} -- \
            "sh -c 'GTK_IM_MODULE=wayland zenity --entry --text Teste > $OSK/$1.txt'" > "$OSK/$1.log" 2>&1
        cat "$OSK/$1-out/mide.log" >> "$OSK/$1.log" 2>/dev/null
    }
    osk_check() { # osk_check nome regex arquivo
        if grep -Eq "$2" "$3"; then
            echo "ok    $1"; PASS=$((PASS + 1))
        else
            echo "FALHA $1"; FAIL=$((FAIL + 1))
        fi
    }

    osk_run abre sempre "600 list"
    osk_check "campo de texto ativo: o teclado aparece (opção ligada)" "teclado na tela: aberto" "$OSK/abre.log"
    osk_check "o teclado fica acima da dock" 'camada "mide-keyboard" em \(0, 361\) 1366x340' "$OSK/abre.log"

    osk_run digita sempre "700 click 1070 498; 720 click 1070 554; 740 press 185 554; 790 move 60 495; 800 release 60 495; 830 click 461 611; 850 click 1153 611; 870 click 1059 667; 950 list" 796
    if [ "$(cat "$OSK/digita.txt" 2>/dev/null)" = "olá" ]; then
        echo "ok    digitar \"olá\" (segurar o a para o á, apagar e Enter) chega ao app"; PASS=$((PASS + 1))
    else
        echo "FALHA digitar \"olá\" chega ao app (veio \"$(cat "$OSK/digita.txt" 2>/dev/null)\")"; FAIL=$((FAIL + 1))
    fi
    osk_check "segurar uma letra mostra as variações" "variações de a: á@" "$OSK/digita.log"
    osk_check "o campo fechado (Enter) esconde o teclado" "teclado na tela: fechado" "$OSK/digita.log"

    osk_run esconde sempre "700 click 1185 667; 800 list"
    osk_check "a tecla de esconder fecha o teclado" "teclado na tela: fechado" "$OSK/esconde.log"

    # Fora do modo tablet e sem a opção, o campo ativo não abre o teclado.
    osk_run sem tablet "600 list"
    if grep -q "teclado na tela: campo de texto ativo" "$OSK/sem.log" && ! grep -q "teclado na tela: aberto" "$OSK/sem.log"; then
        echo "ok    sem modo tablet nem a opção, o teclado não aparece sozinho"; PASS=$((PASS + 1))
    else
        echo "FALHA sem modo tablet nem a opção, o teclado não aparece sozinho"; FAIL=$((FAIL + 1))
    fi

    # Como no iPad: puxar o "q" para baixo digita 1; segurar o espaço vira trackpad;
    # a barra de atalhos (desfazer em (155, 447)) e o botão de flutuar em (1211, 447):
    # o teclado flutuante fica com 400 de largura.
    osk_run ipad sempre "700 press 185 498; 712 move 185 530; 720 release 185 530; 760 click 155 447; 800 press 683 667; 850 move 760 667; 860 move 800 667; 870 release 800 667; 900 click 1211 447; 980 list"
    osk_check "puxar a letra de cima para baixo digita o número" "letra puxada para baixo \(1\)" "$OSK/ipad.log"
    osk_check "segurar o espaço vira trackpad" "trackpad \(espaço segurado\)" "$OSK/ipad.log"
    osk_check "barra de atalhos (desfazer)" "barra de atalhos \(edit-undo-symbolic\)" "$OSK/ipad.log"
    osk_check "o teclado fica flutuante (menor)" 'camada "mide-keyboard" em \([0-9]+, [0-9]+\) 400x' "$OSK/ipad.log"
    # Dois espaços viram ". " (o teclado volta acoplado: o anterior gravou o flutuante).
    printf '[keyboard]\non_screen = true\n' > "$OSK/sempre/mide/config.toml"
    osk_run ponto sempre "700 click 1070 498; 720 click 683 667; 723 click 683 667; 760 click 1059 667; 850 list"
    if [ "$(cat "$OSK/ponto.txt" 2>/dev/null)" = "o. " ]; then
        echo "ok    dois espaços viram ponto e espaço"; PASS=$((PASS + 1))
    else
        echo "FALHA dois espaços viram ponto e espaço (veio \"$(cat "$OSK/ponto.txt" 2>/dev/null)\")"; FAIL=$((FAIL + 1))
    fi

    # Escala 1,5: o painel em pixels da tela (2049 de largura) e nítido.
    printf '[keyboard]\non_screen = true\n[monitors.outputs."HEADLESS-1"]\nscale = 1.5\n' > "$OSK/sempre/mide/config.toml"
    MIDE_WINIT_SIZE=2049x1152 osk_run escala sempre "600 list" 600
    osk_check "em escala 1,5 o teclado aparece" 'camada "mide-keyboard" em \(0, 361\) 1366x340' "$OSK/escala.log"
    # A letra "q" (centro em (185, 498) pontos = (277, 747) pixels) tem pixels escuros
    # nítidos: o centro da letra é bem mais escuro que a tecla branca em volta.
    Q="$(python3 -c "import sys; from PIL import Image; i=Image.open(sys.argv[1]).convert('L'); print(min(i.getpixel((x, y)) for x in range(268, 287) for y in range(735, 760)))" "$OSK/escala-out/frame.png" 2>/dev/null)"
    if [ -n "$Q" ] && [ "$Q" -lt 90 ]; then
        echo "ok    em escala 1,5 as letras das teclas saem nítidas"; PASS=$((PASS + 1))
    else
        echo "FALHA em escala 1,5 as letras das teclas saem nítidas (mais escuro: $Q)"; FAIL=$((FAIL + 1))
    fi
    rm -rf "$OSK"
else
    echo "pulado: teclado na tela (sem o zenity para o campo de texto)"
fi
