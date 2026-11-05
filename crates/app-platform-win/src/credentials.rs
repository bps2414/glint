//! Segredos no Gerenciador de Credenciais do Windows (credencial "genérica" do
//! usuário atual; o Windows a protege com DPAPI).

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

use crate::Result;

/// Grava (ou troca) o segredo `secret` sob o nome `target`.
pub fn save(target: &str, user: &str, secret: &str) -> Result<()> {
    let mut target_w: Vec<u16> = target.encode_utf16().chain([0]).collect();
    let mut user_w: Vec<u16> = user.encode_utf16().chain([0]).collect();
    let mut blob = secret.as_bytes().to_vec();
    let cred = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target_w.as_mut_ptr()),
        UserName: PWSTR(user_w.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    unsafe { CredWriteW(&cred, 0)? };
    Ok(())
}

/// Segredo gravado sob `target`, se houver.
pub fn load(target: &str) -> Option<String> {
    let mut p: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe {
        CredReadW(&HSTRING::from(target), CRED_TYPE_GENERIC, None, &mut p).ok()?;
        let c = &*p;
        let bytes =
            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec();
        CredFree(p as *const _);
        String::from_utf8(bytes).ok()
    }
}

/// Apaga o segredo (não é erro se ele não existir).
pub fn delete(target: &str) -> Result<()> {
    match unsafe { CredDeleteW(&HSTRING::from(target), CRED_TYPE_GENERIC, None) } {
        Err(e) if e.code() == ERROR_NOT_FOUND.to_hresult() => Ok(()),
        r => Ok(r?),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn grava_le_e_apaga() {
        let t = format!("app-platform-win-teste-{}", std::process::id());
        super::save(&t, "eu", "segredo-ç").unwrap();
        assert_eq!(super::load(&t).as_deref(), Some("segredo-ç"));
        super::save(&t, "eu", "outro").unwrap();
        assert_eq!(super::load(&t).as_deref(), Some("outro"));
        super::delete(&t).unwrap();
        assert_eq!(super::load(&t), None);
        super::delete(&t).unwrap();
    }
}
