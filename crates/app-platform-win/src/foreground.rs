//! Observação da janela em primeiro plano para a detecção de jogo.
//!
//! Tudo aqui é leitura de informação pública do sistema (janela em foco,
//! retângulo, nome do executável, contadores de desempenho da GPU). **Nada lê
//! memória do jogo nem injeta código** — seguro para anti-cheats como o Vanguard.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, RECT, WAIT_TIMEOUT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
    PDH_MORE_DATA,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindow,
};

/// Janela em primeiro plano (sem a medição de GPU, que é feita à parte).
#[derive(Clone, Debug)]
pub struct Foreground {
    pub hwnd: HWND,
    pub pid: u32,
    /// Só o nome do arquivo (`VALORANT-Win64-Shipping.exe`).
    pub exe: String,
    pub title: String,
    /// Cobre o monitor inteiro (tela cheia exclusiva ou sem borda).
    pub fullscreen: bool,
}

// SAFETY: HWND é só um identificador.
unsafe impl Send for Foreground {}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Caminho completo do executável do processo (precisa só de acesso "limitado").
pub fn process_image(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let r =
            QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

fn exe_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

fn covers_monitor(hwnd: HWND) -> bool {
    unsafe {
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return false;
        }
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return false;
        }
        let m = mi.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

/// Lê a janela em primeiro plano. `None` se não há janela, é o próprio app,
/// é a área de trabalho ou está minimizada.
pub fn sample() -> Option<Foreground> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() || IsIconic(hwnd).as_bool() {
            return None;
        }
        let class = class_name(hwnd);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == GetCurrentProcessId() {
            return None;
        }
        let exe = process_image(pid).map(|p| exe_name(&p)).unwrap_or_default();
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        Some(Foreground {
            hwnd,
            pid,
            exe,
            title,
            fullscreen: covers_monitor(hwnd),
        })
    }
}

/// A janela ainda existe e ainda é do processo `pid` (plano B quando não dá
/// para abrir o processo, ex.: protegido por anti-cheat).
pub fn window_alive(hwnd: HWND, pid: u32) -> bool {
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        let mut owner = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut owner));
        owner == pid
    }
}

/// Acompanha se um processo ainda está vivo (handle só com `SYNCHRONIZE`).
pub struct ProcessWatch {
    handle: HANDLE,
}

unsafe impl Send for ProcessWatch {}

impl ProcessWatch {
    pub fn open(pid: u32) -> Option<Self> {
        unsafe {
            OpenProcess(PROCESS_SYNCHRONIZE, false, pid)
                .ok()
                .map(|handle| Self { handle })
        }
    }

    pub fn alive(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle, 0) == WAIT_TIMEOUT }
    }
}

impl Drop for ProcessWatch {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Uso do motor 3D da GPU por um processo, pelos contadores de desempenho
/// "GPU Engine" do Windows (os mesmos do Gerenciador de Tarefas).
///
/// Os contadores são de taxa: a primeira leitura não tem valor; a partir da
/// segunda, cada [`GpuWatch::utilization`] devolve a média desde a anterior.
pub struct GpuWatch {
    pid: u32,
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}

unsafe impl Send for GpuWatch {}

impl GpuWatch {
    pub fn new(pid: u32) -> Option<Self> {
        unsafe {
            let mut query = PDH_HQUERY::default();
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                return None;
            }
            let path = format!("\\GPU Engine(pid_{pid}_*engtype_3D)\\Utilization Percentage");
            let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
            let mut counter = PDH_HCOUNTER::default();
            if PdhAddEnglishCounterW(query, PCWSTR(wide.as_ptr()), 0, &mut counter) != 0 {
                let _ = PdhCloseQuery(query);
                return None;
            }
            let _ = PdhCollectQueryData(query);
            Some(Self {
                pid,
                query,
                counter,
            })
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Porcentagem de uso do 3D (soma das instâncias do processo).
    pub fn utilization(&mut self) -> Option<f64> {
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let mut size = 0u32;
            let mut count = 0u32;
            let r = PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                None,
            );
            if r != PDH_MORE_DATA || size == 0 {
                // Sem instâncias (processo ainda não usou a GPU).
                return Some(0.0);
            }
            let items = (size as usize)
                .div_ceil(std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>())
                .max(1);
            let mut buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> = vec![Default::default(); items];
            let r = PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                Some(buf.as_mut_ptr()),
            );
            if r != 0 {
                return None;
            }
            let total: f64 = buf[..(count as usize).min(items)]
                .iter()
                .filter(|i| i.FmtValue.CStatus == 0)
                .map(|i| i.FmtValue.Anonymous.doubleValue)
                .sum();
            Some(total)
        }
    }
}

impl Drop for GpuWatch {
    fn drop(&mut self) {
        unsafe {
            let _ = PdhCloseQuery(self.query);
        }
    }
}
