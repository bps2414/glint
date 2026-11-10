//! Feedback e bug report do app para o Worker do mantenedor (`server/feedback/`), não para o
//! servidor de clipes. A mensagem vira issue pública no GitHub; logs e contato ficam privados.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::Agent;

use crate::api::CloudError;

/// Limite dos logs enviados (o Worker aceita até 512 Ki caracteres).
pub const MAX_LOGS: usize = 500 * 1024;
pub const MAX_MESSAGE: usize = 5000;
pub const MAX_CONTACT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Bug,
    Feedback,
}

/// Informações do PC que acompanham o report.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Specs {
    pub app_version: String,
    pub os: String,
    pub gpu: String,
    pub capture_backend: String,
    pub locale: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub kind: Kind,
    pub message: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub contact: String,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub specs: Option<Specs>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub logs: String,
}

impl Report {
    /// Texto exatamente como vai ser enviado ("Ver o que será enviado").
    pub fn preview(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct Sent {
    id: String,
}

/// Envia e devolve o id do report.
pub fn send_report(url: &str, app_version: &str, r: &Report) -> Result<String, CloudError> {
    let agent: Agent = Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    let mut resp = agent
        .post(url)
        .header("X-Glint-Client", app_version)
        .send_json(r)?;
    let status = resp.status().as_u16();
    if status == 429 {
        return Err(CloudError::Other("rate_limited".into()));
    }
    if status >= 400 {
        return Err(CloudError::Other(format!("feedback: HTTP {status}")));
    }
    Ok(resp.body_mut().read_json::<Sent>()?.id)
}

/// As 2 últimas `recorder*.log` de `dir` (a mais velha antes), limpas e cortadas no final para
/// caber em [`MAX_LOGS`]. Vazio se não houver log.
pub fn collect_logs(dir: &Path) -> String {
    let Ok(read) = std::fs::read_dir(dir) else {
        return String::new();
    };
    let mut files: Vec<_> = read
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            n.starts_with("recorder") && n.contains(".log")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    let mut out = String::new();
    for (_, path) in files.into_iter().take(2).rev() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        out.push_str(&format!("===== {name} =====\n"));
        out.push_str(&String::from_utf8_lossy(&bytes));
        out.push('\n');
    }
    let user = std::env::var("USERNAME").unwrap_or_default();
    tail(&scrub(&out, &user), MAX_LOGS)
}

/// Final de `s` com no máximo `max` bytes (cortado num começo de linha).
fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    let cut = &s[start..];
    let cut = cut.find('\n').map_or(cut, |i| &cut[i + 1..]);
    format!("[...]\n{cut}")
}

/// Tira dado pessoal dos logs: pasta do perfil (`\Users\<nome>\`), nome do usuário do Windows,
/// `Bearer <token>` e query strings de URLs (links assinados).
pub fn scrub(text: &str, user: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let mut l = redact_users_dir(line);
        if user.chars().count() >= 3 {
            l = replace_ci(&l, user, "<user>");
        }
        l = redact_after(&l, "Bearer ", "<redacted>");
        l = redact_queries(&l);
        out.push_str(&l);
        out.push('\n');
    }
    out
}

fn redact_users_dir(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let mut out = String::new();
    let mut i = 0;
    while let Some(p) = find_users(&lower[i..]) {
        let start = i + p.0 + p.1; // depois de "\users\"
        out.push_str(&line[i..start]);
        let end = line[start..]
            .find(['\\', '/', ' ', '"', '\''])
            .map_or(line.len(), |e| start + e);
        out.push_str("<user>");
        i = end;
    }
    out.push_str(&line[i..]);
    out
}

/// Posição e tamanho de `\users\` ou `/users/` em `s` (já em minúsculas).
fn find_users(s: &str) -> Option<(usize, usize)> {
    [r"\users\", "/users/"]
        .iter()
        .filter_map(|n| s.find(n).map(|p| (p, n.len())))
        .min_by_key(|x| x.0)
}

fn replace_ci(line: &str, needle: &str, with: &str) -> String {
    let lower = line.to_lowercase();
    let n = needle.to_lowercase();
    if lower.len() != line.len() || !lower.contains(&n) {
        return line.to_string();
    }
    let mut out = String::new();
    let mut i = 0;
    while let Some(p) = lower[i..].find(&n) {
        out.push_str(&line[i..i + p]);
        out.push_str(with);
        i += p + n.len();
    }
    out.push_str(&line[i..]);
    out
}

/// Troca o que vem depois de `marker` até o próximo espaço/aspas.
fn redact_after(line: &str, marker: &str, with: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some(p) = line[i..].find(marker) {
        let start = i + p + marker.len();
        out.push_str(&line[i..start]);
        out.push_str(with);
        i = line[start..]
            .find([' ', '"', '\'', ','])
            .map_or(line.len(), |e| start + e);
    }
    out.push_str(&line[i..]);
    out
}

/// `http(s)://host/path?query` → `http(s)://host/path?<redacted>`.
fn redact_queries(line: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some(p) = line[i..].find("http") {
        let start = i + p;
        let end = line[start..]
            .find([' ', '"', '\'', ')', '>'])
            .map_or(line.len(), |e| start + e);
        let url = &line[start..end];
        out.push_str(&line[i..start]);
        match url.find('?') {
            Some(q) => {
                out.push_str(&url[..q]);
                out.push_str("?<redacted>");
            }
            None => out.push_str(url),
        }
        i = end;
    }
    out.push_str(&line[i..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limpa_dados_pessoais() {
        let s = scrub(
            "salvo em C:\\Users\\Bryan\\Videos\\x.mp4 por bryan, Authorization: Bearer abc.def end https://x.dev/v?sig=1&e=2 ok",
            "Bryan",
        );
        assert_eq!(
            s,
            "salvo em C:\\Users\\<user>\\Videos\\x.mp4 por <user>, Authorization: Bearer <redacted> end https://x.dev/v?<redacted> ok\n"
        );
    }

    #[test]
    fn corta_no_final_numa_linha() {
        let s = "aaaa\nbbbb\ncccc\n";
        assert_eq!(tail(s, 7), "[...]\ncccc\n");
        assert_eq!(tail(s, 100), s);
    }

    #[test]
    fn json_sem_campos_vazios() {
        let r = Report {
            kind: Kind::Feedback,
            message: "oi".into(),
            contact: String::new(),
            specs: None,
            logs: String::new(),
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"kind":"feedback","message":"oi"}"#
        );
    }
}
