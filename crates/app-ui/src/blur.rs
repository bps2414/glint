//! Desfoque na CPU para o vidro sobre imagens (chips das miniaturas, cápsula do player,
//! modal). O GPUI não tem `backdrop-filter` dentro da janela: o vidro desenha embaixo de
//! si um pedaço de uma cópia pequena e desfocada do que está atrás.
//!
//! Caminho: BGRA → reduz 1/[`DOWN`] (média de blocos) com um pouco mais de saturação
//! (o `saturate(150%)` do mockup) → [`PASSES`] passadas de box blur (≈ gaussiano). Sem
//! nada do GPUI: puro e testado; a imagem sai em BGRA, pronta para `RenderImage`.

use std::ops::Range;

/// Redução antes do desfoque (o desfoque fica barato e ainda mais largo).
pub const DOWN: u32 = 8;
/// Raio de cada passada (em pixels da imagem reduzida).
pub const RADIUS: u32 = 2;
/// Passadas de box blur (3 ≈ gaussiano).
pub const PASSES: u32 = 3;
/// Saturação extra (1 = igual).
const SATURATE: f32 = 1.4;

/// Imagem BGRA pequena e desfocada.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blurred {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Desfoca as linhas `rows` de uma imagem BGRA `w`×`h` (4 bytes por pixel, sem folga
/// no fim da linha). Devolve a faixa reduzida por [`DOWN`] e desfocada.
pub fn blur_bgra(src: &[u8], w: u32, h: u32, rows: Range<u32>) -> Blurred {
    blur_with(src, w, h, rows, DOWN, RADIUS, PASSES)
}

