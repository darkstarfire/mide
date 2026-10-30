# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== tela de login (mide-login, greeter do greetd, C5)"
# Nada aqui usa o greetd nem o PAM de verdade: o compositor roda em modo de login
# (MIDE_LOGIN=1) com o greetd falso (tests/fake/greetd, senha "segredo"; o usuário
# "dois" pede também o código "codigo"), usuários de teste (MIDE_LOGIN_TEST_USERS, só
# em compilação de desenvolvimento), sessões de teste e o desligar só registrado
# (MIDE_SYSTEM_DRY_RUN). Em 1366x768 há 4 fotos de usuário (ana, bruno, dois e
# "Outro…") com o centro em x = 461, 609, 757 e 905 e y = 474; o campo fica em
# y = 552; a sessão no canto de cima à direita (1302, 17), com o menu embaixo (Niri na
# segunda linha, y = 77); Desligar em (579, 693); Cancelar em (683, 693).
LG="$(mktemp -d)"
LGS="/run/user/$(id -u)/mide-greetd-$$.sock"
OUT="${SCENARIO_OUT:-/tmp/mide-scenario}"
mkdir -p "$LG/ws" "$LG/cfg"
printf '[Desktop Entry]\nName=mide\nExec=mide-session\nDesktopNames=mide\n' > "$LG/ws/mide.desktop"
printf '[Desktop Entry]\nName=Niri\nExec=niri --session\n' > "$LG/ws/niri.desktop"

# login_run "roteiro" [quadro] [tamanho]: cenário com o greetd falso; o log do falso fica
# em $LG/greetd.log. Sem o roteiro "concluído" quando a sessão começa (o compositor
# sai), então espera um tempo fixo.
login_run() {
    rm -f "$LG/greetd.log" "$LGS"
    python3 tests/fake/greetd "$LGS" "$LG/greetd.log" &
    local fake=$!
    for _ in $(seq 30); do [ -S "$LGS" ] && break; sleep 0.1; done
    GREETD_SOCK="$LGS" MIDE_LOGIN=1 MIDE_LOGIN_TEST_USERS="ana:Ana Souza:o nome do gato;bruno:Bruno Lima;dois:Dois Fatores" \
        MIDE_LOGIN_SESSIONS_DIRS="$LG/ws" MIDE_SYSTEM_DRY_RUN=1 XDG_STATE_HOME="$LG/state" \
        XDG_CONFIG_HOME="${LOGIN_CFG:-$LG/cfg}" \
        MIDE_WINIT_SIZE="${3:-1366x768}" SCENARIO_WAIT="${SCENARIO_LOGIN_WAIT:-10}" \
        timeout 150 "$SCEN" "$1" ${2:+"$2"} >/dev/null 2>&1
    kill "$fake" 2>/dev/null
    rm -f "$LGS"
}
login_log() { sed 's/\x1b\[[0-9;]*m//g' "$OUT/mide.log"; }
login_ok() {
    local name="$1"
    shift
    if "$@"; then
        echo "ok    $name"; PASS=$((PASS + 1))
    else
        echo "FALHA $name"; FAIL=$((FAIL + 1))
        login_log | grep -E "login|lock" | grep -v DEBUG | tail -4 | sed 's/^/      /'
        sed 's/^/      greetd: /' "$LG/greetd.log" 2>/dev/null | tail -3
    fi
}

# Senha certa: o greetd recebe a sessão do mide e o compositor de login sai.
login_run "400 click 461 474; 470 type segredo; 480 key enter"
login_ok "senha certa: start_session com a sessão do mide" grep -q 'SESSAO {"user": "ana", "cmd": \["mide-session"\]' "$LG/greetd.log"
login_ok "senha certa: o compositor de login sai" sh -c "sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -q 'login: sessão aceita pelo greetd; saindo'"
login_ok "a senha não vai ao log" sh -c "! grep -q segredo '$LG/greetd.log' '$OUT/mide.log'"
login_ok "lembra o último usuário e a sessão" grep -q "session.ana=mide" "$LG/state/mide-login/state"

