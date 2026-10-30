//! What is left of the user's Claude and ChatGPT (Codex) subscription
//! limits, read the way Hermes Agent's `account_usage` reads them.
//!
//! - **Claude**: `GET https://api.anthropic.com/api/oauth/usage` with Claude
//!   Code's own OAuth access token, from `~/.claude/.credentials.json`
//!   (`$CLAUDE_CONFIG_DIR` when set) or, on macOS, the login Keychain item
//!   `Claude Code-credentials`. Windows: 5-hour, weekly, weekly Opus and
//!   weekly Sonnet.
//! - **Codex**: `GET https://chatgpt.com/backend-api/wham/usage` with the
//!   Codex CLI's tokens from `~/.codex/auth.json` (`$CODEX_HOME` when set).
//!   Windows: the 5-hour session and the week.
//!
//! Credentials are only read, never refreshed or written: an expired token
//! is reported as such, and the agent's own CLI refreshes it the next time it
//! runs. Tokens never leave this module except in the request's
//! `Authorization` header, and are never logged. Every call is blocking; run
//! it off the UI thread. [`get`] keeps the last answer for a few minutes.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use base64::Engine as _;
use dove_proto::RateWindow;
use parking_lot::Mutex;
use serde_json::Value;

const CLAUDE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CODEX_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const TIMEOUT: Duration = Duration::from_secs(15);
/// How long [`get`] reuses an answer.
pub const MAX_AGE: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

/// Whether the limits could be read, and if not, why.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Ok,
    /// No sign-in to read.
    SignedOut,
    /// The stored sign-in has expired (the agent's CLI refreshes it when it
    /// next runs).
    Expired,
    /// Signed in a way that has no plan limits (an API key).
    NoPlan,
    /// The request failed: a short, token-free reason.
    Failed(String),
}

/// One provider's plan limits.
#[derive(Debug, Clone, PartialEq)]
pub struct Quota {
    pub provider: Provider,
    pub status: Status,
    /// "Max", "Pro", "Plus", ...
    pub plan: Option<String>,
    pub windows: Vec<RateWindow>,
    /// Extra lines worth showing (Codex credits).
    pub notes: Vec<String>,
    /// When this was read (ms).
    pub fetched_at: i64,
}

impl Quota {
    fn new(provider: Provider, status: Status) -> Self {
        Self { provider, status, plan: None, windows: Vec::new(), notes: Vec::new(), fetched_at: dove_store::now_ms() }
    }

    /// The line to show instead of meters, when there are none.
    pub fn message(&self) -> Option<String> {
        let cli = match self.provider {
            Provider::Claude => "Claude Code",
            Provider::Codex => "Codex",
        };
        Some(match &self.status {
            Status::Ok if self.windows.is_empty() => "No limits reported for this plan".into(),
            Status::Ok => return None,
            Status::SignedOut => format!("Sign in to {cli} to see your limits"),
            Status::Expired => format!("{cli}'s sign-in has expired; run {cli} once to refresh it and see your limits"),
            Status::NoPlan => match self.provider {
                Provider::Claude => "Claude Code uses an API key; plan limits apply to Claude subscriptions".into(),
                Provider::Codex => "Codex uses an API key; sign in with ChatGPT to see plan limits".into(),
            },
            Status::Failed(why) => format!("Couldn't read limits: {why}"),
        })
    }
}

static CACHE: Mutex<Option<(Instant, Vec<Quota>)>> = Mutex::new(None);
/// One fetch at a time; a second caller waits and takes its answer.
static FETCHING: Mutex<()> = Mutex::new(());

/// Both providers' limits, at most [`MAX_AGE`] old. Blocking.
pub fn get() -> Vec<Quota> {
    if let Some(q) = cached(MAX_AGE) {
        return q;
    }
    let _one = FETCHING.lock();
    if let Some(q) = cached(MAX_AGE) {
        return q;
    }
    let fresh = fetch_all();
    *CACHE.lock() = Some((Instant::now(), fresh.clone()));
    fresh
}

