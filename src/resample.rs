//! Windowed-sinc sample-rate conversion and down-mixing.
//!
//! Normally Windows converts the microphone signal to 16 kHz mono for us
//! (`AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM`). This is the fallback for devices/drivers where that fails, and it is
//! used to read test WAV files at other rates.

use std::f64::consts::PI;

const HALF_TAPS: f64 = 16.0;

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// Blackman window over [-1, 1].
fn window(x: f64) -> f64 {
    if x.abs() >= 1.0 {
        0.0
    } else {
        let t = (x + 1.0) / 2.0;
        0.42 - 0.5 * (2.0 * PI * t).cos() + 0.08 * (4.0 * PI * t).cos()
    }
}

/// Resamples a complete mono signal from `from` Hz to `to` Hz.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    // Low-pass at 95 % of the lower Nyquist frequency.
    let cutoff = ratio.min(1.0) * 0.95;
    let half_width = HALF_TAPS / cutoff; // in input samples
    let out_len = ((input.len() as f64) * ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let t = i as f64 / ratio; // position in input samples
        let lo = (t - half_width).ceil().max(0.0) as usize;
        let hi = ((t + half_width).floor() as usize).min(input.len() - 1);
        let mut acc = 0.0;
        let mut norm = 0.0;
        for (k, &x) in input.iter().enumerate().take(hi + 1).skip(lo) {
            let d = t - k as f64;
            let w = cutoff * sinc(cutoff * d) * window(d / half_width);
            acc += x as f64 * w;
            norm += w;
        }
        out.push(if norm.abs() > 1e-9 { (acc / norm) as f32 } else { 0.0 });
    }
    out
}

/// Averages interleaved channels into mono.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f64, rate: u32, secs: f64) -> Vec<f32> {
        (0..(rate as f64 * secs) as usize)
            .map(|i| (2.0 * PI * freq * i as f64 / rate as f64).sin() as f32 * 0.5)
            .collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn keeps_in_band_tone() {
        let x = tone(1000.0, 48_000, 0.5);
        let y = resample(&x, 48_000, 16_000);
        assert_eq!(y.len(), 8000);
        let r = rms(&y[500..7500]);
        assert!((r - 0.3535).abs() < 0.02, "rms {r}");
    }

    #[test]
    fn removes_aliasing_tone() {
        // 12 kHz cannot be represented at 16 kHz; it must be filtered, not folded to 4 kHz
        let x = tone(12_000.0, 48_000, 0.5);
        let y = resample(&x, 48_000, 16_000);
        assert!(rms(&y[500..7500]) < 0.01);
    }

    #[test]
    fn upsampling_and_identity() {
        let x = tone(440.0, 8_000, 0.25);
        assert_eq!(resample(&x, 8_000, 16_000).len(), 4000);
        assert_eq!(resample(&x, 16_000, 16_000), x);
    }

    #[test]
    fn downmix_stereo() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
    }
}
