//! Finds recorder chunks (WAV) anywhere below the given folders (tidy or messy)
//! and groups the chunks a recorder split each recording into.
//!
//! Every WAV is a candidate. Its start comes from the best source it has: the
//! Broadcast WAV `bext` chunk, a date and time in the file name, or the file
//! dates (modification time minus duration). Chunk B continues chunk A when
//! both have the same audio format and B starts where A ends: sample-exact by
//! bext TimeReference, otherwise within a tolerance that depends on the worse
//! time source. Only a chunk the recorder cut off (known chunk size, repeated
//! size, typical split limit, largest file) can have a continuation, unless the
//! TimeReference proves it. Two transmitters produce identical file names, so
//! when several candidates fit, the audio across the join decides: a linear
//! predictor trained on one side keeps predicting the other side of a true
//! join, but not a foreign chunk. Folder and sequence number are weak
//! tie-breakers. Rules and thresholds: docs/CHUNK-ERKENNUNG.md.

use crate::i18n::{t, tf, Msg};
use crate::wav::{self, Bext, WavInfo};
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub const OUTPUT_DIR_NAME: &str = "tracks";
const MIB: u64 = 1 << 20;
const FINGERPRINT_BLOCK: u64 = 64 * 1024;
/// Frames used on each side of a join to train the predictor.
const JOIN_CONTEXT: u64 = 4800;
const LPC_ORDER: usize = 32;
/// Samples predicted across the join.
const JOIN_PROBE: usize = 8;
const ENERGY_FLOOR: f64 = 1e-16;
const CONTINUITY_UNKNOWN: f64 = 1.5;
const PENALTY_OTHER_FOLDER: f64 = 0.25;
const GAP_WEIGHT: f64 = 0.5;
const AMBIGUITY_MARGIN: f64 = 0.5;
/// Score of a sample-exact TimeReference link before the seam cost: below every time-based link.
const EXACT_LINK_SCORE: f64 = -20.0;
/// A continuation's TimeReference may differ from predecessor + frames by this many samples.
const EXACT_SAMPLES: i128 = 2;
/// Larger TimeReference differences up to this many seconds (timecode quantised to frames)
/// still count as a time match, but not as exact.
const TIME_REF_LOOSE: f64 = 0.05;
/// TimeReference refines the start only if it agrees with OriginationTime this closely (s);
/// otherwise it is free-running timecode, still good for exact links.
const TIME_REF_CLOCK_AGREEMENT: f64 = 2.0;
/// The creation time replaces mtime − duration only if both agree this closely (s);
/// a copy gets a new creation time.
const BIRTH_AGREEMENT: f64 = 3.0;
const SEQ_SAME_BONUS: f64 = 0.25;
const SEQ_OTHER_PENALTY: f64 = 1.0;
/// A link resting on file dates alone needs a seam at least this clean
/// (38 real joins: true −1.1…1.3, foreign 1.3…7.9).
const FILE_LINK_MAX_COST: f64 = 3.0;
/// File size limits at which recorders split (2 GB, 2 GiB, 4 GB, 4 GiB − 1) …
const SPLIT_LIMITS: [u64; 4] = [2_000_000_000, 1 << 31, 4_000_000_000, (1 << 32) - 1];
/// … and how far below a limit a cut-off chunk may end (the last write block did not fit).
const SPLIT_MARGIN: u64 = 16 * MIB;
/// Recorders that split by time (one-hour segments).
const SPLIT_DURATION: f64 = 3600.0;
const SPLIT_DURATION_MARGIN: f64 = 1.0;
/// Score of an iXML file-set link before the seam cost: the strongest evidence.
const FILE_SET_LINK_SCORE: f64 = -30.0;
/// Seam cost up to which the audio confirms a time-based link (true joins measured ≤ 1.3).
const SEAM_OK: f64 = 2.0;
/// Seam cost above which the audio contradicts a link.
const SEAM_CONTRADICTS: f64 = FILE_LINK_MAX_COST;
/// Seconds at the end of A and the start of B whose noise floors are compared.
/// Calibration output only: on 26 real lavalier joins true jumps reached 33 dB and
/// foreign ones went down to 0.2 dB, so the floor cannot decide a link.
const NOISE_WINDOW: f64 = 2.0;
/// Earlier start years mean the recorder's clock was not set.
const PLAUSIBLE_YEAR: i64 = 2010;

#[derive(Debug, Clone)]
pub struct Options {
    /// Seconds a follow-up chunk may deviate from the predecessor's end (metadata or name times).
    pub gap_tolerance: f64,
    /// The same when a start rests on file dates (2 s FAT resolution at both ends).
    pub file_time_tolerance: f64,
    pub max_depth: usize,
    /// Smallest data size a chunk cut by the recorder can have.
    pub min_chunk_bytes: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self { gap_tolerance: 3.0, file_time_tolerance: 5.0, max_depth: 24, min_chunk_bytes: 100 * MIB }
    }
}

/// Where a chunk's start time comes from, best first. Serialized as "bext", "name", "file".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeSource {
    Bext,
    Name,
    File,
}

/// How sure a link between two chunks is. Serialized as "low", "medium", "high".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// Ambiguous, the audio contradicts the time evidence, or the predecessor is no full chunk.
    Low,
    /// Name or file time within tolerance, full-chunk pattern, seam confirmed.
    Medium,
    /// iXML file set or bext TimeReference exact, seam not contradicting.
    High,
}

