//! Deterministic commands, matched before any model runs.
//!
//! A command matches only when it is the whole utterance (give or take
//! fillers like "please" or "okay"), so "don't stop the server, restart it"
//! is a message for the agent, never a Stop. Whether a match can act (is
//! anything waiting for approval?) is the conductor's call, not the
//! grammar's.

/// A command the grammar recognised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastCommand {
    /// Stop the current agent's turn and Dove's speech.
    Stop,
    /// Approve the waiting request; `session` asks for approve-for-session.
    Approve {
        session: bool,
    },
    Deny,
    MuteMic,
    UnmuteMic,
    /// Stop Dove talking without touching the microphone or the agent.
    Silence,
    Faster,
    Slower,
    /// Status of every thread (`None`) or the current one.
    Status {
        all: bool,
    },
    ShowDiff,
    SwitchBack,
    EndCall,
    /// "Repeat that".
    Repeat,
}

/// Fillers dropped from either end before matching.
const FILLERS: &[&str] = &[
    "please",
    "keplorayx",
    "k22",
    "dove",
    "hey",
    "ok",
    "okay",
    "um",
    "uh",
    "er",
    "so",
    "now",
    "just",
    "and",
    "yeah",
    "thanks",
    "thank you",
];

/// Lowercase, ASCII apostrophes, punctuation to spaces, fillers trimmed.
pub fn normalize(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        let c = match c {
            '\u{2019}' | '\u{2018}' => '\'',
            c => c,
        };
        if c.is_alphanumeric() || c == '\'' {
            s.extend(c.to_lowercase());
        } else {
            s.push(' ');
        }
    }
    let mut words: Vec<&str> = s.split_whitespace().collect();
    loop {
        let before = words.len();
        for f in FILLERS {
            let fw: Vec<&str> = f.split(' ').collect();
            if words.len() > fw.len() && words[..fw.len()] == fw[..] {
                words.drain(..fw.len());
            }
            if words.len() > fw.len() && words[words.len() - fw.len()..] == fw[..] {
                words.truncate(words.len() - fw.len());
            }
        }
        if words.len() == before {
            break;
        }
    }
    words.join(" ")
}

/// Exact phrases per command, after [`normalize`].
const TABLE: &[(&[&str], FastCommand)] = &[
    (
        &[
            "stop",
            "stop it",
            "stop that",
            "stop now",
            "stop the agent",
            "stop the task",
            "stop working",
            "halt",
            "cancel",
            "cancel that",
            "cancel it",
            "abort",
            "interrupt",
            "interrupt it",
            "hold on",
            "wait wait",
        ],
        FastCommand::Stop,
    ),
    (
        &[
            "approve",
            "approved",
            "approve it",
            "approve that",
            "allow",
            "allow it",
            "allow that",
            "yes approve",
            "yes allow it",
            "go ahead",
            "yes go ahead",
            "do it",
            "yes do it",
            "accept",
            "permission granted",
        ],
        FastCommand::Approve { session: false },
    ),
    (
        &[
            "approve for session",
            "approve for this session",
            "allow for session",
            "allow for this session",
            "always allow",
            "always allow that",
            "allow always",
        ],
        FastCommand::Approve { session: true },
    ),
    (
        &["deny", "deny it", "deny that", "reject", "reject it", "don't allow", "do not allow", "no don't", "refuse"],
        FastCommand::Deny,
    ),
    (
        &["mute", "mute me", "mute mic", "mute my mic", "mute the mic", "mute microphone", "mute my microphone"],
        FastCommand::MuteMic,
    ),
    (&["unmute", "unmute me", "unmute mic", "unmute my mic", "unmute microphone"], FastCommand::UnmuteMic),
    (
        &[
            "be quiet",
            "quiet",
            "shush",
            "shh",
            "hush",
            "silence",
            "stop talking",
            "you can stop talking",
            "enough",
            "that's enough",
            "skip",
            "skip it",
        ],
        FastCommand::Silence,
    ),
    (&["talk faster", "speak faster", "faster", "speed up", "go faster"], FastCommand::Faster),
    (&["talk slower", "speak slower", "slower", "slow down"], FastCommand::Slower),
    (
        &[
            "status",
            "what's the status",
            "what is the status",
            "what's happening",
            "what is happening",
            "what's going on",
            "what is going on",
            "how's it going",
            "how is it going",
            "where are we",
            "status update",
            "give me a status update",
            "how's it looking",
        ],
        FastCommand::Status { all: false },
    ),
    (
        &[
            "status of everything",
            "status of all threads",
            "how are all the threads",
            "what's everyone doing",
            "what is everyone doing",
            "all threads",
            "overall status",
        ],
        FastCommand::Status { all: true },
    ),
    (
        &[
            "show diff",
            "show the diff",
            "show me the diff",
            "show changes",
            "show the changes",
            "show me the changes",
            "open the diff",
        ],
        FastCommand::ShowDiff,
    ),
    (
        &["take me back", "switch back", "go back", "back to the previous thread", "previous thread", "back"],
        FastCommand::SwitchBack,
    ),
    (
        &[
            "end call",
            "end the call",
            "hang up",
            "goodbye",
            "bye",
            "bye bye",
            "stop the call",
            "end voice mode",
            "that's all",
        ],
        FastCommand::EndCall,
    ),
    (&["repeat", "repeat that", "say that again", "what did you say", "come again"], FastCommand::Repeat),
];

