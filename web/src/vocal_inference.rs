//! Browser vocal models feed the same source-bound Rust vocal generator.
use crate::{
    models,
    onnx::{Input, Runtime},
};
use anyhow::{Context, Result, ensure};
use std::path::Path;
pub async fn generate(
    audio: &Path,
    manifest: &crate::stem_review::StemManifest,
    timing: &crate::timing::TempoMap,
    progress: &mut impl FnMut(String),
) -> Result<crate::vocal_generation::Generation> {
    generate_language(
        audio,
        manifest,
        timing,
        Default::default(),
        Default::default(),
        None,
        None,
        progress,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub async fn generate_language(
    audio: &Path,
    manifest: &crate::stem_review::StemManifest,
    timing: &crate::timing::TempoMap,
    language: crate::lyric_language::Language,
    analysis_mode: crate::vocal_generation::AnalysisMode,
    evidence_output: Option<&Path>,
    lyrics: Option<&crate::vocal_generation::LyricSource>,
    progress: &mut impl FnMut(String),
) -> Result<crate::vocal_generation::Generation> {
    generate_language_configured(
        audio,
        manifest,
        timing,
        language,
        analysis_mode,
        evidence_output,
        lyrics,
        false,
        Default::default(),
        progress,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub async fn generate_language_configured(
    audio: &Path,
    manifest: &crate::stem_review::StemManifest,
    timing: &crate::timing::TempoMap,
    language: crate::lyric_language::Language,
    analysis_mode: crate::vocal_generation::AnalysisMode,
    evidence_output: Option<&Path>,
    lyrics: Option<&crate::vocal_generation::LyricSource>,
    recognize_lyrics: bool,
    settings: crate::detection_settings::Settings,
    progress: &mut impl FnMut(String),
) -> Result<crate::vocal_generation::Generation> {
    settings.validate()?;
    let stem = manifest
        .stems
        .iter()
        .find(|s| s.id == crate::stem_review::StemId::Vocals)
        .context("No vocal stem")?;
    manifest.validate_binding(&models::hash_file(audio)?, manifest.frames as f64 / 44100.)?;
    ensure!(
        models::hash_file(&stem.path)? == stem.sha256,
        "Vocal stem changed"
    );
    let binding = crate::clean::StemAlignment {
        audio_sha256: manifest.source_audio_sha256.clone(),
        stem_sha256: stem.sha256.clone(),
        evidence: manifest.alignment.details.clone(),
    };
    let mut alignment_warning = None;
    let acoustic = if recognize_lyrics
        && let Some(lyrics) = lyrics.filter(|lyrics| language.use_english_alignment(&lyrics.text))
    {
        match crate::acoustic_alignment::align(audio, &stem.path, &binding, lyrics, progress).await
        {
            Ok(result) => {
                if let Some(output) = evidence_output {
                    crate::storage::write(
                        output.with_file_name("alignment.json"),
                        serde_json::to_vec_pretty(&result)?,
                    )?;
                }
                if result
                    .words
                    .words
                    .iter()
                    .any(|word| word.start.is_some() && word.end.is_some())
                {
                    Some(result)
                } else {
                    alignment_warning=Some("Acoustic alignment found no supported word boundaries; local recognition was used instead.".to_owned());
                    progress(
                        "Acoustic alignment found no supported words; using local recognition"
                            .into(),
                    );
                    None
                }
            }
            Err(error) => {
                alignment_warning = Some(format!(
                    "Acoustic lyric alignment was unavailable; recognition fallback: {error:#}"
                ));
                progress(format!(
                    "Acoustic alignment unavailable; local recognition fallback: {error:#}"
                ));
                None
            }
        }
    } else {
        None
    };
    let mut recognition_warning = None;
    let recognized = if recognize_lyrics && acoustic.is_none() {
        match crate::recognition::analyze(audio, Some((&stem.path, &binding)), language, progress)
            .await
        {
            Ok(result) => Some(result),
            Err(error) => {
                recognition_warning = Some(format!(
                    "Optional lyric recognition unavailable: {error:#}. Acoustic pitch generation continued; lyrics remain unknown."
                ));
                None
            }
        }
    } else {
        None
    };
    let words = acoustic
        .as_ref()
        .map(|result| &result.words)
        .or_else(|| recognized.as_ref().map(|result| &result.words));
    if let (Some(output), Some(recognized)) = (evidence_output, &recognized) {
        crate::storage::write(output, serde_json::to_vec_pretty(recognized)?)?;
    }
    let neural = if analysis_mode == crate::vocal_generation::AnalysisMode::Standard {
        let candidates = crate::transcription::pitch(&stem.path, progress).await?;
        Some(crate::vocal_generation::NeuralPitchEvidence {
            duration_seconds: manifest.frames as f64 / 44100.,
            model_sha256: models::assets()
                .into_iter()
                .find(|a| a.id == "pitch")
                .unwrap()
                .sha256,
            audio_sha256: manifest.source_audio_sha256.clone(),
            vocal_stem_sha256: stem.sha256.clone(),
            implementation_sha256: crate::vocal_generation::neural_pitch_implementation_hash(),
            notes: candidates.into_iter().map(|e| e.candidate).collect(),
        })
    } else {
        None
    };
    let evidence = if analysis_mode == crate::vocal_generation::AnalysisMode::Standard {
        let mut e = swift(&models::decode(&stem.path, 16000, 1)?, progress).await?;
        e.audio_sha256 = manifest.source_audio_sha256.clone();
        e.vocal_stem_sha256 = stem.sha256.clone();
        e.duration_seconds = manifest.frames as f64 / 44100.;
        Some(e)
    } else {
        None
    };
    let rmvpe_pitch = if analysis_mode == crate::vocal_generation::AnalysisMode::HighAccuracy {
        let mut e = rmvpe(&models::decode(&stem.path, 16000, 1)?, progress).await?;
        e.audio_sha256 = binding.audio_sha256.clone();
        e.vocal_stem_sha256 = binding.stem_sha256.clone();
        e.duration_seconds = manifest.frames as f64 / 44100.;
        Some(e)
    } else {
        None
    };
    progress("Assembling source-bound vocal pitches and review phrases".into());
    let last = std::cell::Cell::new(js_sys::Date::now());
    let progress = std::cell::RefCell::new(progress);
    let mut result = crate::vocal_generation::generate_cancellable(
        crate::vocal_generation::Request {
            audio,
            vocal_stem: &stem.path,
            alignment: &crate::clean::StemAlignment {
                audio_sha256: manifest.source_audio_sha256.clone(),
                stem_sha256: stem.sha256.clone(),
                evidence: manifest.alignment.details.clone(),
            },
            timing,
            words,
            lyrics,
            policy: &settings.vocals(analysis_mode),
            neural_pitch: neural.as_ref(),
            vocal_pitch: evidence.as_ref(),
            rmvpe_pitch: rmvpe_pitch.as_ref(),
        },
        &|| {
            let now = js_sys::Date::now();
            if now - last.get() > 1000. {
                last.set(now);
                progress.borrow_mut()("Assembling vocal pitch and phrase evidence locally…".into());
            }
            false
        },
    )?;
    if let Some(warning) = recognition_warning {
        result.warnings.push(warning);
    }
    if language != crate::lyric_language::Language::English {
        result.warnings.push("Multilingual recognition supplies word estimates without the English-only acoustic aligner. Spanish singing, rapid syllables, rap and language changes still need word/pitch review; uncertain pitch is never automatically converted to unpitched rap.".into());
    }
    if let Some(warning) = alignment_warning {
        result.warnings.push(warning);
    }
    if let Some(alignment) = acoustic {
        result.warnings.extend(alignment.warnings);
    }
    Ok(result)
}

pub async fn pitch_comparison(
    audio: &Path,
    manifest: &crate::stem_review::StemManifest,
    timing: &crate::timing::TempoMap,
    progress: &mut impl FnMut(String),
) -> Result<crate::vocal_generation::Generation> {
    let stem = manifest
        .stems
        .iter()
        .find(|s| s.id == crate::stem_review::StemId::Vocals)
        .context("Attach a vocal stem before comparing pitches")?;
    manifest.validate_binding(&models::hash_file(audio)?, models::duration(audio)?)?;
    ensure!(
        models::hash_file(&stem.path)? == stem.sha256,
        "Vocal stem changed"
    );
    let binding = crate::clean::StemAlignment {
        audio_sha256: manifest.source_audio_sha256.clone(),
        stem_sha256: stem.sha256.clone(),
        evidence: manifest.alignment.details.clone(),
    };
    let mut e = rmvpe(&models::decode(&stem.path, 16000, 1)?, progress).await?;
    e.audio_sha256 = binding.audio_sha256.clone();
    e.vocal_stem_sha256 = binding.stem_sha256.clone();
    e.duration_seconds = manifest.frames as f64 / 44100.;
    progress("Comparing stable note centres with the recording…".into());
    crate::vocal_generation::generate(crate::vocal_generation::Request {
        audio,
        vocal_stem: &stem.path,
        alignment: &binding,
        timing,
        words: None,
        lyrics: None,
        policy: &crate::vocal_generation::Policy {
            analysis_mode: crate::vocal_generation::AnalysisMode::HighAccuracy,
            ..Default::default()
        },
        neural_pitch: None,
        vocal_pitch: None,
        rmvpe_pitch: Some(&e),
    })
}

async fn rmvpe(
    pcm: &[f32],
    progress: &mut impl FnMut(String),
) -> Result<crate::vocal_pitch::Evidence> {
    ensure!(
        pcm.len() > 512 && pcm.len() <= 16000 * 3600,
        "High accuracy requires finite audio between 33ms and one hour"
    );
    let model = Runtime::load("vocal-rmvpe", progress, false).await?;
    let count = pcm.len() / 160 + 1;
    let mut frames = Vec::with_capacity(count);
    for start in (0..count).step_by(3000) {
        let end = (start + 3000).min(count);
        progress(format!(
            "Comparing vocal pitch · section {} of {}",
            start / 3000 + 1,
            count.div_ceil(3000)
        ));
        let input = crate::rmvpe_frontend::input(pcm, start, end)?;
        let outputs = model
            .run_inputs(&[Input {
                name: "input",
                values: &input.values,
                shape: &[1, 128, input.frames],
            }])
            .await?;
        let output = Runtime::output(&outputs, "output")?;
        ensure!(
            output.shape == [1, input.frames, 360],
            "Unsupported RMVPE output shape"
        );
        frames.extend(crate::vocal_rmvpe::decode_chunk(
            pcm,
            start,
            &output.values[input.skip * 360..(input.skip + input.count) * 360],
        )?);
    }
    crate::vocal_rmvpe::finish_pcm(pcm, frames)
}

async fn swift(
    pcm: &[f32],
    progress: &mut impl FnMut(String),
) -> Result<crate::vocal_pitch::Evidence> {
    use crate::vocal_pitch::{HOP, decode_chunk, finish_pcm};
    ensure!(
        !pcm.is_empty() && pcm.len() <= 16000 * 3600,
        "Invalid vocal pitch source length"
    );
    let model = Runtime::load("vocal-pitch", progress, false).await?;
    let count = (pcm.len() / HOP).max(1);
    let mut frames = Vec::with_capacity(count);
    for start in (0..count).step_by(1875) {
        progress(format!(
            "Detecting vocal contour · chunk {} of {}",
            start / 1875 + 1,
            count.div_ceil(1875)
        ));
        let end = (start + 1875).min(count);
        let left = start.saturating_sub(11);
        let tail = if end < count {
            ((end + 10) * HOP).min(pcm.len())
        } else {
            pcm.len()
        };
        let samples = &pcm[left * HOP..tail];
        let outputs = model
            .run_inputs(&[
                Input {
                    name: "audio",
                    values: samples,
                    shape: &[1, samples.len()],
                },
                Input {
                    name: "fmin",
                    values: &[46.875],
                    shape: &[],
                },
                Input {
                    name: "fmax",
                    values: &[2093.75],
                    shape: &[],
                },
            ])
            .await?;
        let (pitch_shape, pitches) = Runtime::output_f64(&outputs, "pitch")?;
        let confidence = Runtime::output(&outputs, "confidence")?;
        ensure!(
            pitch_shape.len() == 2
                && pitch_shape[0] == 1
                && confidence.shape == pitch_shape
                && pitches.len() >= end - left,
            "Unexpected vocal pitch output contract"
        );
        frames.extend(decode_chunk(
            pcm,
            start,
            end,
            left,
            &pitches,
            &confidence.values,
        )?);
    }
    finish_pcm(pcm, frames)
}
