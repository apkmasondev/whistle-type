//! Minimal RIFF/WAVE reader and writer (used by tests, the benchmark tool and the e2e audio source).
//! The app itself never writes recordings to disk.

use std::io::{self, Read};
use std::path::Path;

pub struct Wav {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved samples in [-1, 1].
    pub samples: Vec<f32>,
}

impl Wav {
    /// Mono, 16 kHz, ready for Whistle.
    pub fn to_whistle_input(&self) -> Vec<f32> {
        let mono = crate::resample::downmix(&self.samples, self.channels as usize);
        crate::resample::resample(&mono, self.sample_rate, 16_000)
    }
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

pub fn read(path: &Path) -> io::Result<Wav> {
    let mut data = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut data)?;
    parse(&data)
}

pub fn parse(data: &[u8]) -> io::Result<Wav> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(bad("not a RIFF/WAVE file"));
    }
    let mut pos = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body_start = pos + 8;
        let body_end = (body_start + size).min(data.len());
        let body = &data[body_start..body_end];
        if id == b"fmt " {
            if body.len() < 16 {
                return Err(bad("short fmt chunk"));
            }
            let mut tag = u16::from_le_bytes([body[0], body[1]]);
            let channels = u16::from_le_bytes([body[2], body[3]]);
            let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if tag == 0xFFFE && body.len() >= 26 {
                tag = u16::from_le_bytes([body[24], body[25]]); // sub-format GUID's first two bytes
            }
            fmt = Some((tag, channels, rate, bits));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = fmt.ok_or_else(|| bad("data before fmt"))?;
            if channels == 0 || rate == 0 {
                return Err(bad("invalid format"));
            }
            let samples: Vec<f32> = match (tag, bits) {
                (1, 16) => body.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect(),
                (1, 24) => body
                    .chunks_exact(3)
                    .map(|b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0)
                    .collect(),
                (1, 32) => body
                    .chunks_exact(4)
                    .map(|b| i32::from_le_bytes(b.try_into().unwrap()) as f32 / 2_147_483_648.0)
                    .collect(),
                (1, 8) => body.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
                (3, 32) => body.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect(),
                _ => return Err(bad(&format!("unsupported WAV encoding (format {tag}, {bits} bit)"))),
            };
            return Ok(Wav { sample_rate: rate, channels, samples });
        }
        pos = body_start + size + (size & 1);
    }
    Err(bad("no data chunk"))
}

/// Writes 16-bit PCM mono.
pub fn write_pcm16(path: &Path, samples: &[f32], rate: u32) -> io::Result<()> {
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = (samples.len() * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_pcm16() {
        let dir = std::env::temp_dir().join(format!("wt-wav-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.wav");
        let s: Vec<f32> = (0..1600).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        write_pcm16(&p, &s, 16_000).unwrap();
        let w = read(&p).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(w.sample_rate, 16_000);
        assert_eq!(w.channels, 1);
        assert_eq!(w.samples.len(), s.len());
        assert!(w.samples.iter().zip(&s).all(|(a, b)| (a - b).abs() < 1e-3));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"hello world, definitely not a wav").is_err());
        assert!(parse(b"").is_err());
    }
}
