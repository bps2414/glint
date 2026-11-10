//! Uma janela só: a segunda execução do app (atalho, ou o Windows abrindo um link
//! `glint://` do navegador) passa o link para a janela aberta pelo IPC e fecha.

use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;

use app_ipc::{Client, Request, Response, Server};

/// Pedidos que chegam de outras execuções; viva enquanto a janela existir.
pub struct Inbox {
    _server: Option<Server>,
    rx: Receiver<Option<String>>,
}

impl Inbox {
    /// Próximo pedido sem esperar: `Some(link)` (o link pode ser `None`: só trazer a janela).
    pub fn try_next(&self) -> Option<Option<String>> {
        self.rx.try_recv().ok()
    }
}

/// `None` = outra janela já estava aberta e recebeu `link`; esta execução deve fechar.
pub fn claim(link: Option<String>) -> Option<Inbox> {
    let name = app_ipc::ui_socket_name();
    if let Ok(mut c) = Client::connect_to(&name) {
        #[cfg(windows)]
        app_platform_win::window::allow_foreground();
        if matches!(
            c.request(&Request::Activate { link: link.clone() }),
            Ok(Response::Ok)
        ) {
            return None;
        }
    }
    let (tx, rx) = channel();
    let tx = Mutex::new(tx);
    let server = Server::start(&name, move |req| match req {
        Request::Activate { link } => {
            let _ = tx.lock().map(|t| t.send(link));
            Response::Ok
        }
        _ => Response::Error("socket da interface".into()),
    })
    .map_err(|e| tracing::warn!("instância única da janela: {e}"))
    .ok();
    Some(Inbox {
        _server: server,
        rx,
    })
}
