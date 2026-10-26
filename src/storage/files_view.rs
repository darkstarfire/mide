//! Visualização lembrada do Arquivos (B21): ícones, lista ou colunas, a ordenação e o
//! tamanho dos ícones de cada pasta, como no Finder.
//!
//! Regras:
//! - Cada pasta em que a pessoa trocou a visualização ou a ordenação volta como estava.
//! - Pastas nunca ajustadas abrem com o padrão, que é a última escolha feita em
//!   qualquer pasta (a Lixeira, sempre em lista, não muda o padrão).
//! - No máximo [`LIMIT`] pastas lembradas: as que ficaram mais tempo sem ajuste saem.
//!   A existência da pasta não é conferida: pendrive desconectado ou rede desmontada
//!   não apagam as escolhas, e um compartilhamento travado não congela a janela.
//! - A chave é o caminho absoluto sem `.`, barras repetidas ou barra no fim
//!   ([`key_for_path`]), sem perder bytes que não são UTF-8.
//!
//! Fica em `~/.local/state/mide/files-view.toml` (respeitando `XDG_STATE_HOME`): é
//! estado do app, não configuração. Gravado em arquivo temporário e trocado de uma vez
//! (nunca fica pela metade), relendo o disco antes para não perder o que outra janela
//! do Arquivos acabou de gravar.

use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Quantas pastas ficam lembradas.
pub const LIMIT: usize = 500;

/// Chave da Lixeira (não é uma pasta comum).
pub const TRASH_KEY: &str = "trash:///";

/// Como uma pasta aparece: ícones ou lista, e a coluna ordenada (0 nome,
/// 1 modificado, 2 tamanho, 3 tipo) com o sentido.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    #[serde(default)]
    pub list: bool,
    #[serde(default)]
    pub sort_column: usize,
    #[serde(default = "yes")]
    pub ascending: bool,
    /// Visualização em colunas (como no Finder); vale mais que `list`.
    #[serde(default)]
    pub columns: bool,
    /// Lado dos ícones da grade, em pontos (zoom: Ctrl+roda, Ctrl +/−).
    #[serde(default = "icon_default")]
    pub icon_size: u32,
}

fn yes() -> bool {
    true
}

/// Ícones da grade: tamanho padrão e limites do zoom.
pub const ICON_DEFAULT: u32 = 64;
pub const ICON_MIN: u32 = 32;
pub const ICON_MAX: u32 = 256;

fn icon_default() -> u32 {
    ICON_DEFAULT
}

impl Default for View {
    fn default() -> Self {
        Self { list: false, sort_column: 0, ascending: true, columns: false, icon_size: ICON_DEFAULT }
    }
}

impl View {
    fn clean(mut self) -> Self {
        if self.sort_column > 3 {
            self.sort_column = 0;
        }
        self.icon_size = self.icon_size.clamp(ICON_MIN, ICON_MAX);
        self
    }
}

/// Pasta lembrada. Campos faltando (arquivo de outra versão) tomam o padrão, para um
/// campo só não derrubar o arquivo inteiro.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Folder {
    path: String,
    #[serde(default)]
    list: bool,
    #[serde(default)]
    sort_column: usize,
    #[serde(default = "yes")]
    ascending: bool,
    #[serde(default)]
    columns: bool,
    #[serde(default = "icon_default")]
    icon_size: u32,
    /// Ordem do último ajuste (maior = mais recente).
    #[serde(default)]
    used: u64,
}

