//! "Jogo" sintético para medir o custo do gravador sem um jogo de verdade: janela sem
//! borda cobrindo o monitor, D3D11 com swap chain flip, **sem limite de FPS** (Present sem
//! vsync, com tearing quando suportado) e um pixel shader pesado ajustável.
//!
//! Uso: `carga-gpu [iteracoes] [segundos]` (padrão 48 iterações; 0 s = até apertar Esc).
//! Ao sair imprime `fps_proprio=<n>` (contagem do próprio app, para conferir o ETW).

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    win::run()
}

#[cfg(windows)]
mod win {
    use std::time::{Duration, Instant};

    use windows::core::{s, w, Interface, Result, BOOL, PCSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
    use windows::Win32::Graphics::Direct3D::{
        ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_DRIVER_TYPE_HARDWARE,
    };
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::*;
    use windows::Win32::Graphics::Dxgi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, KEYEVENTF_KEYUP, VK_ESCAPE, VK_MENU,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    const SHADER: &str = r#"
float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}
cbuffer C : register(b0) { float t; float iters; float2 res; };
float4 ps(float4 p : SV_Position) : SV_Target {
    float2 uv = p.xy / res;
    float2 z = uv * 2 - 1;
    float3 c = 0;
    for (int i = 0; i < (int)iters; i++) {
        z = float2(sin(z.x * 1.3 + t), cos(z.y * 1.7 - t)) + uv;
        c += 0.5 + 0.5 * cos(float3(0, 2, 4) + z.xyx + t);
    }
    return float4(c / iters, 1);
}
"#;

    unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
                let _ = DestroyWindow(h);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(h, msg, wp, lp),
        }
    }

    fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
        let mut blob = None;
        let mut err = None;
        let r = unsafe {
            D3DCompile(
                SHADER.as_ptr() as *const _,
                SHADER.len(),
                None,
                None,
                None,
                entry,
                target,
                0,
                0,
                &mut blob,
                Some(&mut err),
            )
        };
        if let Err(e) = r {
            if let Some(err) = err {
                let msg = unsafe {
                    std::slice::from_raw_parts(
                        err.GetBufferPointer() as *const u8,
                        err.GetBufferSize(),
                    )
                };
                eprintln!("{}", String::from_utf8_lossy(msg));
            }
            return Err(e);
        }
        Ok(blob.expect("D3DCompile devolveu blob"))
    }

    fn bytes(b: &ID3DBlob) -> &[u8] {
        unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
    }

    pub fn run() -> Result<()> {
        let mut args = std::env::args().skip(1);
        let iters: f32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(48.0);
        let secs: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);

        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let inst = GetModuleHandleW(None)?;
            let class = w!("CargaGpu");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: inst.into(),
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
            let hwnd = CreateWindowExW(
                WS_EX_APPWINDOW,
                class,
                w!("Carga GPU"),
                WS_POPUP | WS_VISIBLE,
                0,
                0,
                sw,
                sh,
                None,
                None,
                Some(inst.into()),
                None,
            )?;
            // Truque do Alt: deixa a janela tomar o foco mesmo aberta por um processo em segundo plano.
            keybd_event(VK_MENU.0 as u8, 0, Default::default(), 0);
            keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
            let _ = SetForegroundWindow(hwnd);

            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device: ID3D11Device = device.expect("device");
            let context: ID3D11DeviceContext = context.expect("context");

            let factory: IDXGIFactory2 = CreateDXGIFactory1()?;
            let mut tearing = BOOL(0);
            if let Ok(f5) = factory.cast::<IDXGIFactory5>() {
                let _ = f5.CheckFeatureSupport(
                    DXGI_FEATURE_PRESENT_ALLOW_TEARING,
                    &mut tearing as *mut _ as *mut _,
                    std::mem::size_of::<BOOL>() as u32,
                );
            }
            let tearing = tearing.as_bool();
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: sw as u32,
                Height: sh as u32,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                Flags: if tearing {
                    DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 as u32
                } else {
                    0
                },
                ..Default::default()
            };
            let swap = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)?;
            let back: ID3D11Texture2D = swap.GetBuffer(0)?;
            let mut rtv = None;
            device.CreateRenderTargetView(&back, None, Some(&mut rtv))?;
            let rtv = rtv.expect("rtv");

            let vsb = compile(s!("vs"), s!("vs_5_0"))?;
            let psb = compile(s!("ps"), s!("ps_5_0"))?;
            let mut vs = None;
            device.CreateVertexShader(bytes(&vsb), None, Some(&mut vs))?;
            let mut ps = None;
            device.CreatePixelShader(bytes(&psb), None, Some(&mut ps))?;
            let cb_desc = D3D11_BUFFER_DESC {
                ByteWidth: 16,
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                ..Default::default()
            };
            let mut cb = None;
            device.CreateBuffer(&cb_desc, None, Some(&mut cb))?;
            let cb = cb.expect("cb");

            let vp = D3D11_VIEWPORT {
                Width: sw as f32,
                Height: sh as f32,
                MaxDepth: 1.0,
                ..Default::default()
            };
            let start = Instant::now();
            let mut frames: u64 = 0;
            let mut msg = MSG::default();
            'main: loop {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        break 'main;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                if secs > 0 && start.elapsed() >= Duration::from_secs(secs) {
                    break;
                }
                let consts = [start.elapsed().as_secs_f32(), iters, sw as f32, sh as f32];
                context.UpdateSubresource(&cb, 0, None, consts.as_ptr() as *const _, 0, 0);
                context.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
                context.RSSetViewports(Some(&[vp]));
                context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                context.VSSetShader(vs.as_ref(), None);
                context.PSSetShader(ps.as_ref(), None);
                context.PSSetConstantBuffers(0, Some(&[Some(cb.clone())]));
                context.Draw(3, 0);
                let flags = if tearing {
                    DXGI_PRESENT_ALLOW_TEARING
                } else {
                    DXGI_PRESENT(0)
                };
                swap.Present(0, flags).ok()?;
                frames += 1;
            }
            println!(
                "fps_proprio={:.1}",
                frames as f64 / start.elapsed().as_secs_f64()
            );
        }
        Ok(())
    }
}
