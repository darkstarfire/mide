//! Configurações do Sistema do mide, feitas com o mide-ui: barra lateral com busca e
//! as seções; à direita, cartões com as opções.
//!
//! Seções: Wi-Fi, Bluetooth, Som, Papel de parede, Dock, Efeitos, Teclado e Sobre.
//! O que é gravado vai para o `config.toml` (mantendo os comentários) e vale na hora:
//! o compositor relê a configuração (`reload`), e a dock e a barra percebem a mudança.
//!
//! Uso: `mide-settings [wifi|bluetooth|sound|wallpaper|dock|effects|keyboard|about]`.

use std::{
    collections::HashMap,
    error::Error,
    fs,
    path::PathBuf,
    sync::mpsc::{channel, Receiver, Sender},
    time::Duration,
};

use mide::{
    control_client::ask_compositor,
    displays::{self, Display},
    kit::{run, App, ArrangedScreen, Key, Tile, Ui, Window},
    settings_file::{self, Value},
    system::{self, Status},
};
use tiny_skia::Pixmap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Wifi,
    Bluetooth,
    Appearance,
    Sound,
    Displays,
    Wallpaper,
    Dock,
    Bar,
    Effects,
    Lock,
    Mouse,
    Keyboard,
    Region,
    Energy,
    DefaultApps,
    About,
}

/// Itens da barra lateral, na ordem: seção (ou espaço entre grupos), nome, ícone do
/// tema e cor do quadradinho.
const SIDEBAR: &[(Option<Section>, &str, &str, [u8; 3])] = &[
    (Some(Section::Wifi), "Wi-Fi", "network-wireless-symbolic", [10, 132, 255]),
    (Some(Section::Bluetooth), "Bluetooth", "bluetooth-symbolic", [10, 132, 255]),
    (None, "", "", [0, 0, 0]),
    (Some(Section::Appearance), "Aparência", "preferences-desktop-appearance-symbolic", [72, 72, 80]),
    (Some(Section::Sound), "Som", "audio-volume-high-symbolic", [255, 55, 95]),
    (Some(Section::Displays), "Monitores", "video-display-symbolic", [52, 120, 246]),
    (Some(Section::Wallpaper), "Papel de parede", "preferences-desktop-wallpaper-symbolic", [100, 210, 255]),
    (Some(Section::Dock), "Dock", "view-app-grid-symbolic", [52, 199, 89]),
    (Some(Section::Bar), "Barra superior", "window-new-symbolic", [88, 86, 214]),
    (Some(Section::Effects), "Efeitos", "starred-symbolic", [191, 90, 242]),
    (Some(Section::Lock), "Bloqueio de tela", "system-lock-screen-symbolic", [255, 59, 48]),
    (None, "", "", [0, 0, 0]),
    (Some(Section::Mouse), "Mouse e trackpad", "input-mouse-symbolic", [142, 142, 147]),
    (Some(Section::Keyboard), "Teclado", "input-keyboard-symbolic", [142, 142, 147]),
    (Some(Section::Region), "Idioma e região", "preferences-desktop-locale-symbolic", [10, 132, 255]),
    (Some(Section::Energy), "Energia", "battery-full-symbolic", [52, 199, 89]),
    (Some(Section::DefaultApps), "Apps padrão", "applications-system-symbolic", [142, 142, 147]),
    (Some(Section::About), "Sobre", "help-about-symbolic", [142, 142, 147]),
];

/// Efeitos do `[effects]`: chave, rótulo e explicação.
const EFFECTS: &[(&str, &str, &str)] = &[
    ("shadows", "Sombras das janelas", "Sombra suave em volta de cada janela"),
    ("rounded_corners", "Cantos arredondados", "Janelas com cantos arredondados e contorno fino"),
    ("animations", "Animações", "Maximizar, restaurar, fechar janelas e abrir o launcher com transição suave"),
    ("lamp", "Efeito lâmpada", "Minimizar escorre a janela até o ícone da dock"),
    ("dock_blur", "Desfoque na dock", "A dock desfoca o que está atrás dela, como vidro fosco"),
    ("launcher_blur", "Desfoque no launcher", "O fundo do launcher é o papel de parede desfocado"),
    ("liquid_glass", "Vidro líquido", "Dock e pastas do launcher com bordas que refratam a luz e reflexo, sobre o desfoque"),
    ("panel_glass", "Vidro em barras e menus", "Barra superior, menus, notificações e prévias das janelas com vidro"),
    ("menu_animation", "Animação dos menus", "Menus e listas aparecem com um esmaecer curto"),
    ("transitions", "Transições suaves", "Prévias da dock e Alt+Tab, páginas e pastas do launcher, pastas do Arquivos, troca entre ícones e lista e a miniatura da captura"),
    ("shake_cursor", "Agitar o ponteiro para localizá-lo", "Mexer o mouse rápido de um lado para o outro deixa o ponteiro grande por um instante"),
];

/// O que a busca encontra: nome da opção, palavras-chave e a seção dela.
const SEARCHABLE: &[(&str, &str, Section)] = &[
    ("Bloquear a tela após", "bloqueio bloquear senha inatividade tempo sem uso protetor de tela", Section::Lock),
    ("Bloquear ao suspender", "bloqueio suspender dormir senha", Section::Lock),
    ("Apagar a tela após", "apagar tela monitor desligar energia dpms protetor", Section::Lock),
    ("Aparência", "tema claro escuro modo noturno dark cores", Section::Appearance),
    ("Wi-Fi", "rede internet wireless conectar senha", Section::Wifi),
    ("Bluetooth", "fone mouse teclado aparelhos parear sem fio", Section::Bluetooth),
    ("Volume de saída", "som audio alto-falante volume", Section::Sound),
    ("Sem som", "mudo silenciar som", Section::Sound),
    ("Efeitos sonoros", "som efeitos alerta bip notificacao sons do sistema", Section::Sound),
    ("Monitores", "tela telas monitor resolucao frequencia hz escala organizar principal segundo monitor", Section::Displays),
    ("Resolução e frequência da tela", "monitor resolucao frequencia hz taxa atualizacao", Section::Displays),
    ("Escala da tela", "monitor escala tamanho zoom hidpi", Section::Displays),
    ("Papel de parede", "fundo imagem wallpaper area de trabalho", Section::Wallpaper),
    ("Adicionar imagem de fundo", "papel de parede imagem foto", Section::Wallpaper),
    ("Tamanho dos ícones da dock", "dock icones tamanho", Section::Dock),
    ("Ocultar e mostrar a Dock automaticamente", "dock esconder ocultar automatico", Section::Dock),
    ("Clicar no app em foco minimiza", "dock minimizar clique foco previas janelas", Section::Dock),
    ("Itens da barra superior", "barra menu topo icones som wifi bluetooth bateria central controle engrenagem", Section::Bar),
    ("Sombras das janelas", "efeitos sombra", Section::Effects),
    ("Cantos arredondados", "efeitos cantos bordas", Section::Effects),
    ("Animações", "efeitos animacao transicao", Section::Effects),
    ("Efeito lâmpada", "efeitos minimizar genie", Section::Effects),
    ("Desfoque na dock", "efeitos blur desfoque vidro", Section::Effects),
    ("Desfoque no launcher", "efeitos blur desfoque launcher", Section::Effects),
    ("Vidro líquido", "efeitos vidro liquid glass transparencia", Section::Effects),
    ("Vidro em barras e menus", "efeitos vidro barra menus notificacoes transparencia", Section::Effects),
    ("Animação dos menus", "efeitos animacao menus contexto esmaecer", Section::Effects),
    ("Transições suaves", "efeitos animacao transicao suave launcher pastas arquivos lista icones", Section::Effects),
    ("Velocidade do ponteiro", "mouse trackpad touchpad ponteiro velocidade cursor", Section::Mouse),
    ("Rolagem natural", "mouse trackpad touchpad rolagem natural scroll invertida", Section::Mouse),
    ("Toque para clicar", "trackpad touchpad toque clicar tap", Section::Mouse),
    ("Aceleração do mouse", "mouse aceleracao ponteiro precisao", Section::Mouse),
    ("Layout do teclado", "teclado layout abnt2 abnt idioma us internacional acentos", Section::Keyboard),
    ("Teclas no estilo Mac", "teclado command option cmd super ctrl atalhos", Section::Keyboard),
    ("Atalhos de teclado", "teclado atalhos teclas combinacoes", Section::Keyboard),
    ("Mudar ou criar atalho", "teclado atalho novo gravar combinacao remover padrao", Section::Keyboard),
    ("Idioma", "idioma lingua language traducao portugues ingles locale", Section::Region),
    ("Formatos de data e números", "regiao formato data hora numeros moeda locale", Section::Region),
    ("Suspender o computador após", "energia suspender dormir repouso inatividade bateria", Section::Energy),
    ("Botão de ligar", "energia botao ligar desligar power suspender", Section::Energy),
    ("Apps padrão", "apps padrao navegador email abrir com pdf imagens videos musica pastas texto", Section::DefaultApps),
    ("Sobre este computador", "sistema versao memoria processador", Section::About),
];


const DOCK_MIN: f32 = 32.0;
const DOCK_MAX: f32 = 64.0;
const DOCK_DEFAULT: i64 = 40;
/// Miniaturas dos papéis de parede e do seletor de imagens.
const THUMB: (u32, u32) = (120, 75);

/// Miniatura de um papel de parede (`None` = papel de parede padrão do mide).
type Thumb = (Option<PathBuf>, Option<Pixmap>);

/// Seletor de imagens ("Adicionar imagem…"): pasta, itens e miniaturas.
struct Picker {
    dir: PathBuf,
    entries: Vec<(PathBuf, bool)>,
    thumbs: HashMap<PathBuf, Option<Pixmap>>,
    rx: Receiver<(PathBuf, Option<Pixmap>)>,
}

