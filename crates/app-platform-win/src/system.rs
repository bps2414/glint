//! Versão do Windows (para o bug report), ex.: `Windows 11 24H2 (26100)`.

use windows::core::HSTRING;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

fn value(name: &str) -> Option<String> {
    let key = HSTRING::from(KEY);
    let value = HSTRING::from(name);
    let mut buf = vec![0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
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

pub fn os_version() -> String {
    let build = value("CurrentBuild").unwrap_or_default();
    // O ProductName continua "Windows 10" no 11; o build separa os dois.
    let name = if build.parse::<u32>().is_ok_and(|b| b >= 22000) {
        "Windows 11"
    } else {
        "Windows 10"
    };
    match value("DisplayVersion") {
        Some(v) => format!("{name} {v} ({build})"),
        None => format!("{name} ({build})"),
    }
}
