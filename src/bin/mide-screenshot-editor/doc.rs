//! O documento do Editor de Capturas (como a Marcação do Mac): a imagem aberta, as
//! formas por cima dela (editáveis até salvar: escolher, mover, redimensionar, trocar a
//! cor) e o recorte, com desfazer e refazer do estado inteiro. Nada é aplicado na
//! imagem original: o resultado é montado ao salvar ou copiar.
//!
//! Coordenadas: tudo em pixels da imagem (já girada, se girou).

use std::{path::Path, rc::Rc};

use mide::{
    annotate::{self, Shape},
    markup::RectF,
};
use tiny_skia::{IntRect, Pixmap, Transform};

/// Um estado do documento (o desfazer guarda estados inteiros: as formas são poucas
/// e a imagem é compartilhada).
#[derive(Clone)]
pub struct State {
    pub base: Rc<Pixmap>,
    pub shapes: Vec<Shape>,
    pub crop: IntRect,
    /// Número do estado (muda a cada edição): diz se está como foi salvo.
    generation: u64,
}

pub struct Doc {
    pub state: State,
    history: Vec<State>,
    future: Vec<State>,
    next_generation: u64,
    saved: u64,
    /// Aumenta a cada mudança (também desfazer e refazer): chave de cache.
    pub version: u64,
}

impl Doc {
    pub fn new(base: Pixmap) -> Self {
        let crop = IntRect::from_xywh(0, 0, base.width(), base.height()).expect("imagem com tamanho");
        Self { state: State { base: Rc::new(base), shapes: Vec::new(), crop, generation: 0 }, history: Vec::new(), future: Vec::new(), next_generation: 1, saved: 0, version: 0 }
    }

    pub fn width(&self) -> u32 {
        self.state.base.width()
    }

    pub fn height(&self) -> u32 {
        self.state.base.height()
    }

    pub fn full(&self) -> IntRect {
        IntRect::from_xywh(0, 0, self.width(), self.height()).expect("imagem com tamanho")
    }

    pub fn crop(&self) -> IntRect {
        self.state.crop
    }

    /// Espessura padrão do traço, proporcional ao tamanho da imagem.
    pub fn stroke_width(&self) -> f32 {
        4.0 * (self.width().max(self.height()) as f32 / 1600.0).max(1.0)
    }

    pub fn modified(&self) -> bool {
        self.state.generation != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.state.generation;
    }

    /// Guarda o estado atual no desfazer, antes de uma edição (que muda o estado).
    pub fn checkpoint(&mut self) {
        self.history.push(self.state.clone());
        self.future.clear();
        self.state.generation = self.next_generation;
        self.next_generation += 1;
        self.version += 1;
    }

    /// Mudança no meio de um arrasto (mover, redimensionar): sem novo ponto de desfazer.
    pub fn touched(&mut self) {
        self.version += 1;
    }

    pub fn add(&mut self, shape: Shape) -> usize {
        self.checkpoint();
        self.state.shapes.push(shape);
        self.state.shapes.len() - 1
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.state.shapes.len() {
            self.checkpoint();
            self.state.shapes.remove(index);
        }
    }

