//! Reading audio of any supported format, shared by „Synchronisieren“ and „Mastern“.
//!
//! WAV (also RF64) goes through the app's own reader; MP3, AAC/M4A, FLAC, ALAC,
//! AIFF, CAF and OGG Vorbis through Symphonia, compiled into the app.
//!
//! The sync step works with random access on WAV data. Every other format is
//! therefore decoded once into a 32-bit float WAV (native rate and channels) in
//! `~/Library/Caches/city.bias.prepareaudio/decoded/<hash>.wav`. The hash covers
//! the absolute path, the size and the modification time of the source, so the
//! file is reused while the source stays unchanged and a changed source gets a
//! new one. Files are written as `.part` and renamed when complete; files not
//! used for 30 days are removed when an analysis starts.

use crate::i18n::{self, t, tf, Msg};
use crate::merge::{available_bytes, human_bytes};
use crate::wav::{self, WavInfo};
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const AUDIO_EXT: [&str; 11] = ["wav", "wave", "mp3", "m4a", "mp4", "aac", "flac", "aif", "aiff", "ogg", "caf"];
/// Bump when the cached file layout changes: old files are then simply not found again.
const CACHE_VERSION: u32 = 1;
const CACHE_MAX_AGE: Duration = Duration::from_secs(30 * 86_400);
/// Unfinished `.part` files older than this are leftovers of a crash.
const PART_MAX_AGE: Duration = Duration::from_secs(86_400);
/// Size estimate when the stream does not state its length: as if coded at 64 kbit/s.
const FALLBACK_BITRATE: f64 = 64_000.0;

pub fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Container name for an extension ("MP3", "M4A", …).
pub fn container(ext: &str) -> &'static str {
    match ext {
        "wav" | "wave" => "WAV",
        "mp3" => "MP3",
        "m4a" | "mp4" => "M4A",
        "aac" => "AAC",
        "flac" => "FLAC",
        "aif" | "aiff" => "AIFF",
        "ogg" => "OGG",
        "caf" => "CAF",
        _ => "Audio",
    }
}

/// Formats that are not WAV but can be decoded.
pub fn is_compressed(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    let ext = extension(path);
    !name.starts_with('.') && ext != "wav" && ext != "wave" && AUDIO_EXT.contains(&ext.as_str())
}

// ------------------------------------------------------------------ readers

pub(crate) trait AudioReader {
    fn channels(&self) -> usize;
    fn rate(&self) -> u32;
    /// Appends the next block of interleaved samples; false at the end.
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String>;
}

pub(crate) struct Details {
    pub codec: String,
    pub bits: Option<u32>,
}

struct WavReader {
    info: WavInfo,
    file: File,
    remaining: u64,
    buf: Vec<u8>,
}

impl WavReader {
    fn open(path: &Path) -> Result<(Self, Details), String> {
        let info = wav::read_info(path).map_err(|e| e.to_string())?;
        let kind = info.sample_kind().ok_or(t(Msg::WavFormatUnsupported))?;
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(info.data_offset)).map_err(|e| e.to_string())?;
        let bits = (info.block_align / info.channels.max(1)) as u32 * 8;
        let codec = match kind {
            wav::SampleKind::F32 | wav::SampleKind::F64 => format!("pcm_f{bits}le"),
            wav::SampleKind::U8 => "pcm_u8".to_string(),
            _ => format!("pcm_s{bits}le"),
        };
        let details = Details { codec, bits: Some(bits) };
        let buf = vec![0u8; info.block_align as usize * 16384];
        Ok((WavReader { remaining: info.data_len, file, buf, info }, details))
    }
}

impl AudioReader for WavReader {
    fn channels(&self) -> usize {
        self.info.channels as usize
    }
    fn rate(&self) -> u32 {
        self.info.sample_rate
    }
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        if self.remaining == 0 {
            return Ok(false);
        }
        let kind = self.info.sample_kind().ok_or(t(Msg::WavFormatUnsupported))?;
        let bps = (self.info.block_align / self.info.channels.max(1)) as usize;
        let n = (self.buf.len() as u64).min(self.remaining) as usize;
        self.file.read_exact(&mut self.buf[..n]).map_err(|e| tf(Msg::ReadError, &[("e", &e)]))?;
        self.remaining -= n as u64;
        out.reserve(n / bps);
        out.extend(self.buf[..n].chunks_exact(bps).map(|s| wav::decode_sample(s, kind) as f32));
        Ok(true)
    }
}

