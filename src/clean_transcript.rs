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
