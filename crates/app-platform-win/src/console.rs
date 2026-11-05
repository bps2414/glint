//! O gravador é um app "de janela" (sem console piscando ao iniciar com o
//! Windows). Os modos de linha de comando (`--diagnose`, `--list-windows`,
//! gravação manual) pegam o console de quem chamou (cmd/.bat) ou abrem um.

use windows::core::w;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Console::{
    AllocConsole, AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS,
    STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};

fn missing(which: STD_HANDLE) -> bool {
    unsafe {
        match GetStdHandle(which) {
            Ok(h) => h.is_invalid() || h.0.is_null(),
            Err(_) => true,
        }
    }
}

fn open(name: windows::core::PCWSTR, access: u32) -> Option<HANDLE> {
    unsafe {
        CreateFileW(
            name,
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .ok()
    }
}

/// Garante um console para `println!`/`stdin` (não mexe em saídas redirecionadas).
pub fn ensure() {
    unsafe {
        if !missing(STD_OUTPUT_HANDLE) {
            return; // já tem console ou saída redirecionada
        }
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            let _ = AllocConsole();
        }
        if missing(STD_OUTPUT_HANDLE) || missing(STD_ERROR_HANDLE) {
            if let Some(out) = open(w!("CONOUT$"), (GENERIC_READ | GENERIC_WRITE).0) {
                if missing(STD_OUTPUT_HANDLE) {
                    let _ = SetStdHandle(STD_OUTPUT_HANDLE, out);
                }
                if missing(STD_ERROR_HANDLE) {
                    let _ = SetStdHandle(STD_ERROR_HANDLE, out);
                }
            }
        }
        if missing(STD_INPUT_HANDLE) {
            if let Some(inp) = open(w!("CONIN$"), (GENERIC_READ | GENERIC_WRITE).0) {
                let _ = SetStdHandle(STD_INPUT_HANDLE, inp);
            }
        }
    }
}
