//! Terminal integrado do Code (painel embaixo do editor, Ctrl+`), com o emulador do
//! mide (`mide::terminal`, o mesmo do `mide-terminal`): o shell do usuário roda num
//! pseudoterminal na pasta do projeto, e o painel desenha a grade de células com as
//! cores do tema claro ou escuro do Terminal.
//!
//! Versão simples: um shell, rolagem do histórico pela roda, colar com Ctrl+Shift+V.
//! Abas, seleção com o mouse e imagens ficam com o `mide-terminal`.

use std::path::PathBuf;
use std::sync::mpsc;

use mide::kit::{KeyPress, Ui};
use mide::terminal::{
    keys::{key_bytes, paste_bytes},
    palette,
    pty::{Pty, Size, Spawn},
    screen::{attr, CursorShape, Wide},
    Terminal,
};

/// Margem interna do painel.
const PAD: f32 = 6.0;

pub struct TermPanel {
    term: Terminal,
    pty: Pty,
    output: mpsc::Receiver<Vec<u8>>,
    /// Tamanho de uma célula (medido ao desenhar).
    cell: (f32, f32),
    /// Linhas roladas para cima no histórico (0 = no fim).
    scroll: usize,
    /// O shell terminou (o painel mostra o aviso e fecha no próximo Ctrl+`).
    pub ended: bool,
    pub dir: PathBuf,
}

impl TermPanel {
    /// Abre o shell do usuário em `dir`. `wake` acorda a janela quando chega saída.
    pub fn open(dir: PathBuf, wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (tx, output) = mpsc::channel();
        let mut spawn = Spawn { command: None, directory: Some(dir.clone()), env: Vec::new() };
        // Nos testes, um shell simples e sem a configuração de quem roda.
        if let Some(shell) = std::env::var_os("MIDE_CODE_TEST_SHELL") {
            spawn.command = Some((shell.to_string_lossy().into_owned(), Vec::new()));
        }
        let (cols, rows) = (80, 12);
        let pty = Pty::spawn(&spawn, Size { cols, rows, width: 0, height: 0 }, tx, wake)?;
        let term = Terminal::new(cols as usize, rows as usize);
        tracing::info!("terminal integrado: {}", dir.display());
        Ok(Self { term, pty, output, cell: (8.0, 17.0), scroll: 0, ended: false, dir })
    }

    /// Lê a saída nova do programa. Devolve se chegou algo.
    pub fn read(&mut self) -> bool {
        let mut got = false;
        while let Ok(chunk) = self.output.try_recv() {
            if chunk.is_empty() {
                self.ended = true;
                self.pty.reap();
                break;
            }
            got = true;
            self.term.feed(&chunk);
        }
        let responses = self.term.take_responses();
        if !responses.is_empty() {
            self.pty.write(responses);
        }
        if got {
            self.scroll = 0;
        }
        got
    }

    /// Teclas para o programa (o painel tem o foco).
    pub fn keys(&mut self, presses: &[KeyPress], paste: Option<&str>) {
        for press in presses {
            if let Some(bytes) = key_bytes(press, &self.term.modes) {
                self.scroll = 0;
                self.pty.write(bytes);
            }
        }
        if let Some(text) = paste {
            self.pty.write(paste_bytes(text, self.term.modes.bracketed_paste));
        }
    }

    /// Texto da tela (para os testes e o log).
    pub fn screen_text(&self) -> String {
        (0..self.term.rows()).map(|y| self.term.visible_row(y, 0).cells.iter().map(|c| c.c).collect::<String>().trim_end().to_string()).collect::<Vec<_>>().join("\n")
    }

