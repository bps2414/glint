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

/// Aplicação do Discord usada no Rich Presence ("Clipping with Glint"). O ID é público e fica
/// fixo no app: quem usa o Glint não precisa criar nada no Developer Portal. O nome mostrado no
/// perfil é o nome dessa aplicação no Portal.
pub const DISCORD_APP_ID: &str = "1567585264858308689";

/// Para onde o botão do Rich Presence leva (trocar pela landing page quando existir).
pub const PRESENCE_URL: &str = "https://github.com/bps2414/glint";

/// Imagem grande do Rich Presence (URL pública; o Discord aceita URL no lugar de asset).
pub const PRESENCE_IMAGE: &str =
    "https://raw.githubusercontent.com/bps2414/glint/main/assets/brand/glint-512.png";

/// Prefixo dos arquivos de clipe (`<prefixo>_2026-11-04_21-30-00.mp4`).
pub const CLIP_FILE_PREFIX: &str = APP_NAME;

/// Variável de ambiente para o nível de log (ex.: `CLIPE_LOG=debug`).
pub fn log_env_var() -> String {
    format!("{}_LOG", APP_SLUG.to_uppercase())
}

/// Página do repositório (botão "Leave a star"; dar star exige conta no GitHub).
pub const REPO_URL: &str = "https://github.com/bps2414/glint";

/// Worker do mantenedor que recebe feedback e bug reports (`server/feedback/`).
pub const FEEDBACK_URL: &str = "https://feedback.getglint.workers.dev/v1/report";
