//! FPS de outro processo sem tocar nele: escuta os eventos `Present` do provedor
//! ETW `Microsoft-Windows-DXGI` (a mesma fonte do PresentMon). Nada de injeção nem
//! leitura de memória; precisa de administrador (sessão ETW).
//!
//! Conta chamadas de `IDXGISwapChain::Present` (D3D10/11/12). Jogos em Vulkan/OpenGL
//! puros não aparecem.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Diagnostics::Etw::*;
use windows::Win32::System::Performance::QueryPerformanceFrequency;

use crate::{Error, Result};

/// Microsoft-Windows-DXGI.
const DXGI_PROVIDER: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
/// Evento `Present_Start` do provedor DXGI.
const PRESENT_START: u16 = 42;
const SESSION: &str = "AppFpsMeter";

static TARGET_PID: AtomicU32 = AtomicU32::new(0);
static STAMPS: Mutex<Vec<i64>> = Mutex::new(Vec::new());

/// Resultado de uma medição.
#[derive(Debug, Clone)]
pub struct FpsReport {
    pub frames: usize,
    pub secs: f64,
    pub avg_fps: f64,
    /// FPS do 1% de quadros mais lentos (média dos tempos acima do percentil 99).
    pub low1_fps: f64,
    /// Pior tempo de quadro em ms.
    pub worst_ms: f64,
}

unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
    let h = &(*rec).EventHeader;
    if h.EventDescriptor.Id == PRESENT_START
        && h.ProviderId == DXGI_PROVIDER
        && h.ProcessId == TARGET_PID.load(Ordering::Relaxed)
    {
        STAMPS.lock().push(h.TimeStamp);
    }
}

/// Buffer de `EVENT_TRACE_PROPERTIES` + espaço para o nome da sessão.
fn properties() -> Vec<u8> {
    let name_bytes = (SESSION.len() + 1) * 2;
    let size = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + name_bytes;
    let mut buf = vec![0u8; size];
    let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
    unsafe {
        (*p).Wnode.BufferSize = size as u32;
        (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        (*p).Wnode.ClientContext = 1; // TimeStamp em QPC
        (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        (*p).LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    }
    buf
}

fn check(e: WIN32_ERROR, what: &str) -> Result<()> {
    if e == ERROR_SUCCESS {
        Ok(())
    } else if e == ERROR_ACCESS_DENIED {
        Err(Error::Other(format!(
            "{what}: acesso negado (medir FPS precisa rodar como administrador)"
        )))
    } else {
        Err(Error::Other(format!("{what}: erro {}", e.0)))
    }
}

/// Mede o FPS do processo `pid` por `dur`. Bloqueia durante a medição.
pub fn measure(pid: u32, dur: Duration) -> Result<FpsReport> {
    let name: Vec<u16> = SESSION.encode_utf16().chain(Some(0)).collect();
    TARGET_PID.store(pid, Ordering::Relaxed);
    STAMPS.lock().clear();

    unsafe {
        // Sessão que sobrou de uma execução interrompida.
        let mut props = properties();
        let _ = ControlTraceW(
            CONTROLTRACE_HANDLE::default(),
            PCWSTR(name.as_ptr()),
            props.as_mut_ptr() as *mut _,
            EVENT_TRACE_CONTROL_STOP,
        );

        let mut props = properties();
        let mut handle = CONTROLTRACE_HANDLE::default();
        check(
            StartTraceW(
                &mut handle,
                PCWSTR(name.as_ptr()),
                props.as_mut_ptr() as *mut _,
            ),
            "StartTrace",
        )?;
        let stop = |props: &mut Vec<u8>| {
            let _ = ControlTraceW(
                handle,
                PCWSTR(name.as_ptr()),
                props.as_mut_ptr() as *mut _,
                EVENT_TRACE_CONTROL_STOP,
            );
        };

        let r = EnableTraceEx2(
            handle,
            &DXGI_PROVIDER,
            EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
            TRACE_LEVEL_VERBOSE as u8,
            u64::MAX,
            0,
            0,
            None,
        );
        if let Err(e) = check(r, "EnableTraceEx2") {
            stop(&mut props);
            return Err(e);
        }

        let mut logfile = EVENT_TRACE_LOGFILEW {
            LoggerName: PWSTR(name.as_ptr() as *mut u16),
            ..Default::default()
        };
        logfile.Anonymous1.ProcessTraceMode =
            PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(on_event);
        let trace = OpenTraceW(&mut logfile);
        if trace.Value == u64::MAX {
            stop(&mut props);
            return Err(Error::Other("OpenTrace falhou".into()));
        }

        // ProcessTrace bloqueia até a sessão parar.
        let worker = std::thread::spawn(move || {
            let _ = ProcessTrace(&[trace], None, None);
        });
        std::thread::sleep(dur);
        stop(&mut props);
        let _ = worker.join();
        let _ = CloseTrace(trace);
    }

    let mut freq = 0i64;
    unsafe {
        let _ = QueryPerformanceFrequency(&mut freq);
    }
    let stamps = std::mem::take(&mut *STAMPS.lock());
    Ok(summarize(&stamps, freq as f64, dur.as_secs_f64()))
}

/// Estatísticas a partir dos instantes dos Presents (em ticks de `freq` por s).
fn summarize(stamps: &[i64], freq: f64, secs: f64) -> FpsReport {
    let mut ms: Vec<f64> = stamps
        .windows(2)
        .map(|w| (w[1] - w[0]) as f64 * 1000.0 / freq)
        .collect();
    let span = ms.iter().sum::<f64>() / 1000.0;
    let avg_fps = if span > 0.0 {
        ms.len() as f64 / span
    } else {
        0.0
    };
    ms.sort_by(|a, b| b.total_cmp(a));
    let n = (ms.len() / 100).max(1).min(ms.len());
    let worst = &ms[..n];
    let low1 = if worst.is_empty() {
        0.0
    } else {
        1000.0 / (worst.iter().sum::<f64>() / worst.len() as f64)
    };
    FpsReport {
        frames: stamps.len(),
        secs,
        avg_fps,
        low1_fps: low1,
        worst_ms: ms.first().copied().unwrap_or(0.0),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn resumo() {
        // 100 quadros a 10 ms + 1 de 50 ms, relógio de 1000 ticks/s.
        let mut t = vec![0i64];
        for i in 1..=100 {
            t.push(i * 10);
        }
        t.push(1050);
        let r = super::summarize(&t, 1000.0, 1.05);
        assert_eq!(r.frames, 102);
        assert!((r.avg_fps - 101.0 / 1.05).abs() < 0.01);
        assert!((r.low1_fps - 20.0).abs() < 0.01);
        assert!((r.worst_ms - 50.0).abs() < 0.01);
        let vazio = super::summarize(&[], 1000.0, 1.0);
        assert_eq!(vazio.frames, 0);
        assert_eq!(vazio.avg_fps, 0.0);
    }
}
