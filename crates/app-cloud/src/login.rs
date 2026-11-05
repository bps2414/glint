//! Login do app: loopback 127.0.0.1 + PKCE (RFC 8252 / RFC 7636), como em `docs/API.md`.
//!
//! ```text
//! start() ─► abre o navegador em  <site>/auth/app?port&challenge&state
//!            usuário entra com o Discord (e o convite, se a conta é nova)
//! wait()  ◄─ navegador volta em    http://127.0.0.1:<porta>/callback?code&state
//! Client::exchange_code(code, verifier) ─► token de 180 dias
//! ```

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::api::{CloudError, Result};

/// `len` bytes aleatórios em base64url sem `=`.
pub fn random_b64(len: usize) -> String {
    let mut buf = vec![0u8; len];
    getrandom::fill(&mut buf).expect("gerador aleatório do sistema");
    URL_SAFE_NO_PAD.encode(buf)
}

/// `base64url(SHA-256(verifier))`.
pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Login em andamento: o servidor local já está escutando.
pub struct Pending {
    listener: TcpListener,
    pub port: u16,
    pub verifier: String,
    state: String,
    /// Página a abrir no navegador.
    pub url: String,
}

/// Escuta numa porta livre de 127.0.0.1 e monta o link de login.
pub fn start(base: &str) -> io::Result<Pending> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let verifier = random_b64(32);
    let state = random_b64(24);
    let url = format!(
        "{}/auth/app?port={port}&challenge={}&state={state}",
        base.trim_end_matches('/'),
        challenge_for(&verifier)
    );
    Ok(Pending {
        listener,
        port,
        verifier,
        state,
        url,
    })
}

impl Pending {
    /// Espera o navegador voltar com o código (até `timeout`, ou até
    /// `cancel()` dizer para parar). `page` é o HTML mostrado na aba.
    pub fn wait(&self, timeout: Duration, page: &str, cancel: &dyn Fn() -> bool) -> Result<String> {
        let until = Instant::now() + timeout;
        loop {
            if cancel() {
                return Err(CloudError::Cancelled);
            }
            if Instant::now() >= until {
                return Err(CloudError::Other(
                    "O login não terminou a tempo. Tente de novo.".into(),
                ));
            }
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(code) = self.handle(stream, page)? {
                        return Ok(code);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Uma conexão do navegador. `Some(code)` = era o retorno certo.
    fn handle(&self, mut stream: TcpStream, page: &str) -> Result<Option<String>> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let Some(target) = read_request_target(&mut stream) else {
            return Ok(None);
        };
        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
        if path != "/callback" {
            respond(&mut stream, "404 Not Found", "");
            return Ok(None);
        }
        let param = |k: &str| {
            query
                .split('&')
                .filter_map(|kv| kv.split_once('='))
                .find(|(key, _)| *key == k)
                .map(|(_, v)| percent_decode(v))
        };
        if param("state").as_deref() != Some(self.state.as_str()) {
            respond(&mut stream, "400 Bad Request", "");
            return Err(CloudError::Other(
                "O retorno do login não confere. Tente de novo.".into(),
            ));
        }
        match param("code").filter(|c| !c.is_empty()) {
            Some(code) => {
                respond(&mut stream, "200 OK", page);
                Ok(Some(code))
            }
            None => {
                respond(&mut stream, "400 Bad Request", "");
                Err(CloudError::Other(
                    "O login foi cancelado no navegador.".into(),
                ))
            }
        }
    }
}

/// Lê a primeira linha (`GET <alvo> HTTP/1.1`) e devolve o alvo.
fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buf);
    let mut parts = text.lines().next()?.split(' ');
    (parts.next()? == "GET").then(|| parts.next().map(str::to_string))?
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let msg = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(msg.as_bytes());
    let _ = stream.flush();
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        match b[i] {
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h << 4 | l);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    use super::{challenge_for, percent_decode, random_b64, start};

    #[test]
    fn pkce() {
        // SHA-256("abc"), em base64url sem `=`.
        assert_eq!(
            challenge_for("abc"),
            "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"
        );
        let v = random_b64(32);
        assert_eq!(v.len(), 43);
        assert!(v
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(v, random_b64(32));
    }

    #[test]
    fn decodifica_query() {
        assert_eq!(percent_decode("a%2Fb+c"), "a/b c");
        assert_eq!(percent_decode("abc%"), "abc%");
    }

    fn get(port: u16, target: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn recebe_o_codigo_do_navegador() {
        let p = start("https://exemplo.dev/").unwrap();
        assert!(p.url.starts_with("https://exemplo.dev/auth/app?port="));
        let state = p.url.split("state=").nth(1).unwrap().to_string();
        let port = p.port;
        let t = std::thread::spawn(move || {
            let favicon = get(port, "/favicon.ico");
            assert!(favicon.starts_with("HTTP/1.1 404"));
            get(port, &format!("/callback?code=xyz&state={state}"))
        });
        let code = p
            .wait(Duration::from_secs(10), "<p>pronto</p>", &|| false)
            .unwrap();
        assert_eq!(code, "xyz");
        let page = t.join().unwrap();
        assert!(page.starts_with("HTTP/1.1 200") && page.ends_with("<p>pronto</p>"));
    }

    #[test]
    fn state_errado_falha() {
        let p = start("https://exemplo.dev").unwrap();
        let port = p.port;
        let t = std::thread::spawn(move || get(port, "/callback?code=xyz&state=outro"));
        assert!(p.wait(Duration::from_secs(10), "", &|| false).is_err());
        t.join().unwrap();
    }
}
