//! Captura de tela/janela.
//!
//! Duas APIs, ambas no nível do compositor (DWM), que **não injetam nada no
//! processo do jogo** (seguro para o Vanguard):
//! - Windows Graphics Capture (WGC): padrão onde dá para tirar a borda
//!   amarela (Windows 11).
//! - DXGI Desktop Duplication ([`crate::capture_dxgi`]): no Windows 10, onde a
//!   WGC sempre desenha a borda.
//!
//! Cada frame chega como textura D3D11 e é copiado (GPU→GPU) para um "slot"
//! com o frame mais recente, que o loop de encode consome no seu próprio
//! ritmo (CFR).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use app_core::CaptureBackend;
use parking_lot::Mutex;
use windows::core::{IInspectable, Interface, HSTRING};
use windows::Foundation::Metadata::ApiInformation;
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, HMONITOR, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use crate::capture_dxgi::DxgiCapture;
use crate::gpu::Gpu;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug)]
pub enum CaptureTarget {
    /// Monitor principal (funciona com qualquer jogo em tela cheia/sem borda).
    PrimaryMonitor,
    /// Monitor onde está a janela.
    MonitorOf(HWND),
    /// Uma janela específica.
    Window(HWND),
}

/// Frame mais recente, compartilhado entre a captura e o encoder.
pub struct LatestFrame {
    pub texture: ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
pub struct CaptureShared {
    pub latest: Mutex<Option<LatestFrame>>,
    /// Incrementa a cada frame novo.
    pub frames: AtomicU64,
    /// A fonte foi fechada (janela fechou etc.).
    pub closed: AtomicBool,
}

pub struct Capture {
    backend: Backend,
    shared: Arc<CaptureShared>,
}

enum Backend {
    Wgc {
        session: GraphicsCaptureSession,
        pool: Direct3D11CaptureFramePool,
    },
    Dxgi(DxgiCapture),
}

const PIXEL_FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;
const BUFFERS: i32 = 2;

fn create_item(target: CaptureTarget) -> Result<GraphicsCaptureItem> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    unsafe {
        let item = match target {
            CaptureTarget::PrimaryMonitor => {
                let mon: HMONITOR = MonitorFromWindow(HWND::default(), MONITOR_DEFAULTTOPRIMARY);
                interop.CreateForMonitor(mon)?
            }
            CaptureTarget::MonitorOf(hwnd) => {
                let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
                interop.CreateForMonitor(mon)?
            }
            CaptureTarget::Window(hwnd) => interop.CreateForWindow(hwnd)?,
        };
        Ok(item)
    }
}

/// A WGC deixa tirar a borda amarela (`IsBorderRequired`, build 20348+:
/// Windows 11). No Windows 10 a propriedade não existe e a borda fica.
fn wgc_border_removable() -> bool {
    ApiInformation::IsPropertyPresent(
        &HSTRING::from("Windows.Graphics.Capture.GraphicsCaptureSession"),
        &HSTRING::from("IsBorderRequired"),
    )
    .unwrap_or(false)
}

impl Capture {
    pub fn is_supported() -> bool {
        GraphicsCaptureSession::IsSupported().unwrap_or(false)
    }

    pub fn start(
        gpu: &Gpu,
        target: CaptureTarget,
        fps: u32,
        backend: CaptureBackend,
    ) -> Result<Self> {
        let dxgi = match backend {
            CaptureBackend::Dxgi => true,
            CaptureBackend::Wgc => false,
            CaptureBackend::Auto => !wgc_border_removable(),
        };
        if dxgi {
            let shared = Arc::new(CaptureShared::default());
            match DxgiCapture::start(gpu, target, fps, shared.clone()) {
                Ok(c) => {
                    return Ok(Self {
                        backend: Backend::Dxgi(c),
                        shared,
                    })
                }
                Err(e) if backend == CaptureBackend::Auto => {
                    tracing::warn!("DXGI Desktop Duplication falhou, usando WGC (com borda): {e}");
                }
                Err(e) => return Err(e),
            }
        }
        Self::start_wgc(gpu, target, fps)
    }

