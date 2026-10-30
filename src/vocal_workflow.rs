//! Create and save vocal proposals for the desktop app and command line.
use crate::{
    clean, clean_recognition, models, pipeline,
    stem_review::{StemId, StemManifest},
    timing::TempoMap,
    vocal_generation::{self, Generation, LyricSource, Policy},
};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

fn optional_vocal_pitch_model(
    directory: &Path,
    enabled: bool,
    download: bool,
) -> Result<(Option<PathBuf>, Option<String>)> {
    if !enabled {
        return Ok((None, None));
    }
    if models::verified(directory, "vocal-pitch").is_err() && download {
        models::fetch_required(directory, &["vocal-pitch"])?;
    }
    match models::verified(directory, "vocal-pitch") {
        Ok(model) => Ok((Some(model), None)),
        Err(error) => Ok((
            None,
            Some(format!(
                "Optional vocal contour model unavailable: {error}. Basic Pitch assistance remains active; additional vocal gaps were not checked with SwiftF0. Enable model downloads or install the pinned vocal-pitch asset."
            )),
        )),
    }
}

pub struct ProjectRequest<'a> {
    pub project: &'a Path,
    pub model_dir: &'a Path,
    pub recognize: bool,
    pub language: crate::lyric_language::Language,
    pub neural_pitch: bool,
    pub download_model: bool,
    pub lookup_lyrics: bool,
    pub actor: crate::editor::Actor,
}
#[derive(serde::Serialize)]
pub struct ProjectResult {
    pub output: PathBuf,
    pub applied: bool,
    pub status: vocal_generation::Status,
}

/// Proposals survive a revision conflict. A new vocal document is applied only
/// while the captured source and prior vocal document remain current.
pub fn generate_project(
    request: ProjectRequest<'_>,
    control: Arc<clean_recognition::Control>,
) -> Result<ProjectResult> {
    let store = crate::editor::Store::new(request.project);
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(request.project.join("vocal-worker.lock"))?;
    lock.try_lock()
        .context("Vocal generation is already running for this project")?;
    let original = store.load()?;
    let manifest = original
        .content
        .stems
        .as_ref()
        .context("Attach isolated stems before generating vocals")?;
    let audio = store.audio_path(&original)?;
    let timing = original.timing(&store)?;
    let parent = request.project.join("vocal-proposals");
    fs::create_dir_all(&parent)?;
    let job = tempfile::Builder::new()
        .prefix("draft-")
        .tempdir_in(&parent)?
        .keep();
    crate::generation::write_json(&job.join("isolation.json"), manifest)?;
    let lyrics = if request.lookup_lyrics {
        let existing = request.project.join("source/chart-engine-lyrics.json");
        let record = if existing.exists() {
            Some(serde_json::from_slice::<serde_json::Value>(&fs::read(
                existing,
            )?)?)
        } else {
            let fields =
                pipeline::ini_fields(&pipeline::song_ini(&request.project.join("source"))?);
            let mut meta = pipeline::metadata(&audio)?;
            for (key, field) in [
                ("name", &mut meta.name),
                ("artist", &mut meta.artist),
                ("album", &mut meta.album),
            ] {
                if let Some(value) = fields.get(key).filter(|s| !s.trim().is_empty()) {
                    *field = value.clone();
                }
            }
            // A provider outage does not prevent local recognition/pitch work.
            pipeline::lyrics(&meta).unwrap_or_default()
        };
        record.and_then(|value| {
            value["plainLyrics"]
                .as_str()
                .filter(|text| !text.trim().is_empty())
                .map(|text| LyricSource {
                    text: text.into(),
                    synced_lyrics: value["syncedLyrics"].as_str().map(str::to_owned),
                    source: value["id"].as_u64().map_or_else(
                        || "Saved matched lyrics".into(),
                        |id| format!("https://lrclib.net/api/get/{id}"),
                    ),
                })
        })
    } else {
        None
    };
    let output = job.join("proposal");
    let result = generate(
        Request {
            audio: &audio,
            manifest: &job.join("isolation.json"),
            timing: &timing,
            words: None,
            lyrics: lyrics.as_ref(),
            recognize: request.recognize,
            language: request.language,
            neural_pitch: request.neural_pitch,
            model_dir: request.model_dir,
            download_model: request.download_model,
        },
        &output,
        control.clone(),
    )?;
    ensure!(
        !control.is_cancelled(),
        "Vocal generation cancelled; proposals retained at {}",
        output.display()
    );
    let current = store.load()?;
    let applied = if let Some(document) = &result.chart {
        if current.audio_sha256 == original.audio_sha256
            && current.content.vocals == original.content.vocals
        {
            store.apply(crate::editor::Request {
                expected_revision: current.revision,
                actor: request.actor,
                action: crate::editor::Action::SetVocalGeneration {
                    document: document.clone(),
                    review: crate::vocal_review::VocalReviewMap::from_generation(&result)?,
                },
            })?;
            true
        } else {
            false
        }
    } else {
        false
    };
    crate::generation::write_json(&request.project.join("vocal-generation.json"), &result)?;
    Ok(ProjectResult {
        output,
        applied,
        status: result.status,
    })
}