impl TimeSource {
    fn tolerance(self, opt: &Options) -> f64 {
        match self {
            TimeSource::Bext | TimeSource::Name => opt.gap_tolerance,
            TimeSource::File => opt.file_time_tolerance,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Part {
    pub path: String,
    pub name: String,
    pub folder: String,
    /// Sequence number of a `<PREFIX>_<seq>_<YYYYMMDD>_<HHMMSS>` name, otherwise null.
    pub seq: Option<u32>,
    pub start: String,
    pub time_source: TimeSource,
    pub duration: f64,
    pub size: u64,
    pub gap_to_prev: Option<f64>,
    /// Confidence of the link from the previous part (null for the first part).
    pub link_confidence: Option<Confidence>,
    pub full_chunk: bool,
    pub repaired: bool,
    #[serde(skip)]
    pub path_buf: PathBuf,
    #[serde(skip)]
    pub folder_path: PathBuf,
    #[serde(skip)]
    pub start_secs: i64,
    /// Start in seconds since 1970 on the local clock, with the fraction the source has.
    #[serde(skip)]
    pub start_exact: f64,
    /// Start from the file dates (local clock), whatever the primary source is.
    #[serde(skip)]
    pub file_start: Option<f64>,
    /// bext OriginationDate in days since 1970.
    #[serde(skip)]
    pub bext_day: Option<i64>,
    #[serde(skip)]
    pub info: WavInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recording {
    pub id: usize,
    pub label: String,
    pub seq: Option<u32>,
    pub date: String,
    pub start: String,
    pub end: String,
    pub duration: f64,
    pub format: String,
    pub data_bytes: u64,
    pub output_bytes: u64,
    pub out_name: String,
    /// Weakest link of the chain (null for a single file).
    pub confidence: Option<Confidence>,
    pub parts: Vec<Part>,
    pub warnings: Vec<String>,
    #[serde(skip)]
    pub start_secs: i64,
    #[serde(skip)]
    pub frames: u64,
}

impl Recording {
    pub fn fmt(&self) -> &[u8] {
        &self.parts[0].info.fmt
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Scan {
    pub roots: Vec<String>,
    pub default_out_dir: String,
    pub files_seen: usize,
    pub recordings: Vec<Recording>,
    pub duplicates: Vec<Skipped>,
    pub ignored: Vec<Skipped>,
}

pub fn scan(inputs: &[PathBuf], opt: &Options) -> Result<Scan, String> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut walk_dirs: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut ignored = Vec::new();

    for input in inputs {
        let meta = fs::metadata(input).map_err(|e| format!("{}: {e}", input.display()))?;
        if meta.is_dir() {
            push_unique(&mut roots, input.clone());
            push_unique(&mut walk_dirs, input.clone());
        } else if is_wav_name(input) {
            files.push(input.clone());
            if let Some(parent) = input.parent() {
                push_unique(&mut roots, parent.to_path_buf());
            }
        } else {
            ignored.push(Skipped { path: input.display().to_string(), reason: t(Msg::NotWav).into() });
        }
    }
    if roots.is_empty() {
        return Err(t(Msg::NoFoldersOrWavs).into());
    }

    let out_dir = default_out_dir(&roots);
    let mut skip: Vec<PathBuf> = roots.iter().map(|r| r.join(OUTPUT_DIR_NAME)).collect();
    skip.push(out_dir.clone());
    for dir in &walk_dirs {
        walk(dir, 0, opt.max_depth, &skip, &mut files);
    }
    // Results of earlier runs lying around elsewhere are neither input nor noise.
    files.retain(|p| !p.file_name().map_or(false, |n| is_own_output(&n.to_string_lossy())));
    files.sort();
    files.dedup();
    let files_seen = files.len();

    let mut parts = Vec::new();
    for path in files {
        match load_part(&path) {
            Ok(p) => parts.push(p),
            Err(reason) => ignored.push(Skipped { path: path.display().to_string(), reason }),
        }
    }

    let (parts, duplicates) = dedupe(parts);
    let recordings = group(parts, opt);

    Ok(Scan {
        roots: roots.iter().map(|r| r.display().to_string()).collect(),
        default_out_dir: out_dir.display().to_string(),
        files_seen,
        recordings,
        duplicates,
        ignored,
    })
}

/// One folder: `<folder>/tracks`. Several folders (e.g. the two recorder
/// folders of one day): `tracks` next to them, in their common parent.
fn default_out_dir(roots: &[PathBuf]) -> PathBuf {
    if roots.len() > 1 {
        let mut common = roots[0].clone();
        for r in &roots[1..] {
            while !r.starts_with(&common) {
                if !common.pop() {
                    break;
                }
            }
        }
        if common.parent().is_some() {
            return common.join(OUTPUT_DIR_NAME);
        }
    }
    roots[0].join(OUTPUT_DIR_NAME)
}

/// Matches names this app writes: `yymmdd_SHHMMSS-EHHMMSS_DHHMMSS_<label>.wav`.
fn is_own_output(name: &str) -> bool {
    let b = name.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    b.len() > 35
        && name.to_ascii_lowercase().ends_with(".wav")
        && digits(0..6)
        && &b[6..8] == b"_S"
        && digits(8..14)
        && &b[14..16] == b"-E"
        && digits(16..22)
        && &b[22..24] == b"_D"
        && digits(24..30)
        && b[30] == b'_'
}

fn push_unique(v: &mut Vec<PathBuf>, p: PathBuf) {
    if !v.contains(&p) {
        v.push(p);
    }
}

fn is_wav_name(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    !name.starts_with("._")
        && path.extension().map_or(false, |e| e.eq_ignore_ascii_case("wav"))
}

fn walk(dir: &Path, depth: usize, max_depth: usize, skip: &[PathBuf], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        // file_type() does not follow symlinks, so link loops cannot trap us.
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if depth < max_depth && !skip.contains(&path) {
                walk(&path, depth + 1, max_depth, skip, out);
            }
        } else if ft.is_file() && is_wav_name(&path) {
            out.push(path);
        }
    }
}

/// Reads one candidate file. Errors are user-facing reasons for skipping it.
pub fn load_part(path: &Path) -> Result<Part, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let info = wav::read_info(path).map_err(|e| tf(Msg::UnreadableWith, &[("e", &e)]))?;
    if info.data_len == 0 {
        return Err(t(Msg::NoAudioData).into());
    }
    let duration = info.duration();
    let file_start = fs::metadata(path).ok().and_then(|m| file_start_time(&m, duration));
    let (seq, name_secs) = match parse_chunk_name(&name) {
        Some((seq, secs)) => (Some(seq), Some(secs)),
        None => (None, parse_name_time(&name)),
    };
    let bext_day = info.bext.as_ref().and_then(|b| b.date).map(|(y, m, d)| days_from_civil(y, m, d));
    let bext = info.bext.as_ref().and_then(|b| bext_start(b, info.sample_rate));
    let (time_source, start_exact) = match (bext, name_secs, file_start) {
        (Some(secs), _, _) => (TimeSource::Bext, secs),
        (None, Some(secs), _) => (TimeSource::Name, secs as f64),
        (None, None, Some(secs)) => (TimeSource::File, secs),
        (None, None, None) => return Err(t(Msg::Unreadable).into()),
    };
    let start_secs = start_exact.floor() as i64;
    let folder_path = path.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok(Part {
        path: path.display().to_string(),
        name,
        folder: folder_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        seq,
        start: fmt_datetime(start_secs),
        time_source,
        duration,
        size: info.file_size,
        gap_to_prev: None,
        link_confidence: None,
        full_chunk: false,
        repaired: info.repaired,
        path_buf: path.to_path_buf(),
        folder_path,
        start_secs,
        start_exact,
        file_start,
        bext_day,
        info,
    })
}

/// Local start from bext OriginationDate + OriginationTime, refined to the
/// sample by TimeReference when that is the time of day (agrees with the clock).
fn bext_start(b: &Bext, sample_rate: u32) -> Option<f64> {
    let (y, m, d) = b.date?;
    let clock = b.time? as f64;
    let mut secs = clock;
    if b.time_reference > 0 {
        let tr = b.time_reference as f64 / sample_rate as f64;
        // Nearest to the clock, allowing for midnight between the two.
        let tr = [tr - 86_400.0, tr, tr + 86_400.0]
            .into_iter()
            .min_by(|x, y| (x - clock).abs().total_cmp(&(y - clock).abs()))
            .unwrap_or(tr);
        if (tr - clock).abs() <= TIME_REF_CLOCK_AGREEMENT {
            secs = tr;
        }
    }
    Some((days_from_civil(y, m, d) * 86_400) as f64 + secs)
}

/// Start from the file dates on the local clock: the modification time marks
/// the end of writing, so start = mtime − duration. The creation time is more
/// precise on memory cards but is reset by copying; it is used only if it agrees.
pub(crate) fn file_start_time(meta: &fs::Metadata, duration: f64) -> Option<f64> {
    let unix = |t: std::time::SystemTime| t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs_f64());
    let mut start = unix(meta.modified().ok()?)? - duration;
    if let Some(birth) = meta.created().ok().and_then(unix) {
        if (birth - start).abs() <= BIRTH_AGREEMENT {
            start = birth;
        }
    }
    Some(start + local_offset(start.floor() as i64) as f64)
}

/// Seconds the local clock is ahead of UTC at `t` (names and bext use local time).
#[cfg(unix)]
pub(crate) fn local_offset(t: i64) -> i64 {
    let tt = t as libc::time_t;
    // SAFETY: localtime_r only writes into the zeroed tm we own.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&tt, &mut tm).is_null() {
            0
        } else {
            tm.tm_gmtoff as i64
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn local_offset(_t: i64) -> i64 {
    0
}

/// Whether a file ends where recorders typically cut a long recording: at most
/// 16 MiB below a 2 GB, 2 GiB, 4 GB or 4 GiB size limit, or at one hour (±1 s).
pub fn at_split_limit(file_size: u64, duration: f64) -> bool {
    SPLIT_LIMITS.iter().any(|&l| file_size <= l && file_size + SPLIT_MARGIN > l)
        || (duration - SPLIT_DURATION).abs() <= SPLIT_DURATION_MARGIN
}

/// Parses `<PREFIX>_<seq>_<YYYYMMDD>_<HHMMSS>` anywhere in a file name, the
/// prefix being 1–16 letters or digits (`REC_01_20260906_112326.WAV`,
/// `ZOOM_0007_…`, also copies like `Kopie von rec_01_20260908_072908 (1).wav`).
/// Returns the sequence number and the start as seconds since 1970 (local clock).
pub fn parse_chunk_name(name: &str) -> Option<(u32, i64)> {
    let b = name.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c != b'_' {
            continue;
        }
        let prefix = b[..i].iter().rev().take_while(|c| c.is_ascii_alphanumeric()).count();
        if !(1..=16).contains(&prefix) {
            continue;
        }
        if let Some(r) = parse_after_prefix(&b[i + 1..]) {
            return Some(r);
        }
    }
    None
}

fn parse_after_prefix(b: &[u8]) -> Option<(u32, i64)> {
    let digits = |s: &[u8]| -> Option<i64> {
        if s.iter().all(u8::is_ascii_digit) {
            Some(s.iter().fold(0i64, |a, c| a * 10 + (c - b'0') as i64))
        } else {
            None
        }
    };
    let seq_len = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if seq_len == 0 || seq_len > 6 {
        return None;
    }
    let rest = &b[seq_len..];
    if rest.len() < 16 || rest[0] != b'_' || rest[9] != b'_' {
        return None;
    }
    if rest.len() > 16 && rest[16].is_ascii_digit() {
        return None;
    }
    let seq = digits(&b[..seq_len])? as u32;
    let (y, mo, d) = (digits(&rest[1..5])?, digits(&rest[5..7])?, digits(&rest[7..9])?);
    let (h, mi, s) = (digits(&rest[10..12])?, digits(&rest[12..14])?, digits(&rest[14..16])?);
    // Unset clocks write 1970, 1980, 2000 …: kept (the recording gets a warning).
    if y < 1970 || !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    Some((seq, days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s))
}

/// Finds a date and time with seconds in any file name, each group delimited by
/// non-digits: `YYYYMMDD[_- T]HHMMSS`, `YYMMDD[_-]HHMMSS`,
/// `YYYY-MM-DD[_- T]HHMMSS` or `YYYY-MM-DD[_- T]HH-MM-SS` (also `HH.MM.SS`).
/// Years 1970–2099 (two-digit years 2000–2099), real calendar dates. Returns local seconds since 1970.
pub fn parse_name_time(name: &str) -> Option<i64> {
    let b = name.as_bytes();
    (0..b.len())
        .filter(|&i| b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_digit()))
        .find_map(|i| name_time_at(&b[i..]))
}

