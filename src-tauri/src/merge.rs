//! Writes grouped recordings into the output folder: one gapless WAV per
//! recording, audio bytes copied bit-exactly, never overwriting anything.

use crate::scan::Recording;
use crate::wav;
use serde::Serialize;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const COPY_BUFFER: usize = 4 << 20;
const SPACE_MARGIN: u64 = 64 << 20;
const COMPARE_BLOCK: u64 = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub index: usize,
    pub count: usize,
    pub id: usize,
    pub name: String,
    pub done: u64,
    pub total: u64,
    pub milestone: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Written,
    Existing,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub id: usize,
    pub status: Status,
    pub path: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub out_dir: String,
    pub outcomes: Vec<Outcome>,
    pub cancelled: bool,
}

enum Target {
    New(PathBuf),
    Existing(PathBuf),
}

enum WriteError {
    Cancelled,
    Io(String),
}

impl From<io::Error> for WriteError {
    fn from(e: io::Error) -> Self {
        WriteError::Io(e.to_string())
    }
}

pub fn run<F: FnMut(&Progress)>(
    recs: &[&Recording],
    out_dir: &Path,
    cancel: &AtomicBool,
    mut progress: F,
) -> Result<Summary, String> {
    fs::create_dir_all(out_dir)
        .map_err(|e| format!("Zielordner {} kann nicht angelegt werden: {e}", out_dir.display()))?;

    let mut outcomes = Vec::new();
    let mut todo: Vec<(&Recording, PathBuf)> = Vec::new();
    let mut reserved = HashSet::new();
    for &rec in recs {
        match resolve_target(out_dir, rec, &mut reserved) {
            Ok(Target::Existing(p)) => outcomes.push(Outcome {
                id: rec.id,
                status: Status::Existing,
                path: Some(p.display().to_string()),
                message: None,
            }),
            Ok(Target::New(p)) => todo.push((rec, p)),
            Err(e) => outcomes.push(Outcome { id: rec.id, status: Status::Failed, path: None, message: Some(e.to_string()) }),
        }
    }

    let need: u64 = todo.iter().map(|(r, _)| r.output_bytes).sum();
    if let Some(free) = available_bytes(out_dir) {
        if need > 0 && free < need + SPACE_MARGIN {
            return Err(format!(
                "Zu wenig Speicherplatz im Zielordner: benötigt {}, frei {}.",
                human_bytes(need),
                human_bytes(free)
            ));
        }
    }

    let total: u64 = todo.iter().map(|(r, _)| r.data_bytes).sum();
    let count = todo.len();
    let mut done = 0u64;
    let mut cancelled = false;
    for (index, (rec, target)) in todo.iter().enumerate() {
        if cancelled || cancel.load(Ordering::SeqCst) {
            cancelled = true;
            outcomes.push(Outcome { id: rec.id, status: Status::Cancelled, path: None, message: None });
            continue;
        }
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let base = done;
        let result = {
            let mut report = |rec_done: u64, milestone: bool| {
                progress(&Progress { index, count, id: rec.id, name: name.clone(), done: base + rec_done, total, milestone })
            };
            report(0, true);
            let r = write_recording(rec, target, cancel, &mut report);
            if r.is_ok() {
                report(rec.data_bytes, true);
            }
            r
        };
        done += rec.data_bytes;
        outcomes.push(match result {
            Ok(()) => Outcome { id: rec.id, status: Status::Written, path: Some(target.display().to_string()), message: None },
            Err(WriteError::Cancelled) => {
                cancelled = true;
                Outcome { id: rec.id, status: Status::Cancelled, path: None, message: None }
            }
            Err(WriteError::Io(msg)) => Outcome { id: rec.id, status: Status::Failed, path: None, message: Some(msg) },
        });
    }

    outcomes.sort_by_key(|o| o.id);
    Ok(Summary { out_dir: out_dir.display().to_string(), outcomes, cancelled })
}

fn resolve_target(out_dir: &Path, rec: &Recording, reserved: &mut HashSet<PathBuf>) -> io::Result<Target> {
    let stem = rec.out_name.trim_end_matches(".wav");
    for n in 1..1000 {
        let name = if n == 1 { format!("{stem}.wav") } else { format!("{stem}_{n}.wav") };
        let path = out_dir.join(name);
        if reserved.contains(&path) {
            continue;
        }
        if !path.exists() {
            reserved.insert(path.clone());
            return Ok(Target::New(path));
        }
        if is_same_output(&path, rec) {
            reserved.insert(path.clone());
            return Ok(Target::Existing(path));
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "kein freier Dateiname im Zielordner"))
}

/// An earlier run already wrote this recording (same format, length, and the
/// same audio at the very start and the very end).
fn is_same_output(path: &Path, rec: &Recording) -> bool {
    let Ok(info) = wav::read_info(path) else { return false };
    if info.repaired || info.data_len != rec.data_bytes || info.fmt != rec.fmt() {
        return false;
    }
    let first = &rec.parts[0];
    let last = rec.parts.last().unwrap();
    let head = COMPARE_BLOCK.min(first.info.data_len);
    let tail = COMPARE_BLOCK.min(last.info.data_len);
    let same = |a: io::Result<Vec<u8>>, b: io::Result<Vec<u8>>| matches!((a, b), (Ok(a), Ok(b)) if a == b);
    same(
        wav::read_at(path, info.data_offset, head),
        wav::read_at(&first.path_buf, first.info.data_offset, head),
    ) && same(
        wav::read_at(path, info.data_offset + info.data_len - tail, tail),
        wav::read_at(&last.path_buf, last.info.data_offset + last.info.data_len - tail, tail),
    )
}

