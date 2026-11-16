//! Discos (`mide-disks`), como o Utilitário de Disco do Mac: os discos e as partições,
//! com o uso de cada uma, montar e desmontar, formatar, criar e apagar partições, apagar
//! o disco inteiro, ejetar e criar pendrives de boot de Linux (a imagem gravada como é,
//! inicia em UEFI e em BIOS) e de Windows (os arquivos copiados para um pendrive FAT32,
//! inicia em UEFI). Por baixo, o UDisks2 (`mide::disks`), que pede a senha de
//! administrador pelo polkit. O disco do sistema que está rodando fica protegido.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use mide::disks::{self, Disk, Volume, MIB};
use mide::kit::{run, window_menu, App, Choice, FileChooser, MenuGroup, MenuItem, Ui, Window};
use mide::processes::bytes_text;
use mide::tr;

/// Formatos oferecidos (nome do UDisks, nome para mostrar).
const FORMATS: &[(&str, &str)] = &[("exfat", "ExFAT"), ("vfat", "FAT32"), ("ntfs", "NTFS"), ("ext4", "ext4"), ("btrfs", "Btrfs")];

/// O que está aberto em cima da janela.
enum Dialog {
    /// Formatar uma partição: nome e formato.
    Format { block: String, name: String, format: usize },
    /// Apagar o disco inteiro: nome, formato e esquema (0 = GPT, 1 = MBR).
    Erase { name: String, format: usize, scheme: usize },
    /// Partição nova no espaço livre: tamanho (GB), nome e formato.
    NewPartition { offset: u64, free: u64, size_gb: f32, name: String, format: usize },
    /// Apagar uma partição (confirmação).
    DeletePartition { block: String, title: String },
    /// Pendrive de boot: a imagem e o tipo (0 = Linux, 1 = Windows).
    Bootable { image: Option<PathBuf>, kind: usize },
    /// Confirmar o pendrive de boot (apaga tudo).
    ConfirmBootable { image: PathBuf, kind: usize },
}

/// Ação decidida num diálogo e feita depois de fechá-lo.
type Deferred = Box<dyn FnOnce(&mut DisksApp)>;

enum Msg {
    Disks(Result<Vec<Disk>, disks::Error>),
    Progress(String, Option<f32>),
    Done(Result<String, disks::Error>),
}

struct DisksApp {
    disks: Vec<Disk>,
    selected: usize,
    /// Partição escolhida no disco (índice em `volumes`), ou espaço livre (início).
    volume: Option<usize>,
    free: Option<(u64, u64)>,
    dialog: Option<Dialog>,
    chooser: Option<FileChooser>,
    busy: Option<(String, Option<f32>)>,
    message: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    waker: Option<mide::kit::Waker>,
    started: bool,
}

impl DisksApp {
    fn new() -> Self {
        let (tx, rx) = channel();
        Self { disks: Vec::new(), selected: 0, volume: None, free: None, dialog: None, chooser: None, busy: None, message: None, tx, rx, waker: None, started: false }
    }

