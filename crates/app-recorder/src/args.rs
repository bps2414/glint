use app_core::{Codec, Resolution};

pub const USAGE: &str = "\
Gravador — mantém os últimos segundos e salva na hotkey

USO:
  app-recorder                 Roda na bandeja e grava sozinho quando um jogo abre.
  app-recorder [opções]        Modo manual no console (testes).

MODO MANUAL (console, Ctrl+C encerra):
  --window <texto>    Captura a janela cujo título contém <texto> (ex.: VALORANT).
                      Também grava só o áudio desse processo.
  --monitor           Captura o monitor principal.
  --secs <n>          Segundos no buffer (padrão: config, 60).
  --codec <c>         hevc | h264 (padrão: config, hevc).
  --res <r>           720 | 1080 | native (padrão: config, 1080).
  --fps <n>           Quadros por segundo (padrão: 60).
  --bitrate <kbps>    Bitrate do vídeo (padrão: config).
  --encoder <nome>    Força um encoder (ex.: hevc_amf, h264_nvenc).
  --no-audio          Não grava áudio do jogo/sistema.
  --no-mic            Não grava microfone.
  --list-windows      Lista as janelas visíveis e sai.
  --diagnose          Testa GPU, captura e encoders e sai.
  --measure-fps <pid> Mede o FPS do processo <pid> por --secs (padrão 30) via ETW
                      (sem injeção; precisa de administrador) e sai.

GRAVADOR NA BANDEJA (falam com ele pelo IPC):
  --status            Mostra o estado do gravador.
  --save              Salva um clipe agora (como a hotkey).
  --record-screen     Começa a gravar a tela (como o menu da bandeja).
  --stop              Para a gravação atual.
  --quit              Encerra o gravador.

  -h, --help          Esta ajuda.

Hotkey de salvar: config (padrão F8). Ctrl+C encerra.
Config: %APPDATA%\\<app>\\config.toml   Logs: %LOCALAPPDATA%\\<app>\\logs";

#[derive(Debug, Default)]
pub struct Args {
    pub window: Option<String>,
    /// Modo manual capturando o monitor principal.
    pub monitor: bool,
    pub secs: Option<u32>,
    pub codec: Option<Codec>,
    pub res: Option<Resolution>,
    pub fps: Option<u32>,
    pub bitrate: Option<u32>,
    pub encoder: Option<String>,
    pub no_audio: bool,
    pub no_mic: bool,
    pub list_windows: bool,
    pub diagnose: bool,
    /// PID cujo FPS medir (`--measure-fps`).
    pub measure_fps: Option<u32>,
    pub status: bool,
    pub save: bool,
    pub record_screen: bool,
    pub stop: bool,
    pub quit: bool,
    /// Iniciado pelo Windows no login (entrada `Run`).
    pub autostart: bool,
    pub help: bool,
}

/// O que o processo vai fazer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Daemon,
    Manual,
    Diagnose,
    MeasureFps,
    ListWindows,
    Ipc,
    Help,
}

impl Args {
    pub fn mode(&self) -> Mode {
        if self.help {
            Mode::Help
        } else if self.list_windows {
            Mode::ListWindows
        } else if self.diagnose {
            Mode::Diagnose
        } else if self.measure_fps.is_some() {
            Mode::MeasureFps
        } else if self.status || self.save || self.quit || self.record_screen || self.stop {
            Mode::Ipc
        } else if self.window.is_some() || self.monitor {
            Mode::Manual
        } else {
            Mode::Daemon
        }
    }
}

impl Args {
    pub fn parse(mut it: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut a = Args::default();
        fn value(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
            it.next()
                .ok_or_else(|| format!("{flag} precisa de um valor"))
        }
        fn num(s: String, flag: &str) -> Result<u32, String> {
            s.parse()
                .map_err(|_| format!("{flag}: número inválido '{s}'"))
        }
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--window" => {
                    a.window = Some(value(&mut it, &arg)?);
                    a.monitor = false;
                }
                "--monitor" => {
                    a.window = None;
                    a.monitor = true;
                }
                "--secs" => a.secs = Some(num(value(&mut it, &arg)?, &arg)?),
                "--fps" => a.fps = Some(num(value(&mut it, &arg)?, &arg)?),
                "--bitrate" => a.bitrate = Some(num(value(&mut it, &arg)?, &arg)?),
                "--encoder" => a.encoder = Some(value(&mut it, &arg)?),
                "--codec" => {
                    a.codec = Some(match value(&mut it, &arg)?.to_lowercase().as_str() {
                        "hevc" | "h265" => Codec::Hevc,
                        "h264" | "avc" => Codec::H264,
                        o => return Err(format!("codec desconhecido '{o}'")),
                    })
                }
                "--res" => {
                    a.res = Some(match value(&mut it, &arg)?.to_lowercase().as_str() {
                        "720" | "720p" => Resolution::P720,
                        "1080" | "1080p" => Resolution::P1080,
                        "native" | "nativa" => Resolution::Native,
                        o => return Err(format!("resolução desconhecida '{o}'")),
                    })
                }
                "--no-audio" => a.no_audio = true,
                "--no-mic" => a.no_mic = true,
                "--list-windows" => a.list_windows = true,
                "--diagnose" => a.diagnose = true,
                "--measure-fps" => a.measure_fps = Some(num(value(&mut it, &arg)?, &arg)?),
                "--status" => a.status = true,
                "--save" => a.save = true,
                "--record-screen" => a.record_screen = true,
                "--stop" => a.stop = true,
                "--quit" => a.quit = true,
                "--autostart" => a.autostart = true,
                "-h" | "--help" => a.help = true,
                o => return Err(format!("opção desconhecida '{o}'")),
            }
        }
        Ok(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Args, String> {
        Args::parse(s.split_whitespace().map(String::from))
    }

    #[test]
    fn parses() {
        let a = p("--window VALORANT --secs 30 --codec h264 --res 720 --no-mic").unwrap();
        assert_eq!(a.window.as_deref(), Some("VALORANT"));
        assert_eq!(a.secs, Some(30));
        assert_eq!(a.codec, Some(Codec::H264));
        assert_eq!(a.res, Some(Resolution::P720));
        assert!(a.no_mic && !a.no_audio);
        assert!(p("--secs abc").is_err());
        assert!(p("--wat").is_err());
        assert!(p("--window").is_err());
    }

    #[test]
    fn modes() {
        assert_eq!(p("").unwrap().mode(), Mode::Daemon);
        assert_eq!(p("--autostart").unwrap().mode(), Mode::Daemon);
        assert_eq!(p("--monitor").unwrap().mode(), Mode::Manual);
        assert_eq!(p("--window x --secs 5").unwrap().mode(), Mode::Manual);
        assert_eq!(p("--diagnose").unwrap().mode(), Mode::Diagnose);
        assert_eq!(p("--status").unwrap().mode(), Mode::Ipc);
        let m = p("--measure-fps 1234 --secs 10").unwrap();
        assert_eq!(m.mode(), Mode::MeasureFps);
        assert_eq!((m.measure_fps, m.secs), (Some(1234), Some(10)));
        assert_eq!(p("--save").unwrap().mode(), Mode::Ipc);
        assert_eq!(p("--record-screen").unwrap().mode(), Mode::Ipc);
        assert_eq!(p("--stop").unwrap().mode(), Mode::Ipc);
        assert_eq!(p("--help --save").unwrap().mode(), Mode::Help);
    }
}
