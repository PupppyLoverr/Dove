//! Sign-in for remote MCP servers (MCP authorization, 2025-06-18).
//!
//! Dove signs in once per server and its own MCP server (`dove
//! --mcp-browser`) sends the token upstream, so every harness gets the
//! connection without its own OAuth flow.
//!
//! The flow: probe the server for its Protected Resource Metadata (RFC 9728),
//! read the authorization server's metadata (RFC 8414 / OIDC discovery),
//! register Dove as a public client (RFC 7591) unless a client id is known,
//! then authorization code + PKCE (S256) through the user's browser with a
//! one-shot loopback redirect on 127.0.0.1. Tokens live in
//! `<data>/mcp_auth.json` (0600 on Unix), keyed by the server's id; the bridge
//! refreshes them when they expire or the server answers 401.

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How long the browser has to come back to the loopback redirect.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Grant {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix seconds; `None` when the server did not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    pub token_endpoint: String,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// The canonical MCP server URL the token was issued for (RFC 8707).
    pub resource: String,
}

fn store_path() -> PathBuf {
    crate::pages::integrations::data_root().join("mcp_auth.json")
}

fn load_all() -> BTreeMap<String, Grant> {
    std::fs::read_to_string(store_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_all(all: &BTreeMap<String, Grant>) -> Result<(), String> {
    let path = store_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(all).unwrap_or_default();
    // Write aside and rename, so the bridge never reads half a file.
    let tmp = path.with_extension(format!("json.{}", std::process::id()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn grant(id: &str) -> Option<Grant> {
    load_all().remove(id)
}

pub fn signed_in(id: &str) -> bool {
    grant(id).is_some_and(|g| !g.access_token.is_empty())
}

pub fn sign_out(id: &str) {
    let mut all = load_all();
    if all.remove(id).is_some() {
        let _ = save_all(&all);
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A usable access token for `id`, refreshed first when it is about to
/// expire. `None` when Dove never signed in to it.
pub fn token(id: &str) -> Option<String> {
    let g = grant(id)?;
    let stale = g.expires_at.is_some_and(|t| t <= now() + 60);
    if stale && g.refresh_token.is_some() {
        return refresh(id).ok();
    }
    Some(g.access_token)
}

/// Trade the refresh token for a new access token and store it.
pub fn refresh(id: &str) -> Result<String, String> {
    let g = grant(id).ok_or("not signed in")?;
    let refresh = g.refresh_token.clone().ok_or("this sign-in cannot be refreshed; sign in again")?;
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh.clone()),
        ("client_id", g.client_id.clone()),
        ("resource", g.resource.clone()),
    ];
    if let Some(secret) = &g.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    let reply = post_form(&g.token_endpoint, &form)?;
    let mut next = token_from(&reply, g)?;
    // Servers that don't rotate refresh tokens leave it out of the reply.
    if next.refresh_token.is_none() {
        next.refresh_token = Some(refresh);
    }
    let access = next.access_token.clone();
    let mut all = load_all();
    all.insert(id.to_string(), next);
    save_all(&all)?;
    Ok(access)
}

fn token_from(reply: &Value, base: Grant) -> Result<Grant, String> {
    let access = reply.get("access_token").and_then(Value::as_str).ok_or_else(|| {
        let err = reply.get("error_description").or_else(|| reply.get("error")).and_then(Value::as_str);
        format!("the server returned no token{}", err.map(|e| format!(": {e}")).unwrap_or_default())
    })?;
    Ok(Grant {
        access_token: access.to_string(),
        refresh_token: reply.get("refresh_token").and_then(Value::as_str).map(str::to_string),
        expires_at: reply.get("expires_in").and_then(Value::as_u64).map(|s| now() + s),
        ..base
    })
}

/// A sign-in waiting for the browser: open `authorize_url`, then call
/// [`Pending::finish`] off the UI thread.
pub struct Pending {
    pub authorize_url: String,
    id: String,
    listener: TcpListener,
    redirect: String,
    state: String,
    verifier: String,
    base: Grant,
}

/// Discover, register and build the authorization URL for the MCP server
/// at `url`. `client_id` skips registration for servers that need a
/// pre-registered client; `port` pins the loopback port they registered.
pub fn begin(id: &str, url: &str, client_id: Option<&str>, port: Option<u16>) -> Result<Pending, String> {
    let resource = canonical(url)?;
    let prm = protected_resource_metadata(url);
    let issuer = prm
        .as_ref()
        .and_then(|m| m.pointer("/authorization_servers/0"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| origin(url).unwrap_or_default());
    let meta = authorization_server_metadata(&issuer);
    let endpoint = |key: &str, default: &str| {
        meta.as_ref()
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}{default}", issuer.trim_end_matches('/')))
    };
    let authorization_endpoint = endpoint("authorization_endpoint", "/authorize");
    let token_endpoint = endpoint("token_endpoint", "/token");
    let scope = prm
        .as_ref()
        .and_then(|m| m.get("scopes_supported"))
        .and_then(Value::as_array)
        .map(|s| s.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty());

    let listener = TcpListener::bind(("127.0.0.1", port.unwrap_or(0)))
        .map_err(|e| format!("could not open the sign-in callback port: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");

    let (client_id, client_secret) = match client_id {
        Some(c) => (c.to_string(), None),
        None => {
            let registration = meta
                .as_ref()
                .and_then(|m| m.get("registration_endpoint"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or("this server does not let apps register for sign-in; add it with an API key instead")?;
            register(&registration, &redirect, scope.as_deref())?
        }
    };

    let verifier = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let state = uuid::Uuid::new_v4().simple().to_string();
    let challenge = base64url(&sha256(verifier.as_bytes()));
    let mut authorize = url::Url::parse(&authorization_endpoint).map_err(|e| format!("bad authorization URL: {e}"))?;
    {
        let mut q = authorize.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", &client_id)
            .append_pair("redirect_uri", &redirect)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("resource", &resource);
        if let Some(scope) = &scope {
            q.append_pair("scope", scope);
        }
    }
    Ok(Pending {
        authorize_url: authorize.to_string(),
        id: id.to_string(),
        listener,
        redirect,
        state,
        verifier,
        base: Grant { token_endpoint, client_id, client_secret, resource, ..Grant::default() },
    })
}

impl Pending {
    /// Wait for the browser's redirect, trade the code for tokens and store
    /// them. Blocks up to five minutes.
    pub fn finish(self) -> Result<(), String> {
        let code = self.wait_for_code()?;
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code),
            ("redirect_uri", self.redirect.clone()),
            ("client_id", self.base.client_id.clone()),
            ("code_verifier", self.verifier.clone()),
            ("resource", self.base.resource.clone()),
        ];
        if let Some(secret) = &self.base.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        let reply = post_form(&self.base.token_endpoint, &form)?;
        let grant = token_from(&reply, self.base)?;
        let mut all = load_all();
        all.insert(self.id, grant);
        save_all(&all)
    }

    fn wait_for_code(&self) -> Result<String, String> {
        self.listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + SIGN_IN_TIMEOUT;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(result) = self.handle(stream) {
                        return result;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() > deadline {
                        return Err("sign-in timed out".into());
                    }
                    std::thread::sleep(Duration::from_millis(120));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    /// One request on the loopback port. `None` for strays (favicon and the
    /// like), so the wait goes on.
    fn handle(&self, mut stream: TcpStream) -> Option<Result<String, String>> {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).ok()?;
        let target = line.split_whitespace().nth(1)?;
        if !target.starts_with("/callback") {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
            return None;
        }
        let parsed = url::Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
        let q: BTreeMap<String, String> = parsed.query_pairs().into_owned().collect();
        let result = if q.get("state") != Some(&self.state) {
            Err("the sign-in reply did not match this request".to_string())
        } else if let Some(code) = q.get("code") {
            Ok(code.clone())
        } else {
            Err(q
                .get("error_description")
                .or_else(|| q.get("error"))
                .cloned()
                .unwrap_or_else(|| "sign-in was cancelled".into()))
        };
        let (title, body) = match &result {
            Ok(_) => ("Signed in", "Dove is connected. You can close this tab."),
            Err(_) => ("Sign-in failed", "Dove could not finish signing in. Go back to Dove and try again."),
        };
        let page = format!(
            "<!doctype html><meta charset=utf-8><title>{title}</title><body style=\"font:15px system-ui;\
             margin:15vh auto;max-width:28em;text-align:center\"><h2>{title}</h2><p>{body}</p>"
        );
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
            page.len()
        );
        Some(result)
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(30)).build().unwrap_or_default()
}

fn get_json(url: &str) -> Option<Value> {
    runtime().block_on(async {
        let resp = client().get(url).header("accept", "application/json").send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        serde_json::from_str(&resp.text().await.ok()?).ok()
    })
}

fn post_form(url: &str, form: &[(&str, String)]) -> Result<Value, String> {
    let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(form.iter().map(|(k, v)| (*k, v.as_str()))).finish();
    runtime().block_on(async {
        let resp = client()
            .post(url)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("could not reach {url}: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&text)
            .or_else(|_| {
                // A few servers still answer form-encoded.
                let pairs: serde_json::Map<String, Value> = url::form_urlencoded::parse(text.as_bytes())
                    .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
                    .collect();
                if pairs.is_empty() { Err(()) } else { Ok(Value::Object(pairs)) }
            })
            .map_err(|_| format!("{url} answered {status}"))?;
        Ok(v)
    })
}

/// RFC 7591 registration as a public client. Returns (client_id, secret).
fn register(endpoint: &str, redirect: &str, scope: Option<&str>) -> Result<(String, Option<String>), String> {
    let mut body = serde_json::json!({
        "client_name": "Dove",
        "client_uri": "https://github.com/PupppyLoverr/dove",
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "native",
    });
    if let Some(scope) = scope {
        body["scope"] = Value::String(scope.to_string());
    }
    let v: Value = runtime().block_on(async {
        let resp = client()
            .post(endpoint)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("could not register with {endpoint}: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("registration was refused ({status}): {}", text.chars().take(200).collect::<String>()));
        }
        serde_json::from_str(&text).map_err(|e| e.to_string())
    })?;
    let id = v.get("client_id").and_then(Value::as_str).ok_or("registration returned no client id")?;
    Ok((id.to_string(), v.get("client_secret").and_then(Value::as_str).map(str::to_string)))
}

/// The server's Protected Resource Metadata: from the `resource_metadata`
/// hint on an unauthenticated request's 401, else the well-known paths.
fn protected_resource_metadata(url: &str) -> Option<Value> {
    let hinted = runtime().block_on(async {
        let resp = client()
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(r#"{"jsonrpc":"2.0","id":0,"method":"ping"}"#)
            .send()
            .await
            .ok()?;
        let header = resp.headers().get("www-authenticate")?.to_str().ok()?.to_string();
        resource_metadata_hint(&header)
    });
    if let Some(found) = hinted.and_then(|u| get_json(&u)) {
        return Some(found);
    }
    let parsed = url::Url::parse(url).ok()?;
    let origin = origin(url)?;
    let path = parsed.path().trim_end_matches('/');
    let mut candidates = Vec::new();
    if !path.is_empty() {
        candidates.push(format!("{origin}/.well-known/oauth-protected-resource{path}"));
    }
    candidates.push(format!("{origin}/.well-known/oauth-protected-resource"));
    candidates.into_iter().find_map(|u| get_json(&u))
}

/// `resource_metadata="…"` from a `WWW-Authenticate: Bearer …` header.
pub fn resource_metadata_hint(header: &str) -> Option<String> {
    let at = header.find("resource_metadata=")? + "resource_metadata=".len();
    let rest = &header[at..];
    let value = if let Some(quoted) = rest.strip_prefix('"') {
        quoted.split('"').next()?
    } else {
        rest.split([',', ' ']).next()?
    };
    (!value.is_empty()).then(|| value.to_string())
}

/// RFC 8414 metadata for `issuer`, trying the OAuth path first, then OIDC
/// (both path-inserted and path-appended forms).
fn authorization_server_metadata(issuer: &str) -> Option<Value> {
    metadata_candidates(issuer).into_iter().find_map(|u| get_json(&u))
}

pub fn metadata_candidates(issuer: &str) -> Vec<String> {
    let Some(origin) = origin(issuer) else { return Vec::new() };
    let path = url::Url::parse(issuer).map(|u| u.path().trim_end_matches('/').to_string()).unwrap_or_default();
    if path.is_empty() {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server"),
            format!("{origin}/.well-known/openid-configuration"),
        ]
    } else {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server{path}"),
            format!("{origin}/.well-known/openid-configuration{path}"),
            format!("{origin}{path}/.well-known/openid-configuration"),
        ]
    }
}

fn origin(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?;
    Some(match u.port() {
        Some(p) => format!("{}://{host}:{p}", u.scheme()),
        None => format!("{}://{host}", u.scheme()),
    })
}

/// The canonical resource URI (RFC 8707 / MCP): lowercase scheme and host,
/// no fragment, no trailing slash on a bare path.
pub fn canonical(url: &str) -> Result<String, String> {
    let mut u = url::Url::parse(url).map_err(|e| format!("bad server URL: {e}"))?;
    u.set_fragment(None);
    let mut s = u.to_string();
    if u.path() == "/" && u.query().is_none() {
        s.pop();
    }
    Ok(s)
}

fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(data).into()
}

/// Base64url without padding (RFC 7636 §4.2).
pub fn base64url(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(base64url(&sha256(verifier.as_bytes())), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn base64url_pads_nothing() {
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn reads_resource_metadata_hint() {
        let h = r#"Bearer error="invalid_token", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource""#;
        assert_eq!(
            resource_metadata_hint(h).as_deref(),
            Some("https://mcp.example.com/.well-known/oauth-protected-resource")
        );
        assert_eq!(resource_metadata_hint("Bearer realm=x").as_deref(), None);
        assert_eq!(resource_metadata_hint("Bearer resource_metadata=https://a.b/m, x=y").as_deref(), Some("https://a.b/m"));
    }

    #[test]
    fn metadata_paths_follow_rfc8414() {
        assert_eq!(
            metadata_candidates("https://auth.example.com"),
            vec![
                "https://auth.example.com/.well-known/oauth-authorization-server",
                "https://auth.example.com/.well-known/openid-configuration"
            ]
        );
        assert_eq!(
            metadata_candidates("https://example.com/tenant1/")[0],
            "https://example.com/.well-known/oauth-authorization-server/tenant1"
        );
    }

    #[test]
    fn canonical_resource() {
        assert_eq!(canonical("https://MCP.Example.com/").unwrap(), "https://mcp.example.com");
        assert_eq!(canonical("https://mcp.example.com/mcp#x").unwrap(), "https://mcp.example.com/mcp");
    }

    #[test]
    fn token_reply_keeps_client() {
        let base = Grant { client_id: "c".into(), token_endpoint: "https://t".into(), ..Grant::default() };
        let g = token_from(&serde_json::json!({"access_token": "a", "expires_in": 60}), base).unwrap();
        assert_eq!((g.access_token.as_str(), g.client_id.as_str()), ("a", "c"));
        assert!(g.expires_at.is_some());
        assert!(token_from(&serde_json::json!({"error": "invalid_grant"}), Grant::default()).is_err());
    }
}
