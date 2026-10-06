//! Splits recordings longer than Whistle's 30 s limit into segments, cutting at the quietest point.

use std::ops::Range;

pub const SAMPLE_RATE: usize = 16_000;
/// Whistle accepts at most 480,000 samples (30 s). Keep a safety margin.
pub const MAX_SEGMENT: usize = SAMPLE_RATE * 59 / 2; // 29.5 s
/// How far back from the limit we look for a pause.
pub const SEARCH_WINDOW: usize = SAMPLE_RATE * 10;

const WIN: usize = SAMPLE_RATE / 5; // 200 ms analysis window
const HOP: usize = SAMPLE_RATE / 50; // 20 ms hop

/// Returns ranges covering `0..len` in order, each at most `max` samples long.
pub fn split(samples: &[f32], max: usize, search: usize) -> Vec<Range<usize>> {
    assert!(max > WIN * 2 && search < max);
    let mut out = Vec::new();
    let mut start = 0;
    while samples.len() - start > max {
        let lo = start + max - search;
        let hi = start + max; // the cut may be at most here
        let cut = quietest_point(samples, lo, hi).unwrap_or(hi);
        out.push(start..cut);
        start = cut;
    }
    if start < samples.len() || out.is_empty() {
        out.push(start..samples.len());
    }
    out
}

/// Centre of the 200 ms window with the least energy inside `lo..hi`.
fn quietest_point(samples: &[f32], lo: usize, hi: usize) -> Option<usize> {
    if hi < lo + WIN {
        return None;
    }
    let sq: Vec<f64> = samples[lo..hi].iter().map(|&x| (x as f64) * (x as f64)).collect();
    // prefix sums for O(1) window energy
    let mut prefix = Vec::with_capacity(sq.len() + 1);
    prefix.push(0.0);
    for v in &sq {
        prefix.push(prefix.last().unwrap() + v);
    }
    let mut best: Option<(f64, usize)> = None;
    let mut pos = 0;
    while pos + WIN <= sq.len() {
        let e = prefix[pos + WIN] - prefix[pos];
        // prefer later cuts on ties so segments stay long
        if best.map_or(true, |(b, _)| e <= b) {
            best = Some((e, pos));
        }
        pos += HOP;
    }
    best.map(|(_, p)| lo + p + WIN / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_audio_is_one_segment() {
        let s = vec![0.1f32; SAMPLE_RATE * 5];
        assert_eq!(split(&s, MAX_SEGMENT, SEARCH_WINDOW), vec![0..s.len()]);
        assert_eq!(split(&[], MAX_SEGMENT, SEARCH_WINDOW), vec![0..0]);
    }

    #[test]
    fn exact_limit_is_one_segment() {
        let s = vec![0.1f32; MAX_SEGMENT];
        assert_eq!(split(&s, MAX_SEGMENT, SEARCH_WINDOW).len(), 1);
    }

    #[test]
    fn cuts_in_the_pause() {
        // 70 s of "speech" with a pause at 25 s and at 50 s
        let mut s = vec![0.3f32; SAMPLE_RATE * 70];
        for x in &mut s[SAMPLE_RATE * 25..SAMPLE_RATE * 25 + SAMPLE_RATE / 2] {
            *x = 0.0;
        }
        for x in &mut s[SAMPLE_RATE * 50..SAMPLE_RATE * 50 + SAMPLE_RATE / 2] {
            *x = 0.0;
        }
        let segs = split(&s, MAX_SEGMENT, SEARCH_WINDOW);
        assert_eq!(segs.len(), 3);
        for r in &segs {
            assert!(r.len() <= MAX_SEGMENT, "segment too long: {}", r.len());
        }
        let first_cut = segs[0].end as f32 / SAMPLE_RATE as f32;
        assert!((25.0..25.5).contains(&first_cut), "cut at {first_cut}");
        assert_eq!(segs.last().unwrap().end, s.len());
        // contiguous
        for w in segs.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
    }

    #[test]
    fn no_pause_still_respects_limit() {
        let s: Vec<f32> = (0..SAMPLE_RATE * 95).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
        let segs = split(&s, MAX_SEGMENT, SEARCH_WINDOW);
        assert!(segs.len() >= 4);
        assert!(segs.iter().all(|r| r.len() <= MAX_SEGMENT && !r.is_empty()));
        assert_eq!(segs.iter().map(|r| r.len()).sum::<usize>(), s.len());
    }
}