    fn start_wgc(gpu: &Gpu, target: CaptureTarget, fps: u32) -> Result<Self> {
        if !Self::is_supported() {
            return Err(Error::Other(
                "Windows Graphics Capture não suportado (requer Windows 10 1903+)".into(),
            ));
        }
        let item = create_item(target)?;
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &gpu.winrt,
            PIXEL_FORMAT,
            BUFFERS,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;

        // Sem borda amarela (Windows 11) e sem cursor; falhas aqui não são fatais.
        if let Err(e) = session.SetIsBorderRequired(false) {
            tracing::debug!("SetIsBorderRequired indisponível: {e}");
        }
        if let Err(e) = session.SetIsCursorCaptureEnabled(false) {
            tracing::debug!("SetIsCursorCaptureEnabled indisponível: {e}");
        }
        // Windows 11 24H2+: limitar a taxa de frames entregue ao fps do encoder.
        let interval = TimeSpan {
            Duration: 10_000_000 / i64::from(fps.max(1)),
        };
        if let Err(e) = session.SetMinUpdateInterval(interval) {
            tracing::debug!("SetMinUpdateInterval indisponível: {e}");
        }

        let shared = Arc::new(CaptureShared::default());
        {
            let shared = shared.clone();
            item.Closed(
                &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                    tracing::warn!("fonte da captura foi fechada");
                    shared.closed.store(true, Ordering::SeqCst);
                    Ok(())
                }),
            )?;
        }
        {
            let shared = shared.clone();
            let gpu = gpu.clone();
            let pool_ref = pool.clone();
            let last_size = Mutex::new(size);
            pool.FrameArrived(
                &TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
                    if let Err(e) = on_frame(&gpu, &pool_ref, &shared, &last_size) {
                        tracing::warn!("erro ao processar frame: {e}");
                    }
                    Ok(())
                }),
            )?;
        }

        session.StartCapture()?;
        tracing::info!(
            width = size.Width,
            height = size.Height,
            "captura iniciada (Windows Graphics Capture)"
        );
        Ok(Self {
            backend: Backend::Wgc { session, pool },
            shared,
        })
    }

    pub fn shared(&self) -> &Arc<CaptureShared> {
        &self.shared
    }

    pub fn stop(&mut self) {
        match &mut self.backend {
            Backend::Wgc { session, pool } => {
                let _ = session.Close();
                let _ = pool.Close();
            }
            Backend::Dxgi(c) => c.stop(),
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn on_frame(
    gpu: &Gpu,
    pool: &Direct3D11CaptureFramePool,
    shared: &CaptureShared,
    last_size: &Mutex<SizeInt32>,
) -> Result<()> {
    let frame = pool.TryGetNextFrame()?;
    let content = frame.ContentSize()?;
    {
        let mut last = last_size.lock();
        if content.Width != last.Width || content.Height != last.Height {
            tracing::info!(
                width = content.Width,
                height = content.Height,
                "tamanho da fonte mudou"
            );
            pool.Recreate(&gpu.winrt, PIXEL_FORMAT, BUFFERS, content)?;
            *last = content;
        }
    }
    if content.Width <= 0 || content.Height <= 0 {
        return Ok(());
    }
    let surface = frame.Surface()?;
    let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
    let src: ID3D11Texture2D = unsafe { access.GetInterface()? };

    let mut src_desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { src.GetDesc(&mut src_desc) };
    let region = D3D11_BOX {
        left: 0,
        top: 0,
        front: 0,
        right: (content.Width as u32).min(src_desc.Width),
        bottom: (content.Height as u32).min(src_desc.Height),
        back: 1,
    };
    store_frame(gpu, shared, &src, &region)
}

/// Copia `region` de `src` para o slot do frame mais recente (recriando o
/// slot se o tamanho mudou).
pub(crate) fn store_frame(
    gpu: &Gpu,
    shared: &CaptureShared,
    src: &ID3D11Texture2D,
    region: &D3D11_BOX,
) -> Result<()> {
    let w = region.right - region.left;
    let h = region.bottom - region.top;
    if w == 0 || h == 0 {
        return Ok(());
    }
    let mut latest = shared.latest.lock();
    let needs_new = match latest.as_ref() {
        Some(l) => l.width != w || l.height != h,
        None => true,
    };
    if needs_new {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut tex = None;
        unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut tex))? };
        *latest = Some(LatestFrame {
            texture: tex.expect("CreateTexture2D devolveu textura"),
            width: w,
            height: h,
        });
    }
    let dst = latest.as_ref().expect("slot criado acima");
    unsafe {
        gpu.context
            .CopySubresourceRegion(&dst.texture, 0, 0, 0, 0, src, 0, Some(region));
    }
    drop(latest);
    shared.frames.fetch_add(1, Ordering::Release);
    Ok(())
}
