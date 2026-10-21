//! The interactive side of a session: how to open an agent's own terminal
//! UI on the same conversation Dove's chat drives headlessly.
//!
//! Chat ⇄ Terminal hands one session between two drivers. Chat runs the
//! agent's structured mode; Terminal runs its TUI in a PTY, resuming the
//! same native session id, in the same worktree, with the same model and
//! permission level. Only one driver runs at a time (the engine enforces
//! it), so both never write the same session file.

use std::path::{Path, PathBuf};

use dove_proto::{HarnessKind, PermissionMode};

/// A command line for the agent's TUI.
#[derive(Debug, Clone, PartialEq)]
pub struct TuiCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// True when the TUI reopens the chat's conversation; false when it
    /// starts fresh (the agent has no resume flag Dove can use).
    pub resumes: bool,
    /// Extra environment for the program (OpenCode's inline config).
    pub env: Vec<(String, String)>,
}

/// What the TUI needs to know about the session.
#[derive(Debug, Clone, Default)]
pub struct TuiSpec<'a> {
    pub native_session_id: Option<&'a str>,
    pub model: Option<&'a str>,
    pub mode: PermissionMode,
    pub add_dirs: &'a [PathBuf],
}

/// Whether the TUI can reopen a chat session for `kind` once that session
/// has a native id. Antigravity is an ACP server with no terminal UI.
pub fn resumes(kind: HarnessKind) -> bool {
    !matches!(kind, HarnessKind::Antigravity)
}

/// The interactive binary for `kind`. ACP-only servers (Antigravity, and an
/// `amp-acp` path when `amp` itself is installed) are not a terminal UI.
pub fn interactive_program(kind: HarnessKind, program: &Path) -> Option<PathBuf> {
    if kind == HarnessKind::Antigravity || is_acp_server(program) {
        return None;
    }
    if is_acp_wrapper(program) {
        return crate::detect::find(kind).filter(|p| !is_acp_wrapper(p) && !is_acp_server(p));
    }
    Some(program.to_path_buf())
}

fn file_stem(program: &Path) -> String {
    program.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn is_acp_server(program: &Path) -> bool {
    file_stem(program).contains("acp_server")
}

fn is_acp_wrapper(program: &Path) -> bool {
    let stem = file_stem(program);
    stem == "amp-acp" || stem.ends_with("-acp")
}

/// A login shell in `cwd`. The shell harness's session is that directory.
pub fn shell_command(cwd: &Path) -> TuiCommand {
    let (program, args) = if cfg!(windows) {
        let pwsh = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.join("pwsh.exe"))
            .find(|p| p.is_file());
        let program = pwsh.unwrap_or_else(|| PathBuf::from("powershell.exe"));
        (program, vec!["-NoLogo".into()])
    } else {
        let program = std::env::var("SHELL").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/bin/bash"));
        (program, vec!["-l".into()])
    };
    TuiCommand { program, args, cwd: cwd.to_path_buf(), resumes: true, env: Vec::new() }
}

