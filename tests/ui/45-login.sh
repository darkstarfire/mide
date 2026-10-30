# Seção da bateria (tests/ui.sh); usa as funções de tests/ui/comum.sh.
echo "== tela de login (mide-login, greeter do greetd, C5)"
# Nada aqui usa o greetd nem o PAM de verdade: o compositor roda em modo de login
# (MIDE_LOGIN=1) com o greetd falso (tests/fake/greetd, senha "segredo"; o usuário
# "dois" pede também o código "codigo"), usuários de teste (MIDE_LOGIN_TEST_USERS, só
# em compilação de desenvolvimento), sessões de teste e o desligar só registrado
# (MIDE_SYSTEM_DRY_RUN). Em 1366x768 há 4 cartões de usuário (ana, bruno, dois e
# "Outro…") com o centro em x = 485, 617, 749 e 881 e y = 351; o campo fica em
# y = 445; a lista de sessões em (270, 711); Desligar em (798, 711).
LG="$(mktemp -d)"
LGS="/run/user/$(id -u)/mide-greetd-$$.sock"
OUT="${SCENARIO_OUT:-/tmp/mide-scenario}"
mkdir -p "$LG/ws"
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
    GREETD_SOCK="$LGS" MIDE_LOGIN=1 MIDE_LOGIN_TEST_USERS="ana:Ana Souza;bruno:Bruno Lima;dois:Dois Fatores" \
        MIDE_LOGIN_SESSIONS_DIRS="$LG/ws" MIDE_SYSTEM_DRY_RUN=1 XDG_STATE_HOME="$LG/state" \
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
login_run "400 click 485 351; 470 type segredo; 480 key enter"
login_ok "senha certa: start_session com a sessão do mide" grep -q 'SESSAO {"user": "ana", "cmd": \["mide-session"\]' "$LG/greetd.log"
login_ok "senha certa: o compositor de login sai" sh -c "sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -q 'login: sessão aceita pelo greetd; saindo'"
login_ok "a senha não vai ao log" sh -c "! grep -q segredo '$LG/greetd.log' '$OUT/mide.log'"
login_ok "lembra o último usuário e a sessão" grep -q "session.ana=mide" "$LG/state/mide-login/state"

# Senha errada: aviso, cancelamento e a senha pedida de novo.
login_run "400 click 617 351; 470 type errada; 480 key enter; 600 list"
login_ok "senha errada: recusada e pedida de novo" sh -c "[ \"\$(sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -c 'login: pergunta do PAM (secreta)')\" -ge 2 ] && grep -q cancel_session '$LG/greetd.log' && ! grep -q SESSAO '$LG/greetd.log'"

# Segundo fator: depois da senha, o código (pergunta visível).
login_run "400 click 749 351; 470 type segredo; 480 key enter; 560 type codigo; 570 key enter"
login_ok "segundo fator: código pedido e aceito" sh -c "sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -q 'pergunta do PAM (visível): Verification code' && grep -q 'SESSAO {\"user\": \"dois\"' '$LG/greetd.log'"

# Trocar de usuário: Esc volta à lista e outro entra.
login_run "400 click 485 351; 460 key esc; 520 click 617 351; 580 type segredo; 590 key enter"
login_ok "trocar de usuário (Esc volta à lista)" grep -q 'SESSAO {"user": "bruno"' "$LG/greetd.log"

# Escolher a sessão na lista de baixo.
login_run "400 click 270 711; 440 click 275 672; 480 click 485 351; 540 type segredo; 550 key enter"
login_ok "escolher a sessão: Niri" grep -q 'SESSAO {"user": "ana", "cmd": \["niri", "--session"\]' "$LG/greetd.log"

# Outro usuário: digitar o nome.
login_run "400 click 881 351; 460 type bruno; 470 key enter; 540 type segredo; 550 key enter"
login_ok "outro usuário pelo nome" grep -q 'SESSAO {"user": "bruno"' "$LG/greetd.log"

# Energia: Desligar (só registrado nos testes).
login_run "400 click 798 711; 480 list"
login_ok "desligar pela tela de login" sh -c "sed 's/\x1b\[[0-9;]*m//g' '$OUT/mide.log' | grep -q 'sistema (teste): logind PowerOff'"

# Escala 1,5: o relógio na resolução da tela (muitos pixels brancos puros nas letras;
# esticado de 1x, as bordas viram cinza).
mkdir -p "$LG/cfg15/mide"
printf '[monitors.outputs."HEADLESS-1"]\nscale = 1.5\n' > "$LG/cfg15/mide/config.toml"
XDG_CONFIG_HOME="$LG/cfg15" SCENARIO_LOGIN_WAIT=8 login_run "300 list" 300 2049x1152
# Proporção de pixels brancos puros no relógio: em 1x ela fica perto de 11%; esticada
# de 1x para 1,5, as bordas das letras viram cinza e ela cai.
WHITE="$(python3 -c "
import sys
from PIL import Image
i = Image.open(sys.argv[1]).convert('L')
w = i.size[0]
k = w / 1366
box = i.crop((int(w / 2 - 200 * k), int(92 * k), int(w / 2 + 200 * k), int(200 * k)))
print(round(100 * sum(1 for p in box.getdata() if p >= 250) / (box.size[0] * box.size[1])))
" "$OUT/frame.png" 2>/dev/null)"
login_ok "escala 1,5: relógio nítido (${WHITE:-?}% de branco puro)" sh -c "[ '${WHITE:-0}' -ge 9 ]"

rm -rf "$LG"
