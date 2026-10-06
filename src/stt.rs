//! Speech-to-text engine abstraction.
//!
//! WhistleType has two local engines behind one interface:
//! * FAST     - Cactus Compute Whistle on the Needle 3 engine (CPU, ~100 ms, 16.9 MB)
//! * ACCURATE - OpenAI Whisper on whisper.cpp (NVIDIA GPU via CUDA when available, CPU otherwise)
//!
//! Everything around the engines (hotkey, audio, speech gate, insertion) is shared and engine-agnostic.

use crate::engine::{EngineError, LanguagePlan, RawTranscript};
use crate::{log_info, segment};

/// Which engine a dictation should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    Whistle,
    Whisper,
}

/// Where an engine actually runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputeDevice {
    Cpu,
    /// GPU with its name, e.g. "NVIDIA GeForce RTX 3060 Laptop GPU".
    Gpu(String),
}

impl ComputeDevice {
    pub fn is_gpu(&self) -> bool {
        matches!(self, ComputeDevice::Gpu(_))
    }
    pub fn short(&self) -> &'static str {
        match self {
            ComputeDevice::Cpu => "CPU",
            ComputeDevice::Gpu(_) => "GPU",
        }
    }
}

pub trait SpeechEngine {
    /// Human-readable model name, e.g. "Whistle 2.0.0" or "Whisper large-v3-turbo".
    fn label(&self) -> String;
    fn device(&self) -> ComputeDevice;
    /// Longest audio (16 kHz samples) accepted by one `transcribe` call, or None when the engine handles any
    /// length itself.
    fn max_samples(&self) -> Option<usize>;
    /// Transcribes 16 kHz mono float audio. `language` None = detect. `vocabulary` = custom words to favour.
    fn transcribe(&mut self, pcm: &[f32], language: Option<&str>, vocabulary: &[String]) -> Result<RawTranscript, EngineError>;
}

/// Transcribes one piece of audio following `plan` (forced language, detection, or detection limited to an
/// accepted set with a fallback language). Returns the transcript and whether the fallback pass ran.
pub fn transcribe_planned(
    engine: &mut dyn SpeechEngine,
    pcm: &[f32],
    plan: &LanguagePlan,
    vocabulary: &[String],
) -> Result<(RawTranscript, bool), EngineError> {
    let first = engine.transcribe(pcm, plan.language.as_deref(), vocabulary)?;
    if plan.needs_fallback(&first.language) {
        let fb = plan.fallback.as_deref().unwrap_or("pl");
        let second = engine.transcribe(pcm, Some(fb), vocabulary)?;
        log_info!("engine: detected '{}' is not accepted, re-transcribed as '{fb}'", first.language);
        return Ok((second, true));
    }
    Ok((first, false))
}

/// Splits audio for engines with a per-call limit (Whistle: 30 s); returns one piece otherwise.
#[allow(clippy::single_range_in_vec_init)] // one range = the whole recording
pub fn split_for(engine: &dyn SpeechEngine, audio: &[f32]) -> Vec<std::ops::Range<usize>> {
    match engine.max_samples() {
        Some(max) if audio.len() > max => {
            let max = max.min(segment::MAX_SEGMENT);
            segment::split(audio, max, segment::SEARCH_WINDOW)
        }
        _ => vec![0..audio.len()],
    }
}
