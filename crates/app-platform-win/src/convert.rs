//! Conversão de cor e escala na GPU: BGRA (captura) → NV12 (encoder),
//! via D3D11 VideoProcessor (hardware de vídeo dedicado, não usa shaders do jogo).

use std::mem::ManuallyDrop;

use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_VIDEO_COLOR_YCbCrA, ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoContext1,
    ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
    ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView, D3D11_BIND_RENDER_TARGET,
    D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
    DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};

use crate::gpu::Gpu;
use crate::Result;

pub struct Converter {
    gpu: Gpu,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    in_w: u32,
    in_h: u32,
    pub out_w: u32,
    pub out_h: u32,
    fps: u32,
    output: ID3D11Texture2D,
    output_view: ID3D11VideoProcessorOutputView,
    /// Cache da input view (a textura de entrada só muda quando a fonte muda de tamanho).
    input_cache: Option<(usize, ID3D11VideoProcessorInputView)>,
}

unsafe impl Send for Converter {}

impl Converter {
    pub fn new(gpu: &Gpu, in_w: u32, in_h: u32, out_w: u32, out_h: u32, fps: u32) -> Result<Self> {
        let video_device: ID3D11VideoDevice = gpu.device.cast()?;
        let video_context: ID3D11VideoContext = gpu.context.cast()?;
        let (enumerator, processor) =
            create_processor(&video_device, &video_context, in_w, in_h, out_w, out_h, fps)?;

        let desc = D3D11_TEXTURE2D_DESC {
            Width: out_w,
            Height: out_h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut output = None;
        unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut output))? };
        let output: ID3D11Texture2D = output.expect("textura NV12");
        let output_view = create_output_view(&video_device, &enumerator, &output)?;

        tracing::info!(in_w, in_h, out_w, out_h, "conversor BGRA→NV12 criado");
        Ok(Self {
            gpu: gpu.clone(),
            video_device,
            video_context,
            enumerator,
            processor,
            in_w,
            in_h,
            out_w,
            out_h,
            fps,
            output,
            output_view,
            input_cache: None,
        })
    }

    /// Textura NV12 de saída (tamanho fixo `out_w`×`out_h`).
    pub fn output(&self) -> &ID3D11Texture2D {
        &self.output
    }

    /// Converte `input` (BGRA, `w`×`h`) para a textura NV12 de saída,
    /// escalando se necessário. Se a fonte mudar de tamanho, o processador é
    /// recriado e a saída mantém o tamanho (o encoder não precisa reiniciar).
    pub fn convert(&mut self, input: &ID3D11Texture2D, w: u32, h: u32) -> Result<()> {
        if w != self.in_w || h != self.in_h {
            let (e, p) = create_processor(
                &self.video_device,
                &self.video_context,
                w,
                h,
                self.out_w,
                self.out_h,
                self.fps,
            )?;
            self.enumerator = e;
            self.processor = p;
            self.output_view =
                create_output_view(&self.video_device, &self.enumerator, &self.output)?;
            self.input_cache = None;
            self.in_w = w;
            self.in_h = h;
        }

        let key = input.as_raw() as usize;
        let view = match &self.input_cache {
            Some((k, v)) if *k == key => v.clone(),
            _ => {
                let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                    FourCC: 0,
                    ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                        Texture2D: D3D11_TEX2D_VPIV {
                            MipSlice: 0,
                            ArraySlice: 0,
                        },
                    },
                };
                let mut v = None;
                unsafe {
                    self.video_device.CreateVideoProcessorInputView(
                        input,
                        &self.enumerator,
                        &desc,
                        Some(&mut v),
                    )?
                };
                let v = v.expect("input view");
                self.input_cache = Some((key, v.clone()));
                v
            }
        };

        let stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            OutputIndex: 0,
            InputFrameOrField: 0,
            PastFrames: 0,
            FutureFrames: 0,
            ppPastSurfaces: std::ptr::null_mut(),
            pInputSurface: ManuallyDrop::new(Some(view)),
            ppFutureSurfaces: std::ptr::null_mut(),
            ppPastSurfacesRight: std::ptr::null_mut(),
            pInputSurfaceRight: ManuallyDrop::new(None),
            ppFutureSurfacesRight: std::ptr::null_mut(),
        };
        let mut streams = [stream];
        let r = unsafe {
            self.video_context
                .VideoProcessorBlt(&self.processor, &self.output_view, 0, &streams)
        };
        // Libera a referência guardada no ManuallyDrop.
        unsafe { ManuallyDrop::drop(&mut streams[0].pInputSurface) };
        r?;
        let _ = &self.gpu;
        Ok(())
    }
}

