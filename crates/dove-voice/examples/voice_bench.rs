//! Voice Mode benchmark: real models, real audio, real devices.
//! The default stack is Moonshine + Silero + Smart Turn + the system voice;
//! Parakeet is measured too when installed.
//!
//!   cargo run -p dove-voice --release --example voice_bench -- \
//!       --parakeet ~/.dove/voice/parakeet-v3 [--root <scratch>] [--json out.json] [--live-mic]
//!
//! Fixture speech is rendered by the operating system's voice (so the run
//! needs no recordings), resampled to 16 kHz and fed through the same
//! `Listener`, VAD, turn detector and speech recognizer a call uses.
//! Playback timings use the real default speaker. The microphone is opened
//! only with `--live-mic`.
//!
//! Every number printed is measured in this run. End-to-end figures add the
//! audio time a decision has to wait for (a pause is a pause) to the compute
//! measured here, per turn; nothing is averaged across stages.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use dove_voice::conductor::Conductor;
use dove_voice::engines::{SpeechOptions, SpeechToText, TextToSpeech};
use dove_voice::metrics::{cpu_time, percentile, resident_bytes};
use dove_voice::pipeline::{Heard, Listener};
use dove_voice::snapshot::{Agent, Project, Snapshot, Thread, ThreadStatus};
use serde_json::json;

struct Row {
    metric: &'static str,
    target: String,
    measured: String,
    verdict: &'static str,
    note: String,
}

fn verdict(ok: Option<bool>) -> &'static str {
    match ok {
        Some(true) => "PASS",
        Some(false) => "FAIL",
        None => "INFO",
    }
}