fn name_time_at(b: &[u8]) -> Option<i64> {
    let run = |s: &[u8]| s.iter().take_while(|c| c.is_ascii_digit()).count();
    let num = |s: &[u8]| s.iter().fold(0i64, |a, c| a * 10 + (c - b'0') as i64);
    let first = run(b);
    let (y, mo, d, rest) = match first {
        8 => (num(&b[0..4]), num(&b[4..6]), num(&b[6..8]), &b[8..]),
        6 => (2000 + num(&b[0..2]), num(&b[2..4]), num(&b[4..6]), &b[6..]),
        4 if b.len() >= 10 && b[4] == b'-' && run(&b[5..]) == 2 && b[7] == b'-' && run(&b[8..]) == 2 => {
            (num(&b[0..4]), num(&b[5..7]), num(&b[8..10]), &b[10..])
        }
        _ => return None,
    };
    let seps: &[u8] = if first == 6 { b"_-" } else { b"_- T" };
    if !seps.contains(rest.first()?) {
        return None;
    }
    let tm = &rest[1..];
    let (h, mi, s, after) = if run(tm) == 6 {
        (num(&tm[0..2]), num(&tm[2..4]), num(&tm[4..6]), &tm[6..])
    } else if first == 4
        && tm.len() >= 8
        && run(tm) == 2
        && matches!(tm[2], b'-' | b'.')
        && tm[5] == tm[2]
        && run(&tm[3..]) == 2
        && run(&tm[6..]) == 2
    {
        (num(&tm[0..2]), num(&tm[3..5]), num(&tm[6..8]), &tm[8..])
    } else {
        return None;
    };
    if after.first().map_or(false, u8::is_ascii_digit) {
        return None;
    }
    valid_datetime(y, mo, d, h, mi, s)
}

fn valid_datetime(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Option<i64> {
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let ok = (1970..=2099).contains(&y)
        && (1..=12).contains(&mo)
        && d >= 1
        && d <= month_days[(mo - 1) as usize]
        && h <= 23
        && mi <= 59
        && s <= 59;
    ok.then(|| days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s)
}

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// (year, month, day, hour, minute, second)
pub fn civil_from_secs(t: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = t.div_euclid(86400);
    let sod = t.rem_euclid(86400);
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d, sod / 3600, sod % 3600 / 60, sod % 60)
}

fn fmt_datetime(t: i64) -> String {
    let (y, mo, d, h, mi, s) = civil_from_secs(t);
    format!("{d:02}.{mo:02}.{y} {h:02}:{mi:02}:{s:02}")
}

/// Removes byte-identical copies (same recorded time, format, length and
/// content samples), keeping the first path in sort order.
fn dedupe(parts: Vec<Part>) -> (Vec<Part>, Vec<Skipped>) {
    let mut len_count: HashMap<u64, usize> = HashMap::new();
    for p in &parts {
        *len_count.entry(p.info.data_len).or_default() += 1;
    }
    let mut seen: HashMap<(u64, Vec<u8>, u64), usize> = HashMap::new();
    let mut kept: Vec<Part> = Vec::new();
    let mut dups = Vec::new();
    for p in parts {
        if len_count[&p.info.data_len] < 2 {
            kept.push(p);
            continue;
        }
        let Some(fp) = fingerprint(&p) else {
            kept.push(p);
            continue;
        };
        let key = (p.info.data_len, p.info.fmt.clone(), fp);
        if let Some(&i) = seen.get(&key) {
            dups.push(Skipped { path: p.path.clone(), reason: tf(Msg::IdenticalCopy, &[("path", &kept[i].path)]) });
        } else {
            seen.insert(key, kept.len());
            kept.push(p);
        }
    }
    (kept, dups)
}

fn fingerprint(p: &Part) -> Option<u64> {
    let len = p.info.data_len;
    let n = FINGERPRINT_BLOCK.min(len);
    let mut h = DefaultHasher::new();
    p.seq.hash(&mut h);
    // A copy keeps its name and metadata, but not necessarily its file dates.
    if p.time_source != TimeSource::File {
        p.start_secs.hash(&mut h);
    }
    p.info.bext.as_ref().map(|b| b.time_reference).hash(&mut h);
    for k in 0..4u64 {
        let off = (len - n) * k / 3;
        wav::read_at(&p.path_buf, p.info.data_offset + off, n).ok()?.hash(&mut h);
    }
    Some(h.finish())
}

fn read_frames(p: &Part, from: u64, count: u64) -> Option<Vec<f64>> {
    let info = &p.info;
    let kind = info.sample_kind()?;
    let ba = info.block_align as usize;
    let bytes_per_sample = ba / info.channels as usize;
    let raw = wav::read_at(&p.path_buf, info.data_offset + from * ba as u64, count * ba as u64).ok()?;
    Some(raw.chunks_exact(ba).map(|fr| wav::decode_sample(&fr[..bytes_per_sample], kind)).collect())
}

/// Least-squares linear predictor (coefficients oldest sample first) and the
/// mean squared residual inside the training signal.
fn fit_predictor(x: &[f64]) -> Option<(Vec<f64>, f64)> {
    let p = LPC_ORDER;
    let rows = x.len().checked_sub(p)?;
    if rows < p * 4 {
        return None;
    }
    let mut ata = vec![0.0; p * p];
    let mut aty = vec![0.0; p];
    for k in 0..rows {
        let row = &x[k..k + p];
        let y = x[k + p];
        for i in 0..p {
            aty[i] += row[i] * y;
            for j in 0..=i {
                ata[i * p + j] += row[i] * row[j];
            }
        }
    }
    let trace: f64 = (0..p).map(|i| ata[i * p + i]).sum();
    let ridge = trace / p as f64 * 1e-9 + 1e-30;
    for i in 0..p {
        ata[i * p + i] += ridge;
    }
    let coef = solve_cholesky(&mut ata, &mut aty, p)?;
    let err: f64 = (0..rows).map(|k| (x[k + p] - predict(&x[k..k + p], &coef)).powi(2)).sum();
    Some((coef, err / rows as f64))
}

fn predict(history: &[f64], coef: &[f64]) -> f64 {
    history.iter().zip(coef).map(|(h, c)| h * c).sum()
}

/// Solves the symmetric positive definite system `a x = b` (lower triangle of `a` used).
fn solve_cholesky(a: &mut [f64], b: &mut [f64], n: usize) -> Option<Vec<f64>> {
    for j in 0..n {
        let d = a[j * n + j] - (0..j).map(|k| a[j * n + k] * a[j * n + k]).sum::<f64>();
        if !(d > 0.0) {
            return None;
        }
        let d = d.sqrt();
        a[j * n + j] = d;
        for i in j + 1..n {
            let s = a[i * n + j] - (0..j).map(|k| a[i * n + k] * a[j * n + k]).sum::<f64>();
            a[i * n + j] = s / d;
        }
    }
    for i in 0..n {
        let s = b[i] - (0..i).map(|k| a[i * n + k] * b[k]).sum::<f64>();
        b[i] = s / a[i * n + i];
    }
    for i in (0..n).rev() {
        let s = b[i] - (i + 1..n).map(|k| a[k * n + i] * b[k]).sum::<f64>();
        b[i] = s / a[i * n + i];
    }
    Some(b.to_vec())
}

/// Mean squared error when predicting the first samples of `next` from the end of `context`.
fn join_error(context: &[f64], coef: &[f64], next: &[f64]) -> f64 {
    let p = coef.len();
    let mut seq = context[context.len() - p..].to_vec();
    seq.extend_from_slice(&next[..JOIN_PROBE]);
    (0..JOIN_PROBE).map(|k| (seq[k + p] - predict(&seq[k..k + p], coef)).powi(2)).sum::<f64>() / JOIN_PROBE as f64
}

/// How badly the audio breaks when `b` is appended to `a` (first channel):
/// log10 of the prediction error across the join relative to the error inside
/// the signal, forward from `a` plus backward from `b`. Measured on 38 real
/// DJI Mic 2 joins: true continuations −1.1…1.3, foreign chunks 1.3…7.9.
pub fn continuity_cost(a: &Part, b: &Part) -> Option<f64> {
    let n = JOIN_CONTEXT.min(a.info.frames()).min(b.info.frames());
    let tail = read_frames(a, a.info.frames() - n, n)?;
    let head = read_frames(b, 0, n)?;
    let (fwd_coef, fwd_ref) = fit_predictor(&tail)?;
    let forward = (join_error(&tail, &fwd_coef, &head) + ENERGY_FLOOR) / (fwd_ref + ENERGY_FLOOR);
    let head_rev: Vec<f64> = head.iter().rev().copied().collect();
    let tail_rev: Vec<f64> = tail.iter().rev().copied().collect();
    let (bwd_coef, bwd_ref) = fit_predictor(&head_rev)?;
    let backward = (join_error(&head_rev, &bwd_coef, &tail_rev) + ENERGY_FLOOR) / (bwd_ref + ENERGY_FLOOR);
    Some((forward.log10() + backward.log10()).clamp(-2.0, 8.0))
}