/// The last answer, when younger than `max_age`. Never blocks on a fetch.
pub fn cached(max_age: Duration) -> Option<Vec<Quota>> {
    CACHE.lock().as_ref().filter(|(at, _)| at.elapsed() < max_age).map(|(_, q)| q.clone())
}

/// Both providers, fetched side by side. Blocking.
pub fn fetch_all() -> Vec<Quota> {
    let codex = std::thread::spawn(fetch_codex);
    let claude = fetch_claude();
    let codex = codex.join().unwrap_or_else(|_| Quota::new(Provider::Codex, Status::Failed("internal error".into())));
    vec![claude, codex]
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder().timeout(TIMEOUT).build().map_err(|_| "no HTTP client".to_string())
}

/// GET `url` and parse JSON; errors are short and carry no secrets.
fn get_json(req: reqwest::blocking::RequestBuilder, host: &str) -> Result<Value, (Option<u16>, String)> {
    let resp = req.send().map_err(|e| {
        let why =
            if e.is_timeout() { format!("{host} did not answer in time") } else { format!("couldn't reach {host}") };
        (None, why)
    })?;
    let code = resp.status();
    if !code.is_success() {
        return Err((Some(code.as_u16()), format!("{host} answered HTTP {}", code.as_u16())));
    }
    let body = resp.text().map_err(|_| (None, format!("{host} sent an unreadable answer")))?;
    serde_json::from_str(&body).map_err(|_| (None, format!("{host} sent an unexpected answer")))
}

// ------------------------------------------------------------------ Claude

/// Claude Code's stored OAuth sign-in.
#[derive(Debug, Clone, PartialEq)]
struct ClaudeCreds {
    token: String,
    /// ms since the epoch; 0 when not given.
    expires_at: i64,
    plan: Option<String>,
}

fn claude_creds_path() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir).join(".credentials.json")),
        None => home().map(|h| h.join(".claude").join(".credentials.json")),
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// A `{"claudeAiOauth": {...}}` payload.
fn parse_claude_creds(json: &str) -> Option<ClaudeCreds> {
    let v: Value = serde_json::from_str(json.trim_start_matches('\u{feff}')).ok()?;
    let o = v.get("claudeAiOauth")?;
    let token = o.get("accessToken")?.as_str()?.trim().to_string();
    if token.is_empty() {
        return None;
    }
    let plan = o.get("subscriptionType").and_then(Value::as_str).filter(|s| !s.is_empty()).map(title_case);
    Some(ClaudeCreds { token, expires_at: o.get("expiresAt").and_then(Value::as_i64).unwrap_or(0), plan })
}

fn valid(c: &ClaudeCreds, now_ms: i64) -> bool {
    c.expires_at == 0 || now_ms < c.expires_at - 60_000
}

/// Claude Code's login Keychain item (Claude Code 2.1.114 and later keep
/// the sign-in there on macOS), read with `security` as Hermes does.
#[cfg(target_os = "macos")]
fn claude_keychain() -> Option<ClaudeCreds> {
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    /// The Keychain was asked once and said no (or has no item): don't
    /// ask again this run, so a denied prompt isn't repeated every refresh.
    static KEYCHAIN_REFUSED: AtomicBool = AtomicBool::new(false);
    if KEYCHAIN_REFUSED.load(Ordering::Relaxed) {
        return None;
    }
    let mut child = Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // macOS may ask the user to allow access; give them time to answer.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                KEYCHAIN_REFUSED.store(true, Ordering::Relaxed);
                return None;
            }
        }
    }
    let out = child.wait_with_output().ok()?;
    let creds = out
        .status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
        .and_then(|s| parse_claude_creds(&s));
    if creds.is_none() {
        KEYCHAIN_REFUSED.store(true, Ordering::Relaxed);
    }
    creds
}

#[cfg(not(target_os = "macos"))]
fn claude_keychain() -> Option<ClaudeCreds> {
    None
}

