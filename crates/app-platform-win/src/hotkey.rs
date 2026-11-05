//! Hotkeys globais via `RegisterHotKey` (funcionam com o jogo em foco/tela
//! cheia, sem hooks de teclado).

use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, PostThreadMessageW, MSG, WM_HOTKEY, WM_QUIT,
};

use crate::{Error, Result};

/// Hotkey já interpretada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
}

/// Interpreta "F8", "Alt+C", "Ctrl+Shift+S", "Win+F10"...
pub fn parse(spec: &str) -> Option<Hotkey> {
    let mut modifiers = 0u32;
    let mut vk = None;
    for part in spec.split('+').map(|p| p.trim().to_ascii_uppercase()) {
        match part.as_str() {
            "CTRL" | "CONTROL" => modifiers |= MOD_CONTROL.0,
            "ALT" => modifiers |= MOD_ALT.0,
            "SHIFT" => modifiers |= MOD_SHIFT.0,
            "WIN" | "SUPER" => modifiers |= MOD_WIN.0,
            key => vk = Some(key_to_vk(key)?),
        }
    }
    vk.map(|vk| Hotkey { modifiers, vk })
}

fn key_to_vk(key: &str) -> Option<u32> {
    if let Some(n) = key.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1); // VK_F1 = 0x70
        }
    }
    let b = key.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_uppercase() || b[0].is_ascii_digit()) {
        return Some(u32::from(b[0]));
    }
    Some(match key {
        "INSERT" | "INS" => 0x2D,
        "HOME" => 0x24,
        "END" => 0x23,
        "PAGEUP" | "PGUP" => 0x21,
        "PAGEDOWN" | "PGDN" => 0x22,
        "PAUSE" => 0x13,
        "SCROLLLOCK" => 0x91,
        "PRINTSCREEN" | "PRTSC" => 0x2C,
        _ => return None,
    })
}

const BUSY_MSG: &str = "não foi possível registrar a hotkey (outro app já a usa? ex.: Medal)";

/// Resultado do registro de uma hotkey: `Err` com o motivo (em geral, já usada por outro app).
pub type KeyStatus = std::result::Result<(), String>;

/// Thread que registra as hotkeys e envia o índice da que foi apertada.
pub struct HotkeyListener {
    thread_id: u32,
    handle: Option<JoinHandle<()>>,
}

impl HotkeyListener {
    /// Registra as hotkeys (na ordem dada), cada uma de forma independente: uma ocupada não
    /// impede as outras. Devolve o resultado de cada uma, na mesma ordem. Eventos chegam pelo
    /// `Receiver` com o índice da hotkey.
    pub fn start(keys: Vec<Hotkey>) -> Result<(Self, Receiver<usize>, Vec<KeyStatus>)> {
        let (tx, rx): (Sender<usize>, Receiver<usize>) = std::sync::mpsc::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<(u32, Vec<KeyStatus>)>();
        let handle = std::thread::Builder::new()
            .name("hotkeys".into())
            .spawn(move || unsafe {
                let tid = GetCurrentThreadId();
                let status: Vec<KeyStatus> = keys
                    .iter()
                    .enumerate()
                    .map(|(i, k)| {
                        let mods = HOT_KEY_MODIFIERS(k.modifiers | MOD_NOREPEAT.0);
                        RegisterHotKey(None, i as i32 + 1, mods, k.vk)
                            .map_err(|e| format!("{BUSY_MSG}: {e}"))
                    })
                    .collect();
                let _ = ready_tx.send((tid, status.clone()));
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    if msg.message == WM_HOTKEY {
                        let id = msg.wParam.0;
                        if id >= 1 {
                            let _ = tx.send(id - 1);
                        }
                    }
                }
                for (i, st) in status.iter().enumerate() {
                    if st.is_ok() {
                        let _ = UnregisterHotKey(None, i as i32 + 1);
                    }
                }
            })
            .map_err(|e| Error::Other(e.to_string()))?;
        match ready_rx.recv() {
            Ok((thread_id, status)) => Ok((
                Self {
                    thread_id,
                    handle: Some(handle),
                },
                rx,
                status,
            )),
            Err(_) => Err(Error::Other("thread de hotkeys morreu".into())),
        }
    }
}

impl Drop for HotkeyListener {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_hotkeys() {
        assert_eq!(
            parse("F8"),
            Some(Hotkey {
                modifiers: 0,
                vk: 0x77
            })
        );
        assert_eq!(
            parse("alt+c"),
            Some(Hotkey {
                modifiers: MOD_ALT.0,
                vk: u32::from(b'C')
            })
        );
        assert_eq!(
            parse("Ctrl+Shift+S").unwrap().modifiers,
            MOD_CONTROL.0 | MOD_SHIFT.0
        );
        assert_eq!(parse("Ctrl+"), None);
        assert_eq!(parse("F25"), None);
    }
}
