//! A cloud thread, stored the way Zeron stores one.
//!
//! The laptop keeps this list so it can paint while the edge is quiet. The
//! transcript stays on the session machine. Local threads are never written
//! here. A row is only added after a Woozlit session exists and the thread
//! was started on Cloud.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::hosts::dove_home;

/// Where a cloud thread sits. Asleep and starting are the states the UI
/// describes with Zeron's sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudState {
    Off,
    Provisioning,
    Starting,
    Ready,
    Sleeping,
    Stopping,
    Deleting,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSession {
    pub chat_id: String,
    /// The session machine. Also the chat row's host.
    pub device_id: String,
    pub space_id: String,
    /// `owner/name`, when the thread is a GitHub repo.
    pub repo: Option<String>,
    pub path: Option<String>,
    pub state: CloudState,
    pub error: Option<String>,
    pub failed_action: Option<String>,
    pub last_active_at: i64,
    pub created_at: i64,
}

fn path() -> PathBuf {
    dove_home().join("cloud-sessions.json")
}

pub fn load() -> Vec<CloudSession> {
    std::fs::read(path()).ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default()
}

/// Replace the list. Local threads are not part of it.
pub fn save(sessions: &[CloudSession]) -> Result<()> {
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(sessions)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let row = CloudSession {
            chat_id: "c1".into(),
            device_id: "d1".into(),
            space_id: "s1".into(),
            repo: Some("PupppyLoverr/Dove".into()),
            path: None,
            state: CloudState::Sleeping,
            error: None,
            failed_action: None,
            last_active_at: 1,
            created_at: 1,
        };
        let raw = serde_json::to_vec(&row).unwrap();
        let back: CloudSession = serde_json::from_slice(&raw).unwrap();
        assert_eq!(back, row);
        assert!(back.error.is_none());
    }
}
