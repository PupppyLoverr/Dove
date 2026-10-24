//! Cancellable speech playback.
//!
//! Dove plays its own audio (rather than handing text to an OS speaker
//! API) for two reasons: [`Player::stop`] silences it within one device
//! buffer when the user talks over it, and every sample played is handed to
//! the echo canceller as its reference, at 16 kHz, in step with playback.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

use crate::engines::Pcm;

#[derive(Default)]
struct Queue {
    /// Samples at the device rate, mono.
    out: VecDeque<f32>,
    /// The same audio at 16 kHz, released to `played` as `out` drains.
    reference: VecDeque<f32>,
    /// Fractional reference samples owed per output sample.
    owed: f64,
}

pub struct Shared {
    queue: Mutex<Queue>,
    /// What was actually rendered, at 16 kHz, for the echo canceller.
    played: Mutex<VecDeque<f32>>,
    playing: AtomicBool,
    level: AtomicU32,
    /// Set by `stop`; the callback records when silence actually went out.
    stop_requested: Mutex<Option<Instant>>,
    last_stop_us: AtomicU64,
    /// Set when audio is queued into a silent player; the callback records
    /// when it first rendered it.
    start_requested: Mutex<Option<Instant>>,
    last_start_us: AtomicU64,
    /// The device's sample rate.
    rate: u32,
}

/// Keeps the speaker stream open; playback goes through [`Shared`], which any
/// thread may hold.
pub struct Player {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
}

// cpal streams are not Send on every platform; the player is created and
// dropped on the call's audio thread only.
unsafe impl Send for Player {}

impl Player {
    pub fn start(device: Option<&str>) -> Result<Self> {
        let host = cpal::default_host();
        let device = device
            .and_then(|id| id.parse::<cpal::DeviceId>().ok())
            .and_then(|id| host.device_by_id(&id))
            .or_else(|| host.default_output_device())
            .context("No speaker or headphones available")?;
        let config = device.default_output_config()?;
        let rate = config.sample_rate();
        let channels = config.channels() as usize;
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            played: Mutex::new(VecDeque::with_capacity(crate::RATE as usize)),
            playing: AtomicBool::new(false),
            level: AtomicU32::new(0),
            stop_requested: Mutex::new(None),
            last_stop_us: AtomicU64::new(0),
            start_requested: Mutex::new(None),
            last_start_us: AtomicU64::new(0),
            rate,
        });
        let ratio = f64::from(crate::RATE) / f64::from(rate);
        let s = shared.clone();
        let err = |e| tracing::warn!("speaker stream: {e}");
        macro_rules! stream {
            ($t:ty, $conv:expr) => {{
                let s = s.clone();
                let mut mono: Vec<f32> = Vec::with_capacity(8192);
                device.build_output_stream(
                    &config.clone().into(),
                    move |data: &mut [$t], _| {
                        let frames = data.len() / channels.max(1);
                        mono.resize(frames, 0.0);
                        fill(&s, &mut mono, ratio);
                        for (frame, v) in data.chunks_mut(channels.max(1)).zip(&mono) {
                            for x in frame.iter_mut() {
                                *x = $conv(*v);
                            }
                        }
                    },
                    err,
                    None,
                )?
            }};
        }
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => stream!(f32, |v: f32| v),
            cpal::SampleFormat::I16 => stream!(i16, |v: f32| (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16),
            cpal::SampleFormat::U16 => {
                stream!(u16, |v: f32| ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16)
            }
            f => bail!("Unsupported speaker format {f:?}"),
        };
        stream.play()?;
        Ok(Self { _stream: stream, shared })
    }

    pub fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }

    pub fn play(&self, pcm: &Pcm) {
        self.shared.play(pcm);
    }

    pub fn stop(&self) -> u64 {
        self.shared.stop()
    }

    pub fn earcon(&self) {
        self.shared.earcon();
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing()
    }

    pub fn rate(&self) -> u32 {
        self.shared.rate
    }
}

impl Shared {
    /// Queue a clause behind whatever is playing.
    pub fn play(&self, pcm: &Pcm) {
        if pcm.samples.is_empty() {
            return;
        }
        let at_device = crate::resample::convert(&pcm.samples, pcm.rate, self.rate);
        let at_16k = crate::resample::convert(&pcm.samples, pcm.rate, crate::RATE);
        let mut q = self.queue.lock();
        if q.out.is_empty() {
            *self.start_requested.lock() = Some(Instant::now());
        }
        q.out.extend(at_device);
        q.reference.extend(at_16k);
        self.playing.store(true, Ordering::Release);
    }

    /// A short, quiet two-note cue (not words) for "I heard you".
    pub fn earcon(&self) {
        let r = crate::RATE as f32;
        let tone = |f: f32, ms: f32| -> Vec<f32> {
            let n = (r * ms / 1000.0) as usize;
            (0..n)
                .map(|i| 0.08 * (i as f32 * f * std::f32::consts::TAU / r).sin() * (1.0 - i as f32 / n as f32))
                .collect()
        };
        let mut s = tone(660.0, 60.0);
        s.extend(tone(880.0, 70.0));
        self.play(&Pcm { samples: s, rate: crate::RATE });
    }

