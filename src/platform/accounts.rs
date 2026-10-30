//! Contas de usuário pelo AccountsService (`org.freedesktop.Accounts` no barramento do
//! sistema), para Configurações → Usuários e grupos: listar, mudar nome, foto, dica de
//! senha e tipo de conta, criar e apagar usuários.
//!
//! Quem confere a permissão é o AccountsService com o polkit: mudanças na própria conta
//! (nome, foto, dica) passam direto; em outras contas, criar e apagar, o polkit pede a
//! senha de administrador pelo agente da sessão (`mide-polkit-agent`). Por isso as
//! chamadas que mudam algo vão com o aviso `ALLOW_INTERACTIVE_AUTHORIZATION` do D-Bus.
//! Todas bloqueiam (podem esperar a pessoa digitar a senha): chame numa thread.
//!
//! Testes: com `MIDE_ACCOUNTS_TEST="ana:Ana Souza:admin:current;bruno:Bruno:standard"`
//! (só nas compilações de desenvolvimento) a lista é essa, e as mudanças ficam na
//! memória do processo, sem barramento.

use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Mutex,
};

use zbus::{blocking::Connection, blocking::Proxy, proxy::MethodFlags, zvariant::OwnedObjectPath};

const SERVICE: &str = "org.freedesktop.Accounts";
const ROOT: &str = "/org/freedesktop/Accounts";
const USER_IFACE: &str = "org.freedesktop.Accounts.User";
/// Tamanho da foto entregue ao AccountsService (quadrada), como a do GNOME.
pub const ICON_SIZE: u32 = 256;

/// Uma conta de usuário.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Account {
    /// Caminho do objeto no D-Bus (`/org/freedesktop/Accounts/User1000`).
    pub path: String,
    pub uid: u64,
    pub login: String,
    /// Nome completo (pode estar vazio).
    pub real_name: String,
    /// Foto, se houver e existir.
    pub icon: Option<PathBuf>,
    /// Administrador (pode usar o sudo).
    pub admin: bool,
    pub hint: String,
    /// Entra sem senha ao ligar (pelo gerenciador de login).
    pub automatic_login: bool,
    /// É quem está usando as Configurações.
    pub current: bool,
}

impl Account {
    /// Nome para mostrar: o completo, ou o da conta.
    pub fn display_name(&self) -> &str {
        if self.real_name.trim().is_empty() {
            &self.login
        } else {
            &self.real_name
        }
    }
}

/// Erro de uma operação nas contas.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// Sem AccountsService (ou sem barramento do sistema).
    Unavailable(String),
    /// O polkit recusou (senha errada, cancelada ou sem permissão).
    Denied,
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unavailable(e) => write!(f, "AccountsService indisponível: {e}"),
            Error::Denied => write!(f, "permissão negada"),
            Error::Failed(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<zbus::Error> for Error {
    fn from(err: zbus::Error) -> Self {
        match &err {
            zbus::Error::MethodError(name, msg, _) => {
                let name = name.as_str();
                if name.ends_with("PermissionDenied") || name.ends_with("NotAuthorized") || name.contains("PolicyKit1") || name.ends_with("AccessDenied") {
                    Error::Denied
                } else if name.ends_with("ServiceUnknown") || name.ends_with("NameHasNoOwner") {
                    Error::Unavailable(msg.clone().unwrap_or_default())
                } else {
                    Error::Failed(msg.clone().unwrap_or_else(|| name.to_string()))
                }
            }
            _ => Error::Failed(err.to_string()),
        }
    }
}

// ---------------------------------------------------------------- contas de teste

/// Contas de teste (`MIDE_ACCOUNTS_TEST`), guardadas na memória do processo.
static TEST: Mutex<Option<Vec<Account>>> = Mutex::new(None);

fn test_mode() -> bool {
    cfg!(debug_assertions) && std::env::var_os("MIDE_ACCOUNTS_TEST").is_some()
}

/// Lê `login:Nome:admin|standard[:current[:foto]]` separados por `;`.
fn parse_test_accounts(text: &str) -> Vec<Account> {
    text.split(';')
        .filter(|s| !s.trim().is_empty())
        .enumerate()
        .map(|(i, item)| {
            let f: Vec<&str> = item.split(':').map(str::trim).collect();
            let login = f[0].to_string();
            Account {
                path: format!("{ROOT}/User{}", 1000 + i),
                uid: 1000 + i as u64,
                real_name: f.get(1).unwrap_or(&"").to_string(),
                admin: f.get(2) == Some(&"admin"),
                current: f.get(3) == Some(&"current"),
                icon: f.get(4).filter(|p| !p.is_empty()).map(PathBuf::from),
                login,
                ..Default::default()
            }
        })
        .collect()
}

/// Muda uma conta de teste pelo caminho.
fn with_test(path: &str, f: impl FnOnce(&mut Account)) -> Result<(), Error> {
    let mut guard = TEST.lock().map_err(|_| Error::Failed("contas de teste".into()))?;
    let list = guard.get_or_insert_with(|| parse_test_accounts(&std::env::var("MIDE_ACCOUNTS_TEST").unwrap_or_default()));
    let account = list.iter_mut().find(|a| a.path == path).ok_or_else(|| Error::Failed(format!("conta {path} não existe")))?;
    f(account);
    Ok(())
}

// ---------------------------------------------------------------- D-Bus

fn connection() -> Result<Connection, Error> {
    Connection::system().map_err(|e| Error::Unavailable(e.to_string()))
}

fn accounts(conn: &Connection) -> Result<Proxy<'static>, Error> {
    Proxy::new(conn, SERVICE, ROOT, SERVICE).map_err(|e| Error::Unavailable(e.to_string()))
}

