//! Configuração persistida em TOML (`%APPDATA%\<APP_NAME>\config\config.toml` no Windows).
//! Padrões de fábrica: os valores de `Default` abaixo.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    Hevc,
}

impl Codec {
    pub fn label(self) -> &'static str {
        match self {
            Codec::H264 => "H.264",
            Codec::Hevc => "HEVC",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resolution {
    /// 1280x720
    P720,
    /// 1920x1080
    P1080,
    /// Resolução da fonte, sem escala.
    Native,
}

impl Resolution {
    /// Altura alvo (largura segue a proporção da fonte).
    pub fn target_height(self) -> Option<u32> {
        match self {
            Resolution::P720 => Some(720),
            Resolution::P1080 => Some(1080),
            Resolution::Native => None,
        }
    }

    /// Calcula o tamanho de saída preservando proporção, sem aumentar e
    /// sempre com dimensões pares (exigência de NV12/encoders).
    pub fn output_size(self, src_w: u32, src_h: u32) -> (u32, u32) {
        let (w, h) = match self.target_height() {
            Some(th) if th < src_h => {
                let w = (u64::from(src_w) * u64::from(th) / u64::from(src_h)) as u32;
                (w, th)
            }
            _ => (src_w, src_h),
        };
        (w & !1, h & !1)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    pub codec: Codec,
    pub resolution: Resolution,
    pub fps: u32,
    /// Bitrate alvo em kbps.
    pub bitrate_kbps: u32,
    /// Intervalo entre keyframes em segundos (precisão do corte sem re-encode).
    pub keyframe_interval_secs: u32,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            codec: Codec::Hevc,
            resolution: Resolution::P1080,
            fps: 60,
            bitrate_kbps: 25_000,
            keyframe_interval_secs: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Captura o áudio do sistema (loopback). No futuro: só o processo do jogo.
    pub system: bool,
    /// Microfone em trilha separada.
    pub microphone: bool,
    /// Bitrate AAC por trilha em kbps.
    pub bitrate_kbps: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            system: true,
            microphone: true,
            bitrate_kbps: 160,
        }
    }
}

/// O que capturar quando um jogo é detectado.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureMode {
    /// O monitor onde está o jogo: grava o que estiver na tela, inclusive fora do jogo.
    Monitor,
    /// Só a janela do jogo (padrão): alt-tab não vaza o desktop para o clipe.
    Window,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameConfig {
    /// Liga o buffer sozinho quando um jogo abre (tela cheia/sem borda + GPU).
    pub auto_detect: bool,
    pub capture: CaptureMode,
    /// Executáveis sempre tratados como jogo, mesmo em janela (ex.: "minecraft.exe").
    pub include: Vec<String>,
    /// Executáveis nunca tratados como jogo (somam-se à lista interna).
    pub exclude: Vec<String>,
    /// Segundos seguidos em tela cheia antes de começar a gravar.
    pub confirm_secs: u32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            auto_detect: true,
            capture: CaptureMode::Window,
            include: Vec::new(),
            exclude: Vec::new(),
            confirm_secs: 3,
        }
    }
}

/// Canto do vídeo onde vai a marca d'água.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