    fn send(&self) -> impl Fn(Msg) + Send + Clone + 'static {
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        move |m| {
            let _ = tx.send(m);
            if let Some(w) = &waker {
                w.wake();
            }
        }
    }

    fn reload(&self) {
        let send = self.send();
        std::thread::spawn(move || send(Msg::Disks(disks::disks())));
    }

    /// Roda uma operação numa thread, com a barra de progresso e a lista relida no fim.
    fn work(&mut self, title: String, job: impl FnOnce(&dyn Fn(String, Option<f32>)) -> Result<String, disks::Error> + Send + 'static) {
        self.busy = Some((title, None));
        let send = self.send();
        std::thread::spawn(move || {
            let progress = {
                let send = send.clone();
                move |stage: String, fraction: Option<f32>| send(Msg::Progress(stage, fraction))
            };
            let result = job(&progress);
            send(Msg::Done(result));
            send(Msg::Disks(disks::disks()));
        });
    }

    fn receive(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Disks(Ok(list)) => {
                    tracing::info!("discos: {} disco(s)", list.len());
                    self.disks = list;
                    self.selected = self.selected.min(self.disks.len().saturating_sub(1));
                    if self.volume.is_some_and(|v| self.disks.get(self.selected).is_none_or(|d| v >= d.volumes.len())) {
                        self.volume = None;
                    }
                }
                Msg::Disks(Err(err)) => self.message = Some(tr!("Couldn’t read the disks: {err}", err = err)),
                Msg::Progress(stage, fraction) => self.busy = Some((stage, fraction)),
                Msg::Done(result) => {
                    self.busy = None;
                    match result {
                        Ok(done) if !done.is_empty() => {
                            tracing::info!("discos: {done}");
                            self.message = Some(done);
                        }
                        Ok(_) | Err(disks::Error::NotAuthorized) => {}
                        Err(err) => self.message = Some(tr!("Couldn’t finish: {err}", err = err)),
                    }
                }
            }
        }
    }

    fn disk(&self) -> Option<&Disk> {
        self.disks.get(self.selected)
    }

    fn header(&self, ui: &mut Ui, d: &Disk) {
        let icon = if d.optical {
            "media-optical"
        } else if d.removable || d.bus == "usb" {
            "drive-removable-media"
        } else {
            "drive-harddisk"
        };
        let table = match d.table.as_str() {
            "gpt" => tr!("GUID partition table (GPT)"),
            "dos" => tr!("Master Boot Record (MBR)"),
            _ => tr!("No partition table"),
        };
        let bus = if d.bus.is_empty() { String::new() } else { format!(" · {}", d.bus.to_uppercase()) };
        ui.card(|ui| {
            ui.columns(
                72.0,
                12.0,
                |ui| ui.icon(icon, 64),
                |ui| {
                    ui.strong(&d.name());
                    ui.note(&format!("{}{} · {}", bytes_text(d.size as f64), bus, d.device));
                    ui.note(&table);
                    if d.system {
                        ui.note(&tr!("This is the disk of the running system: erasing it is not allowed here."));
                    }
                },
            );
        });
    }

    /// Barra com as partições em proporção (como a do Utilitário de Disco) e a legenda.
    fn usage_bar(&mut self, ui: &mut Ui, d: &Disk) {
        let colors: [[u8; 4]; 6] = [[10, 132, 255, 255], [52, 199, 89, 255], [255, 159, 10, 255], [175, 82, 222, 255], [255, 69, 58, 255], [90, 200, 250, 255]];
        let free_color = [200, 200, 205, 255];
        let mut segments: Vec<(u64, u64, Option<usize>, [u8; 4])> = d.volumes.iter().enumerate().map(|(i, v)| (v.offset, v.size, Some(i), colors[i % colors.len()])).collect();
        for (offset, size) in d.free_regions() {
            segments.push((offset, size, None, free_color));
        }
        segments.sort_by_key(|s| s.0);
        let total = d.size.max(1) as f32;
        let r = ui.region();
        let top = r.1 + r.3 - ui.remaining_height();
        let (x0, w, h) = (r.0, r.2, 34.0);
        let mut clicked = None;
        let mut x = x0;
        let t = ui.theme().clone();
        let accent = t.accent;
        for (offset, size, vol, color) in &segments {
            let sw = (*size as f32 / total * w).max(4.0);
            let rect = (x, top, sw - 2.0, h);
            let resp = ui.background(rect);
            if resp.clicked() {
                clicked = Some((*vol, (*offset, *size)));
            }
            let selected = match vol {
                Some(i) => self.volume == Some(*i),
                None => self.free.is_some_and(|f| f.0 == *offset),
            };
            let c = *color;
            ui.paint(move |canvas, _| {
                mide::ui::draw::fill_rounded(canvas, rect.0, rect.1, rect.2, rect.3, 6.0, tiny_skia::Color::from_rgba8(c[0], c[1], c[2], c[3]));
                if selected {
                    mide::ui::draw::stroke_rounded(canvas, rect.0 - 1.5, rect.1 - 1.5, rect.2 + 3.0, rect.3 + 3.0, 7.5, 2.0, tiny_skia::Color::from_rgba8(accent[0], accent[1], accent[2], 255));
                }
            });
            x += sw;
        }
        ui.space(h + 10.0);
        if let Some((vol, region)) = clicked {
            self.volume = vol;
            self.free = if vol.is_none() { Some(region) } else { None };
        }
        // Legenda.
        ui.wrap(|ui| {
            for (offset, size, vol, _) in &segments {
                let text = match vol {
                    Some(i) => format!("{} · {}", d.volumes[*i].title(), bytes_text(*size as f64)),
                    None => format!("{} · {}", tr!("Free"), bytes_text(*size as f64)),
                };
                let selected = match vol {
                    Some(i) => self.volume == Some(*i),
                    None => self.free.is_some_and(|f| f.0 == *offset),
                };
                if ui.button_styled(&text, if selected { mide::kit::ButtonStyle::Prominent } else { mide::kit::ButtonStyle::Bordered }).clicked() {
                    self.volume = *vol;
                    self.free = if vol.is_none() { Some((*offset, *size)) } else { None };
                }
            }
        });
    }

    fn volume_card(&mut self, ui: &mut Ui, d: &Disk, v: &Volume) {
        let mut action = None;
        ui.card(|ui| {
            ui.strong(&v.title());
            let fs = if v.fstype.is_empty() { tr!("Unknown").into_owned() } else { v.fstype.clone() };
            ui.key_value(&mide::i18n::trc("disk", "Format"), &fs, 160.0);
            ui.key_value(&tr!("Capacity"), &bytes_text(v.size as f64), 160.0);
            if let Some(used) = v.used {
                ui.key_value(&tr!("Used"), &format!("{} ({:.0} %)", bytes_text(used as f64), used as f64 * 100.0 / v.size.max(1) as f64), 160.0);
                ui.key_value(&tr!("Available"), &bytes_text(v.size.saturating_sub(used) as f64), 160.0);
            }
            ui.key_value(&tr!("Mounted at"), &v.mount_points.first().cloned().unwrap_or_else(|| tr!("Not mounted").into_owned()), 160.0);
            ui.key_value(&tr!("Device"), &v.device, 160.0);
            ui.row(|ui| {
                let can_mount = !v.fstype.is_empty() && v.fstype != "swap" && !v.fstype.starts_with("crypto");
                if can_mount {
                    if v.mount_points.is_empty() {
                        if ui.button(&tr!("Mount")).clicked() {
                            action = Some("mount");
                        }
                    } else if !v.system && ui.button(&tr!("Unmount")).clicked() {
                        action = Some("unmount");
                    }
                }
                if !v.mount_points.is_empty() && ui.button(&tr!("Open")).clicked() {
                    action = Some("open");
                }
                if !v.system && !d.system {
                    if ui.button(&tr!("Format…")).clicked() {
                        action = Some("format");
                    }
                    if v.number > 0 && ui.button(&tr!("Delete Partition…")).clicked() {
                        action = Some("delete");
                    }
                }
            });
        });
        let block = v.block.clone();
        match action {
            Some("mount") => self.work(tr!("Mounting…").into_owned(), move |_| disks::mount(&block).map(|_| String::new())),
            Some("unmount") => self.work(tr!("Unmounting…").into_owned(), move |_| disks::unmount(&block).map(|_| String::new())),
            Some("open") => {
                if let Some(m) = v.mount_points.first() {
                    let _ = std::process::Command::new("mide-files").arg(m).spawn();
                }
            }
            Some("format") => self.dialog = Some(Dialog::Format { block, name: v.label.clone(), format: 0 }),
            Some("delete") => self.dialog = Some(Dialog::DeletePartition { block, title: v.title() }),
            _ => {}
        }
    }

    fn dialogs(&mut self, ui: &mut Ui) {
        let Some(dialog) = self.dialog.as_mut() else { return };
        let mut close = ui.input().key(mide::kit::Key::Escape);
        let mut run: Option<Deferred> = None;
        let format_names: Vec<&str> = FORMATS.iter().map(|f| f.1).collect();
        let disk = self.disks.get(self.selected).cloned();
        match dialog {
            Dialog::Format { block, name, format } => {
                ui.modal(460.0, |ui| {
                    ui.title(&tr!("Format"));
                    ui.note(&tr!("Everything on this partition will be erased."));
                    ui.text_input(name, &tr!("Name"));
                    ui.segmented(&format_names, format, None);
                    ui.row(|ui| {
                        if ui.button(&tr!("Cancel")).clicked() {
                            close = true;
                        }
                        if ui.primary_button(&tr!("Erase")).clicked() {
                            let (block, name, fs) = (block.clone(), name.clone(), FORMATS[*format].0);
                            run = Some(Box::new(move |app: &mut DisksApp| app.work(tr!("Formatting…").into_owned(), move |_| disks::format(&block, fs, &name).map(|_| String::new()))));
                        }
                    });
                });
            }
            Dialog::Erase { name, format, scheme } => {
                ui.modal(500.0, |ui| {
                    ui.title(&tr!("Erase Disk"));
                    ui.note(&tr!("Everything on the disk will be erased and a single partition will be created."));
                    ui.text_input(name, &tr!("Name"));
                    ui.segmented(&format_names, format, None);
                    ui.segmented(&[&tr!("GUID (GPT)"), &tr!("MBR (old computers)")], scheme, None);
                    ui.row(|ui| {
                        if ui.button(&tr!("Cancel")).clicked() {
                            close = true;
                        }
                        if ui.primary_button(&tr!("Erase")).clicked()
                            && let Some(d) = &disk
                        {
                            let (block, name, fs, table) = (d.block.clone(), name.clone(), FORMATS[*format].0, if *scheme == 0 { "gpt" } else { "dos" });
                            run = Some(Box::new(move |app: &mut DisksApp| {
                                app.work(tr!("Erasing…").into_owned(), move |_| {
                                    disks::format(&block, table, "")?;
                                    disks::create_partition(&block, MIB, 0, "", fs, &name).map(|_| String::new())
                                })
                            }));
                        }
                    });
                });
            }
            Dialog::NewPartition { offset, free, size_gb, name, format } => {
                let max_gb = *free as f32 / (1024.0 * MIB as f32);
                ui.modal(500.0, |ui| {
                    ui.title(&tr!("New Partition"));
                    ui.slider(&tr!("Size (GB)"), size_gb, 0.1, max_gb);
                    ui.note(&format!("{:.1} GB / {:.1} GB", size_gb, max_gb));
                    ui.text_input(name, &tr!("Name"));
                    ui.segmented(&format_names, format, None);
                    ui.row(|ui| {
                        if ui.button(&tr!("Cancel")).clicked() {
                            close = true;
                        }
                        if ui.primary_button(&tr!("Create")).clicked()
                            && let Some(d) = &disk
                        {
                            let size = if *size_gb >= max_gb - 0.05 { 0 } else { (*size_gb as f64 * 1024.0) as u64 * MIB };
                            let (block, offset, name, fs) = (d.block.clone(), *offset, name.clone(), FORMATS[*format].0);
                            run = Some(Box::new(move |app: &mut DisksApp| app.work(tr!("Creating the partition…").into_owned(), move |_| disks::create_partition(&block, offset, size, "", fs, &name).map(|_| String::new()))));
                        }
                    });
                });
            }
            Dialog::DeletePartition { block, title } => {
                let message = tr!("“{name}” and everything on it will be erased.", name = title.clone());
                match ui.alert(&tr!("Delete the partition?"), &message, &[&tr!("Cancel"), &tr!("Delete")]) {
                    Some(1) => {
                        let block = block.clone();
                        run = Some(Box::new(move |app: &mut DisksApp| app.work(tr!("Deleting…").into_owned(), move |_| disks::delete_partition(&block).map(|_| String::new()))));
                    }
                    Some(_) => close = true,
                    None => {}
                }
            }
            Dialog::Bootable { image, kind } => {
                let mut choose = false;
                ui.modal(520.0, |ui| {
                    ui.title(&tr!("Create a Bootable Drive"));
                    ui.note(&tr!("Linux images are written as they are (they start in UEFI and BIOS). For Windows, the files are copied to a FAT32 drive (starts in UEFI)."));
                    ui.segmented(&["Linux", "Windows"], kind, None);
                    ui.row(|ui| {
                        if ui.button(&tr!("Choose Image…")).clicked() {
                            choose = true;
                        }
                        ui.note(&image.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| tr!("No image chosen").into_owned()));
                    });
                    ui.row(|ui| {
                        if ui.button(&tr!("Cancel")).clicked() {
                            close = true;
                        }
                        if image.is_some() && ui.primary_button(&tr!("Continue")).clicked() {
                            let (image, kind) = (image.clone().unwrap(), *kind);
                            run = Some(Box::new(move |app: &mut DisksApp| app.dialog = Some(Dialog::ConfirmBootable { image, kind })));
                        }
                    });
                });
                if choose {
                    let downloads = mide::places::downloads_dir();
                    self.chooser = Some(FileChooser::open(&tr!("Choose Image"), Some(&downloads)).extensions(&["iso", "img"]));
                }
            }
            Dialog::ConfirmBootable { image, kind } => {
                let name = disk.as_ref().map(|d| d.name()).unwrap_or_default();
                let message = tr!("Everything on “{name}” will be erased.", name = name);
                match ui.alert(&tr!("Create the bootable drive?"), &message, &[&tr!("Cancel"), &tr!("Erase and Create")]) {
                    Some(1) => {
                        if let Some(d) = disk.clone() {
                            let (image, kind) = (image.clone(), *kind);
                            run = Some(Box::new(move |app: &mut DisksApp| {
                                app.work(tr!("Preparing…").into_owned(), move |progress| if kind == 0 { write_image(&d, &image, progress) } else { windows_drive(&d, &image, progress) })
                            }));
                        }
                    }
                    Some(_) => close = true,
                    None => {}
                }
            }
        }
        if let Some(f) = run {
            self.dialog = None;
            f(self);
        } else if close {
            self.dialog = None;
        }
    }
}

