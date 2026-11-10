//! DXGI Desktop Duplication: captura do monitor sem a borda amarela que a
//! Windows Graphics Capture desenha no Windows 10 (lá não dá para tirá-la).
//! Também fica no compositor, sem injetar nada no jogo.
//!
//! A duplicação é sempre do monitor inteiro; para gravar uma janela, cada
//! quadro é recortado no retângulo dela. Uma thread própria espera os quadros
//! e os copia para o mesmo slot da WGC ([`CaptureShared`]), então o encoder
//! não sabe qual API está por trás.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_BOX};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication, IDXGIResource,
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTPUT_DESC,
};
use windows::Win32::Graphics::Gdi::{
    MonitorFromWindow, RedrawWindow, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
    RDW_ALLCHILDREN, RDW_FRAME, RDW_INVALIDATE,
};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow};

use crate::capture::{store_frame, CaptureShared, CaptureTarget};
use crate::gpu::Gpu;
use crate::{Error, Result};

/// Espera máxima por um quadro novo antes de voltar a olhar `stop`.
const ACQUIRE_TIMEOUT_MS: u32 = 100;
/// Pausa entre tentativas de recriar a duplicação (tela segura do UAC,
/// troca de modo de vídeo...).
const RETRY: Duration = Duration::from_millis(250);
/// De quanto em quanto tempo conferir se a janela mudou de monitor.
const MONITOR_CHECK: Duration = Duration::from_millis(500);
/// Sem quadro nenhum depois disso, pede um redesenho da tela.
const NUDGE_AFTER: Duration = Duration::from_millis(300);

pub struct DxgiCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl DxgiCapture {
    /// Começa a duplicar o monitor de `target`. Só volta `Ok` depois que a
    /// duplicação foi criada, para que quem chama possa cair na WGC se falhar.
    pub fn start(
        gpu: &Gpu,
        target: CaptureTarget,
        fps: u32,
        shared: Arc<CaptureShared>,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let thread = {
            let (gpu, stop, target) = (gpu.clone(), stop.clone(), SendTarget(target));
            std::thread::Builder::new()
                .name("capture-dxgi".into())
                .spawn(move || run(gpu, target.get(), fps, shared, stop, tx))
                .map_err(|e| Error::Other(format!("thread da captura: {e}")))?
        };
        match rx.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(Error::Other(
                    "a thread da captura DXGI terminou sem responder".into(),
                ))
            }
        }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for DxgiCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `HWND` é só um identificador de janela: vale em qualquer thread.
struct SendTarget(CaptureTarget);
// SAFETY: o HWND não é desreferenciado, só passado de volta às APIs do Windows.
unsafe impl Send for SendTarget {}

impl SendTarget {
    fn get(self) -> CaptureTarget {
        self.0
    }
}

struct Duplication {
    dupl: IDXGIOutputDuplication,
    /// Retângulo do monitor no desktop virtual (pixels físicos).
    desktop: RECT,
}

fn monitor_of(target: CaptureTarget) -> HMONITOR {
    unsafe {
        match target {
            CaptureTarget::PrimaryMonitor => {
                MonitorFromWindow(HWND::default(), MONITOR_DEFAULTTOPRIMARY)
            }
            CaptureTarget::MonitorOf(h) => MonitorFromWindow(h, MONITOR_DEFAULTTOPRIMARY),
            CaptureTarget::Window(h) => MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST),
        }
    }
}

/// Cria a duplicação da saída de vídeo que mostra `mon`. A saída precisa
/// estar na mesma GPU do dispositivo da gravação.
fn duplicate(gpu: &Gpu, mon: HMONITOR) -> Result<Duplication> {
    let adapter = unsafe { gpu.device.cast::<IDXGIDevice>()?.GetAdapter()? };
    let mut found: Option<(IDXGIOutput1, DXGI_OUTPUT_DESC)> = None;
    for i in 0.. {
        let Ok(out) = (unsafe { adapter.EnumOutputs(i) }) else {
            break;
        };
        let desc = unsafe { out.GetDesc()? };
        if desc.Monitor == mon {
            found = Some((out.cast()?, desc));
            break;
        }
    }
    let Some((out, desc)) = found else {
        return Err(Error::Other(format!(
            "o monitor não está ligado à GPU da gravação ({})",
            gpu.adapter_name
        )));
    };
    if desc.Rotation != DXGI_MODE_ROTATION_IDENTITY
        && desc.Rotation != DXGI_MODE_ROTATION_UNSPECIFIED
    {
        return Err(Error::Other(
            "monitor girado não suportado pela captura DXGI".into(),
        ));
    }
    // DuplicateOutput1 converte HDR/FP16 para BGRA; se não der (versão do
    // Windows, DPI), a versão antiga serve para desktop SDR.
    let dupl = match out
        .cast::<IDXGIOutput5>()
        .and_then(|o| unsafe { o.DuplicateOutput1(&gpu.device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]) })
    {
        Ok(d) => d,
        Err(e) => {
            tracing::debug!("DuplicateOutput1 indisponível ({e}), usando DuplicateOutput");
            unsafe { out.DuplicateOutput(&gpu.device)? }
        }
    };
    Ok(Duplication {
        dupl,
        desktop: desc.DesktopCoordinates,
    })
}

