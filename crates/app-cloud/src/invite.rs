//! Link de convite: o que o usuário cola (`https://<servidor>/i/<código>`) ou o que o
//! site abre (`glint://join?s=<origem>&i=<código>`), e o endereço do servidor.

use crate::login::percent_decode;

/// Convite já separado: servidor (origem, sem barra no fim) e código.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub server: String,
    pub code: String,
}

/// Código de convite: 22 caracteres base64url (mesma regra do servidor).
fn is_code(s: &str) -> bool {
    s.len() == 22
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Origem `http(s)://host[:porta]`, sem caminho; `None` se não for um endereço.
pub fn origin(url: &str) -> Option<String> {
    let t = url.trim();
    let lower = t.to_ascii_lowercase();
    let scheme = ["https://", "http://"]
        .into_iter()
        .find(|p| lower.starts_with(p))?;
    let host = t[scheme.len()..].split(['/', '?', '#']).next()?;
    (!host.is_empty() && !host.contains(char::is_whitespace)).then(|| format!("{scheme}{host}"))
}

/// Link colado (`https://.../i/<código>`) ou `glint://join?s=...&i=...`.
pub fn parse(input: &str) -> Option<Invite> {
    let t = input.trim();
    if let Some(query) = t
        .get(..13)
        .filter(|p| p.eq_ignore_ascii_case("glint://join?"))
        .map(|_| &t[13..])
    {
        let param = |k: &str| {
            query
                .split('&')
                .filter_map(|kv| kv.split_once('='))
                .find(|(key, _)| *key == k)
                .map(|(_, v)| percent_decode(v))
        };
        let server = origin(&param("s")?)?;
        let code = param("i").filter(|c| is_code(c))?;
        return Some(Invite { server, code });
    }
    let server = origin(t)?;
    let path = t[server.len()..].split(['?', '#']).next()?;
    let code = path.trim_end_matches('/').strip_prefix("/i/")?;
    is_code(code).then(|| Invite {
        server,
        code: code.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{origin, parse, Invite};

    const CODE: &str = "AbCdEfGhIjKlMnOpQrSt_-";

    #[test]
    fn link_colado() {
        let want = Some(Invite {
            server: "https://meu.workers.dev".into(),
            code: CODE.into(),
        });
        assert_eq!(
            parse(&format!("  https://meu.workers.dev/i/{CODE}/ ")),
            want
        );
        assert_eq!(
            parse(&format!("https://meu.workers.dev/i/{CODE}?x=1")),
            want
        );
        assert_eq!(parse("https://meu.workers.dev/i/curto"), None);
        assert_eq!(parse(CODE), None);
        assert_eq!(parse(&format!("https://meu.workers.dev/c/{CODE}")), None);
    }

    #[test]
    fn link_do_protocolo() {
        let link = format!("glint://join?s=https%3A%2F%2Fmeu.workers.dev&i={CODE}");
        assert_eq!(
            parse(&link),
            Some(Invite {
                server: "https://meu.workers.dev".into(),
                code: CODE.into(),
            })
        );
        assert_eq!(parse("glint://join?s=nada&i=x"), None);
        assert_eq!(parse(&format!("glint://join/?i={CODE}")), None);
    }

    #[test]
    fn origem() {
        assert_eq!(
            origin("http://127.0.0.1:8787/i/x").as_deref(),
            Some("http://127.0.0.1:8787")
        );
        assert_eq!(origin("ftp://x"), None);
        assert_eq!(origin("https://"), None);
    }
}
