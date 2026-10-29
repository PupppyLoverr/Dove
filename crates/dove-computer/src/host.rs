//! What the `computer` tool needs from the app it runs in: the user's
//! decisions, the human-takeover lease, process identities, OCR and a
//! cache folder. The app implements it over its GUI channel; tests fake it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::policy::AppIdentity;

/// Which grant an action needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Background input into an app: one grant per app per session.
    Background,
    /// Bringing a window forward or foreground input: its own grant.
    Foreground,
    /// A control that sends, buys, deletes...: reviewed every time.
    Consequential,
}

/// One request for the user's approval.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub scope: Scope,
    /// The app acted on, as the driver names it.
    pub app: String,
    /// One line per action ("click Save", "type 'hello'").
    pub actions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// Just this call.
    Once,
    /// This call and later ones in the same scope and app, this session.
    Session,
    Deny {
        reason: String,
    },
    /// Nobody can answer (no Dove window, an unattended run): refused.
    Unavailable {
        reason: String,
    },
}

/// Who controls the desktop. `epoch` moves on every take-over and hand-back,
/// so a result produced across one is discarded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub epoch: u64,
    pub human_control: bool,
}

/// One line of OCR text, in the image's pixel space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OcrLine {
    pub text: String,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

pub trait Host {
    /// Ask the user; blocks until they answer or the request times out.
    fn review(&mut self, review: &Review) -> Decision;
    fn lease(&mut self) -> Lease;
    /// What process `pid` is (name, path, bundle id, verified signer).
    fn identify(&mut self, pid: u32, app: &str) -> AppIdentity;
    /// Whether the field input would land in is a password field, asked of
    /// the OS accessibility API directly (`label`: the target row's label,
    /// when known). None when the platform cannot tell.
    fn secure_target(&mut self, pid: u32, window_id: u64, label: Option<&str>) -> Option<bool>;
    /// Text lines read off an encoded PNG or JPEG.
    fn ocr(&mut self, image: &[u8]) -> Result<Vec<OcrLine>, String>;
    /// Milliseconds since the user last touched mouse or keyboard.
    fn idle_ms(&mut self) -> Option<u64>;
    /// The agent is working in `app` (for the thread's activity line).
    fn activity(&mut self, app: &str, summary: &str);
    /// Where captures, element spills and replays are written (owner-only).
    fn cache_dir(&self) -> PathBuf;
}
