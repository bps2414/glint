//! Marca d'água da exportação: uma imagem RGBA (feita pela interface) composta nos quadros
//! YUV 4:2:0 (NV12 ou YUV420P) que vão para o encoder, num canto, com ~4% da altura do vídeo.
//!
//! A imagem é escalada e convertida para Y/U/V pré-multiplicados uma vez por exportação
//! ([`Overlay::new`]); por quadro só o retângulo da marca é tocado, direto no quadro que
//! vai para o encoder (sem cópia).

use std::fmt;
use std::sync::Arc;

pub use app_core::Corner;
use ffmpeg_sys_next as ff;

/// Altura da marca em relação à altura do vídeo.
const HEIGHT_FRAC: f32 = 0.04;
/// Margem até a borda, em relação à altura do vídeo.
const MARGIN_FRAC: f32 = 0.025;

/// Imagem da marca e o canto onde ela vai.
#[derive(Clone, PartialEq, Eq)]
pub struct Watermark {
    /// RGBA pré-multiplicado (como `tiny_skia::Pixmap`), `width * height * 4` bytes. Opacidade
    /// e sombra já vêm na imagem.
    pub rgba: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub corner: Corner,
}

impl fmt::Debug for Watermark {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Watermark")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("corner", &self.corner)
            .finish_non_exhaustive()
    }
}

/// Posição e tamanho da marca num vídeo `fw`×`fh` (tudo par, por causa do 4:2:0): altura de
/// [`HEIGHT_FRAC`] do vídeo (menor se não couber na largura), a [`MARGIN_FRAC`] da borda.
/// `None` = o vídeo é pequeno demais.
pub fn layout(
    corner: Corner,
    fw: u32,
    fh: u32,
    mw: u32,
    mh: u32,
) -> Option<(usize, usize, usize, usize)> {
    let even = |v: f32| (v.round().max(0.0) as usize) & !1;
    let (fw, fh) = (fw as usize & !1, fh as usize & !1);
    if mw == 0 || mh == 0 {
        return None;
    }
    let aspect = mw as f32 / mh as f32;
    let margin = even(fh as f32 * MARGIN_FRAC).max(2);
    let mut h = even(fh as f32 * HEIGHT_FRAC);
    let mut w = even(h as f32 * aspect);
    let max_w = fw.checked_sub(2 * margin)?;
    if w > max_w {
        w = max_w & !1;
        h = even(w as f32 / aspect);
    }
    if w < 2 || h < 2 || h + 2 * margin > fh {
        return None;
    }
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => margin,
        Corner::TopRight | Corner::BottomRight => fw - margin - w,
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => margin,
        Corner::BottomLeft | Corner::BottomRight => fh - margin - h,
    };
    Some((x, y, w, h))
}

/// Planos de chroma de um quadro 4:2:0.
pub(crate) enum Chroma<'a> {
    /// U e V intercalados (NV12).
    Nv12(&'a mut [u8], usize),
    /// U e V separados (YUV420P), cada um com seu passo.
    Planar(&'a mut [u8], usize, &'a mut [u8], usize),
}

/// A marca pronta para um tamanho de vídeo: valores pré-multiplicados e quanto do fundo fica
/// (`keep`, 0..=256), por pixel de luma e por bloco 2×2 de chroma.
pub(crate) struct Overlay {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    y_p: Vec<u8>,
    y_keep: Vec<u16>,
    u_p: Vec<u8>,
    v_p: Vec<u8>,
    c_keep: Vec<u16>,
}

impl Overlay {
    pub fn new(mark: &Watermark, fw: u32, fh: u32) -> Option<Overlay> {
        let (mw, mh) = (mark.width as usize, mark.height as usize);
        if mark.rgba.len() < mw * mh * 4 {
            return None;
        }
        let (x, y, w, h) = layout(mark.corner, fw, fh, mark.width, mark.height)?;
        let px = resize(&mark.rgba, mw, mh, w, h);

        // BT.709, faixa limitada (o que o quadro do encoder declara).
        const KR: f32 = 0.2126;
        const KB: f32 = 0.0722;
        const KG: f32 = 1.0 - KR - KB;
        let mut y_p = vec![0u8; w * h];
        let mut y_keep = vec![0u16; w * h];
        // U, V e alfa pré-multiplicados, somados por bloco 2×2.
        let (cw, ch) = (w / 2, h / 2);
        let mut acc = vec![[0f32; 3]; cw * ch];
        for row in 0..h {
            for col in 0..w {
                let i = row * w + col;
                let [r, g, b, a] = px[i];
                let luma = KR * r + KG * g + KB * b;
                y_p[i] = (16.0 * a + 219.0 * luma).round().clamp(0.0, 255.0) as u8;
                y_keep[i] = keep(a);
                let c = &mut acc[(row / 2) * cw + col / 2];
                c[0] += 128.0 * a + 224.0 * (b - luma) / 1.8556;
                c[1] += 128.0 * a + 224.0 * (r - luma) / 1.5748;
                c[2] += a;
            }
        }
        let byte = |v: f32| (v / 4.0).round().clamp(0.0, 255.0) as u8;
        Some(Overlay {
            x,
            y,
            w,
            h,
            y_p,
            y_keep,
            u_p: acc.iter().map(|c| byte(c[0])).collect(),
            v_p: acc.iter().map(|c| byte(c[1])).collect(),
            c_keep: acc.iter().map(|c| keep(c[2] / 4.0)).collect(),
        })
    }

