//! Pacotes do sistema pelo PackageKit (D-Bus do sistema): atualizações, busca,
//! instalar e remover. O PackageKit fala com o gerenciador da distribuição (o apt no
//! mios) e pede a senha de administrador pelo polkit (a janela do `mide-polkit-agent`),
//! então o mide nunca roda nada como root nem guarda senha.
//!
//! Cada operação é uma transação: `CreateTransaction` dá o caminho dela, os sinais
//! (`Package`, `Details`, `UpdateDetail`, `ErrorCode`, `Finished`) chegam até o fim e o
//! progresso vem nas propriedades `Percentage` e `Status`. As funções aqui bloqueiam:
//! os apps as chamam numa thread.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SERVICE: &str = "org.freedesktop.PackageKit";
const TRANSACTION: &str = "org.freedesktop.PackageKit.Transaction";

/// Filtros (bits do `PkFilterEnum`).
pub const FILTER_NONE: u64 = 0;
pub const FILTER_INSTALLED: u64 = 1 << 2;
pub const FILTER_NOT_INSTALLED: u64 = 1 << 3;
pub const FILTER_NEWEST: u64 = 1 << 16;
pub const FILTER_ARCH: u64 = 1 << 18;
/// Só pacotes confiáveis (assinados), em toda instalação e atualização.
const FLAG_ONLY_TRUSTED: u64 = 1 << 1;

/// Tipo de um pacote numa lista (`PkInfoEnum`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Info {
    Installed,
    Available,
    Low,
    Enhancement,
    Normal,
    Bugfix,
    Important,
    Security,
    Other(u32),
}

impl Info {
    fn from(n: u32) -> Self {
        match n {
            1 => Info::Installed,
            2 => Info::Available,
            3 => Info::Low,
            4 => Info::Enhancement,
            5 => Info::Normal,
            6 => Info::Bugfix,
            7 => Info::Important,
            8 => Info::Security,
            n => Info::Other(n),
        }
    }
}

/// Um pacote: o id do PackageKit é `nome;versão;arquitetura;origem`.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub id: String,
    pub info: Info,
    pub summary: String,
}

impl Package {
    fn part(&self, i: usize) -> &str {
        self.id.split(';').nth(i).unwrap_or_default()
    }
    pub fn name(&self) -> &str {
        self.part(0)
    }
    pub fn version(&self) -> &str {
        self.part(1)
    }
    pub fn arch(&self) -> &str {
        self.part(2)
    }
}

/// Detalhes de uma atualização (notas e changelog, quando a distribuição dá).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateDetail {
    pub id: String,
    pub update_text: String,
    pub changelog: String,
    pub restart: bool,
}

/// Detalhes de um pacote (para a Loja): descrição, site e tamanho para baixar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub id: String,
    pub summary: String,
    pub description: String,
    pub url: String,
    pub license: String,
    pub size: u64,
}

/// O que está acontecendo numa transação, para a barra de progresso.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// 0 a 100, ou `None` quando não se sabe (o PackageKit manda 101).
    pub percent: Option<u32>,
    pub stage: Stage,
}

/// Fase da transação (resumo do `PkStatusEnum`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Waiting,
    Authenticating,
    Refreshing,
    Downloading,
    Installing,
    Removing,
    Updating,
    Finishing,
    Working,
}

impl Stage {
    fn from(status: u32) -> Self {
        match status {
            1 | 30 => Stage::Waiting,
            31 => Stage::Authenticating,
            7 | 20 | 21 | 22 | 25 | 27 => Stage::Refreshing,
            8 | 23 => Stage::Downloading,
            9 => Stage::Installing,
            6 => Stage::Removing,
            10 | 16 => Stage::Updating,
            11 | 18 => Stage::Finishing,
            _ => Stage::Working,
        }
    }
}

/// Como uma transação terminou mal.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// Sem o PackageKit (ou sem o D-Bus do sistema).
    Unavailable(String),
    /// A senha de administrador não foi dada (cancelado na janela do polkit).
    NotAuthorized,
    Cancelled,
    /// Erro do gerenciador de pacotes, com a mensagem dele.
    Failed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unavailable(e) => write!(f, "PackageKit: {e}"),
            Error::NotAuthorized => write!(f, "not authorized"),
            Error::Cancelled => write!(f, "cancelled"),
            Error::Failed(e) => write!(f, "{e}"),
        }
    }
}

