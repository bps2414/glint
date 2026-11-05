//! Migração 1× dos dados do nome antigo ("Clipe") para o atual ("Glint").
//!
//! Chamada no início do gravador e da UI (quem abrir primeiro). Ordem: para o gravador
//! antigo → copia config e dados (banco, miniaturas, logs) → confere → move a pasta de
//! vídeos padrão → ajusta caminhos e o host do link no banco e o `server_url` da config →
//! token → autostart → marcador → só então apaga o antigo. Qualquer erro deixa o antigo
//! intacto e a próxima abertura tenta de novo.
//!
//! Sem código de SO: o que depende do sistema (IPC, credenciais, registro) entra por
//! [`Host`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::Config;

/// Nome antigo da marca.
pub const OLD_NAME: &str = "Clipe";
/// Host antigo do servidor e o novo.
pub const OLD_HOST: &str = "clipe.bpsstudio.workers.dev";
pub const NEW_HOST: &str = "clips.getglint.workers.dev";

const MARKER: &str = "migrated-from-clipe";
const LOCK: &str = "migrate.lock";
/// Um lock mais velho que isto é de uma migração que morreu no meio.
const LOCK_STALE: Duration = Duration::from_secs(120);

/// O que depende do sistema.
pub trait Host {
    /// Pede para o gravador antigo fechar e espera até ~5 s.
    fn stop_old_recorder(&mut self);
    /// Move o token da conta da credencial antiga para a nova (sem token = nada).
    fn move_token(&mut self) -> Result<(), String>;
    /// Tira o "iniciar com o Windows" do nome antigo (o gravador novo registra o dele).
    fn drop_old_autostart(&mut self) -> Result<(), String>;
}

/// [`Host`] feito de funções (quem chama monta com o que o sistema oferece).
pub struct Hooks<'a> {
    pub stop_old_recorder: Box<dyn FnMut() + 'a>,
    pub move_token: Box<dyn FnMut() -> Result<(), String> + 'a>,
    pub drop_old_autostart: Box<dyn FnMut() -> Result<(), String> + 'a>,
}

impl Host for Hooks<'_> {
    fn stop_old_recorder(&mut self) {
        (self.stop_old_recorder)()
    }
    fn move_token(&mut self) -> Result<(), String> {
        (self.move_token)()
    }
    fn drop_old_autostart(&mut self) -> Result<(), String> {
        (self.drop_old_autostart)()
    }
}

/// Pastas envolvidas.
#[derive(Clone, Debug)]
pub struct Paths {
    pub old_config: PathBuf,
    pub old_data: PathBuf,
    pub new_config: PathBuf,
    pub new_data: PathBuf,
    /// Pasta de vídeos padrão antiga e nova (`Vídeos\Clipe` → `Vídeos\Glint`).
    pub old_videos: Option<PathBuf>,
    pub new_videos: Option<PathBuf>,
}

