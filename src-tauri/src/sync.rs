//! Zweite Funktion „Synchronisieren“: Welche Tracks zweier Sender haben dasselbe
//! Geschehen aufgenommen, und in welchen Abschnitten?
//!
//! Gesucht wird nicht Klangähnlichkeit, sondern gemeinsame Ereignisse mit
//! konstantem Zeitversatz. Zwei Ansteckmikros im selben Geschehen hören dieselben
//! Einsätze (Silben, Stuhlrücken, Geschirr), nur verschoben und anders gewichtet;
//! die lauteste Quelle ist dabei oft gegenläufig (links spricht A, rechts B).
//!
//! 1. Einsatzstärke je Millisekunde: Pegelanstieg in dB in vier Frequenzbändern
//!    (150–400, 400–1000, 1000–2500, 2500–7000 Hz), je Band robust normiert und
//!    begrenzt, dann summiert. Gezählt wird der Anstieg, nicht die Lautstärke,
//!    so bestimmt die dominante Stimme das Ergebnis nicht.
//! 2. Globaler Versatz: Kreuzkorrelation der Einsatzstärke über die ganzen Tracks
//!    (10 ms, ±300 s um den Versatz laut Dateinamen), dann in 60-s-Fenstern
//!    (1 ms, ±1 s) mit Peak-Prominenz; eine robuste Gerade liefert Versatz und
//!    Uhrendrift der beiden Sender.
//! 3. Je 20-s-Fenster (alle 10 s) wird die Laufzeit neu gesucht (±300 ms). Ein
//!    Treffer ist ein scharfer Peak, der höchstens ±20 ms (≈ 7 m Schallweg) neben
//!    der Geraden liegt. Getrennte Geschehen geben flache, zerfranste Kurven mit
//!    wanderndem Maximum. Gegenprobe: Kohärenz (Welch, 150–1200 Hz) nach
//!    Ausgleich des Versatzes.
//! 4. Der Trefferanteil, über 50 s geglättet, ergibt Phasen gemeinsam/getrennt
//!    (gemeinsam ≥ 180 s, getrennt ≥ 120 s, Schnitt in der leisesten Sekunde).
//!
//! Gemeinsame Phasen werden Stereo (links der kleinere Sendername), alles andere
//! bleibt je Sender Mono.

use crate::decode;
use crate::i18n::{self, t, tf, Msg};
use crate::merge::{available_bytes, low_space, Outcome, Progress, Status, Summary};
use crate::scan::{self, civil_from_secs, days_from_civil, Skipped};
use crate::wav::{self, WavInfo};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering as CmpOrdering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const OUTPUT_DIR_NAME: &str = "sync";

const ENV_RATE: f64 = 1000.0;
const BANDS: [(f64, f64); 4] = [(150.0, 400.0), (400.0, 1000.0), (1000.0, 2500.0), (2500.0, 7000.0)];
const ONSET_SPAN: usize = 5;
const ONSET_CLIP: f32 = 20.0;

const COARSE_POOL: usize = 10;
const SEARCH_S: f64 = 300.0;
const COARSE_Z_MIN: f64 = 6.0;
const FINE_WINDOW_MS: usize = 60_000;
const FINE_STEP_S: f64 = 60.0;
const FINE_SEARCH_MS: isize = 1000;
const FINE_Z_MIN: f64 = 6.0;
const FIT_RESID_MAX_S: f64 = 0.03;
const MIN_TRACK_S: f64 = 60.0;
const MIN_OVERLAP_S: f64 = 60.0;

const FRAME_MS: usize = 20_000;
const HOP_S: f64 = 10.0;
const FRAME_SEARCH_MS: isize = 300;
const PEAK_EXCLUDE_MS: isize = 10;
const LAG_TOL_MS: i32 = 20;
const FRAME_Z_MIN: f64 = 6.0;
const SILENCE_DB: f32 = 20.0;
const SMOOTH: usize = 5;
const HIT_TOGETHER: f64 = 0.6;
const HIT_APART: f64 = 0.2;
const DEMOTE_SHARE: f64 = 0.4;
/// Counter-check: independent recordings stay below ~0.005 mean coherence,
/// shared situations measured 0.11–0.32 on real DJI Mic 2 pairs.
const DEMOTE_MSC: f64 = 0.02;
const MIN_TOGETHER_S: f64 = 180.0;
const MIN_APART_S: f64 = 120.0;
const SNAP_WINDOW_S: f64 = 30.0;
const EDGE_ABSORB_S: f64 = 60.0;
const MIN_PIECE_S: f64 = 1.0;

const COH_RATE: f64 = 3000.0;
const COH_SEG: usize = 256;
const COH_LOW: f64 = 150.0;
const COH_HIGH: f64 = 1200.0;

// ------------------------------------------------------------------ data

#[derive(Debug, Clone)]
pub struct Segment {
    pub path: PathBuf,
    pub offset: u64,
    pub len: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub id: usize,
    pub name: String,
    pub path: String,
    pub label: String,
    pub day: String,
    pub start: String,
    pub end: String,
    /// Seconds since local midnight at the start (for the timeline).
    pub clock0: f64,
    pub duration: f64,
    pub parts: usize,
    #[serde(skip)]
    pub start_secs: i64,
    #[serde(skip)]
    pub info: WavInfo,
    #[serde(skip)]
    pub segments: Vec<Segment>,
    #[serde(skip)]
    pub frames: u64,
    /// Container of a non-WAV source ("MP3", "M4A", …); its audio is read from `decoded`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded_from: Option<String>,
    /// The cached 32-bit float WAV decoded from `path` (non-WAV sources only). `path` stays the
    /// original file for names, labels and the edit file; all audio is read through `segments`.
    #[serde(skip)]
    pub decoded: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Frame {
    /// Start of the 20-s window in track A's time.
    pub t: f64,
    /// Deviation of the event peak from the fitted offset.
    pub lag_ms: i32,
    /// Peak prominence (robust z-score of the correlation peak).
    pub z: f32,
    pub msc: Option<f32>,
    pub level_a: f32,
    pub level_b: f32,
    pub hit: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Phase {
    pub kind: &'static str,
    pub start: f64,
    pub end: f64,
    pub hit_share: Option<f64>,
    pub msc: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pair {
    pub a: usize,
    pub b: usize,
    pub ok: bool,
    /// Position in A's time where B's sample 0 sits (at A time 0).
    pub offset: f64,
    pub drift_ppm: f64,
    pub resid_ms: f64,
    pub coarse_z: f64,
    pub n_good: usize,
    pub n_windows: usize,
    pub note: Option<String>,
    pub frames: Vec<Frame>,
    pub phases: Vec<Phase>,
    #[serde(skip)]
    pub drift: f64,
}

impl Pair {
    pub fn to_b(&self, ta: f64) -> f64 {
        ta - (self.offset + self.drift * ta)
    }
    pub fn to_a(&self, tb: f64) -> f64 {
        (tb + self.offset) / (1.0 - self.drift)
    }
}

/// Time in which one recorder of a stereo file did not record (its channel is silent).
#[derive(Debug, Clone, Serialize)]
pub struct Silence {
    pub label: String,
    pub seconds: f64,
}

/// Stretch of an item where the second channel carries audio, on the reference track's clock.
#[derive(Debug, Clone, Serialize)]
pub struct Span {
    pub pair: usize,
    pub other: usize,
    pub t0: f64,
    pub t1: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Draft {
    pub id: usize,
    pub kind: &'static str,
    pub name: String,
    pub day: String,
    pub start: String,
    pub end: String,
    pub clock0: f64,
    pub duration: f64,
    pub left: usize,
    pub right: Option<usize>,
    pub pair: Option<usize>,
    /// Track whose samples run through unchanged; `t0`/`t1` are on its clock.
    pub reference: usize,
    pub t0: f64,
    pub t1: f64,
    /// Where the other channel has audio (stereo only).
    pub spans: Vec<Span>,
    /// Seconds in which one recorder was switched off inside this stereo file.
    pub dropout: f64,
    /// The same, per silent recorder.
    pub silent: Vec<Silence>,
    pub hit_share: Option<f64>,
    pub msc: Option<f64>,
    pub bytes: u64,
    /// Why this stretch is what it is: "gemeinsam", "getrennt" or "allein".
    pub reason: &'static str,
    #[serde(skip)]
    pub start_secs: i64,
}

/// Placement of a track on the day's timeline: timeline second `t` (absolute,
/// on the anchoring recorder's clock) is second `(t − p) · s` of the track.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Place {
    pub p: f64,
    pub s: f64,
}

impl Place {
    pub fn to_track(&self, t: f64) -> f64 {
        (t - self.p) * self.s
    }
    pub fn to_timeline(&self, tau: f64) -> f64 {
        self.p + tau / self.s
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClipMode {
    Stereo,
    Mono,
}

/// A stretch of one track (seconds in the track) and where it goes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Clip {
    pub track: usize,
    pub t0: f64,
    pub t1: f64,
    pub mode: ClipMode,
    #[serde(default)]
    pub deleted: bool,
}

/// Part of an output channel: a track between two timeline seconds.
#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub track: usize,
    pub t0: f64,
    pub t1: f64,
}

/// One output file.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: usize,
    pub kind: &'static str,
    pub name: String,
    pub day: String,
    pub start: String,
    pub end: String,
    /// Seconds since midnight on the timeline where the file starts.
    pub clock0: f64,
    pub duration: f64,
    pub left: usize,
    pub right: Option<usize>,
    /// Stereo: absolute timeline seconds. Mono: seconds in the track `left`.
    pub t0: f64,
    pub t1: f64,
    pub left_sources: Vec<Source>,
    pub right_sources: Vec<Source>,
    /// Seconds in which a channel of this stereo file has no recording.
    pub dropout: f64,
    pub silent: Vec<Silence>,
    pub hit_share: Option<f64>,
    pub msc: Option<f64>,
    pub bytes: u64,
    /// "gemeinsam", "getrennt" or "allein".
    pub reason: &'static str,
    #[serde(skip)]
    pub start_secs: i64,
    #[serde(skip)]
    pub rate: u32,
}

/// Waveform peaks of a track: level k holds the maximum of 10^k milliseconds,
/// coded 0–255 for −60…0 dBFS.
#[derive(Debug, Default)]
pub struct Peaks {
    pub levels: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncPlan {
    pub roots: Vec<String>,
    pub default_out_dir: String,
    /// "tracks" (files from the first function), "chunks" (raw DJI parts) or "mixed".
    pub source: &'static str,
    pub labels: Vec<String>,
    pub tracks: Vec<Track>,
    pub pairs: Vec<Pair>,
    pub places: Vec<Place>,
    /// Current edit state (analysis proposal or the user's edits).
    pub clips: Vec<Clip>,
    /// The clips differ from the analysis proposal (and are saved next to the sources).
    pub edited: bool,
    pub items: Vec<Item>,
    pub ignored: Vec<Skipped>,
    /// Gain that brings each track to a comfortable listening level for previews.
    pub preview_gain_db: Vec<f32>,
    #[serde(skip)]
    pub analysis_clips: Vec<Clip>,
    #[serde(skip)]
    pub peaks: Vec<Arc<Peaks>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncProgress {
    pub stage: &'static str,
    pub done: u64,
    pub total: u64,
    pub text: String,
}

// ------------------------------------------------------------------ loading

struct Clock {
    day: String,
    hms: String,
    date6: String,
    time6: String,
    of_day: f64,
}

fn clock(secs: i64) -> Clock {
    let (y, mo, d, h, mi, s) = civil_from_secs(secs);
    Clock {
        day: format!("{d:02}.{mo:02}.{y}"),
        hms: format!("{h:02}:{mi:02}:{s:02}"),
        date6: format!("{:02}{mo:02}{d:02}", y % 100),
        time6: format!("{h:02}{mi:02}{s:02}"),
        of_day: (h * 3600 + mi * 60 + s) as f64,
    }
}

fn stamp(start: i64, dur: i64) -> String {
    let (a, e) = (clock(start), clock(start + dur));
    format!("{}_S{}-E{}_D{:02}{:02}{:02}", a.date6, a.time6, e.time6, dur / 3600, dur % 3600 / 60, dur % 60)
}

/// Parses names written by the first function: `yymmdd_SHHMMSS-EHHMMSS_DHHMMSS_<label>.wav`.
pub fn parse_track_name(name: &str) -> Option<(i64, String)> {
    if name.len() < 36 || !name.to_ascii_lowercase().ends_with(".wav") {
        return None;
    }
    parse_track_stem(&name[..name.len() - 4])
}

/// The same name without its extension (also a track or output converted to another format).
fn parse_track_stem(name: &str) -> Option<(i64, String)> {
    let b = name.as_bytes();
    if b.len() < 32 {
        return None;
    }
    if &b[6..8] != b"_S" || &b[14..16] != b"-E" || &b[22..24] != b"_D" || b[30] != b'_' {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> {
        let s = &b[from..to];
        s.iter().all(u8::is_ascii_digit).then(|| s.iter().fold(0i64, |a, c| a * 10 + (c - b'0') as i64))
    };
    let (yy, mo, d) = (num(0, 2)?, num(2, 4)?, num(4, 6)?);
    let (h, mi, s) = (num(8, 10)?, num(10, 12)?, num(12, 14)?);
    num(16, 22)?;
    num(24, 30)?;
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    let label = name.get(31..)?;
    if label.is_empty() {
        return None;
    }
    Some((days_from_civil(2000 + yy, mo, d) * 86400 + h * 3600 + mi * 60 + s, label.to_string()))
}

fn is_output_label(label: &str) -> bool {
    label.starts_with("stereo_L-") || label.starts_with("mono_")
}

fn is_wav(p: &Path) -> bool {
    let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    !name.starts_with("._") && p.extension().map_or(false, |e| e.eq_ignore_ascii_case("wav"))
}

/// Collects WAV files and other decodable audio files. Result folders of this
/// step (`sync`) and of mastering (`master`, MP3 copies of outputs) are skipped.
fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 24 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        if ft.is_dir() {
            if name != OUTPUT_DIR_NAME && name != crate::master::OUTPUT_DIR_NAME {
                walk(&path, depth + 1, out);
            }
        } else if ft.is_file() && (is_wav(&path) || decode::is_compressed(&path)) {
            out.push(path);
        }
    }
}

fn make_track(name: String, path: String, label: String, start_secs: i64, info: WavInfo, segments: Vec<Segment>) -> Track {
    let ba = info.block_align as u64;
    let frames: u64 = segments.iter().map(|s| s.len / ba).sum();
    let duration = frames as f64 / info.sample_rate as f64;
    let (c0, c1) = (clock(start_secs), clock(start_secs + duration.round() as i64));
    Track {
        id: 0,
        name,
        path,
        label,
        day: c0.day,
        start: c0.hms,
        end: c1.hms,
        clock0: c0.of_day,
        duration,
        parts: segments.len(),
        start_secs,
        info,
        segments,
        frames,
        decoded_from: None,
        decoded: None,
    }
}

struct Loaded {
    tracks: Vec<Track>,
    /// Tracks from files of step 1 and from raw chunks (decoded files come later).
    from_files: usize,
    from_parts: usize,
    /// Non-WAV audio files, decoded in the analysis before they become tracks.
    compressed: Vec<PathBuf>,
    ignored: Vec<Skipped>,
    roots: Vec<PathBuf>,
    source: &'static str,
}

fn load(inputs: &[PathBuf]) -> Result<Loaded, String> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut files = Vec::new();
    for input in inputs {
        let meta = fs::metadata(input).map_err(|e| format!("{}: {e}", input.display()))?;
        if meta.is_dir() {
            if !roots.contains(input) {
                roots.push(input.clone());
            }
            walk(input, 0, &mut files);
        } else {
            if let Some(p) = input.parent() {
                if !roots.iter().any(|r| r == p) {
                    roots.push(p.to_path_buf());
                }
            }
            if is_wav(input) || decode::is_compressed(input) {
                files.push(input.clone());
            }
        }
    }
    if roots.is_empty() {
        return Err(t(Msg::NoFoldersOrFiles).into());
    }
    files.sort();
    files.dedup();

    let mut tracks = Vec::new();
    let mut ignored = Vec::new();
    let mut others = Vec::new();
    let mut compressed = Vec::new();
    for f in files {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if decode::is_compressed(&f) {
            // Converted copies of this step's outputs (e.g. mastered MP3s) are not sources.
            let stem = f.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !parse_track_stem(&stem).map_or(false, |(_, label)| is_output_label(&label)) {
                compressed.push(f);
            }
            continue;
        }
        match parse_track_name(&name) {
            Some((_, label)) if is_output_label(&label) => {}
            Some((start, label)) => match wav::read_info(&f) {
                Ok(info) if info.sample_kind().is_some() && info.data_len > 0 => {
                    let seg = vec![Segment { path: f.clone(), offset: info.data_offset, len: info.data_len }];
                    tracks.push(make_track(name, f.display().to_string(), label, start, info, seg));
                }
                Ok(_) => ignored.push(Skipped { path: f.display().to_string(), reason: t(Msg::AudioFormatUnsupported).into() }),
                Err(e) => ignored.push(Skipped { path: f.display().to_string(), reason: tf(Msg::UnreadableWith, &[("e", &e)]) }),
            },
            None => others.push(f),
        }
    }
    let from_files = tracks.len();

    // Every other WAV is raw recorder audio: grouped into recordings exactly as in
    // step 1 (metadata, name or file time, seam check). Chains become one track,
    // single files a track of their own. Recordings already merged are skipped.
    let mut from_parts = 0;
    if !others.is_empty() {
        let s = scan::scan(&others, &scan::Options::default())?;
        ignored.extend(s.duplicates);
        ignored.extend(s.ignored);
        for r in s.recordings {
            let merged = tracks[..from_files]
                .iter()
                .any(|t| t.label == r.label && (t.start_secs - r.start_secs).abs() <= 2 && (t.duration - r.duration).abs() <= 2.0);
            if merged {
                continue;
            }
            let info = r.parts[0].info.clone();
            if info.sample_kind().is_none() {
                ignored.push(Skipped { path: r.parts[0].path.clone(), reason: t(Msg::AudioFormatUnsupported).into() });
                continue;
            }
            let segments = r
                .parts
                .iter()
                .map(|p| Segment { path: p.path_buf.clone(), offset: p.info.data_offset, len: p.info.data_len })
                .collect();
            tracks.push(make_track(r.out_name.clone(), r.parts[0].path.clone(), r.label.clone(), r.start_secs, info, segments));
            from_parts += 1;
        }
    }
    let source = source_kind(from_files + compressed.len(), from_parts).ok_or_else(|| t(Msg::NoTracksOrChunks).to_string())?;
    Ok(Loaded { tracks, from_files, from_parts, compressed, ignored, roots, source })
}

/// "tracks", "chunks" or "mixed". Decoded audio files count like tracks: whole recordings, never chunks.
fn source_kind(files: usize, parts: usize) -> Option<&'static str> {
    match (files > 0, parts > 0) {
        (true, false) => Some("tracks"),
        (false, true) => Some("chunks"),
        (true, true) => Some("mixed"),
        (false, false) => None,
    }
}

/// Start of a non-WAV audio file, best source first: a time in the file name (chunk
/// pattern, a converted track of step 1, or any date with seconds), the creation
/// time of an MP4/M4A (only if set, 2010 or later and not after the last change),
/// else the file dates like step 1 (modification time − duration).
fn compressed_start(path: &Path, duration: f64) -> Option<i64> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some((_, start)) = scan::parse_chunk_name(&name) {
        return Some(start);
    }
    if let Some((start, _)) = parse_track_stem(&stem) {
        return Some(start);
    }
    if let Some(start) = scan::parse_name_time(&name) {
        return Some(start);
    }
    let meta = fs::metadata(path).ok()?;
    if matches!(decode::extension(path).as_str(), "m4a" | "mp4") {
        let modified = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs_f64());
        if let Some(created) = decode::mp4_creation_time(path) {
            if civil_from_secs(created).0 >= 2010 && modified.map_or(true, |m| created as f64 <= m + 5.0) {
                return Some(created + scan::local_offset(created));
            }
        }
    }
    scan::file_start_time(&meta, duration).map(|s| s.floor() as i64)
}

