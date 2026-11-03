//! Portable source-frozen generation request and status protocol.
use crate::timing::TempoMap;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Source {
    Audio { path: PathBuf },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub detection: crate::detection_settings::Settings,
    pub source: Source,
    pub bpm: f64,
    /// Frozen audio-analysis proposal. Omitted legacy jobs use constant BPM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo_map: Option<TempoMap>,
    pub title: String,
    pub artist: String,
    pub instruments: Vec<String>,
    /// Explicit supplied-audio mode. Missing/invalid stems must never trigger separation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_stems: Option<crate::stem_review::StemManifest>,
    #[serde(default)]
    pub guitar_source: crate::mapping::GuitarSource,
    #[serde(default)]
    pub keyboard_sound: crate::keyboard_detection::KeyboardSound,
    #[serde(default)]
    pub piano_quality: crate::keyboard_detection::PianoQuality,
    /// Omitted legacy jobs retain their English recognition policy.
    #[serde(default)]
    pub vocal_language: crate::lyric_language::Language,
    #[serde(
        default,
        skip_serializing_if = "crate::vocal_generation::AnalysisMode::is_standard"
    )]
    pub vocal_analysis_mode: crate::vocal_generation::AnalysisMode,
    pub model_dir: PathBuf,
    #[serde(default)]
    pub fetch_models: bool,
    #[serde(default)]
    pub full_mix_drums: bool,
    #[serde(default)]
    pub standard_drums: bool,
    #[serde(default)]
    pub lyrics: bool,
    #[serde(default)]
    pub song_info: Option<crate::song_import::SongInfo>,
}

impl Request {
    pub fn validate_existing_stems(&self) -> Result<()> {
        let Some(manifest) = &self.existing_stems else {
            return Ok(());
        };
        ensure!(
            self.instruments.len() == 1,
            "Choose one part when using existing stems"
        );
        ensure!(
            !self.full_mix_drums,
            "Choose existing stems or the full recording"
        );
        ensure!(
            self.keyboard_sound != crate::keyboard_detection::KeyboardSound::Synth
                || !self
                    .instruments
                    .iter()
                    .any(|part| matches!(part.as_str(), "piano" | "pro_piano")),
            "Experimental Synth analyzes the full recording. Choose Piano to use existing stems."
        );
        let part = self
            .instruments
            .first()
            .map(String::as_str)
            .unwrap_or_default();
        let required = match part {
            "guitar" => self.guitar_source.stem().as_str(),
            "pro_piano" => "piano",
            other => other,
        };
        ensure!(
            manifest
                .stems
                .iter()
                .any(|stem| stem.id.as_str() == required),
            "No attached {required} stem. Attach matching stems or generate stems first."
        );
        Ok(())
    }

    pub fn timing(&self) -> Result<TempoMap> {
        let fallback = TempoMap::constant(self.bpm)?;
        let Some(map) = &self.tempo_map else {
            return Ok(fallback);
        };
        let canonical = TempoMap::new(map.resolution, map.offset, map.tempos.clone())?;
        ensure!(
            canonical == *map,
            "Tempo map must be ordered and start at tick zero"
        );
        ensure!(
            (map.tempos[0].bpm - self.bpm).abs() < 0.001,
            "Starting BPM differs from the tempo map"
        );
        Ok(canonical)
    }

    pub fn validate(&self) -> Result<()> {
        self.detection.validate()?;
        self.timing()?;
        self.validate_existing_stems()?;
        ensure!(
            !self.title.trim().is_empty() && !self.artist.trim().is_empty(),
            "Enter the song title and artist"
        );
        ensure!(
            !self.title.contains(['\n', '\r']) && !self.artist.contains(['\n', '\r']),
            "Title and artist must each be one line"
        );
        ensure!(
            !self.instruments.is_empty()
                && self.instruments.iter().all(|s| [
                    "guitar",
                    "bass",
                    "drums",
                    "vocals",
                    "piano",
                    "pro_piano"
                ]
                .contains(&s.as_str())),
            "Select guitar, bass, drums, Piano (Keys), Pro Piano (Pro Keys) and/or vocals"
        );
        ensure!(
            !self.full_mix_drums || self.instruments.iter().all(|s| s == "drums"),
            "Full-mix transcription supports drums only"
        );
        if !cfg!(any(feature = "separation", target_arch = "wasm32")) {
            ensure!(
                self.existing_stems.is_some()
                    || self.full_mix_drums
                    || (crate::keyboard_detection::high_quality_piano(
                        &self.instruments,
                        self.keyboard_sound,
                        self.piano_quality
                    ) && self
                        .instruments
                        .iter()
                        .all(|s| matches!(s.as_str(), "piano" | "pro_piano"))),
                "This build has no standard separator. Use a full build, higher-quality Piano only, or drums-only from the full mix."
            );
        }
        match &self.source {
            Source::Audio { path } => {
                ensure!(crate::storage::is_file(path), "Choose an audio file");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Running,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
    pub version: u32,
    pub state: State,
    pub stage: String,
    pub detail: String,
    pub request_sha256: String,
    pub error: Option<String>,
}
