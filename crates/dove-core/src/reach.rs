//! Deliver a short note to ntfy and Telegram when a turn ends.
//!
//! The topic and the bot token live in `$DOVE_HOME/cloud-keys.json` under
//! `ntfy` and `telegram` (`bot-token|chat-id`). They are never copied into an
//! agent's environment and never written to the log.

use std::time::Duration;

/// Post `text` to whichever chat apps the user configured. Best-effort:
/// a failure is not logged, because the HTTP error can carry the token.
pub fn deliver(title: &str, text: &str) {
    let keys = dove_harness::hosts::Keys::load();
    if let Some(topic) = keys.get("ntfy").map(str::to_string) {
        post_ntfy(&topic, title, text);
    }
    if let Some(spec) = keys.get("telegram").map(str::to_string) {
        post_telegram(&spec, &format!("{title}\n{text}"));
    }
}

fn client() -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder().timeout(Duration::from_secs(8)).build().ok()
}

fn post_ntfy(topic: &str, title: &str, text: &str) {
    let Some(client) = client() else { return };
    let url = if topic.starts_with("https://") || topic.starts_with("http://") {
        topic.to_string()
    } else {
        format!("https://ntfy.sh/{topic}")
    };
    let _ = client.post(url).header("Title", title).body(text.to_string()).send();
}

fn post_telegram(spec: &str, text: &str) {
    let Some((token, chat)) = spec.split_once('|') else { return };
    let token = token.trim();
    let chat = chat.trim();
    if token.is_empty() || chat.is_empty() {
        return;
    }
    let Some(client) = client() else { return };
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let _ = client.post(url).json(&serde_json::json!({ "chat_id": chat, "text": text })).send();
}
