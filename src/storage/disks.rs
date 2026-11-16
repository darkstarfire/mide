//! Discos pelo UDisks2 (D-Bus do sistema), para o app Discos e o instalador do mios:
//! discos e partições, montar, formatar, particionar, ejetar e gravar imagens em
//! pendrives. O UDisks pede a senha de administrador pelo polkit quando precisa (a
//! janela do mide); o mide não roda nada como root. As funções bloqueiam: os apps as
//! chamam numa thread.

use std::collections::HashMap;
use std::os::fd::OwnedFd;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const SERVICE: &str = "org.freedesktop.UDisks2";
const IFACE: &str = "org.freedesktop.UDisks2";

/// Um disco (pendrive, SSD, HD, leitor de cartão).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Disk {
    /// Objeto do UDisks do disco (`/org/freedesktop/UDisks2/drives/...`).
    pub drive: String,
    /// Objeto do bloco do disco inteiro (`.../block_devices/sda`).
    pub block: String,
    /// Arquivo do dispositivo (`/dev/sda`).
    pub device: String,
    pub vendor: String,
    pub model: String,
    pub size: u64,
    pub removable: bool,
    pub ejectable: bool,
    pub can_power_off: bool,
    /// Ligação: "usb", "sdio", "ata", "nvme" (vazio quando não se sabe).
    pub bus: String,
    pub optical: bool,
    /// Tabela de partições: "gpt", "dos" ou vazio (sem tabela).
    pub table: String,
    /// Partições (ou o disco inteiro, quando o sistema de arquivos está direto nele).
    pub volumes: Vec<Volume>,
    /// Disco onde está o sistema que está rodando.
    pub system: bool,
}

impl Disk {
    /// Nome para mostrar: fabricante e modelo, ou o dispositivo.
    pub fn name(&self) -> String {
        let name = format!("{} {}", self.vendor.trim(), self.model.trim()).trim().to_string();
        if name.is_empty() { self.device.clone() } else { name }
    }

    /// Espaços livres (início, tamanho) entre as partições, de pelo menos 1 MiB.
    pub fn free_regions(&self) -> Vec<(u64, u64)> {
        if self.table.is_empty() {
            return Vec::new();
        }
        let mut parts: Vec<(u64, u64)> = self.volumes.iter().filter(|v| v.number > 0).map(|v| (v.offset, v.size)).collect();
        parts.sort();
        let mut out = Vec::new();
        // O primeiro MiB fica para a tabela; o fim, para a cópia da tabela GPT.
        let mut cursor = MIB;
        let end = self.size.saturating_sub(if self.table == "gpt" { MIB } else { 0 });
        for (offset, size) in parts {
            if offset > cursor + MIB {
                out.push((cursor, offset - cursor));
            }
            cursor = cursor.max(offset + size);
        }
        if end > cursor + MIB {
            out.push((cursor, end - cursor));
        }
        out
    }
}

/// Uma partição (ou o sistema de arquivos de um disco sem tabela).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Volume {
    /// Objeto do bloco da partição.
    pub block: String,
    pub device: String,
    /// Número da partição (0 = o disco inteiro, sem tabela).
    pub number: u32,
    pub offset: u64,
    pub size: u64,
    /// Sistema de arquivos ("ext4", "vfat", "ntfs", "exfat", "btrfs", "swap", "crypto_LUKS"...).
    pub fstype: String,
    pub label: String,
    /// Nome da partição na tabela GPT.
    pub name: String,
    pub mount_points: Vec<String>,
    /// Bytes usados (só montada).
    pub used: Option<u64>,
    /// Partição do sistema (EFI, a raiz que está rodando...).
    pub system: bool,
}

impl Volume {
    pub fn title(&self) -> String {
        if !self.label.is_empty() {
            self.label.clone()
        } else if !self.name.is_empty() {
            self.name.clone()
        } else {
            self.device.trim_start_matches("/dev/").to_string()
        }
    }
}

