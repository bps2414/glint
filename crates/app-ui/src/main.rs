//! Interface do app.
//!
//! Rodar: `cargo run -p app-ui`. Abre a biblioteca (clipes do `library.db` do
//! gravador) e mostra o status do gravador pelo IPC.
//!
//! `glint --player-test [arquivo.mp4] [--sw]`: modo de teste do player (spike da
//! Fase 3) com métricas na tela. Sem arquivo, abre o clipe mais recente da
//! biblioteca; `--sw` força decodificação por software (comparação); `--auto <arquivo.txt>`
//! roda o roteiro sozinho (janela, tela cheia, seeks), grava o relatório e fecha.

// No Windows, sem console atrás da janela em release.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod assets;
mod blur;
// Design system: nem toda peça é usada em todas as telas.
#[allow(dead_code, unused_imports)]
mod components;
mod data;
mod exporter;
mod format;
mod og;
mod player;
mod publisher;
mod recorder;
mod secret;
mod theme;
mod thumbs;
mod trim;
mod updater;
mod views;
mod watermark;
mod welcome;

use std::path::PathBuf;

use app_core::brand;
use app_media::HwPreference;
use gpui_kit::*;

rust_i18n::i18n!("locales", fallback = "pt-BR");

/// Texto traduzido.
pub fn tr(key: &str) -> SharedString {
    rust_i18n::t!(key).to_string().into()
}

/// Texto traduzido com parâmetros `%{nome}`.
pub fn tr_args(key: &str, args: &[(&str, &str)]) -> SharedString {
    let mut s = rust_i18n::t!(key).to_string();
    for (k, v) in args {
        s = s.replace(&format!("%{{{k}}}"), v);
    }
    s.into()
}

/// Modo de abertura escolhido pela linha de comando.
enum Mode {
    App,
    PlayerTest {
        path: Option<PathBuf>,
        hw: HwPreference,
        /// `--auto <relatório.txt>`: roteiro automático que grava o relatório e fecha.
        auto: Option<PathBuf>,
    },
    /// `--glass-test`: spike da janela de vidro (Fase 5).
    GlassTest,
}

fn parse_args(args: impl Iterator<Item = String>) -> Mode {
    let mut player_test = false;
    let mut glass_test = false;
    let mut hw = HwPreference::Auto;
    let mut path = None;
    let mut auto = None;
    let mut args = args.peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--player-test" => player_test = true,
            "--glass-test" => glass_test = true,
            "--sw" => hw = HwPreference::Off,
            "--auto" => auto = args.next().map(PathBuf::from),
            _ if !a.starts_with("--") && !a.is_empty() => path = Some(PathBuf::from(a)),
            _ => {}
        }
    }
    if glass_test {
        Mode::GlassTest
    } else if player_test {
        Mode::PlayerTest { path, hw, auto }
    } else {
        Mode::App
    }
}

/// Clipe mais recente da biblioteca do gravador (se houver).
fn newest_clip() -> Option<PathBuf> {
    let lib = app_core::library::Library::open(&app_core::Config::library_path()).ok()?;
    lib.list()
        .ok()?
        .into_iter()
        .map(|c| c.path)
        .find(|p| p.exists())
}

fn main() {
    rust_i18n::set_locale(app_core::ui_locale(sys_locale::get_locale().as_deref()));
    let mode = parse_args(std::env::args().skip(1));
    app_media::init_logging();
    if matches!(mode, Mode::App) {
        // Dados do nome antigo (Clipe → Glint), 1× (o gravador também tenta; um lock separa os dois).
        app_ipc::legacy::migrate();
    }

    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            if let Err(e) = theme::install(cx) {
                eprintln!("tema: {e:#}");
            }

            let (title, min) = match &mode {
                Mode::App => (
                    SharedString::from(brand::APP_NAME),
                    size(px(960.), px(640.)),
                ),
                Mode::PlayerTest { .. } => (
                    tr_args("player_test.window_title", &[("name", brand::APP_NAME)]),
                    size(px(640.), px(400.)),
                ),
                Mode::GlassTest => (
                    SharedString::from(brand::APP_NAME),
                    size(px(800.), px(520.)),
                ),
            };
            // App e teste de vidro: barra de título própria sobre o acrílico.
            let glass = !matches!(mode, Mode::PlayerTest { .. });
            let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(min),
                titlebar: Some(TitlebarOptions {
                    title: Some(title),
                    appears_transparent: glass,
                    ..Default::default()
                }),
                window_background: if glass {
                    WindowBackgroundAppearance::Blurred
                } else {
                    WindowBackgroundAppearance::Opaque
                },
                app_id: Some(brand::APP_SLUG.into()),
                ..Default::default()
            };
            match mode {
                Mode::App => gpui_kit::open_window(options, cx, |window, cx| {
                    theme::refresh(window, cx);
                    cx.new(|cx| app::Shell::new(window, cx))
                })
                .map(|_| ()),
                Mode::GlassTest => gpui_kit::open_window(options, cx, |window, cx| {
                    let look = theme::Look {
                        dark: true,
                        glass: true,
                    };
                    theme::apply(look, window, cx);
                    cx.new(|cx| views::glass_test::GlassTest::new(window, cx))
                })
                .map(|_| ()),
                Mode::PlayerTest { path, hw, auto } => {
                    let path = path.or_else(newest_clip);
                    gpui_kit::open_window(options, cx, move |window, cx| {
                        cx.new(|cx| views::player_test::PlayerTest::new(path, hw, auto, window, cx))
                    })
                    .map(|_| ())
                }
            }
            .expect("abrir janela");
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn textos_existem() {
        rust_i18n::set_locale("pt-BR");
        for key in [
            "nav.library",
            "library.summary",
            "recorder.waiting",
            "date.weekday.6",
            "date.month_short.12",
        ] {
            assert_ne!(super::tr(key).as_ref(), key, "falta tradução: {key}");
        }
        assert_eq!(
            super::tr_args("recorder.recording", &[("game", "X")]).as_ref(),
            "Gravando X"
        );
        assert_ne!(super::tr("player_test.fps").as_ref(), "player_test.fps");
    }

    #[test]
    fn argumentos_do_teste_do_player() {
        let args = |v: &[&str]| super::parse_args(v.iter().map(|s| s.to_string()));
        assert!(matches!(args(&[]), super::Mode::App));
        assert!(matches!(args(&["--glass-test"]), super::Mode::GlassTest));
        match args(&["--player-test", "C:\\clipes\\a.mp4", "--sw"]) {
            super::Mode::PlayerTest { path, hw, .. } => {
                assert_eq!(path.unwrap().to_str(), Some("C:\\clipes\\a.mp4"));
                assert_eq!(hw, app_media::HwPreference::Off);
            }
            _ => panic!("esperava o modo de teste"),
        }
        match args(&["--player-test"]) {
            super::Mode::PlayerTest { path, hw, .. } => {
                assert!(path.is_none());
                assert_eq!(hw, app_media::HwPreference::Auto);
            }
            _ => panic!("esperava o modo de teste"),
        }
    }
}
