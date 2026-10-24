//! Measure Moonshine and Smart Turn under different ONNX Runtime settings
//! (graph optimization, threads, spinning) on this machine.
//!
//!   cargo run -p dove-voice --release --example stt_tune -- --root <dir with voice/call models>

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use dove_voice::engines::{SpeechOptions, SpeechToText, TextToSpeech};
use dove_voice::metrics::percentile;
use dove_voice::vad::OnnxTuning;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = args
        .iter()
        .position(|a| a == "--root")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("dove-voice-bench"));
    let mut tts = dove_voice::tts::System::new();
    let mut fixtures = Vec::new();
    for p in [
        "What is happening with the tests thread?",
        "Tell Codex to also run clippy on Windows.",
        "Add a retry with exponential backoff to the token refresh.",
        "Stop.",
    ] {
        let pcm = tts.synthesize(p, &SpeechOptions::default(), &AtomicBool::new(false))?;
        fixtures.push(dove_voice::resample::convert(&pcm.samples, pcm.rate, dove_voice::RATE));
    }
    for m in dove_voice::models::ALL {
        if !dove_voice::models::installed(&root, m) {
            dove_voice::models::download(&root, m, &AtomicBool::new(false), |_| {})?;
        }
    }
    println!("{:<34} {:>9} {:>9} {:>9}", "moonshine encoder/decoder", "load ms", "p50 ms", "p95 ms");
    for (opt, threads, spin) in [(3, 4, true), (3, 4, false), (3, 2, true), (3, 1, true), (1, 4, true), (2, 4, true)] {
        let t = OnnxTuning { threads, spin, optimize: opt };
        let start = Instant::now();
        let mut m = dove_voice::stt::Moonshine::load_tuned(&root, dove_voice::models::MoonshineSize::Base, t, t)?;
        let load = start.elapsed().as_secs_f32() * 1000.0;
        let _ = m.finish(&fixtures[0])?;
        let mut ms = Vec::new();
        for _ in 0..3 {
            for f in &fixtures {
                let s = Instant::now();
                let _ = m.finish(f)?;
                ms.push(s.elapsed().as_secs_f32() * 1000.0);
            }
        }
        println!(
            "{:<34} {:>9.0} {:>9.0} {:>9.0}",
            format!("opt {opt}, {threads} threads, spin {spin}"),
            load,
            percentile(&ms, 50.0).unwrap_or(0.0),
            percentile(&ms, 95.0).unwrap_or(0.0)
        );
    }
    for size in [dove_voice::models::MoonshineSize::Base, dove_voice::models::MoonshineSize::Tiny] {
        for m in size.files() {
            if !dove_voice::models::installed(&root, &m) {
                dove_voice::models::download(&root, &m, &AtomicBool::new(false), |_| {})?;
            }
        }
        let mut m = dove_voice::stt::Moonshine::load(&root, size)?;
        let mut total = Vec::new();
        for f in &fixtures {
            let (enc, steps, dec) = m.profile(f)?;
            total.push(enc + dec);
            println!(
                "{} {:.1} s audio: encoder {enc:.0} ms, {steps} decoder steps {dec:.0} ms ({:.1} ms/step): {}",
                size.label(),
                f.len() as f32 / 16000.0,
                dec / steps as f32,
                m.finish(f)?
            );
        }
        println!("{} p50 {:.0} ms", size.label(), percentile(&total, 50.0).unwrap_or(0.0));
    }
    println!("\n{:<34} {:>9} {:>9} {:>9}", "smart turn", "load ms", "p50 ms", "p95 ms");
    let path = dove_voice::models::path(&root, &dove_voice::models::SMART_TURN);
    let mut audio = vec![0.0f32; 16_000];
    audio.extend_from_slice(&fixtures[0]);
    for (threads, spin) in [(2, false), (2, true), (4, true), (1, false)] {
        let start = Instant::now();
        let mut st = dove_voice::turn::SmartTurn::load_tuned(&path, OnnxTuning { threads, spin, optimize: 3 })?;
        let load = start.elapsed().as_secs_f32() * 1000.0;
        let _ = st.probability(&audio)?;
        let mut ms = Vec::new();
        for _ in 0..12 {
            let s = Instant::now();
            let _ = st.probability(&audio)?;
            ms.push(s.elapsed().as_secs_f32() * 1000.0);
        }
        println!(
            "{:<34} {:>9.0} {:>9.1} {:>9.1}",
            format!("{threads} threads, spin {spin}"),
            load,
            percentile(&ms, 50.0).unwrap_or(0.0),
            percentile(&ms, 95.0).unwrap_or(0.0)
        );
    }
    let t = Instant::now();
    let feats = dove_voice::turn::mel::features(&audio);
    println!(
        "\nwhisper log-mel front end alone: {:.1} ms ({} values)",
        t.elapsed().as_secs_f32() * 1000.0,
        feats.len()
    );
    Ok(())
}
