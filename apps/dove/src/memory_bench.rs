//! A memory benchmark with real agents, against a signed-in Supermemory
//! account: OpenCode on DeepSeek and Antigravity on Gemini 3.8 Flash teach
//! each other through Dove's memory in easy, medium, hard, stress and
//! self-improving scenarios. Every scenario's final question also runs with
//! memory off, as the baseline.
//!
//! ```text
//! DOVE_DATA=<home with mcp_auth.json> cargo test -p dove memory_bench -- --ignored --nocapture
//! ```
//!
//! Run it outside an agent session, or strip `CLAUDE*`/`ANTHROPIC*` from the
//! environment first. Repositories go in `C:/dvw/mem-bench/<n>/` (or
//! `$DOVE_BENCH_DIR`), with nothing else in that tree: agents that search
//! around must not find the bench's logs, Dove's transcripts or the report,
//! which go to `$DOVE_BENCH_REPORT` (default: the temp folder).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::memory::{self, Scope};
use dove_core::proto::{AgentEvent, HarnessKind, PermissionDecision, PermissionMode};
use dove_core::store::Project;
use dove_core::{Command, Engine, NewTask, RunSettings};

#[derive(Clone, Copy)]
enum Agent {
    OpenCode,
    Antigravity,
}

impl Agent {
    fn kind(self) -> HarnessKind {
        match self {
            Agent::OpenCode => HarnessKind::OpenCode,
            Agent::Antigravity => HarnessKind::Antigravity,
        }
    }

    fn model(self) -> String {
        match self {
            Agent::OpenCode => std::env::var("BENCH_OPENCODE_MODEL").unwrap_or("opencode/deepseek-v4.1-flash-free".into()),
            Agent::Antigravity => std::env::var("BENCH_AGY_MODEL").unwrap_or("gemini-3.8-flash-medium".into()),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Agent::OpenCode => "OpenCode · DeepSeek V4.1 Flash",
            Agent::Antigravity => "Antigravity · Gemini 3.8 Flash",
        }
    }
}

struct Turn {
    sent: String,
    answer: String,
    secs: f32,
    /// Tool calls the agent made (reading files, running commands).
    tools: usize,
    /// A tool call touched a path outside the repository.
    peeked: bool,
}

/// Windows or POSIX absolute paths in a tool call's input that are not
/// inside `repo`.
fn outside(input: &serde_json::Value, repo: &Path) -> bool {
    let text = input.to_string().to_lowercase().replace("\\\\", "/").replace('\\', "/");
    let repo = repo.display().to_string().to_lowercase().replace('\\', "/");
    let bytes = text.as_bytes();
    let mut i = 1;
    while i + 1 < bytes.len() {
        // `c:/...`
        if bytes[i] == b':' && bytes[i + 1] == b'/' && bytes[i - 1].is_ascii_alphabetic() {
            let start = i - 1;
            let end = text[start..].find(['"', ' ', '\'', ',']).map_or(text.len(), |e| start + e);
            if !text[start..end].starts_with(&repo) {
                return true;
            }
            i = end;
        }
        i += 1;
    }
    false
}

impl Turn {
    fn recalled(&self) -> Vec<String> {
        memory::recalled(&self.sent).unwrap_or_default()
    }
}