/// The command `text` is, when it is one and nothing more.
pub fn parse(text: &str) -> Option<FastCommand> {
    let n = normalize(text);
    if n.is_empty() || n.split(' ').count() > 7 {
        return None;
    }
    TABLE.iter().find(|(phrases, _)| phrases.contains(&n.as_str())).map(|(_, c)| c.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_utterance_commands_match_with_fillers_and_punctuation() {
        let cases: &[(&str, FastCommand)] = &[
            ("Stop.", FastCommand::Stop),
            ("stop!", FastCommand::Stop),
            ("Okay, stop it please", FastCommand::Stop),
            ("Hey Dove, cancel that.", FastCommand::Stop),
            ("Hey K22, cancel that.", FastCommand::Stop),
            ("Keplorayx, stop.", FastCommand::Stop),
            ("Hold on", FastCommand::Stop),
            ("Approve.", FastCommand::Approve { session: false }),
            ("Yes, go ahead", FastCommand::Approve { session: false }),
            ("Approve for this session", FastCommand::Approve { session: true }),
            ("Always allow that.", FastCommand::Approve { session: true }),
            ("Deny", FastCommand::Deny),
            ("No, don\u{2019}t.", FastCommand::Deny),
            ("Mute my mic", FastCommand::MuteMic),
            ("unmute", FastCommand::UnmuteMic),
            ("Be quiet", FastCommand::Silence),
            ("Stop talking.", FastCommand::Silence),
            ("Talk faster", FastCommand::Faster),
            ("slow down please", FastCommand::Slower),
            ("What's happening?", FastCommand::Status { all: false }),
            ("What is happening?", FastCommand::Status { all: false }),
            ("How's it going?", FastCommand::Status { all: false }),
            ("What's everyone doing?", FastCommand::Status { all: true }),
            ("Show me the diff", FastCommand::ShowDiff),
            ("Take me back.", FastCommand::SwitchBack),
            ("End call", FastCommand::EndCall),
            ("Okay, goodbye!", FastCommand::EndCall),
            ("Say that again?", FastCommand::Repeat),
        ];
        for (text, want) in cases {
            assert_eq!(parse(text).as_ref(), Some(want), "{text:?}");
        }
    }

    #[test]
    fn commands_inside_longer_requests_are_messages_not_commands() {
        for text in [
            "Don't stop the server, just restart it",
            "Stop using tabs and switch to spaces in the formatter",
            "Can you approve the pull request on GitHub for me",
            "Add a mute button to the video player",
            "Deny list should include the admin route",
            "Go back to the previous implementation of the parser",
            "Show me the diff between main and the release branch in a table",
            "Tell me what's happening with the flaky test in CI",
            "Silence the noisy warnings in the build log",
            "",
            "   ",
            "stopped",
            "approval",
            "backend",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn fillers_alone_are_not_a_command() {
        assert_eq!(parse("okay"), None);
        assert_eq!(parse("please"), None);
        // The last filler stays: an utterance never normalizes to nothing.
        assert_eq!(normalize("Okay, please"), "okay");
    }

    #[test]
    fn normalize_keeps_inner_words() {
        assert_eq!(normalize("  Please, STOP the task now! "), "stop the task");
        assert_eq!(normalize("Hey Dove — what's going on?"), "what's going on");
    }
}
