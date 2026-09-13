//! Helpers shared by the unit tests: synthetic DJI-style WAV files.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn tempdir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let d = std::env::temp_dir().join(format!("prepareaudio-test-{tag}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

pub fn fmt_float(sr: u32) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&3u16.to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&sr.to_le_bytes());
    f.extend_from_slice(&(sr * 4).to_le_bytes());
    f.extend_from_slice(&4u16.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f
}

/// Writes a plain 44-byte-header float WAV exactly like the DJI Mic 2 does.
pub fn write_float_wav(path: &Path, sr: u32, samples: &[f32]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut f = File::create(path).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&((4 + 8 + 16 + 8 + data.len()) as u32).to_le_bytes()).unwrap();
    f.write_all(b"WAVE").unwrap();
    f.write_all(b"fmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&fmt_float(sr)).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&(data.len() as u32).to_le_bytes()).unwrap();
    f.write_all(&data).unwrap();
}

/// Continuous sine: frames `start..start+count` of one endless signal.
pub fn sine(freq: f64, amp: f64, sr: u32, start: u64, count: u64) -> Vec<f32> {
    (start..start + count)
        .map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / sr as f64).sin()) as f32)
        .collect()
}
