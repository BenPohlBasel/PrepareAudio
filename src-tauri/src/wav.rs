//! Minimal WAV/RF64 reader and writer: just enough to read the chunks the
//! DJI Mic 2 writes and to produce a gapless, bit-identical concatenation.

use crate::i18n::{self, Lang, Msg};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// File size of a full chunk written by the DJI Mic 2 (338 MiB).
pub const CHUNK_FILE_SIZE: u64 = 354_418_688;
/// Largest size field a classic RIFF header can hold; above that we write RF64.
pub const RIFF_LIMIT: u64 = u32::MAX as u64;

#[derive(Debug, Clone, PartialEq)]
pub struct WavInfo {
    /// Raw payload of the `fmt ` chunk (copied verbatim into the output).
    pub fmt: Vec<u8>,
    pub format_tag: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub block_align: u16,
    /// Absolute file offset of the first audio byte.
    pub data_offset: u64,
    /// Usable audio bytes, always a multiple of `block_align`.
    pub data_len: u64,
    pub file_size: u64,
    /// The header's data size was missing or wrong and was derived from the file size.
    pub repaired: bool,
    /// Broadcast WAV `bext` chunk, if the recorder wrote one.
    pub bext: Option<Bext>,
    /// File-set fields of an `iXML` chunk, if present.
    pub ixml: Option<IxmlSet>,
}

/// `<FILE_SET>` of an iXML chunk: files of one take share FAMILY_UID and are
/// numbered by FILE_SET_INDEX (digits or letters) out of TOTAL_FILES.
#[derive(Debug, Clone, PartialEq)]
pub struct IxmlSet {
    pub family_uid: String,
    pub index: String,
    pub total: Option<u32>,
}

/// Largest iXML chunk read (the file-set fields sit near the top).
const IXML_MAX: u64 = 256 * 1024;

/// Extracts the file-set fields from iXML text. Needs FAMILY_UID and FILE_SET_INDEX.
pub fn parse_ixml(text: &str) -> Option<IxmlSet> {
    let tag = |name: &str| -> Option<String> {
        let open = format!("<{name}>");
        let start = text.find(&open)? + open.len();
        let end = start + text[start..].find(&format!("</{name}>"))?;
        let v = text[start..end].trim();
        (!v.is_empty()).then(|| v.to_string())
    };
    Some(IxmlSet { family_uid: tag("FAMILY_UID")?, index: tag("FILE_SET_INDEX")?, total: tag("TOTAL_FILES").and_then(|t| t.parse().ok()) })
}

fn read_ixml(f: &mut File, size: u64) -> io::Result<Option<IxmlSet>> {
    let mut b = vec![0u8; size.min(IXML_MAX) as usize];
    f.read_exact(&mut b)?;
    Ok(parse_ixml(&String::from_utf8_lossy(&b)))
}

/// The time fields of a Broadcast WAV `bext` chunk (EBU Tech 3285).
#[derive(Debug, Clone, PartialEq)]
pub struct Bext {
    /// OriginationDate (local), if valid: (year, month, day).
    pub date: Option<(i64, i64, i64)>,
    /// OriginationTime (local), if valid: seconds since midnight.
    pub time: Option<i64>,
    /// TimeReference: first sample counted in samples since midnight. 0 when unset.
    pub time_reference: u64,
}

/// Offsets inside the bext body: Description 256, Originator 32, OriginatorReference 32,
/// OriginationDate 10, OriginationTime 8, TimeReference 8.
const BEXT_DATE: usize = 320;
const BEXT_TIME: usize = 330;
const BEXT_TIME_REF: usize = 338;
const BEXT_MIN_LEN: usize = 346;

