//! Per-turn timings, measured where they happen and reported as-is.

use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TurnTimings {
    /// Speech from first voiced frame to last.
    pub speech_ms: u32,
    /// Pause after the last voiced frame when the turn detector said "done".
    pub endpoint_silence_ms: u32,
    /// CPU time the turn detector spent deciding (all calls this turn).
    pub turn_decide_ms: f32,
    /// Final transcription time, from turn end to text.
    pub stt_final_ms: f32,
    /// From the last voiced frame to the final transcript being ready.
    pub end_of_speech_to_text_ms: f32,
}

/// Simple percentile over measured samples (nearest rank).
pub fn percentile(samples: &[f32], p: f32) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    let mut v = samples.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let rank = ((p / 100.0) * v.len() as f32).ceil().max(1.0) as usize - 1;
    v.get(rank.min(v.len() - 1)).copied()
}

/// This process's resident memory, in bytes, where the platform tells us.
pub fn resident_bytes() -> Option<u64> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) != 0 {
            return Some(c.WorkingSetSize as u64);
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        let s = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
        Some(pages * 4096)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

/// This process's total CPU time (user + kernel), where available.
pub fn cpu_time() -> Option<std::time::Duration> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let mut a: FILETIME = std::mem::zeroed();
        let mut b: FILETIME = std::mem::zeroed();
        let mut k: FILETIME = std::mem::zeroed();
        let mut u: FILETIME = std::mem::zeroed();
        if GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut k, &mut u) == 0 {
            return None;
        }
        let t = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
        Some(std::time::Duration::from_nanos((t(k) + t(u)) * 100))
    }
    #[cfg(target_os = "linux")]
    {
        let s = std::fs::read_to_string("/proc/self/stat").ok()?;
        let after = s.rsplit_once(')')?.1;
        let f: Vec<&str> = after.split_whitespace().collect();
        let ticks: u64 = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
        Some(std::time::Duration::from_millis(ticks * 10))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_nearest_rank() {
        let s = [5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(percentile(&s, 50.0), Some(3.0));
        assert_eq!(percentile(&s, 95.0), Some(5.0));
        assert_eq!(percentile(&[], 50.0), None);
    }

    #[test]
    fn process_counters_are_readable_here() {
        if cfg!(any(windows, target_os = "linux")) {
            assert!(resident_bytes().unwrap() > 1_000_000);
            assert!(cpu_time().is_some());
        }
    }
}
