//! Temas de cores do Terminal do mide: texto, fundo, cursor, seleção e as 16 cores
//! básicas. As outras 240 da paleta de 256 (cubo 6x6x6 e cinzas) são fixas.

use super::screen::Rgb;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    pub ansi: [Rgb; 16],
}

impl Default for Theme {
    fn default() -> Self {
        THEMES[0].clone()
    }
}

/// Temas prontos (o primeiro é o padrão): claro como o resto do mide, e escuros.
pub const THEMES: &[Theme] = &[
    Theme {
        name: "Claro",
        foreground: [24, 24, 28],
        background: [255, 255, 255],
        cursor: [96, 96, 104],
        selection: [179, 215, 255],
        ansi: [
            [0, 0, 0],
            [194, 54, 33],
            [37, 158, 36],
            [163, 140, 20],
            [53, 74, 215],
            [181, 56, 181],
            [30, 150, 165],
            [190, 190, 195],
            [110, 112, 118],
            [230, 60, 40],
            [40, 190, 40],
            [200, 170, 20],
            [80, 100, 255],
            [220, 70, 220],
            [20, 180, 200],
            [235, 235, 238],
        ],
    },
    Theme {
        name: "Escuro",
        foreground: [228, 228, 232],
        background: [30, 30, 34],
        cursor: [200, 200, 208],
        selection: [62, 88, 130],
        ansi: [
            [40, 40, 46],
            [240, 90, 80],
            [110, 210, 100],
            [235, 200, 90],
            [100, 150, 255],
            [210, 120, 230],
            [90, 210, 220],
            [205, 205, 212],
            [110, 110, 120],
            [255, 120, 110],
            [150, 235, 135],
            [250, 220, 120],
            [140, 180, 255],
            [230, 150, 245],
            [130, 230, 240],
            [255, 255, 255],
        ],
    },
    Theme {
        name: "Grafite",
        foreground: [210, 214, 220],
        background: [44, 48, 54],
        cursor: [240, 180, 80],
        selection: [80, 92, 110],
        ansi: [
            [58, 62, 70],
            [224, 108, 117],
            [152, 195, 121],
            [229, 192, 123],
            [97, 175, 239],
            [198, 120, 221],
            [86, 182, 194],
            [200, 204, 212],
            [100, 106, 118],
            [240, 130, 138],
            [175, 215, 145],
            [240, 210, 150],
            [130, 195, 250],
            [215, 150, 235],
            [120, 205, 215],
            [240, 242, 245],
        ],
    },
    Theme {
        name: "Oceano",
        foreground: [200, 225, 235],
        background: [12, 36, 52],
        cursor: [120, 220, 230],
        selection: [30, 80, 110],
        ansi: [
            [20, 50, 70],
            [235, 100, 90],
            [100, 210, 150],
            [235, 205, 110],
            [80, 160, 240],
            [190, 130, 230],
            [80, 210, 220],
            [190, 215, 225],
            [70, 110, 130],
            [255, 130, 120],
            [140, 235, 175],
            [250, 225, 140],
            [120, 190, 255],
            [215, 160, 245],
            [120, 235, 240],
            [240, 250, 255],
        ],
    },
];

/// Tema pelo nome (o padrão se não existir).
pub fn theme(name: &str) -> Theme {
    THEMES.iter().find(|t| t.name == name).cloned().unwrap_or_default()
}

/// Cor de cada índice da paleta de 256: as 16 do tema, o cubo 6x6x6 e os cinzas.
pub fn default_color(theme: &Theme, index: u8) -> Rgb {
    let i = index as usize;
    match i {
        0..16 => theme.ansi[i],
        16..232 => {
            let i = i - 16;
            let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
            [level(i / 36), level(i / 6 % 6), level(i % 6)]
        }
        _ => {
            let v = (8 + (i - 232) * 10) as u8;
            [v, v, v]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paleta_de_256() {
        let t = Theme::default();
        assert_eq!(default_color(&t, 1), t.ansi[1]);
        assert_eq!(default_color(&t, 16), [0, 0, 0]);
        assert_eq!(default_color(&t, 231), [255, 255, 255]);
        assert_eq!(default_color(&t, 232), [8, 8, 8]);
        assert_eq!(default_color(&t, 255), [238, 238, 238]);
    }

    #[test]
    fn temas_pelo_nome() {
        assert_eq!(theme("Escuro").name, "Escuro");
        assert_eq!(theme("não existe").name, "Claro");
    }
}