pub const MIB: u64 = 1024 * 1024;

/// Erro de uma operação.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Unavailable(String),
    /// A senha de administrador não foi dada.
    NotAuthorized,
    Failed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unavailable(e) => write!(f, "UDisks: {e}"),
            Error::NotAuthorized => write!(f, "not authorized"),
            Error::Failed(e) => write!(f, "{e}"),
        }
    }
}

fn map_err(e: zbus::Error) -> Error {
    match &e {
        zbus::Error::MethodError(name, msg, _) => {
            let n = name.as_str();
            if n.contains("NotAuthorized") || n.contains("NotAuthorizedCanObtain") || n.contains("NotAuthorizedDismissed") {
                Error::NotAuthorized
            } else {
                Error::Failed(msg.clone().unwrap_or_else(|| n.to_string()))
            }
        }
        _ => Error::Failed(e.to_string()),
    }
}

type Props = HashMap<String, OwnedValue>;
type Objects = HashMap<String, HashMap<String, Props>>;

fn connection() -> Result<Connection, Error> {
    Connection::system().map_err(|e| Error::Unavailable(e.to_string()))
}

fn objects(conn: &Connection) -> Result<Objects, Error> {
    let om = zbus::blocking::fdo::ObjectManagerProxy::builder(conn)
        .destination(SERVICE)
        .and_then(|b| b.path("/org/freedesktop/UDisks2"))
        .and_then(|b| b.build())
        .map_err(|e| Error::Unavailable(e.to_string()))?;
    let all = om.get_managed_objects().map_err(|e| Error::Unavailable(e.to_string()))?;
    Ok(all.into_iter().map(|(path, ifaces)| (path.to_string(), ifaces.into_iter().map(|(i, p)| (i.to_string(), p)).collect())).collect())
}

fn get<'a>(o: &'a HashMap<String, Props>, iface: &str, name: &str) -> Option<&'a OwnedValue> {
    o.get(&format!("{IFACE}.{iface}"))?.get(name)
}
fn u64_of(o: &HashMap<String, Props>, iface: &str, name: &str) -> u64 {
    get(o, iface, name).and_then(|v| u64::try_from(&**v).ok()).unwrap_or(0)
}
fn u32_of(o: &HashMap<String, Props>, iface: &str, name: &str) -> u32 {
    get(o, iface, name).and_then(|v| u32::try_from(&**v).ok()).unwrap_or(0)
}
fn bool_of(o: &HashMap<String, Props>, iface: &str, name: &str) -> bool {
    get(o, iface, name).and_then(|v| bool::try_from(&**v).ok()).unwrap_or(false)
}
fn str_of(o: &HashMap<String, Props>, iface: &str, name: &str) -> String {
    get(o, iface, name).and_then(|v| <&str>::try_from(&**v).ok().map(str::to_string)).unwrap_or_default()
}
fn path_of(o: &HashMap<String, Props>, iface: &str, name: &str) -> String {
    get(o, iface, name).and_then(|v| zbus::zvariant::ObjectPath::try_from(&**v).ok().map(|p| p.to_string())).unwrap_or_default()
}
/// Texto de um `ay` terminado em zero (caminhos do UDisks).
fn bytes_to_string(v: &Value) -> Option<String> {
    let bytes: Vec<u8> = v.try_clone().ok()?.try_into().ok()?;
    Some(String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_string())
}
fn mount_points(o: &HashMap<String, Props>) -> Vec<String> {
    let Some(v) = get(o, "Filesystem", "MountPoints") else {
        return Vec::new();
    };
    match &**v {
        Value::Array(a) => a.iter().filter_map(bytes_to_string).filter(|s| !s.is_empty()).collect(),
        _ => Vec::new(),
    }
}

/// Bytes usados de um ponto de montagem.
fn used_bytes(path: &str) -> Option<u64> {
    let st = rustix::fs::statvfs(path).ok()?;
    Some((st.f_blocks - st.f_bfree) * st.f_frsize)
}

