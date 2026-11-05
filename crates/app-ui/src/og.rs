//! Imagem do link no Discord (og:image, 1200×630), composta na hora de publicar: o quadro
//! do clipe cobrindo a imagem, escurecido embaixo, o botão de tocar de vidro no meio e o
//! símbolo da marca no canto. Sem texto (o Discord já mostra o título). Mockup: Site.dc.html.

use image::{ImageEncoder, RgbaImage};
use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap,
    PixmapPaint, Point, Shader, SpreadMode, Stroke, Transform,
};

pub const W: u32 = 1200;
pub const H: u32 = 630;
/// Escala do mockup (prévia de 520 px de largura).
const K: f32 = W as f32 / 520.0;

/// Compõe a imagem a partir do quadro (RGBA) e devolve o JPEG.
pub fn compose(frame: &RgbaImage, quality: u8) -> Option<Vec<u8>> {
    let mut pix = Pixmap::new(W, H)?;
    cover(&mut pix, frame)?;

    // Escurecido embaixo: transparente até 45%, preto 60% no fim.
    let shade = LinearGradient::new(
        Point::from_xy(0., H as f32 * 0.45),
        Point::from_xy(0., H as f32),
        vec![
            GradientStop::new(0., Color::from_rgba8(0, 0, 0, 0)),
            GradientStop::new(1., Color::from_rgba8(0, 0, 0, 153)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )?;
    let full = tiny_skia::Rect::from_xywh(0., 0., W as f32, H as f32)?;
    pix.fill_rect(full, &paint(shade), Transform::identity(), None);

    // Botão de tocar de vidro (56 px no mockup, centro a 42% da altura).
    let (cx, cy, r) = (W as f32 / 2., H as f32 * 0.42, 28. * K);
    let circle = PathBuilder::from_circle(cx, cy, r)?;
    let glass = LinearGradient::new(
        Point::from_xy(cx, cy - r),
        Point::from_xy(cx, cy + r),
        vec![
            GradientStop::new(0., Color::from_rgba8(255, 255, 255, 77)),
            GradientStop::new(1., Color::from_rgba8(255, 255, 255, 26)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )?;
    pix.fill_path(
        &circle,
        &paint(glass),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    let mut edge = Paint::default();
    edge.set_color_rgba8(255, 255, 255, 89);
    edge.anti_alias = true;
    let stroke = Stroke {
        width: K,
        ..Stroke::default()
    };
    pix.stroke_path(&circle, &edge, &stroke, Transform::identity(), None);
    // Triângulo (o ícone de play do mockup, 20 px num viewBox de 24).
    let s = 20. * K / 24.;
    let (ox, oy) = (cx - 12. * s, cy - 12. * s);
    let mut tri = PathBuilder::new();
    tri.move_to(ox + 8.5 * s, oy + 5.5 * s);
    tri.line_to(ox + 19.5 * s, oy + 12. * s);
    tri.line_to(ox + 8.5 * s, oy + 18.5 * s);
    tri.close();
    let mut white = Paint::default();
    white.set_color_rgba8(255, 255, 255, 255);
    white.anti_alias = true;
    pix.fill_path(
        &tri.finish()?,
        &white,
        FillRule::Winding,
        Transform::identity(),
        None,
    );

    // Símbolo da marca embaixo à esquerda (22 px no mockup, 14 da borda).
    if let Some(mark) = mark() {
        let size = 22. * K;
        let k = size / mark.width() as f32;
        pix.draw_pixmap(
            0,
            0,
            mark.as_ref(),
            &PixmapPaint {
                quality: FilterQuality::Bicubic,
                ..PixmapPaint::default()
            },
            Transform::from_scale(k, k).post_translate(14. * K, H as f32 - 12. * K - size),
            None,
        );
    }

    // JPEG (sem alfa).
    let rgb: Vec<u8> = pix
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue()]
        })
        .collect();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .write_image(&rgb, W, H, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(out)
}

fn paint(shader: Shader<'_>) -> Paint<'_> {
    Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    }
}

/// Desenha o quadro cobrindo a imagem inteira (corta as sobras, como `object-fit: cover`).
fn cover(pix: &mut Pixmap, frame: &RgbaImage) -> Option<()> {
    let (fw, fh) = frame.dimensions();
    let mut src = Pixmap::new(fw.max(1), fh.max(1))?;
    for (d, s) in src.pixels_mut().iter_mut().zip(frame.pixels()) {
        *d = tiny_skia::ColorU8::from_rgba(s[0], s[1], s[2], 255).premultiply();
    }
    let k = (W as f32 / fw as f32).max(H as f32 / fh as f32);
    let (dx, dy) = (
        (W as f32 - fw as f32 * k) / 2.,
        (H as f32 - fh as f32 * k) / 2.,
    );
    pix.fill(Color::BLACK);
    pix.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        },
        Transform::from_scale(k, k).post_translate(dx, dy),
        None,
    );
    Some(())
}

/// Símbolo do Glint (PNG gerado pelo `tools/make-icons`).
fn mark() -> Option<Pixmap> {
    Pixmap::decode_png(include_bytes!("../../../assets/brand/glint-512.png")).ok()
}

#[cfg(test)]
mod tests {
    use super::{compose, H, W};

    #[test]
    fn compoe_jpeg_1200x630() {
        // Quadro 16:9 cinza: a imagem sai 1200×630, JPEG, pequena e com o botão claro no meio.
        let frame = image::RgbaImage::from_pixel(640, 360, image::Rgba([90, 100, 110, 255]));
        let frame = match std::env::var("OG_FRAME") {
            // OG_FRAME=quadro.jpg OG_OUT=saida.jpg: confere a composição a olho.
            Ok(p) => image::open(p).expect("quadro").to_rgba8(),
            Err(_) => frame,
        };
        let jpg = compose(&frame, 85).expect("compõe");
        if let Ok(out) = std::env::var("OG_OUT") {
            std::fs::write(out, &jpg).unwrap();
        }
        assert!(jpg.len() < 1024 * 1024);
        let img = image::load_from_memory(&jpg)
            .expect("jpeg válido")
            .to_rgb8();
        assert_eq!(img.dimensions(), (W, H));
        let centro = img.get_pixel(W / 2, (H as f32 * 0.42) as u32);
        let canto_baixo = img.get_pixel(W - 4, H - 4);
        // Triângulo branco no centro; embaixo escurecido.
        assert!(centro[0] > 200);
        assert!(canto_baixo[0] < 60);
    }
}