/// O que chegou de uma transação.
#[derive(Default)]
struct Outcome {
    packages: Vec<Package>,
    update_details: Vec<UpdateDetail>,
    details: Vec<Details>,
}

/// Roda uma transação: chama `method` com `args` e junta os sinais até o fim;
/// `progress` recebe o andamento.
fn transaction<A>(method: &str, args: &A, mut progress: impl FnMut(Progress)) -> Result<Outcome, Error>
where
    A: serde::Serialize + zbus::zvariant::DynamicType,
{
    let unavailable = |e: zbus::Error| Error::Unavailable(e.to_string());
    let conn = Connection::system().map_err(unavailable)?;
    let root = Proxy::new(&conn, SERVICE, "/org/freedesktop/PackageKit", "org.freedesktop.PackageKit").map_err(unavailable)?;
    let path: OwnedObjectPath = root.call("CreateTransaction", &()).map_err(unavailable)?;
    let tx = Proxy::new(&conn, SERVICE, path.clone(), TRANSACTION).map_err(unavailable)?;
    // Idioma das mensagens e do changelog, como o da sessão.
    let lang = std::env::var("LANG").unwrap_or_else(|_| "C".into());
    let _: () = tx.call("SetHints", &(vec![format!("locale={lang}"), "interactive=true".to_string()],)).unwrap_or(());
    // Ouvir antes de chamar: os sinais podem chegar logo.
    let signals = tx.receive_all_signals().map_err(unavailable)?;
    let props = zbus::blocking::fdo::PropertiesProxy::builder(&conn).destination(SERVICE).map_err(unavailable)?.path(path.clone()).map_err(unavailable)?.build().map_err(unavailable)?;
    let changes = props.receive_properties_changed().map_err(unavailable)?;
    tx.call_method(method, args).map_err(|e| match &e {
        zbus::Error::MethodError(name, _, _) if name.as_str().contains("NotAuthorized") || name.as_str().contains("RefusedByPolicy") => Error::NotAuthorized,
        _ => Error::Failed(e.to_string()),
    })?;

    // O andamento vem por outra fila: uma thread repassa para cá.
    let (ptx, prx) = std::sync::mpsc::channel::<Progress>();
    std::thread::spawn(move || {
        let (mut percent, mut stage) = (None, Stage::Waiting);
        for change in changes {
            let Ok(args) = change.args() else { continue };
            for (key, value) in args.changed_properties() {
                match *key {
                    "Percentage" => percent = u32::try_from(value).ok().filter(|p| *p <= 100),
                    "Status" => stage = u32::try_from(value).map(Stage::from).unwrap_or(stage),
                    _ => {}
                }
            }
            if ptx.send(Progress { percent, stage }).is_err() {
                return;
            }
        }
    });

    let mut out = Outcome::default();
    let mut error: Option<(u32, String)> = None;
    for message in signals {
        while let Ok(p) = prx.try_recv() {
            progress(p);
        }
        let header = message.header();
        let Some(member) = header.member() else { continue };
        let body = message.body();
        match member.as_str() {
            "Package" => {
                if let Ok((info, id, summary)) = body.deserialize::<(u32, String, String)>() {
                    out.packages.push(Package { id, info: Info::from(info), summary });
                }
            }
            "UpdateDetail" => {
                // id, atualiza, obsoleta, sites, bugs, CVEs, reinício, texto, changelog, estado, emitida, atualizada
                type Body = (String, Vec<String>, Vec<String>, Vec<String>, Vec<String>, Vec<String>, u32, String, String, u32, String, String);
                if let Ok(d) = body.deserialize::<Body>() {
                    out.update_details.push(UpdateDetail { id: d.0, update_text: d.7, changelog: d.8, restart: d.6 > 1 });
                }
            }
            "Details" => {
                if let Ok(map) = body.deserialize::<HashMap<String, OwnedValue>>() {
                    let text = |k: &str| map.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
                    let size = map.get("size").and_then(|v| u64::try_from(v).ok()).unwrap_or(0);
                    out.details.push(Details {
                        id: text("package-id"),
                        summary: text("summary"),
                        description: text("description"),
                        url: text("url"),
                        license: text("license"),
                        size,
                    });
                }
            }
            "ErrorCode" => {
                if let Ok((code, details)) = body.deserialize::<(u32, String)>() {
                    error = Some((code, details));
                }
            }
            "Finished" => {
                let exit = body.deserialize::<(u32, u32)>().map(|(e, _)| e).unwrap_or(0);
                return match exit {
                    1 => Ok(out),
                    3 | 9 => Err(Error::Cancelled),
                    _ => Err(match error {
                        // 3 = sem permissão (polkit negou ou a senha não veio).
                        Some((3, _)) => Error::NotAuthorized,
                        Some((_, details)) => Error::Failed(details),
                        None => Error::Failed(format!("exit {exit}")),
                    }),
                };
            }
            _ => {}
        }
    }
    Err(Error::Failed("transaction ended without an answer".into()))
}

