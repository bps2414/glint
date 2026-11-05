//! Detecção automática de jogo: regras e máquina de estados, sem código de SO.
//!
//! A plataforma observa a janela em primeiro plano a cada *tick* (~1 s) e
//! entrega um [`ForegroundSample`]; o [`GameDetector`] decide quando ligar e
//! desligar o buffer:
//!
//! - executável na lista de exclusão → nunca é jogo;
//! - executável na lista de inclusão → é jogo assim que fica em foco (mesmo em janela);
//! - senão, é candidato se a janela cobre o monitor inteiro (tela cheia ou sem
//!   borda) **e** usa a GPU (quando a plataforma sabe medir); precisa ficar
//!   assim por `confirm_ticks` ticks seguidos, para não disparar com um vídeo
//!   em tela cheia por um instante;
//! - gravando, só para quando o processo do jogo termina (alt-tab não para).

use crate::brand;
use crate::config::GameConfig;

/// Executáveis que ficam em tela cheia mas não são jogos (navegadores,
/// launchers, players, o próprio app...). Comparação sem maiúsculas.
pub const DEFAULT_EXCLUDE: &[&str] = &[
    // Sistema
    "explorer.exe",
    "applicationframehost.exe",
    "searchhost.exe",
    "searchapp.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "textinputhost.exe",
    "lockapp.exe",
    "logonui.exe",
    "taskmgr.exe",
    "mmc.exe",
    "systemsettings.exe",
    "dwm.exe",
    // Navegadores
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "opera.exe",
    "opera_gx.exe",
    "brave.exe",
    "vivaldi.exe",
    // Comunicação e mídia
    "discord.exe",
    "spotify.exe",
    "vlc.exe",
    "mpc-hc64.exe",
    "mpc-be64.exe",
    "potplayermini64.exe",
    "wmplayer.exe",
    "video.ui.exe",
    "netflix.exe",
    "teams.exe",
    "ms-teams.exe",
    "zoom.exe",
    "whatsapp.exe",
    "telegram.exe",
    // Launchers e lojas
    "steam.exe",
    "steamwebhelper.exe",
    "epicgameslauncher.exe",
    "riotclientux.exe",
    "riotclientservices.exe",
    "battle.net.exe",
    "eadesktop.exe",
    "ubisoftconnect.exe",
    "galaxyclient.exe",
    "xboxpcapp.exe",
    "gamebar.exe",
    // Gravação e streaming
    "obs64.exe",
    "medal.exe",
    "streamlabs obs.exe",
    // Trabalho
    "code.exe",
    "devenv.exe",
    "powerpnt.exe",
    "winword.exe",
    "excel.exe",
    "acrobat.exe",
    "acrord32.exe",
];

/// O que a plataforma viu na janela em primeiro plano neste tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForegroundSample {
    pub pid: u32,
    /// Nome do executável (só o arquivo, ex.: `VALORANT-Win64-Shipping.exe`).
    pub exe: String,
    pub title: String,
    /// A janela cobre o monitor inteiro (tela cheia exclusiva ou sem borda).
    pub fullscreen: bool,
    /// O processo está usando a GPU 3D. `None` = a plataforma não sabe medir.
    pub gpu_active: Option<bool>,
}

/// Jogo reconhecido.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameInfo {
    pub pid: u32,
    pub exe: String,
    /// Nome para mostrar e para o nome do arquivo (título da janela ou o executável).
    pub name: String,
}