/// Grava a imagem no disco inteiro (Linux: a ISO híbrida inicia em UEFI e em BIOS).
fn write_image(d: &Disk, image: &Path, progress: &dyn Fn(String, Option<f32>)) -> Result<String, disks::Error> {
    for v in &d.volumes {
        if !v.mount_points.is_empty() {
            disks::unmount(&v.block)?;
        }
    }
    let fd = disks::open_for_restore(&d.block)?;
    let mut out = std::fs::File::from(fd);
    let mut input = std::fs::File::open(image).map_err(|e| disks::Error::Failed(e.to_string()))?;
    let total = input.metadata().map(|m| m.len()).unwrap_or(1).max(1);
    let mut buf = vec![0u8; 4 * MIB as usize];
    let mut done = 0u64;
    loop {
        let n = input.read(&mut buf).map_err(|e| disks::Error::Failed(e.to_string()))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| disks::Error::Failed(e.to_string()))?;
        done += n as u64;
        progress(tr!("Writing… {done} of {total}", done = bytes_text(done as f64), total = bytes_text(total as f64)), Some(done as f32 / total as f32));
    }
    progress(tr!("Finishing…").into_owned(), None);
    out.sync_all().map_err(|e| disks::Error::Failed(e.to_string()))?;
    Ok(tr!("The bootable drive is ready.").into_owned())
}