/// Build the TUI command for `kind`, whose CLI lives at `program`.
pub fn command(kind: HarnessKind, program: &Path, cwd: &Path, spec: &TuiSpec) -> TuiCommand {
    crate::office_skills::ensure_installed();
    let mut args: Vec<String> = Vec::new();
    let mut push = |a: &str| args.push(a.to_string());
    let id = spec.native_session_id.filter(|s| !s.is_empty());
    let resumed = match kind {
        HarnessKind::ClaudeCode => {
            if let Some(id) = id {
                push("--resume");
                push(id);
            }
            if let Some(m) = spec.model {
                push("--model");
                push(m);
            }
            match spec.mode {
                // Explicit, so the user's own Claude default never overrides
                // the level chosen in Dove.
                PermissionMode::Default => {
                    push("--permission-mode");
                    push("default");
                }
                PermissionMode::AcceptEdits => {
                    push("--permission-mode");
                    push("acceptEdits");
                }
                PermissionMode::Plan => {
                    push("--permission-mode");
                    push("plan");
                }
                PermissionMode::Bypass => push("--dangerously-skip-permissions"),
            }
            for d in spec.add_dirs {
                push("--add-dir");
                push(&d.display().to_string());
            }
            // Dove's browser tools ride beside the user's own MCP servers.
            // A file path, not inline JSON: quoting survives cmd.exe.
            if let Some(path) = crate::tool_server(cwd).and_then(|s| write_claude_mcp_config(&s)) {
                push("--mcp-config");
                push(&path.display().to_string());
                push("--allowedTools");
                push("mcp__dove");
            }
            id.is_some()
        }
        HarnessKind::Codex => {
            if let Some(id) = id {
                push("resume");
                push(id);
            }
            if let Some(m) = spec.model {
                push("-m");
                push(m);
            }
            match spec.mode {
                PermissionMode::Bypass => push("--dangerously-bypass-approvals-and-sandbox"),
                PermissionMode::Plan => {
                    push("--sandbox");
                    push("read-only");
                }
                PermissionMode::AcceptEdits => {
                    push("--sandbox");
                    push("workspace-write");
                    push("--ask-for-approval");
                    push("on-request");
                }
                PermissionMode::Default => {
                    push("--ask-for-approval");
                    push("untrusted");
                }
            }
            id.is_some()
        }
        HarnessKind::OpenCode | HarnessKind::Kilo => {
            if let Some(id) = id {
                push("--session");
                push(id);
            }
            if let Some(m) = spec.model {
                push("--model");
                push(m);
            }
            id.is_some()
        }
        HarnessKind::Cursor | HarnessKind::Copilot | HarnessKind::Cline | HarnessKind::Kimi => {
            if let Some(id) = id {
                push("--resume");
                push(id);
            }
            if let Some(m) = spec.model.filter(|_| matches!(kind, HarnessKind::Cursor | HarnessKind::Copilot)) {
                push("--model");
                push(m);
            }
            if kind == HarnessKind::Copilot && spec.mode == PermissionMode::Bypass {
                push("--yolo");
            }
            id.is_some()
        }
        HarnessKind::Gemini | HarnessKind::Qwen => {
            if let Some(id) = id {
                push("--resume");
                push(id);
            }
            if let Some(m) = spec.model {
                push("--model");
                push(m);
            }
            let approval = match spec.mode {
                PermissionMode::Default => "default",
                PermissionMode::AcceptEdits => "auto_edit",
                PermissionMode::Plan => "plan",
                PermissionMode::Bypass => "yolo",
            };
            push("--approval-mode");
            push(approval);
            for d in spec.add_dirs {
                push("--include-directories");
                push(&d.display().to_string());
            }
            id.is_some()
        }
        HarnessKind::Pi => {
            if let Some(id) = id {
                match find_pi_session(id) {
                    Some(file) => {
                        push("--session");
                        push(&file.display().to_string());
                    }
                    None => push("--continue"),
                }
            }
            id.is_some()
        }
        HarnessKind::Goose => {
            if let Some(id) = id {
                push("session");
                push("resume");
                push("--name");
                push(id);
            }
            id.is_some()
        }
        HarnessKind::Amp => {
            if let Some(id) = id {
                push("--thread");
                push(id);
            }
            id.is_some()
        }
        HarnessKind::Droid => {
            if let Some(id) = id {
                push("resume");
                push(id);
            }
            id.is_some()
        }
        // Devin, Grok and Hermes open their own UI in this folder. Their
        // ACP session id is not a flag their terminal accepts.
        HarnessKind::Devin | HarnessKind::Grok | HarnessKind::Hermes => false,
        HarnessKind::Shell => true,
        HarnessKind::Antigravity => false,
    };
    // Dove's tools (browser, integrations, MCP servers) for Codex's own UI,
    // as root overrides ahead of any subcommand.
    if kind == HarnessKind::Codex
        && let Some(server) = crate::tool_server(cwd)
    {
        args.splice(0..0, codex_mcp_overrides(&server));
    }
    // OpenCode reads inline config from OPENCODE_CONFIG_CONTENT and merges
    // it over the user's own files, so Dove's server rides beside theirs.
    let mut env = Vec::new();
    if kind == HarnessKind::OpenCode
        && let Some(server) = crate::tool_server(cwd)
    {
        let existing = std::env::var("OPENCODE_CONFIG_CONTENT").ok();
        env.push(("OPENCODE_CONFIG_CONTENT".to_string(), opencode_config_content(&server, existing.as_deref())));
    }
    TuiCommand { program: program.to_path_buf(), args, cwd: cwd.to_path_buf(), resumes: resumed, env }
}