fn create_processor(
    vd: &ID3D11VideoDevice,
    vc: &ID3D11VideoContext,
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    fps: u32,
) -> Result<(ID3D11VideoProcessorEnumerator, ID3D11VideoProcessor)> {
    let rate = DXGI_RATIONAL {
        Numerator: fps,
        Denominator: 1,
    };
    let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
        InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        InputFrameRate: rate,
        InputWidth: in_w,
        InputHeight: in_h,
        OutputFrameRate: rate,
        OutputWidth: out_w,
        OutputHeight: out_h,
        Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
    };
    unsafe {
        let enumerator = vd.CreateVideoProcessorEnumerator(&content)?;
        let processor = vd.CreateVideoProcessor(&enumerator, 0)?;
        let src = RECT {
            left: 0,
            top: 0,
            right: in_w as i32,
            bottom: in_h as i32,
        };
        let full = RECT {
            left: 0,
            top: 0,
            right: out_w as i32,
            bottom: out_h as i32,
        };
        // Se a fonte mudar de proporção no meio (jogo trocou de resolução),
        // a imagem vai centralizada com barras pretas em vez de esticar.
        let dst = letterbox(in_w, in_h, out_w, out_h);
        vc.VideoProcessorSetStreamFrameFormat(&processor, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
        vc.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&src));
        vc.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dst));
        vc.VideoProcessorSetOutputTargetRect(&processor, true, Some(&full));
        // Fundo preto em YCbCr (Y=16, Cb=Cr=128 em 8 bits).
        let black = D3D11_VIDEO_COLOR {
            Anonymous: D3D11_VIDEO_COLOR_0 {
                YCbCr: D3D11_VIDEO_COLOR_YCbCrA {
                    Y: 16.0 / 255.0,
                    Cb: 0.5,
                    Cr: 0.5,
                    A: 1.0,
                },
            },
        };
        vc.VideoProcessorSetOutputBackgroundColor(&processor, true, &black);
        // Sem "melhorias" automáticas do driver (nitidez, ruído etc.).
        vc.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
        if let Ok(vc1) = vc.cast::<ID3D11VideoContext1>() {
            vc1.VideoProcessorSetStreamColorSpace1(
                &processor,
                0,
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            );
            vc1.VideoProcessorSetOutputColorSpace1(
                &processor,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
            );
        }
        Ok((enumerator, processor))
    }
}

/// Retângulo de destino que cabe `in` em `out` mantendo a proporção.
fn letterbox(in_w: u32, in_h: u32, out_w: u32, out_h: u32) -> RECT {
    let (iw, ih, ow, oh) = (
        u64::from(in_w.max(1)),
        u64::from(in_h.max(1)),
        u64::from(out_w),
        u64::from(out_h),
    );
    let (w, h) = if iw * oh > ow * ih {
        (ow, (ow * ih / iw).min(oh))
    } else {
        ((oh * iw / ih).min(ow), oh)
    };
    let (w, h) = (w as i32 & !1, h as i32 & !1);
    let left = ((out_w as i32 - w) / 2) & !1;
    let top = ((out_h as i32 - h) / 2) & !1;
    RECT {
        left,
        top,
        right: left + w,
        bottom: top + h,
    }
}

fn create_output_view(
    vd: &ID3D11VideoDevice,
    enumerator: &ID3D11VideoProcessorEnumerator,
    output: &ID3D11Texture2D,
) -> Result<ID3D11VideoProcessorOutputView> {
    let desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
        ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
            Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
        },
    };
    let mut view = None;
    unsafe { vd.CreateVideoProcessorOutputView(output, enumerator, &desc, Some(&mut view))? };
    Ok(view.expect("output view"))
}