pub struct Request<'a> {
    pub audio: &'a Path,
    pub manifest: &'a Path,
    pub timing: &'a TempoMap,
    pub words: Option<&'a clean::WordTimings>,
    pub lyrics: Option<&'a LyricSource>,
    /// Explicit opt-in. Model-free operation never loads Whisper.
    pub recognize: bool,
    pub language: crate::lyric_language::Language,
    /// Explicit local model vote, independent of word recognition. False keeps
    /// supplied-word generation completely model-free.
    pub neural_pitch: bool,
    pub model_dir: &'a Path,
    pub download_model: bool,
}

pub fn generate(
    request: Request<'_>,
    output: &Path,
    control: Arc<clean_recognition::Control>,
) -> Result<Generation> {
    pipeline::destination(output, None)?;
    ensure!(
        !(request.recognize && request.words.is_some()),
        "Choose supplied word timings or recognition"
    );
    let manifest = StemManifest::read(request.manifest)?;
    let duration = manifest.frames as f64 / manifest.sample_rate as f64;
    let audio_hash = models::hash_file(request.audio)?;
    manifest.validate_binding(&audio_hash, duration)?;
    let stem = manifest
        .stems
        .iter()
        .find(|s| s.id == StemId::Vocals)
        .context("Attach an aligned vocal stem first")?;
    let alignment = clean::StemAlignment {
        audio_sha256: audio_hash,
        stem_sha256: stem.sha256.clone(),
        evidence: format!(
            "{}: {}",
            manifest.alignment.method, manifest.alignment.details
        ),
    };
    let stage = tempfile::tempdir_in(
        output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    #[cfg(feature = "vocal-alignment")]
    let mut alignment_warning = None;
    #[cfg(not(feature = "vocal-alignment"))]
    let alignment_warning: Option<String> = None;
    #[cfg(feature = "vocal-alignment")]
    let acoustic = if request.recognize
        && let Some(lyrics) = request.lyrics
        && request.language.use_english_alignment(&lyrics.text)
    {
        match crate::vocal_alignment::align(
            crate::vocal_alignment::Request {
                audio: request.audio,
                vocal_stem: &stem.path,
                alignment: &alignment,
                lyrics,
                model_dir: request.model_dir,
                download_model: request.download_model,
            },
            control.clone(),
        ) {
            Ok(result) => {
                let usable = result
                    .words
                    .words
                    .iter()
                    .any(|word| word.start.is_some() && word.end.is_some());
                fs::write(
                    stage.path().join("alignment.json"),
                    serde_json::to_vec_pretty(&result)?,
                )?;
                if usable {
                    Some(result)
                } else {
                    alignment_warning = Some("Acoustic alignment found no supported word boundaries; local recognition was used instead.".to_owned());
                    None
                }
            }
            Err(error) => {
                ensure!(!control.is_cancelled(), "Vocal generation cancelled");
                alignment_warning = Some(format!(
                    "Acoustic lyric alignment was unavailable; recognition fallback: {error:#}"
                ));
                None
            }
        }
    } else {
        None
    };
    #[cfg(feature = "vocal-alignment")]
    let acoustic_words = acoustic.as_ref().map(|result| &result.words);
    #[cfg(not(feature = "vocal-alignment"))]
    let acoustic_words: Option<&clean::WordTimings> = None;
    let recognition = if request.recognize && acoustic_words.is_none() {
        Some(
            clean_recognition::detect_language(
                clean_recognition::Request {
                    audio: request.audio,
                    model_dir: request.model_dir,
                    download_model: request.download_model,
                    vocal_stem: Some((&stem.path, &alignment)),
                },
                &clean::CleanPolicy::default(),
                clean::CleanMode::FullMix,
                control.clone(),
                request.language,
            )?
            .recognition,
        )
    } else {
        None
    };
    ensure!(!control.is_cancelled(), "Vocal generation cancelled");
    let words = request
        .words
        .or(acoustic_words)
        .or_else(|| recognition.as_ref().map(|r| &r.words));
    let neural_pitch = if request.neural_pitch {
        control.update("Checking vocal pitch with the local note model…");
        if models::verified(request.model_dir, "pitch").is_err() && request.download_model {
            models::fetch_required(request.model_dir, &["pitch"])?;
        }
        let model = models::verified(request.model_dir, "pitch")?;
        let notes = models::pitch(&stem.path, &model)?;
        ensure!(!control.is_cancelled(), "Vocal pitch analysis cancelled");
        let model = models::verified(request.model_dir, "pitch")?;
        ensure!(
            models::hash_file(request.audio)? == alignment.audio_sha256
                && models::hash_file(&stem.path)? == alignment.stem_sha256,
            "Vocal sources changed during local neural pitch inference"
        );
        Some(vocal_generation::NeuralPitchEvidence {
            audio_sha256: alignment.audio_sha256.clone(),
            vocal_stem_sha256: alignment.stem_sha256.clone(),
            duration_seconds: duration,
            model_sha256: models::hash_file(&model)?,
            implementation_sha256: vocal_generation::neural_pitch_implementation_hash(),
            notes,
        })
    } else {
        None
    };
    let (vocal_model, vocal_pitch_warning) = optional_vocal_pitch_model(
        request.model_dir,
        request.neural_pitch,
        request.download_model,
    )?;
    let vocal_pitch = if let Some(model) = vocal_model {
        control.update("Checking the local vocal contour…");
        Some(crate::vocal_pitch::analyze(
            request.audio,
            &stem.path,
            &alignment,
            &model,
            &|| control.is_cancelled(),
        )?)
    } else {
        None
    };
    let mut result = vocal_generation::generate_cancellable(
        vocal_generation::Request {
            audio: request.audio,
            vocal_stem: &stem.path,
            alignment: &alignment,
            timing: request.timing,
            words,
            lyrics: request.lyrics,
            policy: &Policy::default(),
            neural_pitch: neural_pitch.as_ref(),
            vocal_pitch: vocal_pitch.as_ref(),
        },
        &|| control.is_cancelled(),
    )?;
    if request.recognize && request.language != crate::lyric_language::Language::English {
        result.warnings.push("Multilingual recognition supplies word estimates without the English-only acoustic aligner. Spanish singing, rapid syllables, rap and language changes still need word/pitch review; uncertain pitch is never automatically converted to unpitched rap.".into());
    }
    if let Some(warning) = alignment_warning {
        result.warnings.push(warning);
    }
    if let Some(warning) = vocal_pitch_warning {
        result.warnings.push(warning);
    }
    #[cfg(feature = "vocal-alignment")]
    if let Some(alignment) = &acoustic {
        result.warnings.extend(alignment.warnings.iter().cloned());
    }
    ensure!(!control.is_cancelled(), "Vocal generation cancelled");
    fs::write(
        stage.path().join("generation.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    if let Some(chart) = &result.chart {
        fs::write(
            stage.path().join("vocals.json"),
            serde_json::to_vec_pretty(chart)?,
        )?;
        // Same tempo map as the working song, never a guessed 120 BPM export.
        if request.timing.offset == 0.0 {
            let original = crate::midi::new_midi(request.timing, &Default::default())?;
            fs::write(
                stage.path().join("vocals.mid"),
                crate::vocals::append_midi(&original, chart)?,
            )?;
        } else {
            fs::write(
                stage.path().join("midi-export.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "status":"song_export_required","offset_seconds":request.timing.offset,
                    "reason":"Export the vocal document with the song through the editor to preserve its audio offset."
                }))?,
            )?;
        }
    }
    if let Some(recognition) = recognition {
        fs::write(
            stage.path().join("recognition.json"),
            serde_json::to_vec_pretty(&recognition)?,
        )?;
    }
    fs::write(
        stage.path().join("recognition-language.json"),
        serde_json::to_vec(&request.language)?,
    )?;
    pipeline::publish(stage.path(), output)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_free_selection_never_requires_models_or_downloads() {
        let absent = Path::new("/path/that/does/not/exist/yace-no-models");
        let (model, warning) = optional_vocal_pitch_model(absent, false, true).unwrap();
        assert!(model.is_none() && warning.is_none());
    }

    #[test]
    fn missing_or_invalid_optional_model_preserves_explicit_basic_pitch_fallback() {
        let directory = tempfile::tempdir().unwrap();
        for invalid_file in [false, true] {
            if invalid_file {
                fs::write(
                    directory.path().join("swift-f0-v0.3.0.onnx"),
                    b"wrong weights",
                )
                .unwrap();
            }
            let (model, warning) =
                optional_vocal_pitch_model(directory.path(), true, false).unwrap();
            assert!(model.is_none());
            let warning = warning.unwrap();
            assert!(warning.contains("Basic Pitch assistance remains active"));
            assert!(warning.contains("not checked with SwiftF0"));
        }
    }
}