/// Parses the time fields of a bext body. Date "yyyy:mm:dd" and time "hh:mm:ss"
/// accept any single separator, as the standard allows.
pub fn parse_bext(body: &[u8]) -> Option<Bext> {
    if body.len() < BEXT_MIN_LEN {
        return None;
    }
    let num = |s: &[u8]| -> Option<i64> {
        if s.iter().all(u8::is_ascii_digit) {
            Some(s.iter().fold(0i64, |a, c| a * 10 + (c - b'0') as i64))
        } else {
            None
        }
    };
    let d = &body[BEXT_DATE..BEXT_DATE + 10];
    let date = (|| {
        let (y, m, day) = (num(&d[0..4])?, num(&d[5..7])?, num(&d[8..10])?);
        // Unset device clocks write 1970, 1980, 2000 …: kept, the scan warns about them.
        let ok = (1970..=2100).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&day);
        ok.then_some((y, m, day))
    })();
    let t = &body[BEXT_TIME..BEXT_TIME + 8];
    let time = (|| {
        let (h, mi, s) = (num(&t[0..2])?, num(&t[3..5])?, num(&t[6..8])?);
        (h <= 23 && mi <= 59 && s <= 59).then_some(h * 3600 + mi * 60 + s)
    })();
    let time_reference = u64::from_le_bytes(body[BEXT_TIME_REF..BEXT_TIME_REF + 8].try_into().unwrap());
    Some(Bext { date, time, time_reference })
}

fn read_bext(f: &mut File, size: u64) -> io::Result<Option<Bext>> {
    if size < BEXT_MIN_LEN as u64 {
        return Ok(None);
    }
    let mut b = vec![0u8; BEXT_MIN_LEN];
    f.read_exact(&mut b)?;
    Ok(parse_bext(&b))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SampleKind {
    U8,
    I16,
    I24,
    I32,
    F32,
    F64,
}

fn invalid(msg: Msg) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, i18n::t(msg))
}

impl WavInfo {
    pub fn frames(&self) -> u64 {
        self.data_len / self.block_align as u64
    }

    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.sample_rate as f64
    }

    /// Format tag with WAVE_FORMAT_EXTENSIBLE resolved to its sub format.
    pub fn base_tag(&self) -> u16 {
        if self.format_tag == 0xFFFE && self.fmt.len() >= 26 {
            u16::from_le_bytes([self.fmt[24], self.fmt[25]])
        } else {
            self.format_tag
        }
    }

    pub fn sample_kind(&self) -> Option<SampleKind> {
        let bytes = (self.block_align / self.channels.max(1)) as usize;
        match (self.base_tag(), bytes) {
            (1, 1) => Some(SampleKind::U8),
            (1, 2) => Some(SampleKind::I16),
            (1, 3) => Some(SampleKind::I24),
            (1, 4) => Some(SampleKind::I32),
            (3, 4) => Some(SampleKind::F32),
            (3, 8) => Some(SampleKind::F64),
            _ => None,
        }
    }

    /// e.g. "48 kHz · 32-bit float · Mono", in the current language.
    pub fn format_label(&self) -> String {
        self.format_label_in(i18n::current())
    }

    pub fn format_label_in(&self, lang: Lang) -> String {
        let kind = match self.base_tag() {
            3 => "float",
            1 => "PCM",
            _ => "Codec",
        };
        let ch = i18n::channels(lang, self.channels as u32);
        format!("{} · {}-bit {kind} · {ch}", i18n::khz(lang, self.sample_rate), self.bits_per_sample)
    }
}

