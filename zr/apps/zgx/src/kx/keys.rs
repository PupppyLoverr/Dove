//! Keys the person gives Keplorayx for services it calls for them (Boat's,
//! for a buddy's cloud computer). A key goes in and never comes back out to
//! the scripts: they're told only which keys are held.
//!
//!   kx.keys                     { boat = true, … } (shared; names only)
//!   kx.key.set({ name, value }) keeps it (replacing one held)
//!   kx.key.remove(name)
//!
//! Kept in `<data>/keys.json`, each sealed by the system for this person on
//! this machine (Windows' DPAPI): the file copied elsewhere, or read by
//! another account, opens nothing. On a system with no such seal yet, a key
//! is refused rather than kept in the clear.

use std::{cell::RefCell, path::PathBuf};

use serde_json::{Map, Value as Json, json};
use zgx_script::{ScriptHost, Value};

/// The keys there is a use for.
const NAMES: &[&str] = &["boat"];

fn file() -> PathBuf {
    crate::kx_account::data_dir().join("keys.json")
}

fn read() -> Map<String, Json> {
    std::fs::read_to_string(file()).ok().and_then(|s| serde_json::from_str::<Json>(&s).ok()).and_then(|j| j.as_object().cloned()).unwrap_or_default()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

#[cfg(windows)]
mod seal {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData},
    };

    fn run(input: &[u8], shut: bool) -> Option<Vec<u8>> {
        let from = CRYPT_INTEGER_BLOB { cbData: u32::try_from(input.len()).ok()?, pbData: input.as_ptr().cast_mut() };
        let mut to = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: `from` points at `input` for its length and is only read;
        // `to` is filled by the call with memory it allocates, which is
        // copied out and freed with LocalFree as the API asks.
        unsafe {
            let ok = if shut {
                CryptProtectData(&from, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut to)
            } else {
                CryptUnprotectData(&from, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut to)
            };
            if ok == 0 || to.pbData.is_null() {
                return None;
            }
            let out = std::slice::from_raw_parts(to.pbData, to.cbData as usize).to_vec();
            LocalFree(to.pbData.cast());
            Some(out)
        }
    }

    pub fn shut(plain: &[u8]) -> Option<Vec<u8>> {
        run(plain, true)
    }

    pub fn open(sealed: &[u8]) -> Option<Vec<u8>> {
        run(sealed, false)
    }
}

#[cfg(not(windows))]
mod seal {
    pub fn shut(_: &[u8]) -> Option<Vec<u8>> {
        None
    }

    pub fn open(_: &[u8]) -> Option<Vec<u8>> {
        None
    }
}

/// A key held, for the code that calls its service. Never given to a script.
pub fn get(name: &str) -> Option<String> {
    let sealed = unhex(read().get(name)?.as_str()?)?;
    String::from_utf8(seal::open(&sealed)?).ok().filter(|k| !k.is_empty())
}

fn set(name: &str, value: &str) -> Result<(), String> {
    if !NAMES.contains(&name) {
        return Err("That isn't a key Keplorayx uses.".into());
    }
    let value = value.trim();
    if value.len() < 8 || value.len() > 512 || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("That doesn't look like a key.".into());
    }
    let sealed = seal::shut(value.as_bytes()).ok_or("Keys can't be kept safely on this system yet.")?;
    let mut all = read();
    all.insert(name.to_owned(), json!(hex(&sealed)));
    std::fs::write(file(), Json::Object(all).to_string()).map_err(|e| format!("The key couldn't be saved: {e}"))
}

fn remove(name: &str) {
    let mut all = read();
    if all.remove(name).is_some() {
        let _ = std::fs::write(file(), Json::Object(all).to_string());
    }
}

/// Which keys are held: names, never what they are.
fn held() -> Value {
    let all = read();
    let names: Map<String, Json> = NAMES.iter().map(|n| ((*n).to_owned(), json!(all.contains_key(*n)))).collect();
    crate::kx::to_value(&Json::Object(names))
}

thread_local! {
    static HELD: RefCell<Option<zgx_script::Shared>> = const { RefCell::new(None) };
}

fn publish() {
    HELD.with(|h| {
        if let Some(shared) = h.borrow().as_ref() {
            shared.set(held());
        }
    });
}

pub fn install(host: &ScriptHost) {
    let shared = host.shared("kx.keys", held());
    HELD.with(|h| *h.borrow_mut() = Some(shared));
    host.async_action("kx.key.set", |req| {
        let req = crate::kx::widgets::value_json(&req);
        let done = set(req["name"].as_str().unwrap_or_default(), req["value"].as_str().unwrap_or_default());
        publish();
        async move { done.map(|()| Value::Nil) }
    });
    host.action("kx.key.remove", |name| {
        remove(&crate::auth::text(&name));
        publish();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_goes_and_comes_back() {
        let bytes = [0u8, 1, 15, 16, 200, 255];
        assert_eq!(hex(&bytes), "00010f10c8ff");
        assert_eq!(unhex(&hex(&bytes)).as_deref(), Some(&bytes[..]));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_sealed_key_opens_again_and_is_not_itself() {
        let key = b"boat_test_0123456789";
        let sealed = seal::shut(key).expect("sealed");
        assert_ne!(&sealed[..], &key[..]);
        assert!(!sealed.windows(key.len()).any(|w| w == key));
        assert_eq!(seal::open(&sealed).as_deref(), Some(&key[..]));
        assert_eq!(seal::open(b"not sealed at all"), None);
    }
}