impl Paths {
    /// Pastas reais deste usuário.
    pub fn detect() -> Self {
        let old = directories::ProjectDirs::from("", "", OLD_NAME);
        let videos =
            directories::UserDirs::new().and_then(|u| u.video_dir().map(Path::to_path_buf));
        Self {
            old_config: old
                .as_ref()
                .map_or_else(PathBuf::new, |d| d.config_dir().to_path_buf()),
            old_data: old
                .as_ref()
                .map_or_else(PathBuf::new, |d| d.data_local_dir().to_path_buf()),
            new_config: Config::config_dir(),
            new_data: Config::data_dir(),
            old_videos: videos.as_ref().map(|v| v.join(OLD_NAME)),
            new_videos: videos.map(|v| v.join(crate::brand::APP_NAME)),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Já migrado, nada do antigo, ou outra instância está migrando.
    Nothing,
    /// Migrou (`clips` = clipes no banco ajustados).
    Done { clips: usize },
}

/// Roda a migração se ainda não foi feita. Erro = nada do antigo foi apagado.
pub fn run(paths: &Paths, host: &mut dyn Host) -> Result<Outcome, String> {
    let marker = paths.new_data.join(MARKER);
    if marker.exists() || (!paths.old_config.is_dir() && !paths.old_data.is_dir()) {
        return Ok(Outcome::Nothing);
    }
    fs::create_dir_all(&paths.new_data).map_err(|e| format!("criar {:?}: {e}", paths.new_data))?;
    let Some(_lock) = Lock::take(&paths.new_data.join(LOCK)) else {
        return Ok(Outcome::Nothing);
    };
    host.stop_old_recorder();

    // 1. Copia e confere (sobrescreve o que o app novo já tenha criado: logs, config padrão).
    copy_tree(&paths.old_config, &paths.new_config)?;
    copy_tree(&paths.old_data, &paths.new_data)?;

    host.move_token()?;
    if let Err(e) = host.drop_old_autostart() {
        // Não impede o resto: o gravador novo registra o dele.
        tracing::warn!("autostart antigo: {e}");
    }

    // 2. Config: server_url antigo → novo; pasta de vídeos só se for a padrão.
    let cfg_path = paths.new_config.join("config.toml");
    let mut default_dir = true;
    if cfg_path.exists() {
        let mut cfg = Config::load_or_create(&cfg_path).map_err(|e| e.to_string())?;
        if cfg.server_url.contains(OLD_HOST) {
            cfg.server_url = cfg.server_url.replace(OLD_HOST, NEW_HOST);
            cfg.save(&cfg_path).map_err(|e| e.to_string())?;
        }
        default_dir = cfg.output_dir.as_os_str().is_empty();
    }
    let videos = match (&paths.old_videos, &paths.new_videos) {
        (Some(o), Some(n)) if default_dir => {
            move_videos(o, n)?;
            Some((o.clone(), n.clone()))
        }
        _ => None,
    };

    // 3. Banco: caminhos e link publicado. Os caminhos de vídeo trocam mesmo que a pasta
    // já tenha sido movida numa tentativa anterior que falhou depois.
    let db = paths.new_data.join("library.db");
    let clips = if db.exists() {
        fix_library(&db, paths, videos.as_ref())?
    } else {
        0
    };

    fs::write(&marker, b"").map_err(|e| format!("marcador: {e}"))?;

    // 4. Só agora apaga o antigo (falha aqui não desfaz nada).
    for dir in [&paths.old_config, &paths.old_data] {
        if dir.is_dir() {
            if let Err(e) = fs::remove_dir_all(dir) {
                tracing::warn!("apagar {dir:?}: {e}");
            }
        }
    }
    // `ProjectDirs` no Windows usa `<Nome>\config` e `<Nome>\data`: some com a pasta-mãe vazia.
    for dir in [&paths.old_config, &paths.old_data] {
        if let Some(parent) = dir.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
    Ok(Outcome::Done { clips })
}

/// Lock de arquivo entre o gravador e a UI (os dois podem abrir juntos).
struct Lock(PathBuf);

impl Lock {
    fn take(path: &Path) -> Option<Self> {
        for _ in 0..2 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(_) => return Some(Self(path.to_path_buf())),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok())
                        .is_some_and(|age| age > LOCK_STALE);
                    if !stale {
                        return None;
                    }
                    let _ = fs::remove_file(path);
                }
                Err(_) => return None,
            }
        }
        None
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Copia `from` inteiro para `to` e confere o tamanho de cada arquivo.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    if !from.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(to).map_err(|e| format!("criar {to:?}: {e}"))?;
    for entry in fs::read_dir(from).map_err(|e| format!("ler {from:?}: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() && entry.file_name() == "logs" {
            // Logs: o do dia pode estar aberto pelo app novo; copiar o que der.
            if let Err(e) = copy_tree(&src, &dst) {
                tracing::warn!("logs antigos: {e}");
            }
        } else if kind.is_dir() {
            copy_tree(&src, &dst)?;
        } else if kind.is_file() {
            // O lock da migração nova não vem do antigo; o marcador também não.
            if entry.file_name() == LOCK || entry.file_name() == MARKER {
                continue;
            }
            let n = fs::copy(&src, &dst).map_err(|e| format!("copiar {src:?}: {e}"))?;
            let want = fs::metadata(&src).map_err(|e| e.to_string())?.len();
            if n != want || fs::metadata(&dst).map(|m| m.len()).ok() != Some(want) {
                return Err(format!("cópia incompleta de {src:?}"));
            }
        }
    }
    Ok(())
}

/// Move a pasta de vídeos padrão (se ainda existir).
fn move_videos(old: &Path, new: &Path) -> Result<(), String> {
    if !old.is_dir() {
        return Ok(());
    }
    let empty_new = !new.exists()
        || fs::read_dir(new)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
    if empty_new {
        let _ = fs::remove_dir(new);
        if fs::rename(old, new).is_ok() {
            return Ok(());
        }
    }
    // Outro volume, ou a pasta nova já tem clipes: arquivo por arquivo.
    copy_tree(old, new)?;
    fs::remove_dir_all(old).map_err(|e| format!("apagar {old:?}: {e}"))
}