/// Atualiza a lista de pacotes (como `apt update`).
pub fn refresh(force: bool, progress: impl FnMut(Progress)) -> Result<(), Error> {
    transaction("RefreshCache", &(force,), progress).map(|_| ())
}

/// Atualizações disponíveis.
pub fn updates() -> Result<Vec<Package>, Error> {
    transaction("GetUpdates", &(FILTER_NONE,), |_| {}).map(|o| o.packages)
}

/// Notas e changelog das atualizações `ids`.
pub fn update_details(ids: &[String]) -> Result<Vec<UpdateDetail>, Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    transaction("GetUpdateDetail", &(ids.to_vec(),), |_| {}).map(|o| o.update_details)
}

/// Versões instaladas de `names` (nome → versão).
pub fn installed_versions(names: &[String]) -> Result<HashMap<String, String>, Error> {
    if names.is_empty() {
        return Ok(HashMap::new());
    }
    let out = transaction("Resolve", &(FILTER_INSTALLED, names.to_vec()), |_| {})?;
    Ok(out.packages.iter().map(|p| (p.name().to_string(), p.version().to_string())).collect())
}

/// Instala as atualizações `ids` (pede a senha pelo polkit).
pub fn update(ids: &[String], progress: impl FnMut(Progress)) -> Result<(), Error> {
    transaction("UpdatePackages", &(FLAG_ONLY_TRUSTED, ids.to_vec()), progress).map(|_| ())
}

/// Busca pacotes pelo nome (com `filter`).
pub fn search(filter: u64, terms: &[String]) -> Result<Vec<Package>, Error> {
    transaction("SearchNames", &(filter, terms.to_vec()), |_| {}).map(|o| o.packages)
}

/// Resolve nomes em pacotes (instalados e disponíveis, conforme `filter`).
pub fn resolve(filter: u64, names: &[String]) -> Result<Vec<Package>, Error> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    transaction("Resolve", &(filter, names.to_vec()), |_| {}).map(|o| o.packages)
}

/// Detalhes dos pacotes `ids`.
pub fn details(ids: &[String]) -> Result<Vec<Details>, Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    transaction("GetDetails", &(ids.to_vec(),), |_| {}).map(|o| o.details)
}

/// Instala os pacotes `ids` (pede a senha pelo polkit).
pub fn install(ids: &[String], progress: impl FnMut(Progress)) -> Result<(), Error> {
    transaction("InstallPackages", &(FLAG_ONLY_TRUSTED, ids.to_vec()), progress).map(|_| ())
}

/// Remove os pacotes `ids` e o que só eles usavam (pede a senha pelo polkit).
pub fn remove(ids: &[String], progress: impl FnMut(Progress)) -> Result<(), Error> {
    // flags, permitir remover dependentes, remover os que ficaram sem uso
    transaction("RemovePackages", &(0u64, ids.to_vec(), true, true), progress).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partes_do_id_do_pacote() {
        let p = Package { id: "bash;5.2.37-2+b10;amd64;manual:debian-stable-main".into(), info: Info::from(1), summary: String::new() };
        assert_eq!((p.name(), p.version(), p.arch()), ("bash", "5.2.37-2+b10", "amd64"));
        assert_eq!(p.info, Info::Installed);
        assert_eq!(Info::from(8), Info::Security);
        assert_eq!(Stage::from(31), Stage::Authenticating);
    }

    /// Precisa do PackageKit na máquina (só leitura): `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn resolve_um_pacote_instalado() {
        let v = installed_versions(&["bash".to_string()]).unwrap();
        assert!(v.get("bash").is_some_and(|v| !v.is_empty()));
    }
}
