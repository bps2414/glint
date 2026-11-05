//! Gera os ícones do Glint a partir de `docs/brand/icone.svg` (e da variante simplificada
//! `icone-pequeno.svg` para 16–24 px). A saída fica commitada: o build não depende disto.
//!
//! Uso (na raiz do repo): `cargo run -p make-icons`
//!
//! - `assets/brand/glint.ico` (16, 20, 24, 32, 40, 48, 64, 128, 256) — exe e janela
//! - `assets/brand/glint-512.png`, `glint-1024.png` — Discord, README, loja
//! - `server/public/static/icon.svg` + `favicon.ico` (16, 32, 48) — site

use std::fs;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use resvg::{tiny_skia, usvg};

/// Tamanhos dentro do `.ico` do exe.
const ICO_SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];
/// Até este tamanho usa a variante simplificada.
const SMALL_MAX: u32 = 24;

fn main() -> Result<()> {
    let big = load("docs/brand/icone.svg")?;
    let small = load("docs/brand/icone-pequeno.svg")?;
    let pick = |s: u32| if s <= SMALL_MAX { &small } else { &big };

    fs::create_dir_all("assets/brand")?;
    write_ico(
        "assets/brand/glint.ico",
        &ICO_SIZES.map(|s| (s, render(pick(s), s))),
    )?;
    for s in [512, 1024] {
        let png = render(&big, s)
            .encode_png()
            .map_err(|e| anyhow!("png {s}: {e}"))?;
        fs::write(format!("assets/brand/glint-{s}.png"), png)?;
    }
    write_ico(
        "server/public/static/favicon.ico",
        &[16, 32, 48].map(|s| (s, render(pick(s), s))),
    )?;
    fs::copy("docs/brand/icone.svg", "server/public/static/icon.svg")?;
    println!("ícones gerados em assets/brand e server/public/static");
    Ok(())
}

fn load(path: &str) -> Result<usvg::Tree> {
    let data = fs::read(path).with_context(|| format!("lendo {path}"))?;
    usvg::Tree::from_data(&data, &usvg::Options::default()).with_context(|| path.to_string())
}

/// Desenha o SVG num quadrado `size`×`size`.
fn render(tree: &usvg::Tree, size: u32) -> tiny_skia::Pixmap {
    let mut pix = tiny_skia::Pixmap::new(size, size).expect("tamanho válido");
    let k = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(k, k),
        &mut pix.as_mut(),
    );
    pix
}

/// `.ico` com uma imagem por tamanho (RGBA sem pré-multiplicar, como o formato pede).
fn write_ico(path: impl AsRef<Path>, images: &[(u32, tiny_skia::Pixmap)]) -> Result<()> {
    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for (s, pix) in images {
        let rgba: Vec<u8> = pix
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        dir.add_entry(ico::IconDirEntry::encode(&ico::IconImage::from_rgba_data(
            *s, *s, rgba,
        ))?);
    }
    dir.write(fs::File::create(path.as_ref())?)?;
    Ok(())
}
