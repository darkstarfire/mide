//! Miniatura da captura (B23): depois de salvar uma captura, ela aparece no canto de
//! baixo à direita da tela principal, acima da dock, por 5 segundos.
//!
//! - Clique: abre a captura no editor.
//! - Clique direito: Abrir no editor, Mostrar no Arquivos, Copiar e Mover para a Lixeira.
//! - Arrastar para a direita (para fora da tela): some na hora (o arquivo fica salvo).
//! - Com o ponteiro em cima (ou o menu aberto), não some.
//!
//! Entra e sai com o efeito "Surgir" do `docs/design.md` (esmaece e desliza 8 pontos a
//! partir da borda da tela; 160 ms entrando, 120 ms saindo), com o interruptor
//! "Transições suaves" de Configurações → Efeitos. Uma captura nova fecha a miniatura
//! anterior na hora (`thumbnail_pid_file`).

use std::{
    error::Error,
    path::PathBuf,
    time::{Duration, Instant},
};

use mide::{
    control_client::ask_compositor,
    kit::{run, App, Ui, Window},
    markup::{sibling_program, thumbnail_pid_file, RectF},
    motion,
};
use tiny_skia::{FillRule, FilterQuality, Paint, Pattern, Pixmap, SpreadMode, Transform};

/// Tamanho da camada (pontos): a miniatura embaixo à direita, com lugar em cima para o
/// menu de contexto.
const LAYER: (u32, u32) = (280, 300);
/// Maior tamanho da miniatura e a distância dela até a borda da camada.
const MAX_THUMB: (f32, f32) = (180.0, 120.0);
const MARGIN: f32 = 16.0;
/// Moldura branca em volta da imagem e o canto.
const FRAME: f32 = 4.0;
const RADIUS: f32 = 10.0;
/// Sombra em volta (a imagem do cartão tem essa folga).
const SHADOW: f32 = 18.0;
/// Quanto tempo fica, e quanto mais depois que o ponteiro sai de cima.
const SHOW_FOR: Duration = Duration::from_secs(5);
const LINGER: Duration = Duration::from_secs(2);
/// Saída: mais curta que a entrada (design.md, 9.5).
const LEAVE: Duration = motion::MENU_APPEAR;
/// Arrastada mais que isso para a direita, some ao soltar.
const DISMISS_DRAG: f32 = 40.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leave {
    Timeout,
    Dragged,
    Editor,
    Files,
    Trash,
    /// Arrastada para outro app (o arquivo foi junto).
    DragOut,
}

struct Thumb {
    path: PathBuf,
    image: Pixmap,
    /// Tamanho da miniatura em pontos.
    size: (f32, f32),
    /// O cartão (sombra, moldura e imagem) pronto na escala da tela.
    card: Option<(f32, Pixmap)>,
    shown_at: Option<Instant>,
    deadline: Instant,
    leaving: Option<(Instant, Leave)>,
    /// Onde o botão desceu (pontos da camada) e o arrasto para a direita.
    press: Option<(f32, f32)>,
    offset: f32,
    /// Desde quando o ponteiro está fora da camada com o menu aberto.
    away_since: Option<Instant>,
    animate: bool,
    /// A miniatura está sendo arrastada para outro app (Telegram, WhatsApp, uma janela).
    dragging_out: bool,
}

/// Abre a miniatura de `path` (a captura que acabou de ser salva).
pub fn run_thumbnail(path: PathBuf) -> Result<(), Box<dyn Error>> {
    let image = super::doc::load(&path).map_err(|e| format!("miniatura: {}: {e}", path.display()))?;
    // O tamanho em pontos depende da escala da tela, conhecida só no primeiro quadro.
    let size = thumb_size((image.width(), image.height()), 1.0);
    let pid_file = thumbnail_pid_file();
    let _ = std::fs::write(&pid_file, std::process::id().to_string());
    tracing::info!("miniatura da captura: {} ({}x{} pixels)", path.display(), image.width(), image.height());
    let animate = mide::settings_file::effect("transitions");
    let app = Thumb { path, image, size, card: None, shown_at: None, deadline: Instant::now() + SHOW_FOR, leaving: None, press: None, offset: 0.0, away_since: None, animate, dragging_out: false };
    let result = run(Window::new("Miniatura da captura").app_id("mide-screenshot-editor").corner("mide-captura-miniatura").size(LAYER.0, LAYER.1), app);
    // O arquivo do PID só sai se ainda for desta miniatura (uma nova pode ter gravado o dela).
    if std::fs::read_to_string(&pid_file).is_ok_and(|p| p.trim() == std::process::id().to_string()) {
        let _ = std::fs::remove_file(&pid_file);
    }
    result
}