impl GameInfo {
    pub fn from_sample(s: &ForegroundSample) -> Self {
        Self {
            pid: s.pid,
            exe: s.exe.clone(),
            name: display_name(&s.exe, &s.title),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetectorEvent {
    /// Começar a gravar este jogo.
    Start(GameInfo),
    /// O jogo terminou: parar o buffer.
    Stop(GameInfo),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Na lista de exclusão (ou é o próprio app).
    Excluded,
    /// Na lista de inclusão: grava em janela também.
    Included,
    /// Parece jogo (tela cheia + GPU); precisa de confirmação.
    Candidate,
    NotGame,
}

/// Nome curto para mostrar: título da janela (se razoável) ou o executável sem `.exe`.
pub fn display_name(exe: &str, title: &str) -> String {
    let title = title.trim();
    if !title.is_empty() && title.chars().count() <= 60 {
        return title.to_string();
    }
    let stem = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    let stem = stem
        .strip_suffix(".exe")
        .or_else(|| stem.strip_suffix(".EXE"))
        .unwrap_or(stem);
    if stem.is_empty() {
        "Jogo".to_string()
    } else {
        stem.to_string()
    }
}

fn normalize_exe(name: &str) -> String {
    let name = name.trim();
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let mut n = file.to_lowercase();
    if !n.is_empty() && !n.ends_with(".exe") {
        n.push_str(".exe");
    }
    n
}

#[derive(Clone, Debug)]
struct Rules {
    include: Vec<String>,
    exclude: Vec<String>,
    confirm_ticks: u32,
}

impl Rules {
    fn from_config(cfg: &GameConfig) -> Self {
        let mut exclude: Vec<String> = DEFAULT_EXCLUDE.iter().map(|s| s.to_string()).collect();
        exclude.push(format!("{}.exe", brand::APP_SLUG));
        exclude.push(format!("{}-recorder.exe", brand::APP_SLUG));
        // Nome antigo (Fase 5: Clipe → Glint); o gravador antigo pode estar aberto.
        // Tirar depois de uma versão.
        exclude.push("clipe.exe".into());
        exclude.push("clipe-recorder.exe".into());
        exclude.extend(cfg.exclude.iter().map(|s| normalize_exe(s)));
        let include: Vec<String> = cfg.include.iter().map(|s| normalize_exe(s)).collect();
        // Quem está nas duas listas fica incluído (a escolha do usuário vence a interna).
        exclude.retain(|e| !include.contains(e));
        Self {
            include,
            exclude,
            confirm_ticks: cfg.confirm_secs.max(1),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Idle {
        /// pid candidato e quantos ticks seguidos ele pareceu jogo.
        candidate: Option<(u32, u32)>,
    },
    Playing(GameInfo),
}

#[derive(Clone, Debug)]
pub struct GameDetector {
    rules: Rules,
    state: State,
}

impl GameDetector {
    pub fn new(cfg: &GameConfig) -> Self {
        Self {
            rules: Rules::from_config(cfg),
            state: State::Idle { candidate: None },
        }
    }

    /// Atualiza as listas (config recarregada) sem perder o estado.
    pub fn set_config(&mut self, cfg: &GameConfig) {
        self.rules = Rules::from_config(cfg);
    }

    pub fn current(&self) -> Option<&GameInfo> {
        match &self.state {
            State::Playing(g) => Some(g),
            State::Idle { .. } => None,
        }
    }

    /// Esquece o jogo atual/candidato (ex.: sessão falhou ou detecção desligada).
    pub fn reset(&mut self) {
        self.state = State::Idle { candidate: None };
    }

    pub fn is_excluded(&self, exe: &str) -> bool {
        self.rules.exclude.contains(&normalize_exe(exe))
    }

    pub fn is_included(&self, exe: &str) -> bool {
        self.rules.include.contains(&normalize_exe(exe))
    }

    /// Vale a pena medir a GPU deste processo? (medir custa; só quando decide algo).
    pub fn wants_gpu_check(&self, exe: &str, fullscreen: bool) -> bool {
        fullscreen && self.current().is_none() && !self.is_excluded(exe) && !self.is_included(exe)
    }

    pub fn classify(&self, s: &ForegroundSample) -> Verdict {
        if self.is_excluded(&s.exe) {
            Verdict::Excluded
        } else if self.is_included(&s.exe) {
            Verdict::Included
        } else if s.fullscreen && s.gpu_active != Some(false) {
            Verdict::Candidate
        } else {
            Verdict::NotGame
        }
    }

    /// Um passo da máquina de estados.
    ///
    /// - `fg`: janela em primeiro plano (`None` se não há/não deu para ler).
    /// - `current_alive`: o processo do jogo atual ainda existe (ignorado se ocioso).
    pub fn tick(
        &mut self,
        fg: Option<&ForegroundSample>,
        current_alive: bool,
    ) -> Option<DetectorEvent> {
        if let State::Playing(game) = &self.state {
            if current_alive {
                return None;
            }
            let game = game.clone();
            self.state = State::Idle { candidate: None };
            return Some(DetectorEvent::Stop(game));
        }
        let needed = fg.and_then(|s| match self.classify(s) {
            Verdict::Included => Some(1),
            Verdict::Candidate => Some(self.rules.confirm_ticks),
            Verdict::Excluded | Verdict::NotGame => None,
        });
        let (Some(s), Some(needed)) = (fg, needed) else {
            self.state = State::Idle { candidate: None };
            return None;
        };
        let count = match self.state {
            State::Idle {
                candidate: Some((pid, n)),
            } if pid == s.pid => n + 1,
            _ => 1,
        };
        if count >= needed {
            let game = GameInfo::from_sample(s);
            self.state = State::Playing(game.clone());
            Some(DetectorEvent::Start(game))
        } else {
            self.state = State::Idle {
                candidate: Some((s.pid, count)),
            };
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: u32, exe: &str, fullscreen: bool, gpu: Option<bool>) -> ForegroundSample {
        ForegroundSample {
            pid,
            exe: exe.into(),
            title: "VALORANT  ".into(),
            fullscreen,
            gpu_active: gpu,
        }
    }

    fn det() -> GameDetector {
        GameDetector::new(&GameConfig::default())
    }

    #[test]
    fn fullscreen_gpu_game_starts_after_confirmation() {
        let mut d = det();
        let s = sample(10, "VALORANT-Win64-Shipping.exe", true, Some(true));
        assert_eq!(d.tick(Some(&s), false), None);
        assert_eq!(d.tick(Some(&s), false), None);
        match d.tick(Some(&s), false) {
            Some(DetectorEvent::Start(g)) => {
                assert_eq!(g.pid, 10);
                assert_eq!(g.name, "VALORANT");
            }
            o => panic!("esperava Start, veio {o:?}"),
        }
        // Alt-tab para outra coisa não para.
        let other = sample(20, "chrome.exe", true, Some(true));
        assert_eq!(d.tick(Some(&other), true), None);
        assert_eq!(d.tick(None, true), None);
        // Processo fechou → para.
        assert!(matches!(
            d.tick(Some(&other), false),
            Some(DetectorEvent::Stop(g)) if g.pid == 10
        ));
        assert!(d.current().is_none());
    }

    #[test]
    fn interrupted_candidate_restarts_count() {
        let mut d = det();
        let game = sample(10, "game.exe", true, Some(true));
        let desk = sample(1, "notepad.exe", false, Some(false));
        d.tick(Some(&game), false);
        d.tick(Some(&game), false);
        d.tick(Some(&desk), false);
        assert_eq!(d.tick(Some(&game), false), None);
        assert_eq!(d.tick(Some(&game), false), None);
        assert!(matches!(
            d.tick(Some(&game), false),
            Some(DetectorEvent::Start(_))
        ));
    }

    #[test]
    fn excluded_and_windowed_never_start() {
        let mut d = det();
        let browser = sample(5, "Chrome.EXE", true, Some(true));
        let windowed = sample(6, "game.exe", false, Some(true));
        let no_gpu = sample(7, "slides.exe", true, Some(false));
        let own = sample(8, &format!("{}.exe", brand::APP_SLUG), true, Some(true));
        for _ in 0..10 {
            for s in [&browser, &windowed, &no_gpu, &own] {
                assert_eq!(d.tick(Some(s), false), None, "{}", s.exe);
            }
        }
    }

    #[test]
    fn unknown_gpu_falls_back_to_fullscreen() {
        let d = det();
        let s = sample(10, "game.exe", true, None);
        assert_eq!(d.classify(&s), Verdict::Candidate);
    }

    #[test]
    fn include_list_wins_and_needs_no_fullscreen() {
        let cfg = GameConfig {
            include: vec!["Minecraft".into(), r"C:\Games\chrome.exe".into()],
            exclude: vec!["game.exe".into()],
            ..GameConfig::default()
        };
        let mut d = GameDetector::new(&cfg);
        let mc = sample(3, "minecraft.exe", false, Some(false));
        assert!(matches!(
            d.tick(Some(&mc), false),
            Some(DetectorEvent::Start(_))
        ));
        d.reset();
        // Usuário incluiu algo da lista interna: inclusão vence.
        assert_eq!(
            d.classify(&sample(4, "chrome.exe", false, None)),
            Verdict::Included
        );
        assert_eq!(
            d.classify(&sample(4, "GAME.exe", true, Some(true))),
            Verdict::Excluded
        );
    }

    #[test]
    fn gpu_check_only_when_useful() {
        let d = det();
        assert!(d.wants_gpu_check("game.exe", true));
        assert!(!d.wants_gpu_check("game.exe", false));
        assert!(!d.wants_gpu_check("chrome.exe", true));
    }

    #[test]
    fn display_names() {
        assert_eq!(
            display_name("x.exe", "  Counter-Strike 2 "),
            "Counter-Strike 2"
        );
        assert_eq!(display_name("C:\\G\\RocketLeague.exe", ""), "RocketLeague");
        assert_eq!(display_name("", ""), "Jogo");
        let long = "a".repeat(80);
        assert_eq!(display_name("cs2.exe", &long), "cs2");
    }
}
