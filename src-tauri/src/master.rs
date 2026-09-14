//! Dritte Funktion „Mastern“: Format jeder Audiodatei erkennen, Lautheit nach
//! EBU R128 messen, mit einer festen Verstärkung auf −16 LUFS bringen (ein
//! Look-ahead-Limiter fängt nur die Spitzen bei −1,5 dBTP, die Dynamik bleibt)
//! und als MP3 mit 192 kbit/s ausgeben.
//!
//! Alles läuft in der App selbst, ohne installierte Werkzeuge:
//! WAV (auch RF64) über den eigenen Leser, MP3, AAC/M4A, FLAC, ALAC, AIFF, CAF
//! und OGG Vorbis über Symphonia; Lautheit (ITU-R BS.1770-4, True Peak) über
//! ebur128; der Limiter ist hier implementiert; MP3 kodiert LAME 3.100, das fest
//! einkompiliert ist. Nach dem Kodieren wird das MP3 nachgemessen und die
//! Verstärkung notfalls nachgeregelt, bis es höchstens 0,3 LU vom Ziel abweicht.

use crate::decode::{container, extension, open_reader, Details, SymReader, AUDIO_EXT};
use crate::i18n::{self, t, tf, Lang, Msg};
use crate::merge::{available_bytes, low_space, Active, Progress, Status};
use crate::scan::Skipped;
use crate::sync::{self, SyncProgress};
use crate::wav;
use ebur128::{EbuR128, Mode as R128};
use mp3lame_encoder::{Bitrate, Builder, FlushGap, Id3Tag, InterleavedPcm, Mode as LameMode, MonoPcm, Quality, VbrMode};
use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

pub const OUTPUT_DIR_NAME: &str = "master";
pub const TARGET_LUFS: f64 = -16.0;
pub const CEILING_DBTP: f64 = -1.5;
pub const BITRATE_KBPS: u32 = 192;
const MAX_GAIN_DB: f64 = 40.0;
const LUFS_TOLERANCE: f64 = 0.3;
/// MP3 encoding can push true peaks above the PCM ceiling; the result may exceed it by this much at most.
const PEAK_TOLERANCE: f64 = 0.1;
/// Loudness accuracy of the fast search on the limited PCM before encoding.
const PCM_TOLERANCE: f64 = 0.05;
/// MP3's low-pass usually takes 0.1–0.3 LU of treble energy; the search starts aiming this much louder.
const CODEC_LOUDNESS_LOSS: f64 = 0.15;
/// The limiter starts this far below −1.5 dBTP: MP3 encoding adds 0.3–0.6 dB of
/// peaks on heavily limited recordings, and a re-encode costs far more than
/// half a dB of extra limiting on a few transients.
const CODEC_MARGIN_DB: f64 = 0.5;
/// Relative cost of a pass, for honest progress across all passes of a file.
const W_SEARCH: f64 = 0.35;
const W_ENCODE: f64 = 1.0;
const W_CHECK: f64 = 0.3;
const ATTACK_S: f64 = 0.005;
const RELEASE_S: f64 = 0.05;
const MP3_RATES: [u32; 9] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];

