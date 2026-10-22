//! Live dictation, measured: phrases spoken by the system voice are streamed
//! in real time into [`LiveDictation`] with a real local model, and the bench
//! reports when live text first appears and how long after the speaker
//! stopped each phrase was committed.
//!
//! `cargo run -p dove-voice --release --example dictation_bench -- --root <data dir with voice/call models> [--parakeet <dir>]`

use std::time::{Duration, Instant};

use crossbeam_channel::unbounded;
use dove_voice::dictation::{DictationConfig, Event, LiveDictation};
use dove_voice::pipeline::{Input, SttChoice};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

const PHRASES: &[&str] = &[
    "Add a retry with exponential backoff to the token refresh.",
    "Then run the tests on Windows and tell me what failed.",
    "Keep the change small and explain it in the pull request.",
];
const PAUSE_MS: usize = 1200;

fn main() -> anyhow::Result<()> {
    let root = std::path::PathBuf::from(arg("--root").expect("--root <dir>"));
    let stt = match arg("--parakeet") {
        Some(dir) => SttChoice::Parakeet(dir.into()),
        None => SttChoice::Moonshine(root.clone(), dove_voice::models::MoonshineSize::Base),
    };

    // Render the script: quiet, phrase, pause, phrase, ... at 16 kHz.
    let mut tts = dove_voice::tts::System::new();
    let mut audio = vec![0.0f32; 16 * 800];
    let mut spans = Vec::new(); // (start_ms, end_ms) of each phrase in the stream
    for p in PHRASES {
        let pcm = dove_voice::engines::TextToSpeech::synthesize(
            &mut tts,
            p,
            &Default::default(),
            &std::sync::atomic::AtomicBool::new(false),
        )?;
        let samples = dove_voice::resample::convert(&pcm.samples, pcm.rate, dove_voice::RATE);
        let start = audio.len() / 16;
        audio.extend_from_slice(&samples);
        spans.push((start, audio.len() / 16));
        audio.extend(std::iter::repeat_n(0.0, 16 * PAUSE_MS));
    }

    let (tx, rx) = unbounded();
    let cfg = DictationConfig::new(root, stt, Input::Stream { rate: dove_voice::RATE, samples: rx });
    let t_start = Instant::now();
    let d = LiveDictation::start(cfg);
    let ready_ms = loop {
        match d.events.recv_timeout(Duration::from_secs(30))? {
            Event::Ready { stt, startup_ms } => {
                println!("ready: {stt} in {startup_ms} ms");
                break startup_ms;
            }
            Event::Error(e) => anyhow::bail!(e),
            _ => {}
        }
    };
    let _ = ready_ms;

    // Feed in real time on another thread; `t0` is when the audio clock starts.
    let t0 = Instant::now();
    let feeder = std::thread::spawn(move || {
        for (i, chunk) in audio.chunks(512).enumerate() {
            let due = Duration::from_millis(i as u64 * 32);
            if let Some(wait) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(wait);
            }
            if tx.send(chunk.to_vec()).is_err() {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    });

    let mut first_live: Vec<Option<u128>> = vec![None; PHRASES.len()];
    let mut commits: Vec<Option<u128>> = vec![None; PHRASES.len()];
    let mut updates = 0;
    let mut last = String::new();
    let mut committed_count = 0;
    let end_ms = spans.last().unwrap().1 + PAUSE_MS + 3000;
    // Dictation ends by itself when the fed audio runs out.
    let mut ended: Option<String> = None;
    while t0.elapsed() < Duration::from_millis(end_ms as u64) && ended.is_none() {
        let Ok(ev) = d.events.recv_timeout(Duration::from_millis(50)) else { continue };
        let now = t0.elapsed().as_millis();
        if let Event::Ended { text } = &ev {
            ended = Some(text.clone());
        }
        if let Event::Text { committed, live } = ev {
            updates += 1;
            let phrase = committed_count.min(PHRASES.len() - 1);
            if !live.is_empty() && first_live[phrase].is_none() {
                first_live[phrase] = Some(now.saturating_sub(spans[phrase].0 as u128));
            }
            // Each pause commits one phrase: a change here is the next phrase.
            if committed != last {
                if committed_count < PHRASES.len() {
                    commits[committed_count] = Some(now.saturating_sub(spans[committed_count].1 as u128));
                    committed_count += 1;
                }
                println!("[{now:>6} ms] committed: {committed}");
                last = committed;
            } else if !live.is_empty() {
                println!("[{now:>6} ms]   live: {live}");
            }
        }
    }
    let text = match ended {
        Some(t) => t,
        None => {
            d.stop();
            loop {
                if let Event::Ended { text } = d.events.recv_timeout(Duration::from_secs(10))? {
                    break text;
                }
            }
        }
    };
    feeder.join().ok();
    println!("\nfinal: {text}\n");
    println!("{:<4} {:>14} {:>22}", "", "first live text", "committed after speech");
    for i in 0..PHRASES.len() {
        let f = first_live[i].map_or("-".into(), |v| format!("{v} ms after start"));
        let c = commits[i].map_or("-".into(), |v| format!("{v} ms"));
        println!("#{:<3} {:>22} {:>16}", i + 1, f, c);
    }
    println!("{updates} text updates; total {:.1} s", t_start.elapsed().as_secs_f32());
    Ok(())
}
