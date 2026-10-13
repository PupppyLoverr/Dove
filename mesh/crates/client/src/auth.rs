//! Edge auth — `/auth/exchange`, `/auth/orgs`, `/auth/refresh`
//! (edge/src/auth-routes.ts), plus the per-client bearer provider.
//!
//! Live modes, mirroring the engine:
//! - WorkOS: paste-code exchange → access/refresh pair; a refresh scoped to an
//!   organization adds the `org_id` claim the registry room requires.
//! - Woozlit: Firebase ID token → personal workspace, where the uid is both
//!   user and org; Firebase rotates its refresh token.
//! - Dev (`AUTH_MODE=dev` edge): the bearer IS `userId@orgId`.
//!
//! Refresh tokens are single-use: [`TokenProvider`] single-flights every
//! refresh and reports the rotated pair to the platform.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::AuthTokens;
use crate::error::{ClientError, Result};
use crate::events::{ClientEvent, EventPump};
use crate::lock;

/// Refresh this long before `exp` (legacy AppConfig: 60s early margin).
pub const EARLY_REFRESH_SECS: i64 = 60;

/// Production endpoints (edge/wrangler.jsonc). Mobile always talks to prod —
/// a stale override once broke sign-in in the worst ghost way.
pub const PRODUCTION_EDGE_URL: &str = "https://cosmos-edge.cosmos-edge.workers.dev";
pub const WORKOS_CLIENT_ID: &str = "client_01KWD0EAKZKD50YCQJNYSRE4BY";
pub const WORKOS_API_BASE: &str = "https://api.workos.com";
pub const WOOZLIT_ORIGIN: &str = "https://www.woozlit.com";
const FIREBASE_API_KEY: &str = "AIzaSyBbJtkBoJrKM2hjW2H3o9pgtcyiGge9yOo";
/// OAuth redirect: `dove://callback?code=…&state=…`.
pub const CALLBACK_SCHEME: &str = "dove";

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            // Bytes, not `&str` slicing: `%` before a multi-byte char would
            // split it (and panic).
            b'%' if i + 3 <= bytes.len() => match std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                Some(b) => {
                    out.push(b);
                    i += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// WorkOS AuthKit authorization-code URL (the ASWebAuthenticationSession
/// start URL). `state` is the caller's CSRF nonce.
pub fn workos_authorize_url(state: &str) -> String {
    format!(
        "{WORKOS_API_BASE}/user_management/authorize?response_type=code&client_id={}&redirect_uri={}&provider=authkit&state={}",
        percent_encode(WORKOS_CLIENT_ID),
        percent_encode(&format!("{CALLBACK_SCHEME}://callback")),
        percent_encode(state),
    )
}

/// Woozlit sign-in URL, with a callback that returns to this app.
pub fn woozlit_sign_in_url(state: &str) -> String {
    let callback = format!("{CALLBACK_SCHEME}://auth/callback?state={state}");
    format!(
        "{WOOZLIT_ORIGIN}/login?desktop=1&callback={}",
        percent_encode(&callback)
    )
}

/// The `code` + `state` of an OAuth callback URL, or the provider's error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCallback {
    Code {
        code: String,
        state: Option<String>,
    },
    Error {
        error: String,
        description: Option<String>,
    },
}

