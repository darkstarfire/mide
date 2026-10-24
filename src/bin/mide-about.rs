//! Janela "Sobre este computador", no estilo do Mac (Tahoe): em pé, de vidro, com a
//! imagem do computador (desenhada aqui), o nome do modelo, o tipo e o ano, as
//! especificações (quebrando a linha) e o botão "Mais informações…", que abre
//! Configurações → Sobre. Só o botão de fechar funciona.

use mide::kit::{run, App, ButtonStyle, Key, Ui, Window};
use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, SpreadMode, Transform};

/// Tamanho da janela (pontos): retrato, como a do Mac.
const W: f32 = 330.0;
const H: f32 = 610.0;

struct About {
    machine: mide::sysinfo::Machine,
}

impl App for About {
    fn view(&mut self, ui: &mut Ui) {
        if ui.input().key(Key::Escape) {
            ui.close();
        }
        let (w, h) = ui.window_size();
        // Vidro na janela inteira: o compositor desfoca o que está atrás, e um véu
        // translúcido deixa ver a cor do papel de parede (opaco com o vidro desligado).
        let glass = ui.glass_behind((0.0, 0.0, w, h), mide::kit::RADIUS_WINDOW);
        let dark = ui.theme().dark;
        let veil = match (glass, dark) {
            (true, false) => [246, 246, 248, 170],
            (true, true) => [34, 34, 38, 170],
            (false, false) => [246, 246, 248, 255],
            (false, true) => [34, 34, 38, 255],
        };
        ui.paint(|canvas, _| {
            canvas.fill(Color::TRANSPARENT);
            mide::ui::draw::paint_rounded_solid(canvas, 0.0, 0.0, w, h, 0.0, veil);
        });
        ui.window_close_only(16.0, 16.0);
        ui.window_drag_area((0.0, 0.0, w, 44.0));

        // Imagem do computador.
        let laptop = self.machine.laptop;
        let (iw, ih) = (180.0, 120.0);
        let (ix, iy) = ((w - iw) / 2.0, 52.0);
        ui.paint(|canvas, _| draw_computer(canvas, ix, iy, iw, ih, laptop));
        ui.at(20.0, iy + ih + 22.0, w - 40.0, |ui| {
            ui.centered_title(&self.machine.name);
            ui.centered_note(&self.machine.subtitle);
            ui.space(10.0);
            for (key, value) in &self.machine.specs {
                ui.key_value(key, value, 140.0);
            }
            ui.space(10.0);
            ui.row(|ui| {
                let bw = 160.0;
                let x = (ui.remaining_width() - bw) / 2.0;
                ui.space_x(x);
                ui.sized(bw, |ui| {
                    if ui.button_styled("Mais informações…", ButtonStyle::Bordered).clicked() {
                        let _ = std::process::Command::new("mide-settings").arg("about").spawn();
                    }
                });
            });
            ui.space(8.0);
            ui.centered_note("O mide é criado do zero, sem código, imagens ou fontes da Apple.");
        });
    }
}

/// Desenho próprio do computador: monitor com base, ou notebook aberto, com a tela num
/// degradê nas cores do mide.
fn draw_computer(canvas: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, laptop: bool) {
    let tf = mide::ui::draw::tf();
    let fill = |canvas: &mut Pixmap, path: &tiny_skia::Path, c: Color| {
        let mut p = Paint::default();
        p.anti_alias = true;
        p.set_color(c);
        canvas.fill_path(path, &p, FillRule::Winding, tf, None);
    };
    let screen_grad = |sx: f32, sy: f32, sh: f32| {
        LinearGradient::new(
            Point::from_xy(sx, sy),
            Point::from_xy(sx, sy + sh),
            vec![
                GradientStop::new(0.0, Color::from_rgba8(40, 46, 120, 255)),
                GradientStop::new(0.55, Color::from_rgba8(150, 80, 170, 255)),
                GradientStop::new(1.0, Color::from_rgba8(245, 160, 120, 255)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
    };
    if laptop {
        // Tela.
        let (sw, sh) = (w * 0.78, h * 0.72);
        let (sx, sy) = (x + (w - sw) / 2.0, y);
        mide::ui::draw::soft_shadow(canvas, sx, sy, sw, sh, 8.0, 10.0, 4.0, 50);
        if let Some(body) = mide::ui::draw::rounded_rect(sx, sy, sw, sh, 8.0) {
            fill(canvas, &body, Color::from_rgba8(36, 36, 40, 255));
        }
        if let (Some(glass), Some(grad)) = (mide::ui::draw::rounded_rect(sx + 5.0, sy + 5.0, sw - 10.0, sh - 8.0, 4.0), screen_grad(sx, sy, sh)) {
            let mut p = Paint::default();
            p.anti_alias = true;
            p.shader = grad;
            canvas.fill_path(&glass, &p, FillRule::Winding, tf, None);
        }
        // Base (teclado visto de frente: um trapézio fino).
        let mut pb = PathBuilder::new();
        let by = sy + sh;
        pb.move_to(x + w * 0.04, by);
        pb.line_to(x + w * 0.96, by);
        pb.line_to(x + w, by + 8.0);
        pb.line_to(x, by + 8.0);
        pb.close();
        if let Some(base) = pb.finish() {
            fill(canvas, &base, Color::from_rgba8(196, 198, 204, 255));
        }
    } else {
        // Monitor com borda fina, haste e pé.
        let (sw, sh) = (w, h * 0.74);
        let (sx, sy) = (x, y);
        mide::ui::draw::soft_shadow(canvas, sx, sy, sw, sh, 8.0, 10.0, 4.0, 50);
        if let Some(body) = mide::ui::draw::rounded_rect(sx, sy, sw, sh, 8.0) {
            fill(canvas, &body, Color::from_rgba8(30, 30, 34, 255));
        }
        if let (Some(glass), Some(grad)) = (mide::ui::draw::rounded_rect(sx + 5.0, sy + 5.0, sw - 10.0, sh - 10.0, 4.0), screen_grad(sx, sy, sh)) {
            let mut p = Paint::default();
            p.anti_alias = true;
            p.shader = grad;
            canvas.fill_path(&glass, &p, FillRule::Winding, tf, None);
        }
        let neck_w = w * 0.12;
        if let Some(neck) = mide::ui::draw::rounded_rect(x + (w - neck_w) / 2.0, sy + sh, neck_w, h * 0.18, 2.0) {
            fill(canvas, &neck, Color::from_rgba8(176, 178, 184, 255));
        }
        let foot_w = w * 0.42;
        if let Some(foot) = mide::ui::draw::rounded_rect(x + (w - foot_w) / 2.0, sy + sh + h * 0.18, foot_w, 6.0, 3.0) {
            fill(canvas, &foot, Color::from_rgba8(196, 198, 204, 255));
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_about=info".into()))
        .init();
    let machine = mide::sysinfo::machine();
    tracing::info!("sobre este computador: {} ({})", machine.name, machine.subtitle);
    let window = Window::new("Sobre este computador").app_id("mide-about").size(W as u32, H as u32).min_size(W as u32, H as u32).full_size_content();
    run(window, About { machine })
}