fn user(conn: &Connection, path: &str) -> Result<Proxy<'static>, Error> {
    let path = OwnedObjectPath::try_from(path.to_string()).map_err(|e| Error::Failed(e.to_string()))?;
    Ok(Proxy::new(conn, SERVICE, path, USER_IFACE)?)
}

/// Chama um método que muda algo, deixando o polkit pedir a senha.
fn change<B>(proxy: &Proxy<'_>, method: &str, body: &B) -> Result<(), Error>
where
    B: serde::ser::Serialize + zbus::zvariant::DynamicType,
{
    proxy.call_with_flags::<_, _, ()>(method, MethodFlags::AllowInteractiveAuth.into(), body)?;
    Ok(())
}

/// O AccountsService está no barramento do sistema?
pub fn available() -> bool {
    if test_mode() {
        return true;
    }
    let Ok(conn) = connection() else { return false };
    let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return false };
    // Ativável também conta (sobe na primeira chamada).
    let name = zbus::names::BusName::try_from(SERVICE).ok();
    name.is_some_and(|n| dbus.name_has_owner(n.clone()).unwrap_or(false))
        || dbus.list_activatable_names().is_ok_and(|names| names.iter().any(|n| n.as_str() == SERVICE))
}

/// Usuários comuns (sem as contas de serviço), o atual primeiro e os outros por nome.
pub fn list() -> Result<Vec<Account>, Error> {
    let mut list = if test_mode() {
        let mut guard = TEST.lock().map_err(|_| Error::Failed("contas de teste".into()))?;
        guard.get_or_insert_with(|| parse_test_accounts(&std::env::var("MIDE_ACCOUNTS_TEST").unwrap_or_default())).clone()
    } else {
        list_dbus()?
    };
    sort_accounts(&mut list);
    Ok(list)
}

fn sort_accounts(list: &mut [Account]) {
    list.sort_by_key(|a| (!a.current, a.display_name().to_lowercase()));
}

fn list_dbus() -> Result<Vec<Account>, Error> {
    let conn = connection()?;
    let accounts = accounts(&conn)?;
    let paths: Vec<OwnedObjectPath> = accounts.call("ListCachedUsers", &())?;
    let me = rustix::process::getuid().as_raw() as u64;
    let mut list: Vec<Account> = paths
        .into_iter()
        .filter_map(|path| {
            let proxy = Proxy::new(&conn, SERVICE, path.clone(), USER_IFACE).ok()?;
            if proxy.get_property::<bool>("SystemAccount").unwrap_or(false) {
                return None;
            }
            read_account(&proxy, &path, me)
        })
        .collect();
    // O ListCachedUsers só traz quem já entrou alguma vez; o atual entra sempre.
    if !list.iter().any(|a| a.current)
        && let Ok(path) = accounts.call::<_, _, OwnedObjectPath>("FindUserById", &(me as i64))
        && let Ok(proxy) = Proxy::new(&conn, SERVICE, path.clone(), USER_IFACE)
        && let Some(account) = read_account(&proxy, &path, me)
    {
        list.push(account);
    }
    Ok(list)
}

