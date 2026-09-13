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

use crate::merge::{available_bytes, human_bytes, Outcome, Progress, Status, Summary};
use crate::scan::{self, civil_from_secs, days_from_civil, Skipped};
use crate::wav::{self, WavInfo};
use serde::Serialize;
use std::cmp::Ordering as CmpOrdering;
use std::collections::HashSet;
use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub const OUTPUT_DIR_NAME: &str = "sync";
const CANCELLED: &str = "Abgebrochen.";

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
pub struct Item {
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

#[derive(Debug, Clone, Serialize)]
pub struct SyncPlan {
    pub roots: Vec<String>,
    pub default_out_dir: String,
    /// "tracks" (files from the first function) or "chunks" (raw DJI parts).
    pub source: &'static str,
    pub labels: Vec<String>,
    pub tracks: Vec<Track>,
    pub pairs: Vec<Pair>,
    pub items: Vec<Item>,
    pub ignored: Vec<Skipped>,
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
    let b = name.as_bytes();
    if b.len() < 36 || !name.to_ascii_lowercase().ends_with(".wav") {
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
    let label = &name[31..name.len() - 4];
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
            if name != OUTPUT_DIR_NAME {
                walk(&path, depth + 1, out);
            }
        } else if ft.is_file() && is_wav(&path) {
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
    }
}

struct Loaded {
    tracks: Vec<Track>,
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
            if is_wav(input) {
                files.push(input.clone());
            }
        }
    }
    if roots.is_empty() {
        return Err("Keine Ordner oder Dateien angegeben.".into());
    }
    files.sort();
    files.dedup();

    let mut tracks = Vec::new();
    let mut ignored = Vec::new();
    let mut others = Vec::new();
    for f in files {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match parse_track_name(&name) {
            Some((_, label)) if is_output_label(&label) => {}
            Some((start, label)) => match wav::read_info(&f) {
                Ok(info) if info.sample_kind().is_some() && info.data_len > 0 => {
                    let seg = vec![Segment { path: f.clone(), offset: info.data_offset, len: info.data_len }];
                    tracks.push(make_track(name, f.display().to_string(), label, start, info, seg));
                }
                Ok(_) => ignored.push(Skipped { path: f.display().to_string(), reason: "Audioformat wird nicht unterstützt".into() }),
                Err(e) => ignored.push(Skipped { path: f.display().to_string(), reason: format!("nicht lesbar: {e}") }),
            },
            None => others.push(f),
        }
    }
    let from_files = tracks.len();

    // Recordings that exist only as DJI parts (no merged track yet) are used directly.
    let mut from_parts = 0;
    let has_parts = others.iter().any(|f| scan::parse_dji_name(&f.file_name().unwrap_or_default().to_string_lossy()).is_some());
    if has_parts {
        let s = scan::scan(inputs, &scan::Options::default())?;
        ignored.extend(s.duplicates);
        for sk in s.ignored {
            let name = Path::new(&sk.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if parse_track_name(&name).is_none() {
                ignored.push(sk);
            }
        }
        for r in s.recordings {
            let merged = tracks[..from_files]
                .iter()
                .any(|t| t.label == r.label && (t.start_secs - r.start_secs).abs() <= 2 && (t.duration - r.duration).abs() <= 2.0);
            if merged {
                continue;
            }
            let info = r.parts[0].info.clone();
            if info.sample_kind().is_none() {
                ignored.push(Skipped { path: r.parts[0].path.clone(), reason: "Audioformat wird nicht unterstützt".into() });
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
    } else {
        for f in others {
            ignored.push(Skipped { path: f.display().to_string(), reason: "weder Track noch DJI-Aufnahme".into() });
        }
    }
    let source = match (from_files > 0, from_parts > 0) {
        (true, false) => "tracks",
        (false, true) => "chunks",
        (true, true) => "mixed",
        (false, false) => return Err("Keine Tracks und keine DJI-Aufnahmen gefunden.".into()),
    };
    Ok(Loaded { tracks, ignored, roots, source })
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
}

fn envelope(track: &Track, cancel: &AtomicBool, done: &AtomicU64) -> Result<Env, String> {
    let info = &track.info;
    let kind = info.sample_kind().ok_or("Audioformat wird nicht unterstützt")?;
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
    let mut buf = vec![0u8; ba * 65536];
    for seg in &track.segments {
        let mut f = File::open(&seg.path).map_err(|e| format!("{}: {e}", seg.path.display()))?;
        f.seek(SeekFrom::Start(seg.offset)).map_err(|e| e.to_string())?;
        let mut remaining = seg.len - seg.len % ba as u64;
        while remaining > 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
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
                cnt += 1;
                frame += 1;
                if frame == next {
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
    Ok(onset_from_bands(&bands))
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
    Env { onset, loud }
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
        p.note = Some(
            if p.n_windows == 0 { "zu wenig gemeinsame Aufnahmezeit" } else { "keine gemeinsamen Ereignisse mit stabilem Versatz" }
                .to_string(),
        );
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
    progress(&SyncProgress { stage: "load", done: 0, total: 1, text: "Suche Tracks".into() });
    let Loaded { mut tracks, ignored, roots, source } = load(inputs)?;
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

    let mut need: Vec<usize> = cands.iter().flat_map(|&(a, b)| [a, b]).collect();
    need.sort_unstable();
    need.dedup();
    let total: u64 = need.iter().map(|&i| tracks[i].segments.iter().map(|s| s.len).sum::<u64>()).sum();
    let done = AtomicU64::new(0);
    let env_results = {
        let tracks = &tracks;
        let done = &done;
        run_parallel(
            &need,
            &mut || progress(&SyncProgress { stage: "envelope", done: done.load(Ordering::Relaxed), total, text: "Frequenzanalyse der Tracks".into() }),
            &|i: &usize| envelope(&tracks[*i], cancel, done),
        )
    };
    if cancel.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
    }
    let mut envs: Vec<Option<Env>> = (0..tracks.len()).map(|_| None).collect();
    for (k, r) in env_results.into_iter().enumerate() {
        match r {
            Some(Ok(e)) => envs[need[k]] = Some(e),
            Some(Err(e)) => return Err(e),
            None => return Err("Analyse fehlgeschlagen.".into()),
        }
    }

    let pairs_done = AtomicU64::new(0);
    let n_pairs = cands.len() as u64;
    let pair_results = {
        let (tracks, envs, pairs_done) = (&tracks, &envs, &pairs_done);
        run_parallel(
            &cands,
            &mut || progress(&SyncProgress { stage: "pairs", done: pairs_done.load(Ordering::Relaxed), total: n_pairs, text: "Suche gemeinsame Ereignisse".into() }),
            &|&(a, b): &(usize, usize)| {
                let p = analyze_pair(&tracks[a], &tracks[b], envs[a].as_ref().unwrap(), envs[b].as_ref().unwrap(), cancel);
                pairs_done.fetch_add(1, Ordering::Relaxed);
                p
            },
        )
    };
    if cancel.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
    }
    let pairs: Vec<Pair> = pair_results.into_iter().map(|r| r.ok_or_else(|| "Analyse fehlgeschlagen.".to_string())).collect::<Result<_, _>>()?;
    let items = plan_items(&tracks, &pairs);
    Ok(SyncPlan {
        default_out_dir: default_out_dir(&roots).display().to_string(),
        roots: roots.iter().map(|r| r.display().to_string()).collect(),
        source,
        labels,
        tracks,
        pairs,
        items,
        ignored,
    })
}

fn stereo_item(a: &Track, b: &Track, pair: usize, t0: f64, t1: f64, ph: &Phase) -> Item {
    let frames = ((t1 - t0) * a.info.sample_rate as f64).round() as u64;
    let start_secs = a.start_secs + t0.round() as i64;
    let dur = (t1 - t0).round() as i64;
    let (c0, c1) = (clock(start_secs), clock(start_secs + dur));
    Item {
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

fn mono_item(t: &Track, t0: f64, t1: f64, reason: &'static str) -> Item {
    let (f0, f1) = mono_frames(t, t0, t1);
    let start_secs = t.start_secs + t0.round() as i64;
    let dur = (t1 - t0).round() as i64;
    let (c0, c1) = (clock(start_secs), clock(start_secs + dur));
    Item {
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
        bytes: wav::output_size(t.info.fmt.len(), (f1 - f0) * t.info.block_align as u64),
        reason,
        start_secs,
    }
}

fn plan_items(tracks: &[Track], pairs: &[Pair]) -> Vec<Item> {
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
fn on_track(it: &Item, pairs: &[Pair], t: usize) -> Option<(f64, f64)> {
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

fn rebase(it: &Item, pairs: &[Pair], t: usize) -> Item {
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

fn partner_label(tracks: &[Track], it: &Item, t: usize) -> String {
    let own = &tracks[t].label;
    let left = &tracks[it.left].label;
    if left != own {
        left.clone()
    } else {
        it.right.map(|r| tracks[r].label.clone()).unwrap_or_default()
    }
}

fn add_silence(it: &mut Item, label: String, seconds: f64) {
    it.dropout += seconds;
    match it.silent.iter_mut().find(|s| s.label == label) {
        Some(s) => s.seconds += seconds,
        None => it.silent.push(Silence { label, seconds }),
    }
}

/// Name, clock and size of a stereo item after its range changed.
fn refresh_stereo(tracks: &[Track], it: &mut Item) {
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
fn extend_alone(tracks: &[Track], pairs: &[Pair], items: &mut Vec<Item>) {
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
fn merge_dropouts(tracks: &[Track], pairs: &[Pair], items: &mut Vec<Item>) {
    const TOL: f64 = 0.05;
    let labels = |i: &Item| (tracks[i.left].label.clone(), i.right.map(|r| tracks[r].label.clone()));
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

/// The reference track runs through bit-exactly on its side (normally the left
/// recorder). The other channel follows each span via offset and drift of its
/// pair (integer shift when the drift stays below half a sample, otherwise
/// cubic interpolation); outside the spans it is silent.
fn write_stereo<W: Write>(plan: &SyncPlan, it: &Item, out: &mut W, cancel: &AtomicBool, report: &mut dyn FnMut(u64)) -> Result<(), WErr> {
    let r = &plan.tracks[it.reference];
    let ref_left = it.reference == it.left || r.label == plan.tracks[it.left].label;
    let sr = r.info.sample_rate as f64;
    let f0 = (it.t0 * sr).round() as i64;
    let n = ((it.t1 - it.t0) * sr).round() as u64;
    wav::write_header(out, &float_stereo_fmt(r.info.sample_rate), n * 8, n, wav::RIFF_LIMIT)?;
    let lanes: Vec<(&Pair, &Track, i64, i64)> = it
        .spans
        .iter()
        .map(|s| (&plan.pairs[s.pair], &plan.tracks[s.other], (s.t0 * sr).round() as i64, (s.t1 * sr).round() as i64))
        .collect();
    let block = r.info.sample_rate as u64;
    let mut bytes = Vec::with_capacity(block as usize * 8);
    let mut m = 0u64;
    while m < n {
        if cancel.load(Ordering::Relaxed) {
            return Err(WErr::Cancelled);
        }
        let len = (n - m).min(block) as usize;
        let k0 = f0 + m as i64;
        let main = read_mono(r, k0, len)?;
        let mut second = vec![0f32; len];
        for &(p, o, s0, s1) in &lanes {
            let (from, to) = (k0.max(s0), (k0 + len as i64).min(s1));
            if to <= from {
                continue;
            }
            let (cnt, dst) = ((to - from) as usize, (from - k0) as usize);
            let osr = o.info.sample_rate as f64;
            let pos = |k: i64| map_time(p, r.id, k as f64 / sr) * osr;
            let exact = r.info.sample_rate == o.info.sample_rate && (pos(s1) - pos(s0) - (s1 - s0) as f64).abs() < 0.5;
            if exact {
                let shift = pos(s0).round() as i64 - s0;
                second[dst..dst + cnt].copy_from_slice(&read_mono(o, from + shift, cnt)?);
            } else {
                let first = pos(from).floor() as i64 - 1;
                let last = pos(to - 1).floor() as i64 + 2;
                let src = read_mono(o, first, (last - first + 1) as usize)?;
                for j in 0..cnt {
                    let x = pos(from + j as i64);
                    let i = x.floor();
                    let idx = (i as i64 - first) as usize;
                    second[dst + j] = catmull(src[idx - 1], src[idx], src[idx + 1], src[idx + 2], (x - i) as f32);
                }
            }
        }
        bytes.clear();
        for j in 0..len {
            let (l, rr) = if ref_left { (main[j], second[j]) } else { (second[j], main[j]) };
            bytes.extend_from_slice(&l.to_le_bytes());
            bytes.extend_from_slice(&rr.to_le_bytes());
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
            return Err(WErr::Io(format!("{} existiert inzwischen bereits", target.display())));
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
        return Err(WErr::Io("Kontrolle der geschriebenen Datei fehlgeschlagen".into()));
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
    fs::create_dir_all(out_dir).map_err(|e| format!("Zielordner {} kann nicht angelegt werden: {e}", out_dir.display()))?;
    let mut outcomes = Vec::new();
    let mut todo: Vec<(&Item, PathBuf)> = Vec::new();
    let mut reserved = HashSet::new();
    for it in plan.items.iter().filter(|i| ids.contains(&i.id)) {
        match resolve(out_dir, it, &mut reserved) {
            Some((p, true)) => outcomes.push(Outcome { id: it.id, status: Status::Existing, path: Some(p.display().to_string()), message: None }),
            Some((p, false)) => todo.push((it, p)),
            None => outcomes.push(Outcome { id: it.id, status: Status::Failed, path: None, message: Some("kein freier Dateiname".into()) }),
        }
    }
    let total: u64 = todo.iter().map(|(i, _)| i.bytes).sum();
    if let Some(free) = available_bytes(out_dir) {
        if total > 0 && free < total + (64 << 20) {
            return Err(format!("Zu wenig Speicherplatz im Zielordner: benötigt {}, frei {}.", human_bytes(total), human_bytes(free)));
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
            let mut report = |d: u64| progress(&Progress { index, count, id: it.id, name: name.clone(), done: base + d, total, milestone: false });
            report(0);
            write_item(plan, it, target, cancel, &mut report)
        };
        progress(&Progress { index, count, id: it.id, name: name.clone(), done: base + it.bytes, total, milestone: true });
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

    /// Recorder 5 runs through; recorder 4 is off between 300 s and 500 s of
    /// 5's clock. Same situation throughout, so one stereo file results with
    /// silence on the left during the dropout, and the right channel is 5 bit-exactly.
    #[test]
    fn dropout_between_stereo_stretches_stays_one_stereo_file() {
        let dir = tempdir("dropout");
        let sr = 8000u32;
        let s = talk(11, sr, 900 * sr as usize);
        let mut floor = Lcg(21);
        let five: Vec<f32> = (0..900 * sr as usize).map(|k| 0.5 * s[k] + 0.002 * (floor.next() as f32 - 0.5)).collect();
        // 4a starts 2 s before 5 (its first 2 s are its own noise), 4b starts at 5's 500 s.
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
        assert_eq!(plan.pairs.iter().filter(|p| p.ok).count(), 2, "{:?}", plan.pairs.iter().map(|p| (p.ok, p.offset)).collect::<Vec<_>>());
        assert_eq!(plan.items.len(), 1, "{:#?}", plan.items.iter().map(|i| (i.kind, i.reason, i.t0, i.t1)).collect::<Vec<_>>());
        let it = &plan.items[0];
        assert_eq!((it.kind, it.spans.len()), ("stereo", 2));
        assert_eq!(plan.tracks[it.reference].label, "5");
        assert!((it.t0 + 2.0).abs() < 0.01 && (it.t1 - 900.0).abs() < 0.01, "{} {}", it.t0, it.t1);
        assert!((it.dropout - 202.0).abs() < 1.0, "dropout {}", it.dropout);
        assert!(it.name.ends_with("_stereo_L-4_R-5.wav"), "{}", it.name);

        let sum = write(&plan, &[it.id], &dir.join("sync"), &cancel, |_| {}).unwrap();
        let path = PathBuf::from(sum.outcomes[0].path.clone().unwrap());
        let info = wav::read_info(&path).unwrap();
        let raw = wav::read_at(&path, info.data_offset, info.data_len).unwrap();
        let lr: Vec<(f32, f32)> =
            raw.chunks_exact(8).map(|c| (f32::from_le_bytes(c[0..4].try_into().unwrap()), f32::from_le_bytes(c[4..8].try_into().unwrap()))).collect();
        let lead_frames = 2 * sr as usize;
        assert_eq!(lr.len(), 902 * sr as usize);
        for k in [0usize, 5_000, 2_000_000, 3_000_000, 7_100_000] {
            assert_eq!(lr[lead_frames + k].1, five[k], "right channel is recorder 5 bit-exactly");
        }
        assert_eq!(lr[lead_frames - 10].1, 0.0);
        let gap = (lead_frames + 310 * sr as usize)..(lead_frames + 490 * sr as usize);
        assert!(lr[gap].iter().all(|v| v.0 == 0.0), "left channel silent while recorder 4 was off");
        let aligned = |k0: usize, src: &dyn Fn(usize) -> f32| {
            (-8i64..=8)
                .max_by(|&x, &y| {
                    let score = |d: i64| (k0..k0 + 8000).map(|k| lr[k].0 as f64 * src((k as i64 + d) as usize) as f64).sum::<f64>();
                    score(x).total_cmp(&score(y))
                })
                .unwrap()
        };
        assert_eq!(aligned(lead_frames + 100 * sr as usize, &|k| a4[k]), 0, "left follows 4a");
        assert_eq!(aligned(lead_frames + 600 * sr as usize, &|k| b4[k - lead_frames - 500 * sr as usize]), 0, "left follows 4b");
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

    /// Recorder 4 runs 150 s alone before 5 starts, 5 runs 100 s alone after 4
    /// stops. Nothing parallel differs, so it is one stereo file with silence.
    #[test]
    fn long_solo_lead_and_tail_stay_in_the_stereo_file() {
        let dir = tempdir("solo");
        let sr = 8000u32;
        let n = |secs: usize| secs * sr as usize;
        let s = talk(31, sr, n(700));
        let mut floor = Lcg(32);
        let four: Vec<f32> = (0..n(600)).map(|g| s[g] + 0.002 * (floor.next() as f32 - 0.5)).collect();
        let five: Vec<f32> = (0..n(550)).map(|k| 0.5 * s[k + n(150)] + 0.002 * (floor.next() as f32 - 0.5)).collect();
        write_float_wav(&dir.join("tracks/260101_S100000-E101000_D001000_4.wav"), sr, &four);
        write_float_wav(&dir.join("tracks/260101_S100230-E101140_D000910_5.wav"), sr, &five);
        let cancel = AtomicBool::new(false);
        let plan = analyze(&[dir.join("tracks")], &cancel, &mut |_| {}).unwrap();
        assert!(plan.pairs[0].ok);
        assert_eq!(plan.items.len(), 1, "{:#?}", plan.items.iter().map(|i| (i.kind, i.reason, i.t0, i.t1)).collect::<Vec<_>>());
        let it = &plan.items[0];
        assert_eq!(it.kind, "stereo");
        assert!(it.t0.abs() < 0.01 && (it.t1 - 700.0).abs() < 0.05, "{} {}", it.t0, it.t1);
        let silent: Vec<(String, i64)> = it.silent.iter().map(|x| (x.label.clone(), x.seconds.round() as i64)).collect();
        assert!(silent.contains(&("5".to_string(), 150)) && silent.contains(&("4".to_string(), 100)), "{silent:?}");
        assert_eq!(it.name, "260101_S100000-E101140_D001140_stereo_L-4_R-5.wav");

        let sum = write(&plan, &[it.id], &dir.join("sync"), &cancel, |_| {}).unwrap();
        let path = PathBuf::from(sum.outcomes[0].path.clone().unwrap());
        let info = wav::read_info(&path).unwrap();
        let raw = wav::read_at(&path, info.data_offset, info.data_len).unwrap();
        let lr: Vec<(f32, f32)> =
            raw.chunks_exact(8).map(|c| (f32::from_le_bytes(c[0..4].try_into().unwrap()), f32::from_le_bytes(c[4..8].try_into().unwrap()))).collect();
        assert_eq!(lr.len(), n(700));
        assert!(lr[..n(149)].iter().all(|v| v.1 == 0.0), "right silent before 5 starts");
        assert!(lr[n(601)..].iter().all(|v| v.0 == 0.0), "left silent after 4 stops");
        assert!(lr[n(601)..n(602)].iter().any(|v| v.1 != 0.0), "right keeps playing after 4 stops");
        for k in [0usize, 777, n(300)] {
            assert_eq!(lr[k].0, four[k]);
        }
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
        assert!(stereo[0].t0.abs() < 1e-9, "leading seconds of A join the stereo file");
        assert!((stereo[1].t1 - 963.5).abs() < 0.01, "trailing seconds of B join the stereo file");

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
        let f0 = (st.t0 * sr as f64).round() as usize;
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
}