/// Decodes every non-WAV file once into the cache (or reuses it) and makes it a track.
/// Failures go to `ignored`; only cancelling and a full cache volume stop the analysis.
fn decode_sources(
    paths: &[PathBuf],
    existing: &[Track],
    cache: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&SyncProgress),
    ignored: &mut Vec<Skipped>,
) -> Result<Vec<Track>, String> {
    let mut sources: Vec<(PathBuf, decode::Source)> = Vec::new();
    for p in paths {
        match decode::Source::new(p) {
            Ok(src) => sources.push((p.clone(), src)),
            Err(e) => ignored.push(Skipped { path: p.display().to_string(), reason: tf(Msg::UnreadableWith, &[("e", &e)]) }),
        }
    }
    let keys: Vec<String> = sources.iter().map(|(_, s)| s.key.clone()).collect();
    decode::clean_cache(cache, &keys);
    let hits: Vec<Option<(PathBuf, WavInfo)>> = sources.iter().map(|(_, s)| decode::cached(cache, s)).collect();
    let need: u64 = sources.iter().zip(&hits).filter(|(_, h)| h.is_none()).map(|((_, s), _)| decode::estimate_decoded_bytes(s)).sum();
    decode::check_space(cache, need)?;

    let total: u64 = sources.iter().map(|(_, s)| s.size).sum::<u64>().max(1);
    let read: Vec<Arc<AtomicU64>> = sources.iter().zip(&hits).map(|((_, s), h)| Arc::new(AtomicU64::new(if h.is_some() { s.size } else { 0 }))).collect();
    let jobs: Vec<usize> = (0..sources.len()).collect();
    let results = {
        let (sources, hits, read) = (&sources, &hits, &read);
        run_parallel(
            &jobs,
            &mut || {
                let done: u64 = read.iter().zip(sources).map(|(r, (_, s))| r.load(Ordering::Relaxed).min(s.size)).sum();
                progress(&SyncProgress { stage: "decode", done: done.min(total), total, text: t(Msg::ProgressDecode).into() })
            },
            &|&i: &usize| match &hits[i] {
                Some(hit) => Ok(hit.clone()),
                None => decode::decode_to_cache(cache, &sources[i].1, cancel, read[i].clone()),
            },
        )
    };
    if cancel.load(Ordering::SeqCst) {
        return Err(i18n::cancelled());
    }
    let mut out: Vec<Track> = Vec::new();
    for ((orig, src), r) in sources.iter().zip(results) {
        let shown = orig.display().to_string();
        let (wav_path, info) = match r {
            Some(Ok(x)) => x,
            Some(Err(e)) if i18n::is_cancelled(&e) => return Err(e),
            Some(Err(e)) => {
                ignored.push(Skipped { path: shown, reason: e });
                continue;
            }
            None => {
                ignored.push(Skipped { path: shown, reason: t(Msg::AnalysisFailed).into() });
                continue;
            }
        };
        let name = orig.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = orig.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(start) = compressed_start(&src.path, info.duration()) else {
            ignored.push(Skipped { path: shown, reason: t(Msg::Unreadable).into() });
            continue;
        };
        // A converted track of step 1 keeps its label; any other file is labelled by its folder.
        let label = parse_track_stem(&stem)
            .map(|(_, l)| l)
            .unwrap_or_else(|| orig.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let segments = vec![Segment { path: wav_path.clone(), offset: info.data_offset, len: info.data_len }];
        let mut track = make_track(name, shown, label, start, info, segments);
        // The same recording as a WAV track already loaded (e.g. a FLAC copy next to it) is not loaded twice.
        let copy = existing.iter().chain(&out).any(|t| t.label == track.label && (t.start_secs - track.start_secs).abs() <= 2 && (t.duration - track.duration).abs() <= 2.0);
        if copy {
            continue;
        }
        track.decoded_from = Some(decode::container(&decode::extension(orig)).to_string());
        track.decoded = Some(wav_path);
        out.push(track);
    }
    Ok(out)
}

fn default_out_dir(roots: &[PathBuf]) -> PathBuf {
    let mut common = roots[0].clone();
    for r in &roots[1..] {
        while !r.starts_with(&common) {
            if !common.pop() {
                break;
            }
        }
    }
    if common.parent().is_none() {
        common = roots[0].clone();
    }
    if common.file_name().map_or(false, |n| n == scan::OUTPUT_DIR_NAME) {
        if let Some(parent) = common.parent() {
            return parent.join(OUTPUT_DIR_NAME);
        }
    }
    common.join(OUTPUT_DIR_NAME)
}

fn label_order(a: &str, b: &str) -> CmpOrdering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

// ------------------------------------------------------------------ signal basics

#[derive(Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn new(highpass: bool, fs: f64, f0: f64, q: f64) -> Self {
        let w0 = TAU * f0 / fs;
        let (sn, cs) = (w0.sin(), w0.cos());
        let alpha = sn / (2.0 * q);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = if highpass {
            ((1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0)
        } else {
            ((1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0)
        };
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: -2.0 * cs / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0 }
    }

    #[inline]
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        if self.z1.abs() < 1e-25 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-25 {
            self.z2 = 0.0;
        }
        y
    }
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let n = v.len();
    let mid = n / 2;
    v.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    let hi = v[mid];
    if n % 2 == 1 {
        hi
    } else {
        let lo = v[..mid].iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (lo + hi) / 2.0
    }
}

fn fft(re: &mut [f64], im: &mut [f64], invert: bool) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let sign = if invert { -1.0 } else { 1.0 };
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let table: Vec<(f64, f64)> = (0..half)
            .map(|k| {
                let a = sign * TAU * k as f64 / len as f64;
                (a.cos(), a.sin())
            })
            .collect();
        for start in (0..n).step_by(len) {
            for (k, &(wr, wi)) in table.iter().enumerate() {
                let (xr, xi) = (re[start + k + half], im[start + k + half]);
                let (vr, vi) = (xr * wr - xi * wi, xr * wi + xi * wr);
                let (ur, ui) = (re[start + k], im[start + k]);
                re[start + k] = ur + vr;
                im[start + k] = ui + vi;
                re[start + k + half] = ur - vr;
                im[start + k + half] = ui - vi;
            }
        }
        len <<= 1;
    }
    if invert {
        let s = 1.0 / n as f64;
        re.iter_mut().for_each(|x| *x *= s);
        im.iter_mut().for_each(|x| *x *= s);
    }
}

/// `c[lag] = Σ_j b[j]·a[j+lag]` for `lag` in `-(len b − 1) ..= len a − 1`; returns the values and the first lag.
fn xcorr(a: &[f64], b: &[f64]) -> (Vec<f64>, isize) {
    let (la, lb) = (a.len(), b.len());
    let n = (la + lb - 1).next_power_of_two();
    let (mut ar, mut ai) = (vec![0.0; n], vec![0.0; n]);
    let (mut br, mut bi) = (vec![0.0; n], vec![0.0; n]);
    ar[..la].copy_from_slice(a);
    br[..lb].copy_from_slice(b);
    fft(&mut ar, &mut ai, false);
    fft(&mut br, &mut bi, false);
    let (mut cr, mut ci) = (vec![0.0; n], vec![0.0; n]);
    for k in 0..n {
        cr[k] = ar[k] * br[k] + ai[k] * bi[k];
        ci[k] = ai[k] * br[k] - ar[k] * bi[k];
    }
    fft(&mut cr, &mut ci, true);
    let first = -(lb as isize - 1);
    let out = (0..la + lb - 1).map(|i| cr[(first + i as isize).rem_euclid(n as isize) as usize]).collect();
    (out, first)
}

fn centered(v: &[f32]) -> Vec<f64> {
    let mean = v.iter().map(|&x| x as f64).sum::<f64>() / v.len().max(1) as f64;
    v.iter().map(|&x| x as f64 - mean).collect()
}

fn tukey(v: &mut [f64], alpha: f64) {
    let m = v.len();
    if m < 2 {
        return;
    }
    let width = alpha * (m - 1) as f64 / 2.0;
    let flat_end = (m - 1) as f64 * (1.0 - alpha / 2.0);
    for (i, x) in v.iter_mut().enumerate() {
        let n = i as f64;
        let w = if n < width {
            0.5 * (1.0 + (PI * (n / width - 1.0)).cos())
        } else if n <= flat_end {
            1.0
        } else {
            0.5 * (1.0 + (PI * (n / width - 2.0 / alpha + 1.0)).cos())
        };
        *x *= w;
    }
}

/// Peak position and robust prominence `(peak − median) / (1.4826·MAD)` of a
/// correlation curve, ignoring ±`exclude` around the peak for the statistics.
fn peak_stats(curve: &[(isize, f64)], exclude: isize) -> (isize, f64) {
    let (dk, ck) = curve.iter().copied().fold((0, f64::NEG_INFINITY), |best, x| if x.1 > best.1 { x } else { best });
    let mut rest: Vec<f64> = curve.iter().filter(|(d, _)| (d - dk).abs() > exclude).map(|x| x.1).collect();
    if rest.len() < 10 {
        return (dk, 0.0);
    }
    let med = median(&mut rest);
    let mut dev: Vec<f64> = rest.iter().map(|v| (v - med).abs()).collect();
    let mad = median(&mut dev) * 1.4826 + 1e-12;
    (dk, (ck - med) / mad)
}

// ------------------------------------------------------------------ envelopes

struct Env {
    /// Onset strength per millisecond (sum over bands of normalised dB rises).
    onset: Vec<f32>,
    /// Band energy per millisecond (for levels and quiet cut points).
    loud: Vec<f32>,
    /// Waveform peak per millisecond, coded like `Peaks`.
    peak: Vec<u8>,
}

fn peak_code(v: f64) -> u8 {
    if v <= 1e-6 {
        0
    } else {
        (((20.0 * v.log10() + 60.0) / 60.0).clamp(0.0, 1.0) * 255.0).round() as u8
    }
}

fn envelope(track: &Track, cancel: &AtomicBool, done: &AtomicU64) -> Result<Env, String> {
    let info = &track.info;
    let kind = info.sample_kind().ok_or(t(Msg::AudioFormatUnsupported))?;
    let ch = info.channels as usize;
    let ba = info.block_align as usize;
    let bps = ba / ch;
    let fs = info.sample_rate as f64;
    let mut filters: Vec<[Biquad; 2]> = BANDS
        .iter()
        .map(|&(lo, hi)| [Biquad::new(true, fs, lo, FRAC_1_SQRT_2), Biquad::new(false, fs, hi.min(0.45 * fs), FRAC_1_SQRT_2)])
        .collect();
    let cap = (track.duration * ENV_RATE) as usize + 2;
    let mut bands: Vec<Vec<f32>> = (0..BANDS.len()).map(|_| Vec::with_capacity(cap)).collect();
    let mut acc = [0f64; 4];
    let mut cnt = 0u32;
    let mut frame = 0u64;
    let boundary = |k: u64| ((k as f64) * fs / ENV_RATE).round() as u64;
    let mut next = boundary(1);
    let mut peak = 0f64;
    let mut peaks = Vec::with_capacity(cap);
    let mut buf = vec![0u8; ba * 65536];
    for seg in &track.segments {
        let mut f = File::open(&seg.path).map_err(|e| format!("{}: {e}", seg.path.display()))?;
        f.seek(SeekFrom::Start(seg.offset)).map_err(|e| e.to_string())?;
        let mut remaining = seg.len - seg.len % ba as u64;
        while remaining > 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err(i18n::cancelled());
            }
            let n = remaining.min(buf.len() as u64) as usize;
            f.read_exact(&mut buf[..n]).map_err(|e| format!("{}: {e}", seg.path.display()))?;
            for fr in buf[..n].chunks_exact(ba) {
                let x = if ch == 1 {
                    wav::decode_sample(fr, kind)
                } else {
                    (0..ch).map(|c| wav::decode_sample(&fr[c * bps..], kind)).sum::<f64>() / ch as f64
                };
                for (k, flt) in filters.iter_mut().enumerate() {
                    let y0 = flt[0].run(x);
                    let y = flt[1].run(y0);
                    acc[k] += y * y;
                }
                peak = peak.max(x.abs());
                cnt += 1;
                frame += 1;
                if frame == next {
                    peaks.push(peak_code(peak));
                    peak = 0.0;
                    for k in 0..BANDS.len() {
                        bands[k].push((acc[k] / cnt as f64) as f32);
                        acc[k] = 0.0;
                    }
                    cnt = 0;
                    next = boundary(bands[0].len() as u64 + 1);
                }
            }
            remaining -= n as u64;
            done.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
    let mut env = onset_from_bands(&bands);
    env.peak = peaks;
    Ok(env)
}

fn onset_from_bands(bands: &[Vec<f32>]) -> Env {
    let m = bands[0].len();
    let mut onset = vec![0f32; m];
    let mut loud = vec![0f32; m];
    for band in bands {
        let mut cum = vec![0f64; m + 1];
        for i in 0..m {
            loud[i] += band[i];
            cum[i + 1] = cum[i] + 10.0 * (band[i] as f64 + 1e-12).log10();
        }
        let mean_ahead = |t: usize| (cum[t + ONSET_SPAN] - cum[t]) / ONSET_SPAN as f64;
        let mut rise = vec![0f32; m];
        if m >= 2 * ONSET_SPAN {
            for t in ONSET_SPAN..=m - ONSET_SPAN {
                rise[t] = (mean_ahead(t) - mean_ahead(t - ONSET_SPAN)).max(0.0) as f32;
            }
        }
        let mut positive: Vec<f64> = rise.iter().filter(|&&v| v > 0.0).map(|&v| v as f64).collect();
        let scale = if positive.is_empty() { 1.0 } else { median(&mut positive) as f32 };
        for i in 0..m {
            onset[i] += (rise[i] / scale).min(ONSET_CLIP);
        }
    }
    Env { onset, loud, peak: Vec::new() }
}

// ------------------------------------------------------------------ global offset

fn pool(v: &[f32], factor: usize) -> Vec<f64> {
    let m = v.len() / factor;
    let pooled: Vec<f32> = (0..m).map(|k| v[k * factor..(k + 1) * factor].iter().sum::<f32>() / factor as f32).collect();
    centered(&pooled)
}