/// [`blur_bgra`] com os parâmetros abertos (testes).
pub fn blur_with(
    src: &[u8],
    w: u32,
    h: u32,
    rows: Range<u32>,
    down: u32,
    radius: u32,
    passes: u32,
) -> Blurred {
    let rows = rows.start.min(h)..rows.end.min(h);
    let band_h = rows.end.saturating_sub(rows.start);
    if w == 0 || band_h == 0 || src.len() < (w * h * 4) as usize {
        return Blurred {
            width: 0,
            height: 0,
            data: Vec::new(),
        };
    }
    let down = down.max(1);
    let (ow, oh) = (w.div_ceil(down), band_h.div_ceil(down));
    // 1. Reduz: média de cada bloco `down`×`down` (o último bloco pode ser menor).
    let mut px = vec![[0f32; 3]; (ow * oh) as usize];
    for oy in 0..oh {
        let y0 = rows.start + oy * down;
        let y1 = (y0 + down).min(rows.end);
        for ox in 0..ow {
            let x0 = ox * down;
            let x1 = (x0 + down).min(w);
            let mut sum = [0u32; 3];
            for y in y0..y1 {
                let line = (y * w * 4) as usize;
                for x in x0..x1 {
                    let i = line + (x * 4) as usize;
                    sum[0] += u32::from(src[i]);
                    sum[1] += u32::from(src[i + 1]);
                    sum[2] += u32::from(src[i + 2]);
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as f32;
            let (b, g, r) = (sum[0] as f32 / n, sum[1] as f32 / n, sum[2] as f32 / n);
            let l = 0.0722 * b + 0.7152 * g + 0.2126 * r;
            px[(oy * ow + ox) as usize] = [
                l + (b - l) * SATURATE,
                l + (g - l) * SATURATE,
                l + (r - l) * SATURATE,
            ];
        }
    }
    // 2. Box blur separável, `passes` vezes.
    let mut tmp = px.clone();
    for _ in 0..passes {
        box_pass(
            &px,
            &mut tmp,
            ow as usize,
            oh as usize,
            radius as usize,
            true,
        );
        box_pass(
            &tmp,
            &mut px,
            ow as usize,
            oh as usize,
            radius as usize,
            false,
        );
    }
    let mut data = Vec::with_capacity(px.len() * 4);
    for p in px {
        let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        data.extend_from_slice(&[c(p[0]), c(p[1]), c(p[2]), 255]);
    }
    Blurred {
        width: ow,
        height: oh,
        data,
    }
}

/// Uma passada de média móvel (borda repetida), na horizontal ou na vertical.
fn box_pass(src: &[[f32; 3]], dst: &mut [[f32; 3]], w: usize, h: usize, r: usize, horiz: bool) {
    let (len, lines) = if horiz { (w, h) } else { (h, w) };
    let at = |line: usize, k: usize| if horiz { line * w + k } else { k * w + line };
    let span = (2 * r + 1) as f32;
    for line in 0..lines {
        let get = |k: isize| src[at(line, k.clamp(0, len as isize - 1) as usize)];
        let mut acc = [0f32; 3];
        for k in -(r as isize)..=(r as isize) {
            let p = get(k);
            acc[0] += p[0];
            acc[1] += p[1];
            acc[2] += p[2];
        }
        for k in 0..len {
            dst[at(line, k)] = [acc[0] / span, acc[1] / span, acc[2] / span];
            let (add, sub) = (
                get(k as isize + r as isize + 1),
                get(k as isize - r as isize),
            );
            acc[0] += add[0] - sub[0];
            acc[1] += add[1] - sub[1];
            acc[2] += add[2] - sub[2];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{blur_bgra, blur_with, DOWN};

    fn solid(w: u32, h: u32, bgra: [u8; 4]) -> Vec<u8> {
        bgra.repeat((w * h) as usize)
    }

    #[test]
    fn cor_lisa_continua_igual() {
        // Cinza: a saturação não muda nada.
        let img = solid(64, 32, [90, 90, 90, 255]);
        let b = blur_bgra(&img, 64, 32, 0..32);
        assert_eq!((b.width, b.height), (64 / DOWN, 32 / DOWN));
        assert!(b.data.chunks(4).all(|p| p == [90, 90, 90, 255]));
    }

    #[test]
    fn faixa_e_tamanhos_quebrados() {
        let img = solid(100, 50, [10, 20, 30, 255]);
        let b = blur_bgra(&img, 100, 50, 40..60);
        // Linhas 40..50 (o fim passa da imagem): 100/8 → 13, 10/8 → 2.
        assert_eq!((b.width, b.height), (13, 2));
        assert_eq!(b.data.len(), 13 * 2 * 4);
        let vazio = blur_bgra(&img, 100, 50, 60..70);
        assert_eq!((vazio.width, vazio.height), (0, 0));
        assert!(blur_bgra(&[], 10, 10, 0..10).data.is_empty());
    }

    #[test]
    fn ponto_claro_se_espalha() {
        let (w, h) = (9u32, 9u32);
        let mut img = solid(w, h, [0, 0, 0, 255]);
        let c = ((4 * w + 4) * 4) as usize;
        img[c..c + 3].copy_from_slice(&[255, 255, 255]);
        let b = blur_with(&img, w, h, 0..h, 1, 1, 1);
        let v = |x: u32, y: u32| b.data[((y * w + x) * 4) as usize];
        assert!(v(4, 4) < 255 && v(4, 4) > 0);
        assert!(v(3, 4) > 0 && v(4, 5) > 0);
        assert_eq!(v(0, 0), 0);
    }

    #[test]
    fn satura_um_pouco() {
        let img = solid(8, 8, [100, 100, 160, 255]);
        let b = blur_with(&img, 8, 8, 0..8, 8, 0, 0);
        // Vermelho (BGRA: índice 2) sobe, azul desce.
        assert!(b.data[2] > 160 && b.data[0] < 100);
    }

    /// Meta do plano: < 1 ms para uma faixa de 1920×120 (rodar em release).
    #[test]
    #[ignore]
    fn tempo_faixa_1080p() {
        let img = solid(1920, 1080, [40, 80, 120, 255]);
        let t = std::time::Instant::now();
        for _ in 0..100 {
            std::hint::black_box(blur_bgra(&img, 1920, 1080, 960..1080));
        }
        println!("faixa 1920x120: {:?} por chamada", t.elapsed() / 100);
    }
}
