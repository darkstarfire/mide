//! Estado do terminal: a grade de células (principal e alternativa), o histórico, o
//! cursor, os modos e as cores. Recebe o que o [`Parser`] decodifica e aplica como
//! um xterm aplicaria.
//!
//! Como no iTerm2, o histórico guarda as linhas lógicas com a marca de "continua na
//! próxima" (quebra automática), e mudar a largura da janela reorganiza o texto
//! (reflow) em vez de cortar as linhas. Cada linha tem um número absoluto que não
//! muda enquanto ela existe (`dropped` + posição): seleção e marcas de prompt usam
//! esse número.

use std::collections::VecDeque;

use super::{
    palette,
    parser::{Parser, Perform},
    width::char_width,
};

pub type Rgb = [u8; 3];

/// Cor de uma célula: a padrão do tema, uma da paleta de 256 ou RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Atributos de texto (combináveis).
pub mod attr {
    pub const BOLD: u16 = 1;
    pub const DIM: u16 = 1 << 1;
    pub const ITALIC: u16 = 1 << 2;
    pub const UNDERLINE: u16 = 1 << 3;
    pub const BLINK: u16 = 1 << 4;
    pub const INVERSE: u16 = 1 << 5;
    pub const HIDDEN: u16 = 1 << 6;
    pub const STRIKE: u16 = 1 << 7;
    pub const DOUBLE_UNDERLINE: u16 = 1 << 8;
}

/// Cores e atributos com que o texto é escrito.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pen {
    pub fg: Color,
    pub bg: Color,
    pub attr: u16,
}

/// Caractere largo (ocupa duas células): a primeira metade e a segunda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Wide {
    #[default]
    No,
    Lead,
    Spacer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub pen: Pen,
    pub wide: Wide,
    /// Acentos e modificadores combinados com o caractere.
    pub combining: Option<Box<str>>,
}

impl Default for Cell {
    fn default() -> Self {
        Self { c: ' ', pen: Pen::default(), wide: Wide::No, combining: None }
    }
}

impl Cell {
    fn blank(bg: Color) -> Self {
        Self { pen: Pen { bg, ..Pen::default() }, ..Self::default() }
    }

    /// Célula vazia, sem fundo nem atributos.
    pub fn is_blank(&self) -> bool {
        self.c == ' ' && self.pen == Pen::default() && self.combining.is_none()
    }

    /// O texto da célula (o caractere e o que estiver combinado com ele).
    pub fn text(&self) -> String {
        let mut s = String::from(self.c);
        if let Some(extra) = &self.combining {
            s.push_str(extra);
        }
        s
    }
}

#[derive(Debug, Clone, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// O texto continua na linha de baixo (quebra automática, não Enter).
    pub wrapped: bool,
}

impl Row {
    fn new(cols: usize, bg: Color) -> Self {
        Self { cells: vec![Cell::blank(bg); cols], wrapped: false }
    }

    fn is_blank(&self) -> bool {
        !self.wrapped && self.cells.iter().all(Cell::is_blank)
    }

    fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::default());
        if let Some(last) = self.cells.last_mut() {
            if last.wide == Wide::Lead {
                *last = Cell::default();
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Charset {
    #[default]
    Ascii,
    /// Desenho de linhas do DEC (`ESC ( 0`), usado por programas antigos de caixas.
    DecGraphics,
}

#[derive(Debug, Clone, Copy, Default)]
struct SavedCursor {
    row: usize,
    col: usize,
    pen: Pen,
    origin: bool,
    pending_wrap: bool,
    charsets: [Charset; 2],
    gl: usize,
}

/// Quais eventos do mouse o programa quer receber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    #[default]
    Off,
    /// 9: só o clique.
    X10,
    /// 1000: apertar e soltar.
    Press,
    /// 1002: e arrastar com o botão apertado.
    Drag,
    /// 1003: e qualquer movimento.
    Motion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// Movimento sem botão apertado.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEvent {
    Press,
    Release,
    Motion,
}

#[derive(Debug, Clone, Copy)]
pub struct Modes {
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub autowrap: bool,
    pub insert: bool,
    pub origin: bool,
    pub show_cursor: bool,
    pub cursor_blink: bool,
    pub bracketed_paste: bool,
    pub focus_events: bool,
    pub mouse: MouseMode,
    pub mouse_sgr: bool,
    /// LNM: LF também volta ao começo da linha.
    pub newline: bool,
    pub reverse_video: bool,
    /// 2026: o programa está desenhando um quadro (a janela pode esperar).
    pub synchronized: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            app_cursor: false,
            app_keypad: false,
            autowrap: true,
            insert: false,
            origin: false,
            show_cursor: true,
            cursor_blink: false,
            bracketed_paste: false,
            focus_events: false,
            mouse: MouseMode::Off,
            mouse_sgr: false,
            newline: false,
            reverse_video: false,
            synchronized: false,
        }
    }
}

/// Um prompt na tela ou no histórico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptMark {
    /// Linha absoluta do prompt.
    pub line: u64,
    /// Código de saída do comando digitado nele (`None`: nenhum, ou ainda rodando).
    pub status: Option<i32>,
}

/// Um comando que terminou: a linha do prompt dele, o código e quanto levou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finished {
    pub line: u64,
    pub status: i32,
    pub duration: std::time::Duration,
}

/// Linhas guardadas no histórico, por padrão.
pub const HISTORY: usize = 10_000;

