//! Identidade do app num lugar só.
//!
//! Nome: Glint (Fase 5; antes "Clipe"). Para renomear, mude as constantes abaixo
//! (ou rode `scripts/rename-brand.sh NovoNome`). Nada no código deve escrever
//! o nome do app "na mão" — use estas constantes.
//!
//! Atenção: "clipe" também é a palavra comum em português (um clipe de vídeo);
//! textos como "clipe salvo" usam a palavra comum, não a marca.

/// Nome exibido para o usuário (títulos, console, pastas).
pub const APP_NAME: &str = "Glint";

/// Identificador em minúsculas (nomes de executável, variáveis de ambiente).
pub const APP_SLUG: &str = "glint";

/// Repositório do GitHub (`dono/nome`) onde saem os releases; o atualizador lê
/// `https://github.com/<repo>/releases/latest/download/latest.json`.
pub const GITHUB_REPO: &str = "bps2414/glint";

/// Prefixo dos arquivos de clipe (`<prefixo>_2026-11-04_21-30-00.mp4`).
pub const CLIP_FILE_PREFIX: &str = APP_NAME;

/// Variável de ambiente para o nível de log (ex.: `CLIPE_LOG=debug`).
pub fn log_env_var() -> String {
    format!("{}_LOG", APP_SLUG.to_uppercase())
}
