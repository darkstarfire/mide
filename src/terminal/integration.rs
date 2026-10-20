//! Integração do shell (ideia do iTerm2, sem precisar instalar nada): o terminal abre
//! o bash ou o zsh com um script do mide que avisa onde começa cada prompt, quando um
//! comando começa a rodar e com que código ele terminou (sequências OSC 133 da
//! FinalTerm) e a pasta atual (OSC 7). O script também traz o `imgcat` (mostra
//! imagens no terminal) quando o sistema não tem um.
//!
//! O script carrega os arquivos do usuário antes (`/etc/bash.bashrc`, `~/.bashrc`;
//! no zsh, o `ZDOTDIR` dele), então o shell fica como ele configurou.

use std::path::{Path, PathBuf};

use super::pty::Spawn;

const BASH: &str = r#"# Integração do shell do Terminal do mide (gerado; não edite).
if [ -z "$MIDE_INTEGRATION" ]; then
    MIDE_INTEGRATION=1
    # O --rcfile troca o ~/.bashrc por este arquivo: carrega o que o bash carregaria.
    [ -r /etc/bash.bashrc ] && . /etc/bash.bashrc
    [ -r "$HOME/.bashrc" ] && . "$HOME/.bashrc"
    __mide_osc() { printf '\033]%s\007' "$1"; }
    # Primeiro da fila: guarda o código do último comando e o devolve aos outros.
    __mide_precmd() {
        local status=$?
        __mide_osc "133;D;$status"
        __mide_osc "7;file://${HOSTNAME}${PWD}"
        __mide_osc "133;A"
        return $status
    }
    PROMPT_COMMAND="__mide_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
    PS0="${PS0}"$'\033]133;C\007'
    __MIDE_IMGCAT__
fi
"#;

const ZSHENV: &str = r#"# Integração do shell do Terminal do mide (gerado; não edite).
# Volta ao ZDOTDIR do usuário e carrega o .zshenv dele; os ganchos ficam registrados.
if [ -n "$MIDE_USER_ZDOTDIR" ]; then ZDOTDIR="$MIDE_USER_ZDOTDIR"; else unset ZDOTDIR; fi
unset MIDE_USER_ZDOTDIR
[ -r "${ZDOTDIR:-$HOME}/.zshenv" ] && . "${ZDOTDIR:-$HOME}/.zshenv"
if [[ -o interactive && -z "$MIDE_INTEGRATION" ]]; then
    MIDE_INTEGRATION=1
    __mide_osc() { printf '\033]%s\007' "$1"; }
    __mide_precmd() {
        local exit_status=$?
        __mide_osc "133;D;$exit_status"
        __mide_osc "7;file://${HOST}${PWD}"
        __mide_osc "133;A"
    }
    __mide_preexec() { __mide_osc "133;C"; }
    autoload -Uz add-zsh-hook
    add-zsh-hook precmd __mide_precmd
    add-zsh-hook preexec __mide_preexec
    __MIDE_IMGCAT__
fi
"#;

/// `imgcat arquivo...`: manda cada imagem pelo protocolo de imagens do iTerm2.
const IMGCAT: &str = r#"if ! command -v imgcat >/dev/null 2>&1; then
        imgcat() {
            local f
            for f in "$@"; do
                [ -r "$f" ] || { echo "imgcat: não foi possível ler $f" >&2; continue; }
                printf '\033]1337;File=inline=1;size=%s;name=%s:' "$(wc -c < "$f" | tr -d ' ')" "$(printf %s "${f##*/}" | base64 | tr -d '\n')"
                base64 < "$f" | tr -d '\n'
                printf '\007\n'
            done
        }
    fi"#;

/// Pasta dos scripts (`$XDG_RUNTIME_DIR/mide/shell`, ou a temporária).
fn dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("mide").join("shell")
}

fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(text) {
        return Ok(());
    }
    std::fs::write(path, text)
}

/// Ajusta o `spawn` para abrir o shell com a integração (só o shell do usuário, bash
/// ou zsh; um programa pedido com `-e` abre como está).
pub fn apply(spawn: &mut Spawn) {
    if spawn.command.is_some() {
        return;
    }
    let shell = super::pty::user_shell();
    let name = Path::new(&shell).file_name().and_then(|n| n.to_str()).unwrap_or("");
    let dir = dir();
    if std::fs::create_dir_all(dir.join("zsh")).is_err() {
        return;
    }
    match name {
        "bash" => {
            let path = dir.join("bash-integration.bash");
            if write(&path, &BASH.replace("__MIDE_IMGCAT__", IMGCAT)).is_ok() {
                spawn.command = Some((shell.clone(), vec!["--rcfile".into(), path.to_string_lossy().into_owned()]));
            }
        }
        "zsh" => {
            let path = dir.join("zsh").join(".zshenv");
            if write(&path, &ZSHENV.replace("__MIDE_IMGCAT__", IMGCAT)).is_ok() {
                spawn.command = Some((shell.clone(), Vec::new()));
                if let Some(user) = std::env::var_os("ZDOTDIR") {
                    spawn.env.push(("MIDE_USER_ZDOTDIR".into(), user.to_string_lossy().into_owned()));
                }
                spawn.env.push(("ZDOTDIR".into(), dir.join("zsh").to_string_lossy().into_owned()));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// O script do bash, rodado de verdade num PTY: marca o prompt, o começo e o fim
    /// do comando com o código de saída.
    #[test]
    fn bash_marca_prompt_e_fim_do_comando() {
        if Command::new("bash").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("mide-integracao-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rc = dir.join("rc.bash");
        std::fs::write(&rc, BASH.replace("__MIDE_IMGCAT__", IMGCAT)).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let spawn = Spawn {
            command: Some(("bash".into(), vec!["--rcfile".into(), rc.to_string_lossy().into_owned(), "-i".into()])),
            directory: Some(dir.clone()),
            env: vec![("HOME".into(), dir.to_string_lossy().into_owned())],
        };
        let pty = super::super::pty::Pty::spawn(&spawn, super::super::pty::Size { cols: 80, rows: 24, width: 0, height: 0 }, tx, || {}).unwrap();
        pty.write(b"false\rtype imgcat >/dev/null && echo tem-imgcat\rexit\r".to_vec());
        let mut out = Vec::new();
        while let Ok(chunk) = rx.recv_timeout(std::time::Duration::from_secs(5)) {
            if chunk.is_empty() {
                break;
            }
            out.extend(chunk);
        }
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("\x1b]133;A\x07"), "{text:?}");
        assert!(text.contains("\x1b]133;C\x07"), "{text:?}");
        assert!(text.contains("\x1b]133;D;1\x07"), "{text:?}");
        assert!(text.contains("tem-imgcat"), "{text:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