/// OpenCode inline config adding `server` as a local MCP server, merged
/// into `existing` (an inline config already in Dove's environment) when
/// that is a JSON object.
fn opencode_config_content(server: &crate::ToolServer, existing: Option<&str>) -> String {
    use serde_json::{Value, json};
    let mut config = existing
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let command: Vec<&str> =
        std::iter::once(server.command.as_str()).chain(server.args.iter().map(String::as_str)).collect();
    let environment: serde_json::Map<String, Value> =
        server.env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
    if !config.get("mcp").is_some_and(Value::is_object) {
        config["mcp"] = json!({});
    }
    config["mcp"][server.name.as_str()] = json!({
        "type": "local",
        "command": command,
        "environment": environment,
        "enabled": true,
        // Dove's server starts the user's integrations before it lists
        // tools; OpenCode's 5 s default would give up on a slow one.
        "timeout": 60000,
    });
    config.to_string()
}

/// What to type into a TUI to leave it cleanly.
pub fn exit_input(kind: HarnessKind) -> &'static str {
    match kind {
        HarnessKind::Gemini | HarnessKind::Qwen => "/quit\r",
        HarnessKind::Shell => "exit\r",
        HarnessKind::Devin | HarnessKind::Grok | HarnessKind::Hermes | HarnessKind::Antigravity => "\u{4}",
        _ => "/exit\r",
    }
}

/// Pi addresses a session by file (`--session`). `--continue` is the
/// fallback when the file isn't in the usual directory yet.
fn find_pi_session(id: &str) -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)?;
    let agent =
        std::env::var_os("PI_CODING_AGENT_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".pi").join("agent"));
    let root = agent.join("sessions");
    let mut stack = vec![root];
    let mut seen = 0u32;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 400 {
                return None;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.contains(id) {
                return Some(path);
            }
        }
    }
    None
}

/// Where the agent writes the session transcript the chat mirrors while
/// the TUI runs.
pub fn transcript_path(kind: HarnessKind, cwd: &Path, native_session_id: &str) -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)?;
    match kind {
        HarnessKind::ClaudeCode => {
            let config =
                std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".claude"));
            Some(config.join("projects").join(claude_project_slug(cwd)).join(format!("{native_session_id}.jsonl")))
        }
        HarnessKind::Codex => {
            let root = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
            find_codex_rollout(&root.join("sessions"), native_session_id)
        }
        _ => None,
    }
}