impl Corner {
    pub const ALL: [Corner; 4] = [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomLeft,
        Corner::BottomRight,
    ];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Segundos guardados para trás.
    pub buffer_secs: u32,
    /// Teto de RAM do buffer em MB.
    pub buffer_max_mb: u32,
    /// Hotkey para salvar o clipe (ex.: "F8", "Alt+C").
    pub save_hotkey: String,
    /// Toca um som curto ao salvar.
    pub save_sound: bool,
    /// Pasta dos clipes. Vazio = `Vídeos\Clipe`.
    pub output_dir: PathBuf,
    /// Limite de disco em GB da pasta de clipes. Ao passar, os clipes mais
    /// antigos que não são favoritos nem publicados são apagados.
    pub disk_limit_gb: u32,
    /// Sobe o gravador (só ele, na bandeja) junto com o sistema.
    pub start_with_os: bool,
    /// Servidor onde os clipes são publicados (cada um sobe o seu). Vazio = sem servidor.
    pub server_url: String,
    /// Procura versão nova do app no servidor (ao abrir e a cada 6 h).
    pub check_updates: bool,
    /// Mostra "Clipping with Glint" e o tempo no perfil do Discord (Rich Presence) enquanto o
    /// gravador está aberto. Não precisa de servidor nem de login.
    pub discord_presence: bool,
    /// Já viu a tela de boas-vindas (primeiro uso).
    pub welcome_done: bool,
    /// Marca d'água (símbolo + "glint") no vídeo exportado/publicado; o clipe original
    /// nunca muda. Cada clipe pode trocar no painel Publicar.
    pub watermark: bool,
    pub watermark_corner: Corner,
    pub video: VideoConfig,
    pub audio: AudioConfig,
    pub game: GameConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            buffer_secs: 60,
            buffer_max_mb: 1024,
            save_hotkey: "F8".into(),
            save_sound: true,
            output_dir: PathBuf::new(),
            disk_limit_gb: 50,
            start_with_os: true,
            server_url: String::new(),
            check_updates: true,
            discord_presence: true,
            welcome_done: false,
            watermark: false,
            watermark_corner: Corner::BottomRight,
            video: VideoConfig::default(),
            audio: AudioConfig::default(),
            game: GameConfig::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("erro de E/S em {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("config inválida em {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("falha ao serializar config: {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl Config {
    /// Diretório de configuração (`%APPDATA%\Glint\config`).
    pub fn config_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", crate::brand::APP_NAME)
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Diretório de dados locais (`%LOCALAPPDATA%\Glint\data`): logs, banco, cache.
    pub fn data_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", crate::brand::APP_NAME)
            .map(|d| d.data_local_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn default_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// Banco SQLite da biblioteca de clipes.
    pub fn library_path() -> PathBuf {
        Self::data_dir().join("library.db")
    }

    /// Miniaturas dos clipes (fora da pasta de vídeos, para não poluí-la).
    pub fn thumbs_dir() -> PathBuf {
        Self::data_dir().join("thumbs")
    }

    pub fn disk_limit_bytes(&self) -> u64 {
        u64::from(self.disk_limit_gb) * 1_000_000_000
    }

    /// Pasta onde os clipes são salvos.
    pub fn clips_dir(&self) -> PathBuf {
        if !self.output_dir.as_os_str().is_empty() {
            return self.output_dir.clone();
        }
        directories::UserDirs::new()
            .and_then(|u| u.video_dir().map(|v| v.join(crate::brand::APP_NAME)))
            .unwrap_or_else(|| Self::data_dir().join("clips"))
    }

    /// Carrega a config; se não existir, cria com os padrões.
    pub fn load_or_create(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|source| ConfigError::Parse {
                path: path.to_path_buf(),
                source,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let cfg = Self::default();
                cfg.save(path)?;
                Ok(cfg)
            }
            Err(source) => Err(ConfigError::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let io = |source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_partial_files() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);

        // Arquivo parcial usa padrões no resto.
        let partial: Config =
            toml::from_str("buffer_secs = 30\n[video]\ncodec = \"h264\"\n").unwrap();
        assert_eq!(partial.buffer_secs, 30);
        assert_eq!(partial.video.codec, Codec::H264);
        assert_eq!(partial.video.fps, 60);
        assert_eq!(partial.save_hotkey, "F8");
        assert!(partial.game.auto_detect);
        assert_eq!(partial.game.capture, CaptureMode::Window);

        let games: Config =
            toml::from_str("[game]\ncapture = \"monitor\"\ninclude = [\"minecraft.exe\"]\n")
                .unwrap();
        assert_eq!(games.game.capture, CaptureMode::Monitor);
        assert_eq!(games.game.include, vec!["minecraft.exe".to_string()]);
        assert_eq!(games.game.confirm_secs, 3);

        // Config antiga (sem os campos da marca d'água): desligada, canto inferior direito.
        assert!(!partial.watermark);
        assert_eq!(partial.watermark_corner, Corner::BottomRight);
        let mark: Config =
            toml::from_str("watermark = true\nwatermark_corner = \"top_left\"\n").unwrap();
        assert!(mark.watermark);
        assert_eq!(mark.watermark_corner, Corner::TopLeft);
    }

    #[test]
    fn output_size_keeps_aspect_and_even() {
        assert_eq!(Resolution::P1080.output_size(2560, 1440), (1920, 1080));
        assert_eq!(Resolution::P720.output_size(1920, 1080), (1280, 720));
        assert_eq!(Resolution::P1080.output_size(1280, 720), (1280, 720));
        assert_eq!(Resolution::P1080.output_size(3440, 1440), (2580, 1080));
        assert_eq!(Resolution::Native.output_size(1919, 1081), (1918, 1080));
        assert_eq!(Resolution::P720.output_size(1680, 1050), (1152, 720));
    }
}