struct Bench {
    engine: Arc<Engine>,
    backend: Arc<dyn memory::Backend>,
    n: String,
    dir: PathBuf,
    /// Report and trace, outside the repositories' tree.
    out: PathBuf,
    log: String,
    rows: Vec<String>,
    failures: Vec<String>,
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

impl Bench {
    fn new() -> Self {
        crate::supermemory::refresh(&crate::pages::integrations::data_root());
        let backend = memory::backend().expect("sign in to Supermemory in DOVE_DATA first");
        let n = format!(
            "{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() % 1_000_000_000
        );
        let root = std::env::var("DOVE_BENCH_DIR").unwrap_or("C:/dvw/mem-bench".into());
        let dir = PathBuf::from(root).join(&n);
        std::fs::create_dir_all(&dir).unwrap();
        let out = std::env::var("DOVE_BENCH_REPORT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("dove-bench-reports"))
            .join(&n);
        std::fs::create_dir_all(&out).unwrap();
        // Dove's own store (transcripts of every teaching turn) lives far from
        // the repositories, so an agent that searches around cannot read it.
        let store = std::env::temp_dir().join(format!("dove-bench-store-{n}"));
        let engine = Engine::new(store).unwrap();
        let trace = std::fs::File::create(out.join("trace.log")).unwrap();
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("warn,dove_core::memory=debug,dove::supermemory=debug"))
            .with_writer(std::sync::Mutex::new(trace))
            .with_ansi(false)
            .try_init();
        engine.rescan_harnesses();
        let agy = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("agy/bin/agy.exe");
        if agy.is_file() {
            let _ = engine.store.set_setting("agent_path.antigravity", &agy.display().to_string());
        }
        for a in [Agent::OpenCode, Agent::Antigravity] {
            assert!(engine.program_for(a.kind()).is_some(), "{} is not installed", a.label());
        }
        Self { engine, backend, n, dir, out, log: String::new(), rows: Vec::new(), failures: Vec::new() }
    }

    fn say(&mut self, line: impl AsRef<str>) {
        println!("{}", line.as_ref());
        self.log.push_str(line.as_ref());
        self.log.push('\n');
    }

    /// A fresh repository with `remote` as origin.
    fn repo(&self, name: &str, remote: &str, files: &[(&str, &str)]) -> Project {
        let path = self.dir.join(name);
        std::fs::create_dir_all(&path).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(&path).output().unwrap();
        };
        git(&["init", "-q"]);
        git(&["remote", "add", "origin", remote]);
        for (f, body) in files {
            let p = path.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        git(&["add", "-A"]);
        git(&["-c", "user.name=bench", "-c", "user.email=bench@example.com", "commit", "-qm", "init"]);
        self.engine.store.add_project(&path).unwrap()
    }

    /// One turn in a new session of `agent`, without tools (a question).
    fn ask(&mut self, project: &Project, agent: Agent, prompt: &str) -> Turn {
        self.ask_with(project, agent, prompt, false)
    }

    /// One turn; `edit` lets the agent use tools without asking (a real
    /// change), otherwise it runs in the normal mode and every tool request
    /// is denied, so it answers from what it was given.
    fn ask_with(&mut self, project: &Project, agent: Agent, prompt: &str, edit: bool) -> Turn {
        let started = Instant::now();
        let mode = if edit { PermissionMode::Bypass } else { PermissionMode::Default };
        let (_task, live) = self
            .engine
            .start_task(
                project,
                NewTask {
                    title: "bench".into(),
                    prompt: prompt.to_string(),
                    harness: agent.kind(),
                    run: RunSettings { mode, model: Some(agent.model()), ..Default::default() },
                    join: None,
                    persona: None,
                },
            )
            .expect("start the session");
        let (mut sent, mut answer, mut tools, mut peeked) = (String::new(), String::new(), 0, false);
        let until = Instant::now() + Duration::from_secs(420);
        while Instant::now() < until {
            match live.events.recv_timeout(Duration::from_secs(5)) {
                Ok(AgentEvent::UserMessage { text }) if sent.is_empty() => sent = text.to_string(),
                Ok(AgentEvent::TextDelta { text }) => answer.push_str(&text),
                Ok(AgentEvent::ToolCall { input, .. }) => {
                    tools += 1;
                    peeked |= outside(&input, &project.path);
                    answer.push('\n');
                }
                Ok(AgentEvent::PermissionRequested(r)) => {
                    let decision = if edit { PermissionDecision::AllowOnce } else { PermissionDecision::Deny };
                    live.send(Command::Permission { request_id: r.request_id, decision });
                }
                Ok(AgentEvent::Error { message }) => self.say(format!("    ! {}: {message}", agent.label())),
                Ok(AgentEvent::TurnEnded { .. }) | Ok(AgentEvent::Exited { .. }) => break,
                _ => {}
            }
        }
        live.send(Command::Kill);
        let turn =
            Turn { sent, answer: answer.trim().to_string(), secs: started.elapsed().as_secs_f32(), tools, peeked };
        let short: String = turn.answer.chars().take(300).collect::<String>().replace('\n', " ⏎ ");
        self.say(format!(
            "    {} ({:.0}s, {} tool calls{}, recalled {}): {short}",
            agent.label(),
            turn.secs,
            turn.tools,
            if turn.peeked { ", LOOKED OUTSIDE THE REPO" } else { "" },
            turn.recalled().len()
        ));
        turn
    }

