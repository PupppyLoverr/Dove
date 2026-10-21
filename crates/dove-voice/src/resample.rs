//! Streaming conversion between a device's native rate and 16 kHz.

use rubato::{FftFixedIn, Resampler};

/// Converts arbitrary-length chunks from `from` Hz to `to` Hz.
pub struct Stream {
    inner: Option<FftFixedIn<f32>>,
    pending: Vec<f32>,
    chunk: usize,
}

impl Stream {
    pub fn new(from: u32, to: u32) -> anyhow::Result<Self> {
        if from == to {
            return Ok(Self { inner: None, pending: Vec::new(), chunk: 0 });
        }
        let chunk = (from as usize / 100).max(64); // 10 ms
        let inner = FftFixedIn::<f32>::new(from as usize, to as usize, chunk, 2, 1)?;
        Ok(Self { inner: Some(inner), pending: Vec::with_capacity(chunk * 2), chunk })
    }

    /// Push input; returns whatever output is ready.
    pub fn push(&mut self, input: &[f32]) -> Vec<f32> {
        let Some(r) = self.inner.as_mut() else { return input.to_vec() };
        self.pending.extend_from_slice(input);
        let mut out = Vec::new();
        while self.pending.len() >= self.chunk {
            let block: Vec<f32> = self.pending.drain(..self.chunk).collect();
            if let Ok(mut o) = r.process(&[block], None) {
                out.append(&mut o[0]);
            }
        }
        out
    }
}

/// One-shot conversion of a whole buffer.
pub fn convert(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let Ok(mut s) = Stream::new(from, to) else { return Vec::new() };
    let delay = s.inner.as_ref().map_or(0, |r| r.output_delay());
    let mut out = s.push(samples);
    // Flush the tail with silence so the last words are not cut.
    out.extend(s.push(&vec![0.0; s.chunk * 3]));
    out.drain(..delay.min(out.len()));
    let want = (samples.len() as u64 * to as u64 / from as u64) as usize;
    out.truncate(want.max(1));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_convert_with_the_right_length_and_pitch() {
        let tone: Vec<f32> = (0..48_000).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin()).collect();
        let out = convert(&tone, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        // Count zero crossings in the steady middle: ~880 per second.
        let mid = &out[2000..14_000];
        let crossings = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((430..450).contains(&(crossings * 16_000 / 12_000)), "{crossings}");
        assert_eq!(convert(&tone[..100], 16_000, 16_000).len(), 100);
    }
}
