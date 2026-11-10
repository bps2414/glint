//! Núcleo do app (nome em `brand`).
//!
//! Regra de arquitetura (`docs/ARCHITECTURE.md`): este crate não contém
//! nenhum código específico de sistema operacional. Captura, encoder de
//! hardware e áudio vivem em crates de plataforma.

pub mod brand;
pub mod config;
pub mod game;
pub mod library;
pub mod migrate;
pub mod naming;
pub mod packet;
pub mod replay;
pub mod retention;

pub use config::{
    CaptureBackend, CaptureMode, Codec, Config, Corner, GameConfig, KillConfig, Resolution,
};
pub use packet::{EncodedPacket, StreamId};
pub use replay::{ReplayBuffer, ReplaySnapshot};

/// Língua da interface a partir da tag do sistema (`sys_locale`): português → `pt-BR`,
/// qualquer outra → `en`.
pub fn ui_locale(os: Option<&str>) -> &'static str {
    match os {
        Some(tag) if tag.to_ascii_lowercase().starts_with("pt") => "pt-BR",
        _ => "en",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lingua_da_interface() {
        assert_eq!(super::ui_locale(Some("pt-BR")), "pt-BR");
        assert_eq!(super::ui_locale(Some("pt-PT")), "pt-BR");
        assert_eq!(super::ui_locale(Some("en-US")), "en");
        assert_eq!(super::ui_locale(Some("es-ES")), "en");
        assert_eq!(super::ui_locale(None), "en");
    }
}