fn coarse_offset(ea: &[f64], eb: &[f64], nominal: f64, rate: f64) -> Option<(f64, f64)> {
    if ea.len() < 10 || eb.len() < 10 {
        return None;
    }
    let (c, first) = xcorr(ea, eb);
    let lag_s = |i: usize| (first + i as isize) as f64 / rate;
    let mut idx: Vec<usize> = (0..c.len()).filter(|&i| (lag_s(i) - nominal).abs() <= SEARCH_S).collect();
    if idx.is_empty() {
        idx = (0..c.len()).collect();
    }
    let best = *idx.iter().max_by(|&&i, &&j| c[i].total_cmp(&c[j]))?;
    let rest: Vec<f64> = idx.iter().filter(|&&i| (lag_s(i) - lag_s(best)).abs() > 2.0).map(|&i| c[i]).collect();
    let z = if rest.len() > 10 {
        let mean = rest.iter().sum::<f64>() / rest.len() as f64;
        let sd = (rest.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / rest.len() as f64).sqrt();
        (c[best] - mean) / (sd + 1e-9)
    } else {
        0.0
    };
    Some((lag_s(best), z))
}

struct Win {
    t: f64,
    lag: f64,
    z: f64,
}

fn fine_windows(oa: &[f32], ob: &[f32], coarse: f64) -> Vec<Win> {
    let w = FINE_WINDOW_MS;
    let fs = FINE_SEARCH_MS;
    let t_end = (oa.len() as f64 / ENV_RATE).min(ob.len() as f64 / ENV_RATE + coarse + fs as f64 / ENV_RATE);
    let mut t = (coarse - fs as f64 / ENV_RATE).max(0.0);
    let mut out = Vec::new();
    while t + w as f64 / ENV_RATE <= t_end {
        let ia = (t * ENV_RATE) as isize;
        let ib = ((t - coarse) * ENV_RATE) as isize;
        let (lo, hi) = (ib - fs, ib + w as isize + fs);
        if lo >= 0 && hi as usize <= ob.len() && ia as usize + w <= oa.len() {
            let mut a = centered(&oa[ia as usize..ia as usize + w]);
            let mut b = centered(&ob[lo as usize..hi as usize]);
            tukey(&mut a, 0.1);
            tukey(&mut b, 0.1);
            let (c, first) = xcorr(&a, &b);
            let curve: Vec<(isize, f64)> = c
                .iter()
                .enumerate()
                .map(|(i, &v)| (first + i as isize + fs, v))
                .filter(|(d, _)| d.abs() <= fs - fs / 20)
                .collect();
            let (d, z) = peak_stats(&curve, PEAK_EXCLUDE_MS);
            out.push(Win { t, lag: (ia + (d - fs) - lo) as f64 / ENV_RATE, z });
        }
        t += FINE_STEP_S;
    }
    out
}

struct Fit {
    offset: f64,
    drift: f64,
    resid: f64,
    n: usize,
}

fn line(pts: &[(f64, f64)]) -> (f64, f64) {
    let n = pts.len() as f64;
    let mt = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = pts.iter().map(|p| (p.0 - mt).powi(2)).sum();
    let sxy: f64 = pts.iter().map(|p| (p.0 - mt) * (p.1 - my)).sum();
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    (my - slope * mt, slope)
}

fn robust_fit(wins: &[Win]) -> Option<Fit> {
    let mut pts: Vec<(f64, f64)> = wins.iter().filter(|w| w.z >= FINE_Z_MIN).map(|w| (w.t, w.lag)).collect();
    if pts.len() < 2 {
        return None;
    }
    for _ in 0..6 {
        let (a, b) = line(&pts);
        let resid: Vec<f64> = pts.iter().map(|&(t, y)| y - (a + b * t)).collect();
        let med = median(&mut resid.clone());
        let mad = median(&mut resid.iter().map(|r| (r - med).abs()).collect::<Vec<_>>()) * 1.4826;
        let thr = (3.0 * mad).max(0.005);
        let keep: Vec<bool> = resid.iter().map(|r| r.abs() < thr).collect();
        let kept = keep.iter().filter(|&&k| k).count();
        if kept == pts.len() || kept < 2 {
            break;
        }
        pts = pts.into_iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| p).collect();
    }
    let (a, b) = line(&pts);
    let resid = (pts.iter().map(|&(t, y)| (y - a - b * t).powi(2)).sum::<f64>() / pts.len() as f64).sqrt();
    Some(Fit { offset: a, drift: b, resid, n: pts.len() })
}

// ------------------------------------------------------------------ per-window event test

fn read_mono(track: &Track, start: i64, n: usize) -> io::Result<Vec<f32>> {
    let mut out = vec![0f32; n];
    let (s, e) = (start.max(0), (start + n as i64).min(track.frames as i64));
    if e <= s {
        return Ok(out);
    }
    let info = &track.info;
    let kind = info.sample_kind().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Format"))?;
    let (ch, ba) = (info.channels as usize, info.block_align as u64);
    let bps = ba as usize / ch;
    let mut frame = s as u64;
    let mut seg0 = 0u64;
    for seg in &track.segments {
        let seg_end = seg0 + seg.len / ba;
        if frame < seg_end && (frame as i64) < e {
            let to = (e as u64).min(seg_end);
            let raw = wav::read_at(&seg.path, seg.offset + (frame - seg0) * ba, (to - frame) * ba)?;
            for (k, fr) in raw.chunks_exact(ba as usize).enumerate() {
                let v = if ch == 1 {
                    wav::decode_sample(fr, kind)
                } else {
                    (0..ch).map(|c| wav::decode_sample(&fr[c * bps..], kind)).sum::<f64>() / ch as f64
                };
                out[(frame as i64 - start) as usize + k] = v as f32;
            }
            frame = to;
        }
        seg0 = seg_end;
    }
    Ok(out)
}

/// Low-passed, decimated excerpt (~3 kHz) for the coherence check.
fn read_decimated(track: &Track, t0: f64, secs: f64) -> io::Result<(Vec<f32>, f64)> {
    let sr = track.info.sample_rate as f64;
    let factor = ((sr / COH_RATE).round() as usize).max(1);
    let rate = sr / factor as f64;
    let warm = (0.1 * sr) as usize;
    let start = (t0 * sr).round() as i64 - warm as i64;
    let n = (secs * sr).round() as usize + warm;
    let x = read_mono(track, start, n)?;
    let cutoff = 0.4 * rate;
    let mut lp = [Biquad::new(false, sr, cutoff, 0.541_196), Biquad::new(false, sr, cutoff, 1.306_563)];
    let mut out = Vec::with_capacity(n / factor + 1);
    for (i, &v) in x.iter().enumerate() {
        let y0 = lp[0].run(v as f64);
        let y = lp[1].run(y0);
        if i >= warm && (i - warm) % factor == 0 {
            out.push(y as f32);
        }
    }
    Ok((out, rate))
}

/// Mean magnitude-squared coherence (Welch, Hann, 50 % overlap) in 150–1200 Hz.
fn msc(a: &[f32], b: &[f32], rate: f64) -> Option<f64> {
    let n = COH_SEG;
    let len = a.len().min(b.len());
    if len < n * 4 {
        return None;
    }
    let win: Vec<f64> = (0..n).map(|i| 0.5 - 0.5 * (TAU * i as f64 / n as f64).cos()).collect();
    let bins: Vec<usize> = (0..=n / 2)
        .filter(|&k| {
            let f = k as f64 * rate / n as f64;
            (COH_LOW..=COH_HIGH).contains(&f)
        })
        .collect();
    if bins.is_empty() {
        return None;
    }
    let (mut saa, mut sbb, mut sre, mut sim) = (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    let (mut ar, mut ai, mut br, mut bi) = (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    let mut start = 0;
    while start + n <= len {
        let ma = a[start..start + n].iter().map(|&x| x as f64).sum::<f64>() / n as f64;
        let mb = b[start..start + n].iter().map(|&x| x as f64).sum::<f64>() / n as f64;
        for i in 0..n {
            ar[i] = (a[start + i] as f64 - ma) * win[i];
            br[i] = (b[start + i] as f64 - mb) * win[i];
            ai[i] = 0.0;
            bi[i] = 0.0;
        }
        fft(&mut ar, &mut ai, false);
        fft(&mut br, &mut bi, false);
        for &k in &bins {
            saa[k] += ar[k] * ar[k] + ai[k] * ai[k];
            sbb[k] += br[k] * br[k] + bi[k] * bi[k];
            sre[k] += ar[k] * br[k] + ai[k] * bi[k];
            sim[k] += ai[k] * br[k] - ar[k] * bi[k];
        }
        start += n / 2;
    }
    Some(bins.iter().map(|&k| (sre[k] * sre[k] + sim[k] * sim[k]) / (saa[k] * sbb[k] + 1e-30)).sum::<f64>() / bins.len() as f64)
}

fn span(p: &Pair, a: &Track, b: &Track) -> (f64, f64) {
    (p.to_a(0.0).max(0.0), p.to_a(b.duration).min(a.duration))
}

fn frame_features(p: &Pair, ta: &Track, tb: &Track, ea: &Env, eb: &Env, cancel: &AtomicBool) -> Vec<Frame> {
    let (ov0, ov1) = span(p, ta, tb);
    let (n, s) = (FRAME_MS, FRAME_SEARCH_MS);
    let secs = n as f64 / ENV_RATE;
    let level = |e: &[f32]| (10.0 * (e.iter().map(|&x| x as f64).sum::<f64>() / e.len() as f64 + 1e-12).log10()) as f32;
    let same_rate = ta.info.sample_rate == tb.info.sample_rate;
    let mut rows = Vec::new();
    let mut t = ov0;
    while t + secs <= ov1 {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let ia = (t * ENV_RATE).round() as isize;
        let ib = (p.to_b(t) * ENV_RATE).round() as isize;
        if ia >= 0 && ia as usize + n <= ea.onset.len() && ib - s >= 0 && (ib + n as isize + s) as usize <= eb.onset.len() {
            let (ia, ib) = (ia as usize, ib as usize);
            let a = centered(&ea.onset[ia..ia + n]);
            let b = centered(&eb.onset[ib - s as usize..ib + n + s as usize]);
            let (c, first) = xcorr(&a, &b);
            let curve: Vec<(isize, f64)> =
                c.iter().enumerate().map(|(i, &v)| (first + i as isize + s, v)).filter(|(d, _)| d.abs() <= s).collect();
            let (d, z) = peak_stats(&curve, PEAK_EXCLUDE_MS);
            let msc = if same_rate {
                match (read_decimated(ta, t, secs), read_decimated(tb, p.to_b(t) - d as f64 / ENV_RATE, secs)) {
                    (Ok((xa, rate)), Ok((xb, _))) => msc(&xa, &xb, rate).map(|v| v as f32),
                    _ => None,
                }
            } else {
                None
            };
            rows.push(Frame {
                t,
                lag_ms: d as i32,
                z: z as f32,
                msc,
                level_a: level(&ea.loud[ia..ia + n]),
                level_b: level(&eb.loud[ib..ib + n]),
                hit: (d as i32).abs() <= LAG_TOL_MS && z >= FRAME_Z_MIN,
                active: true,
            });
        }
        t += HOP_S;
    }
    // Windows where both microphones are quiet carry no evidence either way.
    let med = |f: &dyn Fn(&Frame) -> f32| {
        let mut v: Vec<f64> = rows.iter().map(|r| f(r) as f64).collect();
        median(&mut v) as f32
    };
    let (med_a, med_b) = (med(&|r| r.level_a), med(&|r| r.level_b));
    for r in rows.iter_mut() {
        r.active = !(r.level_a < med_a - SILENCE_DB && r.level_b < med_b - SILENCE_DB);
    }
    rows
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Together,
    Apart,
}

#[derive(Clone, Debug)]
struct Ph {
    kind: Kind,
    start: f64,
    end: f64,
}

fn classify(frames: &[Frame]) -> Vec<Kind> {
    let n = frames.len();
    let mut st: Vec<Option<Kind>> = (0..n)
        .map(|i| {
            if !frames[i].active {
                return None;
            }
            let win: Vec<&Frame> = frames[i.saturating_sub(SMOOTH / 2)..(i + SMOOTH / 2 + 1).min(n)].iter().filter(|f| f.active).collect();
            let share = win.iter().filter(|f| f.hit).count() as f64 / win.len() as f64;
            if share >= HIT_TOGETHER {
                Some(Kind::Together)
            } else if share <= HIT_APART {
                Some(Kind::Apart)
            } else {
                None
            }
        })
        .collect();
    let mut last = None;
    for s in st.iter_mut() {
        if s.is_none() {
            *s = last;
        } else {
            last = *s;
        }
    }
    let mut last = None;
    for s in st.iter_mut().rev() {
        if s.is_none() {
            *s = Some(last.unwrap_or(Kind::Together));
        } else {
            last = *s;
        }
    }
    st.into_iter().map(|s| s.unwrap_or(Kind::Together)).collect()
}

fn to_phases(kinds: &[Kind], frames: &[Frame]) -> Vec<Ph> {
    let mut out: Vec<Ph> = Vec::new();
    for (k, f) in kinds.iter().zip(frames) {
        match out.last_mut() {
            Some(p) if p.kind == *k => p.end = f.t + HOP_S,
            _ => out.push(Ph { kind: *k, start: f.t, end: f.t + HOP_S }),
        }
    }
    out
}

fn rejoin(ph: Vec<Ph>) -> Vec<Ph> {
    let mut out: Vec<Ph> = Vec::new();
    for p in ph {
        match out.last_mut() {
            Some(q) if q.kind == p.kind => q.end = p.end,
            _ => out.push(p),
        }
    }
    out
}

fn longest(ph: &[Ph], nb: &[usize]) -> usize {
    let mut best = nb[0];
    for &j in &nb[1..] {
        if ph[j].end - ph[j].start > ph[best].end - ph[best].start {
            best = j;
        }
    }
    best
}

/// Phases that are too short take the kind of their neighbour, shortest first,
/// so a brief gap inside a long stretch never swallows the stretch. A short
/// together phase between apart phases becomes apart (a wrong stereo pair costs
/// more than two mono files).
fn merge_short(mut ph: Vec<Ph>) -> Vec<Ph> {
    while ph.len() > 1 {
        let too_short = |p: &Ph| {
            let dur = p.end - p.start;
            match p.kind {
                Kind::Together => dur < MIN_TOGETHER_S,
                Kind::Apart => dur < MIN_APART_S,
            }
        };
        let Some(i) = (0..ph.len())
            .filter(|&i| too_short(&ph[i]))
            .min_by(|&x, &y| (ph[x].end - ph[x].start).total_cmp(&(ph[y].end - ph[y].start)))
        else {
            break;
        };
        let nb: Vec<usize> = [i.wrapping_sub(1), i + 1].into_iter().filter(|&j| j < ph.len()).collect();
        let j = longest(&ph, &nb);
        ph[i].kind = ph[j].kind;
        ph = rejoin(ph);
    }
    ph
}

fn share(frames: &[Frame], s: f64, e: f64) -> Option<f64> {
    let v: Vec<&Frame> = frames.iter().filter(|f| f.active && f.t >= s && f.t < e).collect();
    (!v.is_empty()).then(|| v.iter().filter(|f| f.hit).count() as f64 / v.len() as f64)
}

fn mean_msc(frames: &[Frame], s: f64, e: f64) -> Option<f64> {
    let v: Vec<f64> = frames.iter().filter(|f| f.active && f.t >= s && f.t < e).filter_map(|f| f.msc.map(|m| m as f64)).collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

fn phases(p: &Pair, ta: &Track, tb: &Track, ea: &Env, eb: &Env) -> Vec<Phase> {
    let (ov0, ov1) = span(p, ta, tb);
    let out = |ph: &[Ph]| -> Vec<Phase> {
        ph.iter()
            .map(|x| Phase {
                kind: if x.kind == Kind::Together { "together" } else { "apart" },
                start: x.start,
                end: x.end,
                hit_share: share(&p.frames, x.start, x.end),
                msc: mean_msc(&p.frames, x.start, x.end),
            })
            .collect()
    };
    if p.frames.is_empty() {
        return out(&[Ph { kind: Kind::Together, start: ov0, end: ov1 }]);
    }
    let mut ph = merge_short(to_phases(&classify(&p.frames), &p.frames));
    for _ in 0..3 {
        for x in ph.iter_mut() {
            let weak = share(&p.frames, x.start, x.end).map_or(false, |s| s < DEMOTE_SHARE)
                || mean_msc(&p.frames, x.start, x.end).map_or(false, |m| m < DEMOTE_MSC);
            if x.kind == Kind::Together && weak {
                x.kind = Kind::Apart;
            }
        }
        ph = merge_short(ph);
    }
    ph[0].start = ov0;
    let last = ph.len() - 1;
    ph[last].end = ov1;

    // Move each cut to the quietest second within ±30 s so no word is split.
    let second = |s: f64| -> f64 {
        let part = |e: &[f32], i: f64| {
            let i = (i * ENV_RATE).round();
            if i < 0.0 || i as usize + 1000 > e.len() {
                return 0.0;
            }
            e[i as usize..i as usize + 1000].iter().map(|&x| x as f64).sum::<f64>() / 1000.0
        };
        part(&ea.loud, s) + part(&eb.loud, p.to_b(s))
    };
    for i in 1..ph.len() {
        let t = ph[i].start;
        let lo = (ph[i - 1].start + SNAP_WINDOW_S).max(t - SNAP_WINDOW_S);
        let hi = (ph[i].end - SNAP_WINDOW_S).min(t + SNAP_WINDOW_S);
        if hi - lo < 2.0 {
            continue;
        }
        let (mut best, mut best_e, mut s) = (t, f64::INFINITY, lo);
        while s < hi {
            let e = second(s);
            if e < best_e {
                best = s;
                best_e = e;
            }
            s += 1.0;
        }
        ph[i - 1].end = best;
        ph[i].start = best;
    }
    out(&ph)
}

fn analyze_pair(ta: &Track, tb: &Track, ea: &Env, eb: &Env, cancel: &AtomicBool) -> Pair {
    let nominal = (tb.start_secs - ta.start_secs) as f64;
    let mut p = Pair {
        a: ta.id,
        b: tb.id,
        ok: false,
        offset: nominal,
        drift_ppm: 0.0,
        resid_ms: 0.0,
        coarse_z: 0.0,
        n_good: 0,
        n_windows: 0,
        note: None,
        frames: Vec::new(),
        phases: Vec::new(),
        drift: 0.0,
    };
    let rate = ENV_RATE / COARSE_POOL as f64;
    let coarse = coarse_offset(&pool(&ea.onset, COARSE_POOL), &pool(&eb.onset, COARSE_POOL), nominal, rate);
    if let Some((co, z)) = coarse {
        p.coarse_z = z;
        let wins = fine_windows(&ea.onset, &eb.onset, co);
        p.n_windows = wins.len();
        if let Some(fit) = robust_fit(&wins) {
            p.n_good = fit.n;
            p.resid_ms = fit.resid * 1000.0;
            if fit.n >= 3 && fit.resid < FIT_RESID_MAX_S && z >= COARSE_Z_MIN {
                p.ok = true;
                p.offset = fit.offset;
                p.drift = fit.drift;
                p.drift_ppm = fit.drift * 1e6;
            }
        }
    }
    if !p.ok {
        p.note = Some(t(if p.n_windows == 0 { Msg::PairTooLittleOverlap } else { Msg::PairNoStableEvents }).to_string());
        let (ov0, ov1) = span(&p, ta, tb);
        if ov1 > ov0 {
            p.phases.push(Phase { kind: "apart", start: ov0, end: ov1, hit_share: None, msc: None });
        }
        return p;
    }
    p.frames = frame_features(&p, ta, tb, ea, eb, cancel);
    p.phases = phases(&p, ta, tb, ea, eb);
    p
}

// ------------------------------------------------------------------ planning

pub(crate) fn run_parallel<T: Sync, R: Send>(jobs: &[T], progress: &mut dyn FnMut(), work: &(dyn Fn(&T) -> R + Sync)) -> Vec<Option<R>> {
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<R>>> = jobs.iter().map(|_| Mutex::new(None)).collect();
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(jobs.len()).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                s.spawn(|| loop {
                    let k = next.fetch_add(1, Ordering::SeqCst);
                    if k >= jobs.len() {
                        break;
                    }
                    let r = work(&jobs[k]);
                    *slots[k].lock().unwrap() = Some(r);
                })
            })
            .collect();
        loop {
            let finished = handles.iter().all(|h| h.is_finished());
            progress();
            if finished {
                break;
            }
            std::thread::sleep(Duration::from_millis(120));
        }
    });
    slots.into_iter().map(|m| m.into_inner().unwrap_or(None)).collect()
}

