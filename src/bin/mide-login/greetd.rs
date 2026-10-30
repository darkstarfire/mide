//! Protocolo do `greetd` (IPC do greeter), escrito aqui: cada mensagem é o tamanho em
//! 32 bits (ordem da máquina) seguido de um JSON, no socket de `GREETD_SOCK`. Quem
//! confere a senha é o greetd, pelo PAM; o greeter só repassa o que a pessoa digita.

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
};

use serde::{Deserialize, Serialize};

/// Pedidos do greeter.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Começa o login de um usuário.
    CreateSession { username: String },
    /// Resposta à última pergunta do PAM (`None` para avisos).
    PostAuthMessageResponse { response: Option<String> },
    /// Login aceito: a sessão a iniciar quando o greeter sair.
    StartSession { cmd: Vec<String>, env: Vec<String> },
    /// Desiste do login em andamento.
    CancelSession,
}

/// Respostas do greetd.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Success,
    Error { error_type: ErrorType, description: String },
    AuthMessage { auth_message_type: AuthKind, auth_message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    /// Senha (ou código) recusada.
    AuthError,
    Error,
}

/// Tipo da pergunta ou aviso do PAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// Pergunta que mostra o que se digita (nome, código de verificação).
    Visible,
    /// Pergunta que esconde (senha).
    Secret,
    Info,
    Error,
}

/// Conexão com o greetd.
pub struct Client {
    stream: UnixStream,
}

impl Client {
    /// Conecta ao socket de `GREETD_SOCK`.
    pub fn connect() -> std::io::Result<Self> {
        let path = std::env::var_os("GREETD_SOCK").ok_or_else(|| std::io::Error::other("GREETD_SOCK não definido (o greeter roda pelo greetd)"))?;
        Ok(Self { stream: UnixStream::connect(path)? })
    }

    /// Manda um pedido e espera a resposta.
    pub fn send(&mut self, request: &Request) -> std::io::Result<Response> {
        let body = serde_json::to_vec(request).map_err(std::io::Error::other)?;
        let mut frame = (body.len() as u32).to_ne_bytes().to_vec();
        frame.extend_from_slice(&body);
        self.stream.write_all(&frame)?;
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len)?;
        let len = u32::from_ne_bytes(len) as usize;
        if len > 1 << 20 {
            return Err(std::io::Error::other("resposta grande demais do greetd"));
        }
        let mut body = vec![0u8; len];
        self.stream.read_exact(&mut body)?;
        serde_json::from_slice(&body).map_err(std::io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mensagens_no_formato_do_greetd() {
        let json = serde_json::to_string(&Request::CreateSession { username: "ana".into() }).unwrap();
        assert_eq!(json, r#"{"type":"create_session","username":"ana"}"#);
        let json = serde_json::to_string(&Request::PostAuthMessageResponse { response: None }).unwrap();
        assert_eq!(json, r#"{"type":"post_auth_message_response","response":null}"#);
        let json = serde_json::to_string(&Request::StartSession { cmd: vec!["mide-session".into()], env: vec!["XDG_SESSION_TYPE=wayland".into()] }).unwrap();
        assert_eq!(json, r#"{"type":"start_session","cmd":["mide-session"],"env":["XDG_SESSION_TYPE=wayland"]}"#);
        assert_eq!(serde_json::to_string(&Request::CancelSession).unwrap(), r#"{"type":"cancel_session"}"#);

        let r: Response = serde_json::from_str(r#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password: "}"#).unwrap();
        assert_eq!(r, Response::AuthMessage { auth_message_type: AuthKind::Secret, auth_message: "Password: ".into() });
        let r: Response = serde_json::from_str(r#"{"type":"error","error_type":"auth_error","description":"pam_authenticate: AUTH_ERR"}"#).unwrap();
        assert!(matches!(r, Response::Error { error_type: ErrorType::AuthError, .. }));
        assert_eq!(serde_json::from_str::<Response>(r#"{"type":"success"}"#).unwrap(), Response::Success);
    }

    /// Ida e volta pelo socket, com o tamanho na frente.
    #[test]
    fn conversa_pelo_socket() {
        let (a, mut b) = UnixStream::pair().unwrap();
        let server = std::thread::spawn(move || {
            let mut len = [0u8; 4];
            b.read_exact(&mut len).unwrap();
            let mut body = vec![0u8; u32::from_ne_bytes(len) as usize];
            b.read_exact(&mut body).unwrap();
            assert_eq!(body, br#"{"type":"cancel_session"}"#);
            let reply = br#"{"type":"success"}"#;
            b.write_all(&(reply.len() as u32).to_ne_bytes()).unwrap();
            b.write_all(reply).unwrap();
        });
        let mut client = Client { stream: a };
        assert_eq!(client.send(&Request::CancelSession).unwrap(), Response::Success);
        server.join().unwrap();
    }
}
