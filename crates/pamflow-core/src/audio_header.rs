//! Reading a recording's header without decoding it (`pam_audio_info`).
//!
//! WAV follows tuneR 1.4.7 `readWave(header = TRUE)` step by step, quirks
//! included (no padding of odd chunk sizes, at most 5 skipped chunks, the
//! "seems to be corrupted" warning). FLAC reads STREAMINFO, as ffmpeg (R's
//! `av::av_media_info`) does; bits stay unknown, as in R.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// What the header says. `None` fields are NA in the CSV.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioInfo {
    pub sample_rate: Option<i64>,
    pub channels: Option<i64>,
    pub bits: Option<i64>,
    pub duration_s: Option<f64>,
    pub readable: bool,
}

/// The header, plus warnings R would raise while reading it.
pub fn audio_info(path: &Path) -> (AudioInfo, Vec<String>) {
    if !path.exists() {
        return (AudioInfo::default(), Vec::new());
    }
    let is_flac = path.extension().is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("flac"));
    if is_flac {
        return (flac_info(path).unwrap_or_default(), Vec::new());
    }
    let mut warnings = Vec::new();
    let info = match wav_header(path, &mut warnings) {
        Ok(h) => {
            let samples = h.data_length / (h.channels as f64 * (h.bits as f64 / 8.0));
            if samples.is_nan() {
                AudioInfo::default()
            } else {
                AudioInfo {
                    sample_rate: Some(h.sample_rate),
                    channels: Some(h.channels),
                    bits: Some(h.bits),
                    duration_s: Some(samples / h.sample_rate as f64),
                    readable: true,
                }
            }
        }
        Err(_) => AudioInfo::default(),
    };
    (info, warnings)
}

struct WavHeader {
    sample_rate: i64,
    channels: i64,
    bits: i64,
    data_length: f64,
}

/// A reader that behaves like R's connection: short reads at the end of the
/// file, seeking past the end allowed.
struct Con {
    f: File,
}

impl Con {
    /// `readChar(con, 4, useBytes = TRUE)`: None at the end of the file.
    fn chars4(&mut self) -> Option<Vec<u8>> {
        let mut b = Vec::with_capacity(4);
        let _ = (&mut self.f).take(4).read_to_end(&mut b);
        if b.is_empty() {
            None
        } else {
            // a NUL ends an R string
            if let Some(p) = b.iter().position(|&c| c == 0) {
                b.truncate(p);
            }
            Some(b)
        }
    }
    fn bytes(&mut self, n: usize) -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; n];
        let mut got = 0;
        while got < n {
            match self.f.read(&mut b[got..]) {
                Ok(0) => return Err("unexpected end of file".into()),
                Ok(k) => got += k,
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(b)
    }
    fn u32(&mut self) -> Result<u64, String> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64)
    }
    fn u16(&mut self) -> Result<i64, String> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]) as i64)
    }
    fn i16(&mut self) -> Result<i64, String> {
        let b = self.bytes(2)?;
        Ok(i16::from_le_bytes([b[0], b[1]]) as i64)
    }
    fn i32(&mut self) -> Result<i64, String> {
        let b = self.bytes(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64)
    }
    fn skip(&mut self, n: i64) -> Result<(), String> {
        self.f.seek(SeekFrom::Current(n)).map(|_| ()).map_err(|e| e.to_string())
    }
}

fn is(x: &Option<Vec<u8>>, tag: &[u8]) -> Result<bool, String> {
    match x {
        // R: comparing character(0) inside && is an error
        None => Err("zero-length comparison".into()),
        Some(v) => Ok(v.as_slice() == tag),
    }
}