    /// Compõe a marca nos planos de um quadro (que precisa ter o tamanho dado a [`Overlay::new`]).
    pub fn blend(&self, luma: &mut [u8], luma_stride: usize, chroma: Chroma<'_>) {
        let w = self.w;
        for row in 0..self.h {
            let d = &mut luma[(self.y + row) * luma_stride + self.x..][..w];
            let p = &self.y_p[row * w..][..w];
            let k = &self.y_keep[row * w..][..w];
            for ((d, &p), &k) in d.iter_mut().zip(p).zip(k) {
                *d = mix(*d, p, k);
            }
        }
        let (cx, cy, cw) = (self.x / 2, self.y / 2, w / 2);
        match chroma {
            Chroma::Nv12(uv, s) => {
                for row in 0..self.h / 2 {
                    let d = &mut uv[(cy + row) * s + 2 * cx..][..2 * cw];
                    for i in 0..cw {
                        let j = row * cw + i;
                        d[2 * i] = mix(d[2 * i], self.u_p[j], self.c_keep[j]);
                        d[2 * i + 1] = mix(d[2 * i + 1], self.v_p[j], self.c_keep[j]);
                    }
                }
            }
            Chroma::Planar(u, us, v, vs) => {
                for row in 0..self.h / 2 {
                    let du = &mut u[(cy + row) * us + cx..][..cw];
                    let dv = &mut v[(cy + row) * vs + cx..][..cw];
                    for i in 0..cw {
                        let j = row * cw + i;
                        du[i] = mix(du[i], self.u_p[j], self.c_keep[j]);
                        dv[i] = mix(dv[i], self.v_p[j], self.c_keep[j]);
                    }
                }
            }
        }
    }

    /// Compõe a marca num quadro NV12 ou YUV420P do encoder (outros formatos ficam sem marca).
    ///
    /// # Safety
    /// `frame` precisa ser um quadro de vídeo válido, gravável, com os planos alocados.
    pub unsafe fn apply(&self, frame: *mut ff::AVFrame) {
        // SAFETY: garantido por quem chama.
        let f = unsafe { &*frame };
        let (fw, fh, ls) = (
            f.width.max(0) as usize,
            f.height.max(0) as usize,
            f.linesize,
        );
        if self.x + self.w > fw || self.y + self.h > fh || ls.iter().take(3).any(|&l| l < 0) {
            return;
        }
        // SAFETY: cada plano tem `linesize * linhas` bytes (4:2:0: metade das linhas no chroma).
        let plane = |i: usize, rows: usize| unsafe {
            std::slice::from_raw_parts_mut(f.data[i], ls[i] as usize * rows)
        };
        let luma = plane(0, fh);
        if f.format == ff::AVPixelFormat::AV_PIX_FMT_NV12 as i32 {
            self.blend(
                luma,
                ls[0] as usize,
                Chroma::Nv12(plane(1, fh / 2), ls[1] as usize),
            );
        } else if f.format == ff::AVPixelFormat::AV_PIX_FMT_YUV420P as i32 {
            let (u, v) = (plane(1, fh / 2), plane(2, fh / 2));
            self.blend(
                luma,
                ls[0] as usize,
                Chroma::Planar(u, ls[1] as usize, v, ls[2] as usize),
            );
        }
    }
}

/// Quanto do fundo fica para um alfa `a` (0..1), em 1/256.
fn keep(a: f32) -> u16 {
    ((1.0 - a.clamp(0.0, 1.0)) * 256.0).round() as u16
}

/// `fundo * keep / 256 + marca` (a marca já vem pré-multiplicada).
fn mix(dst: u8, p: u8, keep: u16) -> u8 {
    let v = ((u32::from(dst) * u32::from(keep) + 128) >> 8) + u32::from(p);
    v.min(255) as u8
}

