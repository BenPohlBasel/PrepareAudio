//! Helpers shared by the unit tests: synthetic recorder WAV files.

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

/// Writes a mono float WAV with extra raw chunks before `fmt ` and after `data`.
pub fn write_wav_with(path: &Path, sr: u32, samples: &[f32], before: &[u8], after: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut f = File::create(path).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&((4 + before.len() + 8 + 16 + 8 + data.len() + after.len()) as u32).to_le_bytes()).unwrap();
    f.write_all(b"WAVE").unwrap();
    f.write_all(before).unwrap();
    f.write_all(b"fmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&fmt_float(sr)).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&(data.len() as u32).to_le_bytes()).unwrap();
    f.write_all(&data).unwrap();
    f.write_all(after).unwrap();
}

/// Writes a plain 44-byte-header float WAV exactly like the DJI Mic 2 does.
pub fn write_float_wav(path: &Path, sr: u32, samples: &[f32]) {
    write_wav_with(path, sr, samples, &[], &[]);
}

/// A Broadcast WAV `bext` chunk (602-byte body) with date "yyyy-mm-dd", time "hh:mm:ss" and TimeReference.
pub fn bext_chunk(date: &str, time: &str, time_reference: u64) -> Vec<u8> {
    let mut body = vec![0u8; 602];
    body[320..330].copy_from_slice(&date.as_bytes()[..10]);
    body[330..338].copy_from_slice(&time.as_bytes()[..8]);
    body[338..346].copy_from_slice(&time_reference.to_le_bytes());
    let mut c = b"bext".to_vec();
    c.extend_from_slice(&(body.len() as u32).to_le_bytes());
    c.extend_from_slice(&body);
    c
}

/// An `iXML` chunk with a `<FILE_SET>` (padded to even length).
pub fn ixml_chunk(family_uid: &str, index: &str, total: u32) -> Vec<u8> {
    let mut body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><BWFXML><IXML_VERSION>2.10</IXML_VERSION><FILE_SET><TOTAL_FILES>{total}</TOTAL_FILES><FAMILY_UID>{family_uid}</FAMILY_UID><FILE_SET_INDEX>{index}</FILE_SET_INDEX></FILE_SET></BWFXML>"
    )
    .into_bytes();
    if body.len() % 2 == 1 {
        body.push(0);
    }
    let mut c = b"iXML".to_vec();
    c.extend_from_slice(&(body.len() as u32).to_le_bytes());
    c.extend_from_slice(&body);
    c
}

/// A float Broadcast WAV with its bext chunk before `fmt `, like field recorders write it.
pub fn write_float_bwf(path: &Path, sr: u32, samples: &[f32], date: &str, time: &str, time_reference: u64) {
    write_wav_with(path, sr, samples, &bext_chunk(date, time, time_reference), &[]);
}

/// Sets a file's modification time (seconds since 1970, UTC).
pub fn set_mtime(path: &Path, secs: f64) {
    let f = File::options().write(true).open(path).unwrap();
    f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(secs)).unwrap();
}

/// Continuous sine: frames `start..start+count` of one endless signal.
pub fn sine(freq: f64, amp: f64, sr: u32, start: u64, count: u64) -> Vec<f32> {
    (start..start + count)
        .map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / sr as f64).sin()) as f32)
        .collect()
}
