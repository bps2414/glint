//! Biblioteca de clipes em SQLite (`%LOCALAPPDATA%\<app>\library.db`).
//!
//! O gravador insere um registro a cada clipe salvo; a UI lê e marca
//! favoritos/recortes. Modo WAL + `busy_timeout` para os dois processos
//! usarem o banco ao mesmo tempo.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::retention::{self, ClipUsage};

pub type Result<T> = std::result::Result<T, LibraryError>;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("banco de clipes: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("banco de clipes em {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Versão do esquema (`PRAGMA user_version`).
const SCHEMA_VERSION: i64 = 2;

const MIGRATION_1: &str = "
CREATE TABLE clips (
    id            INTEGER PRIMARY KEY,
    path          TEXT    NOT NULL UNIQUE,
    created_ms    INTEGER NOT NULL,
    duration_ms   INTEGER NOT NULL,
    size_bytes    INTEGER NOT NULL,
    width         INTEGER NOT NULL,
    height        INTEGER NOT NULL,
    fps           INTEGER NOT NULL,
    codec         TEXT    NOT NULL,
    audio_tracks  INTEGER NOT NULL,
    game          TEXT,
    game_exe      TEXT,
    title         TEXT,
    thumb_path    TEXT,
    favorite      INTEGER NOT NULL DEFAULT 0,
    published_url TEXT,
    trim_start_ms INTEGER,
    trim_end_ms   INTEGER
);
CREATE INDEX clips_created ON clips(created_ms DESC);
";

/// Clipe de kill: tipo (`replay` = F8) e quantos apertos do combo (`kills`).
const MIGRATION_2: &str = "
ALTER TABLE clips ADD COLUMN kind TEXT NOT NULL DEFAULT 'replay';
ALTER TABLE clips ADD COLUMN kills INTEGER;
";

/// Uma migração por versão, em ordem (`MIGRATIONS[i]` leva o esquema para a versão `i + 1`).
const MIGRATIONS: [&str; 2] = [MIGRATION_1, MIGRATION_2];

/// De onde veio o clipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipKind {
    /// Últimos segundos do buffer (hotkey de salvar).
    Replay,
    /// Trecho em volta da jogada (hotkey de kill).
    Kill,
}

impl ClipKind {
    fn as_db(self) -> &'static str {
        match self {
            ClipKind::Replay => "replay",
            ClipKind::Kill => "kill",
        }
    }

    fn from_db(s: &str) -> Self {
        match s {
            "kill" => ClipKind::Kill,
            _ => ClipKind::Replay,
        }
    }
}

/// Dados de um clipe recém-salvo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewClip {
    pub path: PathBuf,
    pub created_ms: i64,
    pub duration_ms: i64,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: String,
    pub audio_tracks: u32,
    pub game: Option<String>,
    pub game_exe: Option<String>,
    pub thumb_path: Option<PathBuf>,
    pub kind: ClipKind,
    /// Apertos do combo (só clipes de kill).
    pub kills: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    pub id: i64,
    pub path: PathBuf,
    pub created_ms: i64,
    pub duration_ms: i64,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: String,
    pub audio_tracks: u32,
    pub game: Option<String>,
    pub game_exe: Option<String>,
    pub title: Option<String>,
    pub thumb_path: Option<PathBuf>,
    pub favorite: bool,
    pub published_url: Option<String>,
    pub trim_start_ms: Option<i64>,
    pub trim_end_ms: Option<i64>,
    pub kind: ClipKind,
    pub kills: Option<u32>,
}

impl Clip {
    /// Favorito ou publicado: a limpeza automática não apaga.
    pub fn is_protected(&self) -> bool {
        self.favorite || self.published_url.is_some()
    }
}

const CLIP_COLUMNS: &str = "id, path, created_ms, duration_ms, size_bytes, width, height, fps, \
     codec, audio_tracks, game, game_exe, title, thumb_path, favorite, published_url, \
     trim_start_ms, trim_end_ms, kind, kills";