/// Os discos do computador, com as partições (sem loop, ram e zram).
pub fn disks() -> Result<Vec<Disk>, Error> {
    let conn = connection()?;
    let objs = objects(&conn)?;
    let mut by_drive: HashMap<String, Disk> = HashMap::new();
    for (path, o) in &objs {
        if !o.contains_key(&format!("{IFACE}.Drive")) {
            continue;
        }
        by_drive.insert(
            path.clone(),
            Disk {
                drive: path.clone(),
                vendor: str_of(o, "Drive", "Vendor"),
                model: str_of(o, "Drive", "Model"),
                size: u64_of(o, "Drive", "Size"),
                removable: bool_of(o, "Drive", "Removable") || bool_of(o, "Drive", "MediaRemovable"),
                ejectable: bool_of(o, "Drive", "Ejectable"),
                can_power_off: bool_of(o, "Drive", "CanPowerOff"),
                bus: str_of(o, "Drive", "ConnectionBus"),
                optical: bool_of(o, "Drive", "Optical"),
                ..Default::default()
            },
        );
    }
    // Blocos: o do disco inteiro (sem Partition) e as partições.
    for (path, o) in &objs {
        if !o.contains_key(&format!("{IFACE}.Block")) {
            continue;
        }
        let drive = path_of(o, "Block", "Drive");
        let Some(disk) = by_drive.get_mut(&drive) else { continue };
        let device = get(o, "Block", "Device").and_then(|v| bytes_to_string(v)).unwrap_or_default();
        let is_partition = o.contains_key(&format!("{IFACE}.Partition"));
        let mounts = mount_points(o);
        let system = bool_of(o, "Block", "HintSystem") || mounts.iter().any(|m| m == "/" || m == "/boot" || m == "/boot/efi" || m == "/usr");
        let fstype = str_of(o, "Block", "IdType");
        if !is_partition {
            disk.block = path.clone();
            disk.device = device.clone();
            disk.table = str_of(o, "PartitionTable", "Type");
            if disk.size == 0 {
                disk.size = u64_of(o, "Block", "Size");
            }
            // Sistema de arquivos direto no disco (pendrive sem tabela).
            if !fstype.is_empty() && disk.table.is_empty() {
                disk.volumes.push(Volume {
                    block: path.clone(),
                    device,
                    number: 0,
                    offset: 0,
                    size: u64_of(o, "Block", "Size"),
                    label: str_of(o, "Block", "IdLabel"),
                    used: mounts.first().and_then(|m| used_bytes(m)),
                    mount_points: mounts,
                    fstype,
                    system,
                    ..Default::default()
                });
            }
            continue;
        }
        if bool_of(o, "Partition", "IsContainer") {
            continue;
        }
        disk.volumes.push(Volume {
            block: path.clone(),
            device,
            number: u32_of(o, "Partition", "Number"),
            offset: u64_of(o, "Partition", "Offset"),
            size: u64_of(o, "Partition", "Size"),
            label: str_of(o, "Block", "IdLabel"),
            name: str_of(o, "Partition", "Name"),
            used: mounts.first().and_then(|m| used_bytes(m)),
            mount_points: mounts,
            fstype,
            system,
        });
    }
    let mut out: Vec<Disk> = by_drive
        .into_values()
        .filter(|d| !d.block.is_empty() && !d.device.contains("/dev/loop") && !d.device.contains("/dev/zram"))
        .map(|mut d| {
            d.volumes.sort_by_key(|v| v.offset);
            d.system = d.volumes.iter().any(|v| v.system);
            d
        })
        .collect();
    // Internos primeiro, depois os removíveis; pelo dispositivo.
    out.sort_by(|a, b| (a.removable, &a.device).cmp(&(b.removable, &b.device)));
    Ok(out)
}

fn options(pairs: &[(&str, Value)]) -> HashMap<String, Value<'static>> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.try_to_owned().map(Value::from).unwrap_or(Value::from(false)))).collect()
}