/// Escala RGBA pré-multiplicado (u8) para `dw`×`dh` por média de área (separável). Sai em 0..1.
fn resize(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<[f32; 4]> {
    let px = |i: usize| {
        let p = &src[i * 4..i * 4 + 4];
        [p[0], p[1], p[2], p[3]].map(|v| f32::from(v) / 255.0)
    };
    let wx = weights(sw, dw);
    let wy = weights(sh, dh);
    // Horizontal: sh linhas de dw.
    let mut mid = vec![[0f32; 4]; dw * sh];
    for row in 0..sh {
        for (x, ws) in wx.iter().enumerate() {
            let o = &mut mid[row * dw + x];
            for &(sx, k) in ws {
                for (o, p) in o.iter_mut().zip(px(row * sw + sx)) {
                    *o += p * k;
                }
            }
        }
    }
    // Vertical.
    let mut out = vec![[0f32; 4]; dw * dh];
    for (y, ws) in wy.iter().enumerate() {
        for x in 0..dw {
            let o = &mut out[y * dw + x];
            for &(sy, k) in ws {
                for (o, p) in o.iter_mut().zip(mid[sy * dw + x]) {
                    *o += p * k;
                }
            }
        }
    }
    out
}

/// Pesos da média de área de `n_src` amostras para `n_dst` (cada saída soma 1).
fn weights(n_src: usize, n_dst: usize) -> Vec<Vec<(usize, f32)>> {
    let s = n_src as f32 / n_dst as f32;
    (0..n_dst)
        .map(|i| {
            let (a, b) = (i as f32 * s, (i + 1) as f32 * s);
            let mut ws = Vec::new();
            let mut j = a.floor() as usize;
            while (j as f32) < b && j < n_src {
                let overlap = (b.min(j as f32 + 1.0) - a.max(j as f32)).max(0.0);
                if overlap > 0.0 {
                    ws.push((j, overlap / s));
                }
                j += 1;
            }
            ws
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(rgba: [u8; 4], w: u32, h: u32, corner: Corner) -> Watermark {
        Watermark {
            rgba: rgba.repeat((w * h) as usize).into(),
            width: w,
            height: h,
            corner,
        }
    }

    #[test]
    fn posicao_nos_cantos() {
        // 1080p, marca 4:1: 42 px de altura (par), margem 26 px.
        let at = |c| layout(c, 1920, 1080, 400, 100).unwrap();
        assert_eq!(
            at(Corner::BottomRight),
            (1920 - 26 - 168, 1080 - 26 - 42, 168, 42)
        );
        assert_eq!(at(Corner::TopLeft), (26, 26, 168, 42));
        assert_eq!(at(Corner::TopRight), (1920 - 26 - 168, 26, 168, 42));
        assert_eq!(at(Corner::BottomLeft), (26, 1080 - 26 - 42, 168, 42));
        // Larga demais para o vídeo: encolhe para caber entre as margens.
        let (x, _, w, h) = layout(Corner::TopLeft, 100, 1000, 1000, 100).unwrap();
        assert_eq!((x, w, h), (24, 52, 4));
        // Vídeo minúsculo: sem marca.
        assert_eq!(layout(Corner::TopLeft, 16, 16, 4, 1), None);
    }

    #[test]
    fn compoe_so_no_retangulo() {
        let (fw, fh) = (200usize, 100usize);
        // Branco opaco 10×4 → ocupa 10×4 px a 2 px da borda, no canto inferior direito.
        let o = Overlay::new(&mark([255; 4], 10, 4, Corner::BottomRight), 200, 100).unwrap();
        assert_eq!((o.x, o.y, o.w, o.h), (188, 94, 10, 4));
        let mut y = vec![16u8; fw * fh];
        let mut uv = vec![60u8; fw * fh / 2];
        o.blend(&mut y, fw, Chroma::Nv12(&mut uv, fw));
        assert_eq!(y[95 * fw + 190], 235);
        assert_eq!(y[50 * fw + 100], 16);
        assert_eq!(y[95 * fw + 187], 16);
        assert_eq!(uv[47 * fw + 2 * 95], 128);
        assert_eq!(uv[47 * fw + 2 * 95 + 1], 128);
        assert_eq!(uv[10 * fw + 20], 60);

        // Meio transparente (pré-multiplicado) no YUV420P: fica entre o fundo e o branco.
        let o = Overlay::new(&mark([128; 4], 10, 4, Corner::TopLeft), 200, 100).unwrap();
        let mut y = vec![16u8; fw * fh];
        let (mut u, mut v) = (vec![60u8; fw * fh / 4], vec![60u8; fw * fh / 4]);
        o.blend(&mut y, fw, Chroma::Planar(&mut u, fw / 2, &mut v, fw / 2));
        assert!((120..=130).contains(&y[3 * fw + 3]), "{}", y[3 * fw + 3]);
        assert!((90..=97).contains(&u[fw / 2 + 1]), "{}", u[fw / 2 + 1]);
        assert_eq!(y[3 * fw + 20], 16);
    }
}
