//! Rich Presence do Discord: "Clipping VALORANT with Glint" enquanto grava um jogo ("Clipping with
//! Glint" fora dele), tempo desde que o gravador abriu e um botão para o site. Igual ao Medal,
//! aparece enquanto o gravador está na bandeja.
//!
//! Roda numa thread própria porque falar com o Discord bloqueia (pipe local) e o laço do daemon
//! não pode atrasar. Discord fechado é o caso normal: tenta de novo a cada `RETRY`, sem erro na
//! tela. Usa a aplicação do Discord fixa em `brand::DISCORD_APP_ID`, então não precisa de servidor,
//! login nem configurar nada.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use app_core::brand;
use discord_rich_presence::activity::{
    Activity, ActivityType, Assets, Button, StatusDisplayType, Timestamps,
};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

/// Intervalo entre tentativas de conectar e entre reenvios da atividade. O reenvio percebe quando o
/// Discord reiniciou; o Discord aceita até 5 atualizações a cada 20 s.
const RETRY: Duration = Duration::from_secs(20);

enum Cmd {
    Enable(bool),
    Game(Option<String>),
    Stop,
}

pub struct Presence {
    tx: Sender<Cmd>,
    done: Receiver<()>,
    /// Último jogo enviado (evita mandar o mesmo a cada tick).
    game: Option<String>,
}

impl Presence {
    pub fn start(enabled: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let spawned = std::thread::Builder::new()
            .name("discord-presence".into())
            .spawn(move || {
                worker(rx, enabled, started);
                let _ = done_tx.send(());
            });
        if let Err(e) = spawned {
            tracing::warn!("Rich Presence do Discord indisponível: {e}");
        }
        Self {
            tx,
            done,
            game: None,
        }
    }

    pub fn set_enabled(&self, on: bool) {
        let _ = self.tx.send(Cmd::Enable(on));
    }

    /// Jogo que está sendo gravado agora (`None` = nenhum). Só manda ao Discord quando muda.
    pub fn set_game(&mut self, game: Option<String>) {
        if game != self.game {
            self.game = game.clone();
            let _ = self.tx.send(Cmd::Game(game));
        }
    }

    /// Limpa a atividade no Discord antes de o gravador sair (espera pouco: o Discord pode estar travado).
    pub fn stop(&self) {
        if self.tx.send(Cmd::Stop).is_ok() {
            let _ = self.done.recv_timeout(Duration::from_secs(2));
        }
    }
}

fn worker(rx: Receiver<Cmd>, mut enabled: bool, started: i64) {
    let mut client: Option<DiscordIpcClient> = None;
    let mut game: Option<String> = None;
    loop {
        if enabled {
            client = refresh(client.take(), started, game.as_deref());
        }
        // Desligado: dorme até chegar um comando; ligado: acorda a cada `RETRY` para reenviar.
        let cmd = if enabled {
            match rx.recv_timeout(RETRY) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => Some(Cmd::Stop),
            }
        } else {
            Some(rx.recv().unwrap_or(Cmd::Stop))
        };
        match cmd {
            Some(Cmd::Enable(on)) => {
                enabled = on;
                if !on {
                    disconnect(&mut client);
                }
            }
            Some(Cmd::Game(g)) => game = g,
            Some(Cmd::Stop) => {
                disconnect(&mut client);
                return;
            }
            None => {}
        }
    }
}

/// Conecta se preciso e (re)envia a atividade. Devolve `None` se o Discord não respondeu.
fn refresh(
    client: Option<DiscordIpcClient>,
    started: i64,
    game: Option<&str>,
) -> Option<DiscordIpcClient> {
    let fresh = client.is_none();
    let mut c = match client {
        Some(c) => c,
        None => {
            let mut c = DiscordIpcClient::new(brand::DISCORD_APP_ID);
            if let Err(e) = c.connect() {
                tracing::debug!("Discord não encontrado: {e}");
                return None;
            }
            c
        }
    };
    match send_activity(&mut c, started, game) {
        Ok(()) => {
            if fresh {
                tracing::info!("Rich Presence do Discord ativo");
            }
            Some(c)
        }
        Err(e) => {
            tracing::debug!("Discord desconectou: {e}");
            let _ = c.close();
            None
        }
    }
}

fn send_activity(c: &mut DiscordIpcClient, started: i64, game: Option<&str>) -> Result<(), String> {
    let details = match game {
        Some(g) if !g.is_empty() => format!("Clipping {g} with {}", brand::APP_NAME),
        _ => format!("Clipping with {}", brand::APP_NAME),
    };
    let large_text = format!("{} {}", brand::APP_NAME, env!("CARGO_PKG_VERSION"));
    let activity = Activity::new()
        .activity_type(ActivityType::Playing)
        .status_display_type(StatusDisplayType::Details)
        .details(details.as_str())
        .timestamps(Timestamps::new().start(started))
        .assets(
            Assets::new()
                .large_image(brand::PRESENCE_IMAGE)
                .large_text(large_text.as_str()),
        )
        .buttons(vec![Button::new(
            format!("Get {}", brand::APP_NAME),
            brand::PRESENCE_URL,
        )]);
    c.set_activity(activity).map_err(|e| e.to_string())?;
    // O Discord responde cada comando; ler a resposta impede que o pipe encha e mostra erros de
    // payload (que o crate não confere).
    let (_, resp) = c.recv().map_err(|e| e.to_string())?;
    if resp.get("evt").and_then(|v| v.as_str()) == Some("ERROR") {
        tracing::warn!("Discord recusou o Rich Presence: {}", resp["data"]);
    }
    Ok(())
}

fn disconnect(client: &mut Option<DiscordIpcClient>) {
    if let Some(mut c) = client.take() {
        if c.clear_activity().is_ok() {
            let _ = c.recv();
        }
        let _ = c.close();
    }
}
