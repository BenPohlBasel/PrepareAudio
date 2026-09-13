//! Minimal WAV/RF64 reader and writer: just enough to read the chunks the
//! DJI Mic 2 writes and to produce a gapless, bit-identical concatenation.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// File size of a full chunk written by the DJI Mic 2 (338 MiB).
pub const DJI_CHUNK_FILE_SIZE: u64 = 354_418_688;
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

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
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

    pub fn format_label(&self) -> String {
        let kind = match self.base_tag() {
            3 => "float",
            1 => "PCM",
            _ => "Codec",
        };
        let ch = match self.channels {
            1 => "Mono".to_string(),
            2 => "Stereo".to_string(),
            n => format!("{n} Kanäle"),
        };
        let khz = self.sample_rate as f64 / 1000.0;
        let khz = if khz.fract() == 0.0 {
            format!("{}", khz as u64)
        } else {
            format!("{khz:.1}")
        };
        format!("{khz} kHz · {}-bit {kind} · {ch}", self.bits_per_sample)
    }
}

/// Reads the header of a RIFF or RF64 WAVE file.
pub fn read_info(path: &Path) -> io::Result<WavInfo> {
    let mut f = File::open(path)?;
    let file_size = f.metadata()?.len();
    let mut hdr = [0u8; 12];
    f.read_exact(&mut hdr).map_err(|_| invalid("Datei zu kurz"))?;
    if !(&hdr[0..4] == b"RIFF" || &hdr[0..4] == b"RF64") || &hdr[8..12] != b"WAVE" {
        return Err(invalid("keine WAV-Datei"));
    }
    let mut ds64_data: Option<u64> = None;
    let mut fmt: Option<Vec<u8>> = None;
    let mut off = 12u64;
    while off + 8 <= file_size {
        f.seek(SeekFrom::Start(off))?;
        let mut ch = [0u8; 8];
        f.read_exact(&mut ch)?;
        let id: [u8; 4] = [ch[0], ch[1], ch[2], ch[3]];
        let size = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]) as u64;
        let body = off + 8;
        match &id {
            b"ds64" if size >= 16 => {
                let mut b = [0u8; 16];
                f.read_exact(&mut b)?;
                ds64_data = Some(u64::from_le_bytes(b[8..16].try_into().unwrap()));
            }
            b"fmt " => {
                if !(16..=1024).contains(&size) {
                    return Err(invalid("ungültiger fmt-Chunk"));
                }
                let mut b = vec![0u8; size as usize];
                f.read_exact(&mut b)?;
                fmt = Some(b);
            }
            b"data" => {
                let fmt = fmt.ok_or_else(|| invalid("data-Chunk vor fmt-Chunk"))?;
                let le16 = |i: usize| u16::from_le_bytes([fmt[i], fmt[i + 1]]);
                let format_tag = le16(0);
                let channels = le16(2);
                let sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
                let block_align = le16(12);
                let bits_per_sample = le16(14);
                if channels == 0 || sample_rate == 0 || block_align == 0 {
                    return Err(invalid("ungültiges Audioformat"));
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
                });
            }
            _ => {}
        }
        off = body + size + (size & 1);
    }
    Err(invalid("kein data-Chunk gefunden"))
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