/// Pendrive do Windows (UEFI): tabela MBR, uma partição FAT32 e os arquivos da ISO
/// copiados; o `install.wim` maior que 4 GB é dividido pelo `wimlib-imagex`.
fn windows_drive(d: &Disk, image: &Path, progress: &dyn Fn(String, Option<f32>)) -> Result<String, disks::Error> {
    let fail = |e: std::io::Error| disks::Error::Failed(e.to_string());
    progress(tr!("Erasing…").into_owned(), None);
    disks::format(&d.block, "dos", "")?;
    let part = disks::create_partition(&d.block, MIB, 0, "0x0c", "vfat", "WINDOWS")?;
    disks::wait_filesystem(&part, 25);
    let target = PathBuf::from(disks::mount(&part)?);
    progress(tr!("Opening the image…").into_owned(), None);
    let iso_block = disks::loop_setup(image)?;
    disks::wait_filesystem(&iso_block, 25);
    let source = PathBuf::from(disks::mount(&iso_block)?);
    let files = walk(&source);
    let total: u64 = files.iter().map(|(_, size)| *size).sum::<u64>().max(1);
    let mut done = 0u64;
    let big_wim = source.join("sources/install.wim");
    let split = big_wim.metadata().map(|m| m.len() > 4_000_000_000).unwrap_or(false);
    for (file, size) in &files {
        let rel = file.strip_prefix(&source).unwrap_or(file);
        let dest = target.join(rel);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(fail)?;
        }
        if split && *file == big_wim {
            // FAT32 não guarda arquivos de 4 GB ou mais: o Windows aceita o install.wim
            // dividido em partes .swm.
            progress(tr!("Splitting install.wim…").into_owned(), Some(done as f32 / total as f32));
            let ok = std::process::Command::new("wimlib-imagex").arg("split").arg(file).arg(target.join("sources/install.swm")).arg("3800").status().is_ok_and(|s| s.success());
            if !ok {
                let _ = disks::unmount(&iso_block);
                let _ = disks::loop_delete(&iso_block);
                return Err(disks::Error::Failed(tr!("install.wim is larger than 4 GB and wimlib-imagex (package wimtools) isn’t installed to split it.").into_owned()));
            }
        } else {
            std::fs::copy(file, &dest).map_err(fail)?;
        }
        done += size;
        progress(tr!("Copying… {done} of {total}", done = bytes_text(done as f64), total = bytes_text(total as f64)), Some(done as f32 / total as f32));
    }
    progress(tr!("Finishing…").into_owned(), None);
    let _ = std::process::Command::new("sync").status();
    let _ = disks::unmount(&iso_block);
    let _ = disks::loop_delete(&iso_block);
    disks::unmount(&part)?;
    Ok(tr!("The Windows bootable drive is ready (UEFI).").into_owned())
}