#[derive(Debug, Clone, Serialize)]
pub struct Loudness {
    pub lufs: f64,
    pub true_peak: f64,
    pub lra: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioFile {
    pub id: usize,
    pub path: String,
    pub name: String,
    pub folder: String,
    /// e.g. "WAV · 32-bit float · 48 kHz · Stereo"
    pub format: String,
    pub container: String,
    pub codec: String,
    pub bits: Option<u32>,
    pub float: bool,
    pub sample_rate: u32,
    pub channels: u32,
    pub bit_rate: Option<u64>,
    pub duration: f64,
    pub size: u64,
    pub loudness: Option<Loudness>,
    /// Static gain to reach the target (None: silence or unreadable).
    pub gain_db: Option<f64>,
    /// How far peaks would exceed the ceiling after the gain, i.e. what the limiter catches.
    pub limited_db: f64,
    pub note: Option<String>,
    /// Raw DJI part (usually already contained in a track).
    pub dji_part: bool,
    pub out_name: String,
    #[serde(skip)]
    pub path_buf: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct MasterPlan {
    pub roots: Vec<String>,
    pub default_out_dir: String,
    pub target_lufs: f64,
    pub ceiling_dbtp: f64,
    pub bitrate_kbps: u32,
    pub files: Vec<AudioFile>,
    pub ignored: Vec<Skipped>,
    pub engine: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MasterOutcome {
    pub id: usize,
    pub status: Status,
    pub path: Option<String>,
    pub message: Option<String>,
    pub result: Option<Loudness>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MasterSummary {
    pub out_dir: String,
    pub outcomes: Vec<MasterOutcome>,
    pub cancelled: bool,
}

// ------------------------------------------------------------------ loudness

struct Measured {
    loudness: Loudness,
    frames: u64,
    channels: usize,
    rate: u32,
    details: Details,
}

fn measure(path: &Path, ext: &str, cancel: &AtomicBool, on_frames: &mut dyn FnMut(u64)) -> Result<Measured, String> {
    let (mut reader, details) = open_reader(path, ext)?;
    let (ch, rate) = (reader.channels(), reader.rate());
    let mut meter = EbuR128::new(ch as u32, rate, R128::I | R128::LRA | R128::TRUE_PEAK).map_err(|e| loudness_err(e))?;
    let mut buf = Vec::new();
    let mut frames = 0u64;
    loop {
        buf.clear();
        if !reader.read(&mut buf)? {
            break;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(i18n::cancelled());
        }
        let whole = buf.len() - buf.len() % ch;
        meter.add_frames_f32(&buf[..whole]).map_err(|e| loudness_err(e))?;
        frames += (whole / ch) as u64;
        on_frames(frames);
    }
    let lufs = meter.loudness_global().unwrap_or(f64::NEG_INFINITY);
    let peak = (0..ch as u32).filter_map(|c| meter.true_peak(c).ok()).fold(0.0f64, f64::max);
    let true_peak = if peak > 0.0 { 20.0 * peak.log10() } else { f64::NEG_INFINITY };
    let lra = meter.loudness_range().unwrap_or(0.0).max(0.0);
    Ok(Measured { loudness: Loudness { lufs, true_peak, lra }, frames, channels: ch, rate, details })
}

// ------------------------------------------------------------------ limiter and resampling

#[inline]
fn catmull(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    p1 + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

/// Look-ahead limiter that also sees peaks between samples: each frame's peak
/// includes three interpolated points towards its left neighbour (cubic, needs
/// one frame of extra look-ahead). The gain reaches its required value exactly
/// when a peak arrives (minimum over the look-ahead window, then a moving
/// average of the same length gives a smooth ramp) and recovers with a 50-ms
/// release. The output has exactly as many frames as the input and is aligned.
struct Limiter {
    ch: usize,
    limit: f32,
    look: usize,
    release: f64,
    delay: VecDeque<f32>,
    recent: VecDeque<f32>,
    mins: VecDeque<(u64, f32)>,
    ramp: VecDeque<f32>,
    sum: f64,
    env: f64,
    needs: u64,
    real: u64,
    emitted: u64,
}

impl Limiter {
    fn new(ch: usize, rate: u32, limit: f32) -> Self {
        let look = ((ATTACK_S * rate as f64).round() as usize).max(1);
        Limiter {
            ch,
            limit,
            look,
            release: 1.0 - (-1.0 / (RELEASE_S * rate as f64)).exp(),
            delay: VecDeque::with_capacity((look + 3) * ch),
            recent: VecDeque::with_capacity(3 * ch),
            mins: VecDeque::new(),
            ramp: VecDeque::with_capacity(look + 2),
            sum: 0.0,
            env: 1.0,
            needs: 0,
            real: 0,
            emitted: 0,
        }
    }

    fn push(&mut self, frame: &[f32], out: &mut Vec<f32>) {
        self.real += 1;
        self.feed(frame, out);
    }

    fn flush(&mut self, out: &mut Vec<f32>) {
        let silence = vec![0f32; self.ch];
        while self.emitted < self.real {
            self.feed(&silence, out);
        }
    }

    fn feed(&mut self, frame: &[f32], out: &mut Vec<f32>) {
        self.delay.extend(frame.iter().copied());
        let n = self.recent.len() / self.ch;
        if n > 0 {
            // The newest stored frame now has its right neighbour: measure its peak.
            let mut peak = 0f32;
            for c in 0..self.ch {
                let at = |back: usize| self.recent[(n - 1 - back.min(n - 1)) * self.ch + c];
                let (p0, p1, p2, p3) = (at(2), at(1), at(0), frame[c]);
                peak = peak.max(p2.abs());
                for t in [0.25f32, 0.5, 0.75] {
                    peak = peak.max(catmull(p0, p1, p2, p3, t).abs());
                }
            }
            self.add_need(peak, out);
        }
        self.recent.extend(frame.iter().copied());
        while self.recent.len() > 3 * self.ch {
            self.recent.pop_front();
        }
    }

    fn add_need(&mut self, peak: f32, out: &mut Vec<f32>) {
        let need = if peak > self.limit { self.limit / peak } else { 1.0 };
        while self.mins.back().map_or(false, |&(_, v)| v >= need) {
            self.mins.pop_back();
        }
        self.mins.push_back((self.needs, need));
        while self.mins.front().map_or(false, |&(i, _)| i + (self.look as u64) < self.needs) {
            self.mins.pop_front();
        }
        if self.needs >= self.look as u64 {
            let m = self.mins.front().map_or(1.0, |&(_, v)| v);
            self.ramp.push_back(m);
            self.sum += m as f64;
            if self.ramp.len() > self.look + 1 {
                self.sum -= self.ramp.pop_front().unwrap_or(1.0) as f64;
            }
            let target = (self.sum / self.ramp.len() as f64).min(1.0);
            self.env = if target < self.env { target } else { self.env + (target - self.env) * self.release };
            let keep = self.emitted < self.real;
            for _ in 0..self.ch {
                let s = self.delay.pop_front().unwrap_or(0.0);
                if keep {
                    out.push((s as f64 * self.env) as f32);
                }
            }
            if keep {
                self.emitted += 1;
            }
        }
        self.needs += 1;
    }
}

/// Integer-factor downsampling with a windowed-sinc low-pass (for 88.2/96/176.4/192 kHz sources).
struct Decimator {
    ch: usize,
    factor: usize,
    taps: Vec<f32>,
    history: Vec<VecDeque<f32>>,
    phase: usize,
}

impl Decimator {
    fn new(ch: usize, factor: usize) -> Self {
        let n = 64 * factor + 1;
        let fc = 0.45 / factor as f64;
        let mid = (n / 2) as f64;
        let mut taps: Vec<f64> = (0..n)
            .map(|i| {
                let x = i as f64 - mid;
                let sinc = if x == 0.0 { 2.0 * fc } else { (std::f64::consts::TAU * fc * x).sin() / (std::f64::consts::PI * x) };
                let w = 0.42 - 0.5 * (std::f64::consts::TAU * i as f64 / (n - 1) as f64).cos() + 0.08 * (2.0 * std::f64::consts::TAU * i as f64 / (n - 1) as f64).cos();
                sinc * w
            })
            .collect();
        let sum: f64 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= sum);
        Decimator { ch, factor, taps: taps.into_iter().map(|t| t as f32).collect(), history: (0..ch).map(|_| VecDeque::from(vec![0f32; n])).collect(), phase: 0 }
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for frame in input.chunks_exact(self.ch) {
            for (c, h) in self.history.iter_mut().enumerate() {
                h.pop_front();
                h.push_back(frame[c]);
            }
            self.phase += 1;
            if self.phase == self.factor {
                self.phase = 0;
                for h in &self.history {
                    out.push(h.iter().zip(&self.taps).map(|(x, t)| x * t).sum());
                }
            }
        }
    }
}

fn loudness_err(e: impl std::fmt::Debug) -> String {
    tf(Msg::LoudnessMeasurement, &[("e", &format!("{e:?}"))])
}

fn mp3_rate(rate: u32) -> Result<(u32, usize), String> {
    if MP3_RATES.contains(&rate) {
        return Ok((rate, 1));
    }
    for factor in [2u32, 4, 8] {
        if rate % factor == 0 && MP3_RATES.contains(&(rate / factor)) {
            return Ok((rate / factor, factor as usize));
        }
    }
    Err(tf(Msg::RateUnsupportedMp3, &[("rate", &rate)]))
}

// ------------------------------------------------------------------ analysis

fn is_audio(p: &Path) -> bool {
    let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    !name.starts_with('.') && AUDIO_EXT.contains(&extension(p).as_str())
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
        } else if ft.is_file() && is_audio(&path) {
            out.push(path);
        }
    }
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
    let name = common.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name == crate::scan::OUTPUT_DIR_NAME || name == sync::OUTPUT_DIR_NAME {
        if let Some(parent) = common.parent() {
            return parent.join(OUTPUT_DIR_NAME);
        }
    }
    common.join(OUTPUT_DIR_NAME)
}

/// "WAV · 32-bit float · 48 kHz · Stereo" in `lang` (empty parts left out).
fn format_line(lang: Lang, container: &str, detail: &str, rate: u32, channels: usize) -> String {
    [container.to_string(), detail.to_string(), i18n::khz(lang, rate), i18n::channels(lang, channels as u32)]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Container, codec detail, bit depth and float flag in plain words.
fn describe(ext: &str, codec: &str, bits: Option<u32>, bit_rate: Option<u64>) -> (String, String, Option<u32>, bool) {
    let container = container(ext).to_string();
    if let Some(rest) = codec.strip_prefix("pcm_") {
        let float = rest.starts_with('f');
        let b: Option<u32> = rest.chars().filter(char::is_ascii_digit).collect::<String>().parse().ok().or(bits);
        let detail = match b {
            Some(b) if float => format!("{b}-bit float"),
            Some(b) => format!("{b}-bit"),
            None => "PCM".into(),
        };
        return (container, detail, b, float);
    }
    let lossless = matches!(codec, "flac" | "alac");
    let name = match codec {
        "mp3" => "MP3".to_string(),
        "aac" => "AAC".to_string(),
        "alac" => "ALAC".to_string(),
        "flac" => "FLAC".to_string(),
        "vorbis" => "Vorbis".to_string(),
        other => other.to_uppercase(),
    };
    let mut parts = Vec::new();
    if name != container {
        parts.push(name);
    }
    if lossless {
        if let Some(b) = bits {
            parts.push(format!("{b}-bit"));
        }
    } else if let Some(br) = bit_rate {
        parts.push(format!("{} kbit/s", (br as f64 / 1000.0).round()));
    }
    (container, parts.join(" "), if lossless { bits } else { None }, false)
}

/// WAV files that step 1 would join with others into one recording (raw chunks,
/// any recorder). Merged tracks and single recordings are not chunks.
fn chunk_paths(files: &[PathBuf]) -> HashSet<PathBuf> {
    let wavs: Vec<PathBuf> = files.iter().filter(|p| extension(p) == "wav").cloned().collect();
    if wavs.is_empty() {
        return HashSet::new();
    }
    crate::scan::scan(&wavs, &crate::scan::Options::default())
        .map(|s| s.recordings.into_iter().filter(|r| r.parts.len() > 1).flat_map(|r| r.parts).map(|p| p.path_buf).collect())
        .unwrap_or_default()
}

pub fn analyze(inputs: &[PathBuf], cancel: &AtomicBool, progress: &mut dyn FnMut(&SyncProgress)) -> Result<MasterPlan, String> {
    progress(&SyncProgress { stage: "load", done: 0, total: 1, text: t(Msg::ProgressFindAudio).into() });
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut files = Vec::new();
    let mut ignored = Vec::new();
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
            if is_audio(input) {
                files.push(input.clone());
            } else {
                ignored.push(Skipped { path: input.display().to_string(), reason: t(Msg::NotSupportedAudioFile).into() });
            }
        }
    }
    if roots.is_empty() {
        return Err(t(Msg::NoFoldersOrFiles).into());
    }
    files.sort();
    files.dedup();
    if files.is_empty() {
        return Err(t(Msg::NoAudioFilesFound).into());
    }
    let chunks = chunk_paths(&files);

    let sizes: Vec<u64> = files.iter().map(|p| fs::metadata(p).map_or(0, |m| m.len())).collect();
    let total: u64 = sizes.iter().sum::<u64>().max(1);
    let done: Vec<AtomicU64> = files.iter().map(|_| AtomicU64::new(0)).collect();
    let jobs: Vec<usize> = (0..files.len()).collect();
    let measured = {
        let (files, sizes, done) = (&files, &sizes, &done);
        sync::run_parallel(
            &jobs,
            &mut || {
                let d: u64 = done.iter().map(|x| x.load(Ordering::Relaxed)).sum();
                progress(&SyncProgress { stage: "measure", done: d.min(total), total, text: t(Msg::ProgressMeasure).into() })
            },
            &|&i: &usize| {
                let path = &files[i];
                let ext = extension(path);
                // Progress per file by decoded frames against the expected length (known for WAV).
                let expected = wav::read_info(path).map(|w| w.frames()).unwrap_or(0);
                let r = measure(path, &ext, cancel, &mut |frames| {
                    if expected > 0 {
                        done[i].store((sizes[i] as f64 * (frames as f64 / expected as f64).min(1.0)) as u64, Ordering::Relaxed);
                    }
                });
                done[i].store(sizes[i], Ordering::Relaxed);
                r
            },
        )
    };
    if cancel.load(Ordering::SeqCst) {
        return Err(i18n::cancelled());
    }

    let mut names: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for ((path, size), m) in files.into_iter().zip(sizes).zip(measured) {
        let m = match m {
            Some(Ok(m)) if m.frames > 0 => m,
            Some(Ok(_)) => {
                ignored.push(Skipped { path: path.display().to_string(), reason: t(Msg::NoAudioData).into() });
                continue;
            }
            Some(Err(e)) => {
                ignored.push(Skipped { path: path.display().to_string(), reason: e });
                continue;
            }
            None => {
                ignored.push(Skipped { path: path.display().to_string(), reason: t(Msg::Unreadable).into() });
                continue;
            }
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = extension(&path);
        let duration = m.frames as f64 / m.rate as f64;
        let lossy = !m.details.codec.starts_with("pcm_") && !matches!(m.details.codec.as_str(), "flac" | "alac");
        let bit_rate = (lossy && duration > 0.0).then(|| (size as f64 * 8.0 / duration) as u64);
        let (container, detail, bits, float) = describe(&ext, &m.details.codec, m.details.bits, bit_rate);
        let format = format_line(i18n::current(), &container, &detail, m.rate, m.channels);
        let l = m.loudness;
        let (mut gain_db, mut limited_db, mut note) = (None, 0.0, None);
        if l.lufs.is_finite() && l.lufs > -70.0 {
            let g = TARGET_LUFS - l.lufs;
            if g > MAX_GAIN_DB {
                note = Some(tf(Msg::NoteVeryQuiet, &[("max", &format!("{MAX_GAIN_DB:.0}"))]));
            }
            let g = g.min(MAX_GAIN_DB);
            if l.true_peak.is_finite() {
                limited_db = (l.true_peak + g - CEILING_DBTP).max(0.0);
            }
            gain_db = Some(g);
        } else {
            note = Some(t(Msg::NoteSilence).into());
        }
        if let Err(e) = mp3_rate(m.rate) {
            note = Some(e);
            gain_db = None;
        }
        let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut out_name = format!("{stem}.mp3");
        let mut n = 2;
        while !names.insert(out_name.to_lowercase()) {
            out_name = format!("{stem}_{n}.mp3");
            n += 1;
        }
        out.push(AudioFile {
            id: out.len(),
            path: path.display().to_string(),
            folder: path.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            dji_part: chunks.contains(&path),
            name,
            format,
            container,
            codec: m.details.codec,
            bits,
            float,
            sample_rate: m.rate,
            channels: m.channels as u32,
            bit_rate,
            duration,
            size,
            loudness: Some(l),
            gain_db,
            limited_db,
            note,
            out_name,
            path_buf: path,
        });
    }
    if out.is_empty() {
        return Err(t(Msg::NoReadableAudio).into());
    }
    Ok(MasterPlan {
        default_out_dir: default_out_dir(&roots).display().to_string(),
        roots: roots.iter().map(|r| r.display().to_string()).collect(),
        target_lufs: TARGET_LUFS,
        ceiling_dbtp: CEILING_DBTP,
        bitrate_kbps: BITRATE_KBPS,
        files: out,
        ignored,
        engine: format!("Symphonia · ebur128 · LAME {}", mp3lame_encoder::mp3lame_version()),
    })
}

// ------------------------------------------------------------------ writing

fn lame_err<E>(_: E) -> String {
    t(Msg::EncoderSetting).to_string()
}

/// Progress of one file across all its passes, in weighted milliseconds of audio.
/// The total grows when an extra pass is needed, so 100 % means really done.
struct Work {
    done: AtomicU64,
    total: AtomicU64,
    base: AtomicU64,
    dur_ms: u64,
}

impl Work {
    fn new(duration: f64) -> Self {
        let dur_ms = (duration * 1000.0) as u64;
        let expected = dur_ms as f64 * (2.0 * W_SEARCH + W_ENCODE + W_CHECK);
        Work { done: AtomicU64::new(0), total: AtomicU64::new(expected as u64), base: AtomicU64::new(0), dur_ms }
    }
    /// Starts a pass of `weight`; `after` is the weight of the passes that must still follow.
    fn begin(&self, weight: f64, after: f64) {
        let d = self.done.load(Ordering::Relaxed);
        self.base.store(d, Ordering::Relaxed);
        self.total.fetch_max(d + (self.dur_ms as f64 * (weight + after)) as u64, Ordering::Relaxed);
    }
    fn advance(&self, secs: f64, weight: f64) {
        let ms = (secs * 1000.0).min(self.dur_ms as f64);
        self.done.fetch_max(self.base.load(Ordering::Relaxed) + (ms * weight) as u64, Ordering::Relaxed);
    }
    fn end(&self, weight: f64) {
        self.done.fetch_max(self.base.load(Ordering::Relaxed) + (self.dur_ms as f64 * weight) as u64, Ordering::Relaxed);
    }
    fn finish(&self) {
        self.total.store(self.done.load(Ordering::Relaxed), Ordering::Relaxed);
    }
}

fn layout(f: &AudioFile) -> Result<(usize, u32), String> {
    let (rate, _) = mp3_rate(f.sample_rate)?;
    Ok(((f.channels as usize).clamp(1, 2), rate))
}

/// Decodes the source, applies gain and limiter, downsamples if needed and hands every block to `sink`.
fn render(
    f: &AudioFile,
    gain_db: f64,
    ceiling_db: f64,
    cancel: &AtomicBool,
    on_secs: &mut dyn FnMut(f64),
    sink: &mut dyn FnMut(&[f32]) -> Result<(), String>,
) -> Result<(), String> {
    let ext = extension(&f.path_buf);
    let (mut reader, _) = open_reader(&f.path_buf, &ext)?;
    let (ch_in, rate) = (reader.channels(), reader.rate());
    let out_ch = ch_in.clamp(1, 2);
    let (_, factor) = mp3_rate(rate)?;
    let gain = 10f32.powf(gain_db as f32 / 20.0);
    let mut limiter = Limiter::new(out_ch, rate, 10f32.powf(ceiling_db as f32 / 20.0));
    let mut decimator = (factor > 1).then(|| Decimator::new(out_ch, factor));
    let (mut input, mut limited, mut resampled) = (Vec::new(), Vec::new(), Vec::new());
    let mut frames = 0u64;
    loop {
        input.clear();
        let more = reader.read(&mut input)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(i18n::cancelled());
        }
        limited.clear();
        if more {
            let whole = input.len() - input.len() % ch_in;
            for frame in input[..whole].chunks_exact(ch_in) {
                let scaled = [frame[0] * gain, frame[if out_ch == 2 { 1 } else { 0 }] * gain];
                limiter.push(&scaled[..out_ch], &mut limited);
            }
            frames += (whole / ch_in) as u64;
            on_secs(frames as f64 / rate as f64);
        } else {
            limiter.flush(&mut limited);
        }
        let pcm: &[f32] = match decimator.as_mut() {
            Some(d) => {
                resampled.clear();
                d.process(&limited, &mut resampled);
                &resampled
            }
            None => &limited,
        };
        if !pcm.is_empty() {
            sink(pcm)?;
        }
        if !more {
            break;
        }
    }
    Ok(())
}

/// Integrated loudness of the limited signal as it would go into the encoder
/// (loudness only: the true peak is judged on the finished MP3).
fn measure_render(f: &AudioFile, gain_db: f64, ceiling_db: f64, cancel: &AtomicBool, on_secs: &mut dyn FnMut(f64)) -> Result<f64, String> {
    let (ch, rate) = layout(f)?;
    let mut meter = EbuR128::new(ch as u32, rate, R128::I).map_err(|e| loudness_err(e))?;
    render(f, gain_db, ceiling_db, cancel, on_secs, &mut |pcm| meter.add_frames_f32(pcm).map_err(|e| loudness_err(e)))?;
    Ok(meter.loudness_global().unwrap_or(f64::NEG_INFINITY))
}

fn debug(f: &AudioFile, msg: String) {
    if std::env::var_os("PA_MASTER_DEBUG").is_some() {
        eprintln!("[master] {}: {msg}", f.name);
    }
}

fn encode(f: &AudioFile, out_path: &Path, gain_db: f64, ceiling_db: f64, cancel: &AtomicBool, on_secs: &mut dyn FnMut(f64)) -> Result<(), String> {
    let (out_ch, out_rate) = layout(f)?;
    let stem = Path::new(&f.out_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut b = Builder::new().ok_or(t(Msg::EncoderUnavailable))?;
    b.set_num_channels(out_ch as u8).map_err(lame_err)?;
    b.set_sample_rate(out_rate).map_err(lame_err)?;
    b.set_vbr_mode(VbrMode::Off).map_err(lame_err)?;
    b.set_brate(Bitrate::Kbps192).map_err(lame_err)?;
    b.set_quality(Quality::NearBest).map_err(lame_err)?;
    b.set_mode(if out_ch == 1 { LameMode::Mono } else { LameMode::JointStereo }).map_err(lame_err)?;
    b.set_to_write_vbr_tag(false).map_err(lame_err)?;
    let comment = format!("PrepareAudio: {TARGET_LUFS} LUFS");
    let _ = b.set_id3_tag(Id3Tag { title: stem.as_bytes(), artist: &[], album: &[], album_art: &[], year: &[], comment: comment.as_bytes() });
    let mut encoder = b.build().map_err(lame_err)?;
    let mut file = BufWriter::with_capacity(1 << 20, File::create(out_path).map_err(|e| e.to_string())?);
    let mut mp3 = Vec::new();
    render(f, gain_db, ceiling_db, cancel, on_secs, &mut |pcm| {
        let count = pcm.len() / out_ch;
        mp3.clear();
        mp3.reserve(count * 5 / 4 + 7200);
        let n = if out_ch == 1 {
            encoder.encode(MonoPcm(pcm), mp3.spare_capacity_mut())
        } else {
            encoder.encode(InterleavedPcm(pcm), mp3.spare_capacity_mut())
        }
        .map_err(|_| t(Msg::EncodingFailed).to_string())?;
        // SAFETY: the encoder initialised the first `n` bytes of the spare capacity.
        unsafe { mp3.set_len(n) };
        file.write_all(&mp3).map_err(|e| e.to_string())
    })?;
    mp3.clear();
    mp3.reserve(7200);
    let n = encoder.flush::<FlushGap>(mp3.spare_capacity_mut()).map_err(|_| t(Msg::EncodingFailed).to_string())?;
    // SAFETY: as above.
    unsafe { mp3.set_len(n) };
    file.write_all(&mp3).map_err(|e| e.to_string())?;
    let file = file.into_inner().map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

/// Finds gain and limiter ceiling on the limited PCM first (fast, no encoding;
/// heavy limiting swallows part of every extra dB, so the step uses the slope
/// between the last two tries), then encodes once and measures the MP3. Only
/// if the MP3 is still off (more than 0.3 LU, or MP3 peaks above −1.5 dBTP
/// + 0.1 dB) the search repeats with the correction and encodes again.
fn encode_to_target(f: &AudioFile, part: &Path, cancel: &AtomicBool, work: &Work, stage: &dyn Fn(&str)) -> Result<Loudness, String> {
    let mut gain = f.gain_db.ok_or(t(Msg::NoMeasurableLoudness))?;
    let mut ceiling = CEILING_DBTP - CODEC_MARGIN_DB;
    // Loudness the PCM must reach so that the MP3 lands on the target; MP3's
    // low-pass can take away a little of the (K-weighted) treble energy.
    let mut pcm_target = TARGET_LUFS + CODEC_LOUDNESS_LOSS;
    let mut last = None;
    for round in 0..3 {
        let mut tries: Vec<(f64, f64)> = Vec::new();
        for _ in 0..5 {
            stage(i18n::t(Msg::StageLevel));
            work.begin(W_SEARCH, W_ENCODE + W_CHECK);
            let t = std::time::Instant::now();
            let lufs = measure_render(f, gain, ceiling, cancel, &mut |secs| work.advance(secs, W_SEARCH))?;
            work.end(W_SEARCH);
            debug(f, format!("search gain {gain:+.2} ceiling {ceiling:.2}: {lufs:.2} LUFS ({:.1?})", t.elapsed()));
            let err = pcm_target - lufs;
            tries.push((gain, lufs));
            if !err.is_finite() || err.abs() <= PCM_TOLERANCE {
                break;
            }
            let slope = match tries.as_slice() {
                [.., (g1, l1), (g2, l2)] if (g2 - g1).abs() > 1e-3 => ((l2 - l1) / (g2 - g1)).clamp(0.2, 1.0),
                _ => 1.0,
            };
            let next = (gain + err / slope).min(MAX_GAIN_DB);
            if (next - gain).abs() < 0.01 {
                break;
            }
            gain = next;
        }
        stage(i18n::t(Msg::StageEncode));
        work.begin(W_ENCODE, W_CHECK);
        let t = std::time::Instant::now();
        encode(f, part, gain, ceiling, cancel, &mut |secs| work.advance(secs, W_ENCODE))?;
        work.end(W_ENCODE);
        debug(f, format!("encode ({:.1?})", t.elapsed()));
        stage(i18n::t(Msg::StageCheck));
        work.begin(W_CHECK, 0.0);
        let rate = f.sample_rate as f64;
        let t = std::time::Instant::now();
        let result = measure(part, "mp3", cancel, &mut |frames| work.advance(frames as f64 / rate, W_CHECK))?.loudness;
        work.end(W_CHECK);
        debug(f, format!("check: {:.2} LUFS, TP {:.2} ({:.1?})", result.lufs, result.true_peak, t.elapsed()));
        let err = TARGET_LUFS - result.lufs;
        let over = result.true_peak - CEILING_DBTP;
        let loud_ok = !err.is_finite() || err.abs() <= LUFS_TOLERANCE;
        let peak_ok = !over.is_finite() || over <= PEAK_TOLERANCE;
        last = Some(result);
        if (loud_ok && peak_ok) || round == 2 {
            break;
        }
        if !peak_ok {
            ceiling -= over + 0.05;
        }
        if !loud_ok {
            pcm_target += err;
        }
    }
    last.ok_or_else(|| t(Msg::NoMeasurement).to_string())
}

fn master_one(f: &AudioFile, target: &Path, cancel: &AtomicBool, work: &Work, stage: &dyn Fn(&str)) -> Result<Loudness, String> {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let part = target.with_file_name(format!(".{name}.part"));
    let result = encode_to_target(f, &part, cancel, work, stage).and_then(|l| {
        if target.exists() {
            return Err(tf(Msg::ExistsMeanwhile, &[("path", &target.display())]));
        }
        fs::rename(&part, target).map_err(|e| e.to_string())?;
        Ok(l)
    });
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

/// An MP3 of about the same length at the target path counts as an earlier result.
fn same_mp3(path: &Path, duration: f64) -> bool {
    let Ok((_, details)) = SymReader::open(path, "mp3", false) else { return false };
    if details.codec != "mp3" {
        return false;
    }
    let estimate = fs::metadata(path).map_or(0.0, |m| m.len() as f64 * 8.0 / (BITRATE_KBPS as f64 * 1000.0));
    (estimate - duration).abs() <= (duration * 0.02).max(1.0)
}

fn resolve(out_dir: &Path, f: &AudioFile, reserved: &mut HashSet<PathBuf>) -> Option<(PathBuf, bool)> {
    let stem = f.out_name.trim_end_matches(".mp3");
    for n in 1..1000 {
        let path = out_dir.join(if n == 1 { format!("{stem}.mp3") } else { format!("{stem}_{n}.mp3") });
        if reserved.contains(&path) {
            continue;
        }
        if !path.exists() {
            reserved.insert(path.clone());
            return Some((path, false));
        }
        if same_mp3(&path, f.duration) {
            reserved.insert(path.clone());
            return Some((path, true));
        }
    }
    None
}

pub fn write<F: FnMut(&Progress)>(plan: &MasterPlan, ids: &[usize], out_dir: &Path, cancel: &AtomicBool, mut progress: F) -> Result<MasterSummary, String> {
    fs::create_dir_all(out_dir).map_err(|e| tf(Msg::CannotCreateDir, &[("path", &out_dir.display()), ("e", &e)]))?;
    let mut outcomes = Vec::new();
    let mut todo: Vec<(usize, &AudioFile, PathBuf)> = Vec::new();
    let mut reserved = HashSet::new();
    for f in plan.files.iter().filter(|f| ids.contains(&f.id)) {
        if f.gain_db.is_none() {
            let msg = f.note.clone().unwrap_or_else(|| t(Msg::NoMeasurableLoudness).into());
            outcomes.push(MasterOutcome { id: f.id, status: Status::Failed, path: None, message: Some(msg), result: None });
            continue;
        }
        match resolve(out_dir, f, &mut reserved) {
            Some((p, true)) => outcomes.push(MasterOutcome { id: f.id, status: Status::Existing, path: Some(p.display().to_string()), message: None, result: None }),
            Some((p, false)) => todo.push((todo.len(), f, p)),
            None => outcomes.push(MasterOutcome { id: f.id, status: Status::Failed, path: None, message: Some(t(Msg::NoFreeName).into()), result: None }),
        }
    }
    let need: u64 = todo.iter().map(|(_, f, _)| (f.duration * BITRATE_KBPS as f64 * 125.0) as u64 + 65_536).sum();
    if let Some(free) = available_bytes(out_dir) {
        if !todo.is_empty() && free < need + (64 << 20) {
            return Err(low_space(need, free));
        }
    }
    let works: Vec<Work> = todo.iter().map(|(_, f, _)| Work::new(f.duration)).collect();
    let durations: Vec<u64> = todo.iter().map(|(_, f, _)| (f.duration * 1000.0) as u64).collect();
    let finished = AtomicUsize::new(0);
    let finished_ms = AtomicU64::new(0);
    let stages: Mutex<Vec<Active>> = Mutex::new(Vec::new());
    let count = todo.len();
    let results = {
        let (works, finished, finished_ms, stages, durations) = (&works, &finished, &finished_ms, &stages, &durations);
        sync::run_parallel(
            &todo,
            &mut || {
                // Honest progress: only finished files count; the stage of the rest is shown as text.
                let total: u64 = durations.iter().sum::<u64>().max(1);
                let active = stages.lock().map(|s| s.clone()).unwrap_or_default();
                let (id, name) = active.first().map(|a| (a.id, a.stage.clone())).unwrap_or_default();
                let done = finished_ms.load(Ordering::Relaxed).min(total);
                progress(&Progress { index: finished.load(Ordering::Relaxed), count, id, name, done, total, milestone: false, active });
            },
            &|(i, f, target): &(usize, &AudioFile, PathBuf)| {
                let stage = |what: &str| {
                    if let Ok(mut s) = stages.lock() {
                        match s.iter_mut().find(|a| a.id == f.id) {
                            Some(a) => a.stage = what.to_string(),
                            None => s.push(Active { id: f.id, stage: what.to_string() }),
                        }
                    }
                };
                let r = master_one(f, target, cancel, &works[*i], &stage);
                if let Ok(mut s) = stages.lock() {
                    s.retain(|a| a.id != f.id);
                }
                finished.fetch_add(1, Ordering::Relaxed);
                finished_ms.fetch_add(durations[*i], Ordering::Relaxed);
                works[*i].finish();
                r
            },
        )
    };
    for ((_, f, target), r) in todo.iter().zip(results) {
        outcomes.push(match r {
            Some(Ok(l)) => MasterOutcome { id: f.id, status: Status::Written, path: Some(target.display().to_string()), message: None, result: Some(l) },
            Some(Err(e)) if i18n::is_cancelled(&e) => MasterOutcome { id: f.id, status: Status::Cancelled, path: None, message: None, result: None },
            Some(Err(e)) => MasterOutcome { id: f.id, status: Status::Failed, path: None, message: Some(e), result: None },
            None => MasterOutcome { id: f.id, status: Status::Failed, path: None, message: Some(t(Msg::StoppedInternal).into()), result: None },
        });
    }
    outcomes.sort_by_key(|o| o.id);
    Ok(MasterSummary { out_dir: out_dir.display().to_string(), outcomes, cancelled: cancel.load(Ordering::SeqCst) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    fn rng(seed: u64) -> impl FnMut() -> f64 {
        let mut s = seed;
        move || {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (s >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// Speech-like bursts plus one short bump far above full scale (too short to move the integrated loudness).
    fn speechy(seed: u64, sr: u32, secs: usize, amp: f64) -> Vec<f32> {
        let mut rnd = rng(seed);
        let n = secs * sr as usize;
        let mut out = vec![0f32; n];
        let mut i = 0;
        while i < n {
            let burst = ((0.1 + 0.3 * rnd()) * sr as f64) as usize;
            let level = amp * (0.3 + rnd());
            for v in out.iter_mut().skip(i).take(burst) {
                *v = (level * (rnd() * 2.0 - 1.0)) as f32;
            }
            i += burst + ((0.05 + 0.3 * rnd()) * sr as f64) as usize;
        }
        for v in out.iter_mut().skip(n / 2).take(4) {
            *v = 3.0;
        }
        out
    }

    fn write_wav(path: &Path, fmt: &[u8], data: &[u8], frames: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = File::create(path).unwrap();
        wav::write_header(&mut f, fmt, data.len() as u64, frames, wav::RIFF_LIMIT).unwrap();
        f.write_all(data).unwrap();
    }

    fn fmt(tag: u16, ch: u16, sr: u32, bits: u16) -> Vec<u8> {
        let ba = ch * bits / 8;
        let mut v = Vec::new();
        for x in [tag, ch] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&sr.to_le_bytes());
        v.extend_from_slice(&(sr * ba as u32).to_le_bytes());
        v.extend_from_slice(&ba.to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v
    }

    #[test]
    fn format_line_and_notes_follow_the_language() {
        assert_eq!(format_line(Lang::De, "WAV", "16-bit", 44_100, 2), "WAV · 16-bit · 44,1 kHz · Stereo");
        assert_eq!(format_line(Lang::En, "WAV", "16-bit", 44_100, 2), "WAV · 16-bit · 44.1 kHz · Stereo");
        assert_eq!(format_line(Lang::Fr, "WAV", "16-bit", 44_100, 2), "WAV · 16-bit · 44,1 kHz · Stéréo");
        assert_eq!(format_line(Lang::It, "WAV", "16-bit", 44_100, 2), "WAV · 16-bit · 44,1 kHz · Stereo");
        assert_eq!(format_line(Lang::En, "MP3", "192 kbit/s", 48_000, 1), "MP3 · 192 kbit/s · 48 kHz · Mono");
        assert_eq!(format_line(Lang::De, "FLAC", "", 96_000, 6), "FLAC · 96 kHz · 6 Kanäle");
        assert_eq!(format_line(Lang::En, "FLAC", "", 96_000, 6), "FLAC · 96 kHz · 6 channels");
        assert_eq!(format_line(Lang::Fr, "CAF", "32-bit float", 32_000, 3), "CAF · 32-bit float · 32 kHz · 3 canaux");
        assert_eq!(format_line(Lang::It, "AIFF", "24-bit", 88_200, 4), "AIFF · 24-bit · 88,2 kHz · 4 canali");
        assert_eq!(i18n::format(Lang::En, Msg::RateUnsupportedMp3, &[("rate", &7350)]), "Sample rate 7350 Hz is not supported for MP3");
        assert_eq!(i18n::format(Lang::Fr, Msg::NoteVeryQuiet, &[("max", &"40")]), "très faible, gain limité à +40 dB");
        for l in Lang::ALL {
            assert!(i18n::is_cancelled(i18n::text(l, Msg::Cancelled)));
        }
        assert!(!i18n::is_cancelled("Analysis failed."));
    }

    #[test]
    fn limiter_holds_the_ceiling_and_leaves_the_rest_alone() {
        let rate = 48_000;
        let mut rnd = rng(3);
        let input: Vec<f32> = (0..rate).map(|i| if (14_400..14_600).contains(&i) { 3.0 } else { (0.1 * (rnd() * 2.0 - 1.0)) as f32 }).collect();
        let mut lim = Limiter::new(1, rate as u32, 0.5);
        let mut out = Vec::new();
        for s in &input {
            lim.push(&[*s], &mut out);
        }
        lim.flush(&mut out);
        assert_eq!(out.len(), input.len());
        assert!(out.iter().all(|v| v.abs() <= 0.5 + 1e-6), "max {}", out.iter().fold(0f32, |m, v| m.max(v.abs())));
        for i in [1000usize, 13_000, 40_000] {
            assert!((out[i] - input[i]).abs() <= input[i].abs() * 0.01 + 1e-6, "sample {i}: {} vs {}", out[i], input[i]);
        }

        // A tone at a quarter of the sample rate, sampled between its crests:
        // sample peaks 0.707, true peak 1.0. A pure sample-peak limiter at 0.8 would not act.
        let tone: Vec<f32> = (0..4800).map(|i| (std::f64::consts::FRAC_PI_2 * i as f64 + std::f64::consts::FRAC_PI_4).sin() as f32).collect();
        let mut lim = Limiter::new(1, rate as u32, 0.8);
        let mut out = Vec::new();
        for s in &tone {
            lim.push(&[*s], &mut out);
        }
        lim.flush(&mut out);
        assert_eq!(out.len(), tone.len());
        let max = out[2400..].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(max < 0.66, "inter-sample peaks are limited too: sample max {max}");
    }

    #[test]
    fn decimator_keeps_the_band_and_removes_aliases() {
        let rms = |v: &[f32]| (v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
        for (freq, keep) in [(1000.0, true), (40_000.0, false)] {
            let input: Vec<f32> = (0..96_000).map(|i| (0.5 * (std::f64::consts::TAU * freq * i as f64 / 96_000.0).sin()) as f32).collect();
            let mut d = Decimator::new(1, 2);
            let mut out = Vec::new();
            d.process(&input, &mut out);
            let ratio = rms(&out[1000..]) / rms(&input);
            if keep {
                assert!((ratio - 1.0).abs() < 0.02, "{freq} Hz: {ratio}");
            } else {
                assert!(ratio < 0.01, "{freq} Hz: {ratio}");
            }
        }
    }

    /// Only files that belong to a multi-part recording count as raw chunks, whatever their names.
    #[test]
    fn marks_chunks_of_multi_part_recordings() {
        let dir = tempdir("master-chunks");
        let sr = 16_000;
        let s = speechy(11, sr, 20, 0.2);
        let tr0 = 9 * 3600 * sr as u64;
        write_float_bwf(&dir.join("take-a.wav"), sr, &s[..160_000], "2026-01-01", "09:00:00", tr0);
        write_float_bwf(&dir.join("take-b.WAV"), sr, &s[160_000..], "2026-01-01", "09:00:10", tr0 + 160_000);
        write_float_wav(&dir.join("DJI_01_20260101_120000.WAV"), sr, &speechy(12, sr, 5, 0.2));
        write_float_wav(&dir.join("tracks/260101_S090000-E090020_D000020_rec.wav"), sr, &s);
        let plan = analyze(&[dir.clone()], &AtomicBool::new(false), &mut |_| {}).unwrap();
        let mut parts: Vec<(String, bool)> = plan.files.iter().map(|f| (f.name.clone(), f.dji_part)).collect();
        parts.sort();
        assert_eq!(
            parts,
            [
                ("260101_S090000-E090020_D000020_rec.wav".to_string(), false),
                ("DJI_01_20260101_120000.WAV".to_string(), false),
                ("take-a.wav".to_string(), true),
                ("take-b.WAV".to_string(), true),
            ]
        );
    }

    #[test]
    fn detects_formats_masters_to_target_and_skips_existing() {
        let dir = tempdir("master");
        write_float_wav(&dir.join("in/quiet_float.wav"), 48_000, &speechy(7, 48_000, 30, 0.02));
        // 16-bit stereo at 44.1 kHz.
        let left = speechy(8, 44_100, 20, 0.2);
        let right = speechy(9, 44_100, 20, 0.15);
        let pcm: Vec<u8> = left.iter().zip(&right).flat_map(|(l, r)| [((l.clamp(-1.0, 1.0)) * 32767.0) as i16, ((r.clamp(-1.0, 1.0)) * 32767.0) as i16]).flat_map(i16::to_le_bytes).collect();
        write_wav(&dir.join("in/sub/pcm16.wav"), &fmt(1, 2, 44_100, 16), &pcm, left.len() as u64);
        // 32-bit float stereo at 96 kHz (needs downsampling for MP3).
        let hi = speechy(10, 96_000, 12, 0.05);
        let hi_data: Vec<u8> = hi.iter().flat_map(|v| [*v, *v * 0.5]).flat_map(f32::to_le_bytes).collect();
        write_wav(&dir.join("in/sub/hires.wav"), &fmt(3, 2, 96_000, 32), &hi_data, hi.len() as u64);
        fs::create_dir_all(dir.join("in/master")).unwrap();
        fs::copy(dir.join("in/quiet_float.wav"), dir.join("in/master/old.wav")).unwrap();

        let cancel = AtomicBool::new(false);
        let plan = analyze(&[dir.join("in")], &cancel, &mut |_| {}).unwrap();
        assert_eq!(plan.default_out_dir, dir.join("in/master").display().to_string());
        assert_eq!(plan.files.len(), 3, "the master folder is skipped");
        let by = |n: &str| plan.files.iter().find(|f| f.name == n).unwrap();
        assert_eq!(by("quiet_float.wav").format, "WAV · 32-bit float · 48 kHz · Mono");
        assert_eq!(by("pcm16.wav").format, "WAV · 16-bit · 44,1 kHz · Stereo");
        assert_eq!(by("hires.wav").format, "WAV · 32-bit float · 96 kHz · Stereo");
        let q = by("quiet_float.wav");
        let ql = q.loudness.as_ref().unwrap();
        assert!(ql.lufs < -25.0 && ql.true_peak > 5.0, "{ql:?}");
        assert!(q.gain_db.unwrap() > 9.0 && q.limited_db > 15.0, "{:?} {}", q.gain_db, q.limited_db);
        assert!((q.duration - 30.0).abs() < 1e-6);

        let ids: Vec<usize> = plan.files.iter().map(|f| f.id).collect();
        let out = PathBuf::from(&plan.default_out_dir);
        let mut events: Vec<(usize, u64, u64)> = Vec::new();
        let sum = write(&plan, &ids, &out, &cancel, |p| events.push((p.index, p.done, p.total))).unwrap();
        assert!(!events.is_empty());
        for (index, done, total) in &events {
            assert!(done <= total);
            assert!(*index == ids.len() || done < total, "100 % shown before all files were finished: {events:?}");
        }
        for o in &sum.outcomes {
            assert_eq!(o.status, Status::Written, "{o:?}");
            let r = o.result.as_ref().unwrap();
            assert!((r.lufs - TARGET_LUFS).abs() <= 0.5, "{o:?}");
            assert!(r.true_peak <= CEILING_DBTP + PEAK_TOLERANCE, "{o:?}");
        }
        assert!(fs::read_dir(&out).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".part")));

        // The MP3s read back correctly and report their format.
        let back = analyze(&[out.join("quiet_float.mp3"), out.join("hires.mp3"), out.join("pcm16.mp3")], &cancel, &mut |_| {}).unwrap();
        let fmt_of = |n: &str| back.files.iter().find(|f| f.name == n).unwrap().format.clone();
        assert_eq!(fmt_of("quiet_float.mp3"), "MP3 · 192 kbit/s · 48 kHz · Mono");
        assert_eq!(fmt_of("hires.mp3"), "MP3 · 192 kbit/s · 48 kHz · Stereo");
        assert_eq!(fmt_of("pcm16.mp3"), "MP3 · 192 kbit/s · 44,1 kHz · Stereo");
        for f in &back.files {
            let l = f.loudness.as_ref().unwrap();
            assert!((l.lufs - TARGET_LUFS).abs() <= 0.5, "{} {l:?}", f.name);
        }

        let again = write(&plan, &ids, &out, &cancel, |_| {}).unwrap();
        assert!(again.outcomes.iter().all(|o| o.status == Status::Existing), "{:?}", again.outcomes);
    }
}