/// Claude Code's sign-in: the file, else the macOS Keychain. With both,
/// the one still valid, else the later expiry (Claude Code refreshes one
/// store and not always the other).
fn claude_creds() -> Option<ClaudeCreds> {
    let now = dove_store::now_ms();
    let file = claude_creds_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| parse_claude_creds(&s));
    if file.as_ref().is_some_and(|c| valid(c, now)) {
        return file;
    }
    let keychain = claude_keychain();
    match (file, keychain) {
        (Some(f), Some(k)) => Some(if valid(&k, now) || k.expires_at >= f.expires_at { k } else { f }),
        (f, k) => f.or(k),
    }
}

pub fn fetch_claude() -> Quota {
    let Some(creds) = claude_creds() else { return Quota::new(Provider::Claude, Status::SignedOut) };
    let mut q = Quota::new(Provider::Claude, Status::Ok);
    q.plan = creds.plan.clone();
    if is_api_key(&creds.token) {
        q.status = Status::NoPlan;
        return q;
    }
    if !valid(&creds, dove_store::now_ms()) {
        q.status = Status::Expired;
        return q;
    }
    let client = match client() {
        Ok(c) => c,
        Err(e) => {
            q.status = Status::Failed(e);
            return q;
        }
    };
    let req = client
        .get(CLAUDE_URL)
        .bearer_auth(&creds.token)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "claude-code/2.1.0");
    match get_json(req, "api.anthropic.com") {
        Ok(v) => q.windows = parse_claude(&v),
        Err((Some(401), _)) => q.status = Status::Expired,
        Err((Some(403), _)) => {
            q.status = Status::Failed("this Claude Code sign-in isn't allowed to read plan limits".into())
        }
        Err((_, why)) => q.status = Status::Failed(why),
    }
    q
}

/// Console API keys pay per token and have no plan windows.
fn is_api_key(token: &str) -> bool {
    ["sk-ant-api", "sk-ant-usr", "sk-ant-svc", "sk-ant-admin"].iter().any(|p| token.starts_with(p))
}

fn parse_claude(v: &Value) -> Vec<RateWindow> {
    let mut windows = Vec::new();
    for (key, minutes) in
        [("five_hour", 300), ("seven_day", 10_080), ("seven_day_opus", 10_080), ("seven_day_sonnet", 10_080)]
    {
        let Some(w) = v.get(key).filter(|w| w.is_object()) else { continue };
        let Some(used) = w.get("utilization").and_then(Value::as_f64) else { continue };
        let used = used.clamp(0.0, 100.0);
        windows.push(RateWindow {
            kind: key.into(),
            used_percent: Some(used),
            resets_at: w.get("resets_at").and_then(parse_time),
            window_minutes: Some(minutes),
            blocked: used >= 100.0,
        });
    }
    windows
}

// ------------------------------------------------------------------- Codex

#[derive(Debug, Clone, PartialEq)]
enum CodexAuth {
    Tokens { access: String, account: Option<String> },
    ApiKey,
}

fn codex_auth_path() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir).join("auth.json")),
        None => home().map(|h| h.join(".codex").join("auth.json")),
    }
}

