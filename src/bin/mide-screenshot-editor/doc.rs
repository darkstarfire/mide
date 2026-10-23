//! O documento do Editor de Capturas: a imagem aberta e as edições por cima dela
//! (marcações e recortes), com desfazer e refazer. Nada é aplicado na imagem original:
//! o resultado é montado quando precisa (mostrar, salvar, copiar), então desfazer um
//! recorte traz de volta o que estava fora dele.
//!
//! Coordenadas: tudo em pixels da imagem original.

use std::path::Path;

use mide::markup::{draw_mark, MarkKind, RectF};
use tiny_skia::{IntRect, Pixmap, Transform};

/// Uma marcação: tipo, cor, pontos (pixels da imagem) e a espessura do traço.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    pub color: [u8; 3],
    pub points: Vec<(f32, f32)>,
    pub width: f32,
}

/// Uma edição.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Mark(Mark),
    /// O recorte passa a ser esta área (pixels inteiros da imagem original).
    Crop(IntRect),
}

pub struct Doc {
    /// A imagem como foi aberta (pré-multiplicada, como o tiny-skia usa).
    pub base: Pixmap,
    ops: Vec<Op>,
    undone: Vec<Op>,
    /// Aumenta a cada mudança (inclusive desfazer e refazer): serve de chave de cache.
    pub version: u64,
    /// Quantas edições tinha quando foi salvo (ou aberto); `None` quando esse estado não
    /// pode mais voltar (desfeito e trocado por outra edição). Como toda edição nova
    /// apaga o refazer, o número de edições identifica o estado.
    saved: Option<usize>,
    /// A imagem com as marcações (sem o recorte), da versão guardada.
    composed: Option<(u64, Pixmap)>,
}

impl Doc {
    pub fn new(base: Pixmap) -> Self {
        Self { base, ops: Vec::new(), undone: Vec::new(), version: 0, saved: Some(0), composed: None }
    }

    pub fn width(&self) -> u32 {
        self.base.width()
    }

    pub fn height(&self) -> u32 {
        self.base.height()
    }

    /// A imagem inteira, em pixels.
    pub fn full(&self) -> IntRect {
        IntRect::from_xywh(0, 0, self.width(), self.height()).expect("imagem com tamanho")
    }

    /// O recorte atual (o último), ou a imagem inteira.
    pub fn crop(&self) -> IntRect {
        self.ops.iter().rev().find_map(|op| if let Op::Crop(r) = op { Some(*r) } else { None }).unwrap_or_else(|| self.full())
    }

    /// Espessura padrão do traço, proporcional ao tamanho da imagem (4 pixels numa
    /// tela comum; mais grosso numa imagem grande, para não sumir quando reduzida).
    pub fn stroke_width(&self) -> f32 {
        4.0 * (self.width().max(self.height()) as f32 / 1600.0).max(1.0)
    }

    /// Há mudanças que não foram salvas (desfazer até o estado salvo volta a `false`).
    pub fn modified(&self) -> bool {
        self.saved != Some(self.ops.len())
    }

    /// O estado atual foi salvo.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.ops.len());
    }

    pub fn push(&mut self, op: Op) {
        // O estado salvo estava no refazer, que esta edição apaga: não volta mais.
        if self.saved.is_some_and(|s| s > self.ops.len()) {
            self.saved = None;
        }
        self.undone.clear();
        self.apply(op);
    }

    /// Acrescenta a edição. Uma marcação é desenhada só ela, por cima da imagem já
    /// montada (sem refazer as outras: numa captura 4K, recompor tudo a cada traço
    /// custa caro).
    fn apply(&mut self, op: Op) {
        let current = self.version;
        self.version += 1;
        if let Some((v, image)) = self.composed.as_mut()
            && *v == current
        {
            // Um recorte não muda a imagem montada (só o que dela aparece).
            if let Op::Mark(m) = &op {
                draw_mark(image, m.kind, m.color, &m.points, m.width, Transform::identity());
            }
            *v = self.version;
        }
        self.ops.push(op);
    }

    /// Recorta na área `r` (pixels da imagem, presa à imagem); `false` se a área é
    /// vazia ou igual ao recorte atual (nada muda).
    pub fn crop_to(&mut self, r: RectF) -> bool {
        let x0 = r.0.round().clamp(0.0, self.width() as f32) as i32;
        let y0 = r.1.round().clamp(0.0, self.height() as f32) as i32;
        let x1 = (r.0 + r.2).round().clamp(0.0, self.width() as f32) as i32;
        let y1 = (r.1 + r.3).round().clamp(0.0, self.height() as f32) as i32;
        let Some(rect) = IntRect::from_ltrb(x0, y0, x1, y1) else {
            return false;
        };
        if rect == self.crop() {
            return false;
        }
        self.push(Op::Crop(rect));
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.ops.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(op) = self.ops.pop() else {
            return false;
        };
        // Desfazer um recorte não muda a imagem montada.
        let keeps = matches!(op, Op::Crop(_));
        self.undone.push(op);
        self.version += 1;
        if keeps && let Some((v, _)) = self.composed.as_mut()
            && *v + 1 == self.version
        {
            *v = self.version;
        }
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(op) = self.undone.pop() else {
            return false;
        };
        self.apply(op);
        true
    }

    /// A imagem inteira com as marcações (sem o recorte), guardada até a próxima mudança
    /// (uma marcação nova é desenhada por cima dela; desfazer uma marcação a refaz).
    pub fn composed(&mut self) -> &Pixmap {
        if self.composed.as_ref().is_none_or(|(v, _)| *v != self.version) {
            let mut out = self.base.clone();
            for op in &self.ops {
                if let Op::Mark(m) = op {
                    draw_mark(&mut out, m.kind, m.color, &m.points, m.width, Transform::identity());
                }
            }
            self.composed = Some((self.version, out));
        }
        &self.composed.as_ref().expect("acabou de ser montada").1
    }

    /// O resultado: as marcações e o recorte.
    pub fn result(&mut self) -> Pixmap {
        let crop = self.crop();
        let full = self.full();
        let composed = self.composed();
        if crop == full {
            return composed.clone();
        }
        composed.clone_rect(crop).unwrap_or_else(|| composed.clone())
    }
}