fn mb(b: u64) -> f64 {
    b as f64 / 1_048_576.0
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn system_info() -> serde_json::Value {
    let ps = |cmd: &str| -> String {
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", cmd])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let (cpu, ram) = if cfg!(windows) {
        (
            ps("(Get-CimInstance Win32_Processor | Select-Object -First 1).Name"),
            ps("[math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory/1GB,1)"),
        )
    } else {
        (String::new(), String::new())
    };
    json!({
        "os": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        "cpu": cpu,
        "ram_gb": ram,
        "threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "build": if cfg!(debug_assertions) { "debug" } else { "release" },
    })
}

fn speak(tts: &mut dove_voice::tts::System, text: &str) -> anyhow::Result<Vec<f32>> {
    let pcm = tts.synthesize(text, &SpeechOptions::default(), &AtomicBool::new(false))?;
    Ok(dove_voice::resample::convert(&pcm.samples, pcm.rate, dove_voice::RATE))
}

fn silence(ms: u32) -> Vec<f32> {
    // A quiet room, not digital zero: VADs behave differently on exact zeros.
    let n = (dove_voice::RATE * ms / 1000) as usize;
    let mut s = 12345u32;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((s >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.002
        })
        .collect()
}

struct TurnRun {
    audio: Vec<f32>,
    /// Audio-time ms from the last voiced frame to the turn decision.
    endpoint_ms: u32,
    /// Audio-time ms from the last voiced frame to the last speculation.
    speculate_ms: Option<u32>,
    speculated_audio: Option<Vec<f32>>,
    speculated: bool,
    costs: Vec<f32>,
    timings: dove_voice::metrics::TurnTimings,
}

/// Feed `audio` through a listener, as the call thread does.
fn run_turn(listener: &mut Listener, audio: &[f32]) -> Option<TurnRun> {
    let frame_ms = (dove_voice::FRAME as u32 * 1000) / dove_voice::RATE;
    let mut costs = Vec::new();
    let reference = vec![0.0; dove_voice::FRAME];
    let mut spec: Option<(u32, Vec<f32>)> = None;
    let mut silent_run = 0u32;
    for f in audio.chunks_exact(dove_voice::FRAME) {
        let mut f = f.to_vec();
        let t = Instant::now();
        let h = listener.feed(&mut f, &reference, false);
        costs.push(t.elapsed().as_secs_f32() * 1000.0);
        silent_run += 1;
        match h {
            Some(Heard::Speculate(a)) => spec = Some((silent_run * frame_ms, a)),
            Some(Heard::Ended { audio, timings, speculated }) => {
                return Some(TurnRun {
                    audio,
                    endpoint_ms: timings.endpoint_silence_ms,
                    speculate_ms: spec.as_ref().map(|(_, _)| listener.cfg.speculate_frames * frame_ms),
                    speculated_audio: spec.map(|(_, a)| a),
                    speculated,
                    costs,
                    timings,
                });
            }
            _ => {}
        }
    }
    None
}

fn main() -> anyhow::Result<()> {
    let parakeet = arg("--parakeet").map(PathBuf::from).unwrap_or_else(|| {
        let home =
            std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
        home.join(".dove").join("voice").join("parakeet-v3")
    });
    let root = arg("--root").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("dove-voice-bench"));
    let mut rows: Vec<Row> = Vec::new();
    let info = system_info();
    println!("Dove Voice Mode benchmark\n{}\n", serde_json::to_string_pretty(&info)?);

    // Models Voice Mode adds (pinned, hash-checked; ~11 MB).
    for m in dove_voice::models::ALL {
        if !dove_voice::models::installed(&root, m) {
            print!("Downloading {} ({:.1} MB)… ", m.name, m.size as f64 / 1e6);
            dove_voice::models::download(&root, m, &AtomicBool::new(false), |_| {})?;
            println!("verified");
        }
    }

    // Fixture speech from the system voice.
    let mut tts = dove_voice::tts::System::new();
    let phrases = [
        "What is happening with the tests thread?",
        "Tell Codex to also run clippy on Windows.",
        "Add a retry with exponential backoff to the token refresh.",
        "Stop.",
    ];
    let mut fixtures = Vec::new();
    for p in phrases {
        fixtures.push((p, speak(&mut tts, p)?));
    }

    let base_rss = resident_bytes().unwrap_or(0);

    // Cold start of the default local stack: Moonshine + Silero + Smart Turn,
    // each file's SHA-256 checked before ONNX Runtime loads it.
    let size = arg("--stt")
        .and_then(|s| dove_voice::models::MoonshineSize::from_id(&format!("moonshine-{s}")))
        .unwrap_or(dove_voice::models::MoonshineSize::Base);
    // As a call loads them: speech recognition on its own thread, the VAD
    // and turn detector on the call thread, at the same time.
    let t = Instant::now();
    let r2 = root.clone();
    let stt_thread = std::thread::spawn(move || {
        let t = Instant::now();
        dove_voice::stt::Moonshine::load(&r2, size).map(|m| (m, t.elapsed()))
    });
    let t2 = Instant::now();
    let silero = dove_voice::vad::SileroVad::load(&dove_voice::models::path(&root, &dove_voice::models::SILERO))?;
    let smart = dove_voice::turn::SmartTurn::load(&dove_voice::models::path(&root, &dove_voice::models::SMART_TURN))?;
    let small_load = t2.elapsed();
    let (mut stt, stt_load) = stt_thread.join().map_err(|_| anyhow::anyhow!("load panicked"))??;
    let listening_ready = t.elapsed();
    let player_t = Instant::now();
    let player = dove_voice::player::Player::start(None);
    let player_open = player_t.elapsed();
    rows.push(Row {
        metric: "Cold call startup (to listening-ready)",
        target: "< 1500 ms".into(),
        measured: format!("{:.0} ms", listening_ready.as_secs_f64() * 1000.0),
        verdict: verdict(Some(listening_ready < Duration::from_millis(1500))),
        note: format!(
            "{} {:.0} ms + Silero/Smart Turn {:.0} ms (hash-checked loads); the speaker opens on its own thread ({:.0} ms here); mic open not included",
            size.label(),
            stt_load.as_secs_f64() * 1000.0,
            small_load.as_secs_f64() * 1000.0,
            player_open.as_secs_f64() * 1000.0
        ),
    });

    // Warm start: engines cached from a previous call; what is left is
    // building the listener (the speaker opens alongside, not before).
    let t = Instant::now();
    let warm_listener = Listener::new(
        Box::new(dove_voice::vad::EnergyVad::default()),
        Some(Box::new(dove_voice::aec::Nlms::default())),
        Box::new(dove_voice::turn::PauseTurn::default()),
    );
    let warm = t.elapsed();
    drop(warm_listener);
    rows.push(Row {
        metric: "Warm call startup (cached engines)",
        target: "< 100 ms".into(),
        measured: format!("{:.2} ms", warm.as_secs_f64() * 1000.0),
        verdict: verdict(Some(warm < Duration::from_millis(100))),
        note: "excludes opening the microphone (driver-dependent; measure with --live-mic)".into(),
    });

    let loaded_rss = resident_bytes().unwrap_or(0);
    rows.push(Row {
        metric: "Active local voice stack RAM",
        target: "< 400 MB".into(),
        measured: format!("{:.0} MB", mb(loaded_rss.saturating_sub(base_rss))),
        verdict: verdict(Some(loaded_rss.saturating_sub(base_rss) < 400 * 1_048_576)),
        note: format!(
            "working set {:.0} → {:.0} MB with {}, Silero, Smart Turn and the speaker",
            mb(base_rss),
            mb(loaded_rss),
            size.label()
        ),
    });

    // Recognition: what each fixture came back as, and the compute.
    println!("Transcripts (system voice in, {} out):", size.label());
    let mut stt_ms = Vec::new();
    for (want, audio) in &fixtures {
        let t = Instant::now();
        let got = stt.finish(audio)?;
        stt_ms.push(t.elapsed().as_secs_f32() * 1000.0);
        println!("  said: {want}\n  got:  {got}");
    }
    for _ in 0..3 {
        for (_, audio) in &fixtures {
            let t = Instant::now();
            let _ = stt.finish(audio)?;
            stt_ms.push(t.elapsed().as_secs_f32() * 1000.0);
        }
    }
    let stt_p50 = percentile(&stt_ms, 50.0).unwrap_or(0.0);
    let stt_p95 = percentile(&stt_ms, 95.0).unwrap_or(0.0);
    rows.push(Row {
        metric: "Speech recognition compute (one turn)",
        target: "-".into(),
        measured: format!("p50 {stt_p50:.0} ms, p95 {stt_p95:.0} ms"),
        verdict: "INFO",
        note: format!("{} int8 on 1-4 s fixtures", size.label()),
    });

    // Turn end on the fixtures, through the real listener, with speculation:
    // the pause is transcribed after 128 ms while the detector decides.
    let mut listener =
        Listener::new(Box::new(silero), Some(Box::new(dove_voice::aec::Nlms::default())), Box::new(smart));
    let mut e2e = Vec::new();
    let mut finalize = Vec::new();
    let mut frame_costs = Vec::new();
    let mut decide = Vec::new();
    let mut endpoints = Vec::new();
    for (_, audio) in &fixtures {
        let mut input = silence(600);
        input.extend_from_slice(audio);
        input.extend(silence(2500));
        let Some(run) = run_turn(&mut listener, &input) else {
            println!("  (no turn detected for a fixture)");
            continue;
        };
        // The recognizer starts at the speculation point; the decision lands
        // at the endpoint. Text is ready at whichever finishes later.
        let (text_ready, after_decision) = match (run.speculated, &run.speculated_audio, run.speculate_ms) {
            (true, Some(a), Some(at)) => {
                let t = Instant::now();
                let _ = stt.finish(a)?;
                let stt_done = at as f32 + t.elapsed().as_secs_f32() * 1000.0;
                let decided = run.endpoint_ms as f32 + run.timings.turn_decide_ms;
                (stt_done.max(decided), (stt_done - decided).max(0.0))
            }
            _ => {
                let t = Instant::now();
                let _ = stt.finish(&run.audio)?;
                let ms = t.elapsed().as_secs_f32() * 1000.0;
                (run.endpoint_ms as f32 + run.timings.turn_decide_ms + ms, ms)
            }
        };
        e2e.push(text_ready);
        finalize.push(after_decision);
        endpoints.push(run.endpoint_ms as f32);
        decide.push(run.timings.turn_decide_ms);
        frame_costs.extend(run.costs);
    }
    let e2e_p50 = percentile(&e2e, 50.0).unwrap_or(f32::NAN);
    let e2e_p95 = percentile(&e2e, 95.0).unwrap_or(f32::NAN);
    rows.push(Row {
        metric: "Final transcript after end of speech",
        target: "< 120 ms".into(),
        measured: format!(
            "after decision p95 {:.0} ms; from last word p50 {e2e_p50:.0} / p95 {e2e_p95:.0} ms",
            percentile(&finalize, 95.0).unwrap_or(f32::NAN)
        ),
        verdict: verdict(percentile(&finalize, 95.0).map(|f| f < 120.0)),
        note: format!(
            "verdict on text-ready after the turn decision; from the last word adds the pause the detector waits for (p50 {:.0} ms of audio)",
            percentile(&endpoints, 50.0).unwrap_or(0.0)
        ),
    });
    rows.push(Row {
        metric: "End-of-turn decision (CPU)",
        target: "< 60 ms".into(),
        measured: format!("p95 {:.1} ms", percentile(&decide, 95.0).unwrap_or(0.0)),
        verdict: verdict(Some(percentile(&decide, 95.0).unwrap_or(f32::MAX) < 60.0)),
        note: "Smart Turn v3.2 inference over the 8 s window, all calls per turn".into(),
    });
    rows.push(Row {
        metric: "Per-frame listening cost (VAD + AEC)",
        target: "< 32 ms per 32 ms frame".into(),
        measured: format!(
            "p50 {:.2} ms, p95 {:.2} ms",
            percentile(&frame_costs, 50.0).unwrap_or(0.0),
            percentile(&frame_costs, 95.0).unwrap_or(0.0)
        ),
        verdict: "INFO",
        note: "Silero VAD on ONNX Runtime (1 thread) + NLMS".into(),
    });

    // Fast path: grammar + conductor + first clause rendered + speaker start.
    let snap = Snapshot {
        threads: vec![Thread {
            id: "t1".into(),
            title: "Make the tests pass".into(),
            agent: "Codex".into(),
            agent_id: "codex".into(),
            project: "dove".into(),
            status: ThreadStatus::Running,
            live: true,
            steers: true,
            interrupts: true,
            session_approvals: true,
            pending: vec![],
            activity: Some("Running cargo test".into()),
            todos: Some((2, 5, Some("Fix the flaky test".into()))),
            diff: None,
            cwd: None,
            error: None,
        }],
        current: Some("t1".into()),
        agents: vec![Agent { id: "codex".into(), label: "Codex".into() }],
        projects: vec![Project { id: "p".into(), name: "dove".into() }],
        default_project: Some("p".into()),
        showing: None,
    };
    let mut fast_total = Vec::new();
    let mut route_us = Vec::new();
    let mut synth_ms = Vec::new();
    let mut start_ms = Vec::new();
    if let Ok(player) = &player {
        for utterance in ["stop", "what's happening", "approve", "talk faster", "status"] {
            let mut c = Conductor::new();
            let t = Instant::now();
            let effects = c.utterance(utterance, &snap, false);
            route_us.push(t.elapsed().as_secs_f32() * 1_000_000.0);
            let say = effects
                .iter()
                .find_map(|e| if let dove_voice::conductor::Effect::Speak(s) = e { Some(s.clone()) } else { None });
            let first = say.as_deref().map(|s| {
                let mut ch = dove_voice::speakable::Chunker::new();
                let mut v = ch.push(s);
                v.extend(ch.finish());
                v.into_iter().next().unwrap_or_default()
            });
            let Some(first) = first else { continue };
            let t = Instant::now();
            let pcm = tts.synthesize(&first, &SpeechOptions::default(), &AtomicBool::new(false))?;
            let synth = t.elapsed().as_secs_f32() * 1000.0;
            player.play(&pcm);
            std::thread::sleep(Duration::from_millis(300));
            let start = player.shared().last_start_latency_us() as f32 / 1000.0;
            player.stop();
            std::thread::sleep(Duration::from_millis(150));
            synth_ms.push(synth);
            start_ms.push(start);
            fast_total.push(route_us.last().unwrap() / 1000.0 + synth + start);
        }
    }
    rows.push(Row {
        metric: "Fast-path command to first audio",
        target: "< 250 ms".into(),
        measured: format!("p50 {:.0} ms, max {:.0} ms", percentile(&fast_total, 50.0).unwrap_or(f32::NAN), percentile(&fast_total, 100.0).unwrap_or(f32::NAN)),
        verdict: verdict(percentile(&fast_total, 100.0).map(|m| m < 250.0)),
        note: format!(
            "after the final transcript: routing p50 {:.0} µs + system voice first clause p50 {:.0} ms + speaker start p50 {:.0} ms",
            percentile(&route_us, 50.0).unwrap_or(0.0),
            percentile(&synth_ms, 50.0).unwrap_or(0.0),
            percentile(&start_ms, 50.0).unwrap_or(0.0)
        ),
    });

    // Added overhead on an LLM turn, TTFT excluded: transcript after speech
    // end, plus the first reply clause rendered and started.
    let reply_clause = "I found the flaky test,";
    let t = Instant::now();
    let pcm = tts.synthesize(reply_clause, &SpeechOptions::default(), &AtomicBool::new(false))?;
    let reply_synth = t.elapsed().as_secs_f32() * 1000.0;
    let reply_start = if let Ok(player) = &player {
        player.play(&pcm);
        std::thread::sleep(Duration::from_millis(300));
        let s = player.shared().last_start_latency_us() as f32 / 1000.0;
        player.stop();
        s
    } else {
        f32::NAN
    };
    let overhead = e2e_p50 + reply_synth + reply_start;
    rows.push(Row {
        metric: "Voice overhead on an LLM turn (excl. TTFT)",
        target: "< 300 ms".into(),
        measured: format!("{overhead:.0} ms (p50 path)"),
        verdict: verdict(Some(overhead < 300.0)),
        note: format!("text ready after the last word {e2e_p50:.0} ms (p50, includes the pause) + first clause synth {reply_synth:.0} ms + speaker start {reply_start:.0} ms"),
    });

    // Barge-in: speech playing on the real speaker, then stop.
    let mut stops = Vec::new();
    if let Ok(player) = &player {
        let long = tts.synthesize(
            "This is a long answer that keeps going so that it can be interrupted in the middle of a sentence by the user.",
            &SpeechOptions::default(),
            &AtomicBool::new(false),
        )?;
        for _ in 0..5 {
            player.play(&long);
            std::thread::sleep(Duration::from_millis(600));
            player.stop();
            std::thread::sleep(Duration::from_millis(120));
            stops.push(player.shared().last_stop_latency_us() as f32 / 1000.0);
        }
    }
    let detect_ms = 4.0 * 32.0;
    rows.push(Row {
        metric: "Barge-in playback stop",
        target: "< 100 ms".into(),
        measured: format!("stop→silence p50 {:.1} ms, max {:.1} ms", percentile(&stops, 50.0).unwrap_or(f32::NAN), percentile(&stops, 100.0).unwrap_or(f32::NAN)),
        verdict: verdict(percentile(&stops, 100.0).map(|m| m < 100.0)),
        note: format!(
            "speaker queue cleared to device silence; detecting the user over Dove first takes {detect_ms:.0} ms of audio (4 voiced frames), so voice-onset→silence ≈ {:.0} ms",
            detect_ms + percentile(&stops, 50.0).unwrap_or(0.0)
        ),
    });

    // Where idle CPU goes: the process doing nothing, with and without the
    // speaker stream open.
    let idle = |secs: u64| -> Option<f64> {
        let c0 = cpu_time()?;
        let w = Instant::now();
        std::thread::sleep(Duration::from_secs(secs));
        Some((cpu_time()? - c0).as_secs_f64() / w.elapsed().as_secs_f64() * 100.0)
    };
    let idle_with_speaker = idle(5);

    // Listening while silent: a real-time paced 10 s of quiet through the
    // listener (VAD + AEC + turn), as the call thread runs it.
    let mut quiet_listener = Listener::new(
        Box::new(dove_voice::vad::SileroVad::load(&dove_voice::models::path(&root, &dove_voice::models::SILERO))?),
        Some(Box::new(dove_voice::aec::Nlms::default())),
        Box::new(dove_voice::turn::PauseTurn::default()),
    );
    let quiet = silence(10_000);
    let cpu0 = cpu_time();
    let wall = Instant::now();
    let reference = vec![0.0; dove_voice::FRAME];
    for (i, f) in quiet.chunks_exact(dove_voice::FRAME).enumerate() {
        let due = Duration::from_millis(32 * i as u64);
        if let Some(wait) = due.checked_sub(wall.elapsed()) {
            std::thread::sleep(wait);
        }
        let mut f = f.to_vec();
        let _ = quiet_listener.feed(&mut f, &reference, false);
    }
    let cpu = match (cpu0, cpu_time()) {
        (Some(a), Some(b)) => Some((b - a).as_secs_f64() / wall.elapsed().as_secs_f64() * 100.0),
        _ => None,
    };
    let speaker_open = player.is_ok();
    drop(player);
    let idle_without = idle(5);
    rows.push(Row {
        metric: "Idle process CPU (diagnostic)",
        target: "-".into(),
        measured: format!(
            "{} with the speaker stream open, {} with no streams",
            idle_with_speaker.map_or("n/a".into(), |c| format!("{c:.2}%")),
            idle_without.map_or("n/a".into(), |c| format!("{c:.2}%"))
        ),
        verdict: "INFO",
        note: format!("loaded models idle; speaker stream was {}", if speaker_open { "open" } else { "unavailable" }),
    });
    rows.push(Row {
        metric: "Listening CPU while silent",
        target: "< 3% of one core".into(),
        measured: cpu.map(|c| format!("{c:.2}%")).unwrap_or_else(|| "unavailable".into()),
        verdict: verdict(cpu.map(|c| c < 3.0)),
        note: "Silero VAD + NLMS + pause detector at real-time pace; excludes the audio driver's capture thread".into(),
    });

    // The high-accuracy option, measured the same way, for comparison.
    if parakeet.join("verified").exists() {
        let before = resident_bytes().unwrap_or(0);
        let t = Instant::now();
        let mut p = dove_voice::stt::Parakeet::load(&parakeet)?;
        let load = t.elapsed();
        let rss = resident_bytes().unwrap_or(0).saturating_sub(before);
        let mut ms = Vec::new();
        for (_, audio) in &fixtures {
            let t = Instant::now();
            let _ = p.finish(audio)?;
            ms.push(t.elapsed().as_secs_f32() * 1000.0);
        }
        rows.push(Row {
            metric: "Parakeet (high-accuracy option)",
            target: "-".into(),
            measured: format!(
                "load {:.0} ms, +{:.0} MB, p50 {:.0} ms/turn",
                load.as_secs_f64() * 1000.0,
                mb(rss),
                percentile(&ms, 50.0).unwrap_or(0.0)
            ),
            verdict: "INFO",
            note: "NVIDIA Parakeet TDT 0.6B v3 int8 (CC-BY-4.0)".into(),
        });
    }

    // Unload: drop every engine and see what stays resident.
    drop(stt);
    drop(listener);
    drop(quiet_listener);
    std::thread::sleep(Duration::from_millis(500));
    let after = resident_bytes().unwrap_or(0);
    rows.push(Row {
        metric: "Memory after the voice stack is dropped",
        target: "0 MB (subsystem unloaded)".into(),
        measured: format!("{:+.0} MB vs. before loading", mb(after) - mb(base_rss)),
        verdict: verdict(Some(after <= base_rss + 20 * 1_048_576)),
        note:
            "allocator and ONNX Runtime may keep freed pages; an idle Dove that never started a call loads none of this"
                .into(),
    });

    if flag("--live-mic") {
        println!("\n--live-mic: opening the default microphone for 10 s of listening (nothing is stored)…");
        let call = dove_voice::pipeline::Call::start(dove_voice::pipeline::CallConfig {
            root: root.clone(),
            input: dove_voice::pipeline::Input::Device(None),
            output: dove_voice::pipeline::Output::Device(None),
            push_to_talk: false,
            echo_cancel: true,
            rate: 1.0,
            voice: None,
            stt: dove_voice::pipeline::SttChoice::Moonshine(root.clone(), size),
            tts: dove_voice::pipeline::TtsChoice::System,
            earcons: false,
        });
        let start = Instant::now();
        let mut ready = None;
        let cpu0 = cpu_time();
        while start.elapsed() < Duration::from_secs(12) {
            while let Ok(ev) = call.events.try_recv() {
                if let dove_voice::pipeline::Event::Ready { startup_ms, warm, .. } = ev {
                    ready = Some((startup_ms, warm));
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let cpu = match (cpu0, cpu_time()) {
            (Some(a), Some(b)) => (b - a).as_secs_f64() / 12.0 * 100.0,
            _ => f64::NAN,
        };
        call.end();
        rows.push(Row {
            metric: "Live call: mic-to-ready and listening CPU",
            target: "< 100 ms warm / < 3%".into(),
            measured: format!("ready {:?}, CPU {cpu:.2}%", ready),
            verdict: "INFO",
            note: "real microphone; includes the capture thread".into(),
        });
    }

    println!("\n{:<44} {:<26} {:<34} {:<5} notes", "metric", "target", "measured", "");
    for r in &rows {
        println!("{:<44} {:<26} {:<34} {:<5} {}", r.metric, r.target, r.measured, r.verdict, r.note);
    }
    if let Some(out) = arg("--json") {
        let doc = json!({
            "system": info,
            "rows": rows.iter().map(|r| json!({"metric": r.metric, "target": r.target, "measured": r.measured, "verdict": r.verdict, "note": r.note})).collect::<Vec<_>>(),
        });
        std::fs::write(out, serde_json::to_string_pretty(&doc)?)?;
    }
    Ok(())
}