/// Noise floor (dB) of the first channel in `count` frames from `from`: the
/// 10th percentile of the RMS of 50 ms blocks.
fn noise_floor_db(p: &Part, from: u64, count: u64) -> Option<f64> {
    let x = read_frames(p, from, count)?;
    let block = (p.info.sample_rate as usize / 20).max(1);
    let mut power: Vec<f64> = x.chunks_exact(block).map(|c| c.iter().map(|v| v * v).sum::<f64>() / c.len() as f64).collect();
    if power.len() < 4 {
        return None;
    }
    power.sort_by(f64::total_cmp);
    Some(10.0 * (power[power.len() / 10] + 1e-12).log10())
}

/// Difference (dB) between the noise floor of the last ~2 s of `a` and the first ~2 s of `b`.
/// Reported by the real-data check; not used for decisions (see NOISE_WINDOW).
pub fn noise_floor_jump(a: &Part, b: &Part) -> Option<f64> {
    let n = (NOISE_WINDOW * a.info.sample_rate as f64) as u64;
    let (na, nb) = (n.min(a.info.frames()), n.min(b.info.frames()));
    Some((noise_floor_db(a, a.info.frames() - na, na)? - noise_floor_db(b, 0, nb)?).abs())
}

/// Why `b` may follow `a`, with the time difference at the join in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Evidence {
    /// iXML: same FAMILY_UID, FILE_SET_INDEX of `b` follows that of `a`.
    FileSet { gap: f64 },
    /// bext TimeReference of `b` is TimeReference of `a` plus its frames.
    Exact { gap: f64 },
    /// Start of `b` near the end of `a`, dated by `source` (the worse of both).
    Time { source: TimeSource, gap: f64 },
}

/// "2" after "1", "B" after "A".
fn index_follows(a: &str, b: &str) -> bool {
    if let (Ok(x), Ok(y)) = (a.parse::<u64>(), b.parse::<u64>()) {
        return y == x + 1;
    }
    let (x, y) = (a.as_bytes(), b.as_bytes());
    x.len() == 1 && y.len() == 1 && x[0].is_ascii_alphabetic() && y[0] == x[0] + 1
}

/// iXML says `b` is the next file of `a`'s set. Files of a set that start
/// together are the channels of one take, not consecutive chunks.
fn file_set_follows(a: &Part, b: &Part, opt: &Options) -> bool {
    let (Some(xa), Some(xb)) = (&a.info.ixml, &b.info.ixml) else { return false };
    let together = a.time_source.max(b.time_source).tolerance(opt).min(a.duration / 2.0);
    xa.family_uid == xb.family_uid && index_follows(&xa.index, &xb.index) && b.start_exact - a.start_exact > together
}

/// The time test for `a → b` (same format already checked).
fn link_evidence(a: &Part, b: &Part, a_can_continue: bool, opt: &Options) -> Option<Evidence> {
    let time_ref = |p: &Part| p.info.bext.as_ref().map(|x| x.time_reference).filter(|&tr| tr > 0);
    // Both carry a TimeReference: a deviation means a gap or a foreign chunk, so no link.
    let mut by_time_ref = None;
    if let (Some(ta), Some(tb)) = (time_ref(a), time_ref(b)) {
        if let (Some(da), Some(db)) = (a.bext_day, b.bext_day) {
            if (db - da).abs() > 1 {
                return None;
            }
        }
        let sr = a.info.sample_rate as i128;
        let day = 86_400 * sr;
        // Samples since midnight: compare modulo one day, so a join across midnight fits.
        let mut diff = (tb as i128 - ta as i128 - a.info.frames() as i128).rem_euclid(day);
        if diff > day / 2 {
            diff -= day;
        }
        let gap = diff as f64 / sr as f64;
        by_time_ref = if diff.abs() <= EXACT_SAMPLES {
            Some(Evidence::Exact { gap })
        } else if gap.abs() <= TIME_REF_LOOSE {
            Some(Evidence::Time { source: TimeSource::Bext, gap })
        } else {
            return None;
        };
    }
    if file_set_follows(a, b, opt) {
        let gap = match by_time_ref {
            Some(Evidence::Exact { gap } | Evidence::Time { gap, .. }) => gap,
            _ => b.start_exact - (a.start_exact + a.duration),
        };
        return Some(Evidence::FileSet { gap });
    }
    match by_time_ref {
        Some(e @ Evidence::Exact { .. }) => return Some(e),
        Some(e) => return a_can_continue.then_some(e),
        None => {}
    }
    if !a_can_continue {
        return None;
    }
    let source = a.time_source.max(b.time_source);
    let gap = b.start_exact - (a.start_exact + a.duration);
    if gap.abs() <= source.tolerance(opt) {
        return Some(Evidence::Time { source, gap });
    }
    // Some recorders give a continuation the first chunk's name or clock time; the
    // file dates may still fit. Chunk names with a sequence number are trusted as they are.
    if source == TimeSource::File || a.seq.is_some() || b.seq.is_some() {
        return None;
    }
    let gap = b.file_start? - (a.file_start? + a.duration);
    (gap.abs() <= opt.file_time_tolerance).then_some(Evidence::Time { source: TimeSource::File, gap })
}

struct Link {
    a: usize,
    b: usize,
    score: f64,
    evidence: Evidence,
    cost: Option<f64>,
}

impl Link {
    fn gap(&self) -> f64 {
        match self.evidence {
            Evidence::FileSet { gap } | Evidence::Exact { gap } | Evidence::Time { gap, .. } => gap,
        }
    }

    fn file_only(&self) -> bool {
        matches!(self.evidence, Evidence::Time { source: TimeSource::File, .. })
    }

    fn confidence(&self, a_full: bool, ambiguous: bool) -> Confidence {
        let contradicts = self.cost.map_or(false, |c| c > SEAM_CONTRADICTS);
        if ambiguous || contradicts {
            return Confidence::Low;
        }
        match self.evidence {
            Evidence::FileSet { .. } | Evidence::Exact { .. } => Confidence::High,
            Evidence::Time { .. } if a_full && self.cost.map_or(false, |c| c <= SEAM_OK) => Confidence::Medium,
            Evidence::Time { .. } => Confidence::Low,
        }
    }
}

/// What the chain knows about the link into a part.
#[derive(Debug, Clone, Copy)]
struct Join {
    gap: f64,
    file_only: bool,
    confidence: Confidence,
}

/// Whether following `next` from `from` arrives at `to` (a new link to→from would close a loop).
fn reaches(next: &[Option<usize>], from: usize, to: usize) -> bool {
    let mut cur = Some(from);
    while let Some(i) = cur {
        if i == to {
            return true;
        }
        cur = next[i];
    }
    false
}

