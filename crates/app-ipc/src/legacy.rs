//! Migração do nome antigo (Clipe → Glint), chamada no início do gravador e da UI.
//! A lógica é a de `app_core::migrate`; aqui só entra o que depende do sistema.

use std::time::Duration;

use app_core::brand;
use app_core::migrate::{self, Hooks, Outcome, Paths};

/// Slug antigo: pipe `clipe-recorder-<usuário>`, credencial `clipe/token`.
const OLD_SLUG: &str = "clipe";

/// Roda a migração 1× (não faz nada se já foi feita ou não há dados antigos).
pub fn migrate() {
    let mut hooks = Hooks {
        stop_old_recorder: Box::new(|| {
            if crate::stop_recorder_at(&crate::socket_name_for(OLD_SLUG), Duration::from_secs(5)) {
                tracing::info!("gravador antigo fechado para migrar os dados");
            }
        }),
        move_token: Box::new(move_token),
        drop_old_autostart: Box::new(drop_old_autostart),
    };
    match migrate::run(&Paths::detect(), &mut hooks) {
        Ok(Outcome::Done { clips }) => tracing::info!(
            clipes = clips,
            "dados de {} migrados para {}",
            migrate::OLD_NAME,
            brand::APP_NAME
        ),
        Ok(Outcome::Nothing) => {}
        Err(e) => tracing::warn!(
            "migração de {}: {e} (tenta de novo na próxima abertura)",
            migrate::OLD_NAME
        ),
    }
}

#[cfg(windows)]
fn move_token() -> Result<(), String> {
    use app_platform_win::credentials;
    let old = format!("{OLD_SLUG}/token");
    let Some(token) = credentials::load(&old).filter(|t| !t.is_empty()) else {
        return Ok(());
    };
    let new = format!("{}/token", brand::APP_SLUG);
    credentials::save(&new, brand::APP_NAME, &token).map_err(|e| e.to_string())?;
    if credentials::load(&new).as_deref() != Some(token.as_str()) {
        return Err("token não conferiu depois de gravar".into());
    }
    credentials::delete(&old).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn move_token() -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn drop_old_autostart() -> Result<(), String> {
    // O gravador novo registra o dele ao abrir (só se as DLLs estiverem ao lado do exe).
    app_platform_win::autostart::set(migrate::OLD_NAME, None).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn drop_old_autostart() -> Result<(), String> {
    Ok(())
}
