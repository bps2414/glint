//! Backend Windows do app.
//!
//! - [`gpu::Gpu`]: dispositivo D3D11 compartilhado (captura, conversão, encoder).
//! - [`capture::Capture`]: Windows Graphics Capture (sem injeção no jogo).
//! - [`convert::Converter`]: BGRA → NV12 + escala via D3D11 VideoProcessor.
//! - [`audio`]: WASAPI loopback (sistema ou processo) e microfone.
//! - [`hotkey`]: hotkeys globais (RegisterHotKey).
//! - [`sound`]: "blip" de confirmação e "tick" do clipe de kill.
//! - [`clock`]: relógio comum (QPC em µs) para vídeo e áudio.
//! - [`foreground`]: janela em foco, processo vivo e uso de GPU (detecção de jogo).
//! - [`tray`]: ícone na bandeja; [`tray_menu`]: o menu de vidro dela.
//! - [`power`]: EcoQoS e prioridades; [`autostart`]: iniciar com o Windows.
//! - [`present`]: FPS de outro processo pelos eventos ETW do DXGI (teste guiado).
//! - [`process`]: CPU e memória do próprio processo.
//! - [`credentials`]: token da conta no Gerenciador de Credenciais.
//! - [`instance`]: instância única; [`console`]: console nos modos de linha de comando.
#![cfg(windows)]

pub mod appearance;
pub mod audio;
pub mod autostart;
pub mod capture;
pub mod clock;
pub mod console;
pub mod convert;
pub mod credentials;
pub mod foreground;
pub mod gpu;
pub mod hotkey;
pub mod instance;
pub mod power;
pub mod present;
pub mod process;
pub mod shell;
pub mod sound;
pub mod tray;
pub mod tray_menu;
pub mod window;

pub use windows;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Windows: {0}")]
    Win(#[from] windows::core::Error),
    #[error("WASAPI: {0}")]
    Wasapi(String),
    #[error("{0}")]
    Other(String),
}