/// Caminho de uma pasta normalizado só no texto (sem tocar no disco, para nunca
/// travar numa rede parada): absoluto, sem `.`, sem barra repetida ou no fim, e `..`
/// resolvido pelo nome.
pub fn normalize(dir: &Path) -> PathBuf {
    let abs = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Chave de uma pasta na memória: o caminho normalizado. Em UTF-8 é o próprio texto;
/// senão `bytes:` seguido do caminho com os bytes fora do ASCII visível (e o `%`) em
/// `%XX`, sem perda e sem colidir com as chaves comuns (que começam com `/`).
pub fn key_for_path(dir: &Path) -> String {
    let dir = normalize(dir);
    if let Some(text) = dir.to_str() {
        return text.to_string();
    }
    let mut key = String::from("bytes:");
    for &b in dir.as_os_str().as_bytes() {
        if (b.is_ascii_graphic() && b != b'%') || b == b' ' {
            key.push(b as char);
        } else {
            key.push_str(&format!("%{b:02X}"));
        }
    }
    key
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewMemory {
    /// Padrão das pastas nunca ajustadas: a última escolha.
    #[serde(default)]
    default: View,
    /// Contador dos ajustes (para saber quais são os mais antigos).
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    folder: Vec<Folder>,
    /// Larguras das colunas Modificado, Tamanho e Tipo da lista (arrastadas no
    /// cabeçalho); valem para todas as pastas.
    #[serde(default)]
    list_widths: Option<[f32; 3]>,
    /// Largura da barra lateral (Favoritos e Lugares), arrastada pela borda dela; vale
    /// para todas as janelas.
    #[serde(default)]
    sidebar_width: Option<f32>,
}

/// Largura padrão da barra lateral e os limites do arrasto.
pub const SIDEBAR_WIDTH: f32 = 220.0;
pub const SIDEBAR_MIN: f32 = 150.0;
pub const SIDEBAR_MAX: f32 = 360.0;

/// Larguras padrão das colunas da lista (Modificado, Tamanho, Tipo).
pub const LIST_WIDTHS: [f32; 3] = [165.0, 85.0, 150.0];

impl ViewMemory {
    /// Visualização de uma pasta: a dela, se foi ajustada, senão o padrão.
    pub fn view_for(&self, key: &str) -> View {
        self.folder
            .iter()
            .find(|f| f.path == key)
            .map(|f| View { list: f.list, sort_column: f.sort_column, ascending: f.ascending, columns: f.columns, icon_size: f.icon_size }.clean())
            .unwrap_or(self.default.clean())
    }

    pub fn default_view(&self) -> View {
        self.default.clean()
    }

    /// Larguras das colunas da lista (as guardadas ou as padrão).
    pub fn list_widths(&self) -> [f32; 3] {
        self.list_widths.filter(|w| w.iter().all(|v| v.is_finite() && (40.0..=2000.0).contains(v))).unwrap_or(LIST_WIDTHS)
    }

    /// Guarda as larguras das colunas relendo o disco antes (outra janela pode ter
    /// gravado).
    pub fn record_list_widths(widths: [f32; 3]) -> std::io::Result<()> {
        let file = path().ok_or_else(|| std::io::Error::other("sem pasta de estado"))?;
        let mut memory = match std::fs::read_to_string(&file) {
            // Arquivo estragado: não sobrescreve (o próximo `record` guarda a cópia).
            Ok(text) => Self::parse(&text).ok_or_else(|| std::io::Error::other("visualizações ilegíveis"))?,
            Err(_) => Self::default(),
        };
        memory.list_widths = Some(widths.map(|w| w.round()));
        memory.save_to(&file)
    }

    /// Largura da barra lateral (a guardada, dentro dos limites, ou a padrão).
    pub fn sidebar_width(&self) -> f32 {
        self.sidebar_width.filter(|w| w.is_finite()).map(|w| w.clamp(SIDEBAR_MIN, SIDEBAR_MAX)).unwrap_or(SIDEBAR_WIDTH)
    }

    /// Guarda a largura da barra lateral relendo o disco antes (como as colunas).
    pub fn record_sidebar_width(width: f32) -> std::io::Result<()> {
        let file = path().ok_or_else(|| std::io::Error::other("sem pasta de estado"))?;
        Self::record_sidebar_width_in(&file, width)
    }

    pub fn record_sidebar_width_in(file: &Path, width: f32) -> std::io::Result<()> {
        let mut memory = match std::fs::read_to_string(file) {
            Ok(text) => Self::parse(&text).ok_or_else(|| std::io::Error::other("visualizações ilegíveis"))?,
            Err(_) => Self::default(),
        };
        memory.sidebar_width = Some(width.clamp(SIDEBAR_MIN, SIDEBAR_MAX).round());
        memory.save_to(file)
    }

    pub fn len(&self) -> usize {
        self.folder.len()
    }

    pub fn is_empty(&self) -> bool {
        self.folder.is_empty()
    }

    /// Guarda a escolha de uma pasta (e, com `as_default`, faz dela o padrão).
    pub fn remember(&mut self, key: &str, view: View, as_default: bool) {
        let view = view.clean();
        self.seq += 1;
        let used = self.seq;
        // Mais recente primeiro (a ordem do arquivo também).
        self.folder.retain(|f| f.path != key);
        self.folder.insert(
            0,
            Folder { path: key.to_string(), list: view.list, sort_column: view.sort_column, ascending: view.ascending, columns: view.columns, icon_size: view.icon_size, used },
        );
        if as_default {
            self.default = view;
        }
        self.trim();
    }

    /// Mantém só as [`LIMIT`] ajustadas mais recentemente.
    fn trim(&mut self) {
        if self.folder.len() > LIMIT {
            self.folder.sort_by_key(|f| std::cmp::Reverse(f.used));
            self.folder.truncate(LIMIT);
        }
    }

    pub fn from_toml(text: &str) -> Self {
        Self::parse(text).unwrap_or_default()
    }

    /// Lê o texto; `None` se não for um arquivo válido.
    fn parse(text: &str) -> Option<Self> {
        let mut memory: Self = toml::from_str(text).ok()?;
        // Arquivo editado à mão ou de outra versão: sem repetidas, dentro do limite.
        let mut seen = std::collections::HashSet::new();
        memory.folder.sort_by_key(|f| std::cmp::Reverse(f.used));
        memory.folder.retain(|f| seen.insert(f.path.clone()));
        memory.trim();
        memory.seq = memory.seq.max(memory.folder.iter().map(|f| f.used).max().unwrap_or(0));
        Some(memory)
    }

    pub fn to_toml(&self) -> String {
        let body = toml::to_string(self).unwrap_or_default();
        format!("# Visualização do Arquivos do mide (ícones ou lista e ordenação) de cada pasta.\n\n{body}")
    }

    /// Lê o arquivo (vazio se não existir ou estiver estragado).
    pub fn load_from(file: &Path) -> Self {
        std::fs::read_to_string(file).map(|t| Self::from_toml(&t)).unwrap_or_default()
    }

    /// Grava de uma vez: temporário ao lado e troca.
    pub fn save_to(&self, file: &Path) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension(format!("toml.{}.tmp", std::process::id()));
        std::fs::write(&tmp, self.to_toml())?;
        std::fs::rename(&tmp, file).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    pub fn load() -> Self {
        path().map(|p| Self::load_from(&p)).unwrap_or_default()
    }

    /// Guarda uma escolha relendo o disco antes (outra janela pode ter gravado) e
    /// devolve a memória atualizada.
    /// Arquivo estragado não é sobrescrito sem volta: fica guardado em `.bak`.
    pub fn record_in(file: &Path, key: &str, view: View, as_default: bool) -> std::io::Result<Self> {
        let mut memory = match std::fs::read_to_string(file) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|| {
                let bak = file.with_extension("toml.bak");
                match std::fs::rename(file, &bak) {
                    Ok(()) => tracing::warn!("visualizações ilegíveis guardadas em {}", bak.display()),
                    Err(err) => tracing::warn!("visualizações ilegíveis e sem cópia: {err}"),
                }
                Self::default()
            }),
            Err(_) => Self::default(),
        };
        memory.remember(key, view, as_default);
        memory.save_to(file)?;
        Ok(memory)
    }

    pub fn record(key: &str, view: View, as_default: bool) -> std::io::Result<Self> {
        let file = path().ok_or_else(|| std::io::Error::other("sem pasta de estado"))?;
        Self::record_in(&file, key, view, as_default)
    }
}