fn parse_codex_auth(json: &str) -> Option<CodexAuth> {
    let v: Value = serde_json::from_str(json.trim_start_matches('\u{feff}')).ok()?;
    let tokens = v.get("tokens").filter(|t| t.is_object());
    let access = tokens.and_then(|t| t.get("access_token")).and_then(Value::as_str).map(str::trim).unwrap_or("");
    if !access.is_empty() {
        let account = tokens
            .and_then(|t| t.get("account_id"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);
        return Some(CodexAuth::Tokens { access: access.to_string(), account });
    }
    v.get("OPENAI_API_KEY").and_then(Value::as_str).filter(|k| !k.trim().is_empty()).map(|_| CodexAuth::ApiKey)
}

/// The claims of a JWT, unverified (only to read its expiry and account).
fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn fetch_codex() -> Quota {
    let auth = codex_auth_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| parse_codex_auth(&s));
    let (access, account) = match auth {
        None => return Quota::new(Provider::Codex, Status::SignedOut),
        Some(CodexAuth::ApiKey) => return Quota::new(Provider::Codex, Status::NoPlan),
        Some(CodexAuth::Tokens { access, account }) => (access, account),
    };
    let mut q = Quota::new(Provider::Codex, Status::Ok);
    let claims = jwt_claims(&access);
    let auth_claims = claims.as_ref().and_then(|c| c.get("https://api.openai.com/auth"));
    if let Some(exp) = claims.as_ref().and_then(|c| c.get("exp")).and_then(Value::as_i64)
        && exp * 1000 <= dove_store::now_ms()
    {
        q.status = Status::Expired;
        return q;
    }
    let client = match client() {
        Ok(c) => c,
        Err(e) => {
            q.status = Status::Failed(e);
            return q;
        }
    };
    let mut req = client
        .get(CODEX_URL)
        .bearer_auth(&access)
        .header("Accept", "application/json")
        .header("User-Agent", "codex-cli");
    let account = account
        .or_else(|| auth_claims.and_then(|a| a.get("chatgpt_account_id")).and_then(Value::as_str).map(str::to_string));
    if let Some(a) = account {
        req = req.header("ChatGPT-Account-ID", a);
    }
    let residency = auth_claims.and_then(|a| {
        a.get("chatgpt_data_residency").or_else(|| a.get("chatgpt_compute_residency")).and_then(Value::as_str)
    });
    if let Some(r) = residency.map(str::trim).filter(|r| !r.is_empty()) {
        req = req.header("x-openai-internal-codex-residency", r);
    }
    match get_json(req, "chatgpt.com") {
        Ok(v) => {
            let (plan, windows, notes) = parse_codex(&v, dove_store::now_ms() / 1000);
            q.plan = plan;
            q.windows = windows;
            q.notes = notes;
        }
        Err((Some(401), _)) => q.status = Status::Expired,
        Err((_, why)) => q.status = Status::Failed(why),
    }
    q
}

fn parse_codex(v: &Value, now_s: i64) -> (Option<String>, Vec<RateWindow>, Vec<String>) {
    let plan = v.get("plan_type").and_then(Value::as_str).filter(|s| !s.is_empty()).map(title_case);
    let mut windows = Vec::new();
    let limits = v.get("rate_limit");
    for (key, kind) in [("primary_window", "primary"), ("secondary_window", "secondary")] {
        let Some(w) = limits.and_then(|l| l.get(key)).filter(|w| w.is_object()) else { continue };
        let Some(used) = w.get("used_percent").and_then(Value::as_f64) else { continue };
        let used = used.clamp(0.0, 100.0);
        let resets_at = w
            .get("reset_at")
            .and_then(parse_time)
            .or_else(|| w.get("reset_after_seconds").and_then(Value::as_i64).map(|s| now_s + s));
        windows.push(RateWindow {
            kind: kind.into(),
            used_percent: Some(used),
            resets_at,
            window_minutes: w.get("limit_window_seconds").and_then(Value::as_u64).map(|s| s / 60),
            blocked: used >= 100.0
                || limits.and_then(|l| l.get("limit_reached")).and_then(Value::as_bool) == Some(true),
        });
    }
    let mut notes = Vec::new();
    if let Some(c) = v.get("credits").filter(|c| c.get("has_credits").and_then(Value::as_bool) == Some(true)) {
        if c.get("unlimited").and_then(Value::as_bool) == Some(true) {
            notes.push("Credits: unlimited".into());
        } else if let Some(b) = c.get("balance").and_then(|b| b.as_f64().or_else(|| b.as_str()?.parse().ok())) {
            notes.push(format!("Credits balance: ${b:.2}"));
        }
    }
    (plan, windows, notes)
}

// ------------------------------------------------------------------ shared

/// Unix seconds from a number (seconds, or ms when huge) or an RFC 3339 string.
fn parse_time(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_f64() {
        let n = n as i64;
        return Some(if n > 100_000_000_000 { n / 1000 } else { n });
    }
    let s = v.as_str()?.trim();
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp())
}

