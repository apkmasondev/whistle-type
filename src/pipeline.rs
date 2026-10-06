//! Audio → text pipeline shared by the app and the benchmark tool:
//! sanitize → speech gate → trim → (engine; Whistle splits into ≤ 29.5 s pieces) → join → safe post-processing.

use crate::engine::RawTranscript;
use crate::text;
use crate::vad::{self, SpeechStats, Verdict};

pub struct Prepared {
    pub stats: SpeechStats,
    pub verdict: Verdict,
    /// The trimmed recording; empty unless `verdict == Speech`.
    pub audio: Vec<f32>,
}

/// Replaces NaN/Inf and clamps to [-1, 1] (the engine expects float PCM in that range).
pub fn sanitize(samples: &mut [f32]) {
    for x in samples.iter_mut() {
        *x = if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 };
    }
}

pub fn prepare(mut samples: Vec<f32>) -> Prepared {
    sanitize(&mut samples);
    let stats = vad::analyze(&samples);
    let verdict = vad::classify(&stats);
    if verdict != Verdict::Speech {
        return Prepared { stats, verdict, audio: Vec::new() };
    }
    let range = vad::trim_range(&samples, &stats);
    let audio = if range == (0..samples.len()) { samples } else { samples[range].to_vec() };
    Prepared { stats, verdict, audio }
}

pub struct Finished {
    /// Text to insert; empty when nothing was recognised.
    pub text: String,
    pub language: String,
}

pub fn finish(parts: &[RawTranscript], raw: bool, append_space: bool) -> Finished {
    let texts: Vec<&str> = parts.iter().map(|p| p.text.as_str()).collect();
    let joined = if parts.len() == 1 { parts[0].text.clone() } else { text::join_segments(&texts) };
    let language = parts.iter().find(|p| !p.language.is_empty()).map(|p| p.language.clone()).unwrap_or_default();
    if !text::has_content(&joined) {
        return Finished { text: String::new(), language };
    }
    Finished { text: text::finalize(&joined, raw, append_space), language }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_nan_and_clamps() {
        let mut v = vec![f32::NAN, f32::INFINITY, 2.0, -3.0, 0.5];
        sanitize(&mut v);
        assert_eq!(v, vec![0.0, 0.0, 1.0, -1.0, 0.5]);
    }

    #[test]
    fn silence_produces_no_segments() {
        let p = prepare(vec![0.0; 32_000]);
        assert_eq!(p.verdict, Verdict::Silence);
        assert!(p.audio.is_empty());
    }

    #[test]
    fn long_audio_is_split_below_whistle_limit() {
        let samples: Vec<f32> = (0..16_000 * 75)
            .map(|i| if (i / 4000) % 3 == 0 { 0.001 } else { ((i as f32) * 0.07).sin() * 0.3 })
            .collect();
        let p = prepare(samples);
        assert_eq!(p.verdict, Verdict::Speech);
        let pieces = crate::segment::split(&p.audio, crate::segment::MAX_SEGMENT, crate::segment::SEARCH_WINDOW);
        assert!(pieces.len() >= 3);
        assert!(pieces.iter().all(|r| r.len() <= crate::engine::MAX_SAMPLES));
    }

    #[test]
    fn finish_rules() {
        let mk = |t: &str| RawTranscript { text: t.into(), language: "pl".into(), ..Default::default() };
        assert_eq!(finish(&[mk("  To jest  test. ")], false, false).text, "To jest test.");
        assert_eq!(finish(&[mk("  To jest  test. ")], true, false).text, "  To jest  test. ");
        assert_eq!(finish(&[mk(" . ")], false, false).text, "");
        assert_eq!(finish(&[mk("")], false, false).text, "");
        assert_eq!(finish(&[mk("Raz."), mk("Dwa.")], false, false).text, "Raz. Dwa.");
    }
}
