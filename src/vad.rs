//! Lightweight, conservative speech gate.
//!
//! Whistle itself already returns an empty transcript for silence and steady noise (documented in `needle.h`
//! and verified locally). This gate only rejects recordings that are *clearly* not speech, so the engine is
//! not even called (and nothing can be typed):
//! * shorter than [`MIN_DURATION_S`] (accidental tap of the hotkey),
//! * digital silence / a muted microphone,
//! * steady noise with no dynamics (fan, hum, hiss),
//! * a single click or knock (only a few loud frames).
//!
//! Everything else goes to Whistle. The thresholds are deliberately loose: rejecting real speech is worse than
//! letting Whistle return an empty string.

pub const SAMPLE_RATE: usize = 16_000;
const FRAME: usize = SAMPLE_RATE / 50; // 20 ms

pub const MIN_DURATION_S: f32 = 0.25;
/// Peak below this level means nothing was picked up at all.
const SILENCE_PEAK_DBFS: f32 = -50.0;
/// Loud frames must rise this far above the noise floor to count as "active".
const ACTIVE_ABOVE_FLOOR_DB: f32 = 12.0;
/// Below this dynamic range (p95 - p10) the signal is stationary noise.
const MIN_DYNAMIC_RANGE_DB: f32 = 6.0;
/// A click/knock lasts at most a few frames; real words have more active frames than this.
const MIN_ACTIVE_FRAMES: usize = 6; // 120 ms

/// Speech contains voiced sounds (vowels): periodic frames with a pitch between 70 and 400 Hz.
/// Keyboard clicks, breathing, rustling and most noise do not. Measured on the test set, Whistle invents
/// "Dziękuję bardzo." / "Thank you." with ~0.95 word probability for typing and breathing noise, so the
/// model's confidence cannot be used to catch this - periodicity can.
const MIN_VOICED_S: f32 = 0.12;
const VOICING_THRESHOLD: f32 = 0.55;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechStats {
    pub duration_s: f32,
    pub peak_dbfs: f32,
    pub noise_floor_dbfs: f32,
    pub p95_dbfs: f32,
    pub active_s: f32,
    /// Seconds of voiced (periodic) frames found; counting stops once it is clearly enough.
    pub voiced_s: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Speech,
    TooShort,
    Silence,
    NoSpeech,
}

fn db(power: f64) -> f32 {
    (10.0 * (power + 1e-12).log10()) as f32
}

fn frame_levels(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(FRAME)
        .filter(|c| c.len() >= FRAME / 2)
        .map(|c| db(c.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / c.len() as f64))
        .collect()
}

fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return -120.0;
    }
    let idx = ((sorted.len() - 1) as f32 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

pub fn analyze(samples: &[f32]) -> SpeechStats {
    let duration_s = samples.len() as f32 / SAMPLE_RATE as f32;
    let peak = samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    let peak_dbfs = 20.0 * (peak.max(1e-6)).log10();
    let levels = frame_levels(samples);
    let mut sorted = levels.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let floor = percentile(&sorted, 0.10).max(-120.0);
    let p95 = percentile(&sorted, 0.95).max(-120.0);
    let threshold = (floor + ACTIVE_ABOVE_FLOOR_DB).max(-60.0);
    let active = levels.iter().filter(|&&l| l > threshold).count();
    let voiced_s = voiced_seconds(samples, (floor + 6.0).max(-55.0), MIN_VOICED_S * 3.0);
    SpeechStats {
        duration_s,
        peak_dbfs,
        noise_floor_dbfs: floor,
        p95_dbfs: p95,
        active_s: active as f32 * FRAME as f32 / SAMPLE_RATE as f32,
        voiced_s,
    }
}

/// Seconds of frames that are loud enough and periodic with a pitch in 70..400 Hz
/// (normalised autocorrelation on a 2x decimated signal). Stops early once `enough` is reached.
fn voiced_seconds(samples: &[f32], min_level_db: f32, enough: f32) -> f32 {
    const RATE: usize = SAMPLE_RATE / 2; // 8 kHz
    const WIN: usize = 256; // 32 ms
    const HOP: usize = 128; // 16 ms
    const MIN_LAG: usize = RATE / 400;
    const MAX_LAG: usize = RATE / 70;
    // [1 2 1]/4 low-pass, then keep every second sample
    let d: Vec<f32> = (0..samples.len() / 2)
        .map(|i| {
            let a = samples[(2 * i).saturating_sub(1)];
            let b = samples[2 * i];
            let c = *samples.get(2 * i + 1).unwrap_or(&b);
            0.25 * a + 0.5 * b + 0.25 * c
        })
        .collect();
    let mut voiced = 0usize;
    let mut pos = 0;
    let hop_s = HOP as f32 / RATE as f32;
    while pos + WIN + MAX_LAG <= d.len() {
        let frame = &d[pos..pos + WIN];
        let mean = frame.iter().sum::<f32>() / WIN as f32;
        let energy: f32 = frame.iter().map(|x| (x - mean) * (x - mean)).sum();
        if db((energy / WIN as f32) as f64) > min_level_db {
            let mut best = 0.0f32;
            for lag in MIN_LAG..=MAX_LAG {
                let other = &d[pos + lag..pos + lag + WIN];
                let mut xy = 0.0f32;
                let mut yy = 0.0f32;
                for (x, y) in frame.iter().zip(other) {
                    let (x, y) = (x - mean, y - mean);
                    xy += x * y;
                    yy += y * y;
                }
                let r = xy / (energy * yy).sqrt().max(1e-12);
                if r > best {
                    best = r;
                }
            }
            if best > VOICING_THRESHOLD {
                voiced += 1;
                if voiced as f32 * hop_s >= enough {
                    break;
                }
            }
        }
        pos += HOP;
    }
    voiced as f32 * hop_s
}

pub fn classify(stats: &SpeechStats) -> Verdict {
    if stats.duration_s < MIN_DURATION_S {
        return Verdict::TooShort;
    }
    if stats.peak_dbfs < SILENCE_PEAK_DBFS {
        return Verdict::Silence;
    }
    if stats.p95_dbfs - stats.noise_floor_dbfs < MIN_DYNAMIC_RANGE_DB {
        return Verdict::NoSpeech;
    }
    let active_frames = (stats.active_s * 50.0).round() as usize;
    if active_frames < MIN_ACTIVE_FRAMES {
        return Verdict::NoSpeech;
    }
    if stats.voiced_s < MIN_VOICED_S {
        return Verdict::NoSpeech;
    }
    Verdict::Speech
}

/// Range of `samples` without long leading/trailing silence. Generous padding is kept so that soft onsets
/// and word endings are never cut. Returns the full range when there is nothing worth trimming.
pub fn trim_range(samples: &[f32], stats: &SpeechStats) -> std::ops::Range<usize> {
    const PAD: usize = SAMPLE_RATE * 2 / 5; // 400 ms
    const MIN_CUT: usize = SAMPLE_RATE / 2; // only bother when > 500 ms would be removed
    let levels = frame_levels(samples);
    let threshold = (stats.noise_floor_dbfs + 6.0).max(-60.0);
    let first = levels.iter().position(|&l| l > threshold);
    let last = levels.iter().rposition(|&l| l > threshold);
    let (Some(first), Some(last)) = (first, last) else {
        return 0..samples.len();
    };
    let mut start = (first * FRAME).saturating_sub(PAD);
    let mut end = ((last + 1) * FRAME + PAD).min(samples.len());
    if start < MIN_CUT {
        start = 0;
    }
    if samples.len() - end < MIN_CUT {
        end = samples.len();
    }
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: &mut u64) -> f32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        ((*seed >> 11) as f64 / (1u64 << 53) as f64) as f32 * 2.0 - 1.0
    }

    fn noise(n: usize, amp: f32, seed: u64) -> Vec<f32> {
        let mut s = seed;
        (0..n).map(|_| rng(&mut s) * amp).collect()
    }

    /// Syllable-like bursts: 200 ms tone-ish noise every 300 ms, on top of background noise.
    fn speechlike(n: usize, amp: f32, bg: f32) -> Vec<f32> {
        let mut v = noise(n, bg, 7);
        let mut s = 99;
        for (i, x) in v.iter_mut().enumerate() {
            let t = i as f32 / SAMPLE_RATE as f32;
            let in_burst = (i % (SAMPLE_RATE * 3 / 10)) < SAMPLE_RATE / 5 && t > 0.3;
            if in_burst {
                *x += amp * (2.0 * std::f32::consts::PI * 180.0 * t).sin() * 0.7 + rng(&mut s) * amp * 0.3;
            }
        }
        v
    }

    #[test]
    fn too_short() {
        let s = speechlike(SAMPLE_RATE / 5, 0.3, 0.001);
        assert_eq!(classify(&analyze(&s)), Verdict::TooShort);
    }

    #[test]
    fn digital_silence() {
        assert_eq!(classify(&analyze(&vec![0.0; SAMPLE_RATE * 2])), Verdict::Silence);
    }

    #[test]
    fn quiet_room() {
        assert_eq!(classify(&analyze(&noise(SAMPLE_RATE * 2, 0.002, 1))), Verdict::Silence);
    }

    #[test]
    fn steady_loud_noise_is_rejected() {
        assert_eq!(classify(&analyze(&noise(SAMPLE_RATE * 3, 0.2, 3))), Verdict::NoSpeech);
    }

    #[test]
    fn single_click_is_rejected() {
        let mut s = noise(SAMPLE_RATE * 2, 0.003, 5);
        for x in s.iter_mut().skip(SAMPLE_RATE).take(SAMPLE_RATE / 50) {
            *x += 0.8;
        }
        assert_eq!(classify(&analyze(&s)), Verdict::NoSpeech);
    }

    #[test]
    fn speech_passes_in_quiet_and_noisy_rooms() {
        assert_eq!(classify(&analyze(&speechlike(SAMPLE_RATE * 2, 0.2, 0.002))), Verdict::Speech);
        assert_eq!(classify(&analyze(&speechlike(SAMPLE_RATE * 2, 0.3, 0.03))), Verdict::Speech);
        // quiet speaker, low mic gain
        assert_eq!(classify(&analyze(&speechlike(SAMPLE_RATE * 2, 0.02, 0.0005))), Verdict::Speech);
    }

    #[test]
    fn trimming_keeps_padding() {
        let mut s = vec![0.0f32; SAMPLE_RATE * 3];
        s.extend(speechlike(SAMPLE_RATE * 2, 0.3, 0.0));
        s.extend(vec![0.0f32; SAMPLE_RATE * 3]);
        let st = analyze(&s);
        let r = trim_range(&s, &st);
        assert!(r.start > SAMPLE_RATE * 2 && r.start <= SAMPLE_RATE * 3);
        assert!(r.end < s.len() - SAMPLE_RATE * 2);
        // short recordings are not trimmed
        let s2 = speechlike(SAMPLE_RATE, 0.3, 0.0);
        let r2 = trim_range(&s2, &analyze(&s2));
        assert_eq!(r2, 0..s2.len());
    }
}
