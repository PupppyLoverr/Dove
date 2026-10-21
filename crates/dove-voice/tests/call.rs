//! The whole call pipeline on its real threads: pushed audio in (system
//! voice, so no recordings are checked in), Silero + Smart Turn + Moonshine,
//! events out. Needs the Voice Mode models; set DOVE_VOICE_MODELS to a data
//! folder that has them (`voice_bench --root <dir>` downloads them), else the
//! test explains itself and is skipped.

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use dove_voice::engines::{SpeechOptions, TextToSpeech};
use dove_voice::pipeline::{Call, CallConfig, Event, Input, Output, SttChoice, TtsChoice};

fn models() -> Option<std::path::PathBuf> {
    let root = std::path::PathBuf::from(std::env::var_os("DOVE_VOICE_MODELS")?);
    let ok = dove_voice::models::ALL
        .iter()
        .filter(|m| !m.id.starts_with("moonshine-tiny"))
        .all(|m| dove_voice::models::installed(&root, m));
    ok.then_some(root)
}

fn speech(text: &str) -> Vec<f32> {
    let pcm = dove_voice::tts::System::new()
        .synthesize(text, &SpeechOptions::default(), &AtomicBool::new(false))
        .expect("system voice");
    dove_voice::resample::convert(&pcm.samples, pcm.rate, dove_voice::RATE)
}

/// Push `audio` in 32 ms chunks at real-time pace.
fn push(tx: &crossbeam_channel::Sender<Vec<f32>>, audio: &[f32]) {
    let start = Instant::now();
    for (i, chunk) in audio.chunks(512).enumerate() {
        if let Some(wait) = Duration::from_millis(32 * i as u64).checked_sub(start.elapsed()) {
            std::thread::sleep(wait);
        }
        let _ = tx.send(chunk.to_vec());
    }
}

fn quiet(ms: usize) -> Vec<f32> {
    (0..16 * ms).map(|i| 0.0005 * (((i * 7919) % 101) as f32 / 101.0 - 0.5)).collect()
}

#[test]
#[ignore = "needs the Voice Mode models (DOVE_VOICE_MODELS) and a system voice"]
fn spoken_turns_come_out_as_text_in_order_offline() {
    let Some(root) = models() else {
        eprintln!("DOVE_VOICE_MODELS is not set to a folder with the Voice Mode models; skipping");
        return;
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let call = Call::start(CallConfig {
        root: root.clone(),
        input: Input::Stream { rate: dove_voice::RATE, samples: rx },
        output: Output::Silent,
        push_to_talk: false,
        echo_cancel: true,
        rate: 1.0,
        voice: None,
        stt: SttChoice::Moonshine(root, dove_voice::models::MoonshineSize::Base),
        tts: TtsChoice::System,
        earcons: false,
    });
    // Ready first.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut ready = None;
    while ready.is_none() && Instant::now() < deadline {
        if let Ok(Event::Ready { startup_ms, stt, vad, turn, .. }) =
            call.events.recv_timeout(Duration::from_millis(100))
        {
            ready = Some((startup_ms, stt, vad, turn));
        }
    }
    let (startup_ms, stt, vad, turn) = ready.expect("call became ready");
    assert_eq!(stt, "Moonshine base");
    assert_eq!(vad, "silero");
    assert!(turn.starts_with("smart-turn"));
    eprintln!("ready in {startup_ms} ms");

    let phrases = ["What is happening with the tests thread?", "Stop."];
    let mut audio = quiet(500);
    for p in phrases {
        audio.extend(speech(p));
        audio.extend(quiet(1600));
    }
    let feeder = std::thread::spawn(move || {
        push(&tx, &audio);
        tx
    });
    let mut turns = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while turns.len() < phrases.len() && Instant::now() < deadline {
        match call.events.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Turn { text, timings }) => {
                eprintln!("turn: {text:?} ({timings:?})");
                turns.push((text, timings));
            }
            Ok(Event::Error(e)) => panic!("call error: {e}"),
            _ => {}
        }
    }
    let _tx = feeder.join().unwrap();
    call.end();
    assert_eq!(turns.len(), 2, "{turns:?}");
    let norm = |s: &str| s.to_lowercase().replace(['.', '?', ','], "");
    assert!(norm(&turns[0].0).starts_with("what is happening with the test"), "{}", turns[0].0);
    assert!(norm(&turns[1].0) == "stop", "{}", turns[1].0);
    // Each turn ended within a second and a half of the last word.
    for (_, t) in &turns {
        assert!(t.end_of_speech_to_text_ms < 1500.0, "{t:?}");
    }
    // The fast path recognises the second turn without a model.
    assert_eq!(dove_voice::fastpath::parse(&turns[1].0), Some(dove_voice::fastpath::FastCommand::Stop));
}