# Senha errada: aviso, cancelamento e a senha pedida de novo.
login_run "400 click 609 474; 470 type errada; 480 key enter; 600 list"
login_ok "senha errada: recusada e pedida de novo" sh -c "[ \"\$(sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -c 'login: pergunta do PAM (secreta)')\" -ge 2 ] && grep -q cancel_session '$LG/greetd.log' && ! grep -q SESSAO '$LG/greetd.log'"

# Segundo fator: depois da senha, o código (pergunta visível).
login_run "400 click 757 474; 470 type segredo; 480 key enter; 560 type codigo; 570 key enter"
login_ok "segundo fator: código pedido e aceito" sh -c "sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -q 'pergunta do PAM (visível): Verification code' && grep -q 'SESSAO {\"user\": \"dois\"' '$LG/greetd.log'"

# Trocar de usuário: Esc volta à lista e outro entra.
login_run "400 click 461 474; 460 key esc; 520 click 609 474; 580 type segredo; 590 key enter"
login_ok "trocar de usuário (Esc volta à lista)" grep -q 'SESSAO {"user": "bruno"' "$LG/greetd.log"

# Escolher a sessão na lista de baixo.
login_run "400 click 1302 17; 440 click 1290 77; 480 click 461 474; 540 type segredo; 550 key enter"
login_ok "escolher a sessão: Niri" grep -q 'SESSAO {"user": "ana", "cmd": \["niri", "--session"\]' "$LG/greetd.log"

# Outro usuário: digitar o nome.
login_run "400 click 905 474; 460 type bruno; 470 key enter; 480 type segredo; 490 key enter"
login_ok "outro usuário pelo nome" grep -q 'SESSAO {"user": "bruno"' "$LG/greetd.log"

# Energia: Desligar (só registrado nos testes).
login_run "400 click 579 693; 480 list"
login_ok "desligar pela tela de login" sh -c "sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -q 'sistema (teste): logind PowerOff'"

# Três senhas erradas: a dica de senha aparece (como no Mac).
login_run "400 click 461 474; 460 type x; 462 key enter; 480 type x; 482 key enter; 500 type x; 502 key enter; 530 list"
login_ok "dica de senha depois de três tentativas" sh -c "sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -q 'login: dica de senha mostrada'"

# Cancelar (botão redondo) volta à lista.
login_run "400 click 461 474; 460 click 683 693; 540 click 609 474; 600 type segredo; 610 key enter"
login_ok "Cancelar volta à lista" grep -q 'SESSAO {"user": "bruno"' "$LG/greetd.log"

# Modo "nome e senha" ([login] show = "name"): os dois campos, sem a lista.
mkdir -p "$LG/cfgnome/mide"
printf '[login]\nshow = "name"\npower_buttons = false\n' > "$LG/cfgnome/mide/config.toml"
LOGIN_CFG="$LG/cfgnome" login_run "400 type ana; 410 key tab; 420 type segredo; 430 key enter"
login_ok "modo nome e senha: entra com os dois campos" sh -c "grep -q 'SESSAO {\"user\": \"ana\"' '$LG/greetd.log' && sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -q 'login: .*nome e senha'"

# Escala 1,5: o relógio na resolução da tela (muitos pixels brancos puros nas letras;
# esticado de 1x, as bordas viram cinza).
mkdir -p "$LG/cfg15/mide"
printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.5\n' > "$LG/cfg15/mide/config.toml"
LOGIN_CFG="$LG/cfg15" SCENARIO_LOGIN_WAIT=8 login_run "300 list" 300 2049x1152
# Proporção de pixels brancos puros na hora: em 1x ela fica perto de 28% e em 1,5 nítido
# perto de 32%; esticada de 1x para 1,5, as bordas das letras viram cinza e ela cai.
WHITE="$(python3 -c "
import sys
from PIL import Image
i = Image.open(sys.argv[1]).convert('L')
w = i.size[0]
k = w / 1366
box = i.crop((int(w / 2 - 120 * k), int(95 * k), int(w / 2 + 120 * k), int(185 * k)))
print(round(100 * sum(1 for p in box.getdata() if p >= 250) / (box.size[0] * box.size[1])))
" "$OUT/frame.png" 2>/dev/null)"
login_ok "escala 1,5: relógio nítido (${WHITE:-?}% de branco puro)" sh -c "[ '${WHITE:-0}' -ge 26 ]"

rm -rf "$LG"
