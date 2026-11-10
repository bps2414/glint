//! Protocolo entre o gravador (servidor) e a interface (cliente).
//!
//! - Transporte: socket local do `interprocess` (named pipe no Windows,
//!   socket abstrato/Unix no Linux) — sem código de SO aqui.
//! - Mensagens: [`Request`] → [`Response`], serializadas com `postcard`, cada
//!   uma precedida do tamanho (`u32` little-endian, no máximo [`MAX_FRAME`]).
//! - Síncrono e simples: a UI pergunta o [`RecorderStatus`] quando precisa
//!   (o contador `clips_saved` avisa quando recarregar a biblioteca).

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Name, Stream};
use serde::{Deserialize, Serialize};

/// Muda quando o formato das mensagens muda de forma incompatível.
pub const PROTOCOL_VERSION: u32 = 3;

/// Maior mensagem aceita (protege contra lixo no pipe).
pub const MAX_FRAME: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("gravador não está rodando ({0})")]
    NotRunning(io::Error),
    #[error("E/S no IPC: {0}")]
    Io(#[from] io::Error),
    #[error("mensagem inválida: {0}")]
    Decode(#[from] postcard::Error),
    #[error("mensagem grande demais ({0} bytes)")]
    TooLarge(usize),
}

pub type Result<T> = std::result::Result<T, IpcError>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Primeiro contato: confere a versão do protocolo.
    Hello {
        version: u32,
    },
    Status,
    /// Mesmo efeito da hotkey.
    SaveClip,
    /// Liga/desliga a gravação automática ao detectar jogo.
    SetAutoDetect(bool),
    /// Grava a tela (monitor principal) agora, sem esperar um jogo.
    StartManual,
    /// Para a gravação atual (manual ou de jogo) e descarta o buffer.
    StopRecording,
    /// Relê o `config.toml` (a UI salvou mudanças).
    ReloadConfig,
    /// Encerra o gravador.
    Shutdown,
    /// Mesmo efeito da hotkey de kill (um aperto).
    SaveKill,
    /// Para a interface (socket [`ui_socket_name`]): outra execução do app passa a vez
    /// para a janela aberta, com o link `glint://` que recebeu (se houver).
    Activate {
        link: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Response {
    Hello { version: u32, app_version: String },
    Status(RecorderStatus),
    Ok,
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecorderState {
    /// Sem jogo: buffer desligado.
    Idle,
    /// Abrindo captura/encoder.
    Starting,
    /// Buffer rodando. `game = None` quando é gravação manual da tela.
    Recording { game: Option<String> },
    /// A última tentativa falhou; tenta de novo sozinho.
    Failed { error: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecorderStatus {
    pub state: RecorderState,
    pub auto_detect: bool,
    /// Segundos de vídeo no buffer agora.
    pub buffer_secs: f32,
    pub buffer_bytes: u64,
    /// Encoder em uso (ex.: `hevc_amf`).
    pub encoder: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Quadros perdidos desde o início da sessão.
    pub dropped_frames: u64,
    /// Clipes salvos desde que o gravador subiu (a UI recarrega quando muda).
    pub clips_saved: u64,
    pub last_clip: Option<String>,
    pub hotkey: String,
    /// A hotkey de salvar foi registrada (`false` = outro app já usa a tecla).
    pub hotkey_ok: bool,
    pub kill_hotkey: String,
    pub kill_hotkey_ok: bool,
    /// Apertos da hotkey de salvar desde que o gravador subiu, gravando ou não (teste do
    /// onboarding).
    pub save_presses: u64,
    /// Idem, hotkey de kill.
    pub kill_presses: u64,
    pub disk_used_bytes: u64,
    pub disk_limit_bytes: u64,
}

impl Default for RecorderStatus {
    fn default() -> Self {
        Self {
            state: RecorderState::Idle,
            auto_detect: true,
            buffer_secs: 0.0,
            buffer_bytes: 0,
            encoder: None,
            width: 0,
            height: 0,
            fps: 0,
            dropped_frames: 0,
            clips_saved: 0,
            last_clip: None,
            hotkey: String::new(),
            hotkey_ok: true,
            kill_hotkey: String::new(),
            kill_hotkey_ok: true,
            save_presses: 0,
            kill_presses: 0,
            disk_used_bytes: 0,
            disk_limit_bytes: 0,
        }
    }
}

/// Nome do socket do gravador (por usuário, pois named pipes são globais na máquina).
pub fn default_socket_name() -> String {
    socket_name_for(app_core::brand::APP_SLUG)
}

/// Nome do socket do gravador de um app com `slug` (o atual ou o nome antigo).
pub fn socket_name_for(slug: &str) -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let user: String = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(32)
        .collect();
    format!("{slug}-recorder-{user}")
}

/// Nome do socket da interface: a segunda execução do app manda [`Request::Activate`] para
/// a primeira e fecha (instância única da janela).
pub fn ui_socket_name() -> String {
    socket_name_for(app_core::brand::APP_SLUG).replace("-recorder-", "-ui-")
}

fn ns_name(name: &str) -> io::Result<Name<'_>> {
    name.to_ns_name::<GenericNamespaced>()
}

fn write_frame<T: Serialize>(w: &mut impl Write, msg: &T) -> Result<()> {
    let bytes = postcard::to_stdvec(msg)?;
    if bytes.len() > MAX_FRAME {
        return Err(IpcError::TooLarge(bytes.len()));
    }
    w.write_all(&(bytes.len() as u32).to_le_bytes())?;
    w.write_all(&bytes)?;
    w.flush()?;
    Ok(())
}

/// `Ok(None)` = o outro lado fechou entre mensagens.
fn read_frame<T: for<'de> Deserialize<'de>>(r: &mut impl Read) -> Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(IpcError::TooLarge(len));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(Some(postcard::from_bytes(&buf)?))
}

/// Conexão do lado da UI.
pub struct Client {
    stream: Stream,
}

impl Client {
    pub fn connect() -> Result<Self> {
        Self::connect_to(&default_socket_name())
    }

    pub fn connect_to(name: &str) -> Result<Self> {
        let stream = Stream::connect(ns_name(name)?).map_err(IpcError::NotRunning)?;
        Ok(Self { stream })
    }

    pub fn request(&mut self, req: &Request) -> Result<Response> {
        write_frame(&mut self.stream, req)?;
        read_frame(&mut self.stream)?.ok_or_else(|| {
            IpcError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "gravador fechou a conexão",
            ))
        })
    }

    /// Conecta, faz uma pergunta e desconecta.
    pub fn call(req: &Request) -> Result<Response> {
        Self::connect()?.request(req)
    }
}

