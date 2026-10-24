//! A thread as plain text another agent can read: what the user asked,
//! what the agent answered, and the tools it ran, turn by turn. Mentioning
//! a thread in the composer (`@` a thread) writes this to a file and points
//! the agent at it.

use dove_proto::AgentEvent;
use dove_store::{Store, Task};

/// The digest's cap: the most recent part of a long thread is kept.
pub const MAX_CHARS: usize = 40_000;

/// `task`'s transcript as readable text, newest turns kept when it is long.
pub fn transcript_digest(store: &Store, task: &Task) -> String {
    digest(&task.title, task.harness.label(), &store.read_transcript(task.id))
}

/// The digest of `events` (split out for tests).
pub fn digest(title: &str, agent: &str, events: &[AgentEvent]) -> String {
    let mut turns: Vec<String> = Vec::new();
    let mut answer = String::new();
    let mut tools: Vec<String> = Vec::new();
    let flush = |turns: &mut Vec<String>, answer: &mut String, tools: &mut Vec<String>| {
        if !tools.is_empty() {
            turns.push(format!("Tools: {}", tools.join("; ")));
            tools.clear();
        }
        let text = answer.trim();
        if !text.is_empty() {
            turns.push(format!("Agent: {text}"));
        }
        answer.clear();
    };
    for e in events {
        match e {
            AgentEvent::UserMessage { text } => {
                flush(&mut turns, &mut answer, &mut tools);
                let text = crate::strip_persona(text).trim();
                if !text.is_empty() {
                    turns.push(format!("User: {text}"));
                }
            }
            AgentEvent::TextDelta { text } => answer.push_str(text),
            AgentEvent::ToolCall { title, .. } if tools.len() < 12 => tools.push(title.to_string()),
            _ => {}
        }
    }
    flush(&mut turns, &mut answer, &mut tools);
    let head = format!("Dove thread \"{title}\" ({agent})\n\n");
    let mut body = turns.join("\n\n");
    if body.len() > MAX_CHARS {
        let cut = body.len() - MAX_CHARS;
        let at = (cut..body.len()).find(|i| body.is_char_boundary(*i)).unwrap_or(body.len());
        body = format!("[earlier turns left out]\n\n{}", &body[at..]);
    }
    if body.is_empty() {
        body = "(no messages yet)".into();
    }
    format!("{head}{body}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_read_in_order_with_their_tools() {
        let events = vec![
            AgentEvent::UserMessage { text: "Fix the login bug".into() },
            AgentEvent::TextDelta { text: "Looking ".into() },
            AgentEvent::ToolCall {
                call_id: "1".into(),
                name: "Read".into(),
                kind: dove_proto::ToolKind::Read,
                title: "Read src/login.rs".into(),
                input: serde_json::Value::Null,
            },
            AgentEvent::TextDelta { text: "Fixed it.".into() },
            AgentEvent::UserMessage { text: "Thanks".into() },
        ];
        let d = digest("Login", "Claude Code", &events);
        assert!(d.starts_with("Dove thread \"Login\" (Claude Code)"));
        let user = d.find("User: Fix the login bug").unwrap();
        let tools = d.find("Tools: Read src/login.rs").unwrap();
        let agent = d.find("Agent: Looking Fixed it.").unwrap();
        let next = d.find("User: Thanks").unwrap();
        assert!(user < tools && tools < agent && agent < next);
    }

    #[test]
    fn long_threads_keep_the_newest_turns() {
        let events: Vec<AgentEvent> = (0..2000)
            .flat_map(|i| {
                [
                    AgentEvent::UserMessage { text: format!("question {i}").into() },
                    AgentEvent::TextDelta { text: "answer ".repeat(10).into() },
                ]
            })
            .collect();
        let d = digest("Big", "Codex", &events);
        assert!(d.len() < MAX_CHARS + 200);
        assert!(d.contains("question 1999") && !d.contains("question 0\n"));
        assert!(d.contains("[earlier turns left out]"));
        assert_eq!(digest("Empty", "Codex", &[]).trim_end(), "Dove thread \"Empty\" (Codex)\n\n(no messages yet)");
    }
}
