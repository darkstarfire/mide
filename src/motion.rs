//! Movimento do mide num lugar só: as durações e as curvas das animações (a tabela da
//! seção 8 do `docs/design.md`). O compositor, a barra, a dock, o launcher e os apps
//! do mide-ui usam daqui; mudar um valor muda todos os lugares que fazem a mesma coisa.

use std::time::Duration;

/// Menu aparecendo (barra, dock, apps, contexto): esmaece e desce 6 px.
pub const MENU_APPEAR: Duration = Duration::from_millis(120);
/// Prévias da dock e alternador (Alt+Tab): esmaecem e sobem 8 px.
pub const PREVIEW_APPEAR: Duration = Duration::from_millis(160);
/// Aviso das teclas de volume e brilho (OSD): entra esmaecendo e subindo 8 px, fica
/// `OSD_HOLD` depois da última tecla e sai esmaecendo.
pub const OSD_APPEAR: Duration = Duration::from_millis(160);
pub const OSD_HOLD: Duration = Duration::from_millis(1500);
pub const OSD_FADE: Duration = Duration::from_millis(200);
/// Fechar janela: encolhe 6% e esmaece.
pub const CLOSE_FADE: Duration = Duration::from_millis(180);
/// Dock aparecendo/escondendo (desliza) e a espera antes de esconder.
pub const DOCK_SLIDE: Duration = Duration::from_millis(200);
pub const DOCK_HIDE_DELAY: Duration = Duration::from_millis(450);
/// Pastas abrindo na lista do Arquivos (cada linha começa um pouco depois da anterior)
/// e troca entre ícones e lista.
pub const LIST_EXPAND: Duration = Duration::from_millis(180);
pub const LIST_STAGGER: Duration = Duration::from_millis(25);
pub const VIEW_SWITCH: Duration = Duration::from_millis(200);
/// Vidro de uma janela aparecendo depois de animar.
pub const GLASS_FADE: Duration = Duration::from_millis(220);
/// Maximizar, restaurar, metade da tela: posição e tamanho interpolados.
pub const WINDOW_MOVE: Duration = Duration::from_millis(220);
/// Pasta do launcher abrindo/fechando.
pub const LAUNCHER_FOLDER: Duration = Duration::from_millis(240);
/// Launcher abrindo / fechando.
pub const LAUNCHER_OPEN: Duration = Duration::from_millis(260);
pub const LAUNCHER_CLOSE: Duration = Duration::from_millis(170);
/// Página do launcher (carrossel).
pub const LAUNCHER_PAGE: Duration = Duration::from_millis(320);
/// Pasta nova surgindo no launcher ("pula" no lugar).
pub const LAUNCHER_POP: Duration = Duration::from_millis(340);
/// Papel de parede trocando.
pub const WALLPAPER_FADE: Duration = Duration::from_millis(450);
/// Pílula do controle segmentado deslizando até a opção nova (Tahoe).
pub const CONTROL_SLIDE: Duration = Duration::from_millis(200);
/// Lâmpada (minimizar e restaurar).
pub const GENIE: Duration = Duration::from_millis(650);

/// Progresso de 0 a 1 de uma animação que começou em `start`.
pub fn progress(start: std::time::Instant, duration: Duration) -> f32 {
    (start.elapsed().as_secs_f32() / duration.as_secs_f32().max(f32::EPSILON)).clamp(0.0, 1.0)
}

/// Entrar e mover: desaceleração cúbica (rápido no começo, assenta devagar).
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Desaceleração leve (quadrática), para esmaecer o que surge.
pub fn ease_out_soft(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * (2.0 - t)
}

/// Esmaecer de troca: suave nos dois lados (`t²(3-2t)`).
pub fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Acelera e desacelera (cúbica), como na lâmpada.
pub fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// Mola: passa um pouco do fim e volta (pasta nova surgindo).
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c = 1.70158;
    1.0 + (c + 1.0) * (t - 1.0).powi(3) + c * (t - 1.0).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curvas_comecam_em_0_e_terminam_em_1() {
        for f in [ease_out, ease_out_soft, smooth, ease_in_out, ease_out_back] {
            assert!(f(0.0).abs() < 1e-5);
            assert!((f(1.0) - 1.0).abs() < 1e-5);
        }
        // A mola passa do fim no meio do caminho.
        assert!(ease_out_back(0.7) > 1.0);
    }
}
