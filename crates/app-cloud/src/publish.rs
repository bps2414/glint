//! Publicar um MP4 pronto: criar → partes de 8 MiB → miniatura → concluir.
//!
//! Cada parte é tentada até 3 vezes em erro de rede/5xx. Se o envio cair no
//! meio, o id fica em `id_out`: chamando de novo com `resume = Some(id)`, só as
//! partes que faltam (segundo `GET /clips/<id>`) são enviadas.

use std::cell::Cell;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use crate::api::{Client, Clip, CloudError, NewClip, Result, UploadState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Antes de cada parte, fora de qualquer requisição: ponto seguro para esperar.
    NextPart,
    /// Durante a parte (dentro da requisição: não segurar aqui).
    Uploading,
    Finishing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub stage: Stage,
    pub sent: u64,
    pub total: u64,
}

const TRIES: u32 = 3;

pub fn publish(
    client: &Client,
    file: &Path,
    thumb: Option<&[u8]>,
    meta: &NewClip,
    resume: Option<&str>,
    id_out: &mut Option<String>,
    on: &mut dyn FnMut(Progress) -> bool,
) -> Result<Clip> {
    let size = std::fs::metadata(file)?.len();
    let mut state: Option<(Clip, UploadState)> = None;
    if let Some(id) = resume {
        match client.get_clip(id) {
            Ok((clip, Some(up))) => state = Some((clip, up)),
            Ok((clip, None)) if clip.status == "ready" => {
                *id_out = Some(clip.id.clone());
                return Ok(clip);
            }
            Ok(_) => {}
            // Sumiu (passou das 24 h) ou é de outra conta: começa de novo.
            Err(CloudError::Api { status: 404, .. }) => {}
            Err(e) => return Err(e),
        }
    }
    let (clip, up) = match state {
        Some(s) => s,
        None => client.create_clip(meta)?,
    };
    *id_out = Some(clip.id.clone());
    let id = clip.id.as_str();

    let part_size = up.part_size.max(1);
    let done_bytes: u64 = up
        .parts_done
        .iter()
        .map(|&n| part_len(n, part_size, size))
        .sum();
    let mut sent = done_bytes;
    if !on(Progress {
        stage: Stage::Uploading,
        sent,
        total: size,
    }) {
        return Err(CloudError::Cancelled);
    }

    let mut f = File::open(file)?;
    let mut buf = Vec::with_capacity(part_size as usize);
    for n in 1..=up.part_count {
        if up.parts_done.contains(&n) {
            continue;
        }
        if !on(Progress {
            stage: Stage::NextPart,
            sent,
            total: size,
        }) {
            return Err(CloudError::Cancelled);
        }
        let len = part_len(n, part_size, size);
        f.seek(SeekFrom::Start(u64::from(n - 1) * part_size))?;
        buf.clear();
        (&mut f).take(len).read_to_end(&mut buf)?;
        if buf.len() as u64 != len {
            return Err(CloudError::Other("o arquivo mudou durante o envio".into()));
        }
        let mut attempt = 0;
        loop {
            attempt += 1;
            let cancelled = Cell::new(false);
            let mut reader = ProgressReader {
                data: &buf,
                pos: 0,
                on: &mut |p| {
                    let ok = on(Progress {
                        stage: Stage::Uploading,
                        sent: sent + p,
                        total: size,
                    });
                    if !ok {
                        cancelled.set(true);
                    }
                    ok
                },
            };
            match client.put_part(id, n, len, &mut reader) {
                Ok(()) => break,
                Err(_) if cancelled.get() => return Err(CloudError::Cancelled),
                Err(e) if e.retryable() && attempt < TRIES => {
                    tracing::warn!("parte {n}, tentativa {attempt}: {e}");
                    std::thread::sleep(Duration::from_secs(u64::from(attempt * 2 - 1)));
                }
                Err(e) => return Err(e),
            }
        }
        sent += len;
    }

    if !on(Progress {
        stage: Stage::Finishing,
        sent: size,
        total: size,
    }) {
        return Err(CloudError::Cancelled);
    }
    if let Some(jpeg) = thumb {
        // Sem miniatura o embed ainda funciona: não derruba a publicação.
        if let Err(e) = client.put_thumb(id, jpeg) {
            tracing::warn!("miniatura não enviada: {e}");
        }
    }
    let mut attempt = 0;
    loop {
        attempt += 1;
        match client.complete(id) {
            Ok(c) => return Ok(c),
            Err(e) if e.retryable() && attempt < TRIES => {
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Tamanho da parte `n` (1..) de um arquivo de `size` bytes.
fn part_len(n: u32, part_size: u64, size: u64) -> u64 {
    let start = u64::from(n.saturating_sub(1)) * part_size;
    size.saturating_sub(start).min(part_size)
}

/// Corpo da parte que conta os bytes lidos (progresso) e aborta no cancelar.
struct ProgressReader<'a> {
    data: &'a [u8],
    pos: usize,
    on: &'a mut dyn FnMut(u64) -> bool,
}

impl Read for ProgressReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if !(self.on)(self.pos as u64) {
            return Err(io::Error::other("cancelado"));
        }
        // Pedaços de 64 KiB: progresso fluido mesmo numa parte só.
        let n = out.len().min(64 * 1024).min(self.data.len() - self.pos);
        out[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use super::{part_len, publish, Progress, Stage};
    use crate::api::{Client, NewClip, Visibility};

    #[test]
    fn tamanho_das_partes() {
        assert_eq!(part_len(1, 8, 20), 8);
        assert_eq!(part_len(3, 8, 20), 4);
        assert_eq!(part_len(4, 8, 20), 0);
    }

    #[derive(Default)]
    struct Fake {
        parts: BTreeMap<u32, Vec<u8>>,
        thumb: Option<Vec<u8>>,
        completed: bool,
        /// Falhas (500) a devolver na próxima parte.
        fail_next: u32,
        log: Vec<String>,
    }

    const PART: usize = 10;

    /// Servidor HTTP mínimo (uma requisição por conexão) que imita a API.
    fn fake_server(size: usize) -> (String, Arc<Mutex<Fake>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let st = Arc::new(Mutex::new(Fake::default()));
        let s2 = st.clone();
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let mut conn = conn.unwrap();
                let mut r = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut len = 0usize;
                let mut auth = String::new();
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    let lower = h.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    if lower.starts_with("authorization:") {
                        auth = h["authorization:".len()..].trim().to_string();
                    }
                }
                let mut body = vec![0u8; len];
                if r.read_exact(&mut body).is_err() {
                    continue; // o cliente cancelou no meio do corpo
                }
                let mut parts = line.split(' ');
                let (method, path) = (parts.next().unwrap(), parts.next().unwrap().to_string());
                let mut f = s2.lock().unwrap();
                f.log.push(format!("{method} {path}"));
                let count = size.div_ceil(PART);
                let done: Vec<u32> = f.parts.keys().copied().collect();
                let up =
                    format!(r#"{{"part_size":{PART},"part_count":{count},"parts_done":{done:?}}}"#);
                let clip = |status: &str, url: &str| {
                    format!(
                        r#"{{"id":"abc","title":"t","game":null,"visibility":"unlisted","status":"{status}","size_bytes":{size},"duration_ms":1000,"url":{url}}}"#
                    )
                };
                let (code, resp) = if auth != "Bearer tok" {
                    (401, String::new())
                } else if method == "POST" && path == "/api/v1/clips" {
                    (
                        201,
                        format!(r#"{{"clip":{},"upload":{up}}}"#, clip("uploading", "null")),
                    )
                } else if method == "GET" && path == "/api/v1/clips/abc" {
                    (
                        200,
                        format!(r#"{{"clip":{},"upload":{up}}}"#, clip("uploading", "null")),
                    )
                } else if method == "PUT" && path.starts_with("/api/v1/clips/abc/parts/") {
                    if f.fail_next > 0 {
                        f.fail_next -= 1;
                        (
                            500,
                            r#"{"error":{"code":"server","message":"falhou"}}"#.into(),
                        )
                    } else {
                        let n: u32 = path.rsplit('/').next().unwrap().parse().unwrap();
                        f.parts.insert(n, body);
                        (200, format!(r#"{{"part_number":{n}}}"#))
                    }
                } else if method == "PUT" && path == "/api/v1/clips/abc/thumb" {
                    f.thumb = Some(body);
                    (204, String::new())
                } else if method == "POST" && path == "/api/v1/clips/abc/complete" {
                    f.completed = true;
                    (
                        200,
                        format!(r#"{{"clip":{}}}"#, clip("ready", r#""http://x/c/abc""#)),
                    )
                } else {
                    (
                        404,
                        r#"{"error":{"code":"not_found","message":"Não encontrado."}}"#.into(),
                    )
                };
                drop(f);
                let _ = write!(
                    conn,
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp}",
                    resp.len()
                );
            }
        });
        (base, st)
    }

    fn meta(size: u64) -> NewClip {
        NewClip {
            title: "t".into(),
            game: None,
            visibility: Visibility::Unlisted,
            size_bytes: size,
            duration_ms: 1000,
            width: 2,
            height: 2,
        }
    }

    fn file(size: usize) -> (std::path::PathBuf, Vec<u8>) {
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        // Nome único por chamada: os testes rodam em paralelo e o Windows não deixa
        // reescrever um arquivo que outro teste ainda tem aberto.
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p =
            std::env::temp_dir().join(format!("app-cloud-{}-{n}-{size}.mp4", std::process::id()));
        std::fs::write(&p, &data).unwrap();
        (p, data)
    }

    #[test]
    fn envia_partes_miniatura_e_conclui() {
        let (path, data) = file(25);
        let (base, st) = fake_server(25);
        st.lock().unwrap().fail_next = 1; // a 1ª parte falha uma vez
        let c = Client::new(&base, Some("tok".into()));
        let mut seen: Vec<Progress> = Vec::new();
        let mut id = None;
        let clip = publish(
            &c,
            &path,
            Some(b"jpg"),
            &meta(25),
            None,
            &mut id,
            &mut |p| {
                seen.push(p);
                true
            },
        )
        .unwrap();
        assert_eq!(clip.url.as_deref(), Some("http://x/c/abc"));
        assert_eq!(id.as_deref(), Some("abc"));
        let f = st.lock().unwrap();
        let joined: Vec<u8> = f.parts.values().flatten().copied().collect();
        assert_eq!(joined, data);
        assert_eq!(f.parts.len(), 3);
        assert_eq!(f.thumb.as_deref(), Some(&b"jpg"[..]));
        assert!(f.completed);
        assert!(seen.windows(2).all(|w| w[0].sent <= w[1].sent));
        assert_eq!(seen.last().unwrap().stage, Stage::Finishing);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn retoma_so_o_que_falta() {
        let (path, data) = file(25);
        let (base, st) = fake_server(25);
        st.lock().unwrap().parts.insert(1, data[..PART].to_vec());
        let c = Client::new(&base, Some("tok".into()));
        let mut id = None;
        publish(
            &c,
            &path,
            None,
            &meta(25),
            Some("abc"),
            &mut id,
            &mut |_| true,
        )
        .unwrap();
        let f = st.lock().unwrap();
        let puts = f.log.iter().filter(|l| l.contains("/parts/")).count();
        assert_eq!(puts, 2, "{:?}", f.log);
        assert!(!f.log.iter().any(|l| l == "POST /api/v1/clips"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn cancelar_e_sessao_vencida() {
        let (path, _) = file(25);
        let (base, _) = fake_server(25);
        let c = Client::new(&base, Some("tok".into()));
        let mut id = None;
        let e = publish(&c, &path, None, &meta(25), None, &mut id, &mut |p| {
            p.sent < 12
        });
        assert!(matches!(e, Err(crate::CloudError::Cancelled)), "{e:?}");
        let bad = Client::new(&base, Some("outro".into()));
        let e = publish(&bad, &path, None, &meta(25), None, &mut None, &mut |_| true);
        assert!(matches!(e, Err(crate::CloudError::Unauthorized)), "{e:?}");
        std::fs::remove_file(path).ok();
    }
}