struct Settings {
    section: Section,
    query: String,
    status: Status,
    status_rx: Receiver<Status>,
    wallpapers: Vec<Option<PathBuf>>,
    thumbs: HashMap<Option<PathBuf>, Option<Pixmap>>,
    thumbs_tx: Sender<Thumb>,
    thumbs_rx: Receiver<Thumb>,
    picker: Option<Picker>,
    bindings: Vec<(String, String)>,
    /// Apps instalados (para criar atalhos que abrem um app e mostrar o nome deles).
    apps: Vec<mide::apps::App>,
    /// Diálogo de mudar ou criar um atalho.
    shortcut: Option<ShortcutEditor>,
    info: Vec<(&'static str, String)>,
    volume: f32,
    dock_size: f32,
    /// A busca acabou de ser limpa (um resultado foi escolhido).
    query_reset: bool,
    /// Telas (Monitores): como estão na tela agora e como o usuário está deixando.
    screens_applied: Vec<Display>,
    screens: Vec<Display>,
    screen_selected: usize,
    /// Mudanças aplicadas esperando confirmação: quando foram aplicadas e as anteriores.
    screens_revert: Option<(std::time::Instant, Vec<Display>)>,
    /// Busca na lista de atalhos.
    shortcut_query: String,
    /// Idiomas instalados (`locale -a`), lidos ao abrir Idioma e região.
    locales: Vec<String>,
    /// Grupos de atalhos recolhidos (Configurações → Teclado).
    closed_groups: Vec<String>,
}

impl Settings {
    fn new(section: Section) -> Self {
        // Som, Wi-Fi e Bluetooth a cada 2 s, numa thread (os comandos levam tempo).
        let (status_tx, status_rx) = channel();
        std::thread::spawn(move || loop {
            if status_tx.send(system::read_status()).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        });
        let (thumbs_tx, thumbs_rx) = channel();
        let wallpapers = wallpaper_list();
        load_thumbs(thumbs_tx.clone(), wallpapers.clone());
        let mut app = Self {
            section,
            query: String::new(),
            status: Status::default(),
            status_rx,
            wallpapers,
            thumbs: HashMap::new(),
            thumbs_tx,
            thumbs_rx,
            picker: None,
            bindings: Vec::new(),
            apps: Vec::new(),
            shortcut: None,
            info: mide::sysinfo::system_info(),
            query_reset: false,
            screens_applied: Vec::new(),
            screens: Vec::new(),
            screen_selected: 0,
            screens_revert: None,
            shortcut_query: String::new(),
            locales: Vec::new(),
            closed_groups: match settings_file::get("settings", "shortcuts_closed") {
                Some(Value::Text(t)) => t.split(',').filter(|g| !g.is_empty()).map(str::to_string).collect(),
                _ => Vec::new(),
            },
            volume: 0.0,
            dock_size: match settings_file::get("dock", "icon_size") {
                Some(Value::Int(n)) => (n as f32).clamp(DOCK_MIN, DOCK_MAX),
                _ => DOCK_DEFAULT as f32,
            },
        };
        app.load_bindings();
        app
    }

    fn load_bindings(&mut self) {
        if self.apps.is_empty() {
            self.apps = mide::apps::load();
        }
        self.bindings = ask_compositor("bindings")
            .unwrap_or_default()
            .iter()
            .filter_map(|l| l.split_once('\t').map(|(c, a)| (c.to_string(), a.to_string())))
            .collect();
    }

    /// Recebe o que as threads mandaram (estado do sistema, miniaturas).
    fn poll(&mut self, ui: &mut Ui) {
        while let Ok(status) = self.status_rx.try_recv() {
            if status != self.status {
                if let Some((v, _)) = status.volume {
                    self.volume = v * 100.0;
                }
                self.status = status;
            }
        }
        let mut loading = false;
        while let Ok((path, thumb)) = self.thumbs_rx.try_recv() {
            self.thumbs.insert(path, thumb);
        }
        if self.thumbs.len() < self.wallpapers.len() {
            loading = true;
        }
        if let Some(picker) = self.picker.as_mut() {
            while let Ok((path, thumb)) = picker.rx.try_recv() {
                picker.thumbs.insert(path, thumb);
            }
            let images = picker.entries.iter().filter(|(_, d)| !d).count();
            loading |= picker.thumbs.len() < images.min(300);
        }
        if loading {
            ui.request_tick();
        }
        // O estado do sistema chega a cada 2 s.
        ui.wake_after(Duration::from_secs(2));
    }

