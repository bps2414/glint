//! Instância única do gravador (mutex nomeado por sessão de usuário).

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

/// Enquanto viver, outras instâncias com o mesmo nome veem `None` em [`acquire`].
pub struct InstanceLock(HANDLE);

unsafe impl Send for InstanceLock {}

/// `None` se outra instância já está rodando nesta sessão do Windows.
pub fn acquire(name: &str) -> Option<InstanceLock> {
    let wide: Vec<u16> = format!("Local\\{name}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let h = CreateMutexW(None, true, PCWSTR(wide.as_ptr())).ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(h);
            return None;
        }
        Some(InstanceLock(h))
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
