//! `app-recorder`: gravador headless do app (nome em `brand`).
//!
//! Sem argumentos: fica na bandeja, grava sozinho quando um jogo abre e salva
//! um clipe na hotkey (F8). Com argumentos: modos de console para teste
//! (`--monitor`, `--window`, `--diagnose`, `--status`...). Ver `args::USAGE`.

// Sem console piscando ao iniciar com o Windows; os modos de linha de
// comando pegam o console de quem chamou (ver `console::ensure`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod args;
#[cfg(windows)]
mod cli;
#[cfg(windows)]
mod daemon;
mod plan;
#[cfg(windows)]
mod presence;
#[cfg(windows)]
mod saver;
#[cfg(windows)]
mod session;

#[cfg(windows)]
rust_i18n::i18n!("locales", fallback = "pt-BR");

use std::path::PathBuf;

use app_core::{brand, Config};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let log_dir: PathBuf = Config::data_dir().join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("recorder")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&log_dir)
        .ok()?;
    let (file, guard) = tracing_appender::non_blocking(file);
    let filter =
        EnvFilter::try_from_env(brand::log_env_var()).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .with(fmt::layer().with_ansi(false).with_writer(file))
        .init();
    tracing::info!(logs = %log_dir.display(), "{} recorder {}", brand::APP_NAME, env!("CARGO_PKG_VERSION"));
    Some(guard)
}

fn main() {
    let parsed = args::Args::parse(std::env::args().skip(1));
    let mode = parsed.as_ref().map(|a| a.mode()).ok();

    #[cfg(windows)]
    if mode != Some(args::Mode::Daemon) {
        app_platform_win::console::ensure();
    }

    let args = match parsed {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}\n\n{}", args::USAGE);
            std::process::exit(2);
        }
    };
    if mode == Some(args::Mode::Help) {
        println!("{}", args::USAGE);
        return;
    }
    let _guard = init_logging();

    #[cfg(windows)]
    {
        rust_i18n::set_locale(app_core::ui_locale(sys_locale::get_locale().as_deref()));
        let r = match args.mode() {
            args::Mode::Daemon => daemon::run(args.autostart),
            args::Mode::Manual => cli::manual(&args),
            args::Mode::Diagnose => cli::diagnose(&args),
            args::Mode::MeasureFps => cli::measure_fps(&args),
            args::Mode::ListWindows => {
                cli::list_windows();
                Ok(())
            }
            args::Mode::Ipc => cli::ipc(&args),
            args::Mode::Help => Ok(()),
        };
        if let Err(e) = r {
            tracing::error!("{e:#}");
            if args.mode() != args::Mode::Daemon {
                eprintln!(
                    "\nERRO: {e:#}\n(Detalhes no log em {})",
                    Config::data_dir().join("logs").display()
                );
                if !matches!(args.mode(), args::Mode::Ipc | args::Mode::MeasureFps) {
                    eprintln!("Pressione Enter para sair...");
                    let _ = std::io::stdin().read_line(&mut String::new());
                }
            }
            std::process::exit(1);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        eprintln!("o gravador só funciona no Windows por enquanto.");
        std::process::exit(1);
    }
}