/// As propriedades de um usuário do AccountsService.
fn read_account(proxy: &Proxy<'_>, path: &OwnedObjectPath, me: u64) -> Option<Account> {
    let uid: u64 = proxy.get_property("Uid").ok()?;
    let icon: String = proxy.get_property("IconFile").unwrap_or_default();
    Some(Account {
        path: path.as_str().to_string(),
        uid,
        login: proxy.get_property("UserName").ok()?,
        real_name: proxy.get_property("RealName").unwrap_or_default(),
        icon: Some(PathBuf::from(icon)).filter(|p| p.is_file()),
        admin: proxy.get_property::<i32>("AccountType").unwrap_or(0) == 1,
        hint: proxy.get_property("PasswordHint").unwrap_or_default(),
        automatic_login: proxy.get_property("AutomaticLogin").unwrap_or(false),
        current: uid == me,
    })
}

/// Muda o nome completo.
pub fn set_real_name(account: &Account, name: &str) -> Result<(), Error> {
    let name = name.trim().to_string();
    if test_mode() {
        tracing::info!("contas (teste): {} passa a se chamar {name}", account.login);
        return with_test(&account.path, |a| a.real_name = name);
    }
    let conn = connection()?;
    change(&user(&conn, &account.path)?, "SetRealName", &(name,))
}

/// Muda a dica de senha.
pub fn set_password_hint(account: &Account, hint: &str) -> Result<(), Error> {
    let hint = hint.trim().to_string();
    if test_mode() {
        tracing::info!("contas (teste): dica de senha de {} mudada", account.login);
        return with_test(&account.path, |a| a.hint = hint);
    }
    let conn = connection()?;
    change(&user(&conn, &account.path)?, "SetPasswordHint", &(hint,))
}

/// Administrador ou padrão.
pub fn set_admin(account: &Account, admin: bool) -> Result<(), Error> {
    if test_mode() {
        tracing::info!("contas (teste): {} passa a ser {}", account.login, if admin { "administrador" } else { "padrão" });
        return with_test(&account.path, |a| a.admin = admin);
    }
    let conn = connection()?;
    change(&user(&conn, &account.path)?, "SetAccountType", &(admin as i32,))
}

/// Troca a foto pela imagem `image` (recortada no meio, quadrada, em 256x256 PNG).
pub fn set_icon(account: &Account, image: &Path) -> Result<(), Error> {
    // O AccountsService lê o arquivo com as permissões de quem chama e copia para a
    // pasta dele: um PNG no cache do usuário basta.
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(std::env::temp_dir);
    let dir = cache.join("mide");
    std::fs::create_dir_all(&dir).map_err(|e| Error::Failed(e.to_string()))?;
    let out = dir.join(format!("account-icon-{}.png", account.login));
    prepare_icon(image, &out)?;
    if test_mode() {
        tracing::info!("contas (teste): foto de {} = {}", account.login, out.display());
        return with_test(&account.path, |a| a.icon = Some(out.clone()));
    }
    let conn = connection()?;
    change(&user(&conn, &account.path)?, "SetIconFile", &(out.to_string_lossy().into_owned(),))
}

/// Recorta o meio de `src` num quadrado e grava em `dst` como PNG de 256x256.
pub fn prepare_icon(src: &Path, dst: &Path) -> Result<(), Error> {
    let img = image::open(src).map_err(|e| Error::Failed(format!("imagem ilegível: {e}")))?.to_rgba8();
    let (w, h) = img.dimensions();
    let side = w.min(h);
    if side == 0 {
        return Err(Error::Failed("imagem vazia".into()));
    }
    let square = image::imageops::crop_imm(&img, (w - side) / 2, (h - side) / 2, side, side).to_image();
    let small = image::imageops::resize(&square, ICON_SIZE, ICON_SIZE, image::imageops::FilterType::Lanczos3);
    small.save_with_format(dst, image::ImageFormat::Png).map_err(|e| Error::Failed(e.to_string()))
}

