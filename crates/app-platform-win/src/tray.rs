//! Ícone na bandeja (área de notificação) com o menu de vidro ([`crate::tray_menu`]).
//!
//! O `tray-icon` exige um loop de mensagens Win32 na thread que criou o ícone:
//! ele vive numa thread própria. Comandos do menu saem por um callback; o
//! estado (texto, checks, ícone) entra por [`Tray::update`], que acorda a
//! thread com uma mensagem `WM_APP`. Clique direito no ícone → `WM_APP + 1` →
//! abre o menu junto do cursor.
//!
//! Ícone (Fase 5): anel chapado + ponto (parado), anel + ponto vermelho (gravando),
//! anel + selo vermelho (erro). O anel é branco na barra escura e preto na clara, e o
//! desenho sai no tamanho do DPI (16/20/24/32).

use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, GetSystemMetricsForDpi, SetThreadDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetCursorPos, GetMessageW, PostThreadMessageW, TranslateMessage, MSG,
    SM_CXSMICON, WM_APP, WM_QUIT,
};

use crate::tray_menu::{native_menu, MenuEntry, MenuModel, TrayMenu};
use crate::{Error, Result};

/// Mensagem da thread: abrir o menu no cursor.
const WM_SHOW_MENU: u32 = WM_APP + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    SaveClip,
    /// Clipe de kill (mesmo efeito de apertar a hotkey de kill).
    SaveKill,
    OpenLibrary,
    OpenFolder,
    ToggleAutoDetect,
    /// Gravar a tela agora / parar a gravação (o texto muda com o estado).
    ToggleManual,
    ToggleStartWithOs,
    Quit,
}

/// Tudo que o menu mostra (textos já traduzidos por quem chama).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayState {
    pub tooltip: String,
    /// Primeira linha do menu (desabilitada), ex.: "Gravando: VALORANT".
    pub status: String,
    pub recording: bool,
    /// Sessão com falha/backoff (selo vermelho no ícone).
    pub failed: bool,
    pub save_label: String,
    pub kill_label: String,
    pub can_save: bool,
    pub manual_label: String,
    pub auto_detect_label: String,
    pub auto_detect: bool,
    pub library_label: String,
    pub folder_label: String,
    pub start_with_os_label: String,
    pub start_with_os: bool,
    pub quit_label: String,
}

pub struct Tray {
    thread_id: u32,
    state: Arc<Mutex<TrayState>>,
    handle: Option<JoinHandle<()>>,
}

impl Tray {
    /// `on_command` roda na thread da bandeja: deve só repassar (ex.: enviar num canal).
    pub fn start(
        initial: TrayState,
        on_command: impl Fn(TrayCommand) + Send + Sync + 'static,
    ) -> Result<Self> {
        let commands: Arc<dyn Fn(TrayCommand) + Send + Sync> = Arc::new(on_command);
        let state = Arc::new(Mutex::new(initial));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<u32, String>>();
        let handle = {
            let state = state.clone();
            std::thread::Builder::new()
                .name("tray".into())
                .spawn(move || {
                    crate::power::set_current_thread_background();
                    if let Err(e) = run(state, commands, &ready_tx) {
                        let _ = ready_tx.send(Err(e));
                    }
                })
                .map_err(|e| Error::Other(e.to_string()))?
        };
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                state,
                handle: Some(handle),
            }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(Error::Other(format!("bandeja: {e}")))
            }
            Err(_) => Err(Error::Other("thread da bandeja morreu".into())),
        }
    }

    /// Troca o estado mostrado (barato se nada mudou).
    pub fn update(&self, new: TrayState) {
        {
            let mut s = self.state.lock();
            if *s == new {
                return;
            }
            *s = new;
        }
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_APP, WPARAM(0), LPARAM(0));
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Como o ícone aparece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayLook {
    Idle,
    Recording,
    Error,
}

impl TrayLook {
    pub fn of(s: &TrayState) -> Self {
        if s.recording {
            TrayLook::Recording
        } else if s.failed {
            TrayLook::Error
        } else {
            TrayLook::Idle
        }
    }
}

/// Distância de `p` ao segmento `a`–`b`.
fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0. {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0., 1.)
    } else {
        0.
    };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

