//! Token da conta: no Gerenciador de Credenciais do Windows. Nos outros
//! sistemas, por enquanto, só na memória (pede login a cada execução).

use app_core::brand;

fn target() -> String {
    format!("{}/token", brand::APP_SLUG)
}

#[cfg(windows)]
pub fn load() -> Option<String> {
    app_platform_win::credentials::load(&target()).filter(|t| !t.is_empty())
}

#[cfg(windows)]
pub fn save(token: &str) {
    if let Err(e) = app_platform_win::credentials::save(&target(), brand::APP_NAME, token) {
        tracing::warn!("token não gravado no Gerenciador de Credenciais: {e}");
    }
}

#[cfg(windows)]
pub fn delete() {
    if let Err(e) = app_platform_win::credentials::delete(&target()) {
        tracing::warn!("token não apagado do Gerenciador de Credenciais: {e}");
    }
}

#[cfg(not(windows))]
static MEM: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[cfg(not(windows))]
pub fn load() -> Option<String> {
    let _ = target();
    MEM.lock().ok()?.clone()
}

#[cfg(not(windows))]
pub fn save(token: &str) {
    if let Ok(mut m) = MEM.lock() {
        *m = Some(token.to_string());
    }
}

#[cfg(not(windows))]
pub fn delete() {
    if let Ok(mut m) = MEM.lock() {
        *m = None;
    }
}