fn write_recording(
    rec: &Recording,
    target: &Path,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(u64, bool),
) -> Result<(), WriteError> {
    let file_name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let partial = target.with_file_name(format!("{file_name}.part"));
    let result = write_into(rec, &partial, cancel, report).and_then(|()| {
        if target.exists() {
            return Err(WriteError::Io(format!("{} existiert inzwischen bereits", target.display())));
        }
        fs::rename(&partial, target).map_err(WriteError::from)
    });
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

fn write_into(
    rec: &Recording,
    path: &Path,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(u64, bool),
) -> Result<(), WriteError> {
    let fmt = rec.fmt();
    let mut out = File::create(path)?;
    wav::write_header(&mut out, fmt, rec.data_bytes, rec.frames, wav::RIFF_LIMIT)?;
    let mut buf = vec![0u8; COPY_BUFFER];
    let mut rec_done = 0u64;
    for (k, part) in rec.parts.iter().enumerate() {
        let mut src = File::open(&part.path_buf)?;
        if src.metadata()?.len() != part.info.file_size {
            return Err(WriteError::Io(format!("Teil {} wurde seit dem Scan verändert: {}", k + 1, part.path)));
        }
        src.seek(SeekFrom::Start(part.info.data_offset))?;
        let mut remaining = part.info.data_len;
        while remaining > 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err(WriteError::Cancelled);
            }
            let n = remaining.min(COPY_BUFFER as u64) as usize;
            src.read_exact(&mut buf[..n])?;
            out.write_all(&buf[..n])?;
            remaining -= n as u64;
            rec_done += n as u64;
            report(rec_done, false);
        }
    }
    if rec.data_bytes % 2 == 1 {
        out.write_all(&[0])?;
    }
    out.sync_all()?;
    drop(out);

    let check = wav::read_info(path)?;
    if check.repaired || check.data_len != rec.data_bytes || check.fmt != fmt || check.file_size != rec.output_bytes {
        return Err(WriteError::Io("Kontrolle der geschriebenen Datei fehlgeschlagen".into()));
    }
    Ok(())
}

#[cfg(unix)]
pub fn available_bytes(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(not(unix))]
pub fn available_bytes(_path: &Path) -> Option<u64> {
    None
}

pub fn human_bytes(b: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i >= 2 {
        format!("{v:.1} {}", units[i])
    } else {
        format!("{v:.0} {}", units[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::scan;
    use crate::scan::tests::small;
    use crate::testutil::*;

    fn float_bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn merges_bit_exact_idempotent_and_never_overwrites() {
        let root = tempdir("merge");
        let sr = 8000;
        let parts = [sine(220.0, 0.5, sr, 0, 80_000), sine(220.0, 0.5, sr, 80_000, 80_000), sine(220.0, 0.5, sr, 160_000, 12_345)];
        write_float_wav(&root.join("SD/x/DJI_03_20260101_235950.WAV"), sr, &parts[0]);
        write_float_wav(&root.join("Kopie/DJI_03_20260102_000000.WAV"), sr, &parts[1]);
        write_float_wav(&root.join("DJI_03_20260102_000010.WAV"), sr, &parts[2]);

        let s = scan(&[root.clone()], &small()).unwrap();
        assert_eq!(s.recordings.len(), 1);
        let rec = &s.recordings[0];
        assert_eq!(rec.parts.len(), 3);
        assert_eq!(rec.date, "01.01.2026");
        assert_eq!(rec.out_name, "260101_S235950-E000012_D000022_x.wav");

        let out = std::path::PathBuf::from(&s.default_out_dir);
        let cancel = AtomicBool::new(false);
        let mut events = 0;
        let sum = run(&[rec], &out, &cancel, |_| events += 1).unwrap();
        assert!(events >= 2);
        assert_eq!(sum.outcomes[0].status, Status::Written);
        let written = std::path::PathBuf::from(sum.outcomes[0].path.as_ref().unwrap());
        assert_eq!(written.parent().unwrap(), root.join("tracks"));

        let info = wav::read_info(&written).unwrap();
        let expected: Vec<u8> = parts.iter().flat_map(|p| float_bytes(p)).collect();
        assert_eq!(wav::read_at(&written, info.data_offset, info.data_len).unwrap(), expected);
        assert_eq!(info.fmt, fmt_float(sr));
        assert!(fs::read_dir(&out).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".part")));

        // Second run: recognised as already present, nothing rewritten.
        let sum2 = run(&[rec], &out, &cancel, |_| {}).unwrap();
        assert_eq!(sum2.outcomes[0].status, Status::Existing);

        // A foreign file with the same name is kept; output goes to _2.
        fs::remove_file(&written).unwrap();
        fs::write(&written, b"not ours").unwrap();
        let sum3 = run(&[rec], &out, &cancel, |_| {}).unwrap();
        assert_eq!(sum3.outcomes[0].status, Status::Written);
        assert!(sum3.outcomes[0].path.as_ref().unwrap().ends_with("_x_2.wav"));
        assert_eq!(fs::read(&written).unwrap(), b"not ours");
    }

    #[test]
    fn cancel_leaves_no_partial_file() {
        let root = tempdir("cancel");
        let sr = 8000;
        write_float_wav(&root.join("DJI_01_20260101_100000.WAV"), sr, &sine(220.0, 0.5, sr, 0, 80_000));
        let s = scan(&[root.clone()], &small()).unwrap();
        let cancel = AtomicBool::new(true);
        let out = root.join("tracks");
        let sum = run(&[&s.recordings[0]], &out, &cancel, |_| {}).unwrap();
        assert!(sum.cancelled);
        assert_eq!(sum.outcomes[0].status, Status::Cancelled);
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
    }
}
