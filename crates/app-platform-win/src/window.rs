//! Utilitários de janelas: listar, achar por título e pegar o PID.

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible,
};

#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub hwnd: HWND,
    pub title: String,
    pub pid: u32,
    pub width: i32,
    pub height: i32,
}

// SAFETY: HWND é só um identificador.
unsafe impl Send for WindowInfo {}

fn info(hwnd: HWND) -> Option<WindowInfo> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        Some(WindowInfo {
            hwnd,
            title,
            pid,
            width: r.right - r.left,
            height: r.bottom - r.top,
        })
    }
}

/// Janelas visíveis com título.
pub fn list() -> Vec<WindowInfo> {
    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam.0 as *mut Vec<WindowInfo>);
        if let Some(i) = info(hwnd) {
            out.push(i);
        }
        true.into()
    }
    let mut out: Vec<WindowInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// Primeira janela cujo título contém `needle` (sem diferenciar maiúsculas).
pub fn find(needle: &str) -> Option<WindowInfo> {
    let needle = needle.to_lowercase();
    list()
        .into_iter()
        .find(|w| w.title.to_lowercase().contains(&needle))
}

pub fn foreground() -> Option<WindowInfo> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        None
    } else {
        info(hwnd)
    }
}
