//! Atualização do app: lê o `latest.json` do último release do GitHub
//! (`https://github.com/<repo>/releases/latest/download/latest.json`), compara com a versão
//! instalada e baixa o instalador conferindo o SHA-256. Não depende do servidor de clipes.
//! Rodar o instalador é coisa do sistema (fica no `app-platform-*`).

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use ureq::Agent;

use crate::api::CloudError;

type Result<T> = std::result::Result<T, CloudError>;

/// Versão publicada (o `latest.json` anexado ao release + a URL de download).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default)]
    pub notes: String,
}

/// A versão mais nova publicada no GitHub de `repo` (`dono/nome`); `None` se ainda não há
/// release (404). O `latest` do GitHub ignora pre-releases, então o CI publica sempre como
/// release normal.
pub fn latest_release(repo: &str) -> Result<Option<Release>> {
    latest_release_within(repo, Duration::from_secs(30))
}

/// Variável de ambiente com a URL de um `latest.json` no lugar do GitHub (só para testar a
/// atualização com um servidor local).
pub const FEED_ENV: &str = "GLINT_UPDATE_FEED";

/// [`latest_release`] com prazo total `timeout` (a abertura do app não pode esperar 30 s).
/// Respeita [`FEED_ENV`].
pub fn latest_release_within(repo: &str, timeout: Duration) -> Result<Option<Release>> {
    let url = std::env::var(FEED_ENV)
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| {
            format!("https://github.com/{repo}/releases/latest/download/latest.json")
        });
    fetch_latest(&url, timeout)
}

/// Baixa e lê um `latest.json` (segue os redirects do GitHub; 404 = sem versão).
fn fetch_latest(url: &str, timeout: Duration) -> Result<Option<Release>> {
    let agent: Agent = Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(timeout.min(Duration::from_secs(15))))
        .timeout_global(Some(timeout))
        .build()
        .into();
    let mut resp = agent.get(url).call()?;
    let status = resp.status().as_u16();
    if status == 404 {
        return Ok(None);
    }
    if status >= 400 {
        return Err(CloudError::Other(format!(
            "a consulta de versão respondeu {status}"
        )));
    }
    resp.body_mut()
        .read_json::<Release>()
        .map(Some)
        .map_err(|e| CloudError::Other(format!("latest.json inválido: {e}")))
}

/// `a` é mais nova que `b`? Semver simples: `1.2.3` > `1.2.3-beta.2` > `1.2.3-beta.1`.
pub fn is_newer(a: &str, b: &str) -> bool {
    fn parse(v: &str) -> (Vec<u64>, Option<Vec<String>>) {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p.split('.').map(str::to_string).collect())),
            None => (v, None),
        };
        let mut nums: Vec<u64> = core.split('.').map(|n| n.parse().unwrap_or(0)).collect();
        nums.resize(3, 0);
        (nums, pre)
    }
    let ((an, ap), (bn, bp)) = (parse(a), parse(b));
    if an != bn {
        return an > bn;
    }
    match (ap, bp) {
        (None, None) => false,
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (Some(x), Some(y)) => {
            for (p, q) in x.iter().zip(&y) {
                let ord = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                    (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                    _ => p.cmp(q),
                };
                if ord != std::cmp::Ordering::Equal {
                    return ord.is_gt();
                }
            }
            x.len() > y.len()
        }
    }
}

