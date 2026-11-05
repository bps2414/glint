//! Nomes de arquivo dos clipes (`Clipe_VALORANT_2026-11-04_21-30-00.mp4`).

use std::path::{Path, PathBuf};

use crate::brand;

/// Parte do nome do jogo segura para nome de arquivo em qualquer SO.
pub fn sanitize(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        let ok = ch.is_alphanumeric() || matches!(ch, '-' | '.' | '(' | ')');
        if ok {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        if out.chars().count() >= 40 {
            break;
        }
    }
    out.trim_matches(['-', '.']).to_string()
}

/// Nome sem extensão. `timestamp` já formatado (ex.: `2026-11-04_21-30-00`).
pub fn clip_stem(game: Option<&str>, timestamp: &str) -> String {
    match game.map(sanitize).filter(|g| !g.is_empty()) {
        Some(g) => format!("{}_{g}_{timestamp}", brand::CLIP_FILE_PREFIX),
        None => format!("{}_{timestamp}", brand::CLIP_FILE_PREFIX),
    }
}

/// Caminho livre em `dir`: `<stem>.<ext>`, `<stem>_2.<ext>`, ...
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}_{n}.{ext}"));
        n += 1;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("VALORANT  "), "VALORANT");
        assert_eq!(sanitize("Counter-Strike 2"), "Counter-Strike-2");
        assert_eq!(sanitize(r#"a<b>:c"/\|?*"#), "a-b-c");
        assert_eq!(sanitize("..."), "");
        assert_eq!(sanitize("Pokémon"), "Pokémon");
        assert!(sanitize(&"x".repeat(100)).len() <= 40);
    }

    #[test]
    fn stems() {
        let p = brand::CLIP_FILE_PREFIX;
        assert_eq!(
            clip_stem(Some("VALORANT"), "2026-11-04_21-30-00"),
            format!("{p}_VALORANT_2026-11-04_21-30-00")
        );
        assert_eq!(clip_stem(Some("???"), "t"), format!("{p}_t"));
        assert_eq!(clip_stem(None, "t"), format!("{p}_t"));
    }

    #[test]
    fn unique_paths() {
        let dir = std::env::temp_dir().join(format!("naming-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = unique_path(&dir, "x", "mp4");
        std::fs::write(&a, b"").unwrap();
        let b = unique_path(&dir, "x", "mp4");
        assert_eq!(b.file_name().unwrap(), "x_2.mp4");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
