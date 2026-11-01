//! The cosmos-ade GitHub App, as far as the desktop is allowed to know it.
//!
//! Device flow needs no client secret, so the client id may ship in the app.
//! The App private key and the webhook secret stay on the Cloudflare worker.
//! A token this flow returns is written only to `cloud-keys.json` and is
//! never logged or sent back to the UI.

use anyhow::{Result, bail};
use serde::Deserialize;

/// Public client id of the `cosmos-ade` GitHub App. Device flow has no secret.
pub const CLIENT_ID: &str = "Iv23lijEVv5jdl2f7RTm";

/// Where the user installs the App on a GitHub owner.
pub const INSTALL_URL: &str = "https://github.com/apps/cosmos-ade/installations/new";

#[derive(Debug, Clone)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
}

#[derive(Deserialize)]
struct DeviceCodeBody {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Deserialize)]
struct TokenBody {
    access_token: Option<String>,
    error: Option<String>,
}

/// Begin GitHub's device flow. The user code is what the UI shows.
pub fn device_start() -> Result<DeviceStart> {
    let body = format!("client_id={CLIENT_ID}");
    let resp = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?
        .post("https://github.com/login/device/code")
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()?
        .error_for_status()?;
    let parsed: DeviceCodeBody = resp.json()?;
    if parsed.user_code.is_empty() || parsed.device_code.is_empty() {
        bail!("GitHub did not return a device code");
    }
    Ok(DeviceStart {
        device_code: parsed.device_code,
        user_code: parsed.user_code,
        verification_uri: parsed.verification_uri,
        interval: parsed.interval.max(5),
    })
}

/// One poll of the device-code grant.
/// `Ok(None)` means the user has not finished yet.
/// `Ok(Some(token))` is the user-to-server token; the caller stores it and
/// must not log it.
pub fn device_poll(device_code: &str) -> Result<Option<String>> {
    let body = format!(
        "client_id={CLIENT_ID}&device_code={device_code}&grant_type=urn:ietf:params:oauth:grant-type:device_code"
    );
    let resp = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()?
        .error_for_status()?;
    let parsed: TokenBody = resp.json()?;
    if let Some(token) = parsed.access_token.filter(|t| !t.is_empty()) {
        return Ok(Some(token));
    }
    match parsed.error.as_deref() {
        Some("authorization_pending" | "slow_down") | None => Ok(None),
        Some("expired_token") => bail!("That GitHub code expired. Connect again."),
        Some("access_denied") => bail!("GitHub sign-in was declined."),
        Some(other) => bail!("GitHub: {other}"),
    }
}