    fn open_picker(&mut self, dir: PathBuf) {
        let entries = mide::places::folder_images(&dir);
        tracing::info!("seletor de imagens: {} ({} item(ns))", dir.display(), entries.len());
        let images: Vec<PathBuf> = entries.iter().filter(|(_, d)| !d).map(|(p, _)| p.clone()).take(300).collect();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for path in images {
                let thumb = image::open(&path).ok().map(|img| to_pixmap(&mide::backdrop::cover(&img.to_rgba8(), THUMB.0, THUMB.1)));
                if tx.send((path, thumb)).is_err() {
                    break;
                }
            }
        });
        self.picker = Some(Picker { dir, entries, thumbs: HashMap::new(), rx });
    }

    fn refresh_wallpapers(&mut self) {
        self.wallpapers = wallpaper_list();
        let missing: Vec<Option<PathBuf>> = self.wallpapers.iter().filter(|p| !self.thumbs.contains_key(*p)).cloned().collect();
        load_thumbs(self.thumbs_tx.clone(), missing);
    }

    fn set_wallpaper(&self, path: Option<PathBuf>) {
        let value = path.as_ref().map(|p| Value::Text(p.to_string_lossy().into_owned()));
        match settings_file::set("desktop", "wallpaper", value) {
            Ok(()) => {
                tracing::info!("papel de parede escolhido: {}", path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "padrão do mide".into()));
                ask_compositor("reload");
            }
            Err(err) => tracing::warn!("papel de parede não gravado: {err}"),
        }
    }

    // ----- Seções -----

    fn wifi(&mut self, ui: &mut Ui) {
        ui.title("Wi-Fi");
        let (on, networks) = self.status.wifi.clone().unwrap_or((false, Vec::new()));
        let mut value = on;
        ui.card(|ui| {
            if ui.switch("Wi-Fi", &mut value).changed() {
                system::set_wifi(value);
            }
        });
        if value != on {
            if let Some(wifi) = self.status.wifi.as_mut() {
                wifi.0 = value;
            }
        }
        if !on {
            return;
        }
        ui.heading("Redes");
        ui.card(|ui| {
            if networks.is_empty() {
                ui.note("Procurando redes…");
            }
            for net in networks.iter().take(10) {
                let detail = if net.active { "Conectado".to_string() } else { format!("{}%{}", net.signal, if net.secure { " · com senha" } else { "" }) };
                if ui.list_item(&net.ssid, &detail).clicked() && !net.active {
                    system::connect_network(&net.ssid);
                }
            }
        });
    }

    fn bluetooth(&mut self, ui: &mut Ui) {
        ui.title("Bluetooth");
        let (on, devices) = self.status.bluetooth.clone().unwrap_or((false, Vec::new()));
        let mut value = on;
        ui.card(|ui| {
            let detail = if on { "Visível para os aparelhos pareados" } else { "Desligado" };
            if ui.switch_detail("Bluetooth", detail, &mut value).changed() {
                system::set_bluetooth(value);
            }
        });
        if value != on {
            if let Some(bt) = self.status.bluetooth.as_mut() {
                bt.0 = value;
            }
        }
        if !on {
            return;
        }
        ui.heading("Meus aparelhos");
        ui.card(|ui| {
            if devices.is_empty() {
                ui.note("Nenhum aparelho pareado");
            }
            for d in &devices {
                let detail = if d.connected { "Conectado · clique para desconectar" } else { "Não conectado · clique para conectar" };
                if ui.list_item(&d.name, detail).clicked() {
                    system::toggle_device(d);
                }
            }
        });
    }

    /// Monitores: arranjo das telas (arrastar), tela principal, ligar e desligar,
    /// resolução, frequência e escala. "Aplicar" grava e o compositor muda na hora;
    /// sem confirmar em 15 s, volta ao que era (se a tela ficar preta, basta esperar).
    fn displays(&mut self, ui: &mut Ui) {
        ui.title("Monitores");
        if self.screens_applied.is_empty() {
            self.screens_applied = displays::load();
            self.screens = self.screens_applied.clone();
            self.screen_selected = self.screens.iter().position(|d| d.primary).unwrap_or(0);
        }
        if self.screens.is_empty() {
            ui.note("O mide não informou as telas (rodando numa janela, a tela é a da janela).");
            return;
        }
        // Arranjo: só as telas ligadas.
        let on: Vec<usize> = (0..self.screens.len()).filter(|&i| self.screens[i].enabled).collect();
        let mut arranged: Vec<ArrangedScreen> = on
            .iter()
            .map(|&i| {
                let d = &self.screens[i];
                ArrangedScreen { name: d.name.clone(), pos: d.position, size: d.logical_size(), primary: d.primary, mm: d.physical_mm() }
            })
            .collect();
        let mut pick = on.iter().position(|&i| i == self.screen_selected).unwrap_or(0);
        let r = ui.arrangement(&mut arranged, &mut pick);
        if r.clicked || r.changed {
            if let Some(&i) = on.get(pick) {
                self.screen_selected = i;
            }
        }
        if r.changed {
            for (a, &i) in arranged.iter().zip(&on) {
                self.screens[i].position = a.pos;
            }
            tracing::info!("monitores: {} movida para ({}, {})", arranged[pick].name, arranged[pick].pos.0, arranged[pick].pos.1);
        }
        ui.note("Arraste as telas para organizar. A faixa branca marca a tela principal, com a barra e a dock.");
        // Aplicar logo abaixo do arranjo (à vista, sem rolar).
        let changed = self.screens != self.screens_applied;
        if changed && self.screens_revert.is_none() {
            ui.row(|ui| {
                if ui.button("Desfazer").clicked() {
                    self.screens = self.screens_applied.clone();
                }
                if ui.primary_button("Aplicar").clicked() {
                    match displays::save(&self.screens) {
                        Ok(()) => {
                            ask_compositor("outputs apply");
                            tracing::info!("monitores: aplicado");
                            self.screens_revert = Some((std::time::Instant::now(), self.screens_applied.clone()));
                            self.screens_applied = self.screens.clone();
                        }
                        Err(err) => tracing::warn!("monitores não gravados: {err}"),
                    }
                }
            });
        }

        let names: Vec<String> = self
            .screens
            .iter()
            .map(|d| if d.description.is_empty() { d.name.clone() } else { format!("{} ({})", d.name, d.description) })
            .collect();
        let others_on = self.screens.iter().enumerate().filter(|(i, d)| *i != self.screen_selected && d.enabled).count();
        ui.card(|ui| {
            if names.len() > 1 {
                ui.dropdown("Tela", &names, &mut self.screen_selected);
            }
            let Some(d) = self.screens.get_mut(self.screen_selected) else {
                return;
            };
            let mut on = d.enabled;
            if ui.switch_detail("Ligada", "Desligada, a tela fica apagada e o mide não a usa", &mut on).changed() {
                // A última tela ligada não desliga.
                d.enabled = on || others_on == 0;
            }
            if !d.enabled {
                return;
            }
            if d.primary {
                ui.list_item("Tela principal", "barra, dock e launcher");
            } else if ui.button("Usar como tela principal").clicked() {
                let name = d.name.clone();
                for other in self.screens.iter_mut() {
                    other.primary = other.name == name;
                }
                return;
            }
            let Some(d) = self.screens.get_mut(self.screen_selected) else {
                return;
            };
            let sizes = d.sizes();
            let mut size_i = sizes.iter().position(|s| *s == (d.current.0, d.current.1)).unwrap_or(0);
            let labels: Vec<String> = sizes.iter().map(|s| displays::size_label(*s)).collect();
            if ui.dropdown("Resolução", &labels, &mut size_i).changed() {
                let size = sizes[size_i];
                let hz = d.rates(size).first().copied().unwrap_or(60.0);
                d.current = (size.0, size.1, hz);
            }
            let rates = d.rates((d.current.0, d.current.1));
            let mut rate_i = rates.iter().position(|r| (r - d.current.2).abs() < 0.5).unwrap_or(0);
            let labels: Vec<String> = rates.iter().map(|r| displays::rate_label(*r)).collect();
            if ui.dropdown("Frequência", &labels, &mut rate_i).changed() {
                d.current.2 = rates[rate_i];
            }
            // Tamanho físico (o arranjo desenha as telas do tamanho de verdade): o que o
            // monitor informa, ou a diagonal escolhida para quem não informa direito.
            let auto = match d.reported_mm() {
                Some((w, h)) => format!("Automático ({:.0}\")", ((w * w + h * h).sqrt() / 25.4).round()),
                None => "Automático (estimado)".to_string(),
            };
            let mut sizes_label = vec![auto];
            sizes_label.extend(displays::DIAGONALS.iter().map(|d| format!("{}\"", d.to_string().replace('.', ","))));
            let mut diag_i = d.diagonal.and_then(|v| displays::DIAGONALS.iter().position(|x| (x - v).abs() < 0.05)).map(|i| i + 1).unwrap_or(0);
            if ui.dropdown("Tamanho da tela", &sizes_label, &mut diag_i).changed() {
                d.diagonal = (diag_i > 0).then(|| displays::DIAGONALS[diag_i - 1]);
            }
            let mut scale_i = displays::SCALES.iter().position(|s| (s - d.scale).abs() < 0.01).unwrap_or(0);
            let labels: Vec<String> = displays::SCALES.iter().map(|s| displays::scale_label(*s)).collect();
            if ui.dropdown("Escala", &labels, &mut scale_i).changed() {
                d.scale = displays::SCALES[scale_i];
            }
        });
        // Espelhar: todas as telas mostram a principal (vale na hora).
        if self.screens.len() >= 2 {
            let mut mirror = matches!(settings_file::get("monitors", "mirror"), Some(Value::Bool(true)));
            ui.card(|ui| {
                if ui.switch_detail("Espelhar telas", "Todas as telas mostram a principal, com a barra e a dock dela", &mut mirror).changed() {
                    match settings_file::set("monitors", "mirror", mirror.then_some(Value::Bool(true))) {
                        Ok(()) => {
                            tracing::info!("monitores: espelhar {}", if mirror { "ligado" } else { "desligado" });
                            ask_compositor("outputs apply");
                            // O arranjo novo vem do compositor no próximo quadro.
                            self.screens_applied.clear();
                        }
                        Err(err) => tracing::warn!("espelhar não gravado: {err}"),
                    }
                }
            });
        }
    }

    /// Depois de aplicar as telas: janelinha (modal) "Manter estas configurações?", por
    /// cima da janela inteira. Sem confirmar em 15 s, volta ao que era (se a tela
    /// ficar preta, basta esperar).
    fn displays_confirm(&mut self, ui: &mut Ui) {
        const KEEP_SECS: u64 = 15;
        let Some((at, previous)) = self.screens_revert.clone() else {
            return;
        };
        let left = KEEP_SECS.saturating_sub(at.elapsed().as_secs());
        let mut keep = false;
        let mut revert = left == 0;
        ui.modal(360.0, |ui| {
            ui.heading("Manter estas configurações?");
            ui.note(&format!("As telas voltam ao que eram em {left} s."));
            ui.row(|ui| {
                revert |= ui.button("Reverter").clicked();
                keep = ui.primary_button("Manter").clicked();
            });
        });
        if keep {
            tracing::info!("monitores: mantido");
            self.screens_revert = None;
            self.screens_applied = displays::load();
            self.screens = self.screens_applied.clone();
        } else if revert {
            tracing::info!("monitores: revertido");
            if displays::save(&previous).is_ok() {
                ask_compositor("outputs apply");
            }
            self.screens_revert = None;
            self.screens_applied = Vec::new();
        } else {
            ui.wake_after(Duration::from_millis(500));
        }
    }

    fn sound(&mut self, ui: &mut Ui) {
        ui.title("Som");
        ui.heading("Saída");
        let (_, muted) = self.status.volume.unwrap_or((0.0, false));
        let mut mute = muted;
        let mut volume = self.volume;
        ui.card(|ui| {
            let r = ui.slider("Volume de saída", &mut volume, 0.0, 100.0);
            // Grava só ao soltar o trilho (cada mudança chama um comando do sistema).
            if r.submitted() {
                system::set_volume(volume / 100.0);
                if muted {
                    system::set_muted(false);
                }
            }
            if ui.switch("Sem som", &mut mute).changed() {
                system::set_muted(mute);
            }
        });
        self.volume = volume;
        if mute != muted {
            if let Some(v) = self.status.volume.as_mut() {
                v.1 = mute;
            }
        }
        // Efeitos sonoros do mide (`[sounds]`): ligados, volume e o som de alerta.
        ui.heading("Efeitos sonoros");
        let mut on = mide::sounds::enabled();
        let mut effects = mide::sounds::volume() * 100.0;
        let names: Vec<String> = mide::sounds::ALERTS.iter().map(|(_, n)| n.to_string()).collect();
        let current = mide::sounds::alert_key();
        let mut alert_i = mide::sounds::ALERTS.iter().position(|(k, _)| *k == current).unwrap_or(0);
        ui.card(|ui| {
            if ui.switch_detail("Tocar efeitos sonoros", "Notificações, captura de tela, Lixeira, volume e avisos", &mut on).changed() {
                match settings_file::set("sounds", "enabled", (!on).then_some(Value::Bool(false))) {
                    Ok(()) => {
                        tracing::info!("efeitos sonoros: {}", if on { "ligados" } else { "desligados" });
                        // Os apps GTK seguem (sons de evento no dconf da sessão).
                        mide::appearance::apply(false);
                    }
                    Err(err) => tracing::warn!("efeitos sonoros não gravados: {err}"),
                }
            }
            if ui.slider("Volume dos efeitos", &mut effects, 0.0, 100.0).submitted() {
                let value = effects.round() as i64;
                tracing::info!("efeitos sonoros: volume {value}");
                if settings_file::set("sounds", "volume", (value != 70).then_some(Value::Int(value))).is_ok() {
                    mide::sounds::play("audio-volume-change");
                }
            }
            if ui.dropdown("Som de alerta", &names, &mut alert_i).changed() {
                let (key, name) = mide::sounds::ALERTS[alert_i];
                tracing::info!("efeitos sonoros: alerta {name}");
                if settings_file::set("sounds", "alert", (key != "vidro").then(|| Value::Text(key.into()))).is_ok() {
                    mide::sounds::preview_alert(key);
                }
            }
        });
        ui.note("O Não perturbe silencia o som das notificações. Os apps GTK abertos depois também usam estes sons.");
    }

    fn wallpaper(&mut self, ui: &mut Ui) {
        if self.picker.is_some() {
            self.picker_view(ui);
            return;
        }
        ui.title("Papel de parede");
        let current = current_wallpaper();
        let mut chosen = None;
        let mut remove = None;
        let mut add = false;
        let wallpapers = self.wallpapers.clone();
        ui.wrap(|ui| {
            for path in &wallpapers {
                let name = match path {
                    None => "Padrão do mide".to_string(),
                    Some(p) => p.file_stem().map(|s| s.to_string_lossy().replace(['-', '_'], " ")).unwrap_or_default(),
                };
                let thumb = self.thumbs.get(path).cloned().flatten();
                let image = match &thumb {
                    Some(pix) => Tile::Image(pix),
                    None => Tile::Icon("image-x-generic"),
                };
                let r = ui.tile_wide(image, &name, *path == current, THUMB.0, THUMB.1);
                if r.clicked() {
                    chosen = Some(path.clone());
                }
                // Imagens adicionadas pelo usuário: menu para tirar da lista.
                if let Some(p) = path.as_ref().filter(|p| p.starts_with(mide::places::user_backgrounds_dir())) {
                    ui.context_menu(&r, |m| {
                        if m.danger("Remover da lista") {
                            remove = Some(p.clone());
                        }
                    });
                }
            }
            add = ui.tile_wide(Tile::Icon("list-add"), "Adicionar imagem…", false, THUMB.0, THUMB.1).clicked();
        });
        ui.note("Clique com o botão direito numa imagem adicionada para tirá-la da lista.");
        if let Some(path) = chosen {
            self.set_wallpaper(path);
        }
        if let Some(path) = remove {
            if current.as_ref() == Some(&path) {
                self.set_wallpaper(None);
            }
            match fs::remove_file(&path) {
                Ok(()) => tracing::info!("papel de parede removido da lista: {}", path.display()),
                Err(err) => tracing::warn!("não deu para remover {}: {err}", path.display()),
            }
            self.refresh_wallpapers();
        }
        if add {
            self.open_picker(mide::places::pictures_dir());
        }
    }

    /// Seletor de imagens: lugares, a pasta atual (pastas e imagens) e Cancelar.
    fn picker_view(&mut self, ui: &mut Ui) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let dir = picker.dir.clone();
        let entries = picker.entries.clone();
        let thumbs = picker.thumbs.clone();
        ui.title("Escolher uma imagem");
        let mut go = None;
        let mut choose = None;
        let mut cancel = false;
        ui.row(|ui| {
            for (name, place) in [("Imagens", mide::places::pictures_dir()), ("Downloads", mide::places::downloads_dir()), ("Pasta pessoal", mide::places::home_dir())] {
                let r = if place == dir { ui.primary_button(name) } else { ui.button(name) };
                if r.clicked() {
                    go = Some(place);
                }
            }
            if ui.button("‹ Voltar").clicked() {
                go = dir.parent().map(|p| p.to_path_buf());
            }
            cancel = ui.button("Cancelar").clicked();
        });
        ui.note(&dir.display().to_string());
        if entries.is_empty() {
            ui.note("Nenhuma imagem nesta pasta.");
        }
        ui.wrap(|ui| {
            for (path, is_dir) in &entries {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let thumb = thumbs.get(path).cloned().flatten();
                let image = match (&thumb, is_dir) {
                    (_, true) => Tile::Icon("folder"),
                    (Some(pix), false) => Tile::Image(pix),
                    (None, false) => Tile::Icon("image-x-generic"),
                };
                if ui.tile_wide(image, &name, false, THUMB.0, THUMB.1).clicked() {
                    if *is_dir {
                        go = Some(path.clone());
                    } else {
                        choose = Some(path.clone());
                    }
                }
            }
        });
        if cancel {
            self.picker = None;
        } else if let Some(dir) = go {
            self.open_picker(dir);
        } else if let Some(image) = choose {
            self.picker = None;
            match mide::places::add_background(&image) {
                Ok(copy) => {
                    tracing::info!("papel de parede adicionado: {}", copy.display());
                    self.refresh_wallpapers();
                    self.set_wallpaper(Some(copy));
                }
                Err(err) => tracing::warn!("não deu para adicionar {}: {err}", image.display()),
            }
        }
    }

    fn dock(&mut self, ui: &mut Ui) {
        ui.title("Dock");
        let mut size = self.dock_size;
        let mut autohide = matches!(settings_file::get("dock", "autohide"), Some(Value::Bool(true)));
        ui.card(|ui| {
            if ui.slider("Tamanho dos ícones", &mut size, DOCK_MIN, DOCK_MAX).submitted() {
                let n = size.round() as i64;
                let value = (n != DOCK_DEFAULT).then_some(Value::Int(n));
                match settings_file::set("dock", "icon_size", value) {
                    Ok(()) => tracing::info!("tamanho dos ícones da dock: {n} px"),
                    Err(err) => tracing::warn!("tamanho da dock não gravado: {err}"),
                }
            }
            let detail = "A dock some e aparece ao levar o ponteiro até a borda de baixo";
            if ui.switch_detail("Ocultar e mostrar a Dock automaticamente", detail, &mut autohide).changed() {
                match settings_file::set("dock", "autohide", autohide.then_some(Value::Bool(true))) {
                    Ok(()) => tracing::info!("dock: ocultar automaticamente {}", if autohide { "ligado" } else { "desligado" }),
                    Err(err) => tracing::warn!("opção da dock não gravada: {err}"),
                }
            }
            // Ampliação (como no Mac): os ícones crescem sob o ponteiro.
            let mut magnify = settings_file::flag("dock", "magnification", false);
            if ui.switch_detail("Ampliação", "Os ícones crescem quando o ponteiro passa sobre eles", &mut magnify).changed() {
                match settings_file::set("dock", "magnification", magnify.then_some(Value::Bool(true))) {
                    Ok(()) => tracing::info!("dock: ampliação {}", if magnify { "ligada" } else { "desligada" }),
                    Err(err) => tracing::warn!("opção da dock não gravada: {err}"),
                }
            }
            if magnify {
                let mut zoom = match settings_file::get("dock", "magnify_size") {
                    Some(Value::Int(n)) => n as f32,
                    _ => 96.0,
                };
                if ui.slider("Tamanho da ampliação", &mut zoom, 56.0, 128.0).submitted() {
                    let n = zoom.round() as i64;
                    match settings_file::set("dock", "magnify_size", (n != 96).then_some(Value::Int(n))) {
                        Ok(()) => tracing::info!("dock: ampliação até {n} px"),
                        Err(err) => tracing::warn!("opção da dock não gravada: {err}"),
                    }
                }
            }
        });
        let mut click_minimize = settings_file::flag("dock", "click_minimize", true);
        ui.card(|ui| {
            let detail = "Com uma janela, minimiza; com várias, mostra as prévias para escolher";
            if ui.switch_detail("Clicar no app em foco minimiza", detail, &mut click_minimize).changed() {
                match settings_file::set("dock", "click_minimize", (!click_minimize).then_some(Value::Bool(false))) {
                    Ok(()) => tracing::info!("dock: clicar no app em foco minimiza {}", if click_minimize { "ligado" } else { "desligado" }),
                    Err(err) => tracing::warn!("opção da dock não gravada: {err}"),
                }
            }
        });
        self.dock_size = size;
        ui.note("Sem ocultar, as janelas maximizadas ficam acima da dock. Para fixar um app, clique com o botão direito no ícone dele e escolha \"Manter na Dock\".");
    }

    /// Bloqueio de tela (`[lock]`): depois de quanto tempo sem uso e ao suspender.
    fn lock(&mut self, ui: &mut Ui) {
        ui.title("Bloqueio de tela");
        const MINUTES: [i64; 7] = [0, 1, 2, 5, 10, 15, 30];
        let current = match settings_file::get("lock", "idle_minutes") {
            Some(Value::Int(n)) => n,
            _ => 5,
        };
        let options: Vec<String> = MINUTES.iter().map(|m| match m {
            0 => "Nunca".to_string(),
            1 => "1 minuto".to_string(),
            n => format!("{n} minutos"),
        }).collect();
        let mut index = MINUTES.iter().position(|m| *m == current).unwrap_or(3);
        const BLANK: [i64; 8] = [0, 1, 2, 5, 10, 15, 30, 60];
        let blank_current = match settings_file::get("lock", "blank_minutes") {
            Some(Value::Int(n)) => n,
            _ => 10,
        };
        let blank_options: Vec<String> = BLANK.iter().map(|m| match m {
            0 => "Nunca".to_string(),
            1 => "1 minuto".to_string(),
            60 => "1 hora".to_string(),
            n => format!("{n} minutos"),
        }).collect();
        let mut blank_index = BLANK.iter().position(|m| *m == blank_current).unwrap_or(4);
        let mut on_suspend = settings_file::flag("lock", "on_suspend", true);
        ui.card(|ui| {
            if ui.dropdown("Bloquear a tela após ficar sem uso", &options, &mut index).changed() {
                let n = MINUTES[index];
                match settings_file::set("lock", "idle_minutes", (n != 5).then_some(Value::Int(n))) {
                    Ok(()) => tracing::info!("bloqueio: após {n} min sem uso"),
                    Err(err) => tracing::warn!("opção do bloqueio não gravada: {err}"),
                }
            }
            if ui.dropdown("Apagar a tela após ficar sem uso", &blank_options, &mut blank_index).changed() {
                let n = BLANK[blank_index];
                match settings_file::set("lock", "blank_minutes", (n != 10).then_some(Value::Int(n))) {
                    Ok(()) => tracing::info!("apagar a tela após {n} min sem uso"),
                    Err(err) => tracing::warn!("opção do bloqueio não gravada: {err}"),
                }
            }
            if ui.switch_detail("Bloquear ao suspender", "Ao voltar da suspensão, a senha é pedida", &mut on_suspend).changed() {
                match settings_file::set("lock", "on_suspend", (!on_suspend).then_some(Value::Bool(false))) {
                    Ok(()) => tracing::info!("bloqueio ao suspender {}", if on_suspend { "ligado" } else { "desligado" }),
                    Err(err) => tracing::warn!("opção do bloqueio não gravada: {err}"),
                }
            }
        });
        if ui.primary_button("Bloquear agora").clicked() {
            if std::env::var_os("MIDE_SYSTEM_DRY_RUN").is_some() {
                tracing::info!("sistema (teste): mide-lock");
            } else if let Err(err) = std::process::Command::new("mide-lock").spawn() {
                tracing::warn!("não foi possível abrir a tela de bloqueio: {err}");
            }
        }
        ui.note("Super+L também bloqueia. Com a tela bloqueada, ela apaga depois de 1 minuto. Vídeos tocando (no navegador, no mpv) impedem o bloqueio e o apagar. Num mide aberto em janela (desenvolvimento), quem bloqueia por tempo sem uso é o desktop de baixo.");
    }

    /// Itens da direita da barra superior (`[bar]`): todos à mostra por padrão.
    fn bar(&mut self, ui: &mut Ui) {
        ui.title("Barra superior");
        ui.card(|ui| {
            for (key, label, detail) in settings_file::BAR_ITEMS {
                let mut on = settings_file::bar_item(key);
                if ui.switch_detail(label, detail, &mut on).changed() {
                    // À mostra é o padrão: grava só quando esconde.
                    let value = if on { None } else { Some(Value::Bool(false)) };
                    match settings_file::set("bar", key, value) {
                        Ok(()) => tracing::info!("barra superior: {label} {}", if on { "à mostra" } else { "escondido" }),
                        Err(err) => tracing::warn!("item da barra não gravado: {err}"),
                    }
                }
            }
        });
        ui.note("O relógio fica sempre na barra. Itens escondidos continuam nas Configurações e na central de controle.");
    }

    /// Claro ou escuro (`[appearance] theme`): os apps do mide mudam na hora; os apps
    /// GTK e Qt abertos depois também (`mide::appearance`).
    fn appearance(&mut self, ui: &mut Ui) {
        ui.title("Aparência");
        let options = ["Claro".to_string(), "Escuro".to_string()];
        let mut chosen = usize::from(mide::kit::system_dark());
        ui.card(|ui| {
            if ui.dropdown("Aparência", &options, &mut chosen).changed() {
                let value = (chosen == 1).then(|| Value::Text("dark".into()));
                match settings_file::set("appearance", "theme", value) {
                    Ok(()) => {
                        tracing::info!("aparência: {}", options[chosen]);
                        ask_compositor("reload");
                        mide::appearance::apply(true);
                    }
                    Err(err) => tracing::warn!("aparência não gravada: {err}"),
                }
            }
        });
        ui.note("Os apps do mide mudam na hora. Apps GTK e Qt seguem a escolha quando são abertos.");
    }

    fn effects(&mut self, ui: &mut Ui) {
        ui.title("Efeitos");
        ui.card(|ui| {
            for (key, label, detail) in EFFECTS {
                let mut on = effect_on(key);
                if ui.switch_detail(label, detail, &mut on).changed() {
                    // Ligado é o padrão: grava só quando desliga, e apaga a chave ao religar.
                    let value = if on { None } else { Some(Value::Bool(false)) };
                    match settings_file::set("effects", key, value) {
                        Ok(()) => {
                            tracing::info!("efeito {label}: {}", if on { "ligado" } else { "desligado" });
                            ask_compositor("reload");
                        }
                        Err(err) => tracing::warn!("efeito não gravado: {err}"),
                    }
                }
            }
        });
        // Transparência das janelas sem foco (`[opacity] inactive`): 0% é opaca.
        let mut clear = match settings_file::get("opacity", "inactive") {
            Some(Value::Float(x)) => ((1.0 - x) * 100.0) as f32,
            _ => 0.0,
        };
        ui.heading("Transparência");
        ui.card(|ui| {
            if ui.slider("Janelas sem foco", &mut clear, 0.0, 60.0).submitted() {
                let opacity = ((100.0 - clear.round()) / 100.0) as f64;
                tracing::info!("janelas sem foco: opacidade {opacity}");
                let value = (opacity < 0.995).then_some(Value::Float(opacity));
                match settings_file::set("opacity", "inactive", value) {
                    Ok(()) => {
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("transparência não gravada: {err}"),
                }
            }
        });
        ui.note("As mudanças valem na hora. Transparência por app: [opacity] no config.toml (\"app_id\" = 0.9).");
    }

    /// Mouse e trackpad (`[input]`): velocidade, aceleração, rolagem natural e toque
    /// para clicar. Valem na hora para os aparelhos conectados.
    fn mouse(&mut self, ui: &mut Ui) {
        ui.title("Mouse e trackpad");
        // Grava só o que difere do padrão (o resto sai do arquivo).
        let save = |key: &str, value: Option<Value>| match settings_file::set("input", key, value) {
            Ok(()) => {
                ask_compositor("reload");
            }
            Err(err) => tracing::warn!("opção de mouse não gravada: {err}"),
        };
        let flag = |key: &str, default: bool| match settings_file::get("input", key) {
            Some(Value::Bool(b)) => b,
            _ => default,
        };
        let mut speed = match settings_file::get("input", "pointer_speed") {
            Some(Value::Float(x)) => x as f32,
            Some(Value::Int(n)) => n as f32,
            _ => 0.0,
        } * 50.0
            + 50.0;
        let mut accel = flag("mouse_acceleration", true);
        let mut mouse_natural = flag("mouse_natural_scroll", false);
        let mut pad_natural = flag("touchpad_natural_scroll", true);
        let mut tap = flag("tap_to_click", true);
        ui.card(|ui| {
            if ui.slider("Velocidade do ponteiro", &mut speed, 0.0, 100.0).submitted() {
                let value = ((speed - 50.0) / 50.0).clamp(-1.0, 1.0) as f64;
                let value = (value * 100.0).round() / 100.0;
                tracing::info!("mouse: velocidade {value}");
                save("pointer_speed", (value.abs() > 0.005).then_some(Value::Float(value)));
            }
            if ui.switch_detail("Aceleração do mouse", "Movimentos rápidos levam o ponteiro mais longe", &mut accel).changed() {
                tracing::info!("mouse: aceleração {}", if accel { "ligada" } else { "desligada" });
                save("mouse_acceleration", (!accel).then_some(Value::Bool(false)));
            }
            if ui.switch_detail("Rolagem natural no mouse", "O conteúdo acompanha o movimento da roda", &mut mouse_natural).changed() {
                tracing::info!("mouse: rolagem natural {}", if mouse_natural { "ligada" } else { "desligada" });
                save("mouse_natural_scroll", mouse_natural.then_some(Value::Bool(true)));
            }
        });
        ui.heading("Trackpad");
        ui.card(|ui| {
            if ui.switch_detail("Rolagem natural", "O conteúdo acompanha os dedos, como no Mac", &mut pad_natural).changed() {
                tracing::info!("trackpad: rolagem natural {}", if pad_natural { "ligada" } else { "desligada" });
                save("touchpad_natural_scroll", (!pad_natural).then_some(Value::Bool(false)));
            }
            if ui.switch_detail("Toque para clicar", "Um toque no trackpad clica, sem apertar", &mut tap).changed() {
                tracing::info!("trackpad: toque para clicar {}", if tap { "ligado" } else { "desligado" });
                save("tap_to_click", (!tap).then_some(Value::Bool(false)));
            }
        });
        ui.note("As mudanças valem na hora para os mouses e trackpads conectados (na sessão do mide).");
    }

    fn keyboard(&mut self, ui: &mut Ui) {
        ui.title("Teclado");
        let mut mac = matches!(settings_file::get("keyboard", "mac_style"), Some(Value::Bool(true)));
        // Layouts: a lista escolhida (vazia: o do sistema). Com mais de um, o atalho
        // Ctrl+Super+Espaço e o seletor da barra trocam entre eles.
        let mut list = mide::keyboard_layouts::configured();
        let shown = if list.is_empty() { vec![mide::keyboard_layouts::system()] } else { list.clone() };
        let mut add_options = vec!["Escolher…".to_string()];
        let addable: Vec<&(&str, &str, &str)> = mide::keyboard_layouts::LAYOUTS.iter().filter(|(_, l, v)| !shown.iter().any(|c| c.layout == *l && c.variant == *v)).collect();
        add_options.extend(addable.iter().map(|(n, ..)| n.to_string()));
        let mut add_i = 0;
        let mut show_bar = settings_file::flag("keyboard", "show_layout", true);
        let mut changed = false;
        ui.card(|ui| {
            for (i, choice) in shown.iter().enumerate() {
                let detail = if shown.len() > 1 { "Remover" } else { "" };
                let label = if list.is_empty() { format!("{} (do sistema)", choice.name()) } else { choice.name() };
                if ui.list_item(&label, detail).clicked() && shown.len() > 1 {
                    list.remove(i);
                    changed = true;
                }
            }
            if ui.dropdown("Adicionar layout", &add_options, &mut add_i).changed() && add_i > 0 {
                if list.is_empty() {
                    list.push(mide::keyboard_layouts::system());
                }
                let (_, l, v) = addable[add_i - 1];
                list.push(mide::keyboard_layouts::Choice { layout: l.to_string(), variant: v.to_string() });
                changed = true;
            }
            if shown.len() > 1 && ui.switch_detail("Mostrar o layout na barra superior", "Sigla do layout atual na barra, com a lista para trocar (Ctrl+Super+Espaço troca também)", &mut show_bar).changed() {
                if let Err(err) = settings_file::set("keyboard", "show_layout", (!show_bar).then_some(Value::Bool(false))) {
                    tracing::warn!("opção do teclado não gravada: {err}");
                }
            }
            let detail = "Command (tecla Alt) faz o papel do Ctrl nos apps e Option (tecla Super/Windows) é o Alt; o Ctrl continua Ctrl. Bloquear: Ctrl+⌘+Q";
            if ui.switch_detail("Teclas no estilo Mac", detail, &mut mac).changed() {
                match settings_file::set("keyboard", "mac_style", mac.then_some(Value::Bool(true))) {
                    Ok(()) => {
                        tracing::info!("teclas no estilo Mac: {}", if mac { "ligado" } else { "desligado" });
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("opção do teclado não gravada: {err}"),
                }
            }
        });
        if changed {
            // Um layout só volta a ser o do sistema quando é o do sistema.
            let value = (list.len() > 1 || list.first().is_some_and(|c| *c != mide::keyboard_layouts::system())).then(|| Value::Text(mide::keyboard_layouts::join(&list)));
            let saved = settings_file::set("keyboard", "layouts", value)
                .and_then(|_| settings_file::set("keyboard", "layout", None))
                .and_then(|_| settings_file::set("keyboard", "variant", None));
            match saved {
                Ok(()) => {
                    tracing::info!("teclado: layouts {}", mide::keyboard_layouts::join(&list));
                    ask_compositor("reload");
                }
                Err(err) => tracing::warn!("layouts não gravados: {err}"),
            }
            ui.request_repaint();
        }
        // Busca pelo nome do atalho (ou pela combinação), sem diferenciar acentos.
        let mut query = std::mem::take(&mut self.shortcut_query);
        ui.text_input(&mut query, "Buscar atalho");
        self.shortcut_query = query;
        let wanted = mide::apps::fold(self.shortcut_query.trim());
        let bindings = self.bindings.clone();
        let mut chosen = None;
        let mut shown = 0;
        // Por grupo: Janelas, Launcher e busca, Apps, Sistema.
        for group in SHORTCUT_GROUPS {
            let items: Vec<(String, String, String)> = bindings
                .iter()
                .filter(|(_, action)| shortcut_group(action) == group)
                .map(|(combo, action)| (describe_action(action, &self.apps), combo.clone(), action.clone()))
                .filter(|(title, combo, _)| {
                    wanted.is_empty() || mide::apps::fold(title).contains(&wanted) || mide::apps::fold(&pretty_combo(combo, mac)).contains(&wanted)
                })
                .collect();
            if items.is_empty() {
                continue;
            }
            shown += items.len();
            // Grupos recolhem (lembrado em `[settings] shortcuts_closed`); buscando,
            // os grupos com resultado ficam abertos.
            let mut open = !wanted.is_empty() || !self.closed_groups.iter().any(|g| g == group);
            let count = if items.len() == 1 { "1 atalho".to_string() } else { format!("{} atalhos", items.len()) };
            if ui.disclosure(group, &count, &mut open).changed() && wanted.is_empty() {
                if open {
                    self.closed_groups.retain(|g| g != group);
                } else {
                    self.closed_groups.push(group.to_string());
                }
                let value = (!self.closed_groups.is_empty()).then(|| Value::Text(self.closed_groups.join(",")));
                if let Err(err) = settings_file::set("settings", "shortcuts_closed", value) {
                    tracing::warn!("grupos de atalhos não gravados: {err}");
                }
            }
            if !open {
                continue;
            }
            ui.card(|ui| {
                for (title, combo, action) in &items {
                    if ui.list_item(title, &pretty_combo(combo, mac)).clicked() {
                        chosen = Some((combo.clone(), action.clone()));
                    }
                }
            });
        }
        if shown == 0 {
            ui.note(if bindings.is_empty() { "Nenhum atalho." } else { "Nenhum atalho com esse nome." });
        }
        let (mut add, mut reset) = (false, false);
        ui.row(|ui| {
            ui.sized(170.0, |ui| add = ui.button("Novo atalho…").clicked());
            ui.sized(200.0, |ui| reset = ui.button("Restaurar os padrões").clicked());
        });
        ui.note("Clique num atalho para mudar a combinação ou removê-lo. As mudanças valem na hora.");
        if let Some((combo, action)) = chosen {
            self.shortcut = Some(ShortcutEditor::edit(combo, action));
            ui.request_repaint();
        }
        if add {
            self.shortcut = Some(ShortcutEditor::new());
            ui.request_repaint();
        }
        if reset {
            match settings_file::clear_section("keybindings") {
                Ok(()) => self.apply_bindings(),
                Err(err) => tracing::warn!("atalhos não restaurados: {err}"),
            }
            ui.request_repaint();
        }
    }

    /// Idioma e formatos (`[locale] lang`/`formats`): o `mide-session` exporta no
    /// próximo login (`LANG` e `LC_TIME`, `LC_NUMERIC`, `LC_MONETARY`...).
    fn region(&mut self, ui: &mut Ui) {
        ui.title("Idioma e região");
        if self.locales.is_empty() {
            self.locales = installed_locales();
        }
        let current = |key: &str| match settings_file::get("locale", key) {
            Some(Value::Text(t)) => t,
            _ => String::new(),
        };
        let mut options = vec!["O do sistema".to_string()];
        options.extend(self.locales.iter().map(|l| locale_name(l)));
        let index_of = |value: &str| self.locales.iter().position(|l| *l == value).map_or(0, |i| i + 1);
        let (mut lang_i, mut formats_i) = (index_of(&current("lang")), index_of(&current("formats")));
        let locales = self.locales.clone();
        let save = |key: &str, i: usize| {
            let value = i.checked_sub(1).and_then(|i| locales.get(i)).map(|l| Value::Text(l.clone()));
            match settings_file::set("locale", key, value) {
                Ok(()) => tracing::info!("idioma e região: {key} = {}", options_name(&locales, i)),
                Err(err) => tracing::warn!("idioma não gravado: {err}"),
            }
        };
        ui.card(|ui| {
            if ui.dropdown("Idioma", &options, &mut lang_i).changed() {
                save("lang", lang_i);
            }
            if ui.dropdown("Formatos (data, hora, números e moeda)", &options, &mut formats_i).changed() {
                save("formats", formats_i);
            }
        });
        ui.note("Vale a partir do próximo login. Os idiomas da lista são os instalados no sistema (locale -a).");
    }

    /// Suspender por tempo sem uso e o botão de ligar (`[power]`).
    fn energy(&mut self, ui: &mut Ui) {
        ui.title("Energia");
        const SUSPEND: &[(&str, i64)] = &[("Nunca", 0), ("15 minutos", 15), ("30 minutos", 30), ("1 hora", 60), ("2 horas", 120), ("3 horas", 180)];
        const BUTTON: &[(&str, &str)] = &[("Perguntar o que fazer", "ask"), ("Suspender", "suspend"), ("Desligar", "poweroff"), ("Não fazer nada", "nothing")];
        let minutes = match settings_file::get("power", "suspend_minutes") {
            Some(Value::Int(n)) => n,
            _ => 0,
        };
        let button = match settings_file::get("power", "button") {
            Some(Value::Text(t)) => t,
            _ => "ask".into(),
        };
        let suspend_names: Vec<String> = SUSPEND.iter().map(|(n, _)| n.to_string()).collect();
        let button_names: Vec<String> = BUTTON.iter().map(|(n, _)| n.to_string()).collect();
        let mut suspend_i = SUSPEND.iter().position(|(_, m)| *m == minutes).unwrap_or(0);
        let mut button_i = BUTTON.iter().position(|(_, b)| *b == button).unwrap_or(0);
        let save = |key: &str, value: Option<Value>| match settings_file::set("power", key, value) {
            Ok(()) => {
                ask_compositor("reload");
            }
            Err(err) => tracing::warn!("energia não gravada: {err}"),
        };
        ui.card(|ui| {
            if ui.dropdown("Suspender o computador após", &suspend_names, &mut suspend_i).changed() {
                let (name, m) = SUSPEND[suspend_i];
                tracing::info!("energia: suspender após {name}");
                save("suspend_minutes", (m > 0).then_some(Value::Int(m)));
            }
            if ui.dropdown("Botão de ligar", &button_names, &mut button_i).changed() {
                let (name, b) = BUTTON[button_i];
                tracing::info!("energia: botão de ligar = {name}");
                save("button", (b != "ask").then(|| Value::Text(b.into())));
            }
        });
        ui.note("Sem uso: o computador não suspende enquanto um app pede para a tela ficar acesa (vídeo). Apagar e bloquear a tela ficam em Bloqueio de tela.");
    }

    /// App que abre cada tipo de arquivo (`~/.config/mide-mimeapps.list`, só na sessão do mide).
    fn default_apps(&mut self, ui: &mut Ui) {
        ui.title("Apps padrão");
        // Por categoria: os apps que abrem o tipo (ids sem ".desktop"), os nomes e o
        // escolhido agora.
        let mut rows: Vec<(&mide::default_apps::Category, Vec<String>, Vec<String>, usize)> = Vec::new();
        for category in mide::default_apps::CATEGORIES {
            let candidates = mide::default_apps::candidates(&self.apps, category);
            let mut ids: Vec<String> = candidates.iter().map(|a| a.id.clone()).collect();
            let mut names: Vec<String> = candidates.iter().map(|a| a.name.clone()).collect();
            let current = mide::default_apps::current(category.mimes[0]).map(|c| desktop_id(&c));
            // O padrão atual pode não estar na lista (app sem o tipo no .desktop).
            if let Some(id) = current.as_ref().filter(|id| !ids.contains(id)) {
                ids.insert(0, id.clone());
                names.insert(0, id.clone());
            }
            if ids.is_empty() {
                continue;
            }
            let chosen = match current.and_then(|c| ids.iter().position(|i| *i == c)) {
                Some(i) => i,
                None => {
                    ids.insert(0, String::new());
                    names.insert(0, "Nenhum".into());
                    0
                }
            };
            rows.push((category, ids, names, chosen));
        }
        let mut changed = None;
        ui.card(|ui| {
            for (i, (category, _, names, chosen)) in rows.iter_mut().enumerate() {
                if ui.dropdown(category.label, names, chosen).changed() {
                    changed = Some(i);
                }
            }
        });
        if let Some((category, ids, _, chosen)) = changed.and_then(|i| rows.get(i)) {
            if !ids[*chosen].is_empty() {
                let id = format!("{}.desktop", ids[*chosen]);
                match mide::default_apps::set(category, &id) {
                    Ok(()) => tracing::info!("apps padrão: {} = {id}", category.label),
                    Err(err) => tracing::warn!("app padrão não gravado: {err}"),
                }
            }
        }
        if rows.is_empty() {
            ui.note("Nenhum app instalado abre esses tipos de arquivo.");
        }
        ui.note("Vale só na sessão do mide: outros desktops instalados (Cinnamon, GNOME) continuam com as escolhas deles.");
    }

    /// Avisa o compositor e relê os atalhos em uso.
    fn apply_bindings(&mut self) {
        ask_compositor("reload");
        self.load_bindings();
    }

    /// Grava a combinação nova da ação do diálogo. A combinação antiga é desligada
    /// (`none`, que também desliga um atalho padrão); a nova substitui o que houver nela.
    fn save_shortcut(&mut self, new: &str) {
        let Some(editor) = self.shortcut.as_ref() else {
            return;
        };
        let Some(action) = editor.action.clone() else {
            return;
        };
        let old = editor.old.clone();
        let result = old
            .filter(|old| old != new)
            .map_or(Ok(()), |old| settings_file::set("keybindings", &old, Some(Value::Text("none".into()))))
            .and_then(|()| settings_file::set("keybindings", new, Some(Value::Text(action.clone()))));
        match result {
            Ok(()) => {
                tracing::info!("atalho gravado: {new} = {action}");
                self.shortcut = None;
                self.apply_bindings();
            }
            Err(err) => {
                tracing::warn!("atalho não gravado: {err}");
                if let Some(editor) = self.shortcut.as_mut() {
                    editor.error = Some(format!("Não foi possível gravar: {err}"));
                }
            }
        }
    }

    /// Diálogo do atalho: escolher a ação (atalho novo), gravar a combinação apertando
    /// as teclas, avisar se ela já é usada por outra ação, ou remover o atalho.
    fn shortcut_dialog(&mut self, ui: &mut Ui, mac: bool) {
        let Some(mut editor) = self.shortcut.take() else {
            return;
        };
        // Gravando: pergunta ao compositor se a combinação já foi apertada.
        if editor.capturing {
            match ask_compositor("shortcuts captured").and_then(|l| l.into_iter().next()).as_deref() {
                Some("waiting") => ui.wake_after(Duration::from_millis(80)),
                Some(combo) if combo != "cancel" && combo != "none" => {
                    editor.capturing = false;
                    editor.error = None;
                    match combo_problem(combo) {
                        Some(problem) => editor.error = Some(problem.into()),
                        None => {
                            let other = self
                                .bindings
                                .iter()
                                .find(|(c, a)| c == combo && Some(c) != editor.old.as_ref() && Some(a) != editor.action.as_ref());
                            match other {
                                Some((_, a)) => {
                                    editor.conflict = Some((combo.to_string(), describe_action(a, &self.apps)));
                                }
                                None => {
                                    self.shortcut = Some(editor);
                                    self.save_shortcut(combo);
                                    ui.request_repaint();
                                    return;
                                }
                            }
                        }
                    }
                    ui.request_repaint();
                }
                _ => {
                    editor.capturing = false;
                    ui.request_repaint();
                }
            }
        }

        let title = match editor.action.as_deref() {
            Some(action) => describe_action(action, &self.apps),
            None => "Novo atalho".to_string(),
        };
        let mut close = !editor.capturing && ui.input().key(Key::Escape);
        let (mut record, mut remove, mut replace) = (false, false, false);
        let mut pick: Option<String> = None;
        let mut query = editor.query.clone();
        ui.modal(420.0, |ui| {
            ui.strong(&title);
            if editor.action.is_none() {
                // Sem busca, as ações do mide; buscando, os apps encontrados no lugar delas
                // (o diálogo cabe na janela).
                ui.note("O que o atalho faz? Uma ação do mide ou abrir um app:");
                ui.text_input(&mut query, "Buscar app");
                ui.card(|ui| {
                    if query.trim().is_empty() {
                        for action in FIXED_ACTIONS {
                            if ui.list_item(&describe_action(action, &[]), "").clicked() {
                                pick = Some(action.to_string());
                            }
                        }
                        return;
                    }
                    let found = mide::apps::filter(&self.apps, &query, None);
                    if found.is_empty() {
                        ui.note("Nenhum app encontrado.");
                    }
                    for app in found.into_iter().take(5) {
                        if ui.list_item(&app.name, "App").clicked() {
                            pick = Some(format!("launch:{}", app_command(app)));
                        }
                    }
                });
                ui.row(|ui| {
                    ui.push_right(110.0);
                    ui.sized(110.0, |ui| close |= ui.button("Cancelar").clicked());
                });
                return;
            }
            if let Some((combo, used_by)) = &editor.conflict {
                ui.label(&format!("{} já é usado por \"{used_by}\".", pretty_combo(combo, mac)));
                ui.note("Substituir faz essa combinação passar para este atalho.");
                ui.row(|ui| {
                    ui.push_right(2.0 * 150.0 + 8.0);
                    ui.sized(150.0, |ui| record = ui.button("Gravar outra").clicked());
                    ui.sized(150.0, |ui| replace = ui.primary_button("Substituir").clicked());
                });
                return;
            }
            if editor.capturing {
                ui.label("Aperte a nova combinação de teclas…");
                ui.note("Use Ctrl, Alt ou Super junto com a tecla. Esc cancela.");
                return;
            }
            let current = editor.old.as_deref().map(|c| pretty_combo(c, mac)).unwrap_or_else(|| "Nenhuma".into());
            ui.list_item("Combinação", &current);
            if let Some(err) = &editor.error {
                ui.note(err);
            }
            ui.row(|ui| {
                let buttons = if editor.old.is_some() { 3.0 } else { 2.0 };
                ui.push_right(buttons * 120.0 + (buttons - 1.0) * 8.0);
                if editor.old.is_some() {
                    ui.sized(120.0, |ui| remove = ui.button("Remover").clicked());
                }
                ui.sized(120.0, |ui| close |= ui.button("Cancelar").clicked());
                ui.sized(120.0, |ui| record = ui.primary_button("Gravar…").clicked());
            });
        });
        editor.query = query;
        if let Some(action) = pick {
            editor.action = Some(action);
            ui.request_repaint();
        }
        if record {
            editor.conflict = None;
            editor.error = None;
            if ask_compositor("shortcuts capture").is_some() {
                editor.capturing = true;
                ui.wake_after(Duration::from_millis(80));
            } else {
                editor.error = Some("O compositor não respondeu.".into());
            }
            ui.request_repaint();
        }
        if close {
            if editor.capturing {
                ask_compositor("shortcuts stop");
            }
            ui.request_repaint();
            return;
        }
        if remove {
            if let Some(old) = editor.old.clone() {
                match settings_file::set("keybindings", &old, Some(Value::Text("none".into()))) {
                    Ok(()) => {
                        tracing::info!("atalho removido: {old}");
                        self.apply_bindings();
                    }
                    Err(err) => tracing::warn!("atalho não removido: {err}"),
                }
            }
            ui.request_repaint();
            return;
        }
        if replace {
            if let Some((combo, _)) = editor.conflict.take() {
                self.shortcut = Some(editor);
                self.save_shortcut(&combo);
                ui.request_repaint();
                return;
            }
        }
        self.shortcut = Some(editor);
    }

    fn about(&mut self, ui: &mut Ui) {
        ui.title("Sobre");
        let info = self.info.clone();
        ui.card(|ui| {
            for (label, value) in &info {
                ui.list_item(label, value);
            }
        });
        ui.note(&format!("mide {}", env!("CARGO_PKG_VERSION")));
    }

    /// Resultados da busca: opções pelo nome ou por palavras-chave.
    fn results(&mut self, ui: &mut Ui) {
        ui.title("Resultados");
        let q = mide::apps::fold(self.query.trim());
        let found: Vec<&(&str, &str, Section)> = SEARCHABLE
            .iter()
            .filter(|(name, keys, _)| {
                let name = mide::apps::fold(name);
                name.contains(&q) || mide::apps::fold(keys).split(' ').any(|k| k.starts_with(&q))
            })
            .collect();
        if found.is_empty() {
            ui.note("Nenhuma opção encontrada.");
            return;
        }
        let mut open = None;
        ui.card(|ui| {
            for (name, _, section) in &found {
                if ui.list_item(name, section_name(*section)).clicked() {
                    open = Some(*section);
                }
            }
        });
        if let Some(section) = open {
            self.section = section;
            self.query.clear();
            self.query_reset = true;
            if section == Section::Keyboard {
                self.load_bindings();
            }
            ui.request_repaint();
        }
    }
}

impl App for Settings {
    fn view(&mut self, ui: &mut Ui) {
        self.poll(ui);
        let items: Vec<(&str, &str, Option<[u8; 3]>)> = SIDEBAR.iter().map(|(s, name, icon, tint)| (*name, if s.is_some() { *icon } else { "" }, s.map(|_| *tint))).collect();
        let selected = SIDEBAR.iter().position(|(s, ..)| *s == Some(self.section)).unwrap_or(0);
        // O campo de busca fica na barra lateral; o conteúdo usa a busca deste quadro
        // (o que for digitado agora pede outro quadro, que já mostra os resultados).
        let mut query = self.query.clone();
        let searching = !query.trim().is_empty();
        // Tahoe: barra lateral flutuante de vidro, como nos Ajustes do Sistema.
        ui.glass_sidebar();
        let chosen = ui.sidebar_search(&items, if searching { usize::MAX } else { selected }, &mut query, "Buscar", |ui, _| {
            if !self.query.trim().is_empty() {
                self.results(ui);
                return;
            }
            match self.section {
                Section::Wifi => self.wifi(ui),
                Section::Bluetooth => self.bluetooth(ui),
                Section::Appearance => self.appearance(ui),
                Section::Sound => self.sound(ui),
                Section::Displays => self.displays(ui),
                Section::Wallpaper => self.wallpaper(ui),
                Section::Dock => self.dock(ui),
                Section::Lock => self.lock(ui),
                Section::Bar => self.bar(ui),
                Section::Effects => self.effects(ui),
                Section::Mouse => self.mouse(ui),
                Section::Keyboard => self.keyboard(ui),
                Section::Region => self.region(ui),
                Section::Energy => self.energy(ui),
                Section::DefaultApps => self.default_apps(ui),
                Section::About => self.about(ui),
            }
        });
        // Confirmação das telas por cima da janela inteira.
        if self.screens_revert.is_some() {
            self.displays_confirm(ui);
        }
        // Diálogo de atalho por cima da janela inteira (fora da área de conteúdo).
        if self.shortcut.is_some() {
            let mac = matches!(settings_file::get("keyboard", "mac_style"), Some(Value::Bool(true)));
            self.shortcut_dialog(ui, mac);
        }
        // Escolher um resultado limpa a busca: aí o que estava no campo não volta.
        if std::mem::take(&mut self.query_reset) {
            ui.request_repaint();
        } else if query != self.query {
            self.query = query;
            ui.request_repaint();
        }
        if chosen != selected && chosen != usize::MAX {
            if let Some((Some(section), ..)) = SIDEBAR.get(chosen) {
                self.section = *section;
                self.query.clear();
                self.picker = None;
                tracing::info!("configurações: {section:?}");
                if *section == Section::Keyboard {
                    self.load_bindings();
                }
                ui.request_repaint();
            }
        }
    }
}

/// Id do app sem o ".desktop" (como em `mide::apps::App::id`).
fn desktop_id(id: &str) -> String {
    id.trim_end_matches(".desktop").to_string()
}

/// Idiomas instalados com UTF-8 (`locale -a`) que a lista sabe nomear, no formato do
/// `LANG` (`pt_BR.UTF-8`).
fn installed_locales() -> Vec<String> {
    let out = std::process::Command::new("locale").arg("-a").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let mut list: Vec<String> = out
        .lines()
        .map(str::trim)
        .filter(|l| l.to_lowercase().ends_with(".utf8") || l.to_lowercase().ends_with(".utf-8"))
        .map(|l| format!("{}.UTF-8", l.split('.').next().unwrap_or(l)))
        .filter(|l| LOCALE_NAMES.iter().any(|(code, _)| l.starts_with(&format!("{code}."))))
        .collect();
    list.sort_by_key(|l| locale_name(l));
    list.dedup();
    list
}

/// Idiomas com nome na lista (os outros instalados não aparecem: são muitos, e a
/// lista precisa caber na janela).
const LOCALE_NAMES: &[(&str, &str)] = &[
    ("pt_BR", "Português (Brasil)"),
    ("pt_PT", "Português (Portugal)"),
    ("en_US", "English (United States)"),
    ("en_GB", "English (United Kingdom)"),
    ("es_ES", "Español (España)"),
    ("es_MX", "Español (México)"),
    ("es_AR", "Español (Argentina)"),
    ("fr_FR", "Français (France)"),
    ("de_DE", "Deutsch (Deutschland)"),
    ("it_IT", "Italiano (Italia)"),
    ("nl_NL", "Nederlands"),
    ("ru_RU", "Русский"),
    ("ja_JP", "日本語"),
    ("zh_CN", "中文 (简体)"),
];

/// Nome do idioma para a lista ("Português (Brasil) — pt_BR").
fn locale_name(code: &str) -> String {
    let base = code.split('.').next().unwrap_or(code);
    match LOCALE_NAMES.iter().find(|(c, _)| *c == base) {
        Some((_, name)) => format!("{name} — {base}"),
        None => base.to_string(),
    }
}

fn options_name(locales: &[String], i: usize) -> String {
    i.checked_sub(1).and_then(|i| locales.get(i)).cloned().unwrap_or_else(|| "o do sistema".into())
}

fn section_name(section: Section) -> &'static str {
    SIDEBAR.iter().find(|(s, ..)| *s == Some(section)).map(|(_, n, ..)| *n).unwrap_or("")
}

