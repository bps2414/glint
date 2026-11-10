//! Atualização em segundo plano pelo gravador (ele sobe com o Windows e fica na bandeja):
//! 60 s depois de subir e a cada 6 h, se `check_updates`, procura versão nova; achou, baixa o
//! instalador (SHA-256 conferido) numa thread de baixa prioridade e pede ao daemon para
//! instalar. O daemon só aceita com tudo ocioso (sem gravação, sem jogo, janela do app fechada);
//! senão tenta de novo a cada 10 min. O instalador roda em modo `/glintbg`: fecha o gravador,
//! instala e reabre só o gravador.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use app_cloud::update;
use app_core::brand::GITHUB_REPO;
use app_platform_win::windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END,
};

const FIRST_CHECK: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(6 * 3600);
const RETRY_INSTALL: Duration = Duration::from_secs(10 * 60);

/// Argumentos do instalador quando o gravador instala sozinho.
pub const INSTALLER_ARGS: &[&str] = &["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/glintbg"];

/// Sobe a thread. `enabled` acompanha `check_updates`; `try_install(instalador)` devolve
/// `true` quando o instalador foi aberto (a thread termina: o instalador fecha o gravador).
pub fn start(enabled: Arc<AtomicBool>, try_install: impl Fn(PathBuf) -> bool + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("autoupdate".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK);
            loop {
                if enabled.load(Ordering::Relaxed) {
                    if let Some(setup) = fetch() {
                        // Espera um momento ocioso (ou alguém desligar a atualização).
                        while enabled.load(Ordering::Relaxed) {
                            if try_install(setup.clone()) {
                                tracing::info!("atualização: instalador aberto");
                                return;
                            }
                            std::thread::sleep(RETRY_INSTALL);
                        }
                    }
                }
                std::thread::sleep(EVERY);
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("atualização em segundo plano indisponível: {e}");
    }
}

/// Versão nova baixada e conferida (`None` = não há, ou falhou).
fn fetch() -> Option<PathBuf> {
    let current = env!("CARGO_PKG_VERSION");
    let rel = match update::latest_release(GITHUB_REPO) {
        Ok(Some(r)) if update::is_newer(&r.version, current) => r,
        Ok(_) => return None,
        Err(e) => {
            tracing::info!("atualização: consulta falhou: {e}");
            return None;
        }
    };
    tracing::info!(versao = %rel.version, "atualização: baixando em segundo plano");
    let dest = std::env::temp_dir().join(&rel.file);
    // Baixa prioridade de CPU e de disco enquanto baixa (pode ter jogo rodando).
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
    let r = update::download(&rel, &dest, &mut |_, _| true);
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END);
    }
    match r {
        Ok(()) => Some(dest),
        Err(e) => {
            tracing::warn!("atualização: download falhou: {e}");
            None
        }
    }
}