pub fn analyze(inputs: &[PathBuf], cancel: &AtomicBool, progress: &mut dyn FnMut(&SyncProgress)) -> Result<SyncPlan, String> {
    analyze_in(inputs, &decode::cache_dir(), cancel, progress)
}

/// `analyze` with the folder that holds decoded copies of non-WAV files.
pub fn analyze_in(inputs: &[PathBuf], cache: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(&SyncProgress)) -> Result<SyncPlan, String> {
    progress(&SyncProgress { stage: "load", done: 0, total: 1, text: t(Msg::ProgressFindTracks).into() });
    let Loaded { mut tracks, from_files, from_parts, compressed, mut ignored, roots, mut source } = load(inputs)?;
    if compressed.is_empty() {
        decode::clean_cache(cache, &[]);
    } else {
        let before = ignored.len();
        let decoded = decode_sources(&compressed, &tracks, cache, cancel, progress, &mut ignored)?;
        source = match source_kind(from_files + decoded.len(), from_parts) {
            Some(s) => s,
            None => {
                return Err(ignored[before..].first().map_or_else(|| t(Msg::NoTracksOrChunks).to_string(), |x| format!("{}: {}", x.path, x.reason)));
            }
        };
        tracks.extend(decoded);
    }
    tracks.sort_by(|a, b| a.start_secs.cmp(&b.start_secs).then_with(|| label_order(&a.label, &b.label)));
    for (i, t) in tracks.iter_mut().enumerate() {
        t.id = i;
    }
    let mut labels: Vec<String> = tracks.iter().map(|t| t.label.clone()).collect::<HashSet<_>>().into_iter().collect();
    labels.sort_by(|a, b| label_order(a, b));

    let mut cands: Vec<(usize, usize)> = Vec::new();
    for i in 0..tracks.len() {
        for j in i + 1..tracks.len() {
            let (x, y) = (&tracks[i], &tracks[j]);
            if x.label == y.label || x.duration < MIN_TRACK_S || y.duration < MIN_TRACK_S {
                continue;
            }
            let (a, b) = if label_order(&x.label, &y.label) == CmpOrdering::Greater { (y, x) } else { (x, y) };
            let s = a.start_secs.max(b.start_secs) as f64;
            let e = (a.start_secs as f64 + a.duration).min(b.start_secs as f64 + b.duration);
            if e - s >= MIN_OVERLAP_S - SEARCH_S {
                cands.push((a.id, b.id));
            }
        }
    }

    // Every track gets an envelope pass: pairs need the onsets, the timeline needs the peaks.
    let need: Vec<usize> = (0..tracks.len()).collect();
    let total: u64 = need.iter().map(|&i| tracks[i].segments.iter().map(|s| s.len).sum::<u64>()).sum();
    let done = AtomicU64::new(0);
    let env_results = {
        let tracks = &tracks;
        let done = &done;
        run_parallel(
            &need,
            &mut || progress(&SyncProgress { stage: "envelope", done: done.load(Ordering::Relaxed), total, text: t(Msg::ProgressEnvelope).into() }),
            &|i: &usize| envelope(&tracks[*i], cancel, done),
        )
    };
    if cancel.load(Ordering::SeqCst) {
        return Err(i18n::cancelled());
    }
    let mut envs: Vec<Option<Env>> = (0..tracks.len()).map(|_| None).collect();
    for (k, r) in env_results.into_iter().enumerate() {
        match r {
            Some(Ok(e)) => envs[need[k]] = Some(e),
            Some(Err(e)) => return Err(e),
            None => return Err(t(Msg::AnalysisFailed).into()),
        }
    }

    let pairs_done = AtomicU64::new(0);
    let n_pairs = cands.len() as u64;
    let pair_results = {
        let (tracks, envs, pairs_done) = (&tracks, &envs, &pairs_done);
        run_parallel(
            &cands,
            &mut || progress(&SyncProgress { stage: "pairs", done: pairs_done.load(Ordering::Relaxed), total: n_pairs, text: t(Msg::ProgressPairs).into() }),
            &|&(a, b): &(usize, usize)| {
                let p = analyze_pair(&tracks[a], &tracks[b], envs[a].as_ref().unwrap(), envs[b].as_ref().unwrap(), cancel);
                pairs_done.fetch_add(1, Ordering::Relaxed);
                p
            },
        )
    };
    if cancel.load(Ordering::SeqCst) {
        return Err(i18n::cancelled());
    }
    let pairs: Vec<Pair> = pair_results.into_iter().map(|r| r.ok_or_else(|| t(Msg::AnalysisFailed).to_string())).collect::<Result<_, _>>()?;
    let drafts = plan_items(&tracks, &pairs);
    let places = placements(&tracks, &pairs, &labels);
    let analysis_clips = clips_from_drafts(&tracks, &pairs, &drafts);
    let peaks: Vec<Arc<Peaks>> = envs.iter().map(|e| Arc::new(e.as_ref().map_or_else(Peaks::default, |e| peak_pyramid(&e.peak)))).collect();
    let preview_gain_db: Vec<f32> = envs.iter().map(|e| e.as_ref().map_or(20.0, |e| preview_gain(&e.loud))).collect();
    drop(envs);
    let (clips, edited) = match load_edits(&tracks, &roots[0]) {
        Some(edits) => {
            let merged = merge_edits(&tracks, &analysis_clips, edits);
            let edited = !same_clips(&merged, &analysis_clips);
            (merged, edited)
        }
        None => (analysis_clips.clone(), false),
    };
    let items = items_from_clips(&tracks, &pairs, &places, &labels, &clips);
    Ok(SyncPlan {
        default_out_dir: default_out_dir(&roots).display().to_string(),
        roots: roots.iter().map(|r| r.display().to_string()).collect(),
        source,
        labels,
        tracks,
        pairs,
        places,
        clips,
        edited,
        items,
        ignored,
        preview_gain_db,
        analysis_clips,
        peaks,
    })
}

fn stereo_item(a: &Track, b: &Track, pair: usize, t0: f64, t1: f64, ph: &Phase) -> Draft {
    let frames = ((t1 - t0) * a.info.sample_rate as f64).round() as u64;
    let start_secs = a.start_secs + t0.round() as i64;
    let dur = (t1 - t0).round() as i64;
    let (c0, c1) = (clock(start_secs), clock(start_secs + dur));
    Draft {
        id: 0,
        kind: "stereo",
        name: format!("{}_stereo_L-{}_R-{}.wav", stamp(start_secs, dur), a.label, b.label),
        day: c0.day,
        start: c0.hms,
        end: c1.hms,
        clock0: a.clock0 + t0,
        duration: t1 - t0,
        left: a.id,
        right: Some(b.id),
        pair: Some(pair),
        reference: a.id,
        t0,
        t1,
        spans: vec![Span { pair, other: b.id, t0, t1 }],
        dropout: 0.0,
        silent: Vec::new(),
        hit_share: ph.hit_share,
        msc: ph.msc,
        bytes: wav::output_size(16, frames * 8),
        reason: "gemeinsam",
        start_secs,
    }
}

fn mono_frames(t: &Track, t0: f64, t1: f64) -> (u64, u64) {
    let sr = t.info.sample_rate as f64;
    let f0 = (t0.max(0.0) * sr).round() as u64;
    let f1 = ((t1 * sr).round() as u64).min(t.frames);
    (f0.min(f1), f1)
}

/// Size of a mono output file with frames `f0..f1` of `t`: a bit-exact copy of a mono
/// track; a multi-channel track (e.g. a stereo phone recording) is written as its
/// mono mix in 32-bit float, the same mix its stereo side and the analysis use.
fn mono_bytes(t: &Track, f0: u64, f1: u64) -> u64 {
    if t.info.channels == 1 {
        wav::output_size(t.info.fmt.len(), (f1 - f0) * t.info.block_align as u64)
    } else {
        wav::output_size(16, (f1 - f0) * 4)
    }
}

fn mono_item(t: &Track, t0: f64, t1: f64, reason: &'static str) -> Draft {
    let (f0, f1) = mono_frames(t, t0, t1);
    let start_secs = t.start_secs + t0.round() as i64;
    let dur = (t1 - t0).round() as i64;
    let (c0, c1) = (clock(start_secs), clock(start_secs + dur));
    Draft {
        id: 0,
        kind: "mono",
        name: format!("{}_mono_{}.wav", stamp(start_secs, dur), t.label),
        day: c0.day,
        start: c0.hms,
        end: c1.hms,
        clock0: t.clock0 + t0,
        duration: t1 - t0,
        left: t.id,
        right: None,
        pair: None,
        reference: t.id,
        t0,
        t1,
        spans: Vec::new(),
        dropout: 0.0,
        silent: Vec::new(),
        hit_share: None,
        msc: None,
        bytes: mono_bytes(t, f0, f1),
        reason,
        start_secs,
    }
}

fn plan_items(tracks: &[Track], pairs: &[Pair]) -> Vec<Draft> {
    // Overlap of every analysed pair, per track in its own time.
    let mut overlap: Vec<Vec<(f64, f64, usize)>> = vec![Vec::new(); tracks.len()];
    for (k, p) in pairs.iter().enumerate() {
        let (a, b) = (&tracks[p.a], &tracks[p.b]);
        let (ov0, ov1) = span(p, a, b);
        if ov1 > ov0 {
            overlap[p.a].push((ov0, ov1, k));
            overlap[p.b].push((p.to_b(ov0).max(0.0), p.to_b(ov1).min(b.duration), k));
        }
    }
    let free = |t: usize, s: f64, e: f64, own: usize| overlap[t].iter().all(|&(x0, x1, k)| k == own || x1 <= s || x0 >= e);

    struct Cand<'a> {
        pair: usize,
        t0: f64,
        t1: f64,
        phase: &'a Phase,
    }
    let mut cands = Vec::new();
    for (k, p) in pairs.iter().enumerate() {
        if !p.ok {
            continue;
        }
        let (a, b) = (&tracks[p.a], &tracks[p.b]);
        let (ov0, ov1) = span(p, a, b);
        let (b_start, b_end) = (p.to_a(0.0), p.to_a(b.duration));
        let n = p.phases.len();
        for (i, ph) in p.phases.iter().enumerate() {
            if ph.kind != "together" {
                continue;
            }
            let (mut t0, mut t1) = (ph.start, ph.end);
            // A few seconds where only one recorder runs at the edge join the stereo file.
            if i == 0 {
                let (len, ok) = if b_start > 0.0 { (b_start, free(p.a, 0.0, ov0, k)) } else { (-b_start, free(p.b, 0.0, p.to_b(ov0), k)) };
                if len > 0.0 && len < EDGE_ABSORB_S && ok {
                    t0 = b_start.min(0.0);
                }
            }
            if i + 1 == n {
                let (len, ok) = if b_end < a.duration {
                    (a.duration - b_end, free(p.a, ov1, a.duration, k))
                } else {
                    (b_end - a.duration, free(p.b, p.to_b(ov1), b.duration, k))
                };
                if len > 0.0 && len < EDGE_ABSORB_S && ok {
                    t1 = b_end.max(a.duration);
                }
            }
            cands.push(Cand { pair: k, t0, t1, phase: ph });
        }
    }
    cands.sort_by(|x, y| y.phase.hit_share.unwrap_or(0.0).total_cmp(&x.phase.hit_share.unwrap_or(0.0)));

    let mut claimed: Vec<Vec<(f64, f64)>> = vec![Vec::new(); tracks.len()];
    let hits = |c: &[(f64, f64)], s: f64, e: f64| c.iter().any(|&(x0, x1)| x0 < e - 0.5 && x1 > s + 0.5);
    let mut items = Vec::new();
    for c in cands {
        let p = &pairs[c.pair];
        let (a, b) = (&tracks[p.a], &tracks[p.b]);
        let ia = (c.t0.max(0.0), c.t1.min(a.duration));
        let ib = (p.to_b(c.t0).max(0.0), p.to_b(c.t1).min(b.duration));
        if hits(&claimed[p.a], ia.0, ia.1) || hits(&claimed[p.b], ib.0, ib.1) {
            continue;
        }
        claimed[p.a].push(ia);
        claimed[p.b].push(ib);
        items.push(stereo_item(a, b, c.pair, c.t0, c.t1, c.phase));
    }
    for t in tracks {
        let mut c = claimed[t.id].clone();
        c.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut pos = 0.0f64;
        for (s, e) in c.into_iter().chain(std::iter::once((t.duration, t.duration))) {
            if s - pos >= MIN_PIECE_S {
                let alone = overlap[t.id].iter().all(|&(x0, x1, _)| x1 <= pos + 0.5 || x0 >= s - 0.5);
                items.push(mono_item(t, pos, s, if alone { "allein" } else { "getrennt" }));
            }
            pos = pos.max(e);
        }
    }
    merge_dropouts(tracks, pairs, &mut items);
    extend_alone(tracks, pairs, &mut items);
    items.sort_by(|x, y| x.start_secs.cmp(&y.start_secs).then_with(|| x.kind.cmp(y.kind)).then_with(|| x.name.cmp(&y.name)));
    let mut used: HashSet<String> = HashSet::new();
    for (i, it) in items.iter_mut().enumerate() {
        it.id = i;
        if !used.insert(it.name.clone()) {
            let stem = it.name.trim_end_matches(".wav").to_string();
            let mut n = 2;
            while !used.insert(format!("{stem}_{n}.wav")) {
                n += 1;
            }
            it.name = format!("{stem}_{n}.wav");
        }
    }
    items
}

/// Maps a time on track `from` (which must belong to pair `p`) to the other track of the pair.
fn map_time(p: &Pair, from: usize, t: f64) -> f64 {
    if from == p.a {
        p.to_b(t)
    } else {
        p.to_a(t)
    }
}

/// Interval of a stereo item on track `t`'s own clock, if the item can be expressed there.
fn on_track(it: &Draft, pairs: &[Pair], t: usize) -> Option<(f64, f64)> {
    if it.reference == t {
        return Some((it.t0, it.t1));
    }
    match it.spans.as_slice() {
        [s] if s.other == t => {
            let p = &pairs[s.pair];
            Some((map_time(p, it.reference, it.t0), map_time(p, it.reference, it.t1)))
        }
        _ => None,
    }
}

fn rebase(it: &Draft, pairs: &[Pair], t: usize) -> Draft {
    if it.reference == t {
        return it.clone();
    }
    let s = &it.spans[0];
    let p = &pairs[s.pair];
    let m = |x: f64| map_time(p, it.reference, x);
    let mut out = it.clone();
    out.reference = t;
    out.t0 = m(it.t0);
    out.t1 = m(it.t1);
    out.spans = vec![Span { pair: s.pair, other: it.reference, t0: m(s.t0), t1: m(s.t1) }];
    out
}