/// Pede para o gravador em `name` fechar e espera ele sumir (até `wait`).
/// `false` = não estava rodando.
pub fn stop_recorder_at(name: &str, wait: std::time::Duration) -> bool {
    let Ok(mut c) = Client::connect_to(name) else {
        return false;
    };
    let _ = c.request(&Request::Shutdown);
    drop(c);
    let until = std::time::Instant::now() + wait;
    while std::time::Instant::now() < until {
        if Client::connect_to(name).is_err() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    true
}

pub mod legacy;

/// Lado do gravador: aceita conexões numa thread e responde cada pedido com
/// `handler`. Para ao ser solto (`drop`).
pub struct Server {
    name: String,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start<H>(name: &str, handler: H) -> Result<Self>
    where
        H: Fn(Request) -> Response + Send + Sync + 'static,
    {
        let listener = ListenerOptions::new()
            .name(ns_name(name)?)
            .try_overwrite(true)
            .create_sync()?;
        let stop = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(handler);
        let accept = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("ipc".into())
                .spawn(move || {
                    for conn in listener.incoming() {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        let conn = match conn {
                            Ok(c) => c,
                            Err(e) => {
                                tracing::warn!("IPC: conexão falhou: {e}");
                                continue;
                            }
                        };
                        let handler = handler.clone();
                        let stop = stop.clone();
                        let _ = std::thread::Builder::new()
                            .name("ipc-conn".into())
                            .spawn(move || serve(conn, &*handler, &stop));
                    }
                })?
        };
        tracing::info!(socket = name, "IPC ouvindo");
        Ok(Self {
            name: name.to_string(),
            stop,
            accept: Some(accept),
        })
    }
}