    /// A teaching turn, then wait until it is searchable (as when the user
    /// comes back later).
    fn teach(&mut self, project: &Project, agent: Agent, prompt: &str, probe: &str) {
        self.ask(project, agent, prompt);
        let scope = Scope::Project(memory::project_tag(&project.path));
        let b = self.backend.clone();
        let started = Instant::now();
        let mut found = false;
        while started.elapsed() < Duration::from_secs(120) {
            if b.search(probe, &scope, 8).is_ok_and(|l| l.iter().any(|m| lower(&m.text).contains(&lower(probe)))) {
                found = true;
                break;
            }
            std::thread::sleep(Duration::from_secs(3));
        }
        self.say(format!(
            "    captured {probe:?}: {} in {:.0}s",
            if found { "searchable" } else { "NOT FOUND" },
            started.elapsed().as_secs_f32()
        ));
    }

    /// The final question with memory on, then off; graded on what the
    /// answer must and must not contain.
    fn check(
        &mut self,
        scenario: &str,
        project: &Project,
        agent: Agent,
        prompt: &str,
        expect: &[String],
        forbid: &[String],
        secrets: &[String],
        file: Option<(&str, &[String])>,
    ) {
        // A secret must not be in the answer either.
        let forbid: Vec<String> = forbid.iter().chain(secrets).cloned().collect();
        let forbid = forbid.as_slice();
        let grade = |answer: &str, file_text: Option<String>| -> (bool, Vec<String>) {
            let a = lower(answer);
            let mut missing: Vec<String> = expect.iter().filter(|e| !a.contains(&lower(e))).cloned().collect();
            missing.extend(forbid.iter().filter(|f| a.contains(&lower(f))).map(|f| format!("forbidden {f}")));
            if let (Some((_, must)), text) = (file, file_text) {
                let t = lower(&text.unwrap_or_default());
                missing.extend(must.iter().filter(|m| !t.contains(&lower(m))).map(|m| format!("file lacks {m}")));
                missing.extend(forbid.iter().filter(|f| t.contains(&lower(f))).map(|f| format!("file has {f}")));
            }
            (missing.is_empty(), missing)
        };
        let read_file = |p: &Project| file.and_then(|(f, _)| std::fs::read_to_string(p.path.join(f)).ok());
        let snapshot = file.and_then(|(f, _)| std::fs::read_to_string(project.path.join(f)).ok());

        // The baseline first: Antigravity keeps its own summaries of past
        // conversations, so a run that saw Dove's recall block would leak
        // the answer into a later baseline.
        self.say(format!("  CHECK without memory ({})", agent.label()));
        let _ = self.engine.store.set_setting(memory::ENABLED, "false");
        let off = self.ask_with(project, agent, prompt, file.is_some());
        let _ = self.engine.store.set_setting(memory::ENABLED, "true");
        let (ok_off, _) = grade(&off.answer, read_file(project));
        // Undo any edit, so both runs start from the same code.
        if let (Some((f, _)), Some(text)) = (file, &snapshot) {
            let _ = std::fs::write(project.path.join(f), text);
        }

        self.say(format!("  CHECK with memory ({})", agent.label()));
        let on = self.ask_with(project, agent, prompt, file.is_some());
        let (ok_on, why_on) = grade(&on.answer, read_file(project));
        // Redaction: no secret may reach any agent, even inside recall.
        let leaked: Vec<&String> = secrets.iter().filter(|f| lower(&on.sent).contains(&lower(f))).collect();
        if !leaked.is_empty() {
            self.say(format!("    ! recall block carried a secret {leaked:?}"));
        }

        let verdict = |ok: bool, t: &Turn| match (ok, t.peeked) {
            (true, true) => "pass, but read outside the repo",
            (true, false) => "PASS",
            (false, _) => "fail",
        };
        self.say(format!(
            "  => {scenario}: memory {} {}| baseline {}",
            verdict(ok_on, &on),
            if ok_on { String::new() } else { format!("({}) ", why_on.join(", ")) },
            verdict(ok_off, &off)
        ));
        self.rows.push(format!(
            "| {scenario} | {} | **{}** | {} | {} | {:.0}s |",
            agent.label(),
            verdict(ok_on, &on),
            verdict(ok_off, &off),
            on.recalled().len(),
            on.secs
        ));
        if !ok_on || on.peeked || !leaked.is_empty() {
            self.failures.push(format!("{scenario}: {}", why_on.join(", ")));
        }
    }

