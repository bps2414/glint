//! Cliente do servidor de clipes (contrato em `docs/API.md`).
//!
//! - [`Client`]: chamadas da API com o token Bearer.
//! - [`login`]: login do app por loopback + PKCE (RFC 8252): abre o navegador
//!   uma vez e troca o código pelo token.
//! - [`publish`]: envio em partes de 8 MiB (com retomada), miniatura e conclusão.
//! - [`update`]: versão mais nova do app e download do instalador (SHA-256 conferido).
//!
//! Tudo é bloqueante: a interface chama de uma thread de fundo.

mod api;
pub mod login;
mod publish;
pub mod update;

pub use api::{
    clip_id_from_url, ApiErrorBody, Client, Clip, CloudError, Me, NewClip, UploadState, Visibility,
};
pub use publish::{publish, Progress, Stage};