fn group(mut parts: Vec<Part>, opt: &Options) -> Vec<Recording> {
    parts.sort_by(|a, b| a.start_exact.total_cmp(&b.start_exact).then_with(|| a.path.cmp(&b.path)));

    // Chunks cut by the recorder all have the same byte size.
    // Fixed chunking also gives every full chunk the same frame count, even when
    // metadata of varying length makes the file sizes differ.
    let mut size_count: HashMap<u64, usize> = HashMap::new();
    let mut frame_count: HashMap<(Vec<u8>, u64), usize> = HashMap::new();
    let mut largest: HashMap<Vec<u8>, u64> = HashMap::new();
    let mut known_size: HashMap<Vec<u8>, bool> = HashMap::new();
    for p in &parts {
        *size_count.entry(p.size).or_default() += 1;
        *frame_count.entry((p.info.fmt.clone(), p.info.frames())).or_default() += 1;
        let l = largest.entry(p.info.fmt.clone()).or_default();
        *l = (*l).max(p.size);
        *known_size.entry(p.info.fmt.clone()).or_default() |= p.size == wav::CHUNK_FILE_SIZE;
    }
    for p in parts.iter_mut() {
        let repeated = size_count[&p.size] >= 2 || frame_count[&(p.info.fmt.clone(), p.info.frames())] >= 2;
        p.full_chunk = p.size == wav::CHUNK_FILE_SIZE
            || (p.info.data_len >= opt.min_chunk_bytes && (repeated || at_split_limit(p.size, p.duration)));
    }
    // Only a chunk the recorder cut off can have a continuation. If no file of
    // this format has the known DJI chunk size, the largest one may be a chunk too.
    let can_continue: Vec<bool> = parts
        .iter()
        .map(|p| {
            p.full_chunk
                || (!known_size[&p.info.fmt] && p.info.data_len >= opt.min_chunk_bytes && p.size == largest[&p.info.fmt])
        })
        .collect();
    let fmt_id: Vec<usize> = {
        let mut ids: HashMap<&[u8], usize> = HashMap::new();
        parts
            .iter()
            .map(|p| {
                let k = ids.len();
                *ids.entry(&p.info.fmt).or_insert(k)
            })
            .collect()
    };

    let n = parts.len();
    let mut links: Vec<Link> = Vec::new();
    for a in 0..n {
        for b in 0..n {
            if a == b || fmt_id[a] != fmt_id[b] {
                continue;
            }
            let Some(evidence) = link_evidence(&parts[a], &parts[b], can_continue[a], opt) else { continue };
            let cost = continuity_cost(&parts[a], &parts[b]);
            let seam = cost.unwrap_or(CONTINUITY_UNKNOWN);
            let mut score = match evidence {
                Evidence::FileSet { .. } => FILE_SET_LINK_SCORE + seam,
                Evidence::Exact { .. } => EXACT_LINK_SCORE + seam,
                Evidence::Time { source, gap } => {
                    if source == TimeSource::File && cost.map_or(false, |c| c > FILE_LINK_MAX_COST) {
                        continue;
                    }
                    gap.abs() * GAP_WEIGHT + seam
                }
            };
            if parts[a].folder_path != parts[b].folder_path {
                score += PENALTY_OTHER_FOLDER;
            }
            match (parts[a].seq, parts[b].seq) {
                (Some(x), Some(y)) if x == y => score -= SEQ_SAME_BONUS,
                (Some(_), Some(_)) => score += SEQ_OTHER_PENALTY,
                _ => {}
            }
            links.push(Link { a, b, score, evidence, cost });
        }
    }

    links.sort_by(|x, y| x.score.total_cmp(&y.score));
    let mut next: Vec<Option<usize>> = vec![None; n];
    let mut prev: Vec<Option<usize>> = vec![None; n];
    let mut join: Vec<Option<Join>> = vec![None; n];
    let mut chosen = Vec::new();
    for (i, l) in links.iter().enumerate() {
        if next[l.a].is_none() && prev[l.b].is_none() && !reaches(&next, l.b, l.a) {
            next[l.a] = Some(l.b);
            prev[l.b] = Some(l.a);
            chosen.push(i);
        }
    }
    let mut ambiguous = vec![false; n];
    for &i in &chosen {
        let c = &links[i];
        let rival = links
            .iter()
            .enumerate()
            .any(|(j, o)| j != i && (o.a == c.a || o.b == c.b) && o.score - c.score < AMBIGUITY_MARGIN);
        if rival {
            ambiguous[c.a] = true;
            ambiguous[c.b] = true;
        }
        join[c.b] = Some(Join { gap: c.gap(), file_only: c.file_only(), confidence: c.confidence(parts[c.a].full_chunk, rival) });
    }

    let mut slots: Vec<Option<Part>> = parts.into_iter().map(Some).collect();
    let mut recordings = Vec::new();
    for head in 0..n {
        if prev[head].is_some() {
            continue;
        }
        let mut chain = Vec::new();
        let mut joins = Vec::new();
        let mut amb = false;
        let mut cur = Some(head);
        while let Some(i) = cur {
            amb |= ambiguous[i];
            chain.push(slots[i].take().expect("chunk assigned twice"));
            joins.push(join[i]);
            cur = next[i];
        }
        recordings.push(build_recording(chain, &joins, amb));
    }

    recordings.sort_by(|a, b| a.start_secs.cmp(&b.start_secs).then_with(|| a.label.cmp(&b.label)));
    let mut used: HashMap<String, usize> = HashMap::new();
    for (i, r) in recordings.iter_mut().enumerate() {
        r.id = i;
        let count = used.entry(r.out_name.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            r.out_name = format!("{}_{}.wav", r.out_name.trim_end_matches(".wav"), count);
        }
    }
    recordings
}

fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    t.trim_matches('-').chars().take(40).collect()
}