/// Cria um usuário; devolve a conta nova.
pub fn create(login: &str, real_name: &str, admin: bool) -> Result<Account, Error> {
    if !valid_login(login) {
        return Err(Error::Failed(format!("nome de conta inválido: {login}")));
    }
    if test_mode() {
        tracing::info!("contas (teste): {login} criado ({})", if admin { "administrador" } else { "padrão" });
        let mut guard = TEST.lock().map_err(|_| Error::Failed("contas de teste".into()))?;
        let list = guard.get_or_insert_with(|| parse_test_accounts(&std::env::var("MIDE_ACCOUNTS_TEST").unwrap_or_default()));
        if list.iter().any(|a| a.login == login) {
            return Err(Error::Failed(format!("o usuário {login} já existe")));
        }
        let uid = list.iter().map(|a| a.uid).max().unwrap_or(999) + 1;
        let account = Account { path: format!("{ROOT}/User{uid}"), uid, login: login.into(), real_name: real_name.trim().into(), admin, ..Default::default() };
        list.push(account.clone());
        return Ok(account);
    }
    let conn = connection()?;
    let accounts = accounts(&conn)?;
    let path: OwnedObjectPath = accounts
        .call_with_flags("CreateUser", MethodFlags::AllowInteractiveAuth.into(), &(login, real_name.trim(), admin as i32))?
        .ok_or_else(|| Error::Failed("CreateUser sem resposta".into()))?;
    Ok(Account { path: path.as_str().to_string(), login: login.into(), real_name: real_name.trim().into(), admin, ..Default::default() })
}

/// Apaga um usuário (e a pasta pessoal, com `remove_files`).
pub fn delete(account: &Account, remove_files: bool) -> Result<(), Error> {
    if account.current {
        return Err(Error::Failed("não dá para apagar a própria conta".into()));
    }
    if test_mode() {
        tracing::info!("contas (teste): {} apagado (pasta pessoal {})", account.login, if remove_files { "apagada" } else { "mantida" });
        let mut guard = TEST.lock().map_err(|_| Error::Failed("contas de teste".into()))?;
        if let Some(list) = guard.as_mut() {
            list.retain(|a| a.path != account.path);
        }
        return Ok(());
    }
    let conn = connection()?;
    change(&accounts(&conn)?, "DeleteUser", &(account.uid as i64, remove_files))
}

/// Define a senha (o AccountsService recebe só o hash crypt, nunca a senha). `password`
/// é apagada da memória no fim.
pub fn set_password(account: &Account, password: &mut String, hint: &str) -> Result<(), Error> {
    let hashed = hash_password(password);
    crate::elevate::wipe(password);
    let hashed = hashed?;
    if test_mode() {
        tracing::info!("contas (teste): senha de {} definida", account.login);
        let hint = hint.to_string();
        return with_test(&account.path, |a| a.hint = hint);
    }
    let conn = connection()?;
    change(&user(&conn, &account.path)?, "SetPassword", &(hashed, hint.trim()))
}

/// Sem senha agora: a pessoa escolhe a dela ao entrar pela primeira vez
/// (`SetPasswordMode(1)`: senha vazia e vencida; a tela de login pede a nova pelo PAM).
pub fn password_at_first_login(account: &Account) -> Result<(), Error> {
    if test_mode() {
        tracing::info!("contas (teste): {} escolhe a senha no primeiro login", account.login);
        return Ok(());
    }
    let conn = connection()?;
    change(&user(&conn, &account.path)?, "SetPasswordMode", &(1i32,))
}

/// Hash `$6$` (SHA-512 crypt, o padrão do `/etc/shadow`), com sal de 16 caracteres do
/// `/dev/urandom`.
pub fn hash_password(password: &str) -> Result<String, Error> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).map_err(|e| Error::Failed(format!("sem /dev/urandom: {e}")))?;
    let salt = salt_from_bytes(&bytes);
    hash_with_salt(password, &salt)
}

