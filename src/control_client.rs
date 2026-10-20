//! Lado cliente do canal de controle do compositor (`$XDG_RUNTIME_DIR/<socket>.ctl`).

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

/// Caminho do canal de controle do compositor (`<socket>.ctl`).
pub fn control_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let display = std::env::var("WAYLAND_DISPLAY").ok()?;
    Some(PathBuf::from(runtime).join(format!("{display}.ctl")))
}

/// Envia um comando ao compositor e devolve as linhas da resposta (sem `end`).
pub fn ask_compositor(command: &str) -> Option<Vec<String>> {
    let mut stream = UnixStream::connect(control_path()?).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(300))).ok()?;
    stream.write_all(format!("{command}\n").as_bytes()).ok()?;
    let mut lines = Vec::new();
    for line in BufReader::new(stream).lines() {
        let line = line.ok()?;
        if line == "end" {
            break;
        }
        lines.push(line);
    }
    Some(lines)
}

/// Regiões de vidro de uma camada (barra, menus, notificações). Cada camada guarda o
/// último pedido aqui e só avisa o compositor quando muda. O envio é feito numa thread
/// própria, em ordem e só o pedido mais recente: o desenho não espera o compositor
/// (nas animações do launcher, um pedido por quadro).
#[derive(Default)]
pub struct Glass {
    last: Option<String>,
    sender: Option<std::sync::mpsc::Sender<String>>,
}

impl Glass {
    /// Pede vidro atrás de `rects` (x, y, largura, altura, raio), relativos à camada
    /// `namespace`. Lista vazia limpa.
    pub fn set(&mut self, namespace: &str, rects: &[(f32, f32, f32, f32, f32)]) {
        let with_alpha: Vec<_> = rects.iter().map(|(x, y, w, h, r)| (*x, *y, *w, *h, *r, 1.0)).collect();
        self.set_faded(namespace, &with_alpha);
    }

    /// Como `set`, com a opacidade de cada região (0 a 1): vidro que aparece ou some
    /// junto com o desenho da camada (as pastas do launcher nas transições).
    pub fn set_faded(&mut self, namespace: &str, rects: &[(f32, f32, f32, f32, f32, f32)]) {
        // Nunca pediu vidro e continua sem: nada a avisar.
        if rects.is_empty() && self.last.is_none() {
            return;
        }
        let mut command = format!("glass {namespace}");
        for (x, y, w, h, r, a) in rects {
            if *a >= 0.999 {
                command.push_str(&format!(" {r}:{},{},{},{}", x.round(), y.round(), w.round(), h.round()));
            } else if *a > 0.01 {
                command.push_str(&format!(" {r}@{a:.2}:{},{},{},{}", x.round(), y.round(), w.round(), h.round()));
            }
        }
        if self.last.as_deref() == Some(command.as_str()) {
            return;
        }
        let sender = self.sender.get_or_insert_with(|| {
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            std::thread::spawn(move || {
                while let Ok(mut command) = rx.recv() {
                    // Chegaram vários enquanto o anterior era enviado: só o último vale.
                    while let Ok(newer) = rx.try_recv() {
                        command = newer;
                    }
                    if ask_compositor(&command).is_none() {
                        tracing::debug!("vidro: compositor não respondeu ao pedido");
                    }
                }
            });
            tx
        });
        if sender.send(command.clone()).is_ok() {
            self.last = Some(command);
        }
    }
}

/// O vidro das barras e menus está ligado nas Configurações? Sem ele, os painéis
/// usam o fundo claro quase opaco de antes.
pub fn panel_glass() -> bool {
    crate::settings_file::effect("panel_glass")
}
