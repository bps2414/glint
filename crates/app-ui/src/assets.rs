//! Fonte de assets: ícones próprios (traço igual ao da proposta visual) e, como
//! reserva, o conjunto Lucide do gpui-kit usado pelos componentes prontos.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*]
    };
}

const OWN: &[(&str, &[u8])] = icons![
    "back",
    "check",
    "chevron-down",
    "chevron-left",
    "chevron-right",
    "close",
    "copy",
    "folder",
    "fullscreen",
    "fullscreen-exit",
    "gamepad",
    "grid",
    "link",
    "mic",
    "mic-off",
    "pause",
    "play",
    "search",
    "settings",
    "star",
    "star-filled",
    "trash",
    "upload",
    "volume",
    "volume-off",
    "win-close",
    "win-max",
    "win-min",
    "win-restore",
];

/// Caminhos dos ícones próprios (`svg().path(icon::GRID)`).
#[allow(dead_code)]
pub mod icon {
    pub const BACK: &str = "icons/back.svg";
    pub const CHECK: &str = "icons/check.svg";
    pub const CHEVRON_DOWN: &str = "icons/chevron-down.svg";
    pub const CHEVRON_LEFT: &str = "icons/chevron-left.svg";
    pub const CHEVRON_RIGHT: &str = "icons/chevron-right.svg";
    pub const CLOSE: &str = "icons/close.svg";
    pub const COPY: &str = "icons/copy.svg";
    pub const FOLDER: &str = "icons/folder.svg";
    pub const FULLSCREEN: &str = "icons/fullscreen.svg";
    pub const FULLSCREEN_EXIT: &str = "icons/fullscreen-exit.svg";
    pub const GAMEPAD: &str = "icons/gamepad.svg";
    pub const GRID: &str = "icons/grid.svg";
    pub const LINK: &str = "icons/link.svg";
    pub const MIC: &str = "icons/mic.svg";
    pub const MIC_OFF: &str = "icons/mic-off.svg";
    pub const PAUSE: &str = "icons/pause.svg";
    pub const PLAY: &str = "icons/play.svg";
    pub const SEARCH: &str = "icons/search.svg";
    pub const SETTINGS: &str = "icons/settings.svg";
    pub const STAR: &str = "icons/star.svg";
    pub const STAR_FILLED: &str = "icons/star-filled.svg";
    pub const TRASH: &str = "icons/trash.svg";
    pub const UPLOAD: &str = "icons/upload.svg";
    pub const VOLUME: &str = "icons/volume.svg";
    pub const VOLUME_OFF: &str = "icons/volume-off.svg";
    pub const WIN_CLOSE: &str = "icons/win-close.svg";
    pub const WIN_MAX: &str = "icons/win-max.svg";
    pub const WIN_MIN: &str = "icons/win-min.svg";
    pub const WIN_RESTORE: &str = "icons/win-restore.svg";
}

/// Símbolo colorido da marca (o mesmo SVG do `docs/brand`, desenhado como imagem).
const BRAND: &[(&str, &[u8])] = &[(
    "brand/glint.svg",
    include_bytes!("../../../docs/brand/icone.svg"),
)];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, data)) = BRAND.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(*data)));
        }
        if let Some((_, data)) = OWN.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(*data)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out: Vec<SharedString> = OWN
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect();
        out.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todos_os_icones_carregam() {
        for (path, _) in OWN {
            let data = Assets.load(path).unwrap().expect(path);
            assert!(
                std::str::from_utf8(&data).unwrap().starts_with("<svg"),
                "{path}"
            );
        }
    }
}
