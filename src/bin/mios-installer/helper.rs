//! A parte do instalador que roda como root (`mios-installer --helper`): recebe o plano
//! (disco, idioma, teclado, fuso, usuário) em JSON pela entrada padrão e instala o mios
//! no disco, mandando o andamento pela saída padrão em linhas
//! `PROGRESS <0-100> <fase>`, `ERROR <mensagem>` e `DONE`.
//!
//! Passos: tabela GPT nova (EFI de 512 MiB + raiz ext4 no UEFI; partição de boot do BIOS
//! de 1 MiB + raiz no BIOS), o sistema do modo live descompactado do squashfs da ISO,
//! fstab, nome da máquina, idioma, teclado, fuso, o usuário (no grupo sudo), o login pelo
//! greeter do mide, os pacotes do modo live tirados e o GRUB instalado.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// O que instalar, escolhido nas telas do instalador.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Plan {
    /// Disco inteiro (`/dev/sda`, `/dev/nvme0n1`): tudo nele é apagado.
    pub disk: String,
    /// "en_US.UTF-8", "pt_BR.UTF-8"...
    pub locale: String,
    /// Layout do teclado do xkb ("us", "br").
    pub keyboard: String,
    /// "America/Sao_Paulo"...
    pub timezone: String,
    pub fullname: String,
    pub username: String,
    pub password: String,
    pub hostname: String,
    pub autologin: bool,
}

const TARGET: &str = "/mnt/mios-target";
/// O sistema do modo live (a mídia montada pelo live-boot).
const SQUASHFS: &str = "/run/live/medium/live/filesystem.squashfs";

fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn progress(percent: u32, stage: &str) {
    say(&format!("PROGRESS {percent} {stage}"));
}

/// Roda um programa; o erro leva a saída dele.
fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("{program} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Roda um programa dentro do sistema novo.
fn chroot(args: &[&str]) -> Result<String, String> {
    let mut all = vec![TARGET];
    all.extend_from_slice(args);
    run("chroot", &all)
}

fn write(path: &str, text: &str) -> Result<(), String> {
    let full = Path::new(TARGET).join(path.trim_start_matches('/'));
    if let Some(dir) = full.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&full, text).map_err(|e| format!("{}: {e}", full.display()))
}

/// Nome do dispositivo da partição `n` (`/dev/sda2`, `/dev/nvme0n1p2`).
pub fn partition(disk: &str, n: u32) -> String {
    if disk.chars().last().is_some_and(|c| c.is_ascii_digit()) { format!("{disk}p{n}") } else { format!("{disk}{n}") }
}

/// Usuário válido para o Linux (letras minúsculas, números, - e _).
pub fn valid_username(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_') && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn efi() -> bool {
    Path::new("/sys/firmware/efi").is_dir()
}

fn uuid(device: &str) -> Result<String, String> {
    Ok(run("blkid", &["-s", "UUID", "-o", "value", device])?.trim().to_string())
}