fn partner_label(tracks: &[Track], it: &Draft, t: usize) -> String {
    let own = &tracks[t].label;
    let left = &tracks[it.left].label;
    if left != own {
        left.clone()
    } else {
        it.right.map(|r| tracks[r].label.clone()).unwrap_or_default()
    }
}

fn add_silence(it: &mut Draft, label: String, seconds: f64) {
    it.dropout += seconds;
    match it.silent.iter_mut().find(|s| s.label == label) {
        Some(s) => s.seconds += seconds,
        None => it.silent.push(Silence { label, seconds }),
    }
}

/// Name, clock and size of a stereo item after its range changed.
fn refresh_stereo(tracks: &[Track], it: &mut Draft) {
    let r = &tracks[it.reference];
    it.duration = it.t1 - it.t0;
    it.clock0 = r.clock0 + it.t0;
    let dur = it.duration.round() as i64;
    let (c0, c1) = (clock(it.start_secs), clock(it.start_secs + dur));
    let right = it.right.map_or(String::new(), |i| tracks[i].label.clone());
    it.name = format!("{}_stereo_L-{}_R-{}.wav", stamp(it.start_secs, dur), tracks[it.left].label, right);
    it.day = c0.day;
    it.start = c0.hms;
    it.end = c1.hms;
    it.bytes = wav::output_size(16, (it.duration * r.info.sample_rate as f64).round() as u64 * 8);
}

/// Only parallel, different conversations are split. While just one recorder
/// runs before, after or between shared stretches, that audio stays in the
/// adjacent stereo file and the other channel is silent, however long it is.
fn extend_alone(tracks: &[Track], pairs: &[Pair], items: &mut Vec<Draft>) {
    const TOL: f64 = 0.05;
    loop {
        let mut found = None;
        'search: for (g, gap) in items.iter().enumerate() {
            if gap.kind != "mono" || gap.reason != "allein" {
                continue;
            }
            let t = gap.reference;
            for (x, st) in items.iter().enumerate() {
                let Some((s0, s1)) = (st.kind == "stereo").then(|| on_track(st, pairs, t)).flatten() else { continue };
                if (s1.min(tracks[t].duration) - gap.t0).abs() <= TOL {
                    found = Some((g, x, true));
                    break 'search;
                }
                if (s0.max(0.0) - gap.t1).abs() <= TOL {
                    found = Some((g, x, false));
                    break 'search;
                }
            }
        }
        let Some((g, x, after)) = found else { break };
        let gap = items[g].clone();
        let t = gap.reference;
        let st = &items[x];
        let to_ref = |tt: f64| if st.reference == t { tt } else { map_time(&pairs[st.spans[0].pair], t, tt) };
        let (t0, t1) = if after { (st.t0, to_ref(gap.t1)) } else { (to_ref(gap.t0), st.t1) };
        let partner = partner_label(tracks, st, t);
        let len = gap.t1 - gap.t0;
        let it = &mut items[x];
        if !after {
            it.start_secs -= (it.t0 - t0).round() as i64;
        }
        if it.reference != t {
            // The solo track is the second channel: its audio span grows with the file.
            let span = &mut it.spans[0];
            if after {
                span.t1 = t1;
            } else {
                span.t0 = t0;
            }
        }
        it.t0 = t0;
        it.t1 = t1;
        add_silence(it, partner, len);
        refresh_stereo(tracks, it);
        items.remove(g);
    }
}

/// One recorder switched off for a while between two stereo stretches of the
/// same two recorders, the other kept running: that is a dropout, not two
/// different situations. The stretches and the gap become one stereo file on
/// the running recorder's clock, with silence on the missing channel.
fn merge_dropouts(tracks: &[Track], pairs: &[Pair], items: &mut Vec<Draft>) {
    const TOL: f64 = 0.05;
    let labels = |i: &Draft| (tracks[i.left].label.clone(), i.right.map(|r| tracks[r].label.clone()));
    loop {
        let mut found = None;
        'search: for (g, gap) in items.iter().enumerate() {
            if gap.kind != "mono" || gap.reason != "allein" {
                continue;
            }
            let t = gap.reference;
            for (x, s1) in items.iter().enumerate() {
                let Some((_, end1)) = (s1.kind == "stereo").then(|| on_track(s1, pairs, t)).flatten() else { continue };
                if (end1.min(tracks[t].duration) - gap.t0).abs() > TOL {
                    continue;
                }
                for (y, s2) in items.iter().enumerate() {
                    if y == x {
                        continue;
                    }
                    let Some((start2, _)) = (s2.kind == "stereo").then(|| on_track(s2, pairs, t)).flatten() else { continue };
                    if (start2.max(0.0) - gap.t1).abs() <= TOL && labels(s1) == labels(s2) {
                        found = Some((g, x, y));
                        break 'search;
                    }
                }
            }
        }
        let Some((g, x, y)) = found else { break };
        let t = items[g].reference;
        let gap_len = items[g].t1 - items[g].t0;
        let (a, b) = (rebase(&items[x], pairs, t), rebase(&items[y], pairs, t));
        let (da, db) = (a.t1 - a.t0, b.t1 - b.t0);
        let weigh = |u: Option<f64>, v: Option<f64>| match (u, v) {
            (Some(u), Some(v)) => Some((u * da + v * db) / (da + db)),
            (u, v) => u.or(v),
        };
        let mut m = a.clone();
        m.start_secs = items[x].start_secs;
        m.t1 = b.t1;
        m.spans.extend(b.spans.iter().cloned());
        for sil in &b.silent {
            add_silence(&mut m, sil.label.clone(), sil.seconds);
        }
        add_silence(&mut m, partner_label(tracks, &a, t), gap_len);
        m.hit_share = weigh(a.hit_share, b.hit_share);
        m.msc = weigh(a.msc, b.msc);
        refresh_stereo(tracks, &mut m);
        let mut gone = [g, x, y];
        gone.sort_unstable();
        for i in gone.iter().rev() {
            items.remove(*i);
        }
        items.push(m);
    }
}

// ------------------------------------------------------------------ timeline, clips, output rules

const MIN_CLIP_S: f64 = 0.02;
/// Clip edges closer than this (0.1 ms after mapping through the placements) count as the same instant.
const SAME_INSTANT_S: f64 = 1e-4;
const PREVIEW_TARGET_DB: f64 = -20.0;
pub const EDIT_FILE: &str = ".prepareaudio-sync.json";

/// Places every track on its day's timeline. Tracks joined by synchronous pairs
/// form a group; the first recorder's earliest track anchors it on its own
/// clock (so it can be copied bit-exactly) and every other track of the group
/// follows through the measured offsets and drifts. Tracks without a partner
/// sit on their own file-name clock. Edits never change placements.
pub fn placements(tracks: &[Track], pairs: &[Pair], labels: &[String]) -> Vec<Place> {
    let n = tracks.len();
    let rank = |t: usize| labels.iter().position(|l| *l == tracks[t].label).unwrap_or(usize::MAX);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&x, &y| rank(x).cmp(&rank(y)).then(tracks[x].start_secs.cmp(&tracks[y].start_secs)));
    let mut place: Vec<Option<Place>> = vec![None; n];
    for seed in order {
        if place[seed].is_some() {
            continue;
        }
        place[seed] = Some(Place { p: tracks[seed].start_secs as f64, s: 1.0 });
        let mut queue = VecDeque::from([seed]);
        while let Some(x) = queue.pop_front() {
            let px = place[x].expect("placed");
            for pr in pairs.iter().filter(|p| p.ok && (p.a == x || p.b == x)) {
                let other = if pr.a == x { pr.b } else { pr.a };
                if place[other].is_some() {
                    continue;
                }
                // B time = A time · (1 − drift) − offset
                place[other] = Some(if pr.a == x {
                    let s = px.s * (1.0 - pr.drift);
                    Place { p: px.p + pr.offset / s, s }
                } else {
                    Place { p: px.p - pr.offset / px.s, s: px.s / (1.0 - pr.drift) }
                });
                queue.push_back(other);
            }
        }
    }
    place.into_iter().map(|p| p.unwrap_or(Place { p: 0.0, s: 1.0 })).collect()
}

/// Clips of the analysis proposal, taken from the drafted output files.
fn clips_from_drafts(tracks: &[Track], pairs: &[Pair], drafts: &[Draft]) -> Vec<Clip> {
    let mut clips = Vec::new();
    for d in drafts {
        let r = d.reference;
        let mode = if d.kind == "stereo" { ClipMode::Stereo } else { ClipMode::Mono };
        clips.push(Clip { track: r, t0: d.t0, t1: d.t1, mode, deleted: false });
        for sp in &d.spans {
            let p = &pairs[sp.pair];
            let (a, b) = (map_time(p, r, sp.t0), map_time(p, r, sp.t1));
            clips.push(Clip { track: sp.other, t0: a.min(b), t1: a.max(b), mode: ClipMode::Stereo, deleted: false });
        }
    }
    normalize_clips(tracks, clips)
}

/// Clamps clips to their recordings, sorts them and removes overlaps within a track.
pub fn normalize_clips(tracks: &[Track], clips: Vec<Clip>) -> Vec<Clip> {
    let mut clips: Vec<Clip> = clips.into_iter().filter(|c| c.track < tracks.len() && c.t0.is_finite() && c.t1.is_finite()).collect();
    for c in clips.iter_mut() {
        let d = tracks[c.track].duration;
        let (a, b) = (c.t0.clamp(0.0, d), c.t1.clamp(0.0, d));
        c.t0 = a.min(b);
        c.t1 = a.max(b);
    }
    clips.sort_by(|a, b| a.track.cmp(&b.track).then(a.t0.total_cmp(&b.t0)));
    let mut out: Vec<Clip> = Vec::new();
    for mut c in clips {
        if let Some(prev) = out.last() {
            if prev.track == c.track && c.t0 < prev.t1 {
                c.t0 = prev.t1;
            }
        }
        if c.t1 - c.t0 >= MIN_CLIP_S {
            out.push(c);
        }
    }
    out
}

fn same_clips(a: &[Clip], b: &[Clip]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.track == y.track && x.mode == y.mode && x.deleted == y.deleted && (x.t0 - y.t0).abs() < 1e-3 && (x.t1 - y.t1).abs() < 1e-3
        })
}

fn extent(tracks: &[Track], places: &[Place], i: usize) -> (f64, f64) {
    (places[i].to_timeline(0.0), places[i].to_timeline(tracks[i].duration))
}

fn day_clock(t: f64) -> (String, f64) {
    let s = t.floor() as i64;
    let c = clock(s);
    (c.day, c.of_day + (t - s as f64))
}

fn evidence(pairs: &[Pair], places: &[Place], t0: f64, t1: f64) -> (Option<f64>, Option<f64>) {
    let (mut hits, mut active, mut msc_sum, mut msc_n) = (0usize, 0usize, 0.0f64, 0usize);
    for p in pairs.iter().filter(|p| p.ok) {
        for f in &p.frames {
            let t = places[p.a].to_timeline(f.t + FRAME_MS as f64 / ENV_RATE / 2.0);
            if !f.active || t < t0 || t >= t1 {
                continue;
            }
            active += 1;
            hits += f.hit as usize;
            if let Some(m) = f.msc {
                msc_sum += m as f64;
                msc_n += 1;
            }
        }
    }
    ((active > 0).then(|| hits as f64 / active as f64), (msc_n > 0).then(|| msc_sum / msc_n as f64))
}

fn covered(sources: &[Source], t0: f64, t1: f64) -> f64 {
    let mut iv: Vec<(f64, f64)> = sources.iter().map(|s| (s.t0.max(t0), s.t1.min(t1))).filter(|(a, b)| b > a).collect();
    iv.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut sum, mut end) = (0.0, f64::NEG_INFINITY);
    for (a, b) in iv {
        let a = a.max(end);
        if b > a {
            sum += b - a;
            end = b;
        }
    }
    sum
}

/// Output files from the clips. Mono clips become mono files. The first two
/// recorders' stereo clips form stereo files (first recorder left): a file ends
/// where no stereo clip continues, i.e. at a gap or where every running stereo
/// clip has an edge at the same instant — a recorder that drops out keeps the
/// file going, splitting both tracks at one instant splits the file. A single
/// stereo track leaves the other channel silent. Further recorders are mono.
pub fn items_from_clips(tracks: &[Track], pairs: &[Pair], places: &[Place], labels: &[String], clips: &[Clip]) -> Vec<Item> {
    let lane = |t: usize| labels.iter().position(|l| *l == tracks[t].label).unwrap_or(usize::MAX);
    let stereo_possible = labels.len() >= 2;
    let is_stereo = |c: &Clip| c.mode == ClipMode::Stereo && stereo_possible && lane(c.track) < 2;
    let mut items = Vec::new();

    let mut ivs: Vec<(usize, Source)> = clips
        .iter()
        .filter(|c| !c.deleted && is_stereo(c))
        .map(|c| (lane(c.track), Source { track: c.track, t0: places[c.track].to_timeline(c.t0), t1: places[c.track].to_timeline(c.t1) }))
        .collect();
    ivs.sort_by(|a, b| a.1.t0.total_cmp(&b.1.t0));
    let mut groups: Vec<Vec<(usize, Source)>> = Vec::new();
    let mut end = f64::NEG_INFINITY;
    for iv in ivs {
        if groups.is_empty() || iv.1.t0 >= end - SAME_INSTANT_S {
            end = iv.1.t1;
            groups.push(Vec::new());
        } else {
            end = end.max(iv.1.t1);
        }
        groups.last_mut().expect("group").push(iv);
    }
    for g in groups {
        let t0 = g.iter().map(|x| x.1.t0).fold(f64::INFINITY, f64::min);
        let t1 = g.iter().map(|x| x.1.t1).fold(f64::NEG_INFINITY, f64::max);
        let mut sides: [Vec<Source>; 2] = [Vec::new(), Vec::new()];
        for (l, src) in g {
            sides[l].push(src);
        }
        let rep = |l: usize| sides[l].first().map(|s| s.track).or_else(|| tracks.iter().position(|t| t.label == labels[l]));
        let (left, right) = (rep(0).unwrap_or(0), rep(1));
        let rate = tracks[sides[0].first().or(sides[1].first()).map_or(0, |s| s.track)].info.sample_rate;
        let mut silent = Vec::new();
        for l in 0..2 {
            let missing = (t1 - t0) - covered(&sides[l], t0, t1);
            if missing > 0.5 {
                silent.push(Silence { label: labels[l].clone(), seconds: missing });
            }
        }
        let start_secs = t0.round() as i64;
        let dur = (t1 - t0).round() as i64;
        let (c0, c1) = (clock(start_secs), clock(start_secs + dur));
        let (day, clock0) = day_clock(t0);
        let (hit_share, msc) = evidence(pairs, places, t0, t1);
        let frames = ((t1 - t0) * rate as f64).round() as u64;
        let [left_sources, right_sources] = sides;
        items.push(Item {
            id: 0,
            kind: "stereo",
            name: format!("{}_stereo_L-{}_R-{}.wav", stamp(start_secs, dur), labels[0], labels[1]),
            day: if c0.day == day { c0.day } else { day },
            start: c0.hms,
            end: c1.hms,
            clock0,
            duration: t1 - t0,
            left,
            right,
            t0,
            t1,
            left_sources,
            right_sources,
            dropout: silent.iter().map(|s| s.seconds).sum(),
            silent,
            hit_share,
            msc,
            bytes: wav::output_size(16, frames * 8),
            reason: "gemeinsam",
            start_secs,
            rate,
        });
    }

    for c in clips.iter().filter(|c| !c.deleted && !is_stereo(c)) {
        let t = &tracks[c.track];
        let (a, b) = (places[c.track].to_timeline(c.t0), places[c.track].to_timeline(c.t1));
        let others = (0..tracks.len()).any(|o| {
            let (x0, x1) = extent(tracks, places, o);
            tracks[o].label != t.label && x0 < b - 0.5 && x1 > a + 0.5
        });
        let (f0, f1) = mono_frames(t, c.t0, c.t1);
        let start_secs = t.start_secs + c.t0.round() as i64;
        let dur = (c.t1 - c.t0).round() as i64;
        let (c0, c1) = (clock(start_secs), clock(start_secs + dur));
        let (_, clock0) = day_clock(a);
        items.push(Item {
            id: 0,
            kind: "mono",
            name: format!("{}_mono_{}.wav", stamp(start_secs, dur), t.label),
            day: c0.day,
            start: c0.hms,
            end: c1.hms,
            clock0,
            duration: c.t1 - c.t0,
            left: c.track,
            right: None,
            t0: c.t0,
            t1: c.t1,
            left_sources: Vec::new(),
            right_sources: Vec::new(),
            dropout: 0.0,
            silent: Vec::new(),
            hit_share: None,
            msc: None,
            bytes: mono_bytes(t, f0, f1),
            reason: if others { "getrennt" } else { "allein" },
            start_secs,
            rate: t.info.sample_rate,
        });
    }

    items.sort_by(|x, y| x.start_secs.cmp(&y.start_secs).then_with(|| x.kind.cmp(y.kind)).then_with(|| x.name.cmp(&y.name)));
    let mut used: HashSet<String> = HashSet::new();
    for (i, it) in items.iter_mut().enumerate() {
        it.id = i;
        if !used.insert(it.name.clone()) {
            let stem = it.name.trim_end_matches(".wav").to_string();
            let mut n = 2;
            while !used.insert(format!("{stem}_{n}.wav")) {
                n += 1;
            }
            it.name = format!("{stem}_{n}.wav");
        }
    }
    items
}