/// Baixa o instalador para `dest`, chamando `on(baixado, total)`; `false` cancela. Confere o
/// tamanho e o SHA-256 antes de devolver (arquivo errado é apagado).
pub fn download(
    release: &Release,
    dest: &Path,
    on: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<()> {
    let agent: Agent = Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut resp = agent.get(&release.url).call()?;
    let status = resp.status().as_u16();
    if status >= 400 {
        return Err(CloudError::Other(format!("o download respondeu {status}")));
    }
    let total = release.size;
    let tmp = dest.with_extension("part");
    let result = (|| {
        let mut out = File::create(&tmp)?;
        let mut body = resp.body_mut().as_reader();
        let mut hash = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut got = 0u64;
        loop {
            let n = body.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            hash.update(&buf[..n]);
            got += n as u64;
            if !on(got, total) {
                return Err(CloudError::Cancelled);
            }
        }
        out.flush()?;
        let sum: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != total || !sum.eq_ignore_ascii_case(&release.sha256) {
            return Err(CloudError::Other(
                "o instalador baixado veio corrompido (tamanho ou SHA-256 diferente)".into(),
            ));
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            let _ = std::fs::remove_file(dest);
            std::fs::rename(&tmp, dest)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{download, fetch_latest, is_newer, Release};
    use sha2::{Digest, Sha256};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    const T: Duration = Duration::from_secs(5);

    #[test]
    fn compara_versoes() {
        assert!(is_newer("0.1.0", "0.1.0-beta.1"));
        assert!(is_newer("1.0.0-rc.1", "0.1.0-beta.1"));
        assert!(is_newer("1.0.0", "1.0.0-rc.2"));
        assert!(is_newer("0.1.0-beta.2", "0.1.0-beta.1"));
        assert!(is_newer("0.1.0-beta.10", "0.1.0-beta.9"));
        assert!(is_newer("0.2.0-beta.1", "0.1.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0-beta.1", "0.1.0-beta.1"));
        assert!(!is_newer("0.1.0-beta.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.1.0-rc.1", "0.1.0-beta.5"));
    }

    /// Servidor que responde o `latest.json` (direto e via redirect) e o arquivo (uma requisição por conexão).
    fn server(payload: Vec<u8>, latest: Option<String>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let mut conn = conn.unwrap();
                let mut r = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                }
                if line.contains("/releases/latest/download/") {
                    // Como o GitHub: o link "latest" redireciona para o arquivo do release.
                    let _ = write!(
                        conn,
                        "HTTP/1.1 302 Found\r\nLocation: /latest.json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    continue;
                }
                let (status, body): (&str, Vec<u8>) = if line.contains("/latest.json") {
                    match &latest {
                        Some(j) => ("200 OK", j.clone().into_bytes()),
                        None => ("404 Not Found", b"{}".to_vec()),
                    }
                } else {
                    ("200 OK", payload.clone())
                };
                let _ = write!(
                    conn,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = conn.write_all(&body);
            }
        });
        base
    }

    #[test]
    fn latest_e_download_conferido() {
        let data = b"MZ instalador falso".to_vec();
        let sum: String = Sha256::digest(&data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let none = server(Vec::new(), None);
        assert_eq!(
            fetch_latest(&format!("{none}/latest.json"), T).unwrap(),
            None
        );

        let base = server(data.clone(), None);
        let json = format!(
            r#"{{"version":"0.2.0","url":"{base}/baixar/x.exe","file":"x.exe","size":{},"sha256":"{sum}","notes":"n","published_at":1}}"#,
            data.len()
        );
        let base2 = server(data.clone(), Some(json));
        let rel: Release = fetch_latest(&format!("{base2}/latest.json"), T)
            .unwrap()
            .unwrap();
        assert_eq!(rel.version, "0.2.0");
        // Pelo link "latest" do GitHub (com redirect).
        let via: Release =
            fetch_latest(&format!("{base2}/releases/latest/download/latest.json"), T)
                .unwrap()
                .unwrap();
        assert_eq!(via, rel);

        let dir = std::env::temp_dir().join(format!("update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("setup.exe");
        let mut calls = 0;
        download(&rel, &dest, &mut |_, _| {
            calls += 1;
            true
        })
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(calls >= 1);

        // SHA errado: não fica arquivo nenhum.
        let bad = Release {
            sha256: "00".repeat(32),
            ..rel
        };
        std::fs::remove_file(&dest).unwrap();
        assert!(download(&bad, &dest, &mut |_, _| true).is_err());
        assert!(!dest.exists() && !dest.with_extension("part").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