/// Instala; devolve o erro do passo que falhou.
pub fn install(plan: &Plan) -> Result<(), String> {
    if !valid_username(&plan.username) {
        return Err(format!("invalid user name: {}", plan.username));
    }
    if !Path::new(&plan.disk).exists() {
        return Err(format!("disk not found: {}", plan.disk));
    }
    let uefi = efi();
    let (boot_part, root_part) = (partition(&plan.disk, 1), partition(&plan.disk, 2));

    progress(2, "partitioning");
    // Nada montado do disco escolhido (o live monta pendrives sozinho).
    let mounted = run("lsblk", &["-nrpo", "MOUNTPOINT", &plan.disk]).unwrap_or_default();
    for m in mounted.lines().filter(|m| !m.trim().is_empty()) {
        let _ = run("umount", &["-R", m.trim()]);
    }
    let _ = run("swapoff", &["-a"]);
    run("wipefs", &["-a", &plan.disk])?;
    let layout = if uefi {
        "label: gpt\n,512M,C12A7328-F81F-11D2-BA4B-00A0C93EC93B,\n,,0FC63DAF-8483-4772-8E79-3D69D8477DE4,\n"
    } else {
        "label: gpt\n,1M,21686148-6449-6E6F-744E-656564454649,\n,,0FC63DAF-8483-4772-8E79-3D69D8477DE4,\n"
    };
    let mut sfdisk = Command::new("sfdisk").args(["--wipe", "always", &plan.disk]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| format!("sfdisk: {e}"))?;
    sfdisk.stdin.take().ok_or("sfdisk")?.write_all(layout.as_bytes()).map_err(|e| e.to_string())?;
    let out = sfdisk.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("sfdisk: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let _ = run("partprobe", &[&plan.disk]);
    let _ = run("udevadm", &["settle"]);

    progress(6, "formatting");
    if uefi {
        run("mkfs.vfat", &["-F", "32", "-n", "EFI", &boot_part])?;
    }
    run("mkfs.ext4", &["-F", "-q", "-L", "mios", &root_part])?;

    progress(8, "mounting");
    std::fs::create_dir_all(TARGET).map_err(|e| e.to_string())?;
    run("mount", &[&root_part, TARGET])?;
    if uefi {
        std::fs::create_dir_all(format!("{TARGET}/boot/efi")).map_err(|e| e.to_string())?;
        run("mount", &[&boot_part, &format!("{TARGET}/boot/efi")])?;
    }

    progress(10, "copying");
    copy_system()?;

    progress(80, "configuring");
    for (src, dst) in [("/dev", "dev"), ("/dev/pts", "dev/pts"), ("/proc", "proc"), ("/sys", "sys"), ("/run", "run")] {
        let d = format!("{TARGET}/{dst}");
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        run("mount", &["--bind", src, &d])?;
    }
    if uefi && Path::new("/sys/firmware/efi/efivars").is_dir() {
        let _ = run("mount", &["--bind", "/sys/firmware/efi/efivars", &format!("{TARGET}/sys/firmware/efi/efivars")]);
    }
    let mut fstab = format!("# /etc/fstab: gerado pelo instalador do mios\nUUID={} / ext4 errors=remount-ro 0 1\n", uuid(&root_part)?);
    if uefi {
        fstab.push_str(&format!("UUID={} /boot/efi vfat umask=0077 0 1\n", uuid(&boot_part)?));
    }
    write("/etc/fstab", &fstab)?;
    let host = if plan.hostname.trim().is_empty() { "mios".to_string() } else { plan.hostname.trim().to_string() };
    write("/etc/hostname", &format!("{host}\n"))?;
    write("/etc/hosts", &format!("127.0.0.1\tlocalhost\n127.0.1.1\t{host}\n::1\tlocalhost ip6-localhost ip6-loopback\n"))?;
    let locale = if plan.locale.is_empty() { "en_US.UTF-8" } else { &plan.locale };
    write("/etc/default/locale", &format!("LANG={locale}\n"))?;
    let keyboard = if plan.keyboard.is_empty() { "us" } else { &plan.keyboard };
    write("/etc/default/keyboard", &format!("XKBMODEL=\"pc105\"\nXKBLAYOUT=\"{keyboard}\"\nXKBVARIANT=\"\"\nXKBOPTIONS=\"\"\nBACKSPACE=\"guess\"\n"))?;
    if !plan.timezone.is_empty() && Path::new("/usr/share/zoneinfo").join(&plan.timezone).is_file() {
        let link = format!("{TARGET}/etc/localtime");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(format!("/usr/share/zoneinfo/{}", plan.timezone), &link).map_err(|e| e.to_string())?;
        write("/etc/timezone", &format!("{}\n", plan.timezone))?;
    }

    progress(84, "user");
    let groups: Vec<&str> = ["sudo", "audio", "video", "netdev", "plugdev", "bluetooth", "lpadmin", "scanner", "users"]
        .into_iter()
        .filter(|g| chroot(&["getent", "group", g]).is_ok())
        .collect();
    chroot(&["useradd", "-m", "-s", "/bin/bash", "-c", &plan.fullname, "-G", &groups.join(","), &plan.username])?;
    let mut chpasswd = Command::new("chroot").args([TARGET, "chpasswd"]).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    chpasswd.stdin.take().ok_or("chpasswd")?.write_all(format!("{}:{}\n", plan.username, plan.password).as_bytes()).map_err(|e| e.to_string())?;
    if !chpasswd.wait().map_err(|e| e.to_string())?.success() {
        return Err("chpasswd failed".into());
    }
    // Teclado também no mide do usuário (a sessão lê o config.toml; `layouts` é um
    // texto com os layouts separados por vírgula, como "br,us:intl").
    let config = format!("{TARGET}/home/{}/.config/mide/config.toml", plan.username);
    if let Ok(text) = std::fs::read_to_string(&config) {
        let _ = std::fs::write(&config, format!("{text}\n[keyboard]\nlayouts = \"{keyboard}\"\n"));
        let _ = chroot(&["chown", "-R", &format!("{0}:{0}", plan.username), &format!("/home/{}/.config", plan.username)]);
    }

    progress(86, "login");
    // Login pelo greeter do mide; entrar sem senha só se escolhido.
    let mut greetd = String::from("[terminal]\nvt = 7\n\n[default_session]\ncommand = \"env XDG_CONFIG_HOME=/etc/mide/login XDG_STATE_HOME=/var/lib/mide-login XDG_DATA_DIRS=/usr/local/share:/usr/share MIDE_BACKEND=drm /usr/bin/mide --login\"\nuser = \"_greetd\"\n");
    if plan.autologin {
        greetd.push_str(&format!(
            "\n[initial_session]\ncommand = \"env XDG_SESSION_TYPE=wayland XDG_SESSION_DESKTOP=mide XDG_CURRENT_DESKTOP=mide /usr/bin/mide-session\"\nuser = \"{}\"\n",
            plan.username
        ));
    }
    write("/etc/greetd/config.toml", &greetd)?;
    let _ = std::fs::remove_file(format!("{TARGET}/etc/systemd/system/greetd.service.d/mios.conf"));

    progress(88, "cleaning");
    // O que só vale no modo live: o atalho e o início automático do instalador e o
    // serviço do teste automático.
    for f in ["/usr/share/applications/mios-installer.desktop", "/etc/xdg/autostart/mios-installer-live.desktop", "/etc/systemd/system/mios-autoinstall.service", "/etc/systemd/system/multi-user.target.wants/mios-autoinstall.service", "/usr/lib/mios/autoinstall"] {
        let _ = std::fs::remove_file(format!("{TARGET}{f}"));
    }
    // Sem o modo live no sistema instalado.
    let _ = chroot(&["env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "purge", "-y", "-q", "live-boot", "live-boot-initramfs-tools", "live-config", "live-config-systemd", "user-setup"]);
    let _ = chroot(&["env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "autoremove", "-y", "-q"]);

    progress(92, "bootloader");
    if uefi {
        // Sem conseguir gravar a entrada no firmware (NVRAM só de leitura), instala do
        // mesmo jeito: a cópia no caminho padrão (abaixo) inicia sozinha.
        if chroot(&["grub-install", "--target=x86_64-efi", "--efi-directory=/boot/efi", "--bootloader-id=mios", "--recheck"]).is_err() {
            chroot(&["grub-install", "--target=x86_64-efi", "--efi-directory=/boot/efi", "--bootloader-id=mios", "--recheck", "--no-nvram"])?;
        }
        // Também no caminho padrão do UEFI (firmwares que esquecem a entrada da NVRAM).
        let _ = chroot(&["grub-install", "--target=x86_64-efi", "--efi-directory=/boot/efi", "--removable"]);
    } else {
        chroot(&["grub-install", "--target=i386-pc", "--recheck", &plan.disk])?;
    }
    progress(95, "initramfs");
    chroot(&["update-initramfs", "-u", "-k", "all"])?;
    chroot(&["update-grub"])?;

    progress(99, "finishing");
    let _ = run("sync", &[]);
    let _ = run("umount", &["-R", TARGET]);
    Ok(())
}

/// Descompacta o sistema do modo live no disco novo, com o andamento (10 a 80%).
fn copy_system() -> Result<(), String> {
    let mut child = Command::new("unsquashfs")
        .args(["-f", "-percentage", "-d", TARGET, SQUASHFS])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("unsquashfs: {e}"))?;
    let stdout = child.stdout.take().ok_or("unsquashfs")?;
    let mut last = 0;
    for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Ok(p) = line.trim().parse::<u32>()
            && p != last
        {
            last = p;
            progress(10 + p * 70 / 100, "copying");
        }
    }
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    if !child.wait().map_err(|e| e.to_string())?.success() {
        return Err(format!("unsquashfs: {}", err.trim()));
    }
    Ok(())
}

/// Modo `--helper`: o plano em JSON na entrada.
pub fn main_helper() -> ! {
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    let plan: Plan = match serde_json::from_str(&text) {
        Ok(p) => p,
        Err(e) => {
            say(&format!("ERROR plan: {e}"));
            std::process::exit(2);
        }
    };
    match install(&plan) {
        Ok(()) => {
            say("DONE");
            std::process::exit(0)
        }
        Err(e) => {
            let _ = run("umount", &["-R", TARGET]);
            say(&format!("ERROR {e}"));
            std::process::exit(1)
        }
    }
}

/// Modo `--auto` (testes no QEMU, pelo parâmetro `mios.autoinstall=/dev/vda` do kernel):
/// instala com um plano fixo (usuário "mios", senha "mios") e desliga.
pub fn main_auto(disk: &str) -> ! {
    let plan = Plan {
        disk: disk.to_string(),
        locale: "pt_BR.UTF-8".into(),
        keyboard: "br".into(),
        timezone: "America/Sao_Paulo".into(),
        fullname: "mios".into(),
        username: "mios".into(),
        password: "mios".into(),
        hostname: "mios".into(),
        autologin: true,
    };
    let result = install(&plan);
    match &result {
        Ok(()) => say("DONE"),
        Err(e) => say(&format!("ERROR {e}")),
    }
    let _ = run("systemctl", &["poweroff"]);
    std::process::exit(if result.is_ok() { 0 } else { 1 })
}

/// O disco onde está a mídia do modo live (não pode ser escolhido).
pub fn live_medium_disk() -> Option<PathBuf> {
    let src = run("findmnt", &["-no", "SOURCE", "/run/live/medium"]).ok()?;
    let disk = run("lsblk", &["-nrpo", "PKNAME", src.trim()]).ok()?;
    let disk = disk.trim();
    Some(PathBuf::from(if disk.is_empty() { src.trim() } else { disk }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nomes_das_particoes() {
        assert_eq!(partition("/dev/sda", 2), "/dev/sda2");
        assert_eq!(partition("/dev/nvme0n1", 1), "/dev/nvme0n1p1");
        assert_eq!(partition("/dev/vda", 1), "/dev/vda1");
        assert_eq!(partition("/dev/mmcblk0", 2), "/dev/mmcblk0p2");
    }

    #[test]
    fn nome_de_usuario_valido() {
        assert!(valid_username("fabio"));
        assert!(valid_username("ana_maria-2"));
        assert!(!valid_username("Fabio"));
        assert!(!valid_username("1fabio"));
        assert!(!valid_username("ana maria"));
        assert!(!valid_username(""));
    }

    #[test]
    fn plano_em_json() {
        let p = Plan { disk: "/dev/vda".into(), username: "mios".into(), ..Default::default() };
        let back: Plan = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }
}