/// Claude names a project's folder after its path with every
/// non-alphanumeric character turned into `-`.
pub fn claude_project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Codex keeps `sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`; newest first.
fn find_codex_rollout(sessions: &Path, id: &str) -> Option<PathBuf> {
    let mut stack = vec![sessions.to_path_buf()];
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.contains(id) && n.ends_with(".jsonl"))
            {
                let modified = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                if best.as_ref().is_none_or(|(t, _)| modified > *t) {
                    best = Some((modified, path));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Codex `-c` overrides that add `server` beside the user's own MCP servers
/// (`mcp_servers.<name>.{command,args,env}`), as TOML values.
fn codex_mcp_overrides(server: &crate::ToolServer) -> Vec<String> {
    let name = &server.name;
    let args = server.args.iter().map(|a| toml_str(a)).collect::<Vec<_>>().join(",");
    let env = server.env.iter().map(|(k, v)| format!("{k}={}", toml_str(v))).collect::<Vec<_>>().join(",");
    vec![
        "-c".into(),
        format!("mcp_servers.{name}.command={}", toml_str(&server.command)),
        "-c".into(),
        format!("mcp_servers.{name}.args=[{args}]"),
        "-c".into(),
        format!("mcp_servers.{name}.env={{{env}}}"),
    ]
}

/// A TOML string: a literal `'…'` (no escapes, so Windows paths survive)
/// unless the text holds a single quote.
fn toml_str(s: &str) -> String {
    if !s.contains('\'') && !s.contains('\n') {
        return format!("'{s}'");
    }
    let escaped: String = s
        .chars()
        .flat_map(|c| match c {
            '\\' => vec!['\\', '\\'],
            '"' => vec!['\\', '"'],
            '\n' => vec!['\\', 'n'],
            c => vec![c],
        })
        .collect();
    format!("\"{escaped}\"")
}

/// Write a Claude `--mcp-config` file for `server` (one per launch config,
/// reused while unchanged).
fn write_claude_mcp_config(server: &crate::ToolServer) -> Option<std::path::PathBuf> {
    let env: serde_json::Map<String, serde_json::Value> =
        server.env.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
    let body = serde_json::json!({
        "mcpServers": { &server.name: { "command": server.command, "args": server.args, "env": env } }
    })
    .to_string();
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut h);
    let path = std::env::temp_dir().join(format!("dove-mcp-{:016x}.json", h.finish()));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
        std::fs::write(&path, body).ok()?;
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_overrides_are_toml() {
        let server = crate::ToolServer {
            name: "dove".into(),
            command: r"C:\Program Files\Dove\dove.exe".into(),
            args: vec!["--mcp-browser".into()],
            env: vec![("DOVE_CWD".into(), r"C:\w".into()), ("Q".into(), "it's".into())],
        };
        let o = codex_mcp_overrides(&server);
        assert_eq!(o[1], r"mcp_servers.dove.command='C:\Program Files\Dove\dove.exe'");
        assert_eq!(o[3], "mcp_servers.dove.args=['--mcp-browser']");
        assert_eq!(o[5], r#"mcp_servers.dove.env={DOVE_CWD='C:\w',Q="it's"}"#);
    }

    #[test]
    fn opencode_config_adds_dove_beside_existing() {
        let server = crate::ToolServer {
            name: "dove".into(),
            command: r"C:\Program Files\Dove\dove.exe".into(),
            args: vec!["--mcp-browser".into()],
            env: vec![("DOVE_DATA".into(), r"C:\d".into())],
        };
        let v: serde_json::Value = serde_json::from_str(&opencode_config_content(&server, None)).unwrap();
        assert_eq!(v["mcp"]["dove"]["type"], "local");
        assert_eq!(
            v["mcp"]["dove"]["command"],
            serde_json::json!([r"C:\Program Files\Dove\dove.exe", "--mcp-browser"])
        );
        assert_eq!(v["mcp"]["dove"]["environment"]["DOVE_DATA"], r"C:\d");
        // An inline config already set keeps its own keys and servers.
        let existing = r#"{"model":"x/y","mcp":{"mine":{"type":"remote","url":"https://m"}}}"#;
        let v: serde_json::Value = serde_json::from_str(&opencode_config_content(&server, Some(existing))).unwrap();
        assert_eq!(v["model"], "x/y");
        assert_eq!(v["mcp"]["mine"]["url"], "https://m");
        assert_eq!(v["mcp"]["dove"]["enabled"], true);
        // Garbage is replaced, not merged.
        let v: serde_json::Value = serde_json::from_str(&opencode_config_content(&server, Some("nope"))).unwrap();
        assert!(v["mcp"]["dove"].is_object());
    }

    fn spec(id: Option<&str>, mode: PermissionMode) -> TuiSpec<'_> {
        TuiSpec { native_session_id: id, model: Some("opus"), mode, add_dirs: &[] }
    }

    #[test]
    fn claude_resumes_with_model_and_mode() {
        let c = command(
            HarnessKind::ClaudeCode,
            Path::new("claude"),
            Path::new("/w"),
            &spec(Some("abc"), PermissionMode::AcceptEdits),
        );
        assert_eq!(c.args, ["--resume", "abc", "--model", "opus", "--permission-mode", "acceptEdits"]);
        let ask = command(
            HarnessKind::ClaudeCode,
            Path::new("claude"),
            Path::new("/w"),
            &spec(Some("abc"), PermissionMode::Default),
        );
        assert!(ask.args.windows(2).any(|w| w == ["--permission-mode", "default"]));
        assert!(c.resumes);
        let fresh = command(
            HarnessKind::ClaudeCode,
            Path::new("claude"),
            Path::new("/w"),
            &spec(None, PermissionMode::Default),
        );
        assert!(!fresh.resumes);
    }

    #[test]
    fn codex_resume_subcommand() {
        let c =
            command(HarnessKind::Codex, Path::new("codex"), Path::new("/w"), &spec(Some("t1"), PermissionMode::Bypass));
        assert_eq!(&c.args[..2], ["resume", "t1"]);
        assert!(c.args.contains(&"--dangerously-bypass-approvals-and-sandbox".to_string()));
    }

    #[test]
    fn gemini_and_qwen_resume_the_same_session() {
        let c = command(
            HarnessKind::Gemini,
            Path::new("gemini"),
            Path::new("/w"),
            &spec(Some("sess"), PermissionMode::Plan),
        );
        assert!(c.resumes);
        assert!(c.args.windows(2).any(|w| w == ["--resume", "sess"]));
        assert!(c.args.windows(2).any(|w| w == ["--approval-mode", "plan"]));
        let q =
            command(HarnessKind::Qwen, Path::new("qwen"), Path::new("/w"), &spec(Some("sess"), PermissionMode::Bypass));
        assert!(q.args.windows(2).any(|w| w == ["--approval-mode", "yolo"]));
    }

    #[test]
    fn every_resumable_harness_reopens_its_id() {
        let cases = [
            (HarnessKind::Cursor, "--resume"),
            (HarnessKind::Copilot, "--resume"),
            (HarnessKind::Cline, "--resume"),
            (HarnessKind::Kimi, "--resume"),
            (HarnessKind::OpenCode, "--session"),
            (HarnessKind::Kilo, "--session"),
            (HarnessKind::Amp, "--thread"),
        ];
        for (kind, flag) in cases {
            let c = command(kind, Path::new("agent"), Path::new("/w"), &spec(Some("sid"), PermissionMode::Default));
            assert!(c.resumes, "{}", kind.label());
            assert!(c.args.windows(2).any(|w| w == [flag, "sid"]), "{} {:?}", kind.label(), c.args);
        }
        let goose = command(
            HarnessKind::Goose,
            Path::new("goose"),
            Path::new("/w"),
            &spec(Some("sid"), PermissionMode::Default),
        );
        assert_eq!(&goose.args[..], ["session", "resume", "--name", "sid"]);
        let droid = command(
            HarnessKind::Droid,
            Path::new("droid"),
            Path::new("/w"),
            &spec(Some("sid"), PermissionMode::Default),
        );
        assert_eq!(&droid.args[..2], ["resume", "sid"]);
        let pi =
            command(HarnessKind::Pi, Path::new("pi"), Path::new("/w"), &spec(Some("sid"), PermissionMode::Default));
        assert!(pi.resumes);
        assert!(pi.args.first().is_some_and(|a| a == "--continue" || a == "--session"));
    }

    #[test]
    fn agents_without_a_resume_flag_stay_in_the_folder() {
        for kind in [HarnessKind::Devin, HarnessKind::Grok, HarnessKind::Hermes, HarnessKind::Antigravity] {
            let c = command(kind, Path::new("agent"), Path::new("/w"), &spec(Some("sid"), PermissionMode::Default));
            assert!(!c.resumes && c.args.is_empty(), "{}", kind.label());
        }
        assert!(!resumes(HarnessKind::Antigravity));
        assert!(resumes(HarnessKind::Gemini));
    }

    #[test]
    fn acp_servers_are_not_a_terminal() {
        assert!(interactive_program(HarnessKind::Antigravity, Path::new("agy_acp_server")).is_none());
        assert!(interactive_program(HarnessKind::ClaudeCode, Path::new("claude")).is_some());
    }

    #[test]
    fn claude_slug_matches_its_folder_naming() {
        assert_eq!(claude_project_slug(Path::new("C:\\Users\\me\\repo.x")), "C--Users-me-repo-x");
        assert_eq!(claude_project_slug(Path::new("/home/me/repo")), "-home-me-repo");
    }
}
