use windows::core::Interface;
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;

use crate::Result;

/// Dispositivo D3D11 usado por todo o pipeline (captura → conversão → encode),
/// para que os frames nunca saiam da GPU.
#[derive(Clone)]
pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    /// Mesmo dispositivo, visto pela API WinRT (exigido pela WGC).
    pub winrt: IDirect3DDevice,
    pub adapter_name: String,
}

// SAFETY: o dispositivo é criado com proteção multithread (ID3D10Multithread),
// então chamadas concorrentes no contexto imediato são serializadas pelo driver.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

impl Gpu {
    pub fn new() -> Result<Self> {
        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        let device: ID3D11Device = device.expect("D3D11CreateDevice devolveu device");
        let context: ID3D11DeviceContext = context.expect("D3D11CreateDevice devolveu contexto");

        let mt: ID3D10Multithread = context.cast()?;
        unsafe {
            let _ = mt.SetMultithreadProtected(true);
        }

        let dxgi: IDXGIDevice = device.cast()?;
        let adapter_name = unsafe {
            dxgi.GetAdapter()
                .and_then(|a| a.GetDesc())
                .map(|d| {
                    let len = d
                        .Description
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(d.Description.len());
                    String::from_utf16_lossy(&d.Description[..len])
                })
                .unwrap_or_else(|_| "desconhecido".into())
        };
        let winrt: IDirect3DDevice =
            unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
        tracing::info!(gpu = %adapter_name, "dispositivo D3D11 criado");
        Ok(Self {
            device,
            context,
            winrt,
            adapter_name,
        })
    }

    /// O driver reiniciou/a GPU sumiu (TDR, atualização de driver): todo o
    /// pipeline precisa ser recriado com um dispositivo novo.
    pub fn is_lost(&self) -> bool {
        unsafe { self.device.GetDeviceRemovedReason().is_err() }
    }

    /// Ponteiro COM cru do `ID3D11Device` (para o FFmpeg).
    pub fn device_ptr(&self) -> *mut std::ffi::c_void {
        self.device.as_raw()
    }

    /// `AddRef` num ponteiro de `ID3D11Device` (usado ao entregar o device ao FFmpeg).
    ///
    /// # Safety
    /// `ptr` deve ser um `ID3D11Device*` válido.
    pub unsafe fn add_ref(ptr: *mut std::ffi::c_void) {
        if let Some(d) = ID3D11Device::from_raw_borrowed(&ptr) {
            // clone = AddRef; forget = não dar Release.
            std::mem::forget(d.clone());
        }
    }
}
