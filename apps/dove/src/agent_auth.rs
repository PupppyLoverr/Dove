//! Whether each agent CLI is signed in, read from the files the CLIs keep
//! for themselves. Local and cheap: no network, no process. Agents whose
//! state lives elsewhere (a keychain, a server) report [`SignIn::Unknown`]
//! and the picker shows nothing for them rather than guessing.

use std::path::{Path, PathBuf};
use std::process::Command;

use dove_core::proto::HarnessKind;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignIn {
    /// Signed in; the account's email or kind ("API key") when known.
    SignedIn(Option<String>),
    SignedOut,
    Unknown,
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

fn env_set(key: &str) -> bool {
    std::env::var_os(key).is_some_and(|v| !v.is_empty())
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn text(v: &Value, pointer: &str) -> Option<String> {
    v.pointer(pointer).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// Read the agent's sign-in state. Does file I/O: call it off the UI thread.
pub fn detect(kind: HarnessKind) -> SignIn {
    match kind {
        HarnessKind::ClaudeCode => claude(),
        HarnessKind::Codex => codex(),
        HarnessKind::Cursor => cursor(),
        _ => SignIn::Unknown,
    }
}

fn claude() -> SignIn {
    if env_set("ANTHROPIC_API_KEY") || env_set("CLAUDE_CODE_OAUTH_TOKEN") {
        return SignIn::SignedIn(Some("API key".into()));
    }
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    let Some(home) = home() else { return SignIn::Unknown };
    // `.claude.json` sits in the config dir when one is set, else in home.
    let profile = config_dir.clone().map(|d| d.join(".claude.json")).unwrap_or_else(|| home.join(".claude.json"));
    let creds = config_dir.unwrap_or_else(|| home.join(".claude")).join(".credentials.json");
    let email = read_json(&profile).and_then(|v| text(&v, "/oauthAccount/emailAddress"));
    match email {
        Some(email) => SignIn::SignedIn(Some(email)),
        None if creds.is_file() => SignIn::SignedIn(None),
        // macOS keeps the token in the keychain; the profile is the tell.
        None => SignIn::SignedOut,
    }
}

fn codex() -> SignIn {
    let dir = std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".codex")));
    let auth = dir.and_then(|d| read_json(&d.join("auth.json")));
    let Some(auth) = auth else {
        return if env_set("OPENAI_API_KEY") { SignIn::SignedIn(Some("API key".into())) } else { SignIn::SignedOut };
    };
    if let Some(token) = text(&auth, "/tokens/id_token") {
        return SignIn::SignedIn(jwt_claim(&token, "email"));
    }
    if text(&auth, "/OPENAI_API_KEY").is_some() || env_set("OPENAI_API_KEY") {
        return SignIn::SignedIn(Some("API key".into()));
    }
    SignIn::SignedOut
}

fn cursor() -> SignIn {
    if env_set("CURSOR_API_KEY") {
        return SignIn::SignedIn(Some("API key".into()));
    }
    let Some(config) = home().and_then(|h| read_json(&h.join(".cursor").join("cli-config.json"))) else {
        return SignIn::SignedOut;
    };
    match config.get("authInfo").filter(|a| a.is_object()) {
        Some(_) => SignIn::SignedIn(text(&config, "/authInfo/email")),
        None => SignIn::SignedOut,
    }
}

/// One string claim from a JWT's payload (unverified; display only).
fn jwt_claim(token: &str, claim: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64url(payload)?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    text(&v, &format!("/{claim}"))
}

fn base64url(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|c| *c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The CLI arguments that start the agent's own sign-in.
fn login_args(kind: HarnessKind) -> Option<&'static [&'static str]> {
    match kind {
        // A signed-out `claude` opens its login flow on start.
        HarnessKind::ClaudeCode => Some(&[]),
        HarnessKind::Codex => Some(&["login"]),
        HarnessKind::Cursor => Some(&["login"]),
        _ => None,
    }
}

/// Whether Dove can start this agent's sign-in for you.
pub fn can_sign_in(kind: HarnessKind) -> bool {
    login_args(kind).is_some()
}

/// Open a terminal window running the agent's sign-in. Returns once the
/// window is open; the picker re-reads the state when it next opens.
pub fn open_login(kind: HarnessKind, program: &Path) -> std::io::Result<()> {
    let args = login_args(kind).unwrap_or(&[]);
    let program = program.to_string_lossy().into_owned();
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", &format!("Sign in to {}", kind.label()), "cmd", "/k", &program]).args(args);
        cmd.spawn().map(|_| ())
    } else {
        let line = std::iter::once(format!("'{program}'")).chain(args.iter().map(|a| a.to_string())).collect::<Vec<_>>().join(" ");
        if cfg!(target_os = "macos") {
            let script = format!("tell application \"Terminal\" to do script \"{line}\"");
            Command::new("osascript")
                .args(["-e", &script, "-e", "tell application \"Terminal\" to activate"])
                .spawn()
                .map(|_| ())
        } else {
            Command::new("x-terminal-emulator").args(["-e", "sh", "-c", &format!("{line}; exec $SHELL")]).spawn().map(|_| ())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_jwt_claim() {
        // {"email":"a@b.co"} base64url-encoded, unpadded.
        let token = format!("x.{}.y", "eyJlbWFpbCI6ImFAYi5jbyJ9");
        assert_eq!(jwt_claim(&token, "email").as_deref(), Some("a@b.co"));
        assert_eq!(jwt_claim("nope", "email"), None);
    }
}