pub struct Terminal {
    cols: usize,
    rows: usize,
    primary: Vec<Row>,
    alternate: Vec<Row>,
    alt: bool,
    history: VecDeque<Row>,
    history_limit: usize,
    /// Linhas que já saíram do histórico (o número absoluto da primeira guardada).
    dropped: u64,
    row: usize,
    col: usize,
    pen: Pen,
    /// O cursor está depois da última coluna: o próximo caractere quebra a linha.
    pending_wrap: bool,
    charsets: [Charset; 2],
    gl: usize,
    saved_primary: Option<SavedCursor>,
    saved_alt: Option<SavedCursor>,
    /// Região de rolagem (primeira e última linha, inclusive).
    top: usize,
    bottom: usize,
    tabs: Vec<bool>,
    pub modes: Modes,
    pub cursor_shape: CursorShape,
    /// Título pedido pelo programa (OSC 0/2).
    pub title: Option<String>,
    title_stack: Vec<Option<String>>,
    /// Cores trocadas pelo programa (OSC 4, 10, 11, 12).
    palette: Box<[Option<Rgb>; 256]>,
    fg_override: Option<Rgb>,
    bg_override: Option<Rgb>,
    cursor_override: Option<Rgb>,
    /// Respostas ao programa (relatórios de posição, de cor...), a mandar pelo PTY.
    responses: Vec<u8>,
    /// Texto que o programa pediu para copiar (OSC 52).
    clipboard: Option<String>,
    /// O programa tocou o sino desde a última consulta.
    pub bell: bool,
    /// Pasta atual informada pelo shell (OSC 7).
    pub cwd: Option<String>,
    /// Prompts (OSC 133;A), em ordem, com o resultado do comando digitado em cada um.
    marks: Vec<PromptMark>,
    /// Um comando está rodando desde (OSC 133;C).
    command_started: Option<std::time::Instant>,
    /// Comandos que terminaram desde a última consulta (OSC 133;D).
    finished: Vec<Finished>,
    last_char: Option<char>,
    /// Tamanho da célula em pixels (para os relatórios de tamanho).
    pub cell_pixels: (u16, u16),
    /// Cores do tema (o programa pode trocar algumas, ver `palette` e os overrides).
    pub theme: palette::Theme,
    /// Negrito com as 8 cores básicas usa a versão clara.
    pub bold_bright: bool,
    parser: Parser,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize) -> Self {
        let (cols, rows) = (cols.max(2), rows.max(1));
        Self {
            cols,
            rows,
            primary: vec![Row::new(cols, Color::Default); rows],
            alternate: vec![Row::new(cols, Color::Default); rows],
            alt: false,
            history: VecDeque::new(),
            history_limit: HISTORY,
            dropped: 0,
            row: 0,
            col: 0,
            pen: Pen::default(),
            pending_wrap: false,
            charsets: [Charset::Ascii; 2],
            gl: 0,
            saved_primary: None,
            saved_alt: None,
            top: 0,
            bottom: rows - 1,
            tabs: default_tabs(cols),
            modes: Modes::default(),
            cursor_shape: CursorShape::Block,
            title: None,
            title_stack: Vec::new(),
            palette: Box::new([None; 256]),
            fg_override: None,
            bg_override: None,
            cursor_override: None,
            responses: Vec::new(),
            clipboard: None,
            bell: false,
            cwd: None,
            marks: Vec::new(),
            command_started: None,
            finished: Vec::new(),
            last_char: None,
            cell_pixels: (8, 16),
            theme: palette::Theme::default(),
            bold_bright: true,
            parser: Parser::new(),
        }
    }

    /// Processa a saída do programa.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut parser = std::mem::take(&mut self.parser);
        parser.advance(self, bytes);
        self.parser = parser;
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Respostas pendentes para o programa.
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.responses)
    }

    /// Texto que o programa pediu para pôr na área de transferência.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    /// A tela alternativa (de programas de tela cheia, como vim e less) está ativa.
    pub fn alt_screen(&self) -> bool {
        self.alt
    }

    /// Linhas no histórico (0 na tela alternativa, que não tem histórico).
    pub fn history_len(&self) -> usize {
        if self.alt { 0 } else { self.history.len() }
    }

    /// Cursor (linha e coluna na tela).
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    /// Linha `y` da tela vista com o histórico rolado `scroll` linhas para cima.
    pub fn visible_row(&self, y: usize, scroll: usize) -> &Row {
        let hist = self.history_len();
        self.line(hist - scroll.min(hist) + y)
    }

    /// Número absoluto da linha `y` da tela rolada `scroll` linhas.
    pub fn visible_abs(&self, y: usize, scroll: usize) -> u64 {
        let hist = self.history_len();
        self.dropped + (hist - scroll.min(hist) + y) as u64
    }

    /// Linha pelo índice no histórico + tela.
    fn line(&self, i: usize) -> &Row {
        let hist = self.history_len();
        if i < hist { &self.history[i] } else { &self.grid()[(i - hist).min(self.rows - 1)] }
    }

    /// Linha pelo número absoluto, se ainda existe.
    fn line_abs(&self, abs: u64) -> Option<&Row> {
        let i = abs.checked_sub(self.dropped)? as usize;
        (i < self.history_len() + self.rows).then(|| self.line(i))
    }

    fn grid(&self) -> &Vec<Row> {
        if self.alt { &self.alternate } else { &self.primary }
    }

    fn grid_mut(&mut self) -> &mut Vec<Row> {
        if self.alt { &mut self.alternate } else { &mut self.primary }
    }

    fn cell_mut(&mut self, row: usize, col: usize) -> &mut Cell {
        &mut self.grid_mut()[row].cells[col]
    }

    // ---- Cores ----

    /// Cor de fundo, de texto e do cursor do tema (ou as trocadas pelo programa).
    pub fn default_fg(&self) -> Rgb {
        self.fg_override.unwrap_or(self.theme.foreground)
    }

    pub fn default_bg(&self) -> Rgb {
        self.bg_override.unwrap_or(self.theme.background)
    }

    pub fn cursor_color(&self) -> Rgb {
        self.cursor_override.unwrap_or(self.theme.cursor)
    }

    /// RGB de uma cor da paleta.
    pub fn indexed(&self, i: u8) -> Rgb {
        self.palette[i as usize].unwrap_or_else(|| palette::default_color(&self.theme, i))
    }

    /// Cores finais de uma célula (texto e fundo), com negrito claro, esmaecido,
    /// invertido e o vídeo reverso aplicados.
    pub fn cell_colors(&self, pen: &Pen) -> (Rgb, Rgb) {
        let bold = pen.attr & attr::BOLD != 0;
        let mut fg = match pen.fg {
            Color::Default => self.default_fg(),
            Color::Indexed(i) if bold && self.bold_bright && i < 8 => self.indexed(i + 8),
            Color::Indexed(i) => self.indexed(i),
            Color::Rgb(r, g, b) => [r, g, b],
        };
        let mut bg = match pen.bg {
            Color::Default => self.default_bg(),
            Color::Indexed(i) => self.indexed(i),
            Color::Rgb(r, g, b) => [r, g, b],
        };
        if pen.attr & attr::DIM != 0 {
            for k in 0..3 {
                fg[k] = ((fg[k] as u32 * 2 + bg[k] as u32) / 3) as u8;
            }
        }
        if (pen.attr & attr::INVERSE != 0) != self.modes.reverse_video {
            std::mem::swap(&mut fg, &mut bg);
        }
        (fg, bg)
    }

    // ---- Mudança de tamanho ----

    /// Muda o tamanho da tela. Na tela principal o texto é reorganizado para a nova
    /// largura (linhas quebradas automaticamente se juntam e quebram de novo).
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        if cols == self.cols && rows == self.rows {
            return;
        }
        // Cursor da tela principal: o atual, ou o guardado ao entrar na alternativa.
        let primary_cursor = if self.alt {
            self.saved_primary.map_or((self.rows - 1, 0), |s| (s.row, s.col))
        } else {
            (self.row, self.col)
        };
        let (row, col) = self.reflow(cols, rows, primary_cursor);
        if self.alt {
            if let Some(saved) = &mut self.saved_primary {
                saved.row = row;
                saved.col = col;
            }
            for line in &mut self.alternate {
                line.resize(cols);
            }
            self.alternate.resize(rows, Row::new(cols, Color::Default));
            self.row = self.row.min(rows - 1);
            self.col = self.col.min(cols - 1);
        } else {
            self.row = row;
            self.col = col;
        }
        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bottom = rows - 1;
        self.tabs = default_tabs(cols);
        self.pending_wrap = false;
        // As marcas apontam para linhas que mudaram de número.
        self.marks.clear();
    }

    /// Reorganiza histórico + tela principal para `cols` x `rows`. Devolve o cursor.
    fn reflow(&mut self, cols: usize, rows: usize, cursor: (usize, usize)) -> (usize, usize) {
        let hist = self.history.len();
        let mut all: Vec<Row> = self.history.drain(..).chain(self.primary.drain(..)).collect();
        let cursor_line = hist + cursor.0;
        // Linhas vazias abaixo do cursor são só tela sem uso.
        while all.len() > cursor_line + 1 && all.last().is_some_and(Row::is_blank) {
            all.pop();
        }

        // Linhas lógicas: as quebradas automaticamente voltam a ser uma só.
        let mut logical: Vec<(Vec<Cell>, Option<usize>)> = Vec::new();
        let mut current: Vec<Cell> = Vec::new();
        let mut cursor_at: Option<usize> = None;
        for (i, row) in all.into_iter().enumerate() {
            if i == cursor_line {
                cursor_at = Some(current.len() + cursor.1);
            }
            let wrapped = row.wrapped;
            let mut cells = row.cells;
            if !wrapped {
                while cells.last().is_some_and(Cell::is_blank) {
                    cells.pop();
                }
            }
            current.extend(cells);
            if !wrapped {
                logical.push((std::mem::take(&mut current), cursor_at.take()));
            }
        }
        if !current.is_empty() || cursor_at.is_some() {
            logical.push((current, cursor_at));
        }

        // Quebra de novo na largura nova.
        let mut out: Vec<Row> = Vec::new();
        let mut new_cursor = (0, 0);
        for (cells, cursor_at) in logical {
            let mut row: Vec<Cell> = Vec::with_capacity(cols);
            for (i, cell) in cells.iter().enumerate() {
                if cell.wide == Wide::Spacer {
                    if cursor_at == Some(i) {
                        new_cursor = (out.len(), row.len().min(cols - 1));
                    }
                    continue;
                }
                let width = if cell.wide == Wide::Lead { 2 } else { 1 };
                if row.len() + width > cols {
                    row.resize(cols, Cell::default());
                    out.push(Row { cells: std::mem::take(&mut row), wrapped: true });
                }
                if cursor_at == Some(i) {
                    new_cursor = (out.len(), row.len());
                }
                row.push(cell.clone());
                if width == 2 {
                    row.push(Cell { c: ' ', pen: cell.pen, wide: Wide::Spacer, combining: None });
                }
            }
            if let Some(at) = cursor_at.filter(|&at| at >= cells.len()) {
                // Cursor depois do fim do texto (espaços não guardados).
                let mut col = row.len() + (at - cells.len());
                while col >= cols {
                    row.resize(cols, Cell::default());
                    out.push(Row { cells: std::mem::take(&mut row), wrapped: true });
                    col -= cols;
                }
                new_cursor = (out.len(), col);
            }
            row.resize(cols, Cell::default());
            out.push(Row { cells: row, wrapped: false });
        }

        // A tela mostra as últimas linhas, com o cursor dentro dela.
        let start = out.len().saturating_sub(rows).min(new_cursor.0);
        let mut grid = out.split_off(start);
        grid.truncate(rows);
        grid.resize(rows, Row::new(cols, Color::Default));
        self.primary = grid;
        for row in out {
            self.push_history(row);
        }
        (new_cursor.0 - start, new_cursor.1.min(cols - 1))
    }

    /// Quantas linhas o histórico guarda (as mais antigas saem).
    pub fn set_history_limit(&mut self, limit: usize) {
        self.history_limit = limit;
        while self.history.len() > self.history_limit {
            self.history.pop_front();
            self.dropped += 1;
        }
    }

    fn push_history(&mut self, row: Row) {
        self.history.push_back(row);
        while self.history.len() > self.history_limit {
            self.history.pop_front();
            self.dropped += 1;
        }
        let dropped = self.dropped;
        self.marks.retain(|m| m.line >= dropped);
    }

    // ---- Texto, seleção e marcas ----

    /// Texto entre dois pontos (número absoluto da linha, coluna), inclusive. Linhas
    /// quebradas automaticamente se juntam sem quebra de linha.
    pub fn text_between(&self, start: (u64, usize), end: (u64, usize)) -> String {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let mut out = String::new();
        for abs in start.0..=end.0 {
            let Some(row) = self.line_abs(abs) else {
                continue;
            };
            let from = if abs == start.0 { start.1 } else { 0 };
            let to = if abs == end.0 { end.1.min(row.cells.len().saturating_sub(1)) } else { row.cells.len().saturating_sub(1) };
            let mut line = String::new();
            for cell in row.cells.iter().take(to + 1).skip(from) {
                if cell.wide != Wide::Spacer {
                    line.push_str(&cell.text());
                }
            }
            if abs != end.0 && row.wrapped {
                out.push_str(&line);
            } else {
                out.push_str(line.trim_end_matches(' '));
                if abs != end.0 {
                    out.push('\n');
                }
            }
        }
        out
    }

    /// Ocorrências de `query` no histórico e na tela, em ordem (começo e fim de cada
    /// uma). Sem diferenciar maiúsculas nem acentos; atravessa as quebras automáticas.
    pub fn search(&self, query: &str) -> Vec<((u64, usize), (u64, usize))> {
        let fold_char = |c: char| crate::apps::fold(&c.to_string()).chars().next().unwrap_or(c);
        let needle: Vec<char> = query.chars().map(fold_char).collect();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut found = Vec::new();
        let total = self.history_len() + self.rows;
        let mut i = 0;
        while i < total && found.len() < 10_000 {
            // Uma linha lógica: as linhas seguidas por quebra automática.
            let mut chars: Vec<char> = Vec::new();
            let mut cells: Vec<(u64, usize)> = Vec::new();
            loop {
                let row = self.line(i);
                let abs = self.dropped + i as u64;
                for (col, cell) in row.cells.iter().enumerate() {
                    if cell.wide != Wide::Spacer {
                        chars.push(fold_char(cell.c));
                        cells.push((abs, col));
                    }
                }
                i += 1;
                if !row.wrapped || i >= total {
                    break;
                }
            }
            let mut start = 0;
            while start + needle.len() <= chars.len() {
                if chars[start..start + needle.len()] == needle[..] {
                    let last = cells[start + needle.len() - 1];
                    let end_col = if self.line_abs(last.0).is_some_and(|r| r.cells[last.1].wide == Wide::Lead) { last.1 + 1 } else { last.1 };
                    found.push((cells[start], (last.0, end_col)));
                    start += needle.len();
                } else {
                    start += 1;
                }
            }
        }
        found
    }

    /// Começo e fim da palavra em (linha absoluta, coluna), para o duplo clique. A
    /// palavra continua nas linhas quebradas automaticamente.
    pub fn word_at(&self, abs: u64, col: usize) -> Option<((u64, usize), (u64, usize))> {
        let cols = self.line_abs(abs)?.cells.len();
        let col = col.min(cols.checked_sub(1)?);
        let cell = |p: (u64, usize)| self.line_abs(p.0).and_then(|r| r.cells.get(p.1));
        let is_word = |p: (u64, usize)| {
            cell(p).is_some_and(|c| c.wide == Wide::Spacer || c.c.is_alphanumeric() || "-_./~:@%+#=?&".contains(c.c))
        };
        // Vizinhos na ordem do texto, atravessando as quebras automáticas.
        let before = |p: (u64, usize)| -> Option<(u64, usize)> {
            if p.1 > 0 {
                Some((p.0, p.1 - 1))
            } else {
                let prev = p.0.checked_sub(1)?;
                self.line_abs(prev).filter(|r| r.wrapped).map(|r| (prev, r.cells.len() - 1))
            }
        };
        let after = |p: (u64, usize)| -> Option<(u64, usize)> {
            let row = self.line_abs(p.0)?;
            if p.1 + 1 < row.cells.len() {
                Some((p.0, p.1 + 1))
            } else if row.wrapped {
                self.line_abs(p.0 + 1).map(|_| (p.0 + 1, 0))
            } else {
                None
            }
        };
        let mut at = (abs, col);
        if cell(at).is_some_and(|c| c.wide == Wide::Spacer) {
            at = before(at).unwrap_or(at);
        }
        if !is_word(at) {
            return Some((at, at));
        }
        let mut first = at;
        while let Some(p) = before(first).filter(|&p| is_word(p)) {
            first = p;
        }
        let mut last = at;
        while let Some(p) = after(last).filter(|&p| is_word(p)) {
            last = p;
        }
        Some((first, last))
    }

    /// Endereço (`https://`, `http://`, `file://`, `ftp://`, `mailto:`) sob a célula,
    /// com o começo e o fim. Atravessa as quebras automáticas e deixa de fora a
    /// pontuação do fim de frase.
    pub fn url_at(&self, abs: u64, col: usize) -> Option<((u64, usize), (u64, usize), String)> {
        let ((first, _), (last, _)) = self.line_bounds(abs)?;
        let mut chars: Vec<char> = Vec::new();
        let mut cells: Vec<(u64, usize)> = Vec::new();
        for line in first..=last {
            let row = self.line_abs(line)?;
            for (c, cell) in row.cells.iter().enumerate() {
                if cell.wide != Wide::Spacer {
                    chars.push(cell.c);
                    cells.push((line, c));
                }
            }
        }
        let at = cells.iter().position(|&p| p == (abs, col)).or_else(|| cells.iter().position(|&p| p == (abs, col.saturating_sub(1))))?;
        const SCHEMES: [&str; 5] = ["https://", "http://", "file://", "ftp://", "mailto:"];
        let stop = |c: char| c.is_whitespace() || c.is_control() || "\"'<>`{}|\\^".contains(c);
        let mut start = 0;
        while start < chars.len() {
            let rest: String = chars[start..].iter().collect();
            if let Some(scheme) = SCHEMES.iter().find(|s| rest.starts_with(**s)) {
                let mut end = start + scheme.chars().count();
                while end < chars.len() && !stop(chars[end]) {
                    end += 1;
                }
                let mut url: String = chars[start..end].iter().collect();
                // Pontuação final não faz parte (fim de frase), nem um ")" sem "(".
                while let Some(last) = url.chars().last() {
                    let unbalanced = last == ')' && url.matches('(').count() < url.matches(')').count();
                    if ".,;:!?'".contains(last) || unbalanced || last == ']' {
                        url.pop();
                        end -= 1;
                    } else {
                        break;
                    }
                }
                if end > start + scheme.chars().count() && (start..end).contains(&at) {
                    return Some((cells[start], cells[end - 1], url));
                }
                start = end.max(start + 1);
            } else {
                start += 1;
            }
        }
        None
    }

    /// Linha inteira (com as continuações por quebra automática), para o triplo clique.
    pub fn line_bounds(&self, abs: u64) -> Option<((u64, usize), (u64, usize))> {
        self.line_abs(abs)?;
        let mut first = abs;
        while first > self.dropped && self.line_abs(first - 1).is_some_and(|r| r.wrapped) {
            first -= 1;
        }
        let mut last = abs;
        while self.line_abs(last).is_some_and(|r| r.wrapped) && self.line_abs(last + 1).is_some() {
            last += 1;
        }
        Some(((first, 0), (last, self.cols - 1)))
    }

    /// Primeira linha absoluta guardada e a última da tela.
    pub fn abs_range(&self) -> (u64, u64) {
        (self.dropped, self.dropped + (self.history_len() + self.rows) as u64 - 1)
    }

    /// Prompt anterior (ou seguinte) a partir da linha absoluta `from`.
    pub fn prompt_near(&self, from: u64, forward: bool) -> Option<u64> {
        if forward {
            self.marks.iter().map(|m| m.line).find(|&m| m > from)
        } else {
            self.marks.iter().rev().map(|m| m.line).find(|&m| m < from)
        }
    }

    /// Os prompts guardados (para as marcas ao lado das linhas).
    pub fn marks(&self) -> &[PromptMark] {
        &self.marks
    }

    /// Comandos que terminaram desde a última consulta.
    pub fn take_finished(&mut self) -> Vec<Finished> {
        std::mem::take(&mut self.finished)
    }

    // ---- Mouse ----

    /// Bytes que informam um evento do mouse ao programa (se ele pediu), com a célula
    /// (coluna e linha, a partir de 0) e os modificadores.
    pub fn mouse_report(&self, button: MouseButton, event: MouseEvent, col: usize, row: usize, mods: (bool, bool, bool)) -> Option<Vec<u8>> {
        let wanted = match (self.modes.mouse, event) {
            (MouseMode::Off, _) => false,
            (MouseMode::X10, MouseEvent::Press) => true,
            (MouseMode::X10, _) => false,
            (_, MouseEvent::Press | MouseEvent::Release) => true,
            (MouseMode::Drag, MouseEvent::Motion) => button != MouseButton::None,
            (MouseMode::Motion, MouseEvent::Motion) => true,
            _ => false,
        };
        if !wanted {
            return None;
        }
        let mut code: u32 = match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::None => 3,
            MouseButton::WheelUp => 64,
            MouseButton::WheelDown => 65,
        };
        if matches!(button, MouseButton::WheelUp | MouseButton::WheelDown) && event == MouseEvent::Release {
            return None;
        }
        if event == MouseEvent::Motion {
            code += 32;
        }
        if self.modes.mouse != MouseMode::X10 {
            let (shift, alt, ctrl) = mods;
            code += 4 * shift as u32 + 8 * alt as u32 + 16 * ctrl as u32;
        }
        let (x, y) = (col + 1, row + 1);
        if self.modes.mouse_sgr {
            let end = if event == MouseEvent::Release { 'm' } else { 'M' };
            return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
        }
        if event == MouseEvent::Release {
            code = (code & !3) | 3;
        }
        if x > 223 || y > 223 {
            return None;
        }
        Some(vec![0x1b, b'[', b'M', (32 + code) as u8, (32 + x) as u8, (32 + y) as u8])
    }

    // ---- Operações da tela ----

    fn blank_row(&self) -> Row {
        Row::new(self.cols, self.pen.bg)
    }

    fn set_cursor(&mut self, row: usize, col: usize) {
        self.row = row.min(self.rows - 1);
        self.col = col.min(self.cols - 1);
        self.pending_wrap = false;
    }

    /// Posição absoluta, respeitando o modo de origem (linhas relativas à região).
    fn goto(&mut self, row: usize, col: usize) {
        let (row, max) = if self.modes.origin { (row + self.top, self.bottom) } else { (row, self.rows - 1) };
        self.set_cursor(row.min(max), col);
    }

    fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top + 1);
        let blank = self.blank_row();
        for _ in 0..n {
            let row = self.grid_mut().remove(top);
            if top == 0 && !self.alt {
                self.push_history(row);
            }
            self.grid_mut().insert(bottom, blank.clone());
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top + 1);
        let blank = self.blank_row();
        for _ in 0..n {
            self.grid_mut().remove(bottom);
            self.grid_mut().insert(top, blank.clone());
        }
    }

    fn linefeed(&mut self) {
        if self.row == self.bottom {
            self.scroll_up(1);
        } else if self.row + 1 < self.rows {
            self.row += 1;
        }
        self.pending_wrap = false;
    }

    fn reverse_index(&mut self) {
        if self.row == self.top {
            self.scroll_down(1);
        } else if self.row > 0 {
            self.row -= 1;
        }
        self.pending_wrap = false;
    }

    /// Antes de escrever em (row, col): se ali há metade de um caractere largo, a
    /// outra metade fica vazia.
    fn break_wide(&mut self, row: usize, col: usize) {
        let cols = self.cols;
        match self.grid()[row].cells[col].wide {
            Wide::Lead if col + 1 < cols => *self.cell_mut(row, col + 1) = Cell::default(),
            Wide::Spacer if col > 0 => *self.cell_mut(row, col - 1) = Cell::default(),
            _ => {}
        }
    }

    fn erase_cells(&mut self, row: usize, from: usize, to: usize) {
        let to = to.min(self.cols);
        if from >= to {
            return;
        }
        self.break_wide(row, from);
        self.break_wide(row, to - 1);
        let blank = Cell::blank(self.pen.bg);
        for cell in &mut self.grid_mut()[row].cells[from..to] {
            *cell = blank.clone();
        }
    }

    fn erase_rows(&mut self, from: usize, to: usize) {
        for row in from..to.min(self.rows) {
            self.erase_cells(row, 0, self.cols);
            self.grid_mut()[row].wrapped = false;
        }
    }

    fn insert_cells(&mut self, n: usize) {
        let (row, col, cols) = (self.row, self.col, self.cols);
        let n = n.min(cols - col);
        self.break_wide(row, col);
        let blank = Cell::blank(self.pen.bg);
        let cells = &mut self.grid_mut()[row].cells;
        cells.truncate(cols - n);
        for _ in 0..n {
            cells.insert(col, blank.clone());
        }
        if cells[cols - 1].wide == Wide::Lead {
            cells[cols - 1] = Cell::default();
        }
    }

    fn delete_cells(&mut self, n: usize) {
        let (row, col, cols) = (self.row, self.col, self.cols);
        let n = n.min(cols - col);
        self.break_wide(row, col);
        self.break_wide(row, (col + n).min(cols) - 1);
        let blank = Cell::blank(self.pen.bg);
        let cells = &mut self.grid_mut()[row].cells;
        cells.drain(col..col + n);
        cells.resize(cols, blank);
    }

    fn insert_lines(&mut self, n: usize) {
        if self.row < self.top || self.row > self.bottom {
            return;
        }
        let (row, bottom) = (self.row, self.bottom);
        let blank = self.blank_row();
        for _ in 0..n.min(bottom - row + 1) {
            self.grid_mut().remove(bottom);
            self.grid_mut().insert(row, blank.clone());
        }
        self.col = 0;
        self.pending_wrap = false;
    }

    fn delete_lines(&mut self, n: usize) {
        if self.row < self.top || self.row > self.bottom {
            return;
        }
        let (row, bottom) = (self.row, self.bottom);
        let blank = self.blank_row();
        for _ in 0..n.min(bottom - row + 1) {
            self.grid_mut().remove(row);
            self.grid_mut().insert(bottom, blank.clone());
        }
        self.col = 0;
        self.pending_wrap = false;
    }

    fn tab_forward(&mut self, n: usize) {
        for _ in 0..n {
            let next = (self.col + 1..self.cols).find(|&c| self.tabs[c]).unwrap_or(self.cols - 1);
            self.col = next;
        }
        self.pending_wrap = false;
    }

    fn tab_backward(&mut self, n: usize) {
        for _ in 0..n {
            self.col = (0..self.col).rev().find(|&c| self.tabs[c]).unwrap_or(0);
        }
        self.pending_wrap = false;
    }

    fn save_cursor(&mut self) {
        let saved = SavedCursor {
            row: self.row,
            col: self.col,
            pen: self.pen,
            origin: self.modes.origin,
            pending_wrap: self.pending_wrap,
            charsets: self.charsets,
            gl: self.gl,
        };
        if self.alt {
            self.saved_alt = Some(saved);
        } else {
            self.saved_primary = Some(saved);
        }
    }

    fn restore_cursor(&mut self) {
        let saved = if self.alt { self.saved_alt } else { self.saved_primary }.unwrap_or_default();
        self.pen = saved.pen;
        self.modes.origin = saved.origin;
        self.charsets = saved.charsets;
        self.gl = saved.gl;
        self.set_cursor(saved.row, saved.col);
        self.pending_wrap = saved.pending_wrap;
    }

    fn enter_alt(&mut self, save: bool, clear: bool) {
        if self.alt {
            return;
        }
        if save {
            self.save_cursor();
        }
        self.alt = true;
        if clear {
            self.alternate = vec![self.blank_row(); self.rows];
        }
        self.pending_wrap = false;
    }

    fn leave_alt(&mut self, restore: bool) {
        if !self.alt {
            return;
        }
        self.alt = false;
        if restore {
            self.restore_cursor();
        }
        self.pending_wrap = false;
    }

    /// Volta ao estado inicial (RIS), mantendo o histórico.
    fn full_reset(&mut self) {
        let (cols, rows) = (self.cols, self.rows);
        let history = std::mem::take(&mut self.history);
        let dropped = self.dropped;
        if !self.alt {
            for row in self.primary.drain(..) {
                if !row.is_blank() {
                    self.history.push_back(row);
                }
            }
        }
        let mut history = history;
        history.extend(self.history.drain(..));
        let cell_pixels = self.cell_pixels;
        let theme = std::mem::take(&mut self.theme);
        let history_limit = self.history_limit;
        let bold_bright = self.bold_bright;
        *self = Terminal::new(cols, rows);
        self.history = history;
        self.dropped = dropped;
        self.cell_pixels = cell_pixels;
        self.theme = theme;
        self.history_limit = history_limit;
        self.bold_bright = bold_bright;
        while self.history.len() > self.history_limit {
            self.history.pop_front();
            self.dropped += 1;
        }
    }

    /// DECSTR: modos e cursor ao padrão, sem apagar a tela.
    fn soft_reset(&mut self) {
        self.modes = Modes { mouse: self.modes.mouse, mouse_sgr: self.modes.mouse_sgr, ..Modes::default() };
        self.pen = Pen::default();
        self.top = 0;
        self.bottom = self.rows - 1;
        self.charsets = [Charset::Ascii; 2];
        self.gl = 0;
        self.saved_primary = None;
        self.saved_alt = None;
        self.cursor_shape = CursorShape::Block;
        self.pending_wrap = false;
    }

    fn respond(&mut self, s: &str) {
        self.responses.extend_from_slice(s.as_bytes());
    }

    // ---- SGR, modos e OSC ----

    fn sgr(&mut self, params: &[Vec<u16>]) {
        if params.is_empty() {
            self.pen = Pen::default();
            return;
        }
        let mut i = 0;
        while i < params.len() {
            let p = &params[i];
            match p[0] {
                0 => self.pen = Pen::default(),
                1 => self.pen.attr |= attr::BOLD,
                2 => self.pen.attr |= attr::DIM,
                3 => self.pen.attr |= attr::ITALIC,
                4 => {
                    self.pen.attr &= !(attr::UNDERLINE | attr::DOUBLE_UNDERLINE);
                    // 4:0 tira; 4:2 duplo; os outros estilos (ondulado...) viram simples.
                    match p.get(1).copied() {
                        Some(0) => {}
                        Some(2) => self.pen.attr |= attr::DOUBLE_UNDERLINE,
                        _ => self.pen.attr |= attr::UNDERLINE,
                    }
                }
                5 | 6 => self.pen.attr |= attr::BLINK,
                7 => self.pen.attr |= attr::INVERSE,
                8 => self.pen.attr |= attr::HIDDEN,
                9 => self.pen.attr |= attr::STRIKE,
                21 => self.pen.attr |= attr::DOUBLE_UNDERLINE,
                22 => self.pen.attr &= !(attr::BOLD | attr::DIM),
                23 => self.pen.attr &= !attr::ITALIC,
                24 => self.pen.attr &= !(attr::UNDERLINE | attr::DOUBLE_UNDERLINE),
                25 => self.pen.attr &= !attr::BLINK,
                27 => self.pen.attr &= !attr::INVERSE,
                28 => self.pen.attr &= !attr::HIDDEN,
                29 => self.pen.attr &= !attr::STRIKE,
                n @ 30..=37 => self.pen.fg = Color::Indexed((n - 30) as u8),
                39 => self.pen.fg = Color::Default,
                n @ 40..=47 => self.pen.bg = Color::Indexed((n - 40) as u8),
                49 => self.pen.bg = Color::Default,
                n @ 90..=97 => self.pen.fg = Color::Indexed((n - 90 + 8) as u8),
                n @ 100..=107 => self.pen.bg = Color::Indexed((n - 100 + 8) as u8),
                n @ (38 | 48 | 58) => {
                    let (color, used) = extended_color(params, i);
                    i += used;
                    if let Some(color) = color {
                        match n {
                            38 => self.pen.fg = color,
                            48 => self.pen.bg = color,
                            _ => {} // cor do sublinhado: aceita e ignora.
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn set_mode(&mut self, mode: u16, private: bool, on: bool) {
        if !private {
            match mode {
                4 => self.modes.insert = on,
                20 => self.modes.newline = on,
                _ => {}
            }
            return;
        }
        match mode {
            1 => self.modes.app_cursor = on,
            5 => self.modes.reverse_video = on,
            6 => {
                self.modes.origin = on;
                self.goto(0, 0);
            }
            7 => self.modes.autowrap = on,
            9 => self.modes.mouse = if on { MouseMode::X10 } else { MouseMode::Off },
            12 => self.modes.cursor_blink = on,
            25 => self.modes.show_cursor = on,
            47 => {
                if on { self.enter_alt(false, false) } else { self.leave_alt(false) }
            }
            1000 => self.modes.mouse = if on { MouseMode::Press } else { MouseMode::Off },
            1002 => self.modes.mouse = if on { MouseMode::Drag } else { MouseMode::Off },
            1003 => self.modes.mouse = if on { MouseMode::Motion } else { MouseMode::Off },
            1004 => self.modes.focus_events = on,
            1006 => self.modes.mouse_sgr = on,
            1047 => {
                if on {
                    self.enter_alt(false, true)
                } else {
                    self.leave_alt(false)
                }
            }
            1048 => {
                if on { self.save_cursor() } else { self.restore_cursor() }
            }
            1049 => {
                if on {
                    self.enter_alt(true, true)
                } else {
                    self.leave_alt(true)
                }
            }
            2004 => self.modes.bracketed_paste = on,
            2026 => self.modes.synchronized = on,
            _ => {}
        }
    }

    /// Estado de um modo para o DECRQM: 1 ligado, 2 desligado, 0 desconhecido.
    fn mode_state(&self, mode: u16, private: bool) -> u8 {
        let state = |on: bool| if on { 1 } else { 2 };
        if !private {
            return match mode {
                4 => state(self.modes.insert),
                20 => state(self.modes.newline),
                _ => 0,
            };
        }
        match mode {
            1 => state(self.modes.app_cursor),
            5 => state(self.modes.reverse_video),
            6 => state(self.modes.origin),
            7 => state(self.modes.autowrap),
            9 => state(self.modes.mouse == MouseMode::X10),
            12 => state(self.modes.cursor_blink),
            25 => state(self.modes.show_cursor),
            47 | 1047 | 1049 => state(self.alt),
            1000 => state(self.modes.mouse == MouseMode::Press),
            1002 => state(self.modes.mouse == MouseMode::Drag),
            1003 => state(self.modes.mouse == MouseMode::Motion),
            1004 => state(self.modes.focus_events),
            1006 => state(self.modes.mouse_sgr),
            2004 => state(self.modes.bracketed_paste),
            2026 => state(self.modes.synchronized),
            _ => 0,
        }
    }

    fn osc_color(&mut self, code: &str, spec: &str) {
        let current = match code {
            "10" => self.default_fg(),
            "11" => self.default_bg(),
            _ => self.cursor_color(),
        };
        if spec == "?" {
            self.respond(&format!("\x1b]{code};{}\x1b\\", color_spec(current)));
            return;
        }
        let Some(rgb) = parse_color(spec) else {
            return;
        };
        match code {
            "10" => self.fg_override = Some(rgb),
            "11" => self.bg_override = Some(rgb),
            _ => self.cursor_override = Some(rgb),
        }
    }
}

impl Perform for Terminal {
    fn print(&mut self, c: char) {
        let c = if self.charsets[self.gl] == Charset::DecGraphics { dec_graphics(c) } else { c };
        let width = char_width(c);
        if width == 0 {
            // Acento combinado com o caractere anterior.
            let col = if self.pending_wrap { self.col } else { self.col.checked_sub(1).unwrap_or(0) };
            let row = self.row;
            let col = if self.grid()[row].cells[col].wide == Wide::Spacer { col.saturating_sub(1) } else { col };
            let cell = self.cell_mut(row, col);
            let mut extra = cell.combining.take().map(String::from).unwrap_or_default();
            if extra.len() < 32 {
                extra.push(c);
            }
            cell.combining = Some(extra.into_boxed_str());
            return;
        }
        if self.pending_wrap {
            if self.modes.autowrap {
                let row = self.row;
                self.grid_mut()[row].wrapped = true;
                self.col = 0;
                self.linefeed();
            }
            self.pending_wrap = false;
        }
        if width == 2 && self.col + 1 >= self.cols {
            if self.modes.autowrap {
                let (row, col) = (self.row, self.col);
                self.erase_cells(row, col, col + 1);
                self.grid_mut()[row].wrapped = true;
                self.col = 0;
                self.linefeed();
            } else {
                self.col = self.cols - 2;
            }
        }
        if self.modes.insert {
            self.insert_cells(width);
        }
        let (row, col) = (self.row, self.col);
        self.break_wide(row, col);
        if width == 2 {
            self.break_wide(row, col + 1);
        }
        let pen = self.pen;
        *self.cell_mut(row, col) = Cell { c, pen, wide: if width == 2 { Wide::Lead } else { Wide::No }, combining: None };
        if width == 2 {
            *self.cell_mut(row, col + 1) = Cell { c: ' ', pen, wide: Wide::Spacer, combining: None };
        }
        self.last_char = Some(c);
        self.col += width;
        if self.col >= self.cols {
            self.col = self.cols - 1;
            self.pending_wrap = true;
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => self.bell = true,
            0x08 => {
                self.col = self.col.saturating_sub(1);
                self.pending_wrap = false;
            }
            0x09 => self.tab_forward(1),
            0x0A..=0x0C => {
                self.linefeed();
                if self.modes.newline {
                    self.col = 0;
                }
            }
            0x0D => {
                self.col = 0;
                self.pending_wrap = false;
            }
            0x0E => self.gl = 1,
            0x0F => self.gl = 0,
            _ => {}
        }
    }

    fn csi(&mut self, params: &[Vec<u16>], private: Option<u8>, intermediates: &[u8], action: u8) {
        // Parâmetro `i`, com `default` quando falta ou é 0.
        let p = |i: usize, default: usize| params.get(i).map(|v| v[0] as usize).filter(|&v| v != 0).unwrap_or(default);
        let raw = |i: usize| params.get(i).map_or(0, |v| v[0]);
        match (private, intermediates, action) {
            (None, [], b'@') => self.insert_cells(p(0, 1)),
            (None, [], b'A') => {
                let min = if self.row >= self.top { self.top } else { 0 };
                let row = self.row.saturating_sub(p(0, 1)).max(min);
                self.set_cursor(row, self.col);
            }
            (None, [], b'B' | b'e') => {
                let max = if self.row <= self.bottom { self.bottom } else { self.rows - 1 };
                let row = (self.row + p(0, 1)).min(max);
                self.set_cursor(row, self.col);
            }
            (None, [], b'C' | b'a') => self.set_cursor(self.row, self.col + p(0, 1)),
            (None, [], b'D') => self.set_cursor(self.row, self.col.saturating_sub(p(0, 1))),
            (None, [], b'E') => {
                let row = (self.row + p(0, 1)).min(self.bottom.max(self.row));
                self.set_cursor(row, 0);
            }
            (None, [], b'F') => {
                let row = self.row.saturating_sub(p(0, 1)).max(if self.row >= self.top { self.top } else { 0 });
                self.set_cursor(row, 0);
            }
            (None, [], b'G' | b'`') => self.set_cursor(self.row, p(0, 1) - 1),
            (None, [], b'H' | b'f') => self.goto(p(0, 1) - 1, p(1, 1) - 1),
            (None, [], b'I') => self.tab_forward(p(0, 1)),
            (None | Some(b'?'), [], b'J') => match raw(0) {
                0 => {
                    let (row, col, cols) = (self.row, self.col, self.cols);
                    self.erase_cells(row, col, cols);
                    self.grid_mut()[row].wrapped = false;
                    self.erase_rows(row + 1, self.rows);
                }
                1 => {
                    let (row, col) = (self.row, self.col);
                    self.erase_rows(0, row);
                    self.erase_cells(row, 0, col + 1);
                }
                2 => self.erase_rows(0, self.rows),
                3 => {
                    self.dropped += self.history.len() as u64;
                    self.history.clear();
                    self.marks.clear();
                }
                _ => {}
            },
            (None | Some(b'?'), [], b'K') => {
                let (row, col, cols) = (self.row, self.col, self.cols);
                match raw(0) {
                    0 => {
                        self.erase_cells(row, col, cols);
                        self.grid_mut()[row].wrapped = false;
                    }
                    1 => self.erase_cells(row, 0, col + 1),
                    2 => {
                        self.erase_cells(row, 0, cols);
                        self.grid_mut()[row].wrapped = false;
                    }
                    _ => {}
                }
            }
            (None, [], b'L') => self.insert_lines(p(0, 1)),
            (None, [], b'M') => self.delete_lines(p(0, 1)),
            (None, [], b'P') => self.delete_cells(p(0, 1)),
            (None, [], b'S') => self.scroll_up(p(0, 1)),
            (None, [], b'T') => self.scroll_down(p(0, 1)),
            (None, [], b'X') => {
                let (row, col) = (self.row, self.col);
                self.erase_cells(row, col, col + p(0, 1));
            }
            (None, [], b'Z') => self.tab_backward(p(0, 1)),
            (None, [], b'b') => {
                if let Some(c) = self.last_char {
                    for _ in 0..p(0, 1).min(65_535) {
                        self.print(c);
                    }
                }
            }
            (None, [], b'c') if raw(0) == 0 => self.respond("\x1b[?62;22c"),
            (Some(b'>'), [], b'c') if raw(0) == 0 => self.respond("\x1b[>1;10;0c"),
            (None, [], b'd') => {
                let row = p(0, 1) - 1;
                let row = if self.modes.origin { (row + self.top).min(self.bottom) } else { row };
                self.set_cursor(row, self.col);
            }
            (None, [], b'g') => match raw(0) {
                0 => self.tabs[self.col] = false,
                3 => self.tabs.iter_mut().for_each(|t| *t = false),
                _ => {}
            },
            (None, [], b'h' | b'l') | (Some(b'?'), [], b'h' | b'l') => {
                for param in params {
                    self.set_mode(param[0], private.is_some(), action == b'h');
                }
            }
            (None, [], b'm') => self.sgr(params),
            (None, [], b'n') => match raw(0) {
                5 => self.respond("\x1b[0n"),
                6 => {
                    let row = if self.modes.origin { self.row - self.top } else { self.row };
                    let s = format!("\x1b[{};{}R", row + 1, self.col + 1);
                    self.respond(&s);
                }
                _ => {}
            },
            (None, [], b'r') => {
                let top = p(0, 1) - 1;
                let bottom = p(1, self.rows).min(self.rows) - 1;
                if top < bottom {
                    self.top = top;
                    self.bottom = bottom;
                    self.goto(0, 0);
                }
            }
            (None, [], b's') => self.save_cursor(),
            (None, [], b'u') => self.restore_cursor(),
            (None, [], b't') => match raw(0) {
                14 => {
                    let (w, h) = (self.cols * self.cell_pixels.0 as usize, self.rows * self.cell_pixels.1 as usize);
                    self.respond(&format!("\x1b[4;{h};{w}t"));
                }
                16 => {
                    let (w, h) = self.cell_pixels;
                    self.respond(&format!("\x1b[6;{h};{w}t"));
                }
                18 => {
                    let s = format!("\x1b[8;{};{}t", self.rows, self.cols);
                    self.respond(&s);
                }
                22 => {
                    if self.title_stack.len() < 16 {
                        self.title_stack.push(self.title.clone());
                    }
                }
                23 => {
                    if let Some(title) = self.title_stack.pop() {
                        self.title = title;
                    }
                }
                _ => {}
            },
            (None, [b' '], b'q') => {
                let style = raw(0);
                self.cursor_shape = match style {
                    3 | 4 => CursorShape::Underline,
                    5 | 6 => CursorShape::Bar,
                    _ => CursorShape::Block,
                };
                self.modes.cursor_blink = style == 0 || style % 2 == 1;
            }
            (None, [b'!'], b'p') => self.soft_reset(),
            (_, [b'$'], b'p') => {
                let mode = raw(0);
                let state = self.mode_state(mode, private == Some(b'?'));
                let mark = if private == Some(b'?') { "?" } else { "" };
                self.respond(&format!("\x1b[{mark}{mode};{state}$y"));
            }
            (Some(b'>'), [], b'q') => self.respond("\x1bP>|mide-terminal 0.1\x1b\\"),
            _ => tracing::trace!("CSI não tratada: {private:?} {params:?} {intermediates:?} {}", action as char),
        }
    }

    fn esc(&mut self, intermediates: &[u8], byte: u8) {
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.linefeed(),
            ([], b'E') => {
                self.col = 0;
                self.linefeed();
            }
            ([], b'H') => self.tabs[self.col] = true,
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.full_reset(),
            ([], b'=') => self.modes.app_keypad = true,
            ([], b'>') => self.modes.app_keypad = false,
            ([b'('], c) | ([b')'], c) => {
                let set = if c == b'0' { Charset::DecGraphics } else { Charset::Ascii };
                self.charsets[(intermediates[0] == b')') as usize] = set;
            }
            ([b'#'], b'8') => {
                // DECALN: tela cheia de "E" (teste de alinhamento).
                let cols = self.cols;
                for row in self.grid_mut() {
                    *row = Row { cells: vec![Cell { c: 'E', ..Cell::default() }; cols], wrapped: false };
                }
                self.top = 0;
                self.bottom = self.rows - 1;
                self.set_cursor(0, 0);
            }
            _ => {}
        }
    }

    fn osc(&mut self, data: &[u8]) {
        let text = String::from_utf8_lossy(data);
        let (code, rest) = text.split_once(';').unwrap_or((&text, ""));
        match code {
            "0" | "2" => self.title = Some(rest.chars().filter(|c| !c.is_control()).take(256).collect()),
            "4" => {
                let parts: Vec<&str> = rest.split(';').collect();
                for pair in parts.chunks(2) {
                    let (Some(index), Some(spec)) = (pair.first().and_then(|i| i.parse::<u8>().ok()), pair.get(1)) else {
                        continue;
                    };
                    if *spec == "?" {
                        let rgb = self.indexed(index);
                        self.respond(&format!("\x1b]4;{index};{}\x1b\\", color_spec(rgb)));
                    } else if let Some(rgb) = parse_color(spec) {
                        self.palette[index as usize] = Some(rgb);
                    }
                }
            }
            "10" | "11" | "12" => {
                // Vários de uma vez: "10;?;?" pergunta o 10 e o 11.
                let mut code_num: u8 = code.parse().unwrap_or(10);
                for spec in rest.split(';') {
                    if code_num > 12 {
                        break;
                    }
                    self.osc_color(&code_num.to_string(), spec);
                    code_num += 1;
                }
            }
            "104" => {
                if rest.is_empty() {
                    self.palette.iter_mut().for_each(|c| *c = None);
                } else {
                    for index in rest.split(';').filter_map(|i| i.parse::<u8>().ok()) {
                        self.palette[index as usize] = None;
                    }
                }
            }
            "110" => self.fg_override = None,
            "111" => self.bg_override = None,
            "112" => self.cursor_override = None,
            "52" => {
                // Copiar pelo programa (ex.: vim e tmux por SSH). Ler a área de
                // transferência pelo terminal não é permitido.
                let (_, payload) = rest.split_once(';').unwrap_or(("", rest));
                if payload != "?" {
                    if let Some(text) = decode_base64(payload).and_then(|b| String::from_utf8(b).ok()) {
                        self.clipboard = Some(text);
                    }
                }
            }
            "7" => self.cwd = parse_file_url(rest),
            "133" => match rest.chars().next() {
                Some('A') => {
                    let line = self.dropped + (self.history_len() + self.row) as u64;
                    if self.marks.last().is_none_or(|m| m.line != line) {
                        self.marks.push(PromptMark { line, status: None });
                    }
                }
                Some('C') => self.command_started = Some(std::time::Instant::now()),
                Some('D') => {
                    // Só vale o fim de um comando que começou (o shell manda D a cada prompt).
                    if let Some(started) = self.command_started.take() {
                        let status = rest.split(';').nth(1).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
                        if let Some(mark) = self.marks.last_mut() {
                            mark.status = Some(status);
                            self.finished.push(Finished { line: mark.line, status, duration: started.elapsed() });
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c > 0 && c % 8 == 0).collect()
}

/// Cor estendida de SGR 38/48/58 a partir de `params[i]`: a cor e quantos
/// parâmetros a mais ela usou (na forma com `;`).
fn extended_color(params: &[Vec<u16>], i: usize) -> (Option<Color>, usize) {
    let p = &params[i];
    if p.len() > 1 {
        // Forma com `:` (38:5:n ou 38:2:espaço:r:g:b ou 38:2:r:g:b).
        return match p[1] {
            5 => (p.get(2).map(|&n| Color::Indexed(n.min(255) as u8)), 0),
            2 => {
                let rgb = if p.len() >= 6 { &p[3..6] } else if p.len() == 5 { &p[2..5] } else { return (None, 0) };
                (Some(Color::Rgb(rgb[0].min(255) as u8, rgb[1].min(255) as u8, rgb[2].min(255) as u8)), 0)
            }
            _ => (None, 0),
        };
    }
    let get = |k: usize| params.get(i + k).map(|v| v[0].min(255) as u8);
    match params.get(i + 1).map(|v| v[0]) {
        Some(5) => (get(2).map(Color::Indexed), 2),
        Some(2) => match (get(2), get(3), get(4)) {
            (Some(r), Some(g), Some(b)) => (Some(Color::Rgb(r, g, b)), 4),
            _ => (None, params.len() - i - 1),
        },
        _ => (None, 0),
    }
}

/// Desenho de linhas do DEC: letras viram cantos e traços.
fn dec_graphics(c: char) -> char {
    match c {
        '`' => '◆',
        'a' => '▒',
        'f' => '°',
        'g' => '±',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        c => c,
    }
}

/// Cor no formato do X11 (`rgb:rr/gg/bb`, de 1 a 4 algarismos cada) ou `#rrggbb`.
fn parse_color(spec: &str) -> Option<Rgb> {
    if let Some(hex) = spec.strip_prefix('#') {
        if hex.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        return Some([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
    }
    let parts: Vec<&str> = spec.strip_prefix("rgb:")?.split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut rgb = [0u8; 3];
    for (k, part) in parts.iter().enumerate() {
        if part.is_empty() || part.len() > 4 {
            return None;
        }
        let v = u32::from_str_radix(part, 16).ok()?;
        let max = (1u32 << (4 * part.len())) - 1;
        rgb[k] = (v * 255 / max) as u8;
    }
    Some(rgb)
}

fn color_spec(rgb: Rgb) -> String {
    format!("rgb:{:02x}{0:02x}/{:02x}{1:02x}/{:02x}{2:02x}", rgb[0], rgb[1], rgb[2])
}

fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Pasta de um `file://máquina/caminho` (OSC 7), com os `%xx` decodificados.
fn parse_file_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&path[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Texto da tela (sem o histórico), uma linha por linha, sem espaços no fim.
    fn screen(t: &Terminal) -> Vec<String> {
        (0..t.rows())
            .map(|y| {
                let row = t.visible_row(y, 0);
                row.cells.iter().filter(|c| c.wide != Wide::Spacer).map(Cell::text).collect::<String>().trim_end().to_string()
            })
            .collect()
    }

    fn term(cols: usize, rows: usize, input: &str) -> Terminal {
        let mut t = Terminal::new(cols, rows);
        t.feed(input.as_bytes());
        t
    }

    #[test]
    fn texto_quebra_e_rola_para_o_historico() {
        let t = term(5, 2, "abcdefg\r\nhi\r\nxy");
        assert_eq!(screen(&t), ["hi", "xy"]);
        assert_eq!(t.history_len(), 2);
        assert_eq!(t.text_between((0, 0), (1, 4)), "abcdefg");
        assert_eq!(t.cursor(), (1, 2));
    }

    #[test]
    fn quebra_so_no_proximo_caractere() {
        let mut t = term(4, 3, "abcd");
        assert_eq!(t.cursor(), (0, 3));
        assert!(t.pending_wrap);
        t.feed(b"\r\n");
        assert_eq!(screen(&t), ["abcd", "", ""]);
        // Sem quebra automática, o último caractere é sobrescrito.
        let t = term(4, 2, "\x1b[?7labcdef");
        assert_eq!(screen(&t), ["abcf", ""]);
    }

    #[test]
    fn cursor_e_apagar() {
        let t = term(10, 3, "hello\x1b[2;3Hx\x1b[1;2H\x1b[K");
        assert_eq!(screen(&t), ["h", "  x", ""]);
        let t = term(10, 3, "abcdef\x1b[1;3H\x1b[2P");
        assert_eq!(screen(&t), ["abef", "", ""]);
        let t = term(10, 3, "abcdef\x1b[1;3H\x1b[2@");
        assert_eq!(screen(&t), ["ab  cdef", "", ""]);
        let t = term(10, 3, "a\r\nb\r\nc\x1b[2;1H\x1b[L");
        assert_eq!(screen(&t), ["a", "", "b"]);
        let t = term(10, 3, "a\r\nb\r\nc\x1b[1;1H\x1b[M");
        assert_eq!(screen(&t), ["b", "c", ""]);
        let t = term(10, 2, "abc\x1b[2J");
        assert_eq!(screen(&t), ["", ""]);
    }

    #[test]
    fn regiao_de_rolagem() {
        let t = term(5, 4, "1\r\n2\r\n3\r\n4\x1b[2;3r\x1b[3;1H\n\nx");
        assert_eq!(screen(&t), ["1", "", "x", "4"]);
        // Fora da linha 0, nada vai para o histórico.
        assert_eq!(t.history_len(), 0);
        let t = term(5, 3, "a\r\nb\r\nc\x1b[1;1H\x1bM");
        assert_eq!(screen(&t), ["", "a", "b"]);
    }

    #[test]
    fn cores_e_atributos() {
        let t = term(10, 1, "\x1b[1;31ma\x1b[38;5;200mb\x1b[48;2;1;2;3mc\x1b[38:2::9:8:7md\x1b[0me");
        let cells = &t.visible_row(0, 0).cells;
        assert_eq!(cells[0].pen, Pen { fg: Color::Indexed(1), bg: Color::Default, attr: attr::BOLD });
        assert_eq!(cells[1].pen.fg, Color::Indexed(200));
        assert_eq!(cells[2].pen.bg, Color::Rgb(1, 2, 3));
        assert_eq!(cells[3].pen.fg, Color::Rgb(9, 8, 7));
        assert_eq!(cells[4].pen, Pen::default());
        // Negrito com as 8 cores básicas usa a versão clara.
        assert_eq!(t.cell_colors(&cells[0].pen).0, palette::default_color(&t.theme, 9));
    }

    #[test]
    fn largos_e_combinados() {
        let t = term(4, 2, "a中b😀");
        assert_eq!(screen(&t), ["a中b", "😀"]);
        assert_eq!(t.visible_row(0, 0).cells[1].wide, Wide::Lead);
        assert_eq!(t.visible_row(0, 0).cells[2].wide, Wide::Spacer);
        let t = term(4, 1, "e\u{301}x");
        assert_eq!(screen(&t), ["e\u{301}x"]);
        // Escrever em cima da metade de um largo apaga a outra metade.
        let t = term(4, 1, "中\x1b[1;2Hx");
        assert_eq!(screen(&t), [" x"]);
    }

    #[test]
    fn tela_alternativa_guarda_a_principal() {
        let mut t = term(5, 2, "prompt");
        t.feed(b"\x1b[?1049h\x1b[Hvim");
        assert!(t.alt_screen());
        assert_eq!(screen(&t), ["vim", ""]);
        t.feed(b"\x1b[?1049l");
        assert!(!t.alt_screen());
        assert_eq!(screen(&t), ["promp", "t"]);
        assert_eq!(t.cursor(), (1, 1));
    }

    #[test]
    fn respostas_ao_programa() {
        let mut t = term(80, 24, "\x1b[5;10H\x1b[6n\x1b[c\x1b[18t\x1b[?2004h\x1b[?2004$p");
        let r = String::from_utf8(t.take_responses()).unwrap();
        assert_eq!(r, "\x1b[5;10R\x1b[?62;22c\x1b[8;24;80t\x1b[?2004;1$y");
        t.feed(b"\x1b]11;?\x07");
        assert_eq!(String::from_utf8(t.take_responses()).unwrap(), "\x1b]11;rgb:ffff/ffff/ffff\x1b\\");
    }

    #[test]
    fn titulo_copiar_pasta_e_marcas() {
        let mut t = term(20, 3, "\x1b]0;meu título\x07\x1b]52;c;b2zDoQ==\x07\x1b]7;file://pc/home/eu/a%20b\x07");
        assert_eq!(t.title.as_deref(), Some("meu título"));
        assert_eq!(t.take_clipboard().as_deref(), Some("olá"));
        assert_eq!(t.cwd.as_deref(), Some("/home/eu/a b"));
        t.feed(b"\x1b]133;A\x07$ ls\r\n\x1b]133;C\x07x\r\n\x1b]133;D;2\x07\x1b]133;A\x07$ ");
        assert_eq!(t.prompt_near(10, false), Some(2));
        assert_eq!(t.prompt_near(2, false), Some(0));
        assert_eq!(t.prompt_near(0, true), Some(2));
        assert_eq!(t.marks()[0].status, Some(2));
        assert_eq!(t.marks()[1].status, None);
        let done = t.take_finished();
        assert_eq!((done.len(), done[0].line, done[0].status), (1, 0, 2));
        // D sem C (prompt vazio) não conta como comando.
        t.feed(b"\x1b]133;D;0\x07");
        assert!(t.take_finished().is_empty());
    }

    #[test]
    fn reflow_ao_mudar_a_largura() {
        let mut t = term(10, 3, "0123456789abcde\r\nxyz");
        assert_eq!(screen(&t), ["0123456789", "abcde", "xyz"]);
        t.resize(5, 3);
        assert_eq!(screen(&t), ["56789", "abcde", "xyz"]);
        assert_eq!(t.history_len(), 1);
        assert_eq!(t.cursor(), (2, 3));
        t.resize(20, 3);
        assert_eq!(screen(&t), ["0123456789abcde", "xyz", ""]);
        assert_eq!(t.history_len(), 0);
        assert_eq!(t.cursor(), (1, 3));
        // Mais linhas: as do histórico voltam para a tela.
        let mut t = term(5, 2, "a\r\nb\r\nc\r\nd");
        t.resize(5, 4);
        assert_eq!(screen(&t), ["a", "b", "c", "d"]);
        assert_eq!(t.cursor(), (3, 1));
    }

    #[test]
    fn selecao_palavra_e_linha() {
        let t = term(10, 2, "foo bar.txt baz");
        let ((a0, a1), (b0, b1)) = t.word_at(0, 6).unwrap();
        assert_eq!(t.text_between((a0, a1), (b0, b1)), "bar.txt");
        let (s, e) = t.line_bounds(1).unwrap();
        assert_eq!(t.text_between(s, e), "foo bar.txt baz");
    }

    #[test]
    fn busca_sem_acentos_e_atravessando_quebras() {
        let t = term(6, 3, "Ação\r\nabcACAOxy");
        let found = t.search("acao");
        assert_eq!(found, vec![((0, 0), (0, 3)), ((1, 3), (2, 0))]);
        assert!(t.search("").is_empty());
    }

    #[test]
    fn enderecos_para_abrir_com_clique() {
        let t = term(40, 3, "veja https://exemplo.com/a_(b). fim");
        let (a, b, url) = t.url_at(0, 10).unwrap();
        assert_eq!(url, "https://exemplo.com/a_(b)");
        assert_eq!((a, b), ((0, 5), (0, 29)));
        assert!(t.url_at(0, 1).is_none());
        // Quebrado em duas linhas.
        let t = term(10, 3, "x http://ab.cd/efgh y");
        assert_eq!(t.url_at(1, 2).unwrap().2, "http://ab.cd/efgh");
        let t = term(30, 1, "(mailto:eu@ex.com)");
        assert_eq!(t.url_at(0, 3).unwrap().2, "mailto:eu@ex.com");
    }

    #[test]
    fn mouse_sgr_e_x10() {
        let mut t = term(80, 24, "\x1b[?1000h\x1b[?1006h");
        assert_eq!(t.mouse_report(MouseButton::Left, MouseEvent::Press, 4, 2, (false, false, false)).unwrap(), b"\x1b[<0;5;3M");
        assert_eq!(t.mouse_report(MouseButton::Left, MouseEvent::Release, 4, 2, (false, false, true)).unwrap(), b"\x1b[<16;5;3m");
        assert!(t.mouse_report(MouseButton::None, MouseEvent::Motion, 4, 2, (false, false, false)).is_none());
        t.feed(b"\x1b[?1006l");
        assert_eq!(t.mouse_report(MouseButton::WheelUp, MouseEvent::Press, 0, 0, (false, false, false)).unwrap(), vec![0x1b, b'[', b'M', 96, 33, 33]);
    }

    #[test]
    fn graficos_dec_e_rep() {
        let t = term(10, 1, "\x1b(0lqk\x1b(Bq\x1b[3b");
        assert_eq!(screen(&t), ["┌─┐qqqq"]);
    }

    #[test]
    fn cores_x11() {
        assert_eq!(parse_color("rgb:ff/80/00"), Some([255, 128, 0]));
        assert_eq!(parse_color("rgb:ffff/0000/8080"), Some([255, 0, 128]));
        assert_eq!(parse_color("#102030"), Some([16, 32, 48]));
        assert_eq!(decode_base64("aGVsbG8="), Some(b"hello".to_vec()));
    }
}