fn clip_from_row(r: &Row<'_>) -> rusqlite::Result<Clip> {
    Ok(Clip {
        id: r.get(0)?,
        path: PathBuf::from(r.get::<_, String>(1)?),
        created_ms: r.get(2)?,
        duration_ms: r.get(3)?,
        size_bytes: r.get::<_, i64>(4)?.max(0) as u64,
        width: r.get(5)?,
        height: r.get(6)?,
        fps: r.get(7)?,
        codec: r.get(8)?,
        audio_tracks: r.get(9)?,
        game: r.get(10)?,
        game_exe: r.get(11)?,
        title: r.get(12)?,
        thumb_path: r.get::<_, Option<String>>(13)?.map(PathBuf::from),
        favorite: r.get::<_, i64>(14)? != 0,
        published_url: r.get(15)?,
        trim_start_ms: r.get(16)?,
        trim_end_ms: r.get(17)?,
        kind: ClipKind::from_db(&r.get::<_, String>(18)?),
        kills: r.get(19)?,
    })
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

pub struct Library {
    conn: Connection,
}

impl Library {
    /// Abre (ou cria) o banco e aplica as migrações.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| LibraryError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < SCHEMA_VERSION {
            for (i, sql) in MIGRATIONS.iter().enumerate() {
                let to = i as i64 + 1;
                if version < to {
                    conn.execute_batch(&format!(
                        "BEGIN; {sql} PRAGMA user_version = {to}; COMMIT;"
                    ))?;
                }
            }
        }
        Ok(Self { conn })
    }

    pub fn insert(&self, c: &NewClip) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO clips (path, created_ms, duration_ms, size_bytes, width, height, fps,
                 codec, audio_tracks, game, game_exe, thumb_path, kind, kills)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                path_str(&c.path),
                c.created_ms,
                c.duration_ms,
                c.size_bytes as i64,
                c.width,
                c.height,
                c.fps,
                c.codec,
                c.audio_tracks,
                c.game,
                c.game_exe,
                c.thumb_path.as_deref().map(path_str),
                c.kind.as_db(),
                c.kills,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get(&self, id: i64) -> Result<Option<Clip>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {CLIP_COLUMNS} FROM clips WHERE id = ?1"),
                [id],
                clip_from_row,
            )
            .optional()?)
    }

    /// Todos os clipes, do mais novo para o mais antigo.
    pub fn list(&self) -> Result<Vec<Clip>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {CLIP_COLUMNS} FROM clips ORDER BY created_ms DESC, id DESC"
        ))?;
        let rows = st.query_map([], clip_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM clips", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    pub fn total_bytes(&self) -> Result<u64> {
        let n: i64 =
            self.conn
                .query_row("SELECT COALESCE(SUM(size_bytes), 0) FROM clips", [], |r| {
                    r.get(0)
                })?;
        Ok(n.max(0) as u64)
    }

    pub fn set_favorite(&self, id: i64, favorite: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE clips SET favorite = ?2 WHERE id = ?1",
            params![id, favorite as i64],
        )?;
        Ok(())
    }

    pub fn set_title(&self, id: i64, title: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE clips SET title = ?2 WHERE id = ?1",
            params![id, title],
        )?;
        Ok(())
    }

    pub fn set_trim(&self, id: i64, start_ms: Option<i64>, end_ms: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE clips SET trim_start_ms = ?2, trim_end_ms = ?3 WHERE id = ?1",
            params![id, start_ms, end_ms],
        )?;
        Ok(())
    }

    pub fn set_published(&self, id: i64, url: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE clips SET published_url = ?2 WHERE id = ?1",
            params![id, url],
        )?;
        Ok(())
    }

    /// Remove só o registro (os arquivos ficam por conta de quem chama).
    pub fn remove(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM clips WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Remove registros cujo arquivo de vídeo sumiu (apagado pelo Explorer etc.).
    /// Devolve os clipes removidos (para apagar a miniatura).
    pub fn forget_missing(&self, exists: impl Fn(&Path) -> bool) -> Result<Vec<Clip>> {
        let gone: Vec<Clip> = self
            .list()?
            .into_iter()
            .filter(|c| !exists(&c.path))
            .collect();
        for c in &gone {
            self.remove(c.id)?;
        }
        Ok(gone)
    }

    /// Clipes que a limpeza apagaria para caber em `limit_bytes`.
    pub fn cleanup_plan(&self, limit_bytes: u64) -> Result<Vec<Clip>> {
        let clips = self.list()?;
        let usage: Vec<ClipUsage> = clips
            .iter()
            .map(|c| ClipUsage {
                id: c.id,
                size_bytes: c.size_bytes,
                created_ms: c.created_ms,
                protected: c.is_protected(),
            })
            .collect();
        let ids = retention::plan_cleanup(&usage, limit_bytes);
        // Na ordem do plano (mais antigo primeiro).
        Ok(ids
            .iter()
            .filter_map(|id| clips.iter().find(|c| c.id == *id).cloned())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_clip(name: &str, t: i64, mb: u64) -> NewClip {
        NewClip {
            path: PathBuf::from(format!("/clips/{name}.mp4")),
            created_ms: t,
            duration_ms: 60_000,
            size_bytes: mb * 1_000_000,
            width: 1920,
            height: 1080,
            fps: 60,
            codec: "hevc".into(),
            audio_tracks: 2,
            game: Some("VALORANT".into()),
            game_exe: Some("VALORANT-Win64-Shipping.exe".into()),
            thumb_path: Some(PathBuf::from(format!("/thumbs/{name}.jpg"))),
            kind: ClipKind::Replay,
            kills: None,
        }
    }

    #[test]
    fn insert_list_update() {
        let lib = Library::open_in_memory().unwrap();
        let a = lib.insert(&new_clip("a", 1000, 100)).unwrap();
        let b = lib.insert(&new_clip("b", 2000, 150)).unwrap();
        assert_eq!(lib.count().unwrap(), 2);
        assert_eq!(lib.total_bytes().unwrap(), 250_000_000);

        let all = lib.list().unwrap();
        assert_eq!(all[0].id, b, "mais novo primeiro");
        assert_eq!(all[1].game.as_deref(), Some("VALORANT"));
        assert!(!all[1].favorite);

        lib.set_favorite(a, true).unwrap();
        lib.set_trim(a, Some(1500), Some(9000)).unwrap();
        lib.set_title(a, Some("ace")).unwrap();
        let ca = lib.get(a).unwrap().unwrap();
        assert!(ca.favorite && ca.is_protected());
        assert_eq!((ca.trim_start_ms, ca.trim_end_ms), (Some(1500), Some(9000)));
        assert_eq!(ca.title.as_deref(), Some("ace"));

        // Caminho repetido é recusado.
        assert!(lib.insert(&new_clip("a", 3000, 1)).is_err());
        lib.remove(b).unwrap();
        assert!(lib.get(b).unwrap().is_none());
    }

    #[test]
    fn kill_clips_keep_kind_and_count() {
        let lib = Library::open_in_memory().unwrap();
        let replay = lib.insert(&new_clip("r", 1, 1)).unwrap();
        let kill_clip = NewClip {
            kind: ClipKind::Kill,
            kills: Some(3),
            ..new_clip("k", 2, 1)
        };
        let kill = lib.insert(&kill_clip).unwrap();
        let r = lib.get(replay).unwrap().unwrap();
        assert_eq!((r.kind, r.kills), (ClipKind::Replay, None));
        let k = lib.get(kill).unwrap().unwrap();
        assert_eq!((k.kind, k.kills), (ClipKind::Kill, Some(3)));
    }

    #[test]
    fn upgrades_v1_database() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "{MIGRATION_1} PRAGMA user_version = 1;
             INSERT INTO clips (path, created_ms, duration_ms, size_bytes, width, height, fps,
                 codec, audio_tracks)
             VALUES ('/clips/old.mp4', 1, 60000, 1000, 1920, 1080, 60, 'hevc', 2);"
        ))
        .unwrap();
        let lib = Library::init(conn).unwrap();
        let version: i64 = lib
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let all = lib.list().unwrap();
        assert_eq!((all[0].kind, all[0].kills), (ClipKind::Replay, None));
        // Banco migrado aceita clipes de kill.
        let kill_clip = NewClip {
            kind: ClipKind::Kill,
            kills: Some(2),
            ..new_clip("k", 2, 1)
        };
        lib.insert(&kill_clip).unwrap();
        assert_eq!(lib.count().unwrap(), 2);
    }

    #[test]
    fn cleanup_respects_protection() {
        let lib = Library::open_in_memory().unwrap();
        let old = lib.insert(&new_clip("old", 1, 100)).unwrap();
        let fav = lib.insert(&new_clip("fav", 2, 100)).unwrap();
        let publ = lib.insert(&new_clip("pub", 3, 100)).unwrap();
        let mid = lib.insert(&new_clip("mid", 4, 100)).unwrap();
        let _new = lib.insert(&new_clip("new", 5, 100)).unwrap();
        lib.set_favorite(fav, true).unwrap();
        lib.set_published(publ, Some("https://x/c/1")).unwrap();
        let plan: Vec<i64> = lib
            .cleanup_plan(250_000_000)
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(plan, vec![old, mid]);
    }

    #[test]
    fn forgets_missing_files() {
        let lib = Library::open_in_memory().unwrap();
        lib.insert(&new_clip("here", 1, 1)).unwrap();
        lib.insert(&new_clip("gone", 2, 1)).unwrap();
        let gone = lib
            .forget_missing(|p| p.to_string_lossy().contains("here"))
            .unwrap();
        assert_eq!(gone.len(), 1);
        assert_eq!(lib.count().unwrap(), 1);
    }

    #[test]
    fn file_database_reopens() {
        let dir = std::env::temp_dir().join(format!("lib-test-{}", std::process::id()));
        let path = dir.join("library.db");
        {
            let lib = Library::open(&path).unwrap();
            lib.insert(&new_clip("a", 1, 1)).unwrap();
        }
        let lib = Library::open(&path).unwrap();
        assert_eq!(lib.count().unwrap(), 1);
        drop(lib);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