    /// Recorta na área `r` (presa à imagem); `false` se nada muda.
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
        self.checkpoint();
        self.state.crop = rect;
        true
    }

    /// Gira a imagem (e as formas e o recorte) 90°.
    pub fn rotate(&mut self, clockwise: bool) {
        self.checkpoint();
        let (w, h) = (self.width(), self.height());
        let Some(mut out) = Pixmap::new(h, w) else {
            return;
        };
        let tf = if clockwise { Transform::from_row(0.0, 1.0, -1.0, 0.0, h as f32, 0.0) } else { Transform::from_row(0.0, -1.0, 1.0, 0.0, 0.0, w as f32) };
        out.draw_pixmap(0, 0, self.state.base.as_ref().as_ref(), &tiny_skia::PixmapPaint::default(), tf, None);
        for shape in &mut self.state.shapes {
            shape.rotate(clockwise, w as f32, h as f32);
        }
        let c = self.state.crop;
        let (cx, cy, cw, ch) = (c.x() as f32, c.y() as f32, c.width() as f32, c.height() as f32);
        let r = if clockwise { (h as f32 - cy - ch, cx, ch, cw) } else { (cy, w as f32 - cx - cw, ch, cw) };
        self.state.base = Rc::new(out);
        self.state.crop = IntRect::from_xywh(r.0.round() as i32, r.1.round() as i32, r.2.round() as u32, r.3.round() as u32).unwrap_or_else(|| self.full());
    }

    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.history.pop() else {
            return false;
        };
        self.future.push(std::mem::replace(&mut self.state, previous));
        self.version += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.future.pop() else {
            return false;
        };
        self.history.push(std::mem::replace(&mut self.state, next));
        self.version += 1;
        true
    }

    /// O resultado: a imagem com as formas e o recorte.
    pub fn result(&self, text: &mut mide::ui::text::Text) -> Pixmap {
        let mut out = self.state.base.as_ref().clone();
        for shape in &self.state.shapes {
            annotate::draw(&mut out, shape, Transform::identity(), text);
        }
        if self.state.crop == self.full() {
            return out;
        }
        out.clone_rect(self.state.crop).unwrap_or(out)
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
    use mide::annotate::ShapeKind;

    fn image(w: u32, h: u32, rgb: [u8; 3]) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255));
        p
    }

    #[test]
    fn formas_desfazer_refazer_e_salvo() {
        let mut doc = Doc::new(image(100, 80, [255, 255, 255]));
        assert!(!doc.modified());
        let i = doc.add(Shape::new(ShapeKind::Rect, vec![(10.0, 10.0), (50.0, 40.0)], [255, 0, 0], 4.0));
        assert_eq!(i, 0);
        assert!(doc.modified());
        doc.mark_saved();
        assert!(!doc.modified());
        assert!(doc.crop_to((10.0, 10.0, 50.0, 40.0)));
        assert_eq!(doc.crop().width(), 50);
        assert!(doc.undo());
        assert_eq!(doc.crop().width(), 100);
        assert!(!doc.modified(), "voltou ao estado salvo");
        assert!(doc.redo());
        assert_eq!(doc.crop().width(), 50);
    }

    #[test]
    fn resultado_tem_a_forma_e_o_recorte() {
        let mut doc = Doc::new(image(100, 80, [255, 255, 255]));
        let mut s = Shape::new(ShapeKind::Rect, vec![(0.0, 0.0), (100.0, 80.0)], [255, 0, 0], 4.0);
        s.fill = Some([0, 0, 255]);
        doc.add(s);
        doc.crop_to((20.0, 20.0, 30.0, 30.0));
        let mut text = mide::ui::text::Text::new();
        let out = doc.result(&mut text);
        assert_eq!((out.width(), out.height()), (30, 30));
        let px = out.pixel(15, 15).unwrap();
        assert_eq!((px.red(), px.green(), px.blue()), (0, 0, 255));
    }

    #[test]
    fn girar_leva_formas_e_recorte_junto() {
        let mut doc = Doc::new(image(100, 50, [255, 255, 255]));
        doc.add(Shape::new(ShapeKind::Line, vec![(0.0, 0.0), (100.0, 0.0)], [0, 0, 0], 2.0));
        doc.rotate(true);
        assert_eq!((doc.width(), doc.height()), (50, 100));
        assert_eq!(doc.state.shapes[0].points[1], (50.0, 100.0));
    }

    #[test]
    fn salva_png_e_jpeg_e_le_de_volta() {
        let dir = std::env::temp_dir().join(format!("mide-editor-doc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = image(20, 10, [200, 100, 50]);
        for name in ["a.png", "a.jpg"] {
            let path = dir.join(name);
            std::fs::write(&path, encode(&img, &path).unwrap()).unwrap();
            let back = load(&path).unwrap();
            assert_eq!((back.width(), back.height()), (20, 10));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
