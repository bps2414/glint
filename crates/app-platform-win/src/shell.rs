//! Abrir pastas/arquivos no Explorer e iniciar a interface sem console.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// Abre uma pasta no Explorer.
pub fn open_folder(path: &Path) {
    if let Err(e) = Command::new("explorer.exe").arg(path).spawn() {
        tracing::warn!("não foi possível abrir {}: {e}", path.display());
    }
}

/// Inicia um executável desacoplado deste processo.
pub fn spawn_detached(exe: &Path, args: &[&str]) -> std::io::Result<()> {
    Command::new(exe)
        .args(args)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|_| ())
}
