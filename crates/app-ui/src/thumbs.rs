//! Cache das miniaturas dos cards (JPEG de 640 px do gravador → imagem do GPUI).
//!
//! A decodificação roda fora da thread da UI; cada imagem é reduzida para
//! [`CARD_W`] de largura (o card nunca passa disso) para não encher o atlas da GPU.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::RenderImage;

/// Largura das miniaturas guardadas (card de ~300 px lógicos em tela 125%).
pub const CARD_W: u32 = 384;

/// Miniatura e a cópia pequena desfocada (fundo dos chips de vidro em cima dela).
#[derive(Clone)]
pub struct Thumb {
    pub image: Arc<RenderImage>,
    pub blur: Option<Arc<RenderImage>>,
}

#[derive(Default)]
pub struct Thumbs {
    ready: HashMap<PathBuf, Thumb>,
    /// Pedidas e ainda não carregadas (ou que falharam: não tenta de novo).
    asked: HashSet<PathBuf>,
}

impl Thumbs {
    pub fn get(&self, path: &Path) -> Option<Arc<RenderImage>> {
        self.ready.get(path).map(|t| t.image.clone())
    }

    /// Versão desfocada da miniatura.
    pub fn blur(&self, path: &Path) -> Option<Arc<RenderImage>> {
        self.ready.get(path).and_then(|t| t.blur.clone())
    }

    /// Marca como pedidas e devolve as que ainda não foram pedidas.
    pub fn missing<'a>(&mut self, paths: impl IntoIterator<Item = &'a Path>) -> Vec<PathBuf> {
        paths
            .into_iter()
            .filter(|p| !self.ready.contains_key(*p) && self.asked.insert(p.to_path_buf()))
            .map(Path::to_path_buf)
            .collect()
    }

    pub fn insert(&mut self, path: PathBuf, thumb: Thumb) {
        self.ready.insert(path, thumb);
    }

    /// Esquece uma miniatura (clipe apagado). Devolve as imagens para sair do atlas.
    pub fn remove(&mut self, path: &Path) -> Vec<Arc<RenderImage>> {
        self.asked.remove(path);
        self.ready
            .remove(path)
            .map(|t| std::iter::once(t.image).chain(t.blur).collect())
            .unwrap_or_default()
    }
}

/// Converte RGBA em BGRA no lugar (o GPUI desenha `RenderImage` como BGRA).
pub fn rgba_to_bgra(buf: &mut [u8]) {
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

/// Imagem BGRA do GPUI a partir de pixels BGRA.
pub fn image_from_bgra(width: u32, height: u32, data: Vec<u8>) -> Option<Arc<RenderImage>> {
    let buf = image::RgbaImage::from_raw(width, height, data)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
}

/// Lê e reduz um JPEG para no máximo `max_w` de largura, com a cópia desfocada.
pub fn load(path: &Path, max_w: u32) -> Option<Thumb> {
    let img = image::open(path).ok()?;
    let img = if img.width() > max_w {
        let h = (u64::from(img.height()) * u64::from(max_w) / u64::from(img.width())) as u32;
        img.resize_exact(max_w, h.max(1), image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    rgba_to_bgra(&mut rgba);
    let b = crate::blur::blur_bgra(&rgba, w, h, 0..h);
    let blur = image_from_bgra(b.width, b.height, b.data);
    Some(Thumb {
        image: image_from_bgra(w, h, rgba.into_raw())?,
        blur,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn troca_canais() {
        let mut px = vec![1, 2, 3, 4, 5, 6, 7, 8];
        rgba_to_bgra(&mut px);
        assert_eq!(px, vec![3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn pede_cada_miniatura_uma_vez() {
        let mut t = Thumbs::default();
        let a = PathBuf::from("a.jpg");
        let b = PathBuf::from("b.jpg");
        assert_eq!(t.missing([a.as_path(), b.as_path()]).len(), 2);
        assert!(t.missing([a.as_path()]).is_empty());
        t.remove(&a);
        assert_eq!(t.missing([a.as_path()]).len(), 1);
    }

    #[test]
    fn carrega_e_reduz_jpeg() {
        let dir = std::env::temp_dir().join(format!("thumbs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.jpg");
        image::RgbImage::from_pixel(640, 360, image::Rgb([200, 10, 10]))
            .save(&p)
            .unwrap();
        assert!(load(&p, CARD_W).is_some());
        assert!(load(&dir.join("nao-existe.jpg"), CARD_W).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
