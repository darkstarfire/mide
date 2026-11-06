//! Discos, pendrives e CDs: os volumes montados nos lugares usuais (`/media`,
//! `/run/media`, `/mnt`) e os removíveis ainda desmontados (pelo `lsblk`), com montar e
//! ejetar pelo UDisks (`udisksctl`). Usado pela área de trabalho e pela barra lateral do
//! Arquivos (Locais), como no Finder.

use std::path::{Path, PathBuf};

use crate::places;

/// Volume removível montado ou detectado ainda desmontado (pendrive, HD, CD/DVD).
#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub device: String,
    pub mount: Option<PathBuf>,
    pub fstype: String,
    pub label: Option<String>,
}

/// Volumes montados nos lugares usuais e partições removíveis ainda desmontadas.
pub fn read_drives() -> Vec<Drive> {
    let path = std::env::var_os("MIDE_DESKTOP_MOUNTINFO").map(PathBuf::from).unwrap_or_else(|| "/proc/self/mountinfo".into());
    let mut drives = std::fs::read_to_string(path).map(|t| parse_mountinfo(&t)).unwrap_or_default();
    if let Ok(output) = std::process::Command::new("lsblk")
        .args(["--json", "--paths", "--output", "PATH,TYPE,RM,TRAN,FSTYPE,LABEL,MOUNTPOINTS"])
        .output()
        && output.status.success()
        && let Ok(text) = std::str::from_utf8(&output.stdout)
    {
        parse_unmounted_removable(text, &mut drives);
    }
    drives
}

pub fn parse_mountinfo(text: &str) -> Vec<Drive> {
    // `\040` no ponto de montagem é um espaço (e `\011` tab, `\134` barra invertida).
    let unescape = |s: &str| s.replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\");
    let mut out: Vec<Drive> = Vec::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let fields: Vec<&str> = left.split(' ').collect();
        let rest: Vec<&str> = right.split(' ').collect();
        let (Some(mount), Some(fstype), Some(device)) = (fields.get(4), rest.first(), rest.get(1)) else {
            continue;
        };
        let mount = unescape(mount);
        let user_mount = ["/media/", "/run/media/", "/mnt/"].iter().any(|p| mount.starts_with(p));
        // Pastas de passagem (o tmpfs de /media/fabio, que só segura os discos) não
        // contam.
        if !user_mount || mount.trim_end_matches('/').matches('/').count() < 2 || matches!(*fstype, "tmpfs" | "autofs") {
            continue;
        }
        if out.iter().any(|d| d.mount.as_deref() == Some(Path::new(&mount))) {
            continue;
        }
        let mount = PathBuf::from(mount);
        out.push(Drive { device: device.to_string(), label: mount.file_name().map(|n| n.to_string_lossy().into_owned()), mount: Some(mount), fstype: fstype.to_string() });
    }
    out
}

/// Volumes removíveis ainda desmontados também aparecem na Mesa: clicar neles pede
/// ao UDisks que monte o volume, como os ambientes gráficos comuns.
pub fn parse_unmounted_removable(text: &str, drives: &mut Vec<Drive>) {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else { return };
    fn visit(node: &serde_json::Value, inherited_removable: bool, drives: &mut Vec<Drive>) {
        let removable = node.get("rm").and_then(|v| v.as_bool().or_else(|| v.as_u64().map(|n| n != 0))).unwrap_or(false)
            || node.get("tran").and_then(|v| v.as_str()) == Some("usb")
            || inherited_removable;
        let path = node.get("path").and_then(|v| v.as_str());
        let kind = node.get("type").and_then(|v| v.as_str());
        let fstype = node.get("fstype").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
        let mounted = node.get("mountpoints").and_then(|v| v.as_array()).is_some_and(|a| a.iter().any(|p| p.as_str().is_some_and(|s| !s.is_empty())));
        if removable && matches!(kind, Some("part" | "disk")) && fstype.is_some() && !mounted
            && let Some(device) = path
            && !drives.iter().any(|d| d.device == device)
        {
            drives.push(Drive {
                device: device.to_string(), mount: None, fstype: fstype.unwrap().to_string(),
                label: node.get("label").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string),
            });
        }
        if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
            for child in children { visit(child, removable, drives); }
        }
    }
    if let Some(devices) = root.get("blockdevices").and_then(|v| v.as_array()) {
        for device in devices { visit(device, false, drives); }
    }
}

/// Ícone do tema para um disco: CD/DVD ou removível.
/// Nome do disco para mostrar: o rótulo, ou o aparelho.
pub fn drive_name(d: &Drive) -> String {
    d.label.clone().unwrap_or_else(|| d.device.rsplit('/').next().unwrap_or(&d.device).to_string())
}

pub fn drive_icon(d: &Drive) -> &'static str {
    match d.fstype.as_str() {
        "iso9660" | "udf" => "media-optical",
        _ => "drive-removable-media",
    }
}

/// Onde o disco aparece: o ponto de montagem, ou o aparelho ainda desmontado.
pub fn drive_path(drive: &Drive) -> PathBuf {
    drive.mount.clone().unwrap_or_else(|| PathBuf::from(&drive.device))
}

/// Abre o disco no gerenciador de arquivos (montando antes, se preciso).
pub fn open_drive(drive: &Drive) {
    if let Some(mount) = &drive.mount {
        places::open(&mount.to_string_lossy());
        return;
    }
    let device = drive.device.clone();
    std::thread::spawn(move || {
        let result = std::process::Command::new("udisksctl").args(["mount", "-b", &device]).output();
        match result {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(mount) = text.lines().find_map(|line| line.split_once(" at ").map(|(_, path)| path.trim_end_matches('.').trim())) {
                    places::open(mount);
                    tracing::info!("montado: {device} em {mount}");
                } else {
                    tracing::info!("montado: {device}");
                }
            }
            Ok(output) => tracing::warn!("não foi possível montar {device}: {}", String::from_utf8_lossy(&output.stderr).trim()),
            Err(err) => tracing::warn!("não foi possível montar {device}: {err}"),
        }
    });
}

/// Desmonta o disco (e desliga, se puder) numa thread: o desenho não espera.
pub fn eject(drive: Drive) {
    std::thread::spawn(move || {
        let unmounted = if drive.mount.is_some() {
            std::process::Command::new("udisksctl").args(["unmount", "-b", &drive.device]).status().is_ok_and(|s| s.success())
        } else {
            true
        };
        if unmounted {
            let powered_off = std::process::Command::new("udisksctl").args(["power-off", "-b", &drive.device]).status().is_ok_and(|s| s.success());
            if powered_off {
                tracing::info!("ejetado: {}", drive.mount.as_deref().unwrap_or(Path::new(&drive.device)).display());
            } else {
                tracing::warn!("desmontado, mas não foi possível ejetar fisicamente: {}", drive.device);
            }
        } else if let Some(mount) = drive.mount.as_ref()
            && !std::process::Command::new("gio").arg("mount").arg("-u").arg(mount).status().is_ok_and(|s| s.success())
        {
            tracing::warn!("não foi possível ejetar {}", mount.display());
        }
    });
}

