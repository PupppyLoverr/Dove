//! The chat example's model, served in memory as a streaming fetcher: a
//! reply to the last message, in markdown (headings, lists, inline code
//! and a code block), streamed a token at a time as server-sent events
//! (`event: delta`, `data: {"text": …}`), `event: done` at the end, as
//! hosted models stream. `fail(n)` makes the next `n` replies fail (a
//! quota refused), `hold(true)` holds every reply after its first token
//! until `hold(false)`, and `asked()` is every conversation it was sent.
//! `manual()` streams only what its caller writes (`write`, `finish`):
//! benchmarks feed it a token a frame, without the clock.
// Shared with its tests and benchmarks, which use what the example does not.
#![allow(dead_code)]
use std::{cell::RefCell, rc::Rc, time::Duration};
use zgx_script::{Body, Request, Response, ScriptHost, Value};

const REPLIES: [&str; 3] = [
    "Here is a function that counts the words in a text:\n\n\
```rust\n\
fn words(text: &str) -> usize {\n    text.split_whitespace().count()\n}\n\
```\n\n\
It splits on any run of whitespace, so `\"a  b\"` counts **two** words. \
For a stream of text, count as each chunk arrives and keep the last \
partial word for the next one.",
    "## Three ways to make it faster\n\n\
1. **Measure first**: profile the slow path rather than guessing.\n\
2. Do less work: cache what does not change, and skip what is not shown.\n\
3. Batch writes, so each frame lays out once.\n\n\
> The fastest code is the code that never runs.\n\n\
Which of these fits your case?",
    "Sure. A retained-mode interface keeps its tree between frames and \
changes only what state touched, so an idle window does *no* work at all. \
An immediate-mode one builds the whole interface every frame, which is \
simple to write and costs CPU even when nothing moves.\n\n\
- Retained: `ui.text { \"{count}\" }` updates one node.\n\
- Immediate: every widget is asked again each frame.",
];

#[derive(Default)]
struct Inner {
    failing: u32,
    held: bool,
    asked: Vec<Vec<(String, String)>>,
    replies: u64,
    /// A manual model's reply under way, and whether it has been finished.
    live: Option<Body>,
    finished: bool,
}

/// The model and its switches; clones share them.
#[derive(Clone)]
pub struct ChatServer {
    inner: Rc<RefCell<Inner>>,
    /// Between tokens; none for a manual model.
    pace: Option<Duration>,
}

impl ChatServer {
    /// A model writing a token every `pace`.
    pub fn new(pace: Duration) -> Self {
        Self {
            inner: Rc::default(),
            pace: Some(pace),
        }
    }
    /// A model whose replies stream only what `write` writes, until
    /// `finish`.
    pub fn manual() -> Self {
        Self {
            inner: Rc::default(),
            pace: None,
        }
    }
    /// Write `text` as the next token of the reply under way: whether one
    /// is.
    pub fn write(&self, text: &str) -> bool {
        let live = self.inner.borrow().live.clone();
        let Some(body) = live.filter(Body::is_open) else {
            return false;
        };
        let data = Value::record([("text", Value::from(text))]).to_json();
        body.write(format!("event: delta\ndata: {data}\n\n").as_bytes());
        true
    }
    /// Whether a manual model's reply is under way.
    pub fn streaming(&self) -> bool {
        self.inner.borrow().live.as_ref().is_some_and(Body::is_open)
    }
    /// End the reply under way.
    pub fn finish(&self) {
        self.inner.borrow_mut().finished = true;
    }
    /// Fail the next `n` replies.
    pub fn fail(&self, n: u32) {
        self.inner.borrow_mut().failing = n;
    }
    /// Hold replies after their first token (`true`) until released.
    pub fn hold(&self, held: bool) {
        self.inner.borrow_mut().held = held;
    }
    /// The conversations sent so far: each message's role and text.
    pub fn asked(&self) -> Vec<Vec<(String, String)>> {
        self.inner.borrow().asked.clone()
    }
    /// The reply to the `n`th question asked (from 0): what the example
    /// streams.
    pub fn reply(n: u64) -> &'static str {
        REPLIES[n as usize % REPLIES.len()]
    }
    /// Its tokens: words, each with the space or line break before it.
    pub fn tokens(text: &str) -> Vec<&str> {
        let mut tokens = Vec::new();
        let mut start = 0;
        let bytes = text.as_bytes();
        for i in 1..bytes.len() {
            if (bytes[i] == b' ' || bytes[i] == b'\n')
                && bytes[i - 1] != b' '
                && bytes[i - 1] != b'\n'
            {
                tokens.push(&text[start..i]);
                start = i;
            }
        }
        tokens.push(&text[start..]);
        tokens
    }
    /// Serve `host`'s `ui.fetch` at `https://chat.example/v1/reply`.
    pub fn serve(&self, host: &ScriptHost) {
        let server = self.clone();
        host.streaming_fetcher(move |request: Request, body: Body| {
            let server = server.clone();
            async move { server.answer(request, body).await }
        });
    }
    async fn answer(&self, request: Request, body: Body) -> Result<Response, String> {
        if request.url != "https://chat.example/v1/reply" || request.method != "POST" {
            return Ok(json(404, r#"{"error":"Not found"}"#));
        }
        let sent = request
            .body
            .as_deref()
            .and_then(|b| Value::from_json(std::str::from_utf8(b).ok()?));
        let messages: Vec<(String, String)> = sent
            .as_ref()
            .and_then(|v| v.field("messages"))
            .map(|list| {
                list.list()
                    .iter()
                    .map(|m| {
                        let field = |name| m.field(name).map(Value::to_string).unwrap_or_default();
                        (field("role"), field("text"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let n = {
            let mut inner = self.inner.borrow_mut();
            inner.asked.push(messages);
            if inner.failing > 0 {
                inner.failing -= 1;
                return Ok(json(
                    429,
                    r#"{"error":"The model is busy: try again in a moment"}"#,
                ));
            }
            inner.replies += 1;
            inner.replies - 1
        };
        let Some(pace) = self.pace else {
            {
                let mut inner = self.inner.borrow_mut();
                inner.live = Some(body.clone());
                inner.finished = false;
            }
            while !self.inner.borrow().finished && body.is_open() {
                zgx::timer::sleep(Duration::from_millis(1)).await;
            }
            self.inner.borrow_mut().live = None;
            body.write(b"event: done\ndata: {}\n\n");
            return Ok(stream());
        };
        zgx::timer::sleep(pace).await;
        for (i, token) in Self::tokens(Self::reply(n)).into_iter().enumerate() {
            if !body.is_open() {
                break;
            }
            let data = Value::record([("text", Value::from(token))]).to_json();
            body.write(format!("event: delta\ndata: {data}\n\n").as_bytes());
            while i == 0 && self.inner.borrow().held && body.is_open() {
                zgx::timer::sleep(Duration::from_millis(1)).await;
            }
            zgx::timer::sleep(pace).await;
        }
        body.write(b"event: done\ndata: {}\n\n");
        Ok(stream())
    }
}

/// The answer to a reply streamed.
fn stream() -> Response {
    Response {
        status: 200,
        headers: vec![("Content-Type".into(), "text/event-stream".into())],
        body: Vec::new(),
    }
}

fn json(status: u16, body: &str) -> Response {
    Response {
        status,
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: body.as_bytes().to_vec(),
    }
}