/// O alfabeto do crypt (`./0-9A-Za-z`): 64 símbolos, então `byte % 64` não pende para
/// nenhum.
fn salt_from_bytes(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    bytes.iter().map(|b| ALPHABET[(*b % 64) as usize] as char).collect()
}

fn hash_with_salt(password: &str, salt: &str) -> Result<String, Error> {
    let params = sha_crypt::Sha512Params::default();
    let hash = sha_crypt::sha512_crypt_b64(password.as_bytes(), salt.as_bytes(), &params).map_err(|e| Error::Failed(format!("hash da senha: {e:?}")))?;
    Ok(format!("$6${salt}${hash}"))
}

/// Nome de conta válido (o padrão do `adduser`): começa com letra minúscula, depois
/// minúsculas, algarismos, `-` e `_`, até 32 caracteres.
pub fn valid_login(login: &str) -> bool {
    let mut chars = login.chars();
    login.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Nome de conta sugerido a partir do nome completo, como o Mac: minúsculas, sem
/// acentos e sem espaços ("Ana Sousa" → "anasousa").
pub fn suggest_login(real_name: &str) -> String {
    let folded = crate::apps::fold(real_name);
    let mut out: String = folded.chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect();
    // Começa com letra.
    while out.starts_with(|c: char| c.is_ascii_digit()) {
        out.remove(0);
    }
    out.truncate(32);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sugestao_do_nome_da_conta() {
        assert_eq!(suggest_login("Ana Sousa"), "anasousa");
        assert_eq!(suggest_login("João Conceição"), "joaoconceicao");
        assert_eq!(suggest_login("  Zé-Maria 2 "), "zemaria2");
        assert_eq!(suggest_login("2pac Shakur"), "pacshakur");
        assert_eq!(suggest_login("李"), "");
        assert_eq!(suggest_login(&"a".repeat(40)).len(), 32);
    }

    #[test]
    fn nome_de_conta_valido() {
        assert!(valid_login("ana"));
        assert!(valid_login("ana_souza-2"));
        assert!(!valid_login(""));
        assert!(!valid_login("2ana"));
        assert!(!valid_login("Ana"));
        assert!(!valid_login("ana souza"));
        assert!(!valid_login("joão"));
        assert!(!valid_login(&"a".repeat(33)));
    }

    /// Vetor do documento do SHA-crypt (Drepper): mesmo resultado que o `crypt(3)`.
    #[test]
    fn hash_sha512_crypt() {
        assert_eq!(
            hash_with_salt("Hello world!", "saltstring").unwrap(),
            "$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1"
        );
        let h = hash_password("segredo").unwrap();
        assert!(h.starts_with("$6$") && h.split('$').count() == 4, "{h}");
        assert_eq!(h.split('$').nth(2).unwrap().len(), 16);
    }

    #[test]
    fn sal_no_alfabeto_do_crypt() {
        let salt = salt_from_bytes(&[0, 63, 64, 255, 128, 11]);
        assert_eq!(salt, ".z.z.9");
    }

    #[test]
    fn contas_de_teste() {
        let list = parse_test_accounts("ana:Ana Souza:admin:current; bruno:Bruno:standard;carla::standard");
        assert_eq!(list.len(), 3);
        assert!(list[0].admin && list[0].current);
        assert!(!list[1].admin && !list[1].current);
        assert_eq!(list[2].display_name(), "carla");
        let mut sorted = vec![list[1].clone(), list[2].clone(), list[0].clone()];
        sort_accounts(&mut sorted);
        assert_eq!(sorted.iter().map(|a| a.login.as_str()).collect::<Vec<_>>(), ["ana", "bruno", "carla"]);
    }

    #[test]
    fn foto_quadrada_de_256() {
        let dir = std::env::temp_dir().join(format!("mide-accounts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("larga.png");
        image::RgbaImage::from_pixel(600, 300, image::Rgba([10, 20, 30, 255])).save(&src).unwrap();
        let dst = dir.join("saida.png");
        prepare_icon(&src, &dst).unwrap();
        let out = image::open(&dst).unwrap();
        assert_eq!((out.width(), out.height()), (ICON_SIZE, ICON_SIZE));
        std::fs::remove_dir_all(&dir).ok();
    }
}