    fn report(&mut self, extra: &str) {
        let mut md = format!(
            "# Dove memory bench {}\n\nOpenCode on DeepSeek V4.1 Flash and Antigravity on Gemini 3.8 Flash, through Dove's \
             recall and capture (Supermemory). Each final question runs in a brand-new session; the baseline runs the same \
             question with memory off.\n\n| Scenario | Answered by | With memory | Without memory | Memories recalled | Turn time |\n|---|---|---|---|---|---|\n",
            self.n
        );
        for r in &self.rows {
            md.push_str(r);
            md.push('\n');
        }
        md.push_str(extra);
        md.push_str("\n\n## Log\n\n```\n");
        md.push_str(&self.log);
        md.push_str("```\n");
        let path = self.out.join("report.md");
        std::fs::write(&path, &md).unwrap();
        println!("\nREPORT {}\n{}", path.display(), md.split("## Log").next().unwrap_or_default());
    }
}

fn s(v: &str) -> String {
    v.to_string()
}

fn uses(path: &Path) -> String {
    memory::project_tag(path)
}

#[test]
#[ignore]
fn memory_bench() {
    let mut b = Bench::new();
    let n = b.n.clone();
    let (oc, ag) = (Agent::OpenCode, Agent::Antigravity);
    let only = std::env::var("BENCH_ONLY").unwrap_or_default();
    let run = |name: &str| only.is_empty() || only.split(',').any(|o| o.trim().eq_ignore_ascii_case(name));

    // ---------------------------------------------------------------- easy
    if run("easy") {
        b.say(format!("\n== EASY: one fact, one hop (OpenCode teaches, Antigravity answers)"));
        let p = b.repo("easy", &format!("git@github.com:dove-bench/easy{n}.git"), &[("README.md", "# easy\n")]);
        b.teach(
            &p,
            oc,
            &format!("For this repo, run the tests with `pnpm vitest run --shard=e{n}`. Just acknowledge in one line."),
            &format!("shard=e{n}"),
        );
        b.check(
            "Easy · one fact",
            &p,
            ag,
            "How do I run this repo's tests? Reply with the exact command only, nothing else. Do not look at files.",
            &[format!("shard=e{n}")],
            &[],
            &[],
            None,
        );
    }

    // -------------------------------------------------------------- medium
    if run("medium") {
        b.say(format!("\n== MEDIUM: two facts from two agents, noise, an indirect question"));
        let p = b.repo("medium", &format!("https://github.com/dove-bench/medium{n}.git"), &[("README.md", "# medium\n")]);
        b.teach(
            &p,
            ag,
            &format!(
                "Deploy notes for this project: production deploys with `fly deploy --app orbit{n}`; staging is the app \
                 `orbit{n}-stg`, same command with that app name. Acknowledge in one line."
            ),
            &format!("orbit{n}-stg"),
        );
        b.teach(
            &p,
            oc,
            &format!(
                "Architecture decision {n}: we dropped Redis. Background jobs take locks with Postgres advisory locks \
                 (pg_try_advisory_lock) because Redis cost too much. Acknowledge in one line."
            ),
            "advisory",
        );
        b.ask(&p, oc, "Unrelated question: in one sentence, what is a mutex?");
        b.check(
            "Medium · combine two facts",
            &p,
            ag,
            "I'm adding a nightly cleanup job to this project. Which locking mechanism should it use here, and what exact \
             command deploys it to staging? Two short lines. Do not look at files.",
            &[s("advisory"), format!("orbit{n}-stg")],
            &[],
            &[],
            None,
        );
    }

    // ---------------------------------------------------------------- hard
    if run("hard") {
        b.say(format!("\n== HARD: a superseded fact, a code convention, noise, and a real edit"));
        let port = 40000 + n.parse::<u64>().unwrap_or(0) % 9000;
        let p = b.repo(
            "hard",
            &format!("git@github.com:dove-bench/hard{n}.git"),
            &[("src/utils.ts", "export function add(a: number, b: number) {\n  return a + b;\n}\n"), ("package.json", "{\"name\":\"hard\"}\n")],
        );
        b.teach(&p, oc, "Project note: the dev server runs on port 4100. Acknowledge in one line.", "4100");
        b.teach(
            &p,
            ag,
            &format!(
                "No, that's out of date: the dev server port changed from 4100 to {port}. Port 4100 is retired. \
                 Acknowledge in one line."
            ),
            &port.to_string(),
        );
        b.teach(
            &p,
            oc,
            &format!(
                "Code convention for this repo: every exported function in src/utils.ts must be named with the prefix \
                 `orb{n}_` (for example `orb{n}_add`). Don't change any code now; acknowledge in one line."
            ),
            &format!("orb{n}_"),
        );
        for q in ["In one sentence, what is tail-call optimisation?", "In one sentence, what does HTTP 418 mean?"] {
            b.ask(&p, ag, q);
        }
        b.check(
            "Hard · superseded fact + convention, real edit",
            &p,
            ag,
            "Add an exported function to src/utils.ts that returns the dev server URL (http://localhost:<port>) for this \
             project, following this project's conventions. Make the edit, then say the function name and the port.",
            &[port.to_string()],
            &[s("4100")],
            &[],
            Some(("src/utils.ts", &[format!("orb{n}_"), port.to_string()])),
        );
    }

    // -------------------------------------------------------------- stress
    if run("stress") {
        b.say(format!(
            "\n== STRESS: buried fact, superseded fact, a secret, an answer-style preference, taught in one clone and asked in \
             another clone (different folder, SSH vs HTTPS remote) by the other agent"
        ));
        let a = b.repo("stress-clone-a", &format!("git@github.com:Dove-Bench/Stress{n}.git"), &[("README.md", "# stress\n")]);
        let c = b.repo("stress-clone-b", &format!("https://github.com/dove-bench/stress{n}"), &[("README.md", "# stress\n")]);
        assert_eq!(uses(&a.path), uses(&c.path), "clones of one repo must share a container");
        let filler = "The team discussed onboarding, CI caching, flaky snapshot tests, the docs site rebuild, and whether \
                      to adopt a monorepo tool. Nothing was decided on most of it. ";
        b.teach(
            &a,
            ag,
            &format!(
                "Long design note, just acknowledge in one line. {} Somewhere in all this: our feature flag service is \
                 called `lumen{n}`, and every new flag must be registered there first. {}",
                filler.repeat(6),
                filler.repeat(6)
            ),
            &format!("lumen{n}"),
        );
        b.teach(&a, oc, &format!("Release branch for this repo is `rel-old{n}`. Acknowledge in one line."), &format!("rel-old{n}"));
        b.teach(
            &a,
            ag,
            &format!("No — the release branch is now `train{n}`; `rel-old{n}` was deleted. Acknowledge in one line."),
            &format!("train{n}"),
        );
        b.teach(
            &a,
            oc,
            &format!("For local testing the database password is password=hunter{n}xyzSECRET. Acknowledge in one line."),
            "database password",
        );
        b.teach(
            &a,
            ag,
            "My preference for this repo: start every answer about it with the tag ORB: (capital letters, then a colon). \
             Acknowledge in one line.",
            "ORB:",
        );
        b.check(
            "Stress · 5 facts, other clone, other agent",
            &c,
            oc,
            "Which feature flag service do we use, and which branch do releases go out from? Also, what's the local \
             database password? Answer briefly. Do not look at files.",
            &[format!("lumen{n}"), format!("train{n}"), s("ORB:")],
            &[format!("rel-old{n}")],
            &[format!("hunter{n}")],
            None,
        );
    }

    // ------------------------------------------------------ self-improving
    if run("learn") {
        b.say(format!("\n== SELF-IMPROVING: a correction in one agent changes what the other agent does"));
        let p = b.repo("learn", &format!("git@github.com:dove-bench/learn{n}.git"), &[("package.json", "{\"name\":\"learn\"}\n")]);
        b.ask(&p, oc, "Add the `zod` package to this project. Reply with the exact install command you'd run (don't run it).");
        b.teach(
            &p,
            oc,
            &format!("No, wrong tool: this repo uses `bun add` for every dependency (lockfile bun.lock{n}), never npm or pnpm. Acknowledge in one line."),
            "bun add",
        );
        b.check(
            "Self-improving · correction carried over",
            &p,
            ag,
            "Add the `dayjs` package to this project. Reply with the exact install command only (don't run it). Do not look \
             at files.",
            &[s("bun add")],
            &[s("npm install"), s("npm i "), s("pnpm add")],
            &[],
            None,
        );
    }

    // ------------------------------------------------------- consolidation
    let mut extra = String::new();
    if run("consolidate") || only.is_empty() {
        b.say("\n== CONSOLIDATION: duplicates and skill drafts from what was captured".to_string());
        match memory::consolidate(&b.engine.store) {
            Ok(r) => {
                b.say(format!(
                    "  scanned {} memories/documents, {} duplicates, {} skill drafts: {:?}",
                    r.scanned,
                    r.duplicate_count(),
                    r.skills.len(),
                    r.skills.iter().map(|s| (&s.name, &s.steps)).collect::<Vec<_>>()
                ));
                extra = format!(
                    "\n\nConsolidation: scanned {}, {} duplicates, skill drafts: {}.",
                    r.scanned,
                    r.duplicate_count(),
                    if r.skills.is_empty() { "none".into() } else { r.skills.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join(", ") }
                );
            }
            Err(e) => b.say(format!("  consolidation failed: {e}")),
        }
    }

    b.report(&extra);
    assert!(b.failures.is_empty(), "with memory, these failed: {:#?}", b.failures);
}

#[test]
fn flags_tool_calls_outside_the_repo() {
    let repo = Path::new("C:/dvw/mem-bench/1/easy");
    let inside = serde_json::json!({"tool_info": {"parameters": {"Path": r"C:\dvw\mem-bench\1\easy\README.md"}}});
    let parent = serde_json::json!({"tool_info": {"parameters": {"CommandLine": r#"Get-ChildItem -Path "C:\dvw\mem-e2e""#}}});
    let relative = serde_json::json!({"command": "ls src"});
    assert!(!outside(&inside, repo));
    assert!(outside(&parent, repo));
    assert!(!outside(&relative, repo));
}