fn proxy<'a>(conn: &'a Connection, path: &str, iface: &str) -> Result<Proxy<'a>, Error> {
    Proxy::new(conn, SERVICE, path.to_string(), format!("{IFACE}.{iface}")).map_err(|e| Error::Unavailable(e.to_string()))
}

/// Monta um volume e devolve onde.
pub fn mount(block: &str) -> Result<String, Error> {
    let conn = connection()?;
    proxy(&conn, block, "Filesystem")?.call("Mount", &(options(&[]),)).map_err(map_err)
}

/// Desmonta um volume.
pub fn unmount(block: &str) -> Result<(), Error> {
    let conn = connection()?;
    proxy(&conn, block, "Filesystem")?.call("Unmount", &(options(&[]),)).map_err(map_err)
}

/// Formata um bloco (partição ou disco) com `fstype` ("vfat", "exfat", "ntfs", "ext4",
/// "btrfs", ou "gpt"/"dos" para uma tabela nova) e o nome `label`.
pub fn format(block: &str, fstype: &str, label: &str) -> Result<(), Error> {
    let conn = connection()?;
    let mut opts = vec![("tear-down", Value::from(true))];
    if !label.is_empty() {
        opts.push(("label", Value::from(label)));
    }
    if !matches!(fstype, "gpt" | "dos") {
        opts.push(("update-partition-type", Value::from(true)));
    }
    proxy(&conn, block, "Block")?.call("Format", &(fstype, options(&opts))).map_err(map_err)
}

/// Cria uma partição de `size` bytes em `offset` (0 = o resto do espaço livre) já
/// formatada com `fstype` e `label`. `part_type` é o tipo na tabela ("" = o padrão do
/// UDisks; "0x0c" é FAT32 numa tabela MBR). Devolve o objeto do bloco novo.
pub fn create_partition(disk_block: &str, offset: u64, size: u64, part_type: &str, fstype: &str, label: &str) -> Result<String, Error> {
    let conn = connection()?;
    let mut fs_opts = vec![("tear-down", Value::from(true))];
    if !label.is_empty() {
        fs_opts.push(("label", Value::from(label)));
    }
    let path: OwnedObjectPath = proxy(&conn, disk_block, "PartitionTable")?
        .call("CreatePartitionAndFormat", &(offset, size, part_type, "", options(&[]), fstype, options(&fs_opts)))
        .map_err(map_err)?;
    Ok(path.to_string())
}

/// Apaga uma partição.
pub fn delete_partition(block: &str) -> Result<(), Error> {
    let conn = connection()?;
    proxy(&conn, block, "Partition")?.call("Delete", &(options(&[("tear-down", Value::from(true))]),)).map_err(map_err)
}

/// Muda o nome (rótulo) do sistema de arquivos.
pub fn set_label(block: &str, label: &str) -> Result<(), Error> {
    let conn = connection()?;
    proxy(&conn, block, "Filesystem")?.call("SetLabel", &(label, options(&[]))).map_err(map_err)
}

/// Ejeta a mídia (CD) ou desliga o disco (pendrive), para tirar com segurança.
pub fn eject(disk: &Disk) -> Result<(), Error> {
    let conn = connection()?;
    for v in &disk.volumes {
        if !v.mount_points.is_empty() {
            unmount(&v.block)?;
        }
    }
    let drive = proxy(&conn, &disk.drive, "Drive")?;
    if disk.can_power_off {
        drive.call("PowerOff", &(options(&[]),)).map_err(map_err)
    } else {
        drive.call("Eject", &(options(&[]),)).map_err(map_err)
    }
}

/// Abre o disco inteiro para gravar uma imagem por cima (o polkit pede a senha). O que
/// chamou escreve no descritor devolvido.
pub fn open_for_restore(disk_block: &str) -> Result<OwnedFd, Error> {
    let conn = connection()?;
    let fd: zbus::zvariant::OwnedFd = proxy(&conn, disk_block, "Block")?.call("OpenForRestore", &(options(&[]),)).map_err(map_err)?;
    Ok(fd.into())
}