#[derive(Serialize, Deserialize)]
struct EditFile {
    app: String,
    version: u32,
    tracks: Vec<EditTrack>,
}

#[derive(Serialize, Deserialize)]
struct EditTrack {
    name: String,
    start_secs: i64,
    frames: u64,
    clips: Vec<EditClip>,
}

#[derive(Serialize, Deserialize)]
struct EditClip {
    t0: f64,
    t1: f64,
    mode: ClipMode,
    #[serde(default)]
    deleted: bool,
}

fn load_edits(tracks: &[Track], root: &Path) -> Option<HashMap<usize, Vec<Clip>>> {
    let text = fs::read_to_string(root.join(EDIT_FILE)).ok()?;
    let file: EditFile = serde_json::from_str(&text).ok()?;
    let mut out = HashMap::new();
    for et in file.tracks {
        if let Some(t) = tracks.iter().find(|t| t.name == et.name && t.start_secs == et.start_secs && t.frames == et.frames) {
            let clips = et.clips.into_iter().map(|c| Clip { track: t.id, t0: c.t0, t1: c.t1, mode: c.mode, deleted: c.deleted }).collect();
            out.insert(t.id, clips);
        }
    }
    (!out.is_empty()).then_some(out)
}

fn merge_edits(tracks: &[Track], analysis: &[Clip], edits: HashMap<usize, Vec<Clip>>) -> Vec<Clip> {
    let mut clips: Vec<Clip> = analysis.iter().filter(|c| !edits.contains_key(&c.track)).cloned().collect();
    for (_, v) in edits {
        clips.extend(v);
    }
    normalize_clips(tracks, clips)
}

