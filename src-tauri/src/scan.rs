//! Finds DJI Mic 2 files anywhere below the given folders (tidy or messy) and
//! groups the chunks the recorder split each recording into.
//!
//! A chunk B continues chunk A when both have the same sequence number and
//! audio format and B's filename timestamp matches A's start + duration.
//! Only a full chunk (338 MiB on the DJI Mic 2) can have a continuation. Two
//! transmitters produce identical file names, so when several candidates fit,
//! the audio across the join decides: a linear predictor trained on one side
//! keeps predicting the other side of a true join, but not a foreign chunk.
//! The folder is only a weak tie-breaker.

use crate::wav::{self, WavInfo};
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

#[derive(Debug, Clone)]
pub struct Options {
    /// Seconds a follow-up chunk may deviate from the predecessor's end.
    pub gap_tolerance: f64,
    pub max_depth: usize,
    /// Smallest data size a chunk cut by the recorder can have.
    pub min_chunk_bytes: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self { gap_tolerance: 3.0, max_depth: 24, min_chunk_bytes: 100 * MIB }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Part {
    pub path: String,
    pub name: String,
    pub folder: String,
    pub seq: u32,
    pub start: String,
    pub duration: f64,
    pub size: u64,
    pub gap_to_prev: Option<f64>,
    pub full_chunk: bool,
    pub repaired: bool,
    #[serde(skip)]
    pub path_buf: PathBuf,
    #[serde(skip)]
    pub folder_path: PathBuf,
    #[serde(skip)]
    pub start_secs: i64,
    #[serde(skip)]
    pub info: WavInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recording {
    pub id: usize,
    pub label: String,
    pub seq: u32,
    pub date: String,
    pub start: String,
    pub end: String,
    pub duration: f64,
    pub format: String,
    pub data_bytes: u64,
    pub output_bytes: u64,
    pub out_name: String,
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
            ignored.push(Skipped { path: input.display().to_string(), reason: "keine WAV-Datei".into() });
        }
    }
    if roots.is_empty() {
        return Err("Keine Ordner oder WAV-Dateien angegeben.".into());
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
    let Some((seq, start_secs)) = parse_dji_name(&name) else {
        return Err("kein DJI-Dateiname (DJI_NN_JJJJMMTT_HHMMSS)".into());
    };
    let info = wav::read_info(path).map_err(|e| format!("nicht lesbar: {e}"))?;
    if info.data_len == 0 {
        return Err("enthält keine Audiodaten".into());
    }
    let folder_path = path.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok(Part {
        path: path.display().to_string(),
        name,
        folder: folder_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        seq,
        start: fmt_datetime(start_secs),
        duration: info.duration(),
        size: info.file_size,
        gap_to_prev: None,
        full_chunk: false,
        repaired: info.repaired,
        path_buf: path.to_path_buf(),
        folder_path,
        start_secs,
        info,
    })
}

/// Parses `DJI_<seq>_<YYYYMMDD>_<HHMMSS>` anywhere in a file name (case
/// insensitive, so copies like `dji_01_20260908_072908 (1).wav` work too).
/// Returns the sequence number and the start as seconds since 1970 (local clock).
pub fn parse_dji_name(name: &str) -> Option<(u32, i64)> {
    let up = name.to_ascii_uppercase();
    let mut from = 0;
    while let Some(pos) = up[from..].find("DJI_") {
        let i = from + pos + 4;
        if let Some(r) = parse_after_prefix(&up.as_bytes()[i..]) {
            return Some(r);
        }
        from = i;
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
    if y < 2000 || !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    Some((seq, days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s))
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

/// Removes byte-identical copies (same name timestamp, format, length and
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
            dups.push(Skipped { path: p.path.clone(), reason: format!("identische Kopie von {}", kept[i].path) });
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
    p.start_secs.hash(&mut h);
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

struct Link {
    a: usize,
    b: usize,
    score: f64,
}

fn group(mut parts: Vec<Part>, opt: &Options) -> Vec<Recording> {
    parts.sort_by(|a, b| a.start_secs.cmp(&b.start_secs).then_with(|| a.path.cmp(&b.path)));

    // Chunks cut by the recorder all have the same byte size.
    let mut size_count: HashMap<u64, usize> = HashMap::new();
    let mut largest: HashMap<Vec<u8>, u64> = HashMap::new();
    let mut known_size: HashMap<Vec<u8>, bool> = HashMap::new();
    for p in &parts {
        *size_count.entry(p.size).or_default() += 1;
        let l = largest.entry(p.info.fmt.clone()).or_default();
        *l = (*l).max(p.size);
        *known_size.entry(p.info.fmt.clone()).or_default() |= p.size == wav::DJI_CHUNK_FILE_SIZE;
    }
    for p in parts.iter_mut() {
        p.full_chunk = p.size == wav::DJI_CHUNK_FILE_SIZE
            || (size_count[&p.size] >= 2 && p.info.data_len >= opt.min_chunk_bytes);
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

    let n = parts.len();
    let tol = opt.gap_tolerance;
    let mut links: Vec<Link> = Vec::new();
    for a in 0..n {
        if !can_continue[a] {
            continue;
        }
        let end = parts[a].start_secs as f64 + parts[a].duration;
        for b in a + 1..n {
            let start_b = parts[b].start_secs as f64;
            if start_b > end + tol {
                break;
            }
            if start_b < end - tol || parts[a].seq != parts[b].seq || parts[a].info.fmt != parts[b].info.fmt {
                continue;
            }
            let mut score = (start_b - end).abs() * GAP_WEIGHT
                + continuity_cost(&parts[a], &parts[b]).unwrap_or(CONTINUITY_UNKNOWN);
            if parts[a].folder_path != parts[b].folder_path {
                score += PENALTY_OTHER_FOLDER;
            }
            links.push(Link { a, b, score });
        }
    }

    links.sort_by(|x, y| x.score.total_cmp(&y.score));
    let mut next: Vec<Option<usize>> = vec![None; n];
    let mut prev: Vec<Option<usize>> = vec![None; n];
    let mut chosen = Vec::new();
    for (i, l) in links.iter().enumerate() {
        if next[l.a].is_none() && prev[l.b].is_none() {
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
    }

    let mut slots: Vec<Option<Part>> = parts.into_iter().map(Some).collect();
    let mut recordings = Vec::new();
    for head in 0..n {
        if prev[head].is_some() {
            continue;
        }
        let mut chain = Vec::new();
        let mut amb = false;
        let mut cur = Some(head);
        while let Some(i) = cur {
            amb |= ambiguous[i];
            chain.push(slots[i].take().expect("chunk assigned twice"));
            cur = next[i];
        }
        recordings.push(build_recording(chain, amb));
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

fn build_recording(mut parts: Vec<Part>, ambiguous: bool) -> Recording {
    for k in 1..parts.len() {
        let gap = parts[k].start_secs as f64 - (parts[k - 1].start_secs as f64 + parts[k - 1].duration);
        parts[k].gap_to_prev = Some((gap * 100.0).round() / 100.0);
    }
    let first = &parts[0];
    let seq = first.seq;
    let start_secs = first.start_secs;
    let sample_rate = first.info.sample_rate;
    let fmt_len = first.info.fmt.len();
    let format = first.info.format_label();
    let mut label = sanitize(&first.folder);
    if label.is_empty() {
        label = format!("DJI{seq:02}");
    }

    let frames: u64 = parts.iter().map(|p| p.info.frames()).sum();
    let data_bytes: u64 = parts.iter().map(|p| p.info.data_len).sum();
    let duration = frames as f64 / sample_rate as f64;
    let dur = duration.round() as i64;
    let end_secs = start_secs + dur;

    let mut warnings = Vec::new();
    if ambiguous {
        warnings.push(
            "Zuordnung der Teile nicht eindeutig (zwei Sender mit fast gleicher Startzeit?). Bitte die Teile prüfen."
                .to_string(),
        );
    }
    let count = parts.len();
    for (k, p) in parts.iter().enumerate() {
        if p.repaired {
            warnings.push(format!(
                "Teil {}: Dateikopf unvollständig (Aufnahme vermutlich abgebrochen), Länge aus der Dateigröße bestimmt.",
                k + 1
            ));
        }
        if let Some(g) = p.gap_to_prev {
            if g.abs() > 1.5 {
                warnings.push(format!("Zwischen Teil {k} und {} weichen die Zeitstempel um {g:+.1} s ab.", k + 1));
            }
        }
        if k + 1 < count && !p.full_chunk {
            warnings.push(format!("Teil {} ist kürzer als ein voller Chunk, hat aber einen Folgeteil.", k + 1));
        }
    }
    if parts.last().map_or(false, |p| p.full_chunk) {
        warnings.push("Der letzte Teil hat volle Chunk-Größe. Möglicherweise fehlt ein Folgeteil.".to_string());
    }

    let (y, mo, d, h, mi, s) = civil_from_secs(start_secs);
    let (_, _, _, eh, emi, es) = civil_from_secs(end_secs);
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
        parts,
        warnings,
        start_secs,
        frames,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testutil::*;

    /// Synthetic test chunks are tiny, so any repeated file size counts as a chunk.
    pub(crate) fn small() -> Options {
        Options { min_chunk_bytes: 0, ..Options::default() }
    }

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
        assert_eq!(parse_dji_name("DJI_01_20260908_072908.WAV"), Some((1, t)));
        assert_eq!(parse_dji_name("dji_01_20260908_072908 (1).wav"), Some((1, t)));
        assert_eq!(parse_dji_name("Kopie von DJI_1234_20260908_072908.WAV"), Some((1234, t)));
        assert_eq!(parse_dji_name("DJI_01_20261308_072908.WAV"), None);
        assert_eq!(parse_dji_name("DJI_01_20260908_0729081.WAV"), None);
        assert_eq!(parse_dji_name("interview.wav"), None);
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_secs(t), (2026, 9, 8, 7, 29, 8));
        assert_eq!(civil_from_secs(days_from_civil(2024, 2, 29) * 86400), (2024, 2, 29, 0, 0, 0));
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
        assert_eq!(s.recordings.len(), 3, "{:#?}", s.recordings.iter().map(rel).collect::<Vec<_>>());
        assert_eq!(s.duplicates.len(), 1);
        assert!(s.duplicates[0].path.ends_with("(1).WAV"));
        assert_eq!(s.ignored.len(), 1);
        assert!(s.ignored[0].path.ends_with("notes.wav"));

        let rec_a = &s.recordings[0];
        assert_eq!(
            rel(rec_a),
            ["a/DJI_01_20260101_100000.WAV", "b/x/DJI_01_20260101_100010.WAV", "DJI_01_20260101_100020.WAV"]
        );
        assert_eq!(rec_a.frames, 192_000);
        assert_eq!(rec_a.out_name, "260101_S100000-E100024_D000024_a.wav");
        let rec_b = &s.recordings[1];
        assert_eq!(
            rel(rec_b),
            ["b/DJI_01_20260101_100000.WAV", "a/DJI_01_20260101_100010.WAV", "deep/y/z/DJI_01_20260101_100020.WAV"]
        );
        assert_eq!(rec_b.frames, 184_000);
        assert_eq!(s.recordings[2].parts.len(), 1);
        assert_eq!(s.recordings[2].seq, 2);
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
}
