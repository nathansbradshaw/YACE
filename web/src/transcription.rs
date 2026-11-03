//! Local neural inference with the native clients' input windows and decoders.
use crate::{
    mapping::{DrumHit, PitchCandidateEvidence},
    models,
    onnx::Runtime,
};
use anyhow::{Context, Result, ensure};
use std::path::Path;

pub async fn instruments(
    audio: &Path,
    manifest: &crate::stem_review::StemManifest,
    wanted: &[String],
    guitar_source: crate::mapping::GuitarSource,
    progress: &mut impl FnMut(String),
) -> Result<crate::mapping::Transcription> {
    instruments_with_sound(
        audio,
        manifest,
        wanted,
        guitar_source,
        Default::default(),
        None,
        progress,
    )
    .await
}
pub async fn instruments_with_sound(
    audio: &Path,
    manifest: &crate::stem_review::StemManifest,
    wanted: &[String],
    guitar_source: crate::mapping::GuitarSource,
    keyboard_sound: crate::keyboard_detection::KeyboardSound,
    keyboard_analysis: Option<&crate::stem_review::StemManifest>,
    progress: &mut impl FnMut(String),
) -> Result<crate::mapping::Transcription> {
    instruments_configured(
        audio,
        manifest,
        wanted,
        guitar_source,
        keyboard_sound,
        keyboard_analysis,
        Default::default(),
        progress,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub async fn instruments_configured(
    audio: &Path,
    manifest: &crate::stem_review::StemManifest,
    wanted: &[String],
    guitar_source: crate::mapping::GuitarSource,
    keyboard_sound: crate::keyboard_detection::KeyboardSound,
    keyboard_analysis: Option<&crate::stem_review::StemManifest>,
    settings: crate::detection_settings::Settings,
    progress: &mut impl FnMut(String),
) -> Result<crate::mapping::Transcription> {
    settings.validate()?;
    ensure!(
        !wanted.is_empty()
            && wanted
                .iter()
                .all(|name| ["guitar", "bass", "drums", "piano", "pro_piano"]
                    .contains(&name.as_str())),
        "Choose supported instrument parts"
    );
    let duration = manifest.frames as f64 / 44100.;
    manifest.validate_binding(&models::hash_file(audio)?, duration)?;
    for stem in &manifest.stems {
        ensure!(
            models::hash_file(&stem.path)? == stem.sha256,
            "Separated audio changed"
        );
    }
    let stem = |name: &str| -> Result<_> {
        manifest
            .stems
            .iter()
            .find(|stem| stem.id.as_str() == name)
            .map(|stem| stem.path.clone())
            .ok_or_else(|| anyhow::anyhow!("Missing {name} audio"))
    };
    let mut out = crate::mapping::Transcription {
        isolation: Some(manifest.clone()),
        guitar_source,
        ..Default::default()
    };
    if wanted.iter().any(|n| n == "drums") {
        let path = stem("drums")?;
        let (hits, recovered) = drums_configured(&path, settings, progress).await?;
        out.drums = hits;
        out.drum_recovered_hits = recovered;
        out.drum_articulations =
            crate::drum_articulation::analyze(&models::decode(&path, 22050, 1)?, &out.drums)?;
        cymbals(&path, &out.drums, &mut out.drum_articulations, progress).await?;
        out.drum_articulation_source =
            Some(crate::drum_articulation::Source::from_manifest(manifest)?);
    }
    if wanted.iter().any(|n| n == "guitar") {
        let path = stem(guitar_source.stem().as_str())?;
        let inference = models::postprocessing::guitar_from_activations_configured(
            pitch_activations(&path, progress).await?,
            &models::decode(&path, 11025, 1)?,
            guitar_source,
            settings,
        );
        out.guitar = inference
            .candidates
            .iter()
            .map(|e| e.candidate.clone())
            .collect();
        out.guitar_candidates = inference.candidates;
        out.guitar_short_candidates = inference.short_candidates;
        out.guitar_polyphonic_notes = inference.polyphonic_notes;
        out.guitar_polyphonic_diagnostics = inference.diagnostics;
    }
    if wanted.iter().any(|n| n == "bass") {
        let path = stem("bass")?;
        let inference = models::postprocessing::recover_short_bass_configured(
            &pitch_activations(&path, progress).await?,
            &models::decode(&path, 11025, 1)?,
            settings,
        );
        out.bass = inference
            .candidates
            .iter()
            .map(|e| e.candidate.clone())
            .collect();
        out.bass_candidates = inference.candidates;
        out.bass_short_candidates = inference.short_candidates;
        let hash = models::assets()
            .into_iter()
            .find(|asset| asset.id == "pitch")
            .unwrap()
            .sha256;
        crate::bass_analysis::record_decoder_identity(&mut out, &path, hash)?;
        crate::bass_analysis::analyze(&mut out, &path)?;
    }
    if wanted.iter().any(|n| n == "piano" || n == "pro_piano") {
        let keyboard_manifest = keyboard_analysis.unwrap_or(manifest);
        if let Some(analysis) = keyboard_analysis {
            analysis.validate_binding(&manifest.source_audio_sha256, duration)?;
            for stem in &analysis.stems {
                ensure!(
                    models::hash_file(&stem.path)? == stem.sha256,
                    "HQ separated audio changed"
                );
            }
            out.keyboard_analysis = Some(analysis.clone());
        }
        let evidence = if keyboard_sound == crate::keyboard_detection::KeyboardSound::Synth {
            crate::keyboard_detection::synth_from_output_configured(
                manifest,
                crate::synth::transcribe(audio, progress).await?,
                settings,
            )
        } else {
            let path = keyboard_manifest
                .stems
                .iter()
                .find(|s| s.id == crate::stem_review::StemId::Piano)
                .context("Missing piano audio")?
                .path
                .clone();
            let model = Runtime::load("piano", progress, true).await?;
            let baseline = piano_samples_configured(
                &models::decode(&path, 16000, 1)?,
                &model,
                settings,
                progress,
            )
            .await?;
            let residual = if baseline.notes.iter().any(|n| n.confidence >= 0.65)
                && [
                    crate::stem_review::StemId::Drums,
                    crate::stem_review::StemId::Bass,
                    crate::stem_review::StemId::Vocals,
                ]
                .iter()
                .all(|id| keyboard_manifest.stems.iter().any(|stem| stem.id == *id))
            {
                let prepared = crate::keyboard_audio::prepare(audio, keyboard_manifest)?;
                let output =
                    piano_samples_configured(&prepared.samples, &model, settings, progress).await?;
                Some((prepared.receipt, output))
            } else {
                None
            };
            crate::keyboard_detection::piano_from_views_configured(
                keyboard_manifest,
                baseline,
                residual,
                settings,
            )?
        };
        if wanted.iter().any(|n| n == "piano") {
            out.piano = evidence.raw_notes.clone();
        }
        if wanted.iter().any(|n| n == "pro_piano") {
            out.pro_piano = evidence.raw_notes.clone();
        }
        out.keyboard_detection = Some(evidence);
    }
    out.warnings.push("Automatic model detections and arrangements require listening review. Model scores do not establish musical correctness.".into());
    out.validate()?;
    Ok(out)
}

async fn cymbals(
    audio: &Path,
    hits: &[DrumHit],
    evidence: &mut [crate::drum_articulation::Evidence],
    progress: &mut impl FnMut(String),
) -> Result<()> {
    let samples = models::decode(audio, 44100, 1)?;
    let count = samples.len() / 441 + 1;
    let model = Runtime::load("drum-articulation", progress, false).await?;
    let mut frames = vec![[0.; 8]; count];
    for core in (0..samples.len()).step_by(20 * 44100) {
        progress(format!(
            "Detecting cymbal articulations · {:.0}/{:.0} seconds",
            core as f64 / 44100.,
            samples.len() as f64 / 44100.
        ));
        let start = core.saturating_sub(3 * 44100);
        let end = (core + 23 * 44100).min(samples.len());
        let mut chunk = samples[start..end].to_vec();
        chunk.resize(chunk.len().max(44100), 0.);
        let outputs = model.run("waveform", &chunk, &[1, chunk.len()]).await?;
        let output = Runtime::output(&outputs, "onset_probs")?;
        ensure!(
            output.shape == [1, chunk.len() / 441 + 1, 8],
            "Unexpected detailed drum output shape"
        );
        let local = (core - start) / 441;
        let global = core / 441;
        let take = if core + 20 * 44100 >= samples.len() {
            count - global
        } else {
            2000.min(count - global)
        };
        ensure!(
            (local + take) * 8 <= output.values.len(),
            "Detailed drum output shorter than source clock"
        );
        for (frame, values) in frames[global..global + take]
            .iter_mut()
            .zip(output.values[local * 8..(local + take) * 8].chunks_exact(8))
        {
            frame.copy_from_slice(values);
        }
    }
    crate::drum_articulation::classify_cymbals(
        &frames,
        hits,
        evidence,
        samples.len() as f64 / 44100.,
    )
}

pub async fn drums(
    audio: &Path,
    progress: &mut impl FnMut(String),
) -> Result<(Vec<DrumHit>, Vec<DrumHit>)> {
    drums_configured(audio, Default::default(), progress).await
}
pub async fn drums_configured(
    audio: &Path,
    settings: crate::detection_settings::Settings,
    progress: &mut impl FnMut(String),
) -> Result<(Vec<DrumHit>, Vec<DrumHit>)> {
    let samples = models::decode(audio, 44100, 1)?;
    let model = Runtime::load("drums", progress, false).await?;
    let count = samples.len().div_ceil(441);
    let mut activations = vec![0.; count * 5];
    for core in (0..samples.len()).step_by(20 * 44100) {
        progress(format!(
            "Transcribing drums · {:.0}/{:.0} seconds",
            core as f64 / 44100.,
            samples.len() as f64 / 44100.
        ));
        let start = core.saturating_sub(3 * 44100);
        let end = (core + 23 * 44100).min(samples.len());
        let mut chunk = samples[start..end].to_vec();
        chunk.resize(chunk.len().max(44100), 0.);
        let outputs = model.run("waveform", &chunk, &[1, chunk.len()]).await?;
        let names = js_sys::Object::keys(&js_sys::Object::from(outputs.clone()));
        ensure!(names.length() == 1, "Unexpected ADTOF output contract");
        let output = Runtime::output(&outputs, &names.get(0).as_string().unwrap_or_default())?;
        ensure!(
            output.shape.len() == 3 && output.shape[0] == 1 && output.shape[2] == 5,
            "Unexpected ADTOF output shape"
        );
        let local = (core - start) / 441;
        let global = core / 441;
        let frames = (20 * 100).min(count - global);
        ensure!(
            (local + frames) * 5 <= output.values.len(),
            "ADTOF output too short"
        );
        activations[global * 5..(global + frames) * 5]
            .copy_from_slice(&output.values[local * 5..(local + frames) * 5]);
    }
    Ok(models::postprocessing::supported_drum_peaks_configured(
        &activations,
        &models::decode(audio, 22050, 1)?,
        settings,
    ))
}

pub async fn pitch_activations(
    audio: &Path,
    progress: &mut impl FnMut(String),
) -> Result<models::postprocessing::PitchActivations> {
    const WINDOW: usize = 43844;
    const HOP: usize = WINDOW - 30 * 256;
    let audio = models::decode(audio, 22050, 1)?;
    let original = audio.len();
    let mut padded = vec![0.; 15 * 256];
    padded.extend_from_slice(&audio);
    let model = Runtime::load("pitch", progress, false).await?;
    let (mut frames, mut onsets) = (Vec::new(), Vec::new());
    for start in (0..padded.len()).step_by(HOP) {
        progress(format!(
            "Detecting instrument pitches · {:.0}/{:.0} seconds",
            start.min(original) as f64 / 22050.,
            original as f64 / 22050.
        ));
        let mut chunk = padded[start..(start + WINDOW).min(padded.len())].to_vec();
        chunk.resize(WINDOW, 0.);
        let outputs = model
            .run("serving_default_input_2:0", &chunk, &[1, WINDOW, 1])
            .await?;
        for (label, target) in [
            ("StatefulPartitionedCall:1", &mut frames),
            ("StatefulPartitionedCall:2", &mut onsets),
        ] {
            let output = Runtime::output(&outputs, label)?;
            ensure!(
                output.shape == [1, 172, 88],
                "Unexpected Basic Pitch {label} output shape"
            );
            target.extend_from_slice(&output.values[15 * 88..(172 - 15) * 88]);
        }
    }
    let duration = original as f64 / 22050.;
    let expected = (0..frames.len() / 88)
        .take_while(|frame| models::pitch_frame_seconds(*frame) < duration)
        .count();
    frames.truncate(expected * 88);
    onsets.truncate(expected * 88);
    Ok(models::postprocessing::PitchActivations {
        frames,
        onsets,
        audio,
    })
}

pub async fn pitch(
    audio: &Path,
    progress: &mut impl FnMut(String),
) -> Result<Vec<PitchCandidateEvidence>> {
    let data = pitch_activations(audio, progress).await?;
    Ok(models::postprocessing::clip_pitch_candidates(
        models::decode_pitch_with_evidence(&data.frames, &data.onsets),
        data.audio.len() as f64 / 22050.,
    ))
}

pub async fn piano(
    audio: &Path,
    progress: &mut impl FnMut(String),
) -> Result<crate::piano_model::Output> {
    let samples = models::decode(audio, 16000, 1)?;
    let model = Runtime::load("piano", progress, true).await?;
    piano_samples(&samples, &model, progress).await
}
async fn piano_samples(
    samples: &[f32],
    model: &Runtime,
    progress: &mut impl FnMut(String),
) -> Result<crate::piano_model::Output> {
    piano_samples_configured(samples, model, Default::default(), progress).await
}
async fn piano_samples_configured(
    samples: &[f32],
    model: &Runtime,
    settings: crate::detection_settings::Settings,
    progress: &mut impl FnMut(String),
) -> Result<crate::piano_model::Output> {
    const WINDOW: usize = 160000;
    const HOP: usize = 80000;
    ensure!(
        !samples.is_empty(),
        "Piano inference requires nonempty audio"
    );
    let heads = [
        ("reg_onset_output", 88),
        ("reg_offset_output", 88),
        ("frame_output", 88),
        ("velocity_output", 88),
        ("reg_pedal_offset_output", 1),
        ("pedal_frame_output", 1),
    ];
    let padded = samples.len().div_ceil(WINDOW) * WINDOW;
    let windows = (padded - WINDOW) / HOP + 1;
    let mut values: [Vec<f32>; 6] = std::array::from_fn(|_| Vec::new());
    for index in 0..windows {
        progress(format!(
            "Detecting piano notes · window {} of {windows}",
            index + 1
        ));
        let start = index * HOP;
        let mut waveform = vec![0.; WINDOW];
        let n = WINDOW.min(samples.len().saturating_sub(start));
        waveform[..n].copy_from_slice(&samples[start..start + n]);
        let outputs = model.run("waveform", &waveform, &[1, WINDOW]).await?;
        let (from, to) = crate::piano_model::retained_frames(index, windows);
        for (head, (name, classes)) in heads.iter().enumerate() {
            let output = Runtime::output(&outputs, name)?;
            ensure!(
                output.shape == [1, 1001, *classes],
                "Unexpected Kong {name} shape"
            );
            values[head].extend_from_slice(&output.values[from * classes..to * classes]);
        }
    }
    let count = (samples.len() / 160 + 1).min(values[0].len() / 88);
    for (head, (_, classes)) in heads.iter().enumerate() {
        values[head].truncate(count * classes);
    }
    let [
        reg_onset,
        reg_offset,
        frame,
        velocity,
        reg_pedal_offset,
        pedal_frame,
    ] = values;
    crate::piano_model::decode(
        &crate::piano_model::Activations {
            duration_seconds: samples.len() as f64 / 16000.,
            reg_onset,
            reg_offset,
            frame,
            velocity,
            reg_pedal_offset,
            pedal_frame,
        },
        settings.piano(),
    )
}