pub(crate) struct SymReader {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    sample_buf: Option<SampleBuffer<f32>>,
    pub channels: usize,
    pub rate: u32,
    /// Frames the container announces (after gapless trimming), if it does.
    pub n_frames: Option<u64>,
    pending: Vec<f32>,
    done: bool,
}

impl SymReader {
    /// `gapless`: trim encoder delay and padding where the format states them (MP3 LAME tag, OGG).
    pub fn open(path: &Path, ext: &str, gapless: bool) -> Result<(Self, Details), String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        Self::open_source(Box::new(file), ext, gapless)
    }

    fn open_source(source: Box<dyn MediaSource>, ext: &str, gapless: bool) -> Result<(Self, Details), String> {
        let mss = MediaSourceStream::new(source, Default::default());
        let mut hint = Hint::new();
        if !ext.is_empty() {
            hint.with_extension(ext);
        }
        let options = FormatOptions { enable_gapless: gapless, ..Default::default() };
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &options, &MetadataOptions::default())
            .map_err(|_| t(Msg::FormatUnsupported).to_string())?;
        let format = probed.format;
        let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or(t(Msg::NoAudioTrack))?;
        let (track_id, params) = (track.id, track.codec_params.clone());
        let registry = symphonia::default::get_codecs();
        let decoder = registry.make(&params, &DecoderOptions::default()).map_err(|_| t(Msg::CodecUnsupported).to_string())?;
        let codec = registry.get_codec(params.codec).map(|d| d.short_name.to_string()).unwrap_or_default();
        let mut reader = SymReader {
            format,
            decoder,
            track: track_id,
            sample_buf: None,
            channels: params.channels.map_or(0, |c| c.count()),
            rate: params.sample_rate.unwrap_or(0),
            n_frames: params.n_frames,
            pending: Vec::new(),
            done: false,
        };
        // The first packet tells channel count and rate reliably for every codec.
        let mut first = Vec::new();
        if !reader.decode_next(&mut first)? {
            reader.done = true;
        }
        reader.pending = first;
        Ok((reader, Details { codec, bits: params.bits_per_sample }))
    }

    fn decode_next(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(SymError::ResetRequired) => return Ok(false),
                Err(e) => return Err(tf(Msg::ReadError, &[("e", &e)])),
            };
            if packet.track_id() != self.track {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    if decoded.frames() == 0 {
                        continue;
                    }
                    let spec = *decoded.spec();
                    self.channels = spec.channels.count();
                    self.rate = spec.rate;
                    let capacity = decoded.capacity();
                    if self.sample_buf.as_ref().map_or(true, |b| b.capacity() < capacity) {
                        self.sample_buf = Some(SampleBuffer::<f32>::new(capacity as u64, spec));
                    }
                    let sb = self.sample_buf.as_mut().expect("sample buffer");
                    sb.copy_interleaved_ref(decoded);
                    out.extend_from_slice(sb.samples());
                    return Ok(true);
                }
                Err(SymError::DecodeError(_)) => continue,
                Err(e) => return Err(tf(Msg::DecodeError, &[("e", &e)])),
            }
        }
    }
}

impl AudioReader for SymReader {
    fn channels(&self) -> usize {
        self.channels
    }
    fn rate(&self) -> u32 {
        self.rate
    }
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        if !self.pending.is_empty() {
            out.append(&mut self.pending);
            return Ok(true);
        }
        if self.done {
            return Ok(false);
        }
        let more = self.decode_next(out)?;
        self.done = !more;
        Ok(more)
    }
}

/// Opens any supported file (WAV through the own reader, the rest through Symphonia, without gapless trimming).
pub(crate) fn open_reader(path: &Path, ext: &str) -> Result<(Box<dyn AudioReader>, Details), String> {
    if ext == "wav" || ext == "wave" {
        if let Ok((r, d)) = WavReader::open(path) {
            return Ok((Box::new(r), d));
        }
    }
    let (r, d) = SymReader::open(path, ext, false)?;
    if r.channels == 0 || r.rate == 0 {
        return Err(t(Msg::NoAudioData).into());
    }
    Ok((Box::new(r), d))
}

// ------------------------------------------------------------------ cache of decoded files

/// `~/Library/Caches/city.bias.prepareaudio/decoded` (elsewhere `~/.cache/…`, or the temp folder).
pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("HOME").map(PathBuf::from).map(|home| {
        if cfg!(target_os = "macos") {
            home.join("Library/Caches")
        } else {
            home.join(".cache")
        }
    });
    base.unwrap_or_else(std::env::temp_dir).join("city.bias.prepareaudio").join("decoded")
}