fn wav_header(path: &Path, warnings: &mut Vec<String>) -> Result<WavHeader, String> {
    let mut c = Con { f: File::open(path).map_err(|e| e.to_string())? };
    let mut riff = c.chars4();
    let mut file_length = c.u32()?;
    let mut wave = c.chars4();
    let mut i = 0;
    while !(is(&riff, b"RIFF")? && is(&wave, b"WAVE")?) {
        i += 1;
        c.skip(file_length as i64 - 4)?;
        riff = c.chars4();
        file_length = c.u32()?;
        wave = c.chars4();
        if i > 5 {
            return Err("This seems not to be a valid RIFF file of type WAVE.".into());
        }
    }
    let mut fmt = c.chars4();
    if fmt.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(b"bext")) {
        let n = c.u32()?;
        c.bytes(n as usize)?;
        fmt = c.chars4();
    }
    let mut i = 0;
    while !is(&fmt, b"fmt ")? {
        i += 1;
        let len = c.u32()?;
        c.skip(len as i64)?;
        fmt = c.chars4();
        if i > 5 {
            return Err("There seems to be no 'fmt ' chunk in this Wave (?) file.".into());
        }
    }
    let fmt_length = c.u32()?;
    let pcm = c.u16()?;
    if ![0, 1, 3, 65534].contains(&pcm) {
        return Err("Only uncompressed PCM and IEEE_FLOAT Wave formats supported".into());
    }
    let channels = c.i16()?;
    let sample_rate = c.i32()?;
    let bytes_second = c.i32()?;
    let block_align = c.i16()?;
    let bits = c.i16()?;
    if ![8, 16, 24, 32, 64].contains(&bits) {
        return Err("Only 8-, 16-, 24-, 32- or 64-bit Wave formats supported".into());
    }
    let mut sub_format = None;
    if fmt_length >= 18 {
        let cb_size = c.i16()?;
        if cb_size == 22 && fmt_length == 40 {
            let _valid_bits = c.i16()?;
            let _mask = c.i32()?;
            sub_format = Some(c.u16()?);
            c.bytes(14)?;
        } else if cb_size >= 0 {
            c.skip(fmt_length as i64 - 18)?;
        }
    }
    if sub_format.is_some_and(|s| ![0, 1, 3].contains(&s)) {
        return Err("Only uncompressed PCM and IEEE_FLOAT Wave formats supported".into());
    }
    let mut data = c.chars4();
    let mut i = 0;
    while data.is_some() && !is(&data, b"data")? {
        i += 1;
        let len = c.u32()?;
        c.skip(len as i64)?;
        data = c.chars4();
        if i > 5 {
            return Err("There seems to be no 'data' chunk in this Wave (?) file.".into());
        }
    }
    if data.is_none() {
        return Err("No data chunk found".into());
    }
    let data_length = c.u32()? as f64;
    let bytes = bits as f64 / 8.0;
    if sample_rate * block_align != bytes_second || (channels as f64 * bytes) != block_align as f64 {
        warnings.push(format!("Wave file '{}' seems to be corrupted.", path.to_string_lossy()));
    }
    Ok(WavHeader { sample_rate, channels, bits, data_length })
}

/// FLAC STREAMINFO: rate, channels and the duration ffmpeg reports
/// (microsecond resolution). Unreadable when it is missing or invalid.
fn flac_info(path: &Path) -> Option<AudioInfo> {
    let mut f = File::open(path).ok()?;
    let mut head = [0u8; 10];
    f.read_exact(&mut head[..4]).ok()?;
    if &head[..3] == b"ID3" {
        // an ID3v2 tag in front: skip it (ffmpeg does)
        f.read_exact(&mut head[4..10]).ok()?;
        let size = head[6..10].iter().fold(0u64, |a, &b| (a << 7) | (b & 0x7f) as u64);
        let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
        f.seek(SeekFrom::Start(10 + size + footer)).ok()?;
        f.read_exact(&mut head[..4]).ok()?;
    }
    if &head[..4] != b"fLaC" {
        return None;
    }
    let mut bh = [0u8; 4];
    f.read_exact(&mut bh).ok()?;
    let (kind, len) = (bh[0] & 0x7f, u32::from_be_bytes([0, bh[1], bh[2], bh[3]]));
    if kind != 0 || len < 34 {
        return None;
    }
    let mut si = [0u8; 34];
    f.read_exact(&mut si).ok()?;
    let rate = ((si[10] as u64) << 12) | ((si[11] as u64) << 4) | ((si[12] as u64) >> 4);
    let channels = ((si[12] >> 1) & 0x07) as i64 + 1;
    let total = (((si[13] & 0x0f) as u64) << 32) | u32::from_be_bytes([si[14], si[15], si[16], si[17]]) as u64;
    if rate == 0 || total == 0 {
        return None;
    }
    // ffmpeg: duration in microseconds, rounded to nearest
    let micros = (total as u128 * 1_000_000 + rate as u128 / 2) / rate as u128;
    Some(AudioInfo {
        sample_rate: Some(rate as i64),
        channels: Some(channels),
        bits: None,
        duration_s: Some(micros as f64 / 1e6),
        readable: true,
    })
}