fn effect_on(key: &str) -> bool {
    !matches!(settings_file::get("effects", key), Some(Value::Bool(false)))
}

fn current_wallpaper() -> Option<PathBuf> {
    match settings_file::get("desktop", "wallpaper") {
        Some(Value::Text(t)) if !t.trim().is_empty() => {
            let t = t.trim();
            Some(match t.strip_prefix("~/") {
                Some(rest) => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
                None => PathBuf::from(t),
            })
        }
        _ => None,
    }
}

/// Papéis de parede para escolher: o padrão do mide (`None`) e as imagens do sistema e
/// as adicionadas pelo usuário. `MIDE_BACKGROUNDS_DIR` troca a pasta do sistema (os
/// testes usam uma com imagens conhecidas).
fn wallpaper_list() -> Vec<Option<PathBuf>> {
    let mut list = vec![None];
    let system = std::env::var_os("MIDE_BACKGROUNDS_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/usr/share/backgrounds"));
    let dirs = [system, mide::places::user_backgrounds_dir()];
    let mut files: Vec<PathBuf> = dirs
        .iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flat_map(|d| d.flatten().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_lowercase().as_str(), "jpg" | "jpeg" | "png")))
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    list.extend(files.into_iter().take(60).map(Some));
    list
}

/// Miniaturas carregadas numa thread, uma por mensagem.
fn load_thumbs(tx: Sender<Thumb>, paths: Vec<Option<PathBuf>>) {
    std::thread::spawn(move || {
        for path in paths {
            let thumb = match &path {
                Some(path) => image::open(path).ok().map(|img| to_pixmap(&mide::backdrop::cover(&img.to_rgba8(), THUMB.0, THUMB.1))),
                None => Some(to_pixmap(&mide::backdrop::default_gradient(THUMB.0, THUMB.1))),
            };
            if tx.send((path, thumb)).is_err() {
                break;
            }
        }
    });
}