/// A source file with its identity at the time of the analysis.
#[derive(Debug, Clone)]
pub struct Source {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Cache file name derived from path, size and modification time.
    pub key: String,
}

impl Source {
    pub fn new(path: &Path) -> io::Result<Source> {
        let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
        let meta = fs::metadata(&abs)?;
        let modified = meta.modified().ok();
        let nanos = modified.and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        bytes.extend_from_slice(abs.as_os_str().as_encoded_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&meta.len().to_le_bytes());
        bytes.extend_from_slice(&nanos.to_le_bytes());
        let key = format!("{:016x}{:016x}.wav", fnv1a(&bytes, 0xcbf2_9ce4_8422_2325), fnv1a(&bytes, 0x6c62_272e_07bb_0142));
        Ok(Source { path: abs, size: meta.len(), modified, key })
    }
}

/// FNV-1a, 64 bit: stable across Rust versions (unlike `DefaultHasher`).
fn fnv1a(bytes: &[u8], basis: u64) -> u64 {
    bytes.iter().fold(basis, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

/// Removes decoded files not used for 30 days (and stale `.part` files). Best effort, never fails.
pub fn clean_cache(dir: &Path, keep: &[String]) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if keep.contains(&name) {
            continue;
        }
        let limit = if name.ends_with(".part") {
            PART_MAX_AGE
        } else if name.ends_with(".wav") {
            CACHE_MAX_AGE
        } else {
            continue;
        };
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|m| now.duration_since(m).ok()).map_or(false, |age| age > limit);
        if old {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// The cached decoded file of `src`, if it exists and is complete. Marks it as used.
pub fn cached(dir: &Path, src: &Source) -> Option<(PathBuf, WavInfo)> {
    let path = dir.join(&src.key);
    let info = wav::read_info(&path).ok()?;
    if info.repaired || info.data_len == 0 || info.sample_kind() != Some(wav::SampleKind::F32) {
        return None;
    }
    if let Ok(f) = File::options().write(true).open(&path) {
        let _ = f.set_modified(SystemTime::now());
    }
    Some((path, info))
}

/// Upper estimate of the decoded size in bytes (for the free-space check).
pub fn estimate_decoded_bytes(src: &Source) -> u64 {
    let ext = extension(&src.path);
    let Ok((r, _)) = SymReader::open(&src.path, &ext, true) else { return 0 };
    let (ch, rate) = (r.channels.max(1) as u64, r.rate.max(1) as u64);
    let frames = r.n_frames.unwrap_or_else(|| (src.size as f64 * 8.0 / FALLBACK_BITRATE * rate as f64) as u64);
    wav::output_size(16, frames * ch * 4)
}

pub fn check_space(dir: &Path, need: u64) -> Result<(), String> {
    if need == 0 {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|e| tf(Msg::CacheWriteFailed, &[("path", &dir.display()), ("e", &e)]))?;
    if let Some(free) = available_bytes(dir) {
        if free < need + (64 << 20) {
            return Err(tf(Msg::LowSpaceCache, &[("path", &dir.display()), ("need", &human_bytes(need)), ("free", &human_bytes(free))]));
        }
    }
    Ok(())
}

/// Reads through a file and counts the bytes consumed, for progress.
struct Counted {
    file: File,
    len: u64,
    pos: Arc<AtomicU64>,
}

impl Read for Counted {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.file.read(buf)?;
        self.pos.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

impl Seek for Counted {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let p = self.file.seek(to)?;
        self.pos.store(p, Ordering::Relaxed);
        Ok(p)
    }
}

impl MediaSource for Counted {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

/// `fmt ` payload of a 32-bit float WAV.
pub(crate) fn float_fmt(channels: u16, rate: u32) -> Vec<u8> {
    let ba = channels * 4;
    let mut f = Vec::with_capacity(16);
    f.extend_from_slice(&3u16.to_le_bytes());
    f.extend_from_slice(&channels.to_le_bytes());
    f.extend_from_slice(&rate.to_le_bytes());
    f.extend_from_slice(&(rate * ba as u32).to_le_bytes());
    f.extend_from_slice(&ba.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f
}

/// Decodes `src` into the cache (gapless) and returns the WAV file with its header.
/// `pos` follows the bytes of the source read so far.
pub fn decode_to_cache(dir: &Path, src: &Source, cancel: &AtomicBool, pos: Arc<AtomicU64>) -> Result<(PathBuf, WavInfo), String> {
    let cache_err = |e: &dyn std::fmt::Display| tf(Msg::CacheWriteFailed, &[("path", &dir.display()), ("e", e)]);
    fs::create_dir_all(dir).map_err(|e| cache_err(&e))?;
    let target = dir.join(&src.key);
    let part = dir.join(format!("{}.part", src.key));
    let result = (|| {
        let file = File::open(&src.path).map_err(|e| tf(Msg::UnreadableWith, &[("e", &e)]))?;
        let source = Counted { file, len: src.size, pos };
        let (mut reader, _) = SymReader::open_source(Box::new(source), &extension(&src.path), true)?;
        let (ch, rate) = (reader.channels, reader.rate);
        if ch == 0 || rate == 0 || ch > u16::MAX as usize / 4 {
            return Err(t(Msg::NoAudioData).to_string());
        }
        let fmt = float_fmt(ch as u16, rate);
        let mut out = BufWriter::with_capacity(4 << 20, File::create(&part).map_err(|e| cache_err(&e))?);
        wav::write_header(&mut out, &fmt, 0, 0, wav::RIFF_LIMIT).map_err(|e| cache_err(&e))?;
        let (mut buf, mut bytes, mut samples) = (Vec::new(), Vec::new(), 0u64);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(i18n::cancelled());
            }
            buf.clear();
            if !reader.read(&mut buf)? {
                break;
            }
            if reader.channels != ch || reader.rate != rate {
                return Err(t(Msg::StreamLayoutChanged).to_string());
            }
            bytes.clear();
            bytes.extend(buf.iter().flat_map(|v| if v.is_finite() { *v } else { 0.0 }.to_le_bytes()));
            out.write_all(&bytes).map_err(|e| cache_err(&e))?;
            samples += buf.len() as u64;
        }
        let frames = samples / ch as u64;
        if frames == 0 {
            return Err(t(Msg::NoAudioData).to_string());
        }
        // Whole frames only: a torn last frame is cut off with the length.
        let data = frames * ch as u64 * 4;
        let mut file = out.into_inner().map_err(|e| cache_err(e.error()))?;
        file.seek(SeekFrom::Start(0)).map_err(|e| cache_err(&e))?;
        wav::write_header(&mut file, &fmt, data, frames, wav::RIFF_LIMIT).map_err(|e| cache_err(&e))?;
        file.set_len(wav::output_size(fmt.len(), data)).map_err(|e| cache_err(&e))?;
        file.sync_all().map_err(|e| cache_err(&e))?;
        drop(file);
        fs::rename(&part, &target).map_err(|e| cache_err(&e))?;
        let info = wav::read_info(&target).map_err(|e| cache_err(&e))?;
        Ok((target.clone(), info))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

/// Creation time of an MP4/M4A file from its `mvhd` box, as seconds since 1970 (UTC). None if unset.
pub fn mp4_creation_time(path: &Path) -> Option<i64> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let moov = find_box(&mut f, 0, len, b"moov")?;
    let (body, end) = moov;
    let (mvhd, _) = find_box(&mut f, body, end, b"mvhd")?;
    f.seek(SeekFrom::Start(mvhd)).ok()?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    let secs_1904 = if head[0] == 1 { u64::from_be_bytes(head[4..12].try_into().ok()?) } else { u32::from_be_bytes(head[4..8].try_into().ok()?) as u64 };
    const MAC_EPOCH_OFFSET: i64 = 2_082_844_800;
    (secs_1904 > 0).then(|| secs_1904 as i64 - MAC_EPOCH_OFFSET)
}

/// Body start and end of the first box `kind` between `from` and `to`.
fn find_box(f: &mut File, from: u64, to: u64, kind: &[u8; 4]) -> Option<(u64, u64)> {
    let mut off = from;
    for _ in 0..4096 {
        if off + 8 > to {
            return None;
        }
        f.seek(SeekFrom::Start(off)).ok()?;
        let mut h = [0u8; 8];
        f.read_exact(&mut h).ok()?;
        let mut size = u32::from_be_bytes(h[0..4].try_into().ok()?) as u64;
        let mut head = 8;
        if size == 1 {
            let mut big = [0u8; 8];
            f.read_exact(&mut big).ok()?;
            size = u64::from_be_bytes(big);
            head = 16;
        } else if size == 0 {
            size = to - off;
        }
        if size < head {
            return None;
        }
        if &h[4..8] == kind {
            return Some((off + head, (off + size).min(to)));
        }
        off += size;
    }
    None
}
