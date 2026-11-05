//! The chat example (examples/chat), run headless against its in-memory
//! model: a question sent with Enter and its reply streaming in as
//! markdown, stopping a reply, a failed one's error, conversations made,
//! switched and deleted, and each one keeping its draft.
mod common;

#[path = "../examples/support/chat_server.rs"]
mod chat_server;

use chat_server::ChatServer;
use common::Harness;
use std::time::Duration;
use zgx::input::Key;
use zgx_script::ScriptHost;

const APP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/chat/app.luau");

fn chat(server: &ChatServer) -> Harness {
    let server = server.clone();
    let h = Harness::new(move |runtime| {
        let host = ScriptHost::new(runtime, APP);
        server.serve(&host);
        host
    });
    h.ui.scene.borrow_mut().resize(1000., 720.);
    h.frames(2);
    h
}
/// An empty composer shows its placeholder.
const EMPTY: &str = "Ask anything";

fn texts(h: &mut zgx_script::testing::TestApp) -> String {
    h.texts().join("\n")
}

#[test]
fn a_question_sent_with_enter_streams_its_reply_in_as_markdown() {
    let server = ChatServer::new(Duration::from_millis(1));
    let mut h = chat(&server);
    assert_eq!(h.error(), None);
    server.hold(true);
    h.type_text("composer", "Write me a function");
    h.key(Key::Enter);
    // The question shows, the composer empties, and the reply begins.
    assert!(h.wait_until(|h| texts(h).contains("Write me a function")));
    assert_eq!(h.text("composer"), EMPTY);
    assert!(h.wait_until(|h| texts(h).contains("Here")));
    assert!(h.shown("stop"));
    assert!(
        !texts(&mut h).contains("words in a text"),
        "held after its first word"
    );
    server.hold(false);
    assert!(h.wait_until(|h| h.shown("send")));
    let all = texts(&mut h);
    assert!(all.contains("fn words(text: &str) -> usize"), "{all}");
    assert!(all.contains("rust"), "the code block's language");
    // The model was sent the conversation so far.
    let asked = server.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].last().unwrap().1, "Write me a function");
    assert_eq!(asked[0].len(), 3, "the welcome's two messages first");
    // A new chat is named for its first question.
    // ⌘N (the menu bar runs the commands).
    assert!(h.host.run_command("new"));
    h.type_text("composer", "Make it faster");
    h.key(Key::Enter);
    assert!(h.wait_until(|h| h.shown("send") && texts(h).contains("Three ways")));
    assert_eq!(h.text("title"), "Make it faster");
    assert_eq!(
        server.asked()[1].len(),
        1,
        "a new chat sends only its question"
    );
    assert_eq!(h.error(), None);
}

#[test]
fn a_reply_stops_and_a_failed_one_says_why() {
    let server = ChatServer::new(Duration::from_millis(1));
    let mut h = chat(&server);
    server.hold(true);
    h.type_text("composer", "Tell me something");
    h.key(Key::Enter);
    assert!(h.wait_until(|h| h.shown("stop")));
    // Esc stops it: what came stays, marked stopped.
    assert!(h.host.run_command("stop"));
    assert!(h.wait_until(|h| h.shown("send")));
    assert!(texts(&mut h).contains("Stopped"));
    server.hold(false);
    server.fail(1);
    h.type_text("composer", "Again");
    h.key(Key::Enter);
    assert!(h.wait_until(|h| texts(h).contains("The model is busy")));
    assert!(h.shown("send"));
    assert_eq!(h.error(), None);
}

#[test]
fn conversations_switch_keep_their_drafts_and_delete() {
    let server = ChatServer::new(Duration::from_millis(1));
    let mut h = chat(&server);
    h.type_text("composer", "half a thought");
    h.click("new");
    h.frames(2);
    assert_eq!(h.text("composer"), EMPTY, "a new chat starts empty");
    assert_eq!(h.text("title"), "New chat");
    // Back to the welcome: its draft is as it was left.
    assert!(h.host.run_command("next"));
    h.frames(2);
    assert_eq!(h.text("title"), "Welcome");
    assert_eq!(h.text("composer"), "half a thought");
    // The search finds by title.
    h.type_text("search", "new");
    h.frames(2);
    assert!(h.find("conversation-c1").is_none());
    assert!(h.find("conversation-c3").is_some());
    h.click("conversation-c3");
    h.frames(2);
    assert_eq!(h.text("title"), "New chat");
    // Deleted, the next one shows.
    assert!(h.host.run_command("delete"));
    h.frames(2);
    assert!(h.find("conversation-c3").is_none());
    assert_eq!(h.text("title"), "Welcome");
    assert_eq!(h.error(), None);
}