/// Imagem RGBA para pixmap (premultiplicado).
fn to_pixmap(img: &image::RgbaImage) -> Pixmap {
    let mut pixmap = Pixmap::new(img.width().max(1), img.height().max(1)).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
    for (dst, src) in pixmap.data_mut().chunks_exact_mut(4).zip(img.as_raw().chunks_exact(4)) {
        let a = src[3] as u32;
        dst[0] = (src[0] as u32 * a / 255) as u8;
        dst[1] = (src[1] as u32 * a / 255) as u8;
        dst[2] = (src[2] as u32 * a / 255) as u8;
        dst[3] = src[3];
    }
    pixmap
}

/// "ctrl+shift+space" vira "Ctrl ⇧ Espaço" (ou "⌘ ⇧ Espaço" com as teclas no estilo Mac).
fn pretty_combo(combo: &str, mac: bool) -> String {
    combo
        .split('+')
        .map(|k| match k {
            "ctrl" if mac => "⌘".to_string(),
            "ctrl" => "Ctrl".to_string(),
            "shift" => "⇧".to_string(),
            "alt" if mac => "⌥".to_string(),
            "alt" => "Alt".to_string(),
            "super" if mac => "Ctrl".to_string(),
            "super" => "Super".to_string(),
            "space" => "Espaço".to_string(),
            "escape" => "Esc".to_string(),
            "return" => "Enter".to_string(),
            "tab" => "Tab".to_string(),
            "up" => "↑".to_string(),
            "down" => "↓".to_string(),
            "left" => "←".to_string(),
            "right" => "→".to_string(),
            "print" => "Print Screen".to_string(),
            "delete" => "Delete".to_string(),
            "backspace" => "⌫".to_string(),
            other => other.to_uppercase(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Diálogo de mudar ou criar um atalho.
#[derive(Default)]
struct ShortcutEditor {
    /// Combinação atual (`None`: atalho novo).
    old: Option<String>,
    /// Ação do atalho (`None`: atalho novo, ainda escolhendo).
    action: Option<String>,
    /// Busca de apps, ao escolher a ação.
    query: String,
    /// Esperando o usuário apertar a combinação (o compositor grava).
    capturing: bool,
    /// Combinação apertada que já é de outra ação: a combinação e o nome da outra.
    conflict: Option<(String, String)>,
    error: Option<String>,
}

impl ShortcutEditor {
    fn new() -> Self {
        Self::default()
    }

    fn edit(combo: String, action: String) -> Self {
        Self { old: Some(combo), action: Some(action), ..Self::default() }
    }
}

/// Ações do mide que um atalho novo pode fazer (além de abrir um app).
const FIXED_ACTIONS: &[&str] = &[
    "switch_windows",
    "switch_windows_back",
    "maximize",
    "unmaximize",
    "toggle_maximize",
    "minimize",
    "tile_left",
    "tile_right",
    "close_window",
    "restore_minimized",
    "show_desktop",
    "toggle_launcher",
    "toggle_search",
    "next_layout",
    "lock_screen",
    "quit",
];

/// Grupos da lista de atalhos, na ordem em que aparecem.
const SHORTCUT_GROUPS: [&str; 4] = ["Janelas", "Launcher e busca", "Apps", "Sistema"];

/// Grupo de um atalho (pela ação).
fn shortcut_group(action: &str) -> &'static str {
    match action {
        "toggle_launcher" | "toggle_search" => "Launcher e busca",
        "lock_screen" | "quit" | "launch:mide-screenshot" | "next_layout" => "Sistema",
        a if a.starts_with("launch:") => "Apps",
        _ => "Janelas",
    }
}

/// Combinação que não serve para atalho global: sem Ctrl, Alt ou Super ela tomaria
/// uma tecla usada para digitar. Teclas de função (F1 a F24) podem ficar sozinhas.
fn combo_problem(combo: &str) -> Option<&'static str> {
    let parts: Vec<&str> = combo.split('+').collect();
    let key = parts.last().copied().unwrap_or("");
    let function_key = key.strip_prefix('f').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    let held = parts.iter().any(|p| matches!(*p, "ctrl" | "alt" | "super"));
    (!held && !function_key).then_some("Use Ctrl, Alt ou Super junto com a tecla, para não tomar uma tecla de digitar.")
}

/// Comando que abre o app (apps de terminal abrem no terminal).
fn app_command(app: &mide::apps::App) -> String {
    if app.terminal {
        format!("mide-terminal -e {}", app.command)
    } else {
        app.command.clone()
    }
}

/// Ação de um atalho em português. Atalhos que abrem um app mostram o nome dele.
fn describe_action(action: &str, apps: &[mide::apps::App]) -> String {
    match action {
        "close_window" => "Fechar a janela".into(),
        "quit" => "Encerrar a sessão do mide".into(),
        "restore_minimized" => "Restaurar a última janela minimizada".into(),
        "toggle_launcher" => "Abrir ou fechar o launcher".into(),
        "toggle_search" => "Abrir ou fechar o buscador".into(),
        "next_layout" => "Próximo layout de teclado".into(),
        "switch_windows" => "Alternar janelas".into(),
        "switch_windows_back" => "Alternar janelas (para trás)".into(),
        "maximize" => "Maximizar a janela".into(),
        "unmaximize" => "Restaurar o tamanho da janela".into(),
        "toggle_maximize" => "Maximizar ou restaurar a janela".into(),
        "minimize" => "Minimizar a janela".into(),
        "tile_left" => "Janela na metade esquerda".into(),
        "tile_right" => "Janela na metade direita".into(),
        "show_desktop" => "Mostrar a área de trabalho".into(),
        "lock_screen" => "Bloquear a tela".into(),
        "launch:mide-screenshot" => "Captura de tela".into(),
        "launch:mide-terminal --hotkey" => "Mostrar ou esconder o terminal de atalho".into(),
        other => match other.strip_prefix("launch:") {
            Some(cmd) => match apps.iter().find(|a| a.command == cmd || app_command(a) == cmd) {
                Some(app) => format!("Abrir {}", app.name),
                None => format!("Abrir: {cmd}"),
            },
            None => other.into(),
        },
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_settings=info".into()))
        .init();
    let section = match std::env::args().nth(1).as_deref() {
        Some("bluetooth") => Section::Bluetooth,
        Some("appearance") => Section::Appearance,
        Some("sound") => Section::Sound,
        Some("displays") => Section::Displays,
        Some("wallpaper") => Section::Wallpaper,
        Some("dock") => Section::Dock,
        Some("bar") => Section::Bar,
        Some("effects") => Section::Effects,
        Some("lock") => Section::Lock,
        Some("mouse") => Section::Mouse,
        Some("keyboard") => Section::Keyboard,
        Some("region") => Section::Region,
        Some("energy") => Section::Energy,
        Some("default-apps") => Section::DefaultApps,
        Some("about") => Section::About,
        _ => Section::Wifi,
    };
    tracing::info!("configurações abertas: {section:?}");
    let window = Window::new("Configurações do Sistema").app_id("mide-settings").size(820, 560).min_size(640, 440).full_size_content();
    run(window, Settings::new(section))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atalhos_em_portugues() {
        assert_eq!(pretty_combo("ctrl+shift+space", false), "Ctrl ⇧ Espaço");
        assert_eq!(pretty_combo("ctrl+shift+t", false), "Ctrl ⇧ T");
        assert_eq!(pretty_combo("ctrl+space", true), "⌘ Espaço");
        assert_eq!(describe_action("toggle_launcher", &[]), "Abrir ou fechar o launcher");
        assert_eq!(describe_action("toggle_search", &[]), "Abrir ou fechar o buscador");
        assert_eq!(describe_action("launch:foot", &[]), "Abrir: foot");
    }

    #[test]
    fn atalho_de_app_mostra_o_nome() {
        let app = mide::apps::App {
            id: "org.codeberg.dnkl.foot".into(),
            name: "Foot".into(),
            command: "foot".into(),
            icon: None,
            group: "Utilitários",
            terminal: false,
            keywords: String::new(),
            mime: Vec::new(),
        };
        assert_eq!(describe_action("launch:foot", &[app]), "Abrir Foot");
    }

    #[test]
    fn combinacao_precisa_de_modificador() {
        assert!(combo_problem("ctrl+shift+t").is_none());
        assert!(combo_problem("super+e").is_none());
        assert!(combo_problem("alt+f4").is_none());
        assert!(combo_problem("f5").is_none());
        assert!(combo_problem("a").is_some());
        assert!(combo_problem("shift+a").is_some());
        assert!(combo_problem("space").is_some());
    }

    #[test]
    fn busca_acha_opcoes_por_palavra_chave() {
        let q = mide::apps::fold("volume");
        let hits: Vec<&str> = SEARCHABLE.iter().filter(|(n, k, _)| mide::apps::fold(n).contains(&q) || mide::apps::fold(k).split(' ').any(|w| w.starts_with(&q))).map(|(n, ..)| *n).collect();
        assert_eq!(hits, vec!["Volume de saída"]);
    }
}
