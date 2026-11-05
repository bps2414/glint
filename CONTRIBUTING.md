# Contributing

Thanks for helping. Glint is a small project; issues and pull requests are welcome. For anything big, open an
issue first so we can agree on the approach.

## Build

Windows 10/11 is the target. You need Rust (MSVC toolchain, 1.85+), Visual Studio Build Tools with C++,
LLVM (libclang) and an LGPL **shared** FFmpeg build. `scripts\build-windows.ps1` downloads FFmpeg to `.deps\`,
builds, tests and packs the app:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
```

The code also builds on Linux for development of the portable crates (`app-core`, `app-media`, `app-ipc`,
the UI) with system FFmpeg and `pkg-config`. Windows-only code is checked from Linux with the mingw
cross-check below.

## Before opening a pull request

```sh
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
scripts/check-windows.sh              # cross-check of the Windows code (CARGO_CMD=clippy for clippy)
```

If you touched `server/`:

```sh
cd server && npm ci && npm run typecheck && npm test
```

CI runs the same checks. Keep pull requests focused: one change, with tests when there is logic to test.

## Style

- **OS-specific code only in `crates/app-platform-*`.** `app-core`, `app-media` (public API) and the UI must
  not use Windows types, so a Linux backend stays possible.
- **UI text goes through the locale files** (`crates/app-ui/locales`, `server/src/i18n`), never hard-coded in
  the views. Texts and code comments in this repository are in Brazilian Portuguese; English contributions are
  fine for pull request descriptions and issues, and I will help with the translation.
- **Nothing inside the game.** No injection, no reading game memory, no in-game overlay. Changes that break this
  will not be accepted (anti-cheat safety is a core requirement).
- The brand name lives only in `crates/app-core/src/brand.rs`. Icons are generated: edit `docs/brand/*.svg`
  and run `cargo run -p make-icons`; do not edit `assets/brand/` by hand.
- Match the surrounding code; no new abstractions or dependencies without a reason.

## License

By contributing you agree that your work is licensed under MIT OR Apache-2.0, as in the README.