fn serve(
    mut conn: Stream,
    handler: &(dyn Fn(Request) -> Response + Send + Sync),
    stop: &AtomicBool,
) {
    loop {
        let req: Request = match read_frame(&mut conn) {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(e) => {
                tracing::debug!("IPC: mensagem inválida, fechando conexão: {e}");
                return;
            }
        };
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let resp = match req {
            Request::Hello { version } if version != PROTOCOL_VERSION => Response::Error(format!(
                "versão do protocolo diferente (gravador {PROTOCOL_VERSION}, cliente {version})"
            )),
            Request::Hello { .. } => Response::Hello {
                version: PROTOCOL_VERSION,
                app_version: env!("CARGO_PKG_VERSION").to_string(),
            },
            other => handler(other),
        };
        if write_frame(&mut conn, &resp).is_err() {
            return;
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Acorda o `incoming()` bloqueado com uma conexão que fica aberta até a
        // thread sair: no Windows, um cliente que fecha antes do servidor aceitar
        // é descartado em silêncio (ERROR_NO_DATA) e o accept continuaria esperando.
        match Client::connect_to(&self.name) {
            Ok(wake) => {
                if let Some(h) = self.accept.take() {
                    let _ = h.join();
                }
                drop(wake);
            }
            // Sem como acordar: deixa a thread solta (ela morre com o processo).
            Err(e) => tracing::debug!("IPC: não foi possível acordar o accept: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Client, IpcError, RecorderState, RecorderStatus, Request, Response, Server};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    fn unique(tag: &str) -> String {
        format!("app-ipc-test-{tag}-{}", std::process::id())
    }

    #[test]
    fn roundtrip_and_hello() {
        let name = unique("rt");
        let saves = Arc::new(AtomicU64::new(0));
        let s2 = saves.clone();
        let _server = Server::start(&name, move |req| match req {
            Request::Status => Response::Status(RecorderStatus {
                state: RecorderState::Recording {
                    game: Some("VALORANT".into()),
                },
                clips_saved: s2.load(Ordering::SeqCst),
                ..RecorderStatus::default()
            }),
            Request::SaveClip => {
                s2.fetch_add(1, Ordering::SeqCst);
                Response::Ok
            }
            _ => Response::Error("não suportado".into()),
        })
        .unwrap();

        let mut c = Client::connect_to(&name).unwrap();
        assert!(matches!(
            c.request(&Request::Hello {
                version: super::PROTOCOL_VERSION
            })
            .unwrap(),
            Response::Hello { .. }
        ));
        assert!(matches!(
            c.request(&Request::Hello { version: 999 }).unwrap(),
            Response::Error(_)
        ));
        assert_eq!(c.request(&Request::SaveClip).unwrap(), Response::Ok);
        assert!(matches!(
            c.request(&Request::SaveKill).unwrap(),
            Response::Error(_)
        ));
        match c.request(&Request::Status).unwrap() {
            Response::Status(s) => {
                assert_eq!(s.clips_saved, 1);
                assert_eq!(
                    s.state,
                    RecorderState::Recording {
                        game: Some("VALORANT".into())
                    }
                );
            }
            o => panic!("{o:?}"),
        }
        // Segunda conexão em paralelo.
        let mut c2 = Client::connect_to(&name).unwrap();
        assert_eq!(c2.request(&Request::SaveClip).unwrap(), Response::Ok);
        assert_eq!(saves.load(Ordering::SeqCst), 2);
        assert!(matches!(
            c.request(&Request::Shutdown).unwrap(),
            Response::Error(_)
        ));
    }

    #[test]
    fn not_running_is_reported() {
        match Client::connect_to(&unique("nobody")) {
            Err(IpcError::NotRunning(_)) => {}
            Err(e) => panic!("erro inesperado: {e}"),
            Ok(_) => panic!("conectou sem servidor"),
        }
    }

    #[test]
    fn rejects_huge_frames() {
        let mut bytes: &[u8] = &(u32::MAX).to_le_bytes();
        let r: super::Result<Option<Request>> = super::read_frame(&mut bytes);
        assert!(matches!(r, Err(IpcError::TooLarge(_))));
    }

    #[test]
    fn server_stops_on_drop() {
        let name = unique("drop");
        let server = Server::start(&name, |_| Response::Ok).unwrap();
        drop(server);
        // Depois de parar, um novo servidor pode usar o mesmo nome.
        let _again = Server::start(&name, |_| Response::Ok).unwrap();
        let mut c = Client::connect_to(&name).unwrap();
        assert_eq!(c.request(&Request::Status).unwrap(), Response::Ok);
    }
}
