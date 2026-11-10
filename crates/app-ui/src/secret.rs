//! Segredos no Gerenciador de Credenciais do Windows: o token da conta ([`TOKEN`]), o token da
//! Cloudflare ([`CLOUDFLARE`]) e o `SETUP_TOKEN` de uma criação de servidor ainda não concluída
//! ([`SETUP`]). Nos outros sistemas, por enquanto, só na memória (pede login a cada execução).

use app_core::brand;

pub const TOKEN: &str = "token";
pub const CLOUDFLARE: &str = "cloudflare";
pub const SETUP: &str = "setup";

fn target(key: &str) -> String {
    format!("{}/{key}", brand::APP_SLUG)
}

/// Token da conta.
pub fn load() -> Option<String> {
    get(TOKEN)
}

pub fn save(token: &str) {
    put(TOKEN, token)
}

pub fn delete() {
    remove(TOKEN)
}

#[cfg(windows)]
pub fn get(key: &str) -> Option<String> {
    app_platform_win::credentials::load(&target(key)).filter(|t| !t.is_empty())
}

#[cfg(windows)]
pub fn put(key: &str, value: &str) {
    if let Err(e) = app_platform_win::credentials::save(&target(key), brand::APP_NAME, value) {
        tracing::warn!("{key} não gravado no Gerenciador de Credenciais: {e}");
    }
}

#[cfg(windows)]
pub fn remove(key: &str) {
    if let Err(e) = app_platform_win::credentials::delete(&target(key)) {
        tracing::warn!("{key} não apagado do Gerenciador de Credenciais: {e}");
    }
}

#[cfg(not(windows))]
static MEM: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

#[cfg(not(windows))]
pub fn get(key: &str) -> Option<String> {
    let k = target(key);
    MEM.lock()
        .ok()?
        .iter()
        .find(|(n, _)| *n == k)
        .map(|(_, v)| v.clone())
}

#[cfg(not(windows))]
pub fn put(key: &str, value: &str) {
    remove(key);
    if let Ok(mut m) = MEM.lock() {
        m.push((target(key), value.to_string()));
    }
}

#[cfg(not(windows))]
pub fn remove(key: &str) {
    let k = target(key);
    if let Ok(mut m) = MEM.lock() {
        m.retain(|(n, _)| *n != k);
    }
}
