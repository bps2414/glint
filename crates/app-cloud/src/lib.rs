//! Cliente do servidor de clipes (contrato em `docs/API.md`).
//!
//! - [`Client`]: chamadas da API com o token Bearer.
//! - [`login`]: login do app por loopback + PKCE (RFC 8252): abre o navegador
//!   uma vez e troca o código pelo token.
//! - [`publish`]: envio em partes de 8 MiB (com retomada), miniatura e conclusão.
//! - [`invite`]: link de convite (`https://.../i/<código>` ou `glint://join`).
//! - [`cloudflare`]: cria ou atualiza o servidor na conta Cloudflare do usuário (API REST).
//! - [`feedback`]: feedback e bug report para o Worker do mantenedor (não o servidor de clipes).
//! - [`update`]: versão mais nova do app e download do instalador (SHA-256 conferido).
//!
//! Tudo é bloqueante: a interface chama de uma thread de fundo.

mod api;
pub mod cloudflare;
pub mod feedback;
pub mod invite;
pub mod login;
mod publish;
pub mod update;

pub use api::{
    clip_id_from_url, ApiErrorBody, Cleanup, Client, Clip, CloudError, Device, LinkCode, Me,
    Member, NewClip, ServerInfo, UploadState, User, Visibility, API_VERSION,
};
pub use publish::{publish, Progress, Stage};