/// Tamanho da miniatura (pontos) de uma imagem de `px` pixels numa tela de escala `k`:
/// cabe em `MAX_THUMB`, sem passar do tamanho dela na tela (um pixel da imagem num
/// pixel da tela), e nunca fina demais para clicar.
fn thumb_size(px: (u32, u32), k: f32) -> (f32, f32) {
    let k = k.max(0.1);
    let (w, h) = (px.0.max(1) as f32 / k, px.1.max(1) as f32 / k);
    let s = (MAX_THUMB.0 / w).min(MAX_THUMB.1 / h).min(1.0);
    ((w * s).round().max(24.0), (h * s).round().max(24.0))
}

/// Arrasto para a direita (pontos) que descarta a miniatura, quando o botão desceu em
/// `start_x` numa camada de largura `layer_w`: `DISMISS_DRAG`, ou menos perto da borda
/// da tela (o ponteiro para nela: a camada encosta na borda direita).
fn dismiss_drag(start_x: f32, layer_w: f32) -> f32 {
    DISMISS_DRAG.min(layer_w - 4.0 - start_x).max(6.0)
}

/// Imagem que segue o ponteiro ao arrastar a captura: a própria captura, reduzida a
/// no máximo 160 pixels, com cantos arredondados.
fn drag_image(image: &Pixmap) -> Option<Pixmap> {
    let (w, h) = (image.width() as f32, image.height() as f32);
    let k = (160.0 / w.max(h)).min(1.0);
    let (dw, dh) = ((w * k).round().max(1.0), (h * k).round().max(1.0));
    let mut out = Pixmap::new(dw as u32, dh as u32)?;
    let mask = tiny_skia::Mask::new(dw as u32, dh as u32).and_then(|mut m| {
        let path = mide::ui::draw::rounded_rect(0.0, 0.0, dw, dh, 8.0)?;
        m.fill_path(&path, tiny_skia::FillRule::Winding, true, tiny_skia::Transform::identity());
        Some(m)
    });
    let paint = tiny_skia::PixmapPaint { opacity: 0.9, quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
    out.draw_pixmap(0, 0, image.as_ref(), &paint, tiny_skia::Transform::from_scale(k, k), mask.as_ref());
    Some(out)
}

/// Ainda é a miniatura da vez? Uma captura nova grava o PID dela no arquivo.
fn still_ours() -> bool {
    match std::fs::read_to_string(thumbnail_pid_file()) {
        Ok(pid) => pid.trim() == std::process::id().to_string(),
        // Sem arquivo (apagado à mão): continua.
        Err(_) => true,
    }
}

impl Thumb {
    /// Onde a miniatura fica parada (pontos da camada).
    fn rest(&self, layer: (f32, f32)) -> RectF {
        (layer.0 - MARGIN - self.size.0, layer.1 - MARGIN - self.size.1, self.size.0, self.size.1)
    }

    fn leave(&mut self, why: Leave) {
        if self.leaving.is_none() {
            self.leaving = Some((Instant::now(), why));
        }
    }

    /// O cartão na escala `k`: sombra, moldura branca e a imagem com os cantos
    /// arredondados.
    fn card(&mut self, k: f32, frame_color: [u8; 4]) -> &Pixmap {
        if self.card.as_ref().is_none_or(|(ck, _)| *ck != k) {
            let (w, h) = self.size;
            let (cw, ch) = (((w + 2.0 * SHADOW) * k).ceil() as u32, ((h + 2.0 * SHADOW) * k).ceil() as u32);
            let mut card = Pixmap::new(cw.max(1), ch.max(1)).expect("tamanho do cartão");
            mide::ui::draw::soft_shadow(&mut card, SHADOW, SHADOW, w, h, RADIUS, 16.0, 5.0, 70);
            mide::ui::draw::fill_rounded(&mut card, SHADOW, SHADOW, w, h, RADIUS, tiny_skia::Color::from_rgba8(frame_color[0], frame_color[1], frame_color[2], frame_color[3]));
            let (iw, ih) = ((w - 2.0 * FRAME).max(1.0), (h - 2.0 * FRAME).max(1.0));
            let small = super::shrink(&self.image, (iw * k).round().max(1.0) as u32, (ih * k).round().max(1.0) as u32);
            let (ix, iy) = ((SHADOW + FRAME) * k, (SHADOW + FRAME) * k);
            let pattern = Pattern::new(small.as_ref(), SpreadMode::Pad, FilterQuality::Nearest, 1.0, Transform::from_translate(ix.round(), iy.round()));
            let paint = Paint { shader: pattern, anti_alias: true, ..Default::default() };
            if let Some(path) = mide::ui::draw::rounded_rect(ix.round(), iy.round(), small.width() as f32, small.height() as f32, (RADIUS - FRAME) * k) {
                card.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            }
            self.card = Some((k, card));
        }
        &self.card.as_ref().expect("acabou de ser feito").1
    }

    /// Ações do menu e do clique.
    fn open_editor(&mut self) {
        match std::process::Command::new(sibling_program("mide-screenshot-editor")).arg(&self.path).spawn() {
            Ok(_) => tracing::info!("miniatura: abrindo no editor"),
            Err(err) => tracing::warn!("miniatura: o editor não abriu: {err}"),
        }
        self.leave(Leave::Editor);
    }

    fn copy(&self) {
        let ok = ask_compositor(&format!("copy-file image/png {}", self.path.display())).is_some_and(|r| r.first().map(String::as_str) == Some("ok"));
        if ok {
            tracing::info!("miniatura: copiada");
        } else {
            tracing::warn!("miniatura: a cópia não foi feita (o mide não respondeu)");
        }
    }

    fn trash(&mut self) {
        match mide::places::move_to_trash(&self.path, &mide::places::trash_dir()) {
            Ok(name) => {
                tracing::info!("miniatura: movida para a Lixeira ({name})");
                mide::sounds::play("trash-empty");
                self.leave(Leave::Trash);
            }
            Err(err) => tracing::warn!("miniatura: não foi para a Lixeira: {err}"),
        }
    }
}

impl App for Thumb {
    fn view(&mut self, ui: &mut Ui) {
        let now = Instant::now();
        if !still_ours() {
            tracing::info!("miniatura: fechada por outra captura");
            ui.close();
            return;
        }
        let layer = ui.window_size();
        // Na escala da tela (conhecida agora; muda se a tela mudar).
        let size = thumb_size((self.image.width(), self.image.height()), mide::ui::draw::scale());
        if size != self.size {
            self.size = size;
            self.card = None;
        }
        let rest = self.rest(layer);
        let shown = *self.shown_at.get_or_insert_with(|| {
            tracing::info!("miniatura: apareceu");
            now
        });
        // Entrada: esmaece e desliza 8 pontos a partir da borda direita.
        let enter = if self.animate { motion::ease_out(motion::progress(shown, motion::PREVIEW_APPEAR)) } else { 1.0 };
        let (mut alpha, mut dx) = (enter, (1.0 - enter) * 8.0);
        let mut animating = enter < 1.0;
        if let Some((at, why)) = self.leaving {
            let t = if self.animate { motion::ease_out(motion::progress(at, LEAVE)) } else { 1.0 };
            alpha *= 1.0 - t;
            dx += t * 8.0;
            if t >= 1.0 {
                tracing::info!("miniatura: fechada ({why:?})");
                ui.close();
                return;
            }
            animating = true;
        }
        dx += self.offset;
        if self.dragging_out {
            alpha *= 0.35;
        }
        let rect = (rest.0 + dx, rest.1, rest.2, rest.3);

        if self.leaving.is_none() {
            let input = ui.input().clone();
            let menu = ui.menu_open();
            let over = input.over(rect);
            // Enquanto a captura é arrastada para outro app, a miniatura não some (o
            // programa precisa entregar o arquivo quando soltar).
            if over || menu.is_some() || self.press.is_some() || self.dragging_out {
                self.deadline = self.deadline.max(now + LINGER);
            }
            // O menu de contexto fecha quando o ponteiro fica fora da camada (o clique
            // fora dela não chega aqui).
            if menu.is_some() && input.pointer.is_none() {
                let since = *self.away_since.get_or_insert(now);
                if now.duration_since(since) > Duration::from_millis(1500) {
                    ui.close_menu();
                }
            } else {
                self.away_since = None;
            }
            if menu.is_none() {
                if input.pressed && over {
                    self.press = input.pointer;
                }
                if let (Some(start), Some(p)) = (self.press, input.pointer) {
                    let (mx, my) = (p.0 - start.0, p.1 - start.1);
                    // Arrastar para a esquerda, para cima ou para baixo leva a captura para
                    // outro app (como no Mac); para a direita, descarta.
                    if input.down && !self.dragging_out && (mx < -6.0 || my.abs() > 6.0) {
                        tracing::info!("miniatura: arrastando a captura para outro app");
                        let icon = drag_image(&self.image);
                        ui.start_drag(std::slice::from_ref(&self.path), icon);
                        self.dragging_out = true;
                        self.press = None;
                        self.offset = 0.0;
                        self.deadline = self.deadline.max(now + LINGER);
                    }
                }
                if let (Some(start), Some(p)) = (self.press, input.pointer) {
                    // Só para a direita (para fora da tela); para a esquerda fica no lugar.
                    self.offset = (p.0 - start.0).max(0.0);
                    if input.released {
                        let moved = (p.0 - start.0).hypot(p.1 - start.1);
                        self.press = None;
                        if self.offset >= dismiss_drag(start.0, layer.0) {
                            tracing::info!("miniatura: descartada (arrastada para fora)");
                            self.leave(Leave::Dragged);
                        } else if moved < 6.0 {
                            self.offset = 0.0;
                            self.open_editor();
                        } else {
                            self.offset = 0.0;
                        }
                    }
                    animating = true;
                } else if input.released {
                    self.press = None;
                    self.offset = 0.0;
                }
            }
            // O arrasto para outro app acabou (solto ou cancelado): a miniatura sai, e a
            // captura continua salva.
            if self.dragging_out && input.drag_ended {
                self.dragging_out = false;
                self.leave(Leave::DragOut);
            }
            // Clique direito: o menu.
            let r = ui.background(rect);
            let (mut edit, mut files, mut copy, mut trash) = (false, false, false, false);
            ui.context_menu(&r, |m| {
                edit = m.item("Abrir no editor");
                files = m.item("Mostrar no Arquivos");
                copy = m.item("Copiar");
                m.separator();
                trash = m.danger("Mover para a Lixeira");
            });
            if edit {
                self.open_editor();
            }
            if files {
                super::show_in_files(&self.path);
                self.leave(Leave::Files);
            }
            if copy {
                self.copy();
            }
            if trash {
                self.trash();
            }
            if self.leaving.is_none() && now >= self.deadline && ui.menu_open().is_none() {
                tracing::info!("miniatura: some sozinha ({} s)", SHOW_FOR.as_secs());
                self.leave(Leave::Timeout);
                animating = true;
            }
        }

        // Desenho.
        let k = mide::ui::draw::scale();
        let frame_color = ui.theme().card;
        let card = self.card(k, frame_color).clone();
        let (x, y) = (rect.0 - SHADOW, rect.1 - SHADOW);
        ui.paint(|canvas, _| {
            mide::ui::draw::blit_px(canvas, &card, mide::ui::draw::px(x), mide::ui::draw::px(y), alpha);
        });
        // Só a miniatura recebe cliques (com o menu aberto, a camada toda).
        match ui.menu_open() {
            Some(_) => ui.input_area((0.0, 0.0, layer.0, layer.1)),
            None => ui.input_area(rect),
        }
        if animating {
            ui.request_tick();
        } else {
            // Confere de vez em quando se uma captura nova chegou, e o fim do tempo.
            let left = self.deadline.saturating_duration_since(now);
            ui.wake_after(left.min(Duration::from_millis(200)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tamanho_da_miniatura() {
        assert_eq!(thumb_size((1366, 768), 1.0), (180.0, 101.0));
        assert_eq!(thumb_size((768, 1366), 1.0), (67.0, 120.0));
        // Pequena: do tamanho dela; fina demais: pelo menos 24 pontos.
        assert_eq!(thumb_size((100, 50), 1.0), (100.0, 50.0));
        assert_eq!(thumb_size((2000, 10), 1.0), (180.0, 24.0));
        // Tela 2x: 200x100 pixels são 100x50 pontos (não 200x100); em 1,25, 160x80.
        assert_eq!(thumb_size((200, 100), 2.0), (100.0, 50.0));
        assert_eq!(thumb_size((200, 100), 1.25), (160.0, 80.0));
        // Grande em 2x: o mesmo tamanho máximo.
        assert_eq!(thumb_size((2732, 1536), 2.0), (180.0, 101.0));
    }

    #[test]
    fn arrastar_perto_da_borda_tambem_descarta() {
        // Longe da borda: 40 pontos.
        assert_eq!(dismiss_drag(100.0, 280.0), DISMISS_DRAG);
        // A 20 pontos da borda: chega até a borda (menos 4) e descarta.
        assert_eq!(dismiss_drag(260.0, 280.0), 16.0);
        // Nunca menos que um arrasto de verdade (um clique não descarta).
        assert_eq!(dismiss_drag(278.0, 280.0), 6.0);
    }
}