/// Arquivos de uma pasta, com o tamanho, recursivamente.
fn walk(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = entry.path();
            match entry.metadata() {
                Ok(m) if m.is_dir() => stack.push(p),
                Ok(m) => out.push((p, m.len())),
                Err(_) => {}
            }
        }
    }
    out.sort();
    out
}

impl App for DisksApp {
    fn view(&mut self, ui: &mut Ui) {
        if !self.started {
            self.started = true;
            self.waker = ui.waker();
            self.reload();
        }
        self.receive();
        for action in ui.input().menu.clone() {
            match action.as_str() {
                "reload" => self.reload(),
                "quit" => ui.close(),
                _ => {}
            }
        }
        if let Some(chooser) = self.chooser.as_mut() {
            match chooser.show(ui) {
                Some(Choice::Chosen(path)) => {
                    let windows = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).is_some_and(|n| n.contains("win"));
                    if let Some(Dialog::Bootable { image, kind }) = self.dialog.as_mut() {
                        *image = Some(path);
                        if windows {
                            *kind = 1;
                        }
                    }
                    self.chooser = None;
                }
                Some(Choice::Cancelled) => self.chooser = None,
                None => {}
            }
            return;
        }
        let names: Vec<String> = self.disks.iter().map(|d| d.name()).collect();
        let icons: Vec<&str> = self.disks.iter().map(|d| if d.removable || d.bus == "usb" { "drive-removable-media-symbolic" } else { "drive-harddisk-symbolic" }).collect();
        let items: Vec<(&str, &str, Option<[u8; 3]>)> = names.iter().zip(&icons).map(|(n, i)| (n.as_str(), *i, None)).collect();
        let mut query = String::new();
        let current = self.selected;
        let chosen = ui.sidebar_search(&items, current, &mut query, &tr!("Search"), |ui, _| {
            let Some(d) = self.disk().cloned() else {
                ui.note(&tr!("Reading the disks…"));
                return;
            };
            ui.row(|ui| {
                if !d.system && ui.button(&tr!("Erase…")).clicked() {
                    self.dialog = Some(Dialog::Erase { name: tr!("Untitled").into_owned(), format: 0, scheme: 0 });
                }
                if !d.system && (d.removable || d.bus == "usb") && ui.button(&tr!("Bootable Drive…")).clicked() {
                    self.dialog = Some(Dialog::Bootable { image: None, kind: 0 });
                }
                if (d.removable || d.ejectable) && !d.system && ui.button(&tr!("Eject")).clicked() {
                    let disk = d.clone();
                    self.work(tr!("Ejecting…").into_owned(), move |_| disks::eject(&disk).map(|_| String::new()));
                }
                if let Some((offset, size)) = self.free.filter(|_| !d.system)
                    && ui.button(&tr!("New Partition…")).clicked()
                {
                    let gb = size as f32 / (1024.0 * MIB as f32);
                    self.dialog = Some(Dialog::NewPartition { offset, free: size, size_gb: gb, name: String::new(), format: 0 });
                }
            });
            self.header(ui, &d);
            self.usage_bar(ui, &d);
            if let Some(v) = self.volume.and_then(|i| d.volumes.get(i)).cloned() {
                self.volume_card(ui, &d, &v);
            } else if self.free.is_some() {
                ui.note(&tr!("Free space: create a partition with “New Partition…”."));
            } else {
                ui.note(&tr!("Choose a partition in the bar above."));
            }
            if let Some((stage, fraction)) = self.busy.clone() {
                ui.card(|ui| {
                    ui.progress_bar(fraction);
                    ui.note(&stage);
                });
            }
        });
        if chosen != current && chosen < self.disks.len() {
            self.selected = chosen;
            self.volume = None;
            self.free = None;
        }
        self.dialogs(ui);
        if let Some(message) = self.message.clone()
            && ui.alert(&message, "", &[&tr!("OK")]).is_some()
        {
            self.message = None;
        }
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        vec![
            MenuGroup {
                title: tr!("File").into_owned(),
                items: vec![MenuItem::action("reload", &tr!("Refresh")).shortcut("Ctrl+R"), MenuItem::Separator, MenuItem::action("quit", &tr!("Quit Disks")).shortcut("Ctrl+Q")],
            },
            window_menu(Vec::new()),
        ]
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _logging = mide::logging::init("mide-disks", "mide_disks=info,mide=info");
    mide::i18n::init();
    let window = Window::new(&tr!("Disks")).app_id("mide-disks").size(980, 660).min_size(720, 500).full_size_content();
    run(window, DisksApp::new())
}