/// Lê um PNG ou JPEG (qualquer formato que o `image` do mide conhece), já
/// pré-multiplicado.
pub fn load(path: &Path) -> Result<Pixmap, String> {
    let image = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = image.dimensions();
    let mut data = image.into_raw();
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u16;
        if a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u16 * a + 127) / 255) as u8;
            }
        }
    }
    let size = tiny_skia::IntSize::from_wh(w, h).ok_or("imagem vazia")?;
    Pixmap::from_vec(data, size).ok_or_else(|| "imagem inválida".to_string())
}

/// A imagem codificada no formato do nome do arquivo: JPEG para `.jpg`/`.jpeg` (sem
/// transparência), PNG para o resto.
pub fn encode(image: &Pixmap, path: &Path) -> Result<Vec<u8>, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let mut out = std::io::Cursor::new(Vec::new());
    if ext == "jpg" || ext == "jpeg" {
        // Sem transparência: o que é transparente fica branco.
        let mut rgb = Vec::with_capacity((image.width() * image.height() * 3) as usize);
        for px in image.data().as_chunks::<4>().0 {
            let a = px[3] as u16;
            for c in &px[..3] {
                rgb.push((*c as u16 + (255 - a)).min(255) as u8);
            }
        }
        let buffer = image::RgbImage::from_raw(image.width(), image.height(), rgb).ok_or("imagem inválida")?;
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92).encode_image(&buffer).map_err(|e| e.to_string())?;
    } else {
        let png = image.encode_png().map_err(|e| e.to_string())?;
        return Ok(png);
    }
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, rgb: [u8; 3]) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255));
        p
    }

    fn rgb(p: &Pixmap, x: u32, y: u32) -> [u8; 3] {
        let c = p.pixel(x, y).unwrap();
        [c.red(), c.green(), c.blue()]
    }

    fn pen(points: &[(f32, f32)]) -> Op {
        Op::Mark(Mark { kind: MarkKind::Pen, color: [235, 52, 52], points: points.to_vec(), width: 4.0 })
    }

    #[test]
    fn recorte_e_desfazer() {
        let mut doc = Doc::new(image(400, 300, [255, 255, 255]));
        assert!(!doc.modified());
        assert!(doc.crop_to((50.4, 20.0, 200.0, 100.6)));
        assert_eq!(doc.crop(), IntRect::from_xywh(50, 20, 200, 101).unwrap());
        assert_eq!((doc.result().width(), doc.result().height()), (200, 101));
        // Recortar de novo na mesma área não vira outra edição.
        assert!(!doc.crop_to((50.0, 20.0, 200.0, 101.0)));
        assert!(doc.undo());
        assert_eq!((doc.result().width(), doc.result().height()), (400, 300));
        assert!(doc.redo());
        assert_eq!(doc.result().width(), 200);
        assert!(doc.modified());
    }

    #[test]
    fn recorte_fora_da_imagem_fica_preso_a_ela() {
        let mut doc = Doc::new(image(100, 80, [0, 0, 0]));
        assert!(doc.crop_to((-20.0, -10.0, 70.0, 200.0)));
        assert_eq!(doc.crop(), IntRect::from_xywh(0, 0, 50, 80).unwrap());
        assert!(!doc.crop_to((120.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn desenho_livre_muda_os_pixels_e_desfaz() {
        let mut doc = Doc::new(image(100, 100, [255, 255, 255]));
        doc.push(pen(&[(10.0, 50.0), (50.0, 50.0), (90.0, 50.0)]));
        let out = doc.result();
        let p = rgb(&out, 50, 50);
        assert!(p[0] > 200 && p[1] < 90 && p[2] < 90, "vermelho no traço: {p:?}");
        assert_eq!(rgb(&out, 50, 10), [255, 255, 255]);
        doc.undo();
        assert_eq!(rgb(&doc.result(), 50, 50), [255, 255, 255]);
    }

    #[test]
    fn marcacao_depois_do_recorte_fica_no_lugar_da_imagem() {
        let mut doc = Doc::new(image(200, 200, [255, 255, 255]));
        doc.crop_to((100.0, 100.0, 100.0, 100.0));
        // Pontos em pixels da imagem original: (150, 150) é o meio do recorte.
        doc.push(Op::Mark(Mark { kind: MarkKind::Square, color: [10, 132, 255], points: vec![(140.0, 140.0), (160.0, 160.0)], width: 4.0 }));
        let out = doc.result();
        assert_eq!((out.width(), out.height()), (100, 100));
        let p = rgb(&out, 40, 50);
        assert!(p[2] > 200 && p[0] < 60, "azul no lado do quadrado: {p:?}");
    }

    #[test]
    fn desfazer_ate_o_salvo_nao_e_mudanca() {
        let mut doc = Doc::new(image(50, 50, [255, 255, 255]));
        doc.push(pen(&[(1.0, 1.0)]));
        assert!(doc.modified());
        doc.undo();
        assert!(!doc.modified(), "igual ao arquivo aberto");
        doc.redo();
        doc.mark_saved();
        assert!(!doc.modified());
        doc.push(pen(&[(2.0, 2.0)]));
        doc.undo();
        assert!(!doc.modified(), "voltou ao salvo");
        // Desfazer o salvo e fazer outra edição: o salvo não volta mais.
        doc.undo();
        doc.push(pen(&[(3.0, 3.0)]));
        assert!(doc.modified());
        doc.undo();
        doc.push(pen(&[(4.0, 4.0)]));
        assert!(doc.modified(), "mesmo número de edições, mas outra imagem");
    }

    #[test]
    fn marcacao_por_cima_igual_a_recompor_tudo() {
        let mut doc = Doc::new(image(120, 90, [255, 255, 255]));
        doc.push(pen(&[(10.0, 10.0), (100.0, 80.0)]));
        let _ = doc.composed();
        // Desenhada só a nova, por cima da montada.
        doc.push(Op::Mark(Mark { kind: MarkKind::Circle, color: [10, 132, 255], points: vec![(20.0, 20.0), (90.0, 70.0)], width: 4.0 }));
        let fast = doc.composed().clone();
        let mut fresh = Doc::new(image(120, 90, [255, 255, 255]));
        fresh.ops = doc.ops.clone();
        assert_eq!(fast.data(), fresh.composed().data());
        doc.undo();
        assert_ne!(doc.composed().data(), fast.data());
        doc.redo();
        assert_eq!(doc.composed().data(), fast.data());
        // Um recorte não muda a imagem montada (nem a refaz).
        doc.crop_to((10.0, 10.0, 50.0, 50.0));
        assert_eq!(doc.composed().data(), fast.data());
        assert_eq!(doc.result().width(), 50);
    }

    #[test]
    fn nova_edicao_apaga_o_refazer() {
        let mut doc = Doc::new(image(50, 50, [255, 255, 255]));
        doc.push(pen(&[(1.0, 1.0)]));
        doc.undo();
        assert!(doc.can_redo());
        doc.push(pen(&[(2.0, 2.0)]));
        assert!(!doc.can_redo());
    }

    #[test]
    fn salva_png_e_jpeg_e_le_de_volta() {
        let dir = std::env::temp_dir().join(format!("mide-editor-doc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut img = image(30, 20, [10, 200, 30]);
        // Um pixel meio transparente (PNG guarda; JPEG vira opaco).
        img.pixels_mut()[0] = tiny_skia::ColorU8::from_rgba(100, 0, 0, 128).premultiply();
        for name in ["a.png", "b.jpg"] {
            let path = dir.join(name);
            std::fs::write(&path, encode(&img, &path).unwrap()).unwrap();
            let back = load(&path).unwrap();
            assert_eq!((back.width(), back.height()), (30, 20));
            let p = rgb(&back, 15, 10);
            assert!(p[1] > 180 && p[0] < 40, "{name}: {p:?}");
        }
        assert_eq!(load(&dir.join("a.png")).unwrap().pixel(0, 0).unwrap().alpha(), 128);
        assert!(load(&dir.join("nada.png")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