fn save_edits(plan: &SyncPlan) -> Result<(), String> {
    let path = Path::new(&plan.roots[0]).join(EDIT_FILE);
    if !plan.edited {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    let file = EditFile {
        app: "PrepareAudio".into(),
        version: 1,
        tracks: plan
            .tracks
            .iter()
            .map(|t| EditTrack {
                name: t.name.clone(),
                start_secs: t.start_secs,
                frames: t.frames,
                clips: plan.clips.iter().filter(|c| c.track == t.id).map(|c| EditClip { t0: c.t0, t1: c.t1, mode: c.mode, deleted: c.deleted }).collect(),
            })
            .collect(),
    };
    let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    let tmp = path.with_file_name(format!("{EDIT_FILE}.tmp"));
    fs::write(&tmp, text).map_err(|e| tf(Msg::SaveEditFailed, &[("e", &e)]))?;
    fs::rename(&tmp, &path).map_err(|e| tf(Msg::SaveEditFailed, &[("e", &e)]))
}

/// Takes the edited clips, recomputes the output files and saves the edit (or
/// removes the edit file when the clips equal the analysis proposal again).
pub fn apply_clips(plan: &mut SyncPlan, clips: Vec<Clip>) -> Result<(), String> {
    plan.clips = normalize_clips(&plan.tracks, clips);
    plan.items = items_from_clips(&plan.tracks, &plan.pairs, &plan.places, &plan.labels, &plan.clips);
    plan.edited = !same_clips(&plan.clips, &plan.analysis_clips);
    save_edits(plan)
}

pub fn reset_clips(plan: &mut SyncPlan) -> Result<(), String> {
    let proposal = plan.analysis_clips.clone();
    apply_clips(plan, proposal)
}

fn peak_pyramid(ms: &[u8]) -> Peaks {
    let mut levels = vec![ms.to_vec()];
    for _ in 0..3 {
        let next: Vec<u8> = levels.last().expect("level").chunks(10).map(|c| c.iter().copied().max().unwrap_or(0)).collect();
        levels.push(next);
    }
    Peaks { levels }
}

/// Peaks of `track` between two track seconds, one value per bucket.
pub fn peaks(plan: &SyncPlan, track: usize, t0: f64, t1: f64, buckets: usize) -> Vec<u8> {
    let Some(pk) = plan.peaks.get(track) else { return Vec::new() };
    if pk.levels.is_empty() || !(t1 > t0) {
        return vec![0; buckets.min(20_000)];
    }
    let buckets = buckets.clamp(1, 20_000);
    let per_bucket_ms = (t1 - t0) * 1000.0 / buckets as f64;
    let (mut level, mut bin_ms) = (0usize, 1.0f64);
    while level + 1 < pk.levels.len() && bin_ms * 10.0 <= per_bucket_ms {
        level += 1;
        bin_ms *= 10.0;
    }
    let data = &pk.levels[level];
    (0..buckets)
        .map(|i| {
            let a = ((t0 * 1000.0 + i as f64 * per_bucket_ms) / bin_ms).floor().max(0.0) as usize;
            let b = (((t0 * 1000.0 + (i + 1) as f64 * per_bucket_ms) / bin_ms).ceil().max(0.0) as usize).min(data.len());
            if a >= b { 0 } else { data[a..b].iter().copied().max().unwrap_or(0) }
        })
        .collect()
}

/// Gain for previews: loud passages (90th percentile of 400-ms blocks) to about −20 dB.
fn preview_gain(loud: &[f32]) -> f32 {
    let mut blocks: Vec<f64> = loud.chunks_exact(400).map(|c| c.iter().map(|&x| x as f64).sum::<f64>() / 400.0).filter(|&e| e > 0.0).collect();
    if blocks.is_empty() {
        return 20.0;
    }
    let k = ((blocks.len() as f64 * 0.9) as usize).min(blocks.len() - 1);
    blocks.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
    (PREVIEW_TARGET_DB - 10.0 * blocks[k].log10()).clamp(0.0, 40.0) as f32
}

/// Fills `dst` with `track` for frames `from..to` of the timeline sampled at `sr`
/// (frame k is timeline second k / sr). On the raster (clock rate 1, start on a
/// whole sample, same rate) the samples are copied bit-exactly, otherwise cubic.
pub(crate) fn render_source(tracks: &[Track], places: &[Place], track: usize, sr: f64, from: i64, to: i64, dst: &mut [f32]) -> io::Result<()> {
    let t = &tracks[track];
    let pl = places[track];
    let tsr = t.info.sample_rate as f64;
    let kp = pl.p * sr;
    let cnt = (to - from).max(0) as usize;
    if cnt == 0 {
        return Ok(());
    }
    if tsr == sr && pl.s == 1.0 && (kp - kp.round()).abs() < 1e-6 {
        let v = read_mono(t, from - kp.round() as i64, cnt)?;
        dst[..cnt].copy_from_slice(&v);
        return Ok(());
    }
    let pos = |k: i64| ((k as f64 - kp) / sr) * pl.s * tsr;
    let first = pos(from).floor() as i64 - 1;
    let last = pos(to - 1).floor() as i64 + 2;
    let src = read_mono(t, first, (last - first + 1) as usize)?;
    for (j, d) in dst[..cnt].iter_mut().enumerate() {
        let x = pos(from + j as i64);
        let i = x.floor();
        let idx = (i as i64 - first) as usize;
        *d = catmull(src[idx - 1], src[idx], src[idx + 1], src[idx + 2], (x - i) as f32);
    }
    Ok(())
}

// ------------------------------------------------------------------ writing

enum WErr {
    Cancelled,
    Io(String),
}

impl From<io::Error> for WErr {
    fn from(e: io::Error) -> Self {
        WErr::Io(e.to_string())
    }
}

fn float_stereo_fmt(sr: u32) -> Vec<u8> {
    let mut f = Vec::with_capacity(16);
    f.extend_from_slice(&3u16.to_le_bytes());
    f.extend_from_slice(&2u16.to_le_bytes());
    f.extend_from_slice(&sr.to_le_bytes());
    f.extend_from_slice(&(sr * 8).to_le_bytes());
    f.extend_from_slice(&8u16.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f
}

#[inline]
fn catmull(p0: f32, p1: f32, p2: f32, p3: f32, x: f32) -> f32 {
    p1 + 0.5 * x * (p2 - p0 + x * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + x * (3.0 * (p1 - p2) + p3 - p0)))
}

fn write_mono<W: Write>(t: &Track, it: &Item, out: &mut W, cancel: &AtomicBool, report: &mut dyn FnMut(u64)) -> Result<(), WErr> {
    let ba = t.info.block_align as u64;
    let (f0, f1) = mono_frames(t, it.t0, it.t1);
    if t.info.channels > 1 {
        return write_mono_mix(t, f0, f1, out, cancel, report);
    }
    let data = (f1 - f0) * ba;
    wav::write_header(out, &t.info.fmt, data, f1 - f0, wav::RIFF_LIMIT)?;
    let mut buf = vec![0u8; 4 << 20];
    let (mut frame, mut seg0, mut done) = (f0, 0u64, 0u64);
    for seg in &t.segments {
        let seg_end = seg0 + seg.len / ba;
        if frame < seg_end && frame < f1 {
            let to = f1.min(seg_end);
            let mut src = File::open(&seg.path)?;
            src.seek(SeekFrom::Start(seg.offset + (frame - seg0) * ba))?;
            let mut remaining = (to - frame) * ba;
            while remaining > 0 {
                if cancel.load(Ordering::Relaxed) {
                    return Err(WErr::Cancelled);
                }
                let n = remaining.min(buf.len() as u64) as usize;
                src.read_exact(&mut buf[..n])?;
                out.write_all(&buf[..n])?;
                remaining -= n as u64;
                done += n as u64;
                report(done);
            }
            frame = to;
        }
        seg0 = seg_end;
    }
    if data % 2 == 1 {
        out.write_all(&[0])?;
    }
    Ok(())
}

/// Mono mix (mean of the channels, as in `read_mono`) of a multi-channel track as 32-bit float.
fn write_mono_mix<W: Write>(t: &Track, f0: u64, f1: u64, out: &mut W, cancel: &AtomicBool, report: &mut dyn FnMut(u64)) -> Result<(), WErr> {
    let n = f1 - f0;
    wav::write_header(out, &decode::float_fmt(1, t.info.sample_rate), n * 4, n, wav::RIFF_LIMIT)?;
    let block = 1u64 << 18;
    let mut bytes = Vec::with_capacity(block as usize * 4);
    let mut m = 0u64;
    while m < n {
        if cancel.load(Ordering::Relaxed) {
            return Err(WErr::Cancelled);
        }
        let len = (n - m).min(block);
        let v = read_mono(t, (f0 + m) as i64, len as usize)?;
        bytes.clear();
        bytes.extend(v.iter().flat_map(|x| x.to_le_bytes()));
        out.write_all(&bytes)?;
        m += len;
        report(m * 4);
    }
    Ok(())
}

/// Output raster = the day's timeline at the item's sample rate; each channel is
/// the list of its sources (see `render_source`), silence elsewhere.
fn write_stereo<W: Write>(plan: &SyncPlan, it: &Item, out: &mut W, cancel: &AtomicBool, report: &mut dyn FnMut(u64)) -> Result<(), WErr> {
    let sr = it.rate as f64;
    let k0 = (it.t0 * sr).round() as i64;
    let n = ((it.t1 - it.t0) * sr).round() as u64;
    wav::write_header(out, &float_stereo_fmt(it.rate), n * 8, n, wav::RIFF_LIMIT)?;
    let block = it.rate as u64;
    let mut bytes = Vec::with_capacity(block as usize * 8);
    let mut m = 0u64;
    while m < n {
        if cancel.load(Ordering::Relaxed) {
            return Err(WErr::Cancelled);
        }
        let len = (n - m).min(block) as usize;
        let ks = k0 + m as i64;
        let mut chans = [vec![0f32; len], vec![0f32; len]];
        for (ci, sources) in [&it.left_sources, &it.right_sources].into_iter().enumerate() {
            for src in sources {
                let (s0, s1) = ((src.t0 * sr).round() as i64, (src.t1 * sr).round() as i64);
                let (from, to) = (ks.max(s0), (ks + len as i64).min(s1));
                if to > from {
                    render_source(&plan.tracks, &plan.places, src.track, sr, from, to, &mut chans[ci][(from - ks) as usize..(to - ks) as usize])?;
                }
            }
        }
        bytes.clear();
        for j in 0..len {
            bytes.extend_from_slice(&chans[0][j].to_le_bytes());
            bytes.extend_from_slice(&chans[1][j].to_le_bytes());
        }
        out.write_all(&bytes)?;
        m += len as u64;
        report(m * 8);
    }
    Ok(())
}

fn write_item(plan: &SyncPlan, it: &Item, target: &Path, cancel: &AtomicBool, report: &mut dyn FnMut(u64)) -> Result<(), WErr> {
    let file_name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let partial = target.with_file_name(format!("{file_name}.part"));
    let result = write_item_into(plan, it, &partial, cancel, report).and_then(|()| {
        if target.exists() {
            return Err(WErr::Io(tf(Msg::ExistsMeanwhile, &[("path", &target.display())])));
        }
        fs::rename(&partial, target).map_err(WErr::from)
    });
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

fn write_item_into(plan: &SyncPlan, it: &Item, path: &Path, cancel: &AtomicBool, report: &mut dyn FnMut(u64)) -> Result<(), WErr> {
    let mut out = io::BufWriter::with_capacity(4 << 20, File::create(path)?);
    if it.kind == "stereo" {
        write_stereo(plan, it, &mut out, cancel, report)?;
    } else {
        write_mono(&plan.tracks[it.left], it, &mut out, cancel, report)?;
    }
    let file = out.into_inner().map_err(|e| WErr::Io(e.to_string()))?;
    file.sync_all()?;
    drop(file);
    let info = wav::read_info(path)?;
    if info.repaired || info.file_size != it.bytes {
        return Err(WErr::Io(t(Msg::VerifyFailed).into()));
    }
    Ok(())
}

fn resolve(out_dir: &Path, it: &Item, reserved: &mut HashSet<PathBuf>) -> Option<(PathBuf, bool)> {
    let stem = it.name.trim_end_matches(".wav");
    for n in 1..1000 {
        let path = out_dir.join(if n == 1 { format!("{stem}.wav") } else { format!("{stem}_{n}.wav") });
        if reserved.contains(&path) {
            continue;
        }
        if !path.exists() {
            reserved.insert(path.clone());
            return Some((path, false));
        }
        let same = fs::metadata(&path).map_or(false, |m| m.len() == it.bytes) && wav::read_info(&path).map_or(false, |i| !i.repaired);
        if same {
            reserved.insert(path.clone());
            return Some((path, true));
        }
    }
    None
}

pub fn write<F: FnMut(&Progress)>(plan: &SyncPlan, ids: &[usize], out_dir: &Path, cancel: &AtomicBool, mut progress: F) -> Result<Summary, String> {
    fs::create_dir_all(out_dir).map_err(|e| tf(Msg::CannotCreateDir, &[("path", &out_dir.display()), ("e", &e)]))?;
    let mut outcomes = Vec::new();
    let mut todo: Vec<(&Item, PathBuf)> = Vec::new();
    let mut reserved = HashSet::new();
    for it in plan.items.iter().filter(|i| ids.contains(&i.id)) {
        match resolve(out_dir, it, &mut reserved) {
            Some((p, true)) => outcomes.push(Outcome { id: it.id, status: Status::Existing, path: Some(p.display().to_string()), message: None }),
            Some((p, false)) => todo.push((it, p)),
            None => outcomes.push(Outcome { id: it.id, status: Status::Failed, path: None, message: Some(t(Msg::NoFreeName).into()) }),
        }
    }
    let total: u64 = todo.iter().map(|(i, _)| i.bytes).sum();
    if let Some(free) = available_bytes(out_dir) {
        if total > 0 && free < total + (64 << 20) {
            return Err(low_space(total, free));
        }
    }
    let count = todo.len();
    let (mut done, mut cancelled) = (0u64, false);
    for (index, (it, target)) in todo.iter().enumerate() {
        if cancelled || cancel.load(Ordering::SeqCst) {
            cancelled = true;
            outcomes.push(Outcome { id: it.id, status: Status::Cancelled, path: None, message: None });
            continue;
        }
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let base = done;
        let result = {
            let mut report = |d: u64| progress(&Progress { index, count, id: it.id, name: name.clone(), done: base + d, total, milestone: false, active: Vec::new() });
            report(0);
            write_item(plan, it, target, cancel, &mut report)
        };
        progress(&Progress { index, count, id: it.id, name: name.clone(), done: base + it.bytes, total, milestone: true, active: Vec::new() });
        done += it.bytes;
        outcomes.push(match result {
            Ok(()) => Outcome { id: it.id, status: Status::Written, path: Some(target.display().to_string()), message: None },
            Err(WErr::Cancelled) => {
                cancelled = true;
                Outcome { id: it.id, status: Status::Cancelled, path: None, message: None }
            }
            Err(WErr::Io(msg)) => Outcome { id: it.id, status: Status::Failed, path: None, message: Some(msg) },
        });
    }
    outcomes.sort_by_key(|o| o.id);
    Ok(Summary { out_dir: out_dir.display().to_string(), outcomes, cancelled })
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// Speech-like signal: noise bursts of random length and loudness between pauses.
    fn talk(seed: u64, sr: u32, frames: usize) -> Vec<f32> {
        let mut rng = Lcg(seed);
        let mut out = vec![0f32; frames];
        let mut i = 0;
        while i < frames {
            let burst = ((0.08 + 0.35 * rng.next()) * sr as f64) as usize;
            let amp = 0.05 + 0.3 * rng.next();
            for v in out.iter_mut().skip(i).take(burst) {
                *v = (amp * (rng.next() * 2.0 - 1.0)) as f32;
            }
            i += burst + ((0.05 + 0.5 * rng.next()) * sr as f64) as usize;
        }
        out
    }

    #[test]
    fn xcorr_matches_brute_force() {
        let a: Vec<f64> = (0..37).map(|i| ((i * 7919) % 23) as f64 - 11.0).collect();
        let b: Vec<f64> = (0..20).map(|i| ((i * 104_729) % 17) as f64 - 8.0).collect();
        let (c, first) = xcorr(&a, &b);
        assert_eq!((first, c.len()), (-19, 56));
        for (i, v) in c.iter().enumerate() {
            let lag = first + i as isize;
            let s: f64 = (0..b.len() as isize)
                .filter(|j| (0..a.len() as isize).contains(&(j + lag)))
                .map(|j| b[j as usize] * a[(j + lag) as usize])
                .sum();
            assert!((v - s).abs() < 1e-6, "lag {lag}: {v} vs {s}");
        }
    }

    #[test]
    fn parses_track_names() {
        let (t, label) = parse_track_name("260906_S112326-E122449_D010123_4.wav").unwrap();
        assert_eq!(label, "4");
        assert_eq!(civil_from_secs(t), (2026, 9, 6, 11, 23, 26));
        assert_eq!(parse_track_name("260906_S173229-E181432_D004203_zürich-5.wav").unwrap().1, "zürich-5");
        assert!(parse_track_name("DJI_01_20260906_092321.WAV").is_none());
        assert!(is_output_label("stereo_L-4_R-5"));
    }

    #[test]
    fn coherence_separates_shared_from_independent() {
        let sr = 3000.0;
        let s = talk(5, 3000, 60_000);
        let q = talk(6, 3000, 60_000);
        let delayed: Vec<f32> = (0..60_000).map(|i| if i >= 2 { 0.4 * s[i - 2] } else { 0.0 }).collect();
        assert!(msc(&s, &delayed, sr).unwrap() > 0.8);
        assert!(msc(&s, &q, sr).unwrap() < 0.05);
    }

    fn lr_of(path: &Path) -> Vec<(f32, f32)> {
        let info = wav::read_info(path).unwrap();
        let raw = wav::read_at(path, info.data_offset, info.data_len).unwrap();
        raw.chunks_exact(8).map(|c| (f32::from_le_bytes(c[0..4].try_into().unwrap()), f32::from_le_bytes(c[4..8].try_into().unwrap()))).collect()
    }

    fn out_path(sum: &Summary, id: usize) -> PathBuf {
        PathBuf::from(sum.outcomes.iter().find(|o| o.id == id).unwrap().path.clone().unwrap())
    }

    /// Integer lag (−8…8 samples) at which a channel matches `src` best over one second at 8 kHz.
    fn best_lag(lr: &[(f32, f32)], left: bool, k0: usize, src: &dyn Fn(usize) -> f32) -> i64 {
        (-8i64..=8)
            .max_by(|&x, &y| {
                let score = |d: i64| {
                    (k0..k0 + 8000).map(|k| (if left { lr[k].0 } else { lr[k].1 }) as f64 * src((k as i64 + d) as usize) as f64).sum::<f64>()
                };
                score(x).total_cmp(&score(y))
            })
            .unwrap()
    }

    /// Recorder 5 runs through; recorder 4 is off between 300 s and 500 s of
    /// 5's clock. Same situation throughout, so one stereo file results with
    /// silence on the left during the dropout. Recorder 4's first track anchors
    /// the timeline and is copied bit-exactly.
    #[test]
    fn dropout_between_stereo_stretches_stays_one_stereo_file() {
        let dir = tempdir("dropout");
        let sr = 8000u32;
        let s = talk(11, sr, 900 * sr as usize);
        let mut floor = Lcg(21);
        let five: Vec<f32> = (0..900 * sr as usize).map(|k| 0.5 * s[k] + 0.002 * (floor.next() as f32 - 0.5)).collect();
        let lead = talk(12, sr, 2 * sr as usize);
        let a4: Vec<f32> = (0..300 * sr as usize)
            .map(|g| if g < 2 * sr as usize { lead[g] } else { s[g - 2 * sr as usize] } + 0.002 * (floor.next() as f32 - 0.5))
            .collect();
        let b4: Vec<f32> = (0..400 * sr as usize).map(|k| s[k + 500 * sr as usize] + 0.002 * (floor.next() as f32 - 0.5)).collect();
        write_float_wav(&dir.join("tracks/260101_S100000-E100500_D000500_4.wav"), sr, &a4);
        write_float_wav(&dir.join("tracks/260101_S100002-E101502_D001500_5.wav"), sr, &five);
        write_float_wav(&dir.join("tracks/260101_S100822-E101502_D000640_4.wav"), sr, &b4);

        let cancel = AtomicBool::new(false);
        let plan = analyze(&[dir.join("tracks")], &cancel, &mut |_| {}).unwrap();
        assert_eq!(plan.pairs.iter().filter(|p| p.ok).count(), 2);
        assert_eq!(plan.items.len(), 1, "{:#?}", plan.items.iter().map(|i| (i.kind, i.reason, i.t0, i.t1)).collect::<Vec<_>>());
        let it = &plan.items[0];
        let p4 = plan.places[0];
        assert_eq!((it.kind, it.left_sources.len(), it.right_sources.len()), ("stereo", 2, 1));
        assert!((it.t0 - p4.p).abs() < 0.01 && (it.t1 - p4.p - 902.0).abs() < 0.05, "{} {}", it.t0 - p4.p, it.t1 - p4.p);
        let silent = |l: &str| it.silent.iter().find(|x| x.label == l).map_or(0.0, |x| x.seconds);
        assert!((silent("4") - 202.0).abs() < 1.0 && (silent("5") - 2.0).abs() < 0.5, "{:?}", it.silent);
        assert!(it.name.ends_with("_stereo_L-4_R-5.wav"), "{}", it.name);

        let sum = write(&plan, &[it.id], &dir.join("sync"), &cancel, |_| {}).unwrap();
        let lr = lr_of(&out_path(&sum, it.id));
        let lead = 2 * sr as usize;
        assert!((lr.len() as i64 - 902 * sr as i64).abs() <= 2, "{}", lr.len());
        for k in [0usize, 5_000, 1_000_000, 2_000_000] {
            assert_eq!(lr[k].0, a4[k], "left is recorder 4's first track bit-exactly");
        }
        let gap = (lead + 310 * sr as usize)..(lead + 490 * sr as usize);
        assert!(lr[gap].iter().all(|v| v.0 == 0.0), "left channel silent while recorder 4 was off");
        assert_eq!(best_lag(&lr, false, lead + 100 * sr as usize, &|k| five[k - lead]), 0, "right follows 5");
        assert_eq!(best_lag(&lr, true, lead + 600 * sr as usize, &|k| b4[k - lead - 500 * sr as usize]), 0, "left follows 4b");
    }

    /// A merged track for recorder 4 plus raw DJI parts for 4 and 5: the track
    /// replaces 4's parts, recorder 5 comes straight from its part.
    #[test]
    fn mixes_merged_tracks_and_raw_parts() {
        let dir = tempdir("mixed");
        let sr = 8000u32;
        let four = talk(41, sr, 70 * sr as usize);
        write_float_wav(&dir.join("4/DJI_01_20260101_100000.WAV"), sr, &four);
        write_float_wav(&dir.join("5/DJI_01_20260101_100002.WAV"), sr, &talk(42, sr, 70 * sr as usize));
        write_float_wav(&dir.join("tracks/260101_S100000-E100110_D000110_4.wav"), sr, &four);
        write_float_wav(&dir.join("sync/260101_S100000-E100110_D000110_mono_4.wav"), sr, &four);
        let loaded = load(&[dir.clone()]).unwrap();
        assert_eq!(loaded.source, "mixed");
        let mut got: Vec<(String, bool)> = loaded.tracks.iter().map(|t| (t.label.clone(), t.path.ends_with(".wav"))).collect();
        got.sort();
        assert_eq!(got, [("4".to_string(), true), ("5".to_string(), false)]);
        assert!(loaded.ignored.is_empty(), "{:?}", loaded.ignored);
    }

    /// Raw chunks of any recorder: a chain without a name pattern (bext TimeReference)
    /// becomes one track, a single WAV a track of its own; nothing is ignored.
    #[test]
    fn raw_chunks_with_any_names_become_tracks() {
        let dir = tempdir("anyraw");
        let sr = 8000u32;
        let s = talk(51, sr, 80 * sr as usize);
        let tr0 = 10 * 3600 * sr as u64;
        let half = 40 * sr as usize;
        write_float_bwf(&dir.join("rec/take-a.wav"), sr, &s[..half], "2026-01-01", "10:00:00", tr0);
        write_float_bwf(&dir.join("rec/take-b.wav"), sr, &s[half..], "2026-01-01", "10:00:40", tr0 + half as u64);
        write_float_wav(&dir.join("other/interview 260101_110000.wav"), sr, &talk(52, sr, 30 * sr as usize));
        let loaded = load(&[dir.clone()]).unwrap();
        assert_eq!(loaded.source, "chunks");
        assert!(loaded.ignored.is_empty(), "{:?}", loaded.ignored);
        let mut got: Vec<(String, usize, i64)> = loaded.tracks.iter().map(|t| (t.label.clone(), t.parts, t.start_secs)).collect();
        got.sort();
        let day = days_from_civil(2026, 1, 1) * 86400;
        assert_eq!(got, [("other".to_string(), 1, day + 11 * 3600), ("rec".to_string(), 2, day + 10 * 3600)]);
    }

    /// Recorder 4 runs 150 s alone, then both record the same talk, then 5 runs 100 s alone.
    fn solo_setup(tag: &str) -> (PathBuf, SyncPlan, Vec<f32>, Vec<f32>, u32) {
        let dir = tempdir(tag);
        let sr = 8000u32;
        let n = |secs: usize| secs * sr as usize;
        let s = talk(31, sr, n(700));
        let mut floor = Lcg(32);
        let four: Vec<f32> = (0..n(600)).map(|g| s[g] + 0.002 * (floor.next() as f32 - 0.5)).collect();
        let five: Vec<f32> = (0..n(550)).map(|k| 0.5 * s[k + n(150)] + 0.002 * (floor.next() as f32 - 0.5)).collect();
        write_float_wav(&dir.join("tracks/260101_S100000-E101000_D001000_4.wav"), sr, &four);
        write_float_wav(&dir.join("tracks/260101_S100230-E101140_D000910_5.wav"), sr, &five);
        let plan = analyze(&[dir.join("tracks")], &AtomicBool::new(false), &mut |_| {}).unwrap();
        (dir, plan, four, five, sr)
    }

    #[test]
    fn long_solo_lead_and_tail_stay_in_the_stereo_file() {
        let (dir, plan, four, _five, sr) = solo_setup("solo");
        let n = |secs: usize| secs * sr as usize;
        assert!(plan.pairs[0].ok);
        assert_eq!(plan.items.len(), 1, "{:#?}", plan.items.iter().map(|i| (i.kind, i.reason, i.t0, i.t1)).collect::<Vec<_>>());
        let it = &plan.items[0];
        let p4 = plan.places[0];
        assert_eq!((it.kind, p4.s, plan.tracks[0].label.as_str()), ("stereo", 1.0, "4"));
        assert!((it.t0 - p4.p).abs() < 0.01 && (it.t1 - p4.p - 700.0).abs() < 0.05, "{} {}", it.t0 - p4.p, it.t1 - p4.p);
        let silent: Vec<(String, i64)> = it.silent.iter().map(|x| (x.label.clone(), x.seconds.round() as i64)).collect();
        assert!(silent.contains(&("5".to_string(), 150)) && silent.contains(&("4".to_string(), 100)), "{silent:?}");
        assert_eq!(it.name, "260101_S100000-E101140_D001140_stereo_L-4_R-5.wav");

        let cancel = AtomicBool::new(false);
        let sum = write(&plan, &[it.id], &dir.join("sync"), &cancel, |_| {}).unwrap();
        let lr = lr_of(&out_path(&sum, it.id));
        assert!((lr.len() as i64 - n(700) as i64).abs() <= 2);
        assert!(lr[..n(149)].iter().all(|v| v.1 == 0.0), "right silent before 5 starts");
        assert!(lr[n(601)..].iter().all(|v| v.0 == 0.0), "left silent after 4 stops");
        assert!(lr[n(601)..n(602)].iter().any(|v| v.1 != 0.0), "right keeps playing after 4 stops");
        for k in [0usize, 777, n(300)] {
            assert_eq!(lr[k].0, four[k]);
        }
    }

    #[test]
    fn placements_follow_pairs_and_invert() {
        let (_dir, plan, ..) = solo_setup("place");
        let (p4, p5) = (plan.places[0], plan.places[1]);
        assert_eq!((p4.p, p4.s), (plan.tracks[0].start_secs as f64, 1.0));
        assert!((p5.p - p4.p - 150.0).abs() < 0.005, "{p5:?}");
        let t = p4.p + 321.25;
        assert!((p5.to_timeline(p5.to_track(t)) - t).abs() < 1e-6);
    }

    #[test]
    fn edits_split_retarget_delete_and_persist() {
        let (dir, mut plan, ..) = solo_setup("edits");
        let p4 = plan.places[0];
        let split = |clips: &[Clip], track: usize, tau: f64| -> Vec<Clip> {
            clips
                .iter()
                .flat_map(|c| {
                    if c.track == track && c.t0 < tau && tau < c.t1 {
                        vec![Clip { t1: tau, ..c.clone() }, Clip { t0: tau, ..c.clone() }]
                    } else {
                        vec![c.clone()]
                    }
                })
                .collect()
        };
        let count = |plan: &SyncPlan, kind: &str| plan.items.iter().filter(|i| i.kind == kind).count();
        let base = plan.clips.clone();
        let (q4, q5) = (plan.places[0], plan.places[1]);
        let mid = p4.p + 300.0;
        let edit_file = dir.join("tracks").join(EDIT_FILE);

        apply_clips(&mut plan, split(&base, 0, q4.to_track(mid))).unwrap();
        assert_eq!((count(&plan, "stereo"), count(&plan, "mono")), (1, 0), "splitting one track keeps one file");
        assert!(plan.edited && edit_file.exists());

        apply_clips(&mut plan, split(&split(&base, 0, q4.to_track(mid)), 1, q5.to_track(mid))).unwrap();
        assert_eq!(count(&plan, "stereo"), 2, "splitting both tracks at one instant splits the file");
        assert!((plan.items[0].t1 - mid).abs() < 0.01 && (plan.items[1].t0 - mid).abs() < 0.01);

        let (a, b) = (q5.to_track(p4.p + 400.0), q5.to_track(p4.p + 500.0));
        let mut m = split(&split(&base, 1, a), 1, b);
        for c in m.iter_mut() {
            if c.track == 1 && (c.t0 - a).abs() < 1e-6 {
                c.mode = ClipMode::Mono;
            }
        }
        apply_clips(&mut plan, m).unwrap();
        assert_eq!((count(&plan, "stereo"), count(&plan, "mono")), (1, 1));
        let st = plan.items.iter().find(|i| i.kind == "stereo").unwrap();
        let right_silent = st.silent.iter().find(|x| x.label == "5").map_or(0.0, |x| x.seconds);
        assert!((right_silent - 250.0).abs() < 1.0, "{:?}", st.silent);
        let mono = plan.items.iter().find(|i| i.kind == "mono").unwrap();
        assert_eq!((mono.reason, plan.tracks[mono.left].label.as_str()), ("getrennt", "5"));
        assert!((mono.duration - 100.0).abs() < 0.01);

        let mut gone = base.clone();
        gone.iter_mut().for_each(|c| c.deleted = true);
        apply_clips(&mut plan, gone).unwrap();
        assert!(plan.items.is_empty(), "deleted clips produce nothing");

        let again = analyze(&[dir.join("tracks")], &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(again.edited && again.items.is_empty(), "the edit survives a new analysis");

        reset_clips(&mut plan).unwrap();
        assert!(!plan.edited && !edit_file.exists());
        assert_eq!((count(&plan, "stereo"), count(&plan, "mono")), (1, 0));
    }

    #[test]
    fn peaks_follow_the_signal() {
        let (_dir, plan, ..) = solo_setup("peaks");
        let d = plan.tracks[0].duration;
        let coarse = peaks(&plan, 0, 0.0, d, 600);
        assert_eq!(coarse.len(), 600);
        assert!(coarse.iter().filter(|&&x| x > 100).count() > 550, "{:?}", &coarse[..20]);
        assert_eq!(peaks(&plan, 0, 10.0, 10.5, 500).len(), 500);
        assert!(peaks(&plan, 0, 5.0, 5.0, 10).iter().all(|&x| x == 0));
        assert!((0.0..=40.0).contains(&plan.preview_gain_db[0]), "{}", plan.preview_gain_db[0]);
    }

    /// Two recorders, B starts 3.5 s later (file names claim 4 s). Same
    /// conversation 0–360 s and 660–960 s, different conversations in between.
    #[test]
    fn finds_offset_phases_and_writes_exact_audio() {
        let dir = tempdir("sync");
        let sr = 8000u32;
        let n = 960 * sr as usize;
        let shift = 28_000usize;
        let s = talk(1, sr, n + 8 * sr as usize);
        let p = talk(2, sr, n);
        let q = talk(3, sr, n);
        let mut floor = Lcg(9);
        let together = |t: f64| t < 360.0 || t >= 660.0;
        let hush = |t: f64| (359.0..361.0).contains(&t) || (659.0..661.0).contains(&t);
        let a: Vec<f32> = (0..n)
            .map(|g| {
                let t = g as f64 / sr as f64;
                let v = if hush(t) { 0.0 } else if together(t) { s[g] } else { p[g] };
                v + 0.002 * (floor.next() as f32 - 0.5)
            })
            .collect();
        let b: Vec<f32> = (0..n)
            .map(|k| {
                let g = k + shift;
                let t = g as f64 / sr as f64;
                let v = if hush(t) { 0.0 } else if together(t) { 0.5 * s[g] } else { q[k] };
                v + 0.002 * (floor.next() as f32 - 0.5)
            })
            .collect();
        write_float_wav(&dir.join("tracks/260101_S100000-E101600_D001600_4.wav"), sr, &a);
        write_float_wav(&dir.join("tracks/260101_S100004-E101604_D001600_5.wav"), sr, &b);

        let cancel = AtomicBool::new(false);
        let plan = analyze(&[dir.join("tracks")], &cancel, &mut |_| {}).unwrap();
        assert_eq!((plan.source, plan.tracks.len(), plan.pairs.len()), ("tracks", 2, 1));
        assert_eq!(plan.default_out_dir, dir.join("sync").display().to_string());
        let pr = &plan.pairs[0];
        assert!(pr.ok, "{:?}", (pr.coarse_z, pr.n_good, pr.n_windows, pr.resid_ms));
        assert!((pr.offset - 3.5).abs() < 0.003, "offset {}", pr.offset);
        let kinds: Vec<(&str, f64, f64)> = pr.phases.iter().map(|p| (p.kind, p.start, p.end)).collect();
        assert_eq!(kinds.iter().map(|k| k.0).collect::<Vec<_>>(), ["together", "apart", "together"], "{kinds:?}");
        assert!((kinds[1].1 - 360.0).abs() <= 3.0 && (kinds[1].2 - 660.0).abs() <= 3.0, "{kinds:?}");
        let apart = &pr.phases[1];
        assert!(apart.hit_share.unwrap() < 0.2 && apart.msc.unwrap() < 0.1, "{apart:?}");
        let tog = &pr.phases[0];
        assert!(tog.hit_share.unwrap() > 0.8 && tog.msc.unwrap() > 0.5, "{tog:?}");

        let stereo: Vec<&Item> = plan.items.iter().filter(|i| i.kind == "stereo").collect();
        let mono: Vec<&Item> = plan.items.iter().filter(|i| i.kind == "mono").collect();
        assert_eq!((stereo.len(), mono.len()), (2, 2), "{:#?}", plan.items);
        let pa = plan.places[0];
        assert!((stereo[0].t0 - pa.p).abs() < 1e-6, "leading seconds of A join the stereo file");
        assert!((stereo[1].t1 - pa.p - 963.5).abs() < 0.01, "trailing seconds of B join the stereo file");

        let out = dir.join("sync");
        let ids: Vec<usize> = plan.items.iter().map(|i| i.id).collect();
        let sum = write(&plan, &ids, &out, &cancel, |_| {}).unwrap();
        assert!(sum.outcomes.iter().all(|o| o.status == Status::Written), "{:?}", sum.outcomes);

        // Stereo: left is A bit-exactly, right is B aligned to within a sample.
        let st = stereo[1];
        let path = PathBuf::from(sum.outcomes.iter().find(|o| o.id == st.id).unwrap().path.clone().unwrap());
        let info = wav::read_info(&path).unwrap();
        assert_eq!((info.channels, info.sample_rate), (2, sr));
        let raw = wav::read_at(&path, info.data_offset, info.data_len).unwrap();
        let lr: Vec<(f32, f32)> =
            raw.chunks_exact(8).map(|c| (f32::from_le_bytes(c[0..4].try_into().unwrap()), f32::from_le_bytes(c[4..8].try_into().unwrap()))).collect();
        let f0 = ((st.t0 - pa.p) * sr as f64).round() as usize;
        for k in [0usize, 777, 100_000, 1_000_000] {
            assert_eq!(lr[k].0, a[f0 + k]);
        }
        let seg = 200_000..208_000usize;
        let best = (-8i64..=8)
            .max_by(|&x, &y| {
                let score = |d: i64| seg.clone().map(|k| lr[k].1 as f64 * b[(f0 + k) as usize - shift + d as usize - 0] as f64).sum::<f64>();
                score(x).total_cmp(&score(y))
            })
            .unwrap();
        assert_eq!(best.abs(), 0, "right channel misaligned by {best} samples");

        // Mono pieces are bit-exact copies.
        let m4 = mono.iter().find(|i| i.left == 0).unwrap();
        let mpath = PathBuf::from(sum.outcomes.iter().find(|o| o.id == m4.id).unwrap().path.clone().unwrap());
        let minfo = wav::read_info(&mpath).unwrap();
        let mraw = wav::read_at(&mpath, minfo.data_offset, minfo.data_len).unwrap();
        let mf0 = (m4.t0 * sr as f64).round() as usize;
        let expect: Vec<u8> = a[mf0..mf0 + mraw.len() / 4].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(mraw, expect);

        // A second run finds everything in place.
        let again = write(&plan, &ids, &out, &cancel, |_| {}).unwrap();
        assert!(again.outcomes.iter().all(|o| o.status == Status::Existing));
    }

    /// Interleaved float samples as a CBR MP3 with LAME tag (encoder delay and padding for gapless decoding).
    fn write_mp3(path: &Path, sr: u32, channels: usize, pcm: &[f32]) {
        use mp3lame_encoder::{Bitrate, Builder, FlushGap, InterleavedPcm, MonoPcm, Quality};
        let mut b = Builder::new().unwrap();
        b.set_num_channels(channels as u8).unwrap();
        b.set_sample_rate(sr).unwrap();
        b.set_brate(Bitrate::Kbps96).unwrap();
        b.set_quality(Quality::Best).unwrap();
        b.set_to_write_vbr_tag(true).unwrap();
        let mut enc = b.build().unwrap();
        let mut out = Vec::new();
        for block in pcm.chunks(channels * 16_384) {
            out.reserve(block.len() * 5 / 4 + 7200);
            if channels == 1 { enc.encode_to_vec(MonoPcm(block), &mut out) } else { enc.encode_to_vec(InterleavedPcm(block), &mut out) }.unwrap();
        }
        out.reserve(7200);
        enc.flush_to_vec::<FlushGap>(&mut out).unwrap();
        let mut tag = Vec::with_capacity(enc.lame_tag_size());
        enc.lame_tag_encode_to_vec(&mut tag).unwrap();
        out[..tag.len()].copy_from_slice(&tag);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, out).unwrap();
    }

    /// Gain of `y` relative to `x` (least squares) and the lag in samples (−8…8) where they match best.
    fn gain_and_lag(y: &dyn Fn(usize) -> f32, x: &dyn Fn(usize) -> f32, k0: usize, n: usize) -> (f64, i64) {
        let score = |d: i64| (k0..k0 + n).map(|k| y(k) as f64 * x((k as i64 + d) as usize) as f64).sum::<f64>();
        let lag = (-8i64..=8).max_by(|&a, &b| score(a).total_cmp(&score(b))).unwrap();
        let energy: f64 = (k0..k0 + n).map(|k| (x((k as i64 + lag) as usize) as f64).powi(2)).sum();
        (score(lag) / energy, lag)
    }

    /// Recorder 4 as a WAV track and a phone recording as stereo MP3 that really starts
    /// 3.5 s later (its name says 4 s). The MP3 is decoded once into the cache, reused,
    /// decoded again after the source changed, and synchronised like any track; its
    /// stereo side and its mono file are the mono mix of both channels.
    #[test]
    fn decodes_compressed_sources_caches_and_syncs_them() {
        let dir = tempdir("mp3");
        let cache = tempdir("mp3-cache");
        let sr = 16_000u32;
        let n = |secs: f64| (secs * sr as f64) as usize;
        let shift = n(3.5);
        let s = talk(61, sr, n(400.0) + shift);
        let mut floor = Lcg(62);
        let four: Vec<f32> = (0..n(400.0)).map(|g| 0.5 * s[g] + 0.002 * (floor.next() as f32 - 0.5)).collect();
        let phone: Vec<f32> = (0..n(400.0)).map(|k| 0.4 * s[k + shift] + 0.002 * (floor.next() as f32 - 0.5)).collect();
        write_float_wav(&dir.join("tracks/260101_S100000-E100640_D000640_4.wav"), sr, &four);
        let mp3 = dir.join("phone/Aufnahme 2026-01-01 10-00-04.mp3");
        write_mp3(&mp3, sr, 2, &phone.iter().flat_map(|&v| [v, 0.6 * v]).collect::<Vec<f32>>());

        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        let (old, recent) = (cache.join("00000000000000000000000000000000.wav"), cache.join("11111111111111111111111111111111.wav"));
        fs::write(&old, b"x").unwrap();
        fs::write(&recent, b"x").unwrap();
        set_mtime(&old, now - 40.0 * 86_400.0);

        let cancel = AtomicBool::new(false);
        let mut stages = Vec::new();
        let mut plan = analyze_in(&[dir.clone()], &cache, &cancel, &mut |p| stages.push(p.stage)).unwrap();
        assert!(!old.exists() && recent.exists(), "only files unused for 30 days leave the cache");
        assert!(stages.contains(&"decode") && stages.contains(&"envelope"), "{stages:?}");
        assert!(plan.ignored.is_empty(), "{:?}", plan.ignored);
        assert_eq!((plan.source, plan.labels.clone()), ("tracks", vec!["4".to_string(), "phone".to_string()]));
        let ph = plan.tracks.iter().find(|t| t.label == "phone").unwrap().clone();
        assert_eq!((ph.decoded_from.as_deref(), ph.name.as_str(), ph.path.as_str()), (Some("MP3"), "Aufnahme 2026-01-01 10-00-04.mp3", mp3.to_str().unwrap()));
        assert_eq!(ph.start_secs, days_from_civil(2026, 1, 1) * 86400 + 10 * 3600 + 4, "start time from the file name");
        assert_eq!(ph.info.channels, 2);
        assert!((ph.duration - 400.0).abs() < 0.002, "gapless decoding keeps the length: {}", ph.duration);
        let cached = ph.decoded.clone().unwrap();
        assert!(cached.starts_with(&cache) && cached.exists());
        let json = serde_json::to_value(&plan.tracks).unwrap();
        assert!(json.to_string().contains("\"decoded_from\":\"MP3\"") && !json.to_string().contains(cache.to_str().unwrap()));

        let pr = plan.pairs.iter().find(|p| p.ok).expect("pair in sync");
        assert!((pr.offset - 3.5).abs() < 0.003, "offset {}", pr.offset);
        let st = plan.items.iter().find(|i| i.kind == "stereo").expect("stereo item").clone();
        let sum = write(&plan, &[st.id], &dir.join("sync"), &cancel, |_| {}).unwrap();
        assert_eq!(sum.outcomes[0].status, Status::Written, "{:?}", sum.outcomes);
        let lr = lr_of(&out_path(&sum, st.id));
        let (p4, pp) = (plan.places[0], plan.places[ph.id]);
        let a0 = ((st.t0 - p4.p) * sr as f64).round() as i64;
        assert_eq!(lr[n(100.0)].0, four[(a0 + n(100.0) as i64) as usize], "left is recorder 4 bit-exactly");
        // Channel 0 of the decoded copy; the file's right channel is 0.6 × left, so the mean is 0.8 × left.
        let dec = wav::read_info(&cached).unwrap();
        let raw = wav::read_at(&cached, dec.data_offset, dec.data_len).unwrap();
        let left0: Vec<f32> = raw.chunks_exact(8).map(|c| f32::from_le_bytes(c[0..4].try_into().unwrap())).collect();
        drop(raw);
        let at_phone = |j: usize| (pp.to_track(st.t0 + j as f64 / sr as f64) * sr as f64).round() as i64;
        let (g, lag) = gain_and_lag(&|j| lr[j].1, &|j| left0.get(at_phone(j).max(0) as usize).copied().unwrap_or(0.0), n(100.0), n(2.0));
        assert!(lag.abs() <= 1 && (g - 0.8).abs() < 0.03, "right = mono mix of the MP3: gain {g}, lag {lag}");

        // A multi-channel source on the mono side becomes a mono file of the same mix.
        let clips: Vec<Clip> = plan.clips.iter().map(|c| Clip { mode: if c.track == ph.id { ClipMode::Mono } else { c.mode }, ..c.clone() }).collect();
        apply_clips(&mut plan, clips).unwrap();
        let mono = plan.items.iter().find(|i| i.kind == "mono" && i.left == ph.id).expect("mono item of the phone").clone();
        let sum = write(&plan, &[mono.id], &dir.join("sync"), &cancel, |_| {}).unwrap();
        assert_eq!(sum.outcomes[0].status, Status::Written, "{:?}", sum.outcomes);
        let mpath = out_path(&sum, mono.id);
        let info = wav::read_info(&mpath).unwrap();
        assert_eq!((info.channels, info.sample_kind(), info.sample_rate), (1, Some(wav::SampleKind::F32), sr));
        let raw = wav::read_at(&mpath, info.data_offset, info.data_len).unwrap();
        let m: Vec<f32> = raw.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        let m0 = (mono.t0 * sr as f64).round() as usize;
        let (g, lag) = gain_and_lag(&|j| m[j], &|j| left0[m0 + j], n(50.0), n(2.0));
        assert!(lag == 0 && (g - 0.8).abs() < 0.03, "mono file = mono mix: gain {g}, lag {lag}");
        let mix = read_mono(&plan.tracks[ph.id], m0 as i64, m.len()).unwrap();
        assert_eq!(m, mix, "the mono file is exactly the mix the analysis and the stereo side use");

        // Unchanged source: the cached file is used as it is. Changed source: decoded again.
        use std::os::unix::fs::MetadataExt;
        let inode = fs::metadata(&cached).unwrap().ino();
        let mut stages = Vec::new();
        let again = analyze_in(&[dir.clone()], &cache, &cancel, &mut |p| stages.push((p.stage, p.done, p.total))).unwrap();
        let ph2 = again.tracks.iter().find(|t| t.label == "phone").unwrap();
        assert_eq!(ph2.decoded.as_ref(), Some(&cached));
        assert_eq!(fs::metadata(&cached).unwrap().ino(), inode, "the cached file is reused, not decoded again");
        assert!(again.edited, "the edit next to the sources applies to the decoded track too");
        set_mtime(&mp3, now - 3600.0);
        let third = analyze_in(&[dir.clone()], &cache, &cancel, &mut |_| {}).unwrap();
        let ph3 = third.tracks.iter().find(|t| t.label == "phone").unwrap();
        let fresh = ph3.decoded.clone().unwrap();
        assert!(fresh != cached && fresh.exists(), "a changed source is decoded again");
        assert_eq!((ph3.start_secs, ph3.frames), (ph.start_secs, ph.frames));
        assert!(fs::read_dir(&cache).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".part")));
    }

    #[test]
    fn mp4_creation_time_is_read_from_mvhd() {
        let dir = tempdir("mp4-time");
        let p = dir.join("rec.m4a");
        let mut mvhd = vec![0u8; 100];
        mvhd[4..8].copy_from_slice(&((1_767_261_600i64 + 2_082_844_800) as u32).to_be_bytes());
        let boxed = |kind: &[u8], body: &[u8]| [&((body.len() + 8) as u32).to_be_bytes()[..], kind, body].concat();
        let file = [boxed(b"ftyp", b"M4A \0\0\0\0"), boxed(b"mdat", &[1u8; 33]), boxed(b"moov", &[boxed(b"free", &[0; 3]), boxed(b"mvhd", &mvhd)].concat())].concat();
        fs::write(&p, file).unwrap();
        assert_eq!(decode::mp4_creation_time(&p), Some(1_767_261_600));
        assert_eq!(compressed_start(&p, 10.0), Some(1_767_261_600 + scan::local_offset(1_767_261_600)));
    }

    /// Writes a real plan plus overview peaks as JSON for the browser tests of the timeline.
    /// PA_DUMP_INPUTS=dir1:dir2 PA_DUMP_OUT=file cargo test --release --lib dump_plan_for_ui -- --ignored
    #[test]
    #[ignore]
    fn dump_plan_for_ui() {
        let inputs: Vec<PathBuf> = std::env::var("PA_DUMP_INPUTS").unwrap().split(':').map(PathBuf::from).collect();
        let out = std::env::var("PA_DUMP_OUT").unwrap();
        let plan = analyze(&inputs, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let peaks: Vec<Vec<u8>> = plan.tracks.iter().map(|t| super::peaks(&plan, t.id, 0.0, t.duration, 20_000)).collect();
        let v = serde_json::json!({ "plan": plan, "peaks": peaks });
        std::fs::write(out, serde_json::to_string(&v).unwrap()).unwrap();
    }

}
