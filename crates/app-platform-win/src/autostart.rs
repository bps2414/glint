//! "Iniciar com o Windows": valor em `HKCU\...\CurrentVersion\Run` (sem admin).

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

use crate::{Error, Result};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Comando registrado hoje para `name`, se houver.
pub fn current(name: &str) -> Option<String> {
    let key = HSTRING::from(RUN_KEY);
    let value = HSTRING::from(name);
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &value,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// Liga (com `command`, ex.: `"C:\...\clipe-recorder.exe" --autostart`) ou desliga.
pub fn set(name: &str, command: Option<&str>) -> Result<()> {
    let key = HSTRING::from(RUN_KEY);
    let value = HSTRING::from(name);
    let r = unsafe {
        match command {
            Some(cmd) => {
                let wide: Vec<u16> = cmd.encode_utf16().chain(Some(0)).collect();
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    &key,
                    &value,
                    REG_SZ.0,
                    Some(wide.as_ptr() as *const _),
                    (wide.len() * 2) as u32,
                )
            }
            None => {
                let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, &key, PCWSTR(value.as_ptr()));
                if r.0 == 2 {
                    ERROR_SUCCESS // ERROR_FILE_NOT_FOUND: já estava desligado
                } else {
                    r
                }
            }
        }
    };
    if r == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(Error::Other(format!("registro (Run): erro {}", r.0)))
    }
}
