//! Aparência do sistema que o app respeita: "Efeitos de transparência", economia de
//! bateria e tema da barra de tarefas.

use windows::core::HSTRING;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

fn dword(key: &str, value: &str) -> Option<u32> {
    let mut data = 0u32;
    let mut size = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    (r == ERROR_SUCCESS).then_some(data)
}

/// "Efeitos de transparência" ligado (Configurações → Personalização → Cores).
/// Sem o valor no registro, o padrão do Windows é ligado.
pub fn transparency_enabled() -> bool {
    dword(PERSONALIZE, "EnableTransparency").is_none_or(|v| v != 0)
}

/// Economia de bateria ligada.
pub fn battery_saver_on() -> bool {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s) }.is_ok() && s.SystemStatusFlag == 1
}

/// Barra de tarefas/bandeja no tema claro (ícone da bandeja preto em vez de branco).
pub fn taskbar_light() -> bool {
    dword(PERSONALIZE, "SystemUsesLightTheme").is_some_and(|v| v != 0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn le_sem_falhar() {
        // Só confere que as chamadas funcionam nesta máquina.
        let _ = super::transparency_enabled();
        let _ = super::battery_saver_on();
        let _ = super::taskbar_light();
    }
}
