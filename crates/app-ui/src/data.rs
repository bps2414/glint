//! Clipes da biblioteca (SQLite do gravador) e as regras de filtro, busca,
//! ordenação e agrupamento por dia, sem nada de UI.

use std::path::Path;

use app_core::library::{Clip, Library};
use app_core::Config;
use chrono::NaiveDate;

use crate::format;

/// O que a grade mostra.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Favorites,
    Published,
    /// Nome do jogo; `None` = clipes sem jogo (gravação da tela).
    Game(Option<String>),
}

impl Filter {
    pub fn matches(&self, c: &Clip) -> bool {
        match self {
            Filter::All => true,
            Filter::Favorites => c.favorite,
            Filter::Published => c.published_url.is_some(),
            Filter::Game(g) => c.game.as_deref() == g.as_deref(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Newest,
    Oldest,
    Largest,
    Longest,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::Newest, Sort::Oldest, Sort::Largest, Sort::Longest];

    pub fn key(self) -> &'static str {
        match self {
            Sort::Newest => "sort.newest",
            Sort::Oldest => "sort.oldest",
            Sort::Largest => "sort.largest",
            Sort::Longest => "sort.longest",
        }
    }

    pub fn next(self) -> Sort {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Ordenações por data mostram separadores de dia; as outras, uma grade só.
    pub fn by_date(self) -> bool {
        matches!(self, Sort::Newest | Sort::Oldest)
    }
}

/// Título para mostrar (o do usuário ou o padrão).
pub fn title_of(c: &Clip, untitled: &str) -> String {
    c.title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(untitled)
        .to_string()
}

fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'ê' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ô' | 'õ' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// Busca por título ou jogo, sem diferenciar maiúsculas e acentos.
pub fn search_matches(c: &Clip, query: &str) -> bool {
    let q = fold(query.trim());
    if q.is_empty() {
        return true;
    }
    let title = c.title.as_deref().map(fold).unwrap_or_default();
    let game = c.game.as_deref().map(fold).unwrap_or_default();
    q.split_whitespace()
        .all(|w| title.contains(w) || game.contains(w))
}

/// Índices de `clips` que passam no filtro e na busca, na ordem pedida.
pub fn visible(clips: &[Clip], filter: &Filter, query: &str, sort: Sort) -> Vec<usize> {
    let mut v: Vec<usize> = (0..clips.len())
        .filter(|&i| filter.matches(&clips[i]) && search_matches(&clips[i], query))
        .collect();
    let key = |i: &usize| {
        let c = &clips[*i];
        match sort {
            Sort::Newest => -c.created_ms,
            Sort::Oldest => c.created_ms,
            Sort::Largest => -(c.size_bytes as i64),
            Sort::Longest => -c.duration_ms,
        }
    };
    v.sort_by_key(key);
    v
}

/// Agrupa índices já ordenados por dia local (`(dia, índices)`).
pub fn group_by_day(clips: &[Clip], order: &[usize]) -> Vec<(NaiveDate, Vec<usize>)> {
    let mut out: Vec<(NaiveDate, Vec<usize>)> = Vec::new();
    for &i in order {
        let d = format::local(clips[i].created_ms).date_naive();
        match out.last_mut() {
            Some((day, items)) if *day == d => items.push(i),
            _ => out.push((d, vec![i])),
        }
    }
    out
}

/// Jogos com clipes, com contagem, do que tem mais clipes para o que tem menos.
pub fn games(clips: &[Clip]) -> Vec<(Option<String>, u32)> {
    let mut out: Vec<(Option<String>, u32)> = Vec::new();
    for c in clips {
        match out.iter_mut().find(|(g, _)| *g == c.game) {
            Some((_, n)) => *n += 1,
            None => out.push((c.game.clone(), 1)),
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Banco de clipes aberto pela UI.
pub struct Store {
    lib: Option<Library>,
    pub clips: Vec<Clip>,
    pub error: Option<String>,
}

impl Store {
    pub fn open() -> Self {
        let mut s = match Library::open(&Config::library_path()) {
            Ok(lib) => Self {
                lib: Some(lib),
                clips: Vec::new(),
                error: None,
            },
            Err(e) => Self {
                lib: None,
                clips: Vec::new(),
                error: Some(e.to_string()),
            },
        };
        s.reload();
        s
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self {
            lib: Some(Library::open_in_memory().unwrap()),
            clips: Vec::new(),
            error: None,
        }
    }

    #[cfg(test)]
    pub fn library(&self) -> &Library {
        self.lib.as_ref().unwrap()
    }

    /// Relê o banco. Clipes cujo arquivo sumiu ficam de fora (o gravador limpa o registro).
    pub fn reload(&mut self) {
        let Some(lib) = &self.lib else { return };
        match lib.list() {
            Ok(list) => {
                self.clips = list.into_iter().filter(|c| c.path.exists()).collect();
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.clips.iter().map(|c| c.size_bytes).sum()
    }

    pub fn index_of(&self, id: i64) -> Option<usize> {
        self.clips.iter().position(|c| c.id == id)
    }

    fn update(&mut self, id: i64, f: impl FnOnce(&Library) -> app_core::library::Result<()>) {
        let Some(lib) = &self.lib else { return };
        if let Err(e) = f(lib) {
            self.error = Some(e.to_string());
            return;
        }
        if let (Some(i), Ok(Some(c))) = (self.index_of(id), lib.get(id)) {
            self.clips[i] = c;
        }
    }

    pub fn set_favorite(&mut self, id: i64, on: bool) {
        self.update(id, |l| l.set_favorite(id, on));
    }

    pub fn set_title(&mut self, id: i64, title: &str) {
        let t = title.trim();
        self.update(id, |l| l.set_title(id, (!t.is_empty()).then_some(t)));
    }

    pub fn set_trim(&mut self, id: i64, start_ms: Option<i64>, end_ms: Option<i64>) {
        self.update(id, |l| l.set_trim(id, start_ms, end_ms));
    }

    pub fn set_published(&mut self, id: i64, url: Option<&str>) {
        self.update(id, |l| l.set_published(id, url));
    }

    /// Apaga o vídeo, a miniatura e o registro.
    pub fn delete(&mut self, id: i64) -> Result<(), String> {
        let Some(i) = self.index_of(id) else {
            return Ok(());
        };
        let c = self.clips[i].clone();
        remove_file(&c.path)?;
        if let Some(t) = &c.thumb_path {
            let _ = std::fs::remove_file(t);
        }
        if let Some(lib) = &self.lib {
            lib.remove(id).map_err(|e| e.to_string())?;
        }
        self.clips.remove(i);
        Ok(())
    }
}

fn remove_file(p: &Path) -> Result<(), String> {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use app_core::library::NewClip;

    use super::*;

    fn clip(id: i64, t: i64, game: Option<&str>, title: Option<&str>, mb: u64, secs: i64) -> Clip {
        Clip {
            id,
            path: PathBuf::from(format!("/c/{id}.mp4")),
            created_ms: t,
            duration_ms: secs * 1000,
            size_bytes: mb * 1_000_000,
            width: 1920,
            height: 1080,
            fps: 60,
            codec: "hevc".into(),
            audio_tracks: 2,
            game: game.map(Into::into),
            game_exe: None,
            title: title.map(Into::into),
            thumb_path: None,
            favorite: false,
            published_url: None,
            trim_start_ms: None,
            trim_end_ms: None,
        }
    }

    fn sample() -> Vec<Clip> {
        let mut v = vec![
            clip(1, 1_000, Some("VALORANT"), Some("Ace no Ascent"), 170, 60),
            clip(2, 3_000, Some("VALORANT"), None, 90, 30),
            clip(3, 2_000, Some("Minecraft"), Some("Ponte"), 300, 45),
            clip(4, 4_000, None, Some("Área de trabalho"), 10, 12),
        ];
        v[2].favorite = true;
        v[0].published_url = Some("https://x/c/1".into());
        v
    }

    #[test]
    fn filtros() {
        let c = sample();
        let ids = |f: Filter| -> Vec<i64> {
            visible(&c, &f, "", Sort::Newest)
                .iter()
                .map(|&i| c[i].id)
                .collect()
        };
        assert_eq!(ids(Filter::All), vec![4, 2, 3, 1]);
        assert_eq!(ids(Filter::Favorites), vec![3]);
        assert_eq!(ids(Filter::Published), vec![1]);
        assert_eq!(ids(Filter::Game(Some("VALORANT".into()))), vec![2, 1]);
        assert_eq!(ids(Filter::Game(None)), vec![4]);
    }

    #[test]
    fn busca_sem_acentos_e_por_jogo() {
        let c = sample();
        let ids = |q: &str| -> Vec<i64> {
            visible(&c, &Filter::All, q, Sort::Oldest)
                .iter()
                .map(|&i| c[i].id)
                .collect()
        };
        assert_eq!(ids("area"), vec![4]);
        assert_eq!(ids("ACE"), vec![1]);
        assert_eq!(ids("valorant"), vec![1, 2]);
        assert_eq!(ids("valorant ace"), vec![1]);
        assert_eq!(ids("   "), vec![1, 3, 2, 4]);
        assert!(ids("xyz").is_empty());
    }

    #[test]
    fn ordenacao() {
        let c = sample();
        let ids = |s: Sort| -> Vec<i64> {
            visible(&c, &Filter::All, "", s)
                .iter()
                .map(|&i| c[i].id)
                .collect()
        };
        assert_eq!(ids(Sort::Largest), vec![3, 1, 2, 4]);
        assert_eq!(ids(Sort::Longest), vec![1, 3, 2, 4]);
        assert_eq!(Sort::Longest.next(), Sort::Newest);
    }

    #[test]
    fn jogos_e_dias() {
        let c = sample();
        let g = games(&c);
        assert_eq!(g[0], (Some("VALORANT".into()), 2));
        assert_eq!(g.len(), 3);
        let order = visible(&c, &Filter::All, "", Sort::Newest);
        // Tudo no mesmo dia (instantes perto da época).
        assert_eq!(group_by_day(&c, &order).len(), 1);
        assert_eq!(title_of(&c[1], "Sem título"), "Sem título");
        assert_eq!(title_of(&c[0], "Sem título"), "Ace no Ascent");
    }

    #[test]
    fn store_atualiza_e_apaga() {
        let dir = std::env::temp_dir().join(format!("store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.mp4");
        std::fs::write(&path, b"x").unwrap();
        let mut s = Store::in_memory();
        let id = s
            .library()
            .insert(&NewClip {
                path: path.clone(),
                created_ms: 1,
                duration_ms: 1000,
                size_bytes: 1,
                width: 16,
                height: 16,
                fps: 60,
                codec: "hevc".into(),
                audio_tracks: 2,
                game: None,
                game_exe: None,
                thumb_path: None,
            })
            .unwrap();
        s.reload();
        assert_eq!(s.clips.len(), 1);
        s.set_favorite(id, true);
        s.set_title(id, "  Ace  ");
        s.set_trim(id, Some(100), Some(900));
        assert!(s.clips[0].favorite);
        assert_eq!(s.clips[0].title.as_deref(), Some("Ace"));
        s.set_title(id, "   ");
        assert_eq!(s.clips[0].title, None);
        assert_eq!(s.clips[0].trim_end_ms, Some(900));
        s.delete(id).unwrap();
        assert!(s.clips.is_empty());
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