/// Parte do quadro do monitor a copiar. `None` = nada visível (janela
/// minimizada ou fora deste monitor).
fn region(target: CaptureTarget, desktop: &RECT) -> Option<D3D11_BOX> {
    let (mw, mh) = (desktop.right - desktop.left, desktop.bottom - desktop.top);
    let CaptureTarget::Window(h) = target else {
        return Some(D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: mw as u32,
            bottom: mh as u32,
            back: 1,
        });
    };
    if unsafe { IsIconic(h) }.as_bool() {
        return None;
    }
    // Moldura visível da janela, em pixels físicos (o DWM não ajusta por DPI).
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
    }
    .ok()?;
    let left = (r.left - desktop.left).clamp(0, mw);
    let top = (r.top - desktop.top).clamp(0, mh);
    let right = (r.right - desktop.left).clamp(0, mw);
    let bottom = (r.bottom - desktop.top).clamp(0, mh);
    (right > left && bottom > top).then_some(D3D11_BOX {
        left: left as u32,
        top: top as u32,
        front: 0,
        right: right as u32,
        bottom: bottom as u32,
        back: 1,
    })
}

/// Pede ao Windows para redesenhar a janela (ou todas, na captura do
/// monitor), só para a duplicação receber o primeiro quadro.
fn redraw(target: CaptureTarget) {
    let hwnd = match target {
        CaptureTarget::Window(h) => Some(h),
        _ => None,
    };
    tracing::debug!("tela parada: pedindo redesenho para o 1º quadro DXGI");
    unsafe {
        let _ = RedrawWindow(
            hwnd,
            None,
            None,
            RDW_INVALIDATE | RDW_FRAME | RDW_ALLCHILDREN,
        );
    }
}

fn run(
    gpu: Gpu,
    target: CaptureTarget,
    fps: u32,
    shared: Arc<CaptureShared>,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<()>>,
) {
    // Coordenadas e duplicação em pixels físicos, mesmo com escala de 125/150%.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut mon = monitor_of(target);
    let mut dup = match duplicate(&gpu, mon) {
        Ok(d) => {
            let r = &d.desktop;
            tracing::info!(
                width = r.right - r.left,
                height = r.bottom - r.top,
                "captura iniciada (DXGI Desktop Duplication)"
            );
            let _ = ready.send(Ok(()));
            Some(d)
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    drop(ready);

    // Monitores de 144/240 Hz entregam mais quadros que o encoder usa:
    // copiar no máximo ~2x o fps basta para o slot estar sempre fresco.
    let min_gap = Duration::from_micros(500_000 / u64::from(fps.max(1)));
    let mut last_copy: Option<Instant> = None;
    let mut last_check = Instant::now();
    let mut retry_logged = false;
    // Desde quando a duplicação atual existe e se já pedimos um redesenho.
    let mut since = Instant::now();
    let mut nudged = false;

    while !stop.load(Ordering::Relaxed) {
        if let CaptureTarget::Window(h) = target {
            if !unsafe { IsWindow(Some(h)) }.as_bool() {
                tracing::warn!("fonte da captura foi fechada");
                shared.closed.store(true, Ordering::SeqCst);
                break;
            }
            if last_check.elapsed() >= MONITOR_CHECK {
                last_check = Instant::now();
                let now = monitor_of(target);
                if now != mon {
                    tracing::info!("a janela mudou de monitor");
                    mon = now;
                    dup = None;
                }
            }
        }

        let Some(d) = dup.as_ref() else {
            match duplicate(&gpu, mon) {
                Ok(n) => {
                    tracing::info!("captura DXGI recriada");
                    retry_logged = false;
                    last_copy = None;
                    since = Instant::now();
                    nudged = false;
                    dup = Some(n);
                }
                Err(e) => {
                    if !retry_logged {
                        tracing::warn!("recriando captura DXGI: {e}");
                        retry_logged = true;
                    }
                    std::thread::sleep(RETRY);
                }
            }
            continue;
        };

        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res: Option<IDXGIResource> = None;
        match unsafe {
            d.dupl
                .AcquireNextFrame(ACQUIRE_TIMEOUT_MS, &mut info, &mut res)
        } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                // Sem mudança na tela não chega nem o 1º quadro (a WGC manda
                // um na hora). Um redesenho, uma vez, faz o DWM apresentar.
                if last_copy.is_none() && !nudged && since.elapsed() >= NUDGE_AFTER {
                    nudged = true;
                    redraw(target);
                }
                continue;
            }
            Err(e) => {
                // ACCESS_LOST: troca de resolução, tela cheia exclusiva, UAC.
                if e.code() != DXGI_ERROR_ACCESS_LOST {
                    tracing::warn!("erro ao esperar quadro DXGI: {e}");
                }
                dup = None;
                continue;
            }
        }

        // O 1º quadro de uma duplicação nova sempre vale (a tela pode estar
        // parada); depois, LastPresentTime = 0 quer dizer que só o cursor mexeu.
        let fresh = match last_copy {
            None => true,
            Some(t) => info.LastPresentTime != 0 && t.elapsed() >= min_gap,
        };
        if fresh {
            let copied = (|| -> Result<bool> {
                let Some(b) = region(target, &d.desktop) else {
                    return Ok(false);
                };
                let src: ID3D11Texture2D = res
                    .as_ref()
                    .ok_or_else(|| Error::Other("quadro DXGI sem textura".into()))?
                    .cast()?;
                store_frame(&gpu, &shared, &src, &b)?;
                Ok(true)
            })();
            match copied {
                Ok(true) => last_copy = Some(Instant::now()),
                Ok(false) => {}
                Err(e) => tracing::warn!("erro ao processar frame: {e}"),
            }
        }
        drop(res);
        if let Err(e) = unsafe { d.dupl.ReleaseFrame() } {
            if e.code() != DXGI_ERROR_ACCESS_LOST {
                tracing::warn!("ReleaseFrame: {e}");
            }
            dup = None;
        }
    }
}