    /// Desenha o painel em `rect` (na janela) e ajusta o tamanho da grade a ele.
    pub fn draw(&mut self, ui: &mut Ui, rect: (f32, f32, f32, f32), size: f32, dark: bool, focused: bool) {
        let theme = palette::theme(if dark { "Escuro" } else { "Claro" });
        if self.term.theme.name != theme.name {
            self.term.theme = theme;
        }
        // Roda do mouse sobre o painel: histórico.
        if ui.input().over(rect) {
            let s = ui.take_scroll();
            if s != 0.0 {
                let lines = (s / self.cell.1).round() as isize;
                self.scroll = (self.scroll as isize - lines).clamp(0, self.term.history_len() as isize) as usize;
            }
        }
        // Grade do tamanho do painel (avisa o programa quando muda).
        let cols = (((rect.2 - 2.0 * PAD) / self.cell.0).floor() as usize).max(10);
        let rows = (((rect.3 - 2.0 * PAD) / self.cell.1).floor() as usize).max(2);
        if (cols, rows) != (self.term.cols(), self.term.rows()) {
            self.term.resize(cols, rows);
            self.pty.resize(Size { cols: cols as u16, rows: rows as u16, width: (cols as f32 * self.cell.0) as u16, height: (rows as f32 * self.cell.1) as u16 });
        }
        let term = &self.term;
        let scroll = self.scroll.min(term.history_len());
        let mut measured = self.cell;
        let ended = self.ended;
        ui.paint_text(|pixmap, text, (ox, oy)| {
            let cell = text.mono_cell(size);
            measured = cell;
            let (x0, y0) = (ox + rect.0, oy + rect.1);
            let bg0 = term.default_bg();
            mide::ui::draw::paint_rounded_solid(pixmap, x0, y0, rect.2, rect.3, 0.0, [bg0[0], bg0[1], bg0[2], 255]);
            let fill = |pixmap: &mut tiny_skia::Pixmap, x: f32, y: f32, w: f32, h: f32, c: [u8; 3]| mide::ui::draw::fill_rounded(pixmap, x, y, w, h, 0.0, mide::ui::draw::rgba(c[0], c[1], c[2], 255));
            let (cursor_row, cursor_col) = term.cursor();
            let show_cursor = term.modes.show_cursor && scroll == 0;
            let (px0, py0) = (x0 + PAD, y0 + PAD);
            for y in 0..term.rows() {
                let row = term.visible_row(y, scroll);
                let py = py0 + y as f32 * cell.1;
                if py + cell.1 > y0 + rect.3 {
                    break;
                }
                for (x, c) in row.cells.iter().enumerate() {
                    if c.wide == Wide::Spacer {
                        continue;
                    }
                    let px = px0 + x as f32 * cell.0;
                    if px + cell.0 > x0 + rect.2 {
                        break;
                    }
                    let width = if c.wide == Wide::Lead { 2.0 } else { 1.0 } * cell.0;
                    let (mut fg, mut bg) = term.cell_colors(&c.pen);
                    let at_cursor = show_cursor && y == cursor_row && x == cursor_col;
                    if at_cursor && focused && term.cursor_shape == CursorShape::Block {
                        bg = term.cursor_color();
                        fg = bg0;
                    }
                    if bg != bg0 {
                        fill(pixmap, px, py, width, cell.1, bg);
                    }
                    if c.c != ' ' && c.pen.attr & attr::HIDDEN == 0 {
                        let s = if c.combining.is_some() { c.text() } else { c.c.to_string() };
                        text.draw_cell(pixmap, &s, (px, py), cell, width, size, [fg[0], fg[1], fg[2], 255], c.pen.attr & attr::BOLD != 0, c.pen.attr & attr::ITALIC != 0);
                    }
                    if at_cursor {
                        let color = term.cursor_color();
                        match (focused, term.cursor_shape) {
                            // Sem foco: só o contorno do cursor.
                            (false, _) => {
                                fill(pixmap, px, py, width, 1.0, color);
                                fill(pixmap, px, py + cell.1 - 1.0, width, 1.0, color);
                                fill(pixmap, px, py, 1.0, cell.1, color);
                                fill(pixmap, px + width - 1.0, py, 1.0, cell.1, color);
                            }
                            (true, CursorShape::Bar) => fill(pixmap, px, py, 2.0, cell.1, color),
                            (true, CursorShape::Underline) => fill(pixmap, px, py + cell.1 - 2.0, width, 2.0, color),
                            (true, CursorShape::Block) => {}
                        }
                    }
                }
            }
            if ended {
                let fg = term.default_fg();
                text.draw(pixmap, "O shell terminou. Ctrl+` fecha o painel.", x0 + rect.2 - 300.0, y0 + rect.3 - 22.0, 12.0, mide::ui::text::FontWeight::Regular, [fg[0], fg[1], fg[2], 160]);
            }
        });
        self.cell = measured;
    }
}