/// Monta uma imagem (ISO) como disco, só para leitura; devolve o objeto do bloco.
pub fn loop_setup(file: &std::path::Path) -> Result<String, Error> {
    loop_setup_with(file, true)
}

/// Como `loop_setup`, podendo escrever (`read_only` falso): um arquivo vira um disco de
/// teste, que o próprio usuário formata e particiona sem a senha de administrador.
pub fn loop_setup_with(file: &std::path::Path, read_only: bool) -> Result<String, Error> {
    let conn = connection()?;
    let f = std::fs::OpenOptions::new().read(true).write(!read_only).open(file).map_err(|e| Error::Failed(e.to_string()))?;
    let fd = zbus::zvariant::Fd::from(&f);
    let path: OwnedObjectPath = proxy(&conn, "/org/freedesktop/UDisks2/Manager", "Manager")?.call("LoopSetup", &(fd, options(&[("read-only", Value::from(read_only))]))).map_err(map_err)?;
    Ok(path.to_string())
}

/// Solta uma imagem montada por `loop_setup`.
pub fn loop_delete(block: &str) -> Result<(), Error> {
    let conn = connection()?;
    proxy(&conn, block, "Loop")?.call("Delete", &(options(&[]),)).map_err(map_err)
}

/// O bloco tem um sistema de arquivos montável (depois de `loop_setup`, ele aparece
/// alguns instantes depois); espera até `tries` vezes de 200 ms.
pub fn wait_filesystem(block: &str, tries: u32) -> bool {
    for _ in 0..tries {
        if let Ok(conn) = connection()
            && let Ok(objs) = objects(&conn)
            && objs.get(block).is_some_and(|o| o.contains_key(&format!("{IFACE}.Filesystem")))
        {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn espacos_livres_entre_as_particoes() {
        let gib = 1024 * MIB;
        let disk = Disk {
            size: 16 * gib,
            table: "gpt".into(),
            volumes: vec![
                Volume { number: 1, offset: MIB, size: 512 * MIB, ..Default::default() },
                Volume { number: 2, offset: 2 * gib, size: 4 * gib, ..Default::default() },
            ],
            ..Default::default()
        };
        let free = disk.free_regions();
        assert_eq!(free.len(), 2);
        assert_eq!(free[0], (513 * MIB, 2 * gib - 513 * MIB));
        assert_eq!(free[1].0, 6 * gib);
        // Sem tabela, nada de espaço livre (é preciso criar a tabela antes).
        assert!(Disk { size: gib, ..Default::default() }.free_regions().is_empty());
    }

    /// Disco de teste (um arquivo ligado como loop pelo UDisks): tabela GPT, uma
    /// partição exFAT, montar, desmontar e soltar. Sem senha (o loop é do usuário).
    /// `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn particiona_e_formata_um_disco_de_teste() {
        let file = std::env::temp_dir().join(format!("mide-disco-{}.img", std::process::id()));
        std::fs::File::create(&file).unwrap().set_len(64 * MIB).unwrap();
        let block = loop_setup_with(&file, false).unwrap();
        format(&block, "gpt", "").unwrap();
        let part = create_partition(&block, MIB, 0, "", "exfat", "TESTE").unwrap();
        assert!(wait_filesystem(&part, 25));
        let at = mount(&part).unwrap();
        assert!(std::path::Path::new(&at).is_dir());
        unmount(&part).unwrap();
        delete_partition(&part).unwrap();
        let _ = loop_delete(&block);
        let _ = std::fs::remove_file(&file);
    }

    /// Precisa do UDisks2 na máquina (só leitura): `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn le_os_discos_do_computador() {
        let disks = disks().unwrap();
        assert!(!disks.is_empty());
        assert!(disks.iter().any(|d| d.system && !d.volumes.is_empty()));
    }
}