/// Reads the header of a RIFF or RF64 WAVE file.
pub fn read_info(path: &Path) -> io::Result<WavInfo> {
    let mut f = File::open(path)?;
    let file_size = f.metadata()?.len();
    let mut hdr = [0u8; 12];
    f.read_exact(&mut hdr).map_err(|_| invalid(Msg::FileTooShort))?;
    if !(&hdr[0..4] == b"RIFF" || &hdr[0..4] == b"RF64") || &hdr[8..12] != b"WAVE" {
        return Err(invalid(Msg::NotWav));
    }
    let mut ds64_data: Option<u64> = None;
    let mut fmt: Option<Vec<u8>> = None;
    let mut bext: Option<Bext> = None;
    let mut ixml: Option<IxmlSet> = None;
    let mut off = 12u64;
    while off + 8 <= file_size {
        f.seek(SeekFrom::Start(off))?;
        let mut ch = [0u8; 8];
        f.read_exact(&mut ch)?;
        let id: [u8; 4] = [ch[0], ch[1], ch[2], ch[3]];
        let size = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]) as u64;
        let body = off + 8;
        match &id {
            b"bext" => bext = read_bext(&mut f, size)?,
            b"iXML" => ixml = read_ixml(&mut f, size.min(file_size - body))?,
            b"ds64" if size >= 16 => {
                let mut b = [0u8; 16];
                f.read_exact(&mut b)?;
                ds64_data = Some(u64::from_le_bytes(b[8..16].try_into().unwrap()));
            }
            b"fmt " => {
                if !(16..=1024).contains(&size) {
                    return Err(invalid(Msg::InvalidFmtChunk));
                }
                let mut b = vec![0u8; size as usize];
                f.read_exact(&mut b)?;
                fmt = Some(b);
            }
            b"data" => {
                let fmt = fmt.ok_or_else(|| invalid(Msg::DataBeforeFmt))?;
                let le16 = |i: usize| u16::from_le_bytes([fmt[i], fmt[i + 1]]);
                let format_tag = le16(0);
                let channels = le16(2);
                let sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
                let block_align = le16(12);
                let bits_per_sample = le16(14);
                if channels == 0 || sample_rate == 0 || block_align == 0 {
                    return Err(invalid(Msg::InvalidAudioFormat));
                }
                let available = file_size - body;
                let declared = if size == 0xFFFF_FFFF {
                    ds64_data.unwrap_or(available)
                } else {
                    size
                };
                let (len, repaired) = if declared == 0 || declared > available {
                    (available, true)
                } else {
                    (declared, false)
                };
                let data_len = len - len % block_align as u64;
                // Some writers put metadata after the audio; look there only if the data size is trustworthy.
                if (bext.is_none() || ixml.is_none()) && !repaired {
                    let (b, x) = metadata_after(&mut f, body + declared + (declared & 1), file_size)?;
                    bext = bext.or(b);
                    ixml = ixml.or(x);
                }
                return Ok(WavInfo {
                    fmt,
                    format_tag,
                    channels,
                    sample_rate,
                    bits_per_sample,
                    block_align,
                    data_offset: body,
                    data_len,
                    file_size,
                    repaired,
                    bext,
                    ixml,
                });
            }
            _ => {}
        }
        off = body + size + (size & 1);
    }
    Err(invalid(Msg::NoDataChunk))
}

/// Looks for bext and iXML among the (few) chunks following the audio data.
fn metadata_after(f: &mut File, mut off: u64, file_size: u64) -> io::Result<(Option<Bext>, Option<IxmlSet>)> {
    let (mut bext, mut ixml) = (None, None);
    for _ in 0..8 {
        if off + 8 > file_size {
            break;
        }
        f.seek(SeekFrom::Start(off))?;
        let mut ch = [0u8; 8];
        if f.read_exact(&mut ch).is_err() {
            break;
        }
        let size = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]) as u64;
        let avail = size.min(file_size - off - 8);
        match &ch[0..4] {
            b"bext" => bext = read_bext(f, avail).ok().flatten(),
            b"iXML" => ixml = read_ixml(f, avail).ok().flatten(),
            _ => {}
        }
        off += 8 + size + (size & 1);
    }
    Ok((bext, ixml))
}

/// Bytes before the first audio byte in files written by [`write_header`].
pub fn header_len(fmt_len: usize) -> u64 {
    let fmt_len = fmt_len as u64;
    12 + (8 + 28) + (8 + fmt_len + (fmt_len & 1)) + 8
}

/// Total size of a file written by [`write_header`] plus `data_len` audio bytes.
pub fn output_size(fmt_len: usize, data_len: u64) -> u64 {
    header_len(fmt_len) + data_len + (data_len & 1)
}

/// Writes a WAVE header. Below `riff_limit` it is a classic RIFF header with a
/// 28-byte JUNK placeholder; above it the same layout becomes RF64 with ds64.
pub fn write_header<W: Write>(
    w: &mut W,
    fmt: &[u8],
    data_len: u64,
    frames: u64,
    riff_limit: u64,
) -> io::Result<()> {
    let riff_size = output_size(fmt.len(), data_len) - 8;
    let rf64 = riff_size > riff_limit || data_len > riff_limit;
    let mut h = Vec::with_capacity(header_len(fmt.len()) as usize);
    if rf64 {
        h.extend_from_slice(b"RF64");
        h.extend_from_slice(&u32::MAX.to_le_bytes());
        h.extend_from_slice(b"WAVE");
        h.extend_from_slice(b"ds64");
        h.extend_from_slice(&28u32.to_le_bytes());
        h.extend_from_slice(&riff_size.to_le_bytes());
        h.extend_from_slice(&data_len.to_le_bytes());
        h.extend_from_slice(&frames.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
    } else {
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&(riff_size as u32).to_le_bytes());
        h.extend_from_slice(b"WAVE");
        h.extend_from_slice(b"JUNK");
        h.extend_from_slice(&28u32.to_le_bytes());
        h.extend_from_slice(&[0u8; 28]);
    }
    h.extend_from_slice(b"fmt ");
    h.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    h.extend_from_slice(fmt);
    if fmt.len() % 2 == 1 {
        h.push(0);
    }
    h.extend_from_slice(b"data");
    let data_field = if rf64 { u32::MAX } else { data_len as u32 };
    h.extend_from_slice(&data_field.to_le_bytes());
    w.write_all(&h)
}