/// `~/.local/state/mide/files-view.toml`, respeitando `XDG_STATE_HOME`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))?;
    Some(base.join("mide").join("files-view.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_BY_DATE: View = View { list: true, sort_column: 1, ascending: false, columns: false, icon_size: ICON_DEFAULT };

    #[test]
    fn pasta_ajustada_volta_e_as_outras_usam_o_padrao() {
        let mut m = ViewMemory::default();
        assert_eq!(m.view_for("/a"), View::default());
        m.remember("/a", LIST_BY_DATE, true);
        assert_eq!(m.view_for("/a"), LIST_BY_DATE);
        // Pasta nunca ajustada: a última escolha.
        assert_eq!(m.view_for("/nunca"), LIST_BY_DATE);
        // Outra pasta volta para ícones: o padrão muda, a /a continua em lista.
        m.remember("/b", View::default(), true);
        assert_eq!(m.view_for("/a"), LIST_BY_DATE);
        assert_eq!(m.view_for("/b"), View::default());
        assert_eq!(m.view_for("/nunca"), View::default());
    }

    #[test]
    fn lixeira_nao_muda_o_padrao() {
        let mut m = ViewMemory::default();
        m.remember(TRASH_KEY, LIST_BY_DATE, false);
        assert_eq!(m.view_for(TRASH_KEY), LIST_BY_DATE);
        assert_eq!(m.default_view(), View::default());
    }

    #[test]
    fn ida_e_volta_pelo_toml() {
        let mut m = ViewMemory::default();
        m.remember("/home/x/Fotos com espaço e ç", LIST_BY_DATE, true);
        m.remember("/b", View { list: false, sort_column: 2, ascending: true, columns: false, icon_size: ICON_DEFAULT }, true);
        let back = ViewMemory::from_toml(&m.to_toml());
        assert_eq!(back, m);
    }

    #[test]
    fn arquivo_estragado_ou_estranho_nao_quebra() {
        assert_eq!(ViewMemory::from_toml("isto não é toml ["), ViewMemory::default());
        let m = ViewMemory::from_toml(
            "[default]\nlist = true\nsort_column = 99\n[[folder]]\npath = \"/a\"\nlist = true\nsort_column = 7\nascending = true\nused = 3\n[[folder]]\npath = \"/a\"\nlist = false\nsort_column = 0\nascending = true\nused = 1\n",
        );
        // Coluna inválida vira Nome; a pasta repetida fica com o ajuste mais recente.
        assert_eq!(m.default_view(), View { list: true, sort_column: 0, ascending: true, columns: false, icon_size: ICON_DEFAULT });
        assert_eq!(m.view_for("/a"), View { list: true, sort_column: 0, ascending: true, columns: false, icon_size: ICON_DEFAULT });
        assert_eq!(m.len(), 1);
        // O contador continua depois do maior `used` lido.
        let mut m = m;
        m.remember("/b", View::default(), false);
        assert_eq!(m.folder.iter().find(|f| f.path == "/b").unwrap().used, 4);
    }

    #[test]
    fn limite_tira_as_mais_antigas() {
        let mut m = ViewMemory::default();
        for i in 0..LIMIT + 20 {
            m.remember(&format!("/p{i}"), LIST_BY_DATE, true);
        }
        assert_eq!(m.len(), LIMIT);
        assert!(m.folder.iter().all(|f| f.path != "/p0" && f.path != "/p19"));
        assert!(m.folder.iter().any(|f| f.path == "/p20"));
        // Ajustar de novo uma antiga a põe entre as recentes.
        m.remember("/p20", View::default(), false);
        for i in 0..10 {
            m.remember(&format!("/q{i}"), LIST_BY_DATE, true);
        }
        assert_eq!(m.view_for("/p20"), View::default());
        assert_eq!(m.view_for("/p21"), LIST_BY_DATE); // saiu: usa o padrão (lista)
        assert!(m.folder.iter().all(|f| f.path != "/p21"));
    }

    #[test]
    fn pastas_sumidas_continuam_lembradas_e_a_gravacao_e_segura() {
        let dir = std::env::temp_dir().join(format!("mide-files-view-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let keep = dir.join("fica");
        let gone = dir.join("pendrive");
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::create_dir_all(&gone).unwrap();
        let file = dir.join("estado").join("files-view.toml");
        ViewMemory::record_in(&file, &key_for_path(&gone), LIST_BY_DATE, true).unwrap();
        ViewMemory::record_in(&file, TRASH_KEY, LIST_BY_DATE, false).unwrap();
        // Pendrive desconectado: a escolha dele não pode sumir.
        std::fs::remove_dir(&gone).unwrap();
        let m = ViewMemory::record_in(&file, &key_for_path(&keep), View::default(), false).unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(m.view_for(&key_for_path(&gone)), LIST_BY_DATE);
        assert_eq!(ViewMemory::load_from(&file), m);
        // Nada de temporário sobrando ao lado.
        let names: Vec<_> = std::fs::read_dir(file.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arquivo_ilegivel_vira_bak_em_vez_de_sumir() {
        let dir = std::env::temp_dir().join(format!("mide-files-view-bak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("files-view.toml");
        std::fs::write(&file, "isto não é toml [").unwrap();
        let m = ViewMemory::record_in(&file, "/a", LIST_BY_DATE, true).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(std::fs::read_to_string(dir.join("files-view.toml.bak")).unwrap(), "isto não é toml [");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn campo_faltando_nao_perde_o_arquivo() {
        let m = ViewMemory::from_toml("[[folder]]\npath = \"/a\"\nlist = true\n[[folder]]\npath = \"/b\"\nused = 2\n");
        assert_eq!(m.len(), 2);
        assert_eq!(m.view_for("/a"), View { list: true, sort_column: 0, ascending: true, columns: false, icon_size: ICON_DEFAULT });
        assert_eq!(m.view_for("/b"), View::default());
    }

    #[test]
    fn largura_da_barra_lateral() {
        assert_eq!(ViewMemory::default().sidebar_width(), SIDEBAR_WIDTH);
        // Fora dos limites (arquivo editado à mão): fica dentro deles.
        assert_eq!(ViewMemory::from_toml("sidebar_width = 9000.0\n").sidebar_width(), SIDEBAR_MAX);
        assert_eq!(ViewMemory::from_toml("sidebar_width = 10.0\n").sidebar_width(), SIDEBAR_MIN);
        let dir = std::env::temp_dir().join(format!("mide-files-view-lateral-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("files-view.toml");
        // Guardar a largura não apaga as visualizações das pastas.
        ViewMemory::record_in(&file, "/a", LIST_BY_DATE, false).unwrap();
        ViewMemory::record_sidebar_width_in(&file, 271.4).unwrap();
        let m = ViewMemory::load_from(&file);
        assert_eq!(m.sidebar_width(), 271.0);
        assert_eq!(m.view_for("/a"), LIST_BY_DATE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chave_normalizada() {
        let k = |s: &str| key_for_path(Path::new(s));
        assert_eq!(k("/home/x/Downloads/"), "/home/x/Downloads");
        assert_eq!(k("/home//x/./Downloads"), "/home/x/Downloads");
        assert_eq!(k("/home/x/y/../Downloads"), "/home/x/Downloads");
        assert_eq!(k("/"), "/");
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(k("."), cwd.to_str().unwrap());
    }

    #[test]
    fn chave_sem_perda_fora_do_utf8() {
        use std::ffi::OsStr;
        let a = Path::new(OsStr::from_bytes(b"/tmp/\xff"));
        let b = Path::new(OsStr::from_bytes(b"/tmp/\xfe"));
        assert_eq!(key_for_path(a), "bytes:/tmp/%FF");
        assert_ne!(key_for_path(a), key_for_path(b));
        // Nome UTF-8 que parece escape não colide.
        assert_ne!(key_for_path(Path::new("/tmp/%FF")), key_for_path(a));
        assert_eq!(key_for_path(Path::new(OsStr::from_bytes(b"/a b%\xc3"))), "bytes:/a b%25%C3");
    }
}
