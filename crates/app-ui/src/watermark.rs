//! Imagem da marca d'água: o wordmark (símbolo + "glint", `docs/brand/wordmark-escuro.svg`)
//! com sombra leve e 70% de opacidade, feita uma vez. O `app_media::export_mp4` escala para
//! ~4% da altura do vídeo e compõe no canto escolhido.

use std::sync::{Arc, OnceLock};

use app_core::Corner;
use app_media::Watermark;
use resvg::tiny_skia::{Pixmap, PixmapPaint, PremultipliedColorU8, Transform};
use resvg::usvg;

const WORDMARK: &str = include_str!("../../../docs/brand/wordmark-escuro.svg");
/// Altura do wordmark na imagem: sobra para 4% de um vídeo 2160p sem aumentar.
const MARK_H: f32 = 96.;
const OPACITY: f32 = 0.7;
/// Sombra: preta, deslocada para baixo e desfocada (frações da altura do wordmark).
const SHADOW_ALPHA: f32 = 0.7;
const SHADOW_DY: f32 = 0.02;
const SHADOW_BLUR: f32 = 0.03;

/// A marca para o canto `corner` (`None` só se o SVG embutido não abrir).
pub fn watermark(corner: Corner) -> Option<Watermark> {
    static IMAGE: OnceLock<Option<Watermark>> = OnceLock::new();
    let mark = IMAGE.get_or_init(render).clone()?;
    Some(Watermark { corner, ..mark })
}

fn render() -> Option<Watermark> {
    let tree = usvg::Tree::from_str(WORDMARK, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let k = MARK_H / size.height();
    // Folga em volta para a sombra não ser cortada.
    let pad = (MARK_H * (SHADOW_DY + 2. * SHADOW_BLUR)).ceil();
    let w = (size.width() * k + 2. * pad).ceil() as u32;
    let h = (MARK_H + 2. * pad).ceil() as u32;
    let mut mark = Pixmap::new(w, h)?;
    resvg::render(
        &tree,
        Transform::from_scale(k, k).post_translate(pad, pad),
        &mut mark.as_mut(),
    );

    // Sombra: o alfa da marca, desfocado e deslocado, em preto.
    let alpha: Vec<f32> = mark.pixels().iter().map(|p| p.alpha() as f32).collect();
    let r = (MARK_H * SHADOW_BLUR).round() as usize;
    let blurred = blur(
        &blur(&alpha, w as usize, h as usize, r),
        w as usize,
        h as usize,
        r,
    );
    let dy = (MARK_H * SHADOW_DY).round() as usize;
    let mut out = Pixmap::new(w, h)?;
    for (i, p) in out.pixels_mut().iter_mut().enumerate() {
        let (x, y) = (i % w as usize, i / w as usize);
        if y >= dy {
            let a = (blurred[(y - dy) * w as usize + x] * SHADOW_ALPHA).round() as u8;
            *p = PremultipliedColorU8::from_rgba(0, 0, 0, a)?;
        }
    }
    out.draw_pixmap(
        0,
        0,
        mark.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );

    // Opacidade (pré-multiplicado: escala os quatro canais).
    let rgba: Vec<u8> = out
        .data()
        .iter()
        .map(|&v| (f32::from(v) * OPACITY).round() as u8)
        .collect();
    Some(Watermark {
        rgba: Arc::from(rgba),
        width: w,
        height: h,
        corner: Corner::default(),
    })
}

/// Média móvel separável de raio `r` (horizontal e vertical).
fn blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    if r == 0 {
        return src.to_vec();
    }
    let n = (2 * r + 1) as f32;
    let mut mid = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            mid[y * w + x] = src[y * w + a..=y * w + b].iter().sum::<f32>() / n;
        }
    }
    let mut out = vec![0f32; w * h];
    for x in 0..w {
        for y in 0..h {
            let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
            out[y * w + x] = (a..=b).map(|yy| mid[yy * w + x]).sum::<f32>() / n;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marca_com_opacidade_e_sombra() {
        let m = watermark(Corner::TopLeft).expect("wordmark embutido abre");
        assert_eq!(m.rgba.len(), (m.width * m.height * 4) as usize);
        // Mais larga que alta (símbolo + nome), e nada passa de 70% de opacidade.
        assert!(m.width > 2 * m.height, "{}x{}", m.width, m.height);
        let max_alpha = m.rgba.chunks(4).map(|p| p[3]).max().unwrap();
        assert!((170..=179).contains(&max_alpha), "{max_alpha}");
        // O canto de cima é transparente (folga da sombra).
        assert_eq!(m.rgba[3], 0);
        // WATERMARK_OUT=marca.jpg: a marca sobre fundo claro (em cima) e escuro (embaixo), a olho.
        if let Ok(out) = std::env::var("WATERMARK_OUT") {
            let (w, h) = (m.width, m.height);
            let img = image::RgbImage::from_fn(w, h * 2, |x, y| {
                let bg = if y < h { 225. } else { 40. };
                let p = &m.rgba[((y % h) * w + x) as usize * 4..][..4];
                let a = f32::from(p[3]) / 255.;
                image::Rgb([0, 1, 2].map(|c| (f32::from(p[c]) + bg * (1. - a)) as u8))
            });
            img.save(out).unwrap();
        }
    }
}