    /// Silence now. Returns how much audio was dropped, in ms.
    pub fn stop(&self) -> u64 {
        let mut q = self.queue.lock();
        let dropped = q.out.len() as u64 * 1000 / u64::from(self.rate.max(1));
        let was = !q.out.is_empty();
        q.out.clear();
        q.reference.clear();
        q.owed = 0.0;
        drop(q);
        if was {
            *self.stop_requested.lock() = Some(Instant::now());
        }
        self.playing.store(false, Ordering::Release);
        dropped
    }

    /// Take `n` samples of played reference audio (zeros when Dove was silent).
    pub fn take_reference(&self, n: usize) -> Vec<f32> {
        let mut p = self.played.lock();
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(p.pop_front().unwrap_or(0.0));
        }
        // Never let the reference run far ahead of the microphone.
        let cap = crate::RATE as usize / 2;
        if p.len() > cap {
            let extra = p.len() - cap;
            p.drain(..extra);
        }
        out
    }

    pub fn playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }

    pub fn take_level(&self) -> f32 {
        f32::from_bits(self.level.swap(0, Ordering::Relaxed))
    }

    /// Microseconds between the last `stop` and the first silent buffer.
    pub fn last_stop_latency_us(&self) -> u64 {
        self.last_stop_us.load(Ordering::Relaxed)
    }

    /// Microseconds between queueing audio into a silent player and the
    /// device callback rendering it.
    pub fn last_start_latency_us(&self) -> u64 {
        self.last_start_us.load(Ordering::Relaxed)
    }
}

/// The device callback: drain the queue into `mono`, release the matching
/// reference audio, note when a stop reached the speaker.
fn fill(s: &Shared, mono: &mut [f32], ratio: f64) {
    let Some(mut q) = s.queue.try_lock() else {
        mono.iter_mut().for_each(|v| *v = 0.0);
        return;
    };
    let mut n = 0;
    for v in mono.iter_mut() {
        match q.out.pop_front() {
            Some(x) => {
                *v = x;
                n += 1;
            }
            None => *v = 0.0,
        }
    }
    q.owed += n as f64 * ratio;
    let take = q.owed.floor() as usize;
    q.owed -= take as f64;
    let refs: Vec<f32> = (0..take).filter_map(|_| q.reference.pop_front()).collect();
    let empty = q.out.is_empty();
    drop(q);
    if let Some(mut p) = s.played.try_lock() {
        p.extend(refs);
    }
    if empty {
        s.playing.store(false, Ordering::Release);
    }
    let rms = (mono.iter().map(|v| v * v).sum::<f32>() / mono.len().max(1) as f32).sqrt();
    s.level.fetch_max(rms.to_bits(), Ordering::Relaxed);
    if n > 0
        && let Some(mut req) = s.start_requested.try_lock()
        && let Some(at) = req.take()
    {
        s.last_start_us.store(at.elapsed().as_micros() as u64, Ordering::Relaxed);
    }
    if n == 0
        && let Some(mut req) = s.stop_requested.try_lock()
        && let Some(at) = req.take()
    {
        s.last_stop_us.store(at.elapsed().as_micros() as u64, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Shared {
        Shared {
            queue: Mutex::new(Queue::default()),
            played: Mutex::new(VecDeque::new()),
            playing: AtomicBool::new(true),
            level: AtomicU32::new(0),
            stop_requested: Mutex::new(None),
            last_stop_us: AtomicU64::new(0),
            start_requested: Mutex::new(None),
            last_start_us: AtomicU64::new(0),
            rate: 48_000,
        }
    }

    #[test]
    fn reference_follows_what_was_rendered() {
        let s = shared();
        {
            let mut q = s.queue.lock();
            q.out.extend(vec![0.5; 4800]); // 100 ms at 48 kHz
            q.reference.extend(vec![0.5; 1600]); // the same 100 ms at 16 kHz
        }
        let mut buf = vec![0.0; 480]; // 10 ms
        fill(&s, &mut buf, 16_000.0 / 48_000.0);
        assert!(buf.iter().all(|v| *v == 0.5));
        assert_eq!(s.take_reference(160), vec![0.5; 160]);
        // Nothing more was released than was played.
        assert_eq!(s.take_reference(10), vec![0.0; 10]);
    }

    #[test]
    fn a_stop_is_measured_when_silence_reaches_the_device() {
        let s = shared();
        s.queue.lock().out.extend(vec![0.3; 100]);
        *s.stop_requested.lock() = Some(Instant::now());
        s.queue.lock().out.clear();
        let mut buf = vec![1.0; 64];
        fill(&s, &mut buf, 1.0);
        assert!(buf.iter().all(|v| *v == 0.0));
        assert!(s.stop_requested.lock().is_none());
        assert!(!s.playing());
    }
}