pub fn parse_auth_callback(url: &str) -> Option<AuthCallback> {
    let query = url.split_once('?')?.1;
    let query = query.split('#').next().unwrap_or(query);
    let mut params = std::collections::HashMap::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        params.insert(percent_decode(key), percent_decode(value));
    }
    if let Some(error) = params.remove("error") {
        return Some(AuthCallback::Error {
            error,
            description: params.remove("error_description"),
        });
    }
    let code = params.remove("code").filter(|c| !c.is_empty())?;
    Some(AuthCallback::Code {
        code,
        state: params.remove("state"),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthUser {
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthOrg {
    pub id: String,
    pub organization_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthExchange {
    pub user: AuthUser,
    pub tokens: AuthTokens,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WoozlitSession {
    pub user_id: String,
    pub email: Option<String>,
    pub tokens: AuthTokens,
}

pub(crate) fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client")
    })
}

fn endpoint(edge_url: &str, path: &str) -> String {
    format!("{}/{path}", edge_url.trim_end_matches('/'))
}

async fn check(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    // Timeouts and rate limits are transient, not a rejected credential
    // (an Auth error signs the user out).
    let transient = matches!(
        status,
        reqwest::StatusCode::REQUEST_TIMEOUT | reqwest::StatusCode::TOO_MANY_REQUESTS
    );
    if status.is_client_error() && !transient {
        Err(ClientError::Auth(format!("{status}: {body}")))
    } else {
        Err(ClientError::Network(format!("{status}: {body}")))
    }
}

fn net(err: reqwest::Error) -> ClientError {
    ClientError::Network(err.to_string())
}

/// WorkOS paste-code exchange.
pub async fn exchange_code(edge_url: &str, code: &str) -> Result<AuthExchange> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Reply {
        user: AuthUser,
        access_token: String,
        refresh_token: String,
    }
    let url = endpoint(edge_url, "auth/exchange");
    let code = code.trim().to_owned();
    crate::runtime::run(async move {
        let response = http()
            .post(url)
            .json(&serde_json::json!({ "code": code }))
            .send()
            .await
            .map_err(net)?;
        let reply: Reply = check(response).await?.json().await.map_err(net)?;
        Ok(AuthExchange {
            user: reply.user,
            tokens: AuthTokens {
                access_token: reply.access_token,
                refresh_token: reply.refresh_token,
            },
        })
    })
    .await
}

/// Organizations the (unscoped) access token's user belongs to.
pub async fn list_orgs(edge_url: &str, access_token: &str) -> Result<Vec<AuthOrg>> {
    #[derive(Deserialize)]
    struct Reply {
        orgs: Vec<AuthOrg>,
    }
    let url = endpoint(edge_url, "auth/orgs");
    let token = access_token.to_owned();
    crate::runtime::run(async move {
        let response = http()
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(net)?;
        let reply: Reply = check(response).await?.json().await.map_err(net)?;
        Ok(reply.orgs)
    })
    .await
}

/// Rotate the pair; `organization_id` re-scopes the access token to an org
/// (the org picker's "select" step).
pub async fn refresh(
    edge_url: &str,
    refresh_token: &str,
    organization_id: Option<&str>,
) -> Result<AuthTokens> {
    let url = endpoint(edge_url, "auth/refresh");
    let mut body = serde_json::json!({ "refreshToken": refresh_token });
    if let Some(org) = organization_id {
        body["organizationId"] = serde_json::Value::String(org.to_owned());
    }
    crate::runtime::run(async move {
        let response = http().post(url).json(&body).send().await.map_err(net)?;
        check(response).await?.json().await.map_err(net)
    })
    .await
}

/// Exchange a Woozlit Firebase custom token, or accept a Firebase ID token
/// directly when the login callback already supplied one.
pub async fn woozlit_exchange(token: &str) -> Result<WoozlitSession> {
    if let Some((user_id, email)) = woozlit_id_token_claims(token) {
        return Ok(WoozlitSession {
            user_id,
            email,
            tokens: AuthTokens {
                access_token: token.to_owned(),
                refresh_token: String::new(),
            },
        });
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Reply {
        id_token: String,
        refresh_token: String,
        #[serde(default)]
        local_id: Option<String>,
    }

    let token = token.to_owned();
    let url = format!(
        "https://identitytoolkit.googleapis.com/v1/accounts:signInWithCustomToken?key={FIREBASE_API_KEY}"
    );
    crate::runtime::run(async move {
        let response = http()
            .post(url)
            .json(&serde_json::json!({ "token": token, "returnSecureToken": true }))
            .send()
            .await
            .map_err(net)?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ClientError::Auth(format!("{status}: {body}")));
        }
        let reply: Reply = response.json().await.map_err(net)?;
        let claims = jwt_claims(&reply.id_token);
        let user_id = reply
            .local_id
            .filter(|id| !id.is_empty())
            .or_else(|| {
                claims
                    .as_ref()
                    .and_then(|claims| claims.get("sub"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .ok_or_else(|| {
                ClientError::Auth("Firebase response did not include a user id".into())
            })?;
        let email = claims
            .as_ref()
            .and_then(|claims| claims.get("email"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        Ok(WoozlitSession {
            user_id,
            email,
            tokens: AuthTokens {
                access_token: reply.id_token,
                refresh_token: reply.refresh_token,
            },
        })
    })
    .await
}

/// Refresh a Woozlit Firebase ID token.
pub async fn woozlit_refresh(refresh_token: &str) -> Result<AuthTokens> {
    if refresh_token.is_empty() {
        return Err(ClientError::Auth("session expired — sign in again".into()));
    }

    #[derive(Deserialize)]
    struct Reply {
        id_token: Option<String>,
        access_token: Option<String>,
        refresh_token: Option<String>,
    }

    let refresh_token = refresh_token.to_owned();
    let url = format!("https://securetoken.googleapis.com/v1/token?key={FIREBASE_API_KEY}");
    crate::runtime::run(async move {
        let response = http()
            .post(url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
            ])
            .send()
            .await
            .map_err(net)?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = format!("{status}: {body}");
            return Err(
                if matches!(
                    status,
                    reqwest::StatusCode::BAD_REQUEST
                        | reqwest::StatusCode::UNAUTHORIZED
                        | reqwest::StatusCode::FORBIDDEN
                ) {
                    ClientError::Auth(message)
                } else {
                    ClientError::Network(message)
                },
            );
        }
        let reply: Reply = response.json().await.map_err(net)?;
        let access_token = reply
            .id_token
            .filter(|token| !token.is_empty())
            .or(reply.access_token.filter(|token| !token.is_empty()))
            .ok_or_else(|| ClientError::Network("Firebase refresh omitted an ID token".into()))?;
        Ok(AuthTokens {
            access_token,
            refresh_token: reply
                .refresh_token
                .filter(|token| !token.is_empty())
                .unwrap_or(refresh_token),
        })
    })
    .await
}

/// Decode a JWT's `exp` claim (seconds). `None` for anything unparseable.
pub fn jwt_expiry(jwt: &str) -> Option<i64> {
    jwt_claims(jwt)?.get("exp")?.as_f64().map(|exp| exp as i64)
}

fn jwt_claims(jwt: &str) -> Option<serde_json::Value> {
    let mut segments = jwt.split('.');
    let (_, payload, _) = (segments.next()?, segments.next()?, segments.next()?);
    let bytes = base64url_decode(payload)?;
    serde_json::from_slice(&bytes).ok()
}

fn woozlit_id_token_claims(jwt: &str) -> Option<(String, Option<String>)> {
    let claims = jwt_claims(jwt)?;
    if !claims
        .get("iss")?
        .as_str()?
        .contains("securetoken.google.com")
    {
        return None;
    }
    let user_id = claims.get("sub")?.as_str()?.to_owned();
    if user_id.is_empty() {
        return None;
    }
    let email = claims
        .get("email")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Some((user_id, email))
}

/// Expired (or inside the early-refresh margin). Unparseable tokens read as
/// NOT expired — the server is the arbiter.
pub fn jwt_expired(jwt: &str, now_secs: i64) -> bool {
    jwt_expiry(jwt).is_some_and(|exp| now_secs > exp - EARLY_REFRESH_SECS)
}

fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        })
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for &c in input.as_bytes() {
        if c == b'=' {
            break;
        }
        buffer = (buffer << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[derive(Clone)]
enum RefreshMode {
    WorkOs { edge_url: String, org_id: String },
    Woozlit,
}

enum Mode {
    Demo,
    Dev {
        bearer: String,
    },
    WorkOs {
        edge_url: String,
        org_id: String,
        tokens: Arc<Mutex<AuthTokens>>,
        refresh_gate: Arc<tokio::sync::Mutex<()>>,
    },
    Woozlit {
        tokens: Arc<Mutex<AuthTokens>>,
        refresh_gate: Arc<tokio::sync::Mutex<()>>,
    },
}

/// Per-client bearer source: dev bearer, or a WorkOS/Woozlit access token
/// refreshed (single-flight) when inside the early-refresh margin.
pub(crate) struct TokenProvider {
    mode: Mode,
    events: Arc<EventPump>,
    expired: Arc<AtomicBool>,
}

impl TokenProvider {
    pub(crate) fn new(
        credentials: &crate::config::Credentials,
        edge_url: &str,
        events: Arc<EventPump>,
    ) -> Self {
        use crate::config::Credentials;
        let mode = match credentials {
            Credentials::Demo(_) => Mode::Demo,
            Credentials::Dev { user_id, org_id } => Mode::Dev {
                bearer: if org_id.is_empty() {
                    user_id.clone()
                } else {
                    format!("{user_id}@{org_id}")
                },
            },
            Credentials::WorkOs { org_id, tokens, .. } => Mode::WorkOs {
                edge_url: edge_url.to_owned(),
                org_id: org_id.clone(),
                tokens: Arc::new(Mutex::new(tokens.clone())),
                refresh_gate: Arc::new(tokio::sync::Mutex::new(())),
            },
            Credentials::Woozlit { tokens, .. } => Mode::Woozlit {
                tokens: Arc::new(Mutex::new(tokens.clone())),
                refresh_gate: Arc::new(tokio::sync::Mutex::new(())),
            },
        };
        Self {
            mode,
            events,
            expired: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Replace the pair (the platform re-signed-in or restored newer tokens).
    pub(crate) fn update_tokens(&self, next: AuthTokens) {
        match &self.mode {
            Mode::WorkOs { tokens, .. } | Mode::Woozlit { tokens, .. } => {
                *lock(tokens) = next;
                self.expired.store(false, Ordering::Release);
            }
            Mode::Demo | Mode::Dev { .. } => {}
        }
    }

    /// Current bearer, refreshing first when the access token is (nearly)
    /// expired. Network failures fall back to the stale token — the server
    /// rejects and the caller's backoff retries through here.
    pub(crate) async fn bearer(&self) -> Result<String> {
        let (refresh_mode, tokens, refresh_gate) = match &self.mode {
            Mode::Demo => {
                return Err(ClientError::Auth(
                    "demo mode has no edge credentials".into(),
                ));
            }
            Mode::Dev { bearer } => return Ok(bearer.clone()),
            Mode::WorkOs {
                edge_url,
                org_id,
                tokens,
                refresh_gate,
            } => (
                RefreshMode::WorkOs {
                    edge_url: edge_url.clone(),
                    org_id: org_id.clone(),
                },
                tokens,
                refresh_gate,
            ),
            Mode::Woozlit {
                tokens,
                refresh_gate,
            } => (RefreshMode::Woozlit, tokens, refresh_gate),
        };
        let now = chrono::Utc::now().timestamp();
        let current = lock(tokens).clone();
        if !jwt_expired(&current.access_token, now) {
            return Ok(current.access_token);
        }
        if self.expired.load(Ordering::Acquire) {
            return Err(ClientError::Auth("session expired".into()));
        }
        // The refresh runs as its own task that stores the rotated pair itself:
        // callers wrap this in timeouts, and dropping a refresh mid-flight
        // would strand us on a spent token.
        let task = refresh_task(
            refresh_mode,
            tokens.clone(),
            refresh_gate.clone(),
            self.events.clone(),
            self.expired.clone(),
        );
        match crate::runtime::shared().spawn(task).await {
            Ok(result) => result,
            Err(err) => Err(ClientError::Auth(format!("token refresh aborted: {err}"))),
        }
    }
}

/// Single-flight refresh (the gate is held for the whole exchange) that
/// persists its own result.
async fn refresh_task(
    mode: RefreshMode,
    tokens: Arc<Mutex<AuthTokens>>,
    gate: Arc<tokio::sync::Mutex<()>>,
    events: Arc<EventPump>,
    expired: Arc<AtomicBool>,
) -> Result<String> {
    let _gate = gate.lock_owned().await;
    // Joined an in-flight refresh: it already rotated the pair.
    let current = lock(&tokens).clone();
    if !jwt_expired(&current.access_token, chrono::Utc::now().timestamp()) {
        return Ok(current.access_token);
    }
    if expired.load(Ordering::Acquire) {
        return Err(ClientError::Auth("session expired".into()));
    }
    let refreshed = match mode {
        RefreshMode::WorkOs { edge_url, org_id } => {
            refresh(&edge_url, &current.refresh_token, Some(&org_id)).await
        }
        RefreshMode::Woozlit => woozlit_refresh(&current.refresh_token).await,
    };
    match refreshed {
        Ok(next) => {
            *lock(&tokens) = next.clone();
            events.ordered(ClientEvent::AuthRefreshed(next.clone()));
            Ok(next.access_token)
        }
        Err(ClientError::Auth(reason)) => {
            if !expired.swap(true, Ordering::AcqRel) {
                events.ordered(ClientEvent::AuthExpired {
                    reason: reason.clone(),
                });
            }
            Err(ClientError::Auth(reason))
        }
        Err(err) => {
            tracing::warn!(error = %err, "token refresh failed; using the stale access token");
            Ok(current.access_token)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_handles_edges() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("end%41"), "endA");
        assert_eq!(percent_decode("%aé"), "%aé");
        assert_eq!(percent_decode("%"), "%");
    }

    #[test]
    fn authorize_url_and_callback_round_trip() {
        let url = workos_authorize_url("s t&1");
        assert!(
            url.starts_with("https://api.workos.com/user_management/authorize?response_type=code")
        );
        assert!(url.contains("redirect_uri=dove%3A%2F%2Fcallback"));
        assert!(url.contains("state=s%20t%261"));
        assert_eq!(
            parse_auth_callback("dove://callback?code=abc&state=s%20t%261"),
            Some(AuthCallback::Code {
                code: "abc".into(),
                state: Some("s t&1".into())
            })
        );
        assert!(matches!(
            parse_auth_callback("dove://callback?error=access_denied"),
            Some(AuthCallback::Error { .. })
        ));
        assert_eq!(parse_auth_callback("dove://callback"), None);
    }

    #[test]
    fn woozlit_sign_in_url_encodes_the_stateful_callback() {
        assert_eq!(
            woozlit_sign_in_url("s t&1"),
            "https://www.woozlit.com/login?desktop=1&callback=dove%3A%2F%2Fauth%2Fcallback%3Fstate%3Ds%20t%261"
        );
    }

    #[test]
    fn woozlit_id_token_claims_require_firebase_issuer_and_read_profile() {
        let token = "eyJhbGciOiJub25lIn0.eyJpc3MiOiJodHRwczovL3NlY3VyZXRva2VuLmdvb2dsZS5jb20vcHJvamVjdCIsInN1YiI6ImZpcmViYXNlLXVpZCIsImVtYWlsIjoidXNlckBleGFtcGxlLmNvbSJ9.c2ln";
        assert_eq!(
            woozlit_id_token_claims(token),
            Some(("firebase-uid".into(), Some("user@example.com".into())))
        );
        let other_issuer =
            "eyJhbGciOiJub25lIn0.eyJpc3MiOiJvdGhlciIsInN1YiI6ImZpcmViYXNlLXVpZCJ9.c2ln";
        assert_eq!(woozlit_id_token_claims(other_issuer), None);
    }

    #[test]
    fn woozlit_credentials_use_uid_as_org_id() {
        let credentials = crate::config::Credentials::Woozlit {
            user_id: "firebase-uid".into(),
            tokens: AuthTokens {
                access_token: "id-token".into(),
                refresh_token: String::new(),
            },
        };
        assert_eq!(credentials.org_id(), "firebase-uid");
        assert_eq!(credentials.user_id(), "firebase-uid");
    }

    /// A raw HTTP/1.1 responder: counts requests, answers each with `reply`.
    async fn serve_http(
        status: u16,
        body: &'static str,
    ) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let _ = socket.read(&mut buf).await;
                    counter.fetch_add(1, Ordering::SeqCst);
                    // Slow enough that concurrent callers overlap.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let response = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        (url, hits)
    }

    struct Collect(Mutex<Vec<ClientEvent>>);

    impl crate::events::ClientListener for Collect {
        fn on_event(&self, event: ClientEvent) {
            lock(&self.0).push(event);
        }
    }

    fn expired_jwt() -> String {
        // {"alg":"none"}.{"exp":1000}.sig — long expired.
        "eyJhbGciOiJub25lIn0.eyJleHAiOjEwMDB9.c2ln".to_owned()
    }

    fn provider(edge: &str) -> (TokenProvider, Arc<Collect>) {
        let collect = Arc::new(Collect(Mutex::new(Vec::new())));
        let events = EventPump::new(collect.clone());
        events.start(tokio_util::sync::CancellationToken::new());
        let credentials = crate::config::Credentials::WorkOs {
            user_id: "u".into(),
            org_id: "org_1".into(),
            tokens: AuthTokens {
                access_token: expired_jwt(),
                refresh_token: "r1".into(),
            },
        };
        (TokenProvider::new(&credentials, edge, events), collect)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refresh_is_single_flight_and_reported() {
        let (edge, hits) =
            serve_http(200, r#"{"accessToken":"fresh-access","refreshToken":"r2"}"#).await;
        let (tokens, events) = provider(&edge);
        let tokens = Arc::new(tokens);
        let calls = (0..8).map(|_| {
            let tokens = tokens.clone();
            tokio::spawn(async move { tokens.bearer().await })
        });
        for call in futures::future::join_all(calls).await {
            assert_eq!(call.unwrap().unwrap(), "fresh-access");
        }
        // The fresh token has no parseable exp → treated valid: one refresh.
        assert_eq!(hits.load(Ordering::SeqCst), 1, "single-flight refresh");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(lock(&events.0).iter().any(|e| matches!(
            e,
            ClientEvent::AuthRefreshed(t) if t.refresh_token == "r2"
        )));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rejected_refresh_expires_the_session_once() {
        let (edge, _hits) = serve_http(401, r#"{"error":"invalid_grant"}"#).await;
        let (tokens, events) = provider(&edge);
        assert!(matches!(tokens.bearer().await, Err(ClientError::Auth(_))));
        assert!(matches!(tokens.bearer().await, Err(ClientError::Auth(_))));
        tokio::time::sleep(Duration::from_millis(100)).await;
        let expired = lock(&events.0)
            .iter()
            .filter(|e| matches!(e, ClientEvent::AuthExpired { .. }))
            .count();
        assert_eq!(expired, 1);
        // New tokens from the platform revive it.
        tokens.update_tokens(AuthTokens {
            access_token: "x.y.z".into(),
            refresh_token: "r9".into(),
        });
        assert_eq!(tokens.bearer().await.unwrap(), "x.y.z");
    }

    #[tokio::test]
    async fn dev_bearer_is_user_at_org() {
        let events = EventPump::new(Arc::new(crate::events::NullListener));
        let credentials = crate::config::Credentials::Dev {
            user_id: "wing".into(),
            org_id: "acme".into(),
        };
        let tokens = TokenProvider::new(&credentials, "http://unused", events);
        assert_eq!(tokens.bearer().await.unwrap(), "wing@acme");
    }

    #[test]
    fn jwt_expiry_reads_the_exp_claim() {
        // {"alg":"none"}.{"exp":1000}.sig
        let jwt = "eyJhbGciOiJub25lIn0.eyJleHAiOjEwMDB9.c2ln";
        assert_eq!(jwt_expiry(jwt), Some(1000));
        assert!(jwt_expired(jwt, 1000 - EARLY_REFRESH_SECS + 1));
        assert!(!jwt_expired(jwt, 900));
        assert!(!jwt_expired("not-a-jwt", i64::MAX));
    }
}