/// "max" → "Max", "chatgpt_plus" → "Chatgpt Plus".
fn title_case(s: &str) -> String {
    s.split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_claude_code_credentials() {
        let c = parse_claude_creds(
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x","refreshToken":"r","expiresAt":1900000000000,"subscriptionType":"max"}}"#,
        )
        .unwrap();
        assert_eq!(c.token, "sk-ant-oat01-x");
        assert_eq!(c.plan.as_deref(), Some("Max"));
        assert!(valid(&c, 1_800_000_000_000));
        assert!(!valid(&c, 1_900_000_000_000));
        assert_eq!(parse_claude_creds(r#"{"claudeAiOauth":{"accessToken":""}}"#), None);
        assert_eq!(parse_claude_creds("{}"), None);
        assert!(is_api_key("sk-ant-api03-abc") && !is_api_key("sk-ant-oat01-abc"));
    }

    #[test]
    fn parses_claude_windows() {
        let v = json!({
            "five_hour": {"utilization": 37.0, "resets_at": "2026-10-29T15:00:00.000000+00:00"},
            "seven_day": {"utilization": 100, "resets_at": "2026-11-02T00:00:00Z"},
            "seven_day_opus": null,
            "seven_day_sonnet": {"utilization": 4.5, "resets_at": null}
        });
        let w = parse_claude(&v);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].label(), "5-hour limit");
        assert_eq!(w[0].used_percent, Some(37.0));
        assert_eq!(w[0].resets_at, Some(1_793_286_000));
        assert!(w[1].blocked);
        assert_eq!(w[2].label(), "Weekly (Sonnet)");
        assert_eq!(w[2].resets_at, None);
    }

    #[test]
    fn reads_codex_auth() {
        let a = parse_codex_auth(r#"{"OPENAI_API_KEY":null,"tokens":{"access_token":"a.b.c","account_id":"acc"}}"#);
        assert_eq!(a, Some(CodexAuth::Tokens { access: "a.b.c".into(), account: Some("acc".into()) }));
        assert_eq!(parse_codex_auth(r#"{"OPENAI_API_KEY":"sk-x"}"#), Some(CodexAuth::ApiKey));
        assert_eq!(parse_codex_auth(r#"{"OPENAI_API_KEY":null,"tokens":null}"#), None);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"exp":1700000000,"https://api.openai.com/auth":{"chatgpt_account_id":"x"}}"#);
        let claims = jwt_claims(&format!("h.{payload}.s")).unwrap();
        assert_eq!(claims["exp"], 1_700_000_000);
    }

    #[test]
    fn parses_codex_windows_by_duration() {
        let v = json!({
            "plan_type": "plus",
            "rate_limit": {
                "primary_window": {"used_percent": 12, "limit_window_seconds": 18000, "reset_at": 1793290000},
                "secondary_window": {"used_percent": 61.5, "limit_window_seconds": 604800, "reset_after_seconds": 3600}
            },
            "credits": {"has_credits": true, "unlimited": false, "balance": "4.5"}
        });
        let (plan, w, notes) = parse_codex(&v, 1_000);
        assert_eq!(plan.as_deref(), Some("Plus"));
        assert_eq!(w[0].label(), "5-hour limit");
        assert_eq!(w[0].resets_at, Some(1_793_290_000));
        assert_eq!(w[1].label(), "Weekly limit");
        assert_eq!(w[1].resets_at, Some(4_600));
        assert_eq!(notes, vec!["Credits balance: $4.50".to_string()]);
        // Only the weekly window, in the primary slot, is still weekly.
        let v = json!({"rate_limit": {"primary_window": {"used_percent": 5, "limit_window_seconds": 604800}}});
        assert_eq!(parse_codex(&v, 0).1[0].label(), "Weekly limit");
    }

    #[test]
    fn messages_are_quiet_and_token_free() {
        assert_eq!(
            Quota::new(Provider::Claude, Status::SignedOut).message().as_deref(),
            Some("Sign in to Claude Code to see your limits")
        );
        let mut ok = Quota::new(Provider::Codex, Status::Ok);
        ok.windows.push(RateWindow {
            kind: "primary".into(),
            used_percent: Some(1.0),
            resets_at: None,
            window_minutes: Some(300),
            blocked: false,
        });
        assert_eq!(ok.message(), None);
    }
}
