//! The person's Composio key, as the app keeps it: `<data>/keys.json`,
//! `{ "composio": "<hex of the DPAPI-sealed key>" }` (see the app's
//! `kx/keys.rs`). Sealed for this person on this machine, so only a process
//! of theirs here opens it.

use std::path::Path;

/// Its name in `keys.json`.
pub const NAME: &str = "composio";

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

/// The key, opened; `None` when there's none, or it can't be opened here.
pub fn read(data_dir: &Path) -> Option<String> {
    let file: serde_json::Value = serde_json::from_slice(&std::fs::read(data_dir.join("keys.json")).ok()?).ok()?;
    let sealed = unhex(file.get(NAME)?.as_str()?)?;
    String::from_utf8(open(&sealed)?).ok().map(|k| k.trim().to_owned()).filter(|k| !k.is_empty())
}

/// Whether a key is held (without opening it).
pub fn held(data_dir: &Path) -> bool {
    std::fs::read(data_dir.join("keys.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|file| file.get(NAME).and_then(|v| v.as_str()).is_some_and(|v| !v.is_empty()))
}

#[cfg(windows)]
fn open(sealed: &[u8]) -> Option<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData},
    };
    let from = CRYPT_INTEGER_BLOB { cbData: u32::try_from(sealed.len()).ok()?, pbData: sealed.as_ptr().cast_mut() };
    let mut to = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: `from` points at `sealed` for its length and is only read;
    // `to` is filled by the call with memory it allocates, copied out and
    // freed with LocalFree as the API asks.
    unsafe {
        let ok = CryptUnprotectData(&from, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut to);
        if ok == 0 || to.pbData.is_null() {
            return None;
        }
        let out = std::slice::from_raw_parts(to.pbData, to.cbData as usize).to_vec();
        LocalFree(to.pbData.cast());
        Some(out)
    }
}

#[cfg(not(windows))]
fn open(_: &[u8]) -> Option<Vec<u8>> {
    None
}