/// Ícone `n`×`n` RGBA desenhado por código no espaço 16×16 do mockup, com borda suave.
/// `light_bar` = barra de tarefas clara (anel preto). Puro.
pub fn tray_rgba(look: TrayLook, light_bar: bool, n: u32) -> Vec<u8> {
    let ink: [f32; 3] = if light_bar {
        [0x1a as f32, 0x1a as f32, 0x1a as f32]
    } else {
        [255., 255., 255.]
    };
    let red: [f32; 3] = if light_bar {
        [0xe5 as f32, 0x32 as f32, 0x2b as f32]
    } else {
        [0xff as f32, 0x45 as f32, 0x3a as f32]
    };
    let white = [255f32, 255., 255.];
    let k = n as f32 / 16.0;
    // Cobertura de uma forma com distância com sinal `sd` (negativo = dentro), em px.
    let cov = |sd: f32| (0.5 - sd * k).clamp(0.0, 1.0);
    let mut out = vec![0u8; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            let p = ((x as f32 + 0.5) / k, (y as f32 + 0.5) / k);
            let d = ((p.0 - 8.).powi(2) + (p.1 - 8.).powi(2)).sqrt();
            // Camadas de baixo para cima: (cor, alfa).
            let mut layers: Vec<([f32; 3], f32)> = Vec::with_capacity(4);
            let mut ring = cov((d - 5.6).abs() - 0.8);
            let badge_d = ((p.0 - 12.6).powi(2) + (p.1 - 3.6).powi(2)).sqrt();
            if look == TrayLook::Error {
                // Folga em volta do selo para ele não grudar no anel.
                ring *= 1.0 - cov(badge_d - 3.8);
            }
            layers.push((ink, ring));
            match look {
                TrayLook::Recording => layers.push((red, cov(d - 2.6))),
                TrayLook::Idle | TrayLook::Error => layers.push((ink, cov(d - 1.6))),
            }
            if look == TrayLook::Error {
                layers.push((red, cov(badge_d - 3.0)));
                // "!" branco: traço e ponto.
                let bar = seg_dist(p, (12.6, 2.2), (12.6, 3.9)) - 0.55;
                let dot = ((p.0 - 12.6).powi(2) + (p.1 - 5.05).powi(2)).sqrt() - 0.55;
                layers.push((white, cov(bar.min(dot)) * cov(badge_d - 3.0)));
            }
            // Composição "over" (não pré-multiplicada no fim).
            let (mut c, mut a) = ([0f32; 3], 0f32);
            for (col, al) in layers {
                let na = al + a * (1.0 - al);
                if na > 0.0 {
                    for i in 0..3 {
                        c[i] = (col[i] * al + c[i] * a * (1.0 - al)) / na;
                    }
                }
                a = na;
            }
            let i = ((y * n + x) * 4) as usize;
            out[i] = c[0].round() as u8;
            out[i + 1] = c[1].round() as u8;
            out[i + 2] = c[2].round() as u8;
            out[i + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

/// Ícone no tamanho do DPI do sistema e na cor da barra de tarefas.
fn icon(look: TrayLook) -> Option<Icon> {
    let n = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }.clamp(16, 64) as u32;
    let light = crate::appearance::taskbar_light();
    Icon::from_rgba(tray_rgba(look, light, n), n, n).ok()
}

/// Itens do menu (mesmos do menu antigo).
fn model(s: &TrayState) -> MenuModel {
    let item = |label: &str, enabled: bool, checked: Option<bool>, command, sep: bool| MenuEntry {
        label: label.to_string(),
        enabled,
        checked,
        command,
        separator_before: sep,
    };
    MenuModel {
        status: s.status.clone(),
        recording: s.recording,
        entries: vec![
            item(&s.save_label, s.can_save, None, TrayCommand::SaveClip, true),
            item(
                &s.kill_label,
                s.can_save,
                None,
                TrayCommand::SaveKill,
                false,
            ),
            item(
                &s.manual_label,
                true,
                None,
                TrayCommand::ToggleManual,
                false,
            ),
            item(
                &s.auto_detect_label,
                true,
                Some(s.auto_detect),
                TrayCommand::ToggleAutoDetect,
                false,
            ),
            item(&s.library_label, true, None, TrayCommand::OpenLibrary, true),
            item(&s.folder_label, true, None, TrayCommand::OpenFolder, false),
            item(
                &s.start_with_os_label,
                true,
                Some(s.start_with_os),
                TrayCommand::ToggleStartWithOs,
                false,
            ),
            item(&s.quit_label, true, None, TrayCommand::Quit, true),
        ],
    }
}

fn run(
    state: Arc<Mutex<TrayState>>,
    commands: Arc<dyn Fn(TrayCommand) + Send + Sync>,
    ready: &std::sync::mpsc::Sender<std::result::Result<u32, String>>,
) -> std::result::Result<(), String> {
    // Menu e ícone nítidos em 125/150%: só esta thread fica ciente do DPI por monitor.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let thread_id = unsafe { GetCurrentThreadId() };
    let s = state.lock().clone();
    let mut builder = TrayIconBuilder::new()
        .with_menu_on_left_click(false)
        .with_tooltip(&s.tooltip);
    let mut look = TrayLook::of(&s);
    let mut light = crate::appearance::taskbar_light();
    if let Some(i) = icon(look) {
        builder = builder.with_icon(i);
    }
    let tray = builder.build().map_err(|e| e.to_string())?;

    let menu = {
        let tx = commands.clone();
        match TrayMenu::new(move |cmd| tx(cmd)) {
            Ok(m) => Some(m),
            Err(e) => {
                tracing::warn!("menu de vidro da bandeja indisponível ({e}); usando o nativo");
                None
            }
        }
    };
    if let Some(m) = &menu {
        m.set_model(model(&s));
    }

    {
        let tx = commands.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| match e {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => tx(TrayCommand::OpenLibrary),
            TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Up,
                ..
            } => unsafe {
                let _ = PostThreadMessageW(thread_id, WM_SHOW_MENU, WPARAM(0), LPARAM(0));
            },
            _ => {}
        }));
    }

    let _ = ready.send(Ok(thread_id));
    let mut shown = s;
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.0.is_null() && msg.message == WM_APP {
                let new = state.lock().clone();
                let (new_look, new_light) =
                    (TrayLook::of(&new), crate::appearance::taskbar_light());
                if (new_look, new_light) != (look, light) {
                    let _ = tray.set_icon(icon(new_look));
                    (look, light) = (new_look, new_light);
                }
                if new.tooltip != shown.tooltip {
                    let _ = tray.set_tooltip(Some(&new.tooltip));
                }
                if let Some(m) = &menu {
                    m.set_model(model(&new));
                }
                shown = new;
                continue;
            }
            if msg.hwnd.0.is_null() && msg.message == WM_SHOW_MENU {
                let mut at = POINT::default();
                let _ = GetCursorPos(&mut at);
                // A barra pode ter mudado de tema desde a última vez.
                let now_light = crate::appearance::taskbar_light();
                if now_light != light {
                    let _ = tray.set_icon(icon(look));
                    light = now_light;
                }
                match &menu {
                    Some(m) => m.show(at),
                    None => {
                        let owner =
                            windows::Win32::Foundation::HWND(tray.window_handle() as *mut _);
                        if let Some(cmd) = native_menu(owner, &model(&shown), at) {
                            commands(cmd);
                        }
                    }
                }
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    drop(menu);
    drop(tray);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{tray_rgba, TrayLook};

    fn px(img: &[u8], n: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * n + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    #[test]
    fn icone_da_bandeja() {
        for n in [16, 20, 24, 32] {
            let idle = tray_rgba(TrayLook::Idle, false, n);
            assert_eq!(idle.len(), (n * n * 4) as usize);
            // Centro: ponto branco parado, vermelho gravando.
            let c = n / 2;
            assert!(px(&idle, n, c, c)[3] > 200);
            assert_eq!(px(&idle, n, c, c)[0], 255);
            let rec = tray_rgba(TrayLook::Recording, false, n);
            let p = px(&rec, n, c, c);
            assert!(p[0] > 200 && p[1] < 120);
            // Canto de baixo à esquerda: vazio.
            assert_eq!(px(&idle, n, 0, n - 1)[3], 0);
            // Selo de erro no canto de cima à direita.
            let err = tray_rgba(TrayLook::Error, true, n);
            let b = px(
                &err,
                n,
                (12.6 * n as f32 / 16.) as u32,
                (2.0 * n as f32 / 16.) as u32,
            );
            assert!(b[3] > 200);
        }
        // Barra clara: anel preto.
        let light = tray_rgba(TrayLook::Idle, true, 32);
        assert!(px(&light, 32, 16, 16)[0] < 40);
    }
}
