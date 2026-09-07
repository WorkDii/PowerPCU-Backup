//! Secrets at rest: Windows DPAPI, machine scope (`CRYPTPROTECT_LOCAL_MACHINE`), so
//! a copied database file cannot be read elsewhere. Stored form: `dpapi:<base64>`.
//! Unprefixed values pass through `unprotect` unchanged (tests, hand-edited DBs).

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN,
    CRYPT_INTEGER_BLOB,
};

const PREFIX: &str = "dpapi:";

pub fn is_protected(s: &str) -> bool {
    s.starts_with(PREFIX)
}

pub fn protect(plain: &str) -> Result<String, String> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    let bytes = plain.as_bytes();
    let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: input points at a live slice; out is written by the API and freed below.
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err(format!("เข้ารหัสค่าลับไม่สำเร็จ (DPAPI {})", unsafe { GetLastError() }));
    }
    let data = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe { LocalFree(out.pbData as *mut _) };
    Ok(format!("{PREFIX}{}", B64.encode(data)))
}

pub fn unprotect(stored: &str) -> Result<String, String> {
    let Some(b64) = stored.strip_prefix(PREFIX) else {
        return Ok(stored.to_string());
    };
    let blob = B64.decode(b64).map_err(|_| "ถอดรหัสค่าลับไม่สำเร็จ (รูปแบบไม่ถูกต้อง)".to_string())?;
    let input = CRYPT_INTEGER_BLOB { cbData: blob.len() as u32, pbData: blob.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err(format!("ถอดรหัสค่าลับไม่สำเร็จ (DPAPI {}) — ค่านี้ถูกเข้ารหัสบนเครื่องอื่นหรือถูกแก้ไข", unsafe { GetLastError() }));
    }
    let data = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe { LocalFree(out.pbData as *mut _) };
    String::from_utf8(data).map_err(|_| "ถอดรหัสค่าลับไม่สำเร็จ (ไม่ใช่ข้อความ)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let stored = protect("p@ss ไทย").unwrap();
        assert!(stored.starts_with("dpapi:"));
        assert!(is_protected(&stored));
        assert_eq!(unprotect(&stored).unwrap(), "p@ss ไทย");
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(protect("").unwrap(), "");
        assert_eq!(unprotect("").unwrap(), "");
    }

    #[test]
    fn plain_passes_through() {
        assert!(!is_protected("plain"));
        assert_eq!(unprotect("plain").unwrap(), "plain");
    }

    #[test]
    fn tampered_blob_errors() {
        let err = unprotect("dpapi:AAAA").unwrap_err();
        assert!(err.contains("ถอดรหัส"), "thai message, got {err}");
    }
}