pub fn read_at(path: &Path, offset: u64, len: u64) -> io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut b = vec![0u8; len as usize];
    f.read_exact(&mut b)?;
    Ok(b)
}

pub fn decode_sample(b: &[u8], kind: SampleKind) -> f64 {
    let v = match kind {
        SampleKind::U8 => (b[0] as f64 - 128.0) / 128.0,
        SampleKind::I16 => i16::from_le_bytes([b[0], b[1]]) as f64 / 32768.0,
        SampleKind::I24 => {
            let raw = (b[0] as i32) | ((b[1] as i32) << 8) | ((b[2] as i32) << 16);
            ((raw << 8) >> 8) as f64 / 8_388_608.0
        }
        SampleKind::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 2_147_483_648.0,
        SampleKind::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
        SampleKind::F64 => f64::from_le_bytes(b[..8].try_into().unwrap()),
    };
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn reads_dji_style_header() {
        let dir = tempdir("wav-read");
        let p = dir.join("DJI_01_20260101_100000.WAV");
        write_float_wav(&p, 48000, &sine(440.0, 0.5, 48000, 0, 4800));
        let info = read_info(&p).unwrap();
        assert_eq!(info.channels, 1);
        assert_eq!(info.sample_rate, 48000);
        assert_eq!(info.frames(), 4800);
        assert_eq!(info.data_offset, 44);
        assert_eq!(info.sample_kind(), Some(SampleKind::F32));
        assert!(!info.repaired);
        assert_eq!(info.format_label(), "48 kHz · 32-bit float · Mono");
    }

    #[test]
    fn reads_bext_before_and_after_data() {
        let dir = tempdir("wav-bext");
        let tr = 11 * 3600 * 48_000 + 123;
        let before = dir.join("before.wav");
        write_float_bwf(&before, 48000, &sine(440.0, 0.5, 48000, 0, 480), "2026-09-06", "11:00:00", tr);
        let info = read_info(&before).unwrap();
        assert_eq!(info.frames(), 480);
        assert_eq!(info.bext, Some(Bext { date: Some((2026, 9, 6)), time: Some(39_600), time_reference: tr }));
        let after = dir.join("after.wav");
        write_wav_with(&after, 48000, &sine(440.0, 0.5, 48000, 0, 480), &[], &bext_chunk("2026/09/07", "23.59.59", 0));
        let info = read_info(&after).unwrap();
        assert_eq!(info.data_offset, 44);
        assert_eq!(info.bext, Some(Bext { date: Some((2026, 9, 7)), time: Some(86_399), time_reference: 0 }));
        let blank = dir.join("blank.wav");
        write_float_bwf(&blank, 48000, &sine(440.0, 0.5, 48000, 0, 480), "\0\0\0\0\0\0\0\0\0\0", "\0\0\0\0\0\0\0\0", 0);
        assert_eq!(read_info(&blank).unwrap().bext, Some(Bext { date: None, time: None, time_reference: 0 }));
        write_float_wav(&dir.join("plain.wav"), 48000, &sine(440.0, 0.5, 48000, 0, 480));
        assert_eq!(read_info(&dir.join("plain.wav")).unwrap().bext, None);
        assert_eq!(read_info(&dir.join("plain.wav")).unwrap().ixml, None);

        let set = IxmlSet { family_uid: "F00D-1".into(), index: "2".into(), total: Some(5) };
        let x1 = dir.join("ixml-before.wav");
        write_wav_with(&x1, 48000, &sine(440.0, 0.5, 48000, 0, 480), &ixml_chunk("F00D-1", "2", 5), &[]);
        assert_eq!(read_info(&x1).unwrap().ixml, Some(set.clone()));
        let x2 = dir.join("ixml-after.wav");
        write_wav_with(&x2, 48000, &sine(440.0, 0.5, 48000, 0, 480), &bext_chunk("2026-09-06", "11:00:00", 5), &ixml_chunk("F00D-1", "2", 5));
        let info = read_info(&x2).unwrap();
        assert_eq!((info.ixml, info.bext.map(|b| b.time_reference)), (Some(set), Some(5)));
        assert_eq!(parse_ixml("<BWFXML><FILE_SET><FAMILY_UID></FAMILY_UID><FILE_SET_INDEX>A</FILE_SET_INDEX></FILE_SET></BWFXML>"), None);
    }

    #[test]
    fn format_label_in_every_language() {
        let dir = tempdir("wav-label");
        let p = dir.join("stereo.wav");
        let mut stereo = fmt_float(44_100);
        stereo[2..4].copy_from_slice(&2u16.to_le_bytes());
        stereo[12..14].copy_from_slice(&8u16.to_le_bytes());
        let mut out = Vec::new();
        write_header(&mut out, &stereo, 80, 10, RIFF_LIMIT).unwrap();
        out.extend_from_slice(&[0u8; 80]);
        std::fs::write(&p, &out).unwrap();
        let mut info = read_info(&p).unwrap();
        assert_eq!(info.format_label_in(Lang::De), "44,1 kHz · 32-bit float · Stereo");
        assert_eq!(info.format_label_in(Lang::En), "44.1 kHz · 32-bit float · Stereo");
        assert_eq!(info.format_label_in(Lang::Fr), "44,1 kHz · 32-bit float · Stéréo");
        assert_eq!(info.format_label_in(Lang::It), "44,1 kHz · 32-bit float · Stereo");
        info.channels = 4;
        info.sample_rate = 48_000;
        assert_eq!(info.format_label_in(Lang::De), "48 kHz · 32-bit float · 4 Kanäle");
        assert_eq!(info.format_label_in(Lang::En), "48 kHz · 32-bit float · 4 channels");
        assert_eq!(info.format_label_in(Lang::Fr), "48 kHz · 32-bit float · 4 canaux");
        assert_eq!(info.format_label_in(Lang::It), "48 kHz · 32-bit float · 4 canali");
    }

    #[test]
    fn repairs_missing_data_size() {
        let dir = tempdir("wav-repair");
        let p = dir.join("broken.wav");
        write_float_wav(&p, 48000, &sine(440.0, 0.5, 48000, 0, 1000));
        // Zero the data size field like a recorder that lost power, and add a stray byte.
        let mut bytes = std::fs::read(&p).unwrap();
        bytes[40..44].copy_from_slice(&0u32.to_le_bytes());
        bytes.push(7);
        std::fs::write(&p, &bytes).unwrap();
        let info = read_info(&p).unwrap();
        assert!(info.repaired);
        assert_eq!(info.frames(), 1000);
    }

    #[test]
    fn riff_and_rf64_roundtrip() {
        let dir = tempdir("wav-rf64");
        let fmt = fmt_float(8000);
        let data: Vec<u8> = (0u32..1000).flat_map(|i| (i as f32).to_le_bytes()).collect();
        for (name, limit) in [("riff.wav", RIFF_LIMIT), ("rf64.wav", 100)] {
            let p = dir.join(name);
            let mut out = Vec::new();
            write_header(&mut out, &fmt, data.len() as u64, 1000, limit).unwrap();
            assert_eq!(out.len() as u64, header_len(fmt.len()));
            out.extend_from_slice(&data);
            std::fs::write(&p, &out).unwrap();
            let info = read_info(&p).unwrap();
            assert_eq!(&out[0..4], if limit == 100 { b"RF64" } else { b"RIFF" });
            assert_eq!(info.data_len, data.len() as u64);
            assert_eq!(info.fmt, fmt);
            assert_eq!(info.file_size, output_size(fmt.len(), data.len() as u64));
            assert!(!info.repaired);
            let back = read_at(&p, info.data_offset, info.data_len).unwrap();
            assert_eq!(back, data);
        }
    }
}