/// `joins[k]`: the link into part k (None for the first).
fn build_recording(mut parts: Vec<Part>, joins: &[Option<Join>], ambiguous: bool) -> Recording {
    for k in 1..parts.len() {
        let gap = joins[k]
            .map(|j| j.gap)
            .unwrap_or_else(|| parts[k].start_exact - (parts[k - 1].start_exact + parts[k - 1].duration));
        parts[k].gap_to_prev = Some((gap * 100.0).round() / 100.0);
        parts[k].link_confidence = joins[k].map(|j| j.confidence);
    }
    let confidence = joins.iter().flatten().map(|j| j.confidence).min();
    let first = &parts[0];
    let seq = first.seq;
    let start_secs = first.start_secs;
    let sample_rate = first.info.sample_rate;
    let fmt_len = first.info.fmt.len();
    let format = first.info.format_label();
    let mut label = sanitize(&first.folder);
    if label.is_empty() {
        label = seq.map_or_else(|| "REC".to_string(), |s| format!("REC{s:02}"));
    }

    let frames: u64 = parts.iter().map(|p| p.info.frames()).sum();
    let data_bytes: u64 = parts.iter().map(|p| p.info.data_len).sum();
    let duration = frames as f64 / sample_rate as f64;
    let dur = duration.round() as i64;
    let end_secs = start_secs + dur;

    let mut warnings = Vec::new();
    if ambiguous {
        warnings.push(t(Msg::WarnAmbiguous).to_string());
    } else if confidence == Some(Confidence::Low) {
        warnings.push(t(Msg::WarnLowConfidence).to_string());
    }
    let count = parts.len();
    for (k, p) in parts.iter().enumerate() {
        if p.repaired {
            warnings.push(tf(Msg::WarnRepaired, &[("n", &(k + 1))]));
        }
        if let Some(g) = p.gap_to_prev {
            if g.abs() > 1.5 {
                let gap = crate::i18n::decimal(crate::i18n::current(), &format!("{g:+.1}"));
                warnings.push(tf(Msg::WarnGap, &[("a", &k), ("b", &(k + 1)), ("gap", &gap)]));
            }
        }
        if joins[k].map_or(false, |j| j.file_only) {
            warnings.push(tf(Msg::WarnFileTimes, &[("a", &k), ("b", &(k + 1))]));
        }
        if k + 1 < count && !p.full_chunk {
            warnings.push(tf(Msg::WarnShortWithNext, &[("n", &(k + 1))]));
        }
    }
    if parts.last().map_or(false, |p| p.full_chunk) {
        warnings.push(t(Msg::WarnLastFull).to_string());
    }

    let (y, mo, d, h, mi, s) = civil_from_secs(start_secs);
    let (_, _, _, eh, emi, es) = civil_from_secs(end_secs);
    if y < PLAUSIBLE_YEAR {
        warnings.push(tf(Msg::WarnClockNotSet, &[("date", &format!("{d:02}.{mo:02}.{y}"))]));
    }
    let out_name = format!(
        "{:02}{mo:02}{d:02}_S{h:02}{mi:02}{s:02}-E{eh:02}{emi:02}{es:02}_D{:02}{:02}{:02}_{label}.wav",
        y % 100,
        dur / 3600,
        dur % 3600 / 60,
        dur % 60
    );

    Recording {
        id: 0,
        label,
        seq,
        date: format!("{d:02}.{mo:02}.{y}"),
        start: format!("{h:02}:{mi:02}:{s:02}"),
        end: format!("{eh:02}:{emi:02}:{es:02}"),
        duration,
        format,
        data_bytes,
        output_bytes: wav::output_size(fmt_len, data_bytes),
        out_name,
        confidence,
        parts,
        warnings,
        start_secs,
        frames,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::i18n::{self, Lang};
    use crate::testutil::*;

    /// Synthetic test chunks are tiny, so any repeated file size counts as a chunk.
    pub(crate) fn small() -> Options {
        Options { min_chunk_bytes: 0, ..Options::default() }
    }

    fn names(r: &Recording) -> Vec<&str> {
        r.parts.iter().map(|p| p.name.as_str()).collect()
    }

    /// 10.01.2026 10:00:00 UTC, a winter date (no daylight saving change nearby).
    const T0: f64 = 1_768_039_200.0;

    /// Real case from 06.09.2026: a short recording on transmitter 4 ended 3 s
    /// before transmitter 5 started with the same sequence number. It must not
    /// be taken for the first chunk of transmitter 5's recording. (Chunks here
    /// are 10 s; a real short recording is always smaller than a full chunk.)
    #[test]
    fn short_recording_is_never_continued() {
        let root = tempdir("short");
        let sr = 8000;
        let s4 = |start: u64, n: u64| sine(220.0, 0.5, sr, start, n);
        let s5 = |start: u64, n: u64| sine(331.0, 0.2, sr, start, n);
        write_float_wav(&root.join("4/DJI_06_20260906_173217.WAV"), sr, &s4(0, 72_000));
        write_float_wav(&root.join("5/DJI_06_20260906_173229.WAV"), sr, &s5(0, 80_000));
        write_float_wav(&root.join("5/DJI_06_20260906_173239.WAV"), sr, &s5(80_000, 32_000));
        write_float_wav(&root.join("4/DJI_07_20260906_173233.WAV"), sr, &s4(0, 80_000));
        write_float_wav(&root.join("4/DJI_07_20260906_173243.WAV"), sr, &s4(80_000, 20_000));
        let s = scan(&[root.clone()], &small()).unwrap();
        let shape: Vec<(String, usize)> = s.recordings.iter().map(|r| (r.parts[0].name.clone(), r.parts.len())).collect();
        assert_eq!(
            shape,
            [
                ("DJI_06_20260906_173217.WAV".to_string(), 1),
                ("DJI_06_20260906_173229.WAV".to_string(), 2),
                ("DJI_07_20260906_173233.WAV".to_string(), 2)
            ]
        );
    }

    /// Dropping the two recorder folders of a day puts `tracks` next to them,
    /// and old results (e.g. a `tracks` inside a recorder folder) are not listed.
    #[test]
    fn two_folders_share_one_tracks_folder() {
        let day = tempdir("day");
        let sr = 8000;
        write_float_wav(&day.join("4/DJI_01_20260906_100000.WAV"), sr, &sine(220.0, 0.5, sr, 0, 8000));
        write_float_wav(&day.join("5/DJI_01_20260906_100500.WAV"), sr, &sine(331.0, 0.2, sr, 0, 8000));
        write_float_wav(&day.join("4/tracks/260906_S100000-E100001_D000001_4.wav"), sr, &sine(220.0, 0.5, sr, 0, 8000));
        let s = scan(&[day.join("4"), day.join("5")], &small()).unwrap();
        assert_eq!(s.default_out_dir, day.join("tracks").display().to_string());
        assert_eq!(s.recordings.len(), 2);
        assert!(s.ignored.is_empty(), "{:?}", s.ignored);
        assert_eq!(s.files_seen, 2);
        let single = scan(&[day.join("4")], &small()).unwrap();
        assert_eq!(single.default_out_dir, day.join("4/tracks").display().to_string());
        assert!(is_own_output("260906_S173233-E181430_D004157_4_2.wav"));
        assert!(!is_own_output("DJI_07_20260906_173233.WAV"));
    }

    #[test]
    fn parses_names() {
        let t = days_from_civil(2026, 9, 8) * 86400 + 7 * 3600 + 29 * 60 + 8;
        assert_eq!(parse_chunk_name("DJI_01_20260908_072908.WAV"), Some((1, t)));
        assert_eq!(parse_chunk_name("dji_01_20260908_072908 (1).wav"), Some((1, t)));
        assert_eq!(parse_chunk_name("Kopie von DJI_1234_20260908_072908.WAV"), Some((1234, t)));
        assert_eq!(parse_chunk_name("DJI_01_20261308_072908.WAV"), None);
        assert_eq!(parse_chunk_name("DJI_01_20260908_0729081.WAV"), None);
        assert_eq!(parse_chunk_name("interview.wav"), None);
        assert_eq!(parse_chunk_name("REC_01_20260908_072908.WAV"), Some((1, t)));
        assert_eq!(parse_chunk_name("ZOOM_0007_20260908_072908.wav"), Some((7, t)));
        assert_eq!(parse_chunk_name("Kopie_von_zoom2_12_20260908_072908 (2).wav"), Some((12, t)));
        assert_eq!(parse_chunk_name("A1B2C3D4E5F6G7H8_3_20260908_072908.wav"), Some((3, t)));
        assert_eq!(parse_chunk_name("A1B2C3D4E5F6G7H8X_3_20260908_072908.wav"), None, "prefix longer than 16");
        assert_eq!(parse_chunk_name("_01_20260908_072908.wav"), None, "no prefix");
        assert_eq!(parse_chunk_name("notes_2026.wav"), None);
        assert_eq!(parse_chunk_name("REC-01-20260908-072908.wav"), None);
        assert_eq!(parse_chunk_name("20260908_072908.wav"), None);
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_secs(t), (2026, 9, 8, 7, 29, 8));
        assert_eq!(civil_from_secs(days_from_civil(2024, 2, 29) * 86400), (2024, 2, 29, 0, 0, 0));
    }

    #[test]
    fn parses_date_and_time_in_any_name() {
        let t = days_from_civil(2026, 9, 6) * 86400 + 11 * 3600 + 23 * 60 + 26;
        for name in [
            "ZOOM0001 20260906-112326.WAV",
            "260906_112326_A.wav",
            "260906-112326_Tr1.WAV",
            "20260906_112326.wav",
            "take20260906T112326.wav",
            "REC-01-20260906-112326.wav",
            "2026-09-06 11-23-26.wav",
            "2026-09-06_11.23.26 Interview.wav",
            "2026-09-06T112326.wav",
            "Kopie von 260906_112326 (1).wav",
        ] {
            assert_eq!(parse_name_time(name), Some(t), "{name}");
        }
        for name in [
            "interview.wav",
            "notes_2026.wav",
            "20260906112326.wav",
            "1260906_112326.wav",
            "260906_1123261.wav",
            "260230_112326.wav",
            "20250229_120000.wav",
            "take_123456_654321.wav",
            "20260906_246000.wav",
            "181010_1010.wav",
            "18990906_112326.wav",
            "2026-09-06 11-23.26.wav",
            "260906 112326.wav",
            "TASCAM_0001.wav",
            "ZOOM0001_LR.WAV",
        ] {
            assert_eq!(parse_name_time(name), None, "{name}");
        }
        assert_eq!(parse_name_time("20240229_000000.wav"), Some(days_from_civil(2024, 2, 29) * 86400));
        // Unset device clocks: kept.
        assert_eq!(parse_name_time("19700101_000512.wav"), Some(312));
        assert_eq!(parse_name_time("19990906_112326.wav"), Some(days_from_civil(1999, 9, 6) * 86400 + 41_006));
        assert_eq!(parse_chunk_name("REC_01_19700101_000512.WAV"), Some((1, 312)));
    }

    #[test]
    fn split_limits() {
        assert!(at_split_limit((1 << 31) - 100_000, 0.0));
        assert!(at_split_limit(1 << 31, 0.0));
        assert!(!at_split_limit((1 << 31) + 1, 0.0));
        assert!(at_split_limit(2_000_000_000 - MIB, 0.0));
        assert!(at_split_limit(4_294_967_295 - 5 * MIB, 0.0));
        assert!(at_split_limit(4_000_000_000 - 44, 0.0));
        assert!(!at_split_limit(1_900_000_000, 0.0));
        assert!(!at_split_limit(3_500_000_000, 0.0));
        assert!(!at_split_limit(wav::CHUNK_FILE_SIZE, 1845.9));
        assert!(at_split_limit(691 * MIB, 3600.2));
        assert!(!at_split_limit(691 * MIB, 3590.0));
    }

    /// Two transmitters, identical file names and start times, chunks shuffled
    /// into the "wrong" folders, a copy, AppleDouble junk and old output.
    #[test]
    fn messy_two_transmitters() {
        let root = tempdir("messy");
        let sr = 8000;
        let a = |start: u64, n: u64| sine(220.0, 0.5, sr, start, n);
        let b = |start: u64, n: u64| sine(331.0, 0.2, sr, start + 13, n);
        write_float_wav(&root.join("a/DJI_01_20260101_100000.WAV"), sr, &a(0, 80_000));
        write_float_wav(&root.join("b/DJI_01_20260101_100000.WAV"), sr, &b(0, 80_000));
        write_float_wav(&root.join("b/x/DJI_01_20260101_100010.WAV"), sr, &a(80_000, 80_000));
        write_float_wav(&root.join("a/DJI_01_20260101_100010.WAV"), sr, &b(80_000, 80_000));
        write_float_wav(&root.join("DJI_01_20260101_100020.WAV"), sr, &a(160_000, 32_000));
        write_float_wav(&root.join("deep/y/z/DJI_01_20260101_100020.WAV"), sr, &b(160_000, 24_000));
        fs::create_dir_all(root.join("copy")).unwrap();
        fs::copy(root.join("a/DJI_01_20260101_100000.WAV"), root.join("copy/DJI_01_20260101_100000 (1).WAV")).unwrap();
        fs::write(root.join("a/._DJI_01_20260101_100000.WAV"), b"junk").unwrap();
        write_float_wav(&root.join("notes.wav"), sr, &a(0, 800));
        write_float_wav(&root.join("a/DJI_02_20260101_110000.WAV"), sr, &a(0, 16_000));
        write_float_wav(&root.join("tracks/DJI_09_20260101_120000.WAV"), sr, &a(0, 16_000));

        let s = scan(&[root.clone()], &small()).unwrap();
        let rel = |r: &Recording| -> Vec<String> {
            r.parts
                .iter()
                .map(|p| p.path_buf.strip_prefix(&root).unwrap().display().to_string())
                .collect()
        };
        // notes.wav has no time in its name: a single recording dated by the file.
        assert_eq!(s.recordings.len(), 4, "{:#?}", s.recordings.iter().map(rel).collect::<Vec<_>>());
        assert_eq!(s.duplicates.len(), 1);
        assert!(s.duplicates[0].path.ends_with("(1).WAV"));
        assert!(s.ignored.is_empty(), "{:?}", s.ignored);

        let rec_a = &s.recordings[0];
        assert_eq!(
            rel(rec_a),
            ["a/DJI_01_20260101_100000.WAV", "b/x/DJI_01_20260101_100010.WAV", "DJI_01_20260101_100020.WAV"]
        );
        assert_eq!(rec_a.frames, 192_000);
        assert_eq!(rec_a.out_name, "260101_S100000-E100024_D000024_a.wav");
        assert!(rec_a.warnings.is_empty(), "{:?}", rec_a.warnings);
        assert_eq!(rec_a.confidence, Some(Confidence::Medium));
        assert_eq!(rec_a.parts.iter().map(|p| p.link_confidence).collect::<Vec<_>>(), [None, Some(Confidence::Medium), Some(Confidence::Medium)]);
        assert_eq!(s.recordings[2].confidence, None);
        let rec_b = &s.recordings[1];
        assert_eq!(
            rel(rec_b),
            ["b/DJI_01_20260101_100000.WAV", "a/DJI_01_20260101_100010.WAV", "deep/y/z/DJI_01_20260101_100020.WAV"]
        );
        assert_eq!(rec_b.frames, 184_000);
        assert_eq!(s.recordings[2].parts.len(), 1);
        assert_eq!(s.recordings[2].seq, Some(2));
        assert_eq!(names(&s.recordings[3]), ["notes.wav"]);
        assert_eq!(s.recordings[3].parts[0].time_source, TimeSource::File);
        assert_eq!(s.recordings[3].seq, None);
        assert!(rec_a.parts.iter().all(|p| p.time_source == TimeSource::Name));
    }

    #[test]
    fn time_gap_splits_recordings() {
        let root = tempdir("gap");
        let sr = 8000;
        write_float_wav(&root.join("DJI_01_20260101_100000.WAV"), sr, &sine(220.0, 0.5, sr, 0, 80_000));
        // Starts 5 s after the first file ended: not a continuation.
        write_float_wav(&root.join("DJI_01_20260101_100015.WAV"), sr, &sine(220.0, 0.5, sr, 80_000, 80_000));
        let s = scan(&[root], &small()).unwrap();
        assert_eq!(s.recordings.len(), 2);
    }

    #[test]
    fn continuity_prefers_true_neighbour() {
        let root = tempdir("cont");
        let sr = 8000;
        write_float_wav(&root.join("a1/DJI_01_20260101_100000.WAV"), sr, &sine(220.0, 0.5, sr, 0, 8000));
        write_float_wav(&root.join("a2/DJI_01_20260101_100001.WAV"), sr, &sine(220.0, 0.5, sr, 8000, 8000));
        write_float_wav(&root.join("b2/DJI_01_20260101_100001.WAV"), sr, &sine(331.0, 0.2, sr, 8013, 8000));
        let a1 = load_part(&root.join("a1/DJI_01_20260101_100000.WAV")).unwrap();
        let a2 = load_part(&root.join("a2/DJI_01_20260101_100001.WAV")).unwrap();
        let b2 = load_part(&root.join("b2/DJI_01_20260101_100001.WAV")).unwrap();
        let good = continuity_cost(&a1, &a2).unwrap();
        let bad = continuity_cost(&a1, &b2).unwrap();
        assert!(good + 0.5 < bad, "good {good} bad {bad}");
    }

    /// Any names, bext TimeReference contiguous: joined, even though the first
    /// chunk is not full by size and even when the continuation repeats the
    /// first chunk's OriginationTime.
    #[test]
    fn bext_time_reference_joins_any_names() {
        let sr = 8000;
        let tr0 = 11 * 3600 * sr as u64;
        for second_clock in ["11:00:05", "11:00:00"] {
            let root = tempdir("bext");
            write_float_bwf(&root.join("take-a.wav"), sr, &sine(220.0, 0.5, sr, 0, 40_000), "2026-09-06", "11:00:00", tr0);
            write_float_bwf(&root.join("take-b.wav"), sr, &sine(220.0, 0.5, sr, 40_000, 80_000), "2026-09-06", second_clock, tr0 + 40_000);
            let s = scan(&[root.clone()], &small()).unwrap();
            assert_eq!(s.recordings.len(), 1, "{second_clock}: {:#?}", s.recordings);
            let r = &s.recordings[0];
            assert_eq!(names(r), ["take-a.wav", "take-b.wav"]);
            assert!(r.parts.iter().all(|p| p.time_source == TimeSource::Bext && p.seq.is_none()));
            assert_eq!(r.parts[1].gap_to_prev, Some(0.0));
            assert_eq!((r.date.as_str(), r.start.as_str(), r.end.as_str()), ("06.09.2026", "11:00:00", "11:00:15"));
            assert_eq!(r.confidence, Some(Confidence::High));
            let json = serde_json::to_value(r).unwrap();
            assert_eq!(json["confidence"], "high");
            assert_eq!(json["parts"][0]["time_source"], "bext");
            assert_eq!(json["parts"][0]["seq"], serde_json::Value::Null);
            assert_eq!(json["parts"][0]["link_confidence"], serde_json::Value::Null);
            assert_eq!(json["parts"][1]["link_confidence"], "high");
        }
    }

    /// TimeReference one second off: not joined, although the clocks and sizes would allow it.
    #[test]
    fn bext_time_reference_mismatch_is_not_joined() {
        let root = tempdir("bext-off");
        let sr = 8000;
        let tr0 = 11 * 3600 * sr as u64;
        write_float_bwf(&root.join("take-a.wav"), sr, &sine(220.0, 0.5, sr, 0, 40_000), "2026-09-06", "11:00:00", tr0);
        write_float_bwf(&root.join("take-b.wav"), sr, &sine(220.0, 0.5, sr, 40_000, 40_000), "2026-09-06", "11:00:06", tr0 + 48_000);
        let s = scan(&[root], &small()).unwrap();
        assert_eq!(s.recordings.len(), 2, "{:#?}", s.recordings);
    }

    /// A join across midnight by TimeReference (the continuation's date is the next day).
    #[test]
    fn bext_time_reference_across_midnight() {
        let sr = 8000;
        let tr0 = 86_400 * sr as u64 - 40_000 + 5;
        for (tr_b, joined) in [(5, true), (6, true), (5 + 8000, false)] {
            let root = tempdir("bext-midnight");
            write_float_bwf(&root.join("a.wav"), sr, &sine(220.0, 0.5, sr, 0, 40_000), "2026-09-06", "23:59:55", tr0);
            write_float_bwf(&root.join("b.wav"), sr, &sine(220.0, 0.5, sr, 40_000, 40_000), "2026-09-07", "00:00:00", tr_b);
            let s = scan(&[root], &small()).unwrap();
            assert_eq!(s.recordings.len() == 1, joined, "TimeReference {tr_b}: {:#?}", s.recordings);
            if joined {
                assert_eq!(s.recordings[0].confidence, Some(Confidence::High));
                assert_eq!(s.recordings[0].date, "06.09.2026");
            }
        }
    }

    /// iXML: same FAMILY_UID and the next FILE_SET_INDEX joins even without a
    /// time match; files of a set that start together (channels) are not chained.
    #[test]
    fn ixml_file_set_joins() {
        let root = tempdir("ixml");
        let sr = 8000;
        let (a, b) = (root.join("S01T03.wav"), root.join("S01T03_2.wav"));
        write_wav_with(&a, sr, &sine(220.0, 0.5, sr, 0, 40_000), &ixml_chunk("FAM-1", "1", 2), &[]);
        write_wav_with(&b, sr, &sine(220.0, 0.5, sr, 40_000, 80_000), &ixml_chunk("FAM-1", "2", 2), &[]);
        set_mtime(&a, T0 + 5.0);
        set_mtime(&b, T0 + 45.0);
        let (c, d) = (root.join("ch/S01T04_A.wav"), root.join("ch/S01T04_B.wav"));
        write_wav_with(&c, sr, &sine(220.0, 0.5, sr, 0, 80_000), &ixml_chunk("FAM-2", "A", 2), &[]);
        write_wav_with(&d, sr, &sine(331.0, 0.2, sr, 0, 80_000), &ixml_chunk("FAM-2", "B", 2), &[]);
        set_mtime(&c, T0 + 1000.0);
        set_mtime(&d, T0 + 1000.0);
        let s = scan(&[root], &small()).unwrap();
        let shape: Vec<Vec<&str>> = s.recordings.iter().map(names).collect();
        assert_eq!(shape, [vec!["S01T03.wav", "S01T03_2.wav"], vec!["S01T04_A.wav"], vec!["S01T04_B.wav"]]);
        let r = &s.recordings[0];
        assert_eq!(r.confidence, Some(Confidence::High));
        assert_eq!(r.parts[1].gap_to_prev, Some(30.0));
    }

    /// Device clock never set (2000 / 1970): the chunks still join, with a warning.
    #[test]
    fn unset_clock_is_kept_and_warned() {
        let root = tempdir("clock");
        let sr = 8000;
        write_float_wav(&root.join("REC_01_20000101_000000.WAV"), sr, &sine(220.0, 0.5, sr, 0, 80_000));
        write_float_wav(&root.join("REC_01_20000101_000010.WAV"), sr, &sine(220.0, 0.5, sr, 80_000, 80_000));
        write_float_wav(&root.join("REC_01_20000101_000020.WAV"), sr, &sine(220.0, 0.5, sr, 160_000, 20_000));
        write_float_wav(&root.join("19700101_000512.wav"), sr, &sine(331.0, 0.2, sr, 0, 8_000));
        let s = scan(&[root], &small()).unwrap();
        assert_eq!(s.recordings.iter().map(|r| r.parts.len()).collect::<Vec<_>>(), [1, 3]);
        let warned = |r: &Recording, date: &str| {
            let w = |l: Lang| i18n::format(l, Msg::WarnClockNotSet, &[("date", &date)]);
            r.warnings.iter().any(|x| Lang::ALL.iter().any(|&l| *x == w(l)))
        };
        assert!(warned(&s.recordings[0], "01.01.1970"), "{:?}", s.recordings[0].warnings);
        assert!(warned(&s.recordings[1], "01.01.2000"), "{:?}", s.recordings[1].warnings);
        assert_eq!(s.recordings[1].confidence, Some(Confidence::Medium));
    }

    /// A predecessor that is only the largest file (no full-chunk pattern): joined, but low and warned.
    #[test]
    fn weak_chain_is_low_confidence() {
        let root = tempdir("low");
        let sr = 8000;
        write_float_wav(&root.join("REC_01_20260101_100000.WAV"), sr, &sine(220.0, 0.5, sr, 0, 80_000));
        write_float_wav(&root.join("REC_01_20260101_100010.WAV"), sr, &sine(220.0, 0.5, sr, 80_000, 20_000));
        let s = scan(&[root], &small()).unwrap();
        assert_eq!(s.recordings.len(), 1);
        let r = &s.recordings[0];
        assert_eq!(r.confidence, Some(Confidence::Low));
        let low = |l: Lang| i18n::text(l, Msg::WarnLowConfidence);
        assert!(r.warnings.iter().any(|w| Lang::ALL.iter().any(|&l| w == low(l))), "{:?}", r.warnings);
    }

    /// No time in metadata or name: file dates (mtime − duration) link the chunks.
    #[test]
    fn file_dates_join_chunks_without_names() {
        let root = tempdir("filedates");
        let sr = 8000;
        let (a, b) = (root.join("Aufnahme A.wav"), root.join("Aufnahme B.wav"));
        write_float_wav(&a, sr, &sine(220.0, 0.5, sr, 0, 80_000));
        write_float_wav(&b, sr, &sine(220.0, 0.5, sr, 80_000, 80_000));
        set_mtime(&a, T0 + 10.0);
        set_mtime(&b, T0 + 20.0);
        let s = scan(&[root.clone()], &small()).unwrap();
        assert_eq!(s.recordings.len(), 1, "{:#?}", s.recordings);
        let r = &s.recordings[0];
        assert_eq!(names(r), ["Aufnahme A.wav", "Aufnahme B.wav"]);
        assert!(r.parts.iter().all(|p| p.time_source == TimeSource::File));
        assert_eq!(serde_json::to_value(&r.parts[1]).unwrap()["time_source"], "file");
        let hint = |l: Lang| i18n::format(l, Msg::WarnFileTimes, &[("a", &1), ("b", &2)]);
        assert!(r.warnings.iter().any(|w| Lang::ALL.iter().any(|&l| *w == hint(l))), "{:?}", r.warnings);
        assert_eq!(r.confidence, Some(Confidence::Medium));
        assert_eq!(r.start_secs + local_offset(T0 as i64).rem_euclid(1), r.parts[0].start_secs);

        // The same files, the second one ending 60 s later: two recordings.
        set_mtime(&b, T0 + 80.0);
        let s = scan(&[root], &small()).unwrap();
        assert_eq!(s.recordings.len(), 2);
    }

    /// Dates and times in names of other recorders, without sequence numbers.
    #[test]
    fn generic_name_times_join() {
        let root = tempdir("generic");
        let sr = 8000;
        let z = |start: u64, n: u64| sine(220.0, 0.5, sr, start, n);
        let t = |start: u64, n: u64| sine(331.0, 0.2, sr, start + 13, n);
        write_float_wav(&root.join("ZOOM0001 20260906-112326.WAV"), sr, &z(0, 80_000));
        write_float_wav(&root.join("ZOOM0002 20260906-112336.WAV"), sr, &z(80_000, 30_000));
        write_float_wav(&root.join("t/260906_120000_A.wav"), sr, &t(0, 80_000));
        write_float_wav(&root.join("t/260906_120010_A.wav"), sr, &t(80_000, 20_000));
        let s = scan(&[root.clone()], &small()).unwrap();
        let shape: Vec<Vec<&str>> = s.recordings.iter().map(names).collect();
        assert_eq!(
            shape,
            [vec!["ZOOM0001 20260906-112326.WAV", "ZOOM0002 20260906-112336.WAV"], vec!["260906_120000_A.wav", "260906_120010_A.wav"]]
        );
        assert!(s.recordings.iter().flat_map(|r| &r.parts).all(|p| p.time_source == TimeSource::Name && p.seq.is_none()));
        assert_eq!(s.recordings[0].out_name, "260906_S112326-E112340_D000014_REC.wav".replace("REC", &sanitize(&root.file_name().unwrap().to_string_lossy())));
    }

    /// A 60 s gap or a different sample rate is never joined, whatever the time source.
    #[test]
    fn gap_or_other_format_never_joins() {
        let root = tempdir("never");
        let sr = 8000;
        let (a, b) = (root.join("gap/260906_130000_A.wav"), root.join("gap/260906_130110_A.wav"));
        write_float_wav(&a, sr, &sine(220.0, 0.5, sr, 0, 80_000));
        write_float_wav(&b, sr, &sine(220.0, 0.5, sr, 80_000, 80_000));
        set_mtime(&a, T0 + 10.0);
        set_mtime(&b, T0 + 80.0);
        let (c, d) = (root.join("rate/take 1.wav"), root.join("rate/take 2.wav"));
        write_float_wav(&c, sr, &sine(220.0, 0.5, sr, 0, 80_000));
        write_float_wav(&d, 2 * sr, &sine(220.0, 0.5, 2 * sr, 80_000, 40_000));
        set_mtime(&c, T0 + 1000.0);
        set_mtime(&d, T0 + 1005.0);
        let tr0 = 13 * 3600 * sr as u64;
        write_float_bwf(&root.join("bext/x.wav"), sr, &sine(220.0, 0.5, sr, 0, 80_000), "2026-09-06", "13:00:00", tr0);
        write_float_bwf(&root.join("bext/y.wav"), 2 * sr, &sine(220.0, 0.5, 2 * sr, 0, 40_000), "2026-09-06", "13:00:10", 2 * (tr0 + 80_000));
        let s = scan(&[root], &small()).unwrap();
        assert_eq!(s.recordings.len(), 6, "{:#?}", s.recordings.iter().map(names).collect::<Vec<_>>());
    }

    /// File dates alone and a broken seam (another signal): not joined.
    #[test]
    fn file_dates_need_a_clean_seam() {
        let root = tempdir("seam");
        let sr = 8000;
        let (a, b) = (root.join("one.wav"), root.join("two.wav"));
        write_float_wav(&a, sr, &sine(220.0, 0.5, sr, 0, 80_000));
        let noise: Vec<f32> = (0..80_000u64).map(|i| (((i * 2_654_435_761) % 1000) as f32 / 1000.0 - 0.5) * 0.8).collect();
        write_float_wav(&b, sr, &noise);
        set_mtime(&a, T0 + 10.0);
        set_mtime(&b, T0 + 20.0);
        let pa = load_part(&a).unwrap();
        let pb = load_part(&b).unwrap();
        assert!(continuity_cost(&pa, &pb).unwrap() > FILE_LINK_MAX_COST);
        assert_eq!(scan(&[root], &small()).unwrap().recordings.len(), 2);
    }
}
