//! Saved recognition evidence and cancellation state do not require a model.
use crate::clean::WordTimings;
use serde::{Deserialize, Serialize};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
pub const RECOGNITION_WARNING: &str = "Automatic English lyric recognition and word timing are estimates. Singing, backing vocals and instruments can cause missed or incorrect words and boundaries. Listen to each edit; zero matches does not establish that a recording is clean.";
#[derive(Default)]
pub struct Control {
    cancelled: AtomicBool,
    message: Mutex<String>,
}
impl Control {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
    pub fn message(&self) -> String {
        self.message.lock().map(|m| m.clone()).unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn update(&self, message: impl Into<String>) {
        let message = message.into();
        crate::generation_progress::detail(&message);
        if let Ok(mut current) = self.message.lock() {
            *current = message;
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.is_cancelled(), "Automatic lyric analysis cancelled");
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecognizedSegment {
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub mean_token_probability: f32,
    pub no_speech_probability: f32,
}
pub const MIN_WORD_PROBABILITY: f32 = 0.30;
/// Retain uncertain text without inventing word boundaries from sentence times.
pub fn aligned_words(
    segments: &[RecognizedSegment],
    duration: f64,
) -> Vec<crate::clean::AlignedWord> {
    let mut words = Vec::new();
    let mut previous_start = 0.;
    for segment in segments {
        let text = segment.text.trim();
        if text.starts_with('[') || text.starts_with('(') || text.starts_with("<|") {
            continue;
        }
        let lexical = crate::clean::tokens(text);
        let start = segment.start_seconds;
        let end = segment.end_seconds.min(duration);
        let confident = segment.mean_token_probability.is_finite()
            && segment.mean_token_probability >= MIN_WORD_PROBABILITY
            && segment.no_speech_probability.is_finite()
            && segment.no_speech_probability < 0.6;
        let timed = confident
            && lexical.len() == 1
            && start.is_finite()
            && end.is_finite()
            && start >= previous_start
            && start >= 0.
            && start < end
            && start < duration;
        if timed {
            previous_start = start;
        }
        for word in lexical {
            words.push(crate::clean::AlignedWord {
                text: Some(word),
                start: timed.then_some(start),
                end: timed.then_some(end),
            });
        }
    }
    words
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recognition {
    pub version: u32,
    pub backend: String,
    pub model_sha256: String,
    pub recognition_audio_sha256: String,
    pub language: String,
    pub words: WordTimings,
    pub segments: Vec<RecognizedSegment>,
    pub warnings: Vec<String>,
}