/// Troca o começo de um caminho guardado como texto.
fn reprefix(path: &str, old: &Path, new: &Path) -> Option<String> {
    let old = old.to_string_lossy();
    if old.is_empty() {
        return None;
    }
    let rest = path.strip_prefix(old.as_ref())?;
    // Só no limite de um componente (não confundir `Clipe` com `Clipes`).
    if !(rest.is_empty() || rest.starts_with(['\\', '/'])) {
        return None;
    }
    Some(format!("{}{rest}", new.to_string_lossy()))
}

/// Ajusta `path`, `thumb_path` e `published_url` numa transação.
fn fix_library(
    db: &Path,
    paths: &Paths,
    videos: Option<&(PathBuf, PathBuf)>,
) -> Result<usize, String> {
    let mut conn = rusqlite::Connection::open(db).map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, Option<String>, Option<String>)> = {
        let mut st = tx
            .prepare("SELECT id, path, thumb_path, published_url FROM clips")
            .map_err(|e| e.to_string())?;
        let it = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(|e| e.to_string())?;
        it.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    let mut changed = 0;
    for (id, path, thumb, url) in rows {
        let new_path = videos
            .and_then(|(o, n)| reprefix(&path, o, n))
            .unwrap_or_else(|| path.clone());
        let new_thumb = thumb.as_deref().map(|t| {
            reprefix(t, &paths.old_data, &paths.new_data).unwrap_or_else(|| t.to_string())
        });
        let new_url = url.as_deref().map(|u| u.replace(OLD_HOST, NEW_HOST));
        if new_path != path || new_thumb != thumb || new_url != url {
            tx.execute(
                "UPDATE clips SET path = ?1, thumb_path = ?2, published_url = ?3 WHERE id = ?4",
                rusqlite::params![new_path, new_thumb, new_url, id],
            )
            .map_err(|e| e.to_string())?;
            changed += 1;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::{run, Host, Outcome, Paths};
    use std::fs;
    use std::path::PathBuf;

    #[derive(Default)]
    struct Fake {
        stopped: bool,
        token: bool,
        autostart: bool,
        fail_token: bool,
    }

    impl Host for Fake {
        fn stop_old_recorder(&mut self) {
            self.stopped = true;
        }
        fn move_token(&mut self) -> Result<(), String> {
            if self.fail_token {
                return Err("falhou".into());
            }
            self.token = true;
            Ok(())
        }
        fn drop_old_autostart(&mut self) -> Result<(), String> {
            self.autostart = true;
            Ok(())
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("migrate-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn paths(root: &std::path::Path) -> Paths {
        Paths {
            old_config: root.join("roaming/Clipe/config"),
            old_data: root.join("local/Clipe/data"),
            new_config: root.join("roaming/Glint/config"),
            new_data: root.join("local/Glint/data"),
            old_videos: Some(root.join("Videos/Clipe")),
            new_videos: Some(root.join("Videos/Glint")),
        }
    }

    fn seed(p: &Paths) {
        fs::create_dir_all(&p.old_config).unwrap();
        fs::create_dir_all(p.old_data.join("thumbs")).unwrap();
        fs::create_dir_all(p.old_videos.as_ref().unwrap()).unwrap();
        fs::write(
            p.old_config.join("config.toml"),
            "server_url = \"https://clipe.bpsstudio.workers.dev\"\n",
        )
        .unwrap();
        let clip = p.old_videos.as_ref().unwrap().join("Clipe_X_1.mp4");
        fs::write(&clip, b"mp4").unwrap();
        let thumb = p.old_data.join("thumbs").join("1.jpg");
        fs::write(&thumb, b"jpg").unwrap();
        let lib = crate::library::Library::open(&p.old_data.join("library.db")).unwrap();
        drop(lib);
        let conn = rusqlite::Connection::open(p.old_data.join("library.db")).unwrap();
        conn.execute(
            "INSERT INTO clips (path, created_ms, duration_ms, size_bytes, width, height, fps, codec,
             audio_tracks, thumb_path, favorite, published_url)
             VALUES (?1, 1, 1000, 3, 1920, 1080, 60, 'hevc', 2, ?2, 1, ?3)",
            rusqlite::params![
                clip.to_string_lossy(),
                thumb.to_string_lossy(),
                "https://clipe.bpsstudio.workers.dev/c/abc"
            ],
        )
        .unwrap();
    }

    #[test]
    fn sem_dados_antigos_nao_faz_nada() {
        let root = tmp("vazio");
        let p = paths(&root);
        let mut h = Fake::default();
        assert_eq!(run(&p, &mut h), Ok(Outcome::Nothing));
        assert!(!h.stopped);
        assert!(!p.new_data.exists());
    }

    #[test]
    fn migra_tudo_e_apaga_o_antigo() {
        let root = tmp("tudo");
        let p = paths(&root);
        seed(&p);
        let mut h = Fake::default();
        assert_eq!(run(&p, &mut h), Ok(Outcome::Done { clips: 1 }));
        assert!(h.stopped && h.token && h.autostart);

        let cfg = fs::read_to_string(p.new_config.join("config.toml")).unwrap();
        assert!(cfg.contains("clips.getglint.workers.dev"), "{cfg}");
        let new_clip = p.new_videos.as_ref().unwrap().join("Clipe_X_1.mp4");
        assert!(new_clip.exists(), "o arquivo mantém o nome antigo");
        let lib = crate::library::Library::open(&p.new_data.join("library.db")).unwrap();
        let c = &lib.list().unwrap()[0];
        assert_eq!(c.path, new_clip);
        assert_eq!(
            c.thumb_path.as_deref(),
            Some(p.new_data.join("thumbs").join("1.jpg").as_path())
        );
        assert_eq!(
            c.published_url.as_deref(),
            Some("https://clips.getglint.workers.dev/c/abc")
        );
        assert!(c.favorite);
        drop(lib);

        assert!(!p.old_config.exists() && !p.old_data.exists());
        assert!(!p.old_videos.as_ref().unwrap().exists());
        // Segunda vez: marcador → nada.
        let mut h2 = Fake::default();
        assert_eq!(run(&p, &mut h2), Ok(Outcome::Nothing));
        assert!(!h2.stopped);
    }

    #[test]
    fn erro_mantem_o_antigo_e_tenta_de_novo() {
        let root = tmp("erro");
        let p = paths(&root);
        seed(&p);
        let mut h = Fake {
            fail_token: true,
            ..Fake::default()
        };
        assert!(run(&p, &mut h).is_err());
        assert!(p.old_config.join("config.toml").exists());
        assert!(p.old_data.join("library.db").exists());
        assert!(!p.new_data.join(super::MARKER).exists());
        assert!(!p.new_data.join(super::LOCK).exists(), "lock solto no erro");
        // Simula uma tentativa anterior que já tinha movido os vídeos antes de falhar.
        fs::rename(
            p.old_videos.as_ref().unwrap(),
            p.new_videos.as_ref().unwrap(),
        )
        .unwrap();
        let mut ok = Fake::default();
        assert!(matches!(run(&p, &mut ok), Ok(Outcome::Done { .. })));
        let lib = crate::library::Library::open(&p.new_data.join("library.db")).unwrap();
        assert_eq!(
            lib.list().unwrap()[0].path,
            p.new_videos.as_ref().unwrap().join("Clipe_X_1.mp4")
        );
    }

    #[test]
    fn pasta_de_video_personalizada_fica() {
        let root = tmp("custom");
        let p = paths(&root);
        seed(&p);
        let custom = root.join("MeusClipes");
        fs::write(
            p.old_config.join("config.toml"),
            format!("output_dir = {:?}\n", custom.to_string_lossy()),
        )
        .unwrap();
        let mut h = Fake::default();
        run(&p, &mut h).unwrap();
        assert!(p
            .old_videos
            .as_ref()
            .unwrap()
            .join("Clipe_X_1.mp4")
            .exists());
        assert!(!p.new_videos.as_ref().unwrap().exists());
    }

    #[test]
    fn prefixo_so_no_limite_do_componente() {
        let o = std::path::Path::new("C:\\V\\Clipe");
        let n = std::path::Path::new("C:\\V\\Glint");
        assert_eq!(
            super::reprefix("C:\\V\\Clipe\\a.mp4", o, n).as_deref(),
            Some("C:\\V\\Glint\\a.mp4")
        );
        assert_eq!(super::reprefix("C:\\V\\Clipes\\a.mp4", o, n), None);
    }
}
