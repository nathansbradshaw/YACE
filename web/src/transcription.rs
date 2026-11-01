//! Local neural inference with the native clients' input windows and decoders.
use crate::{
    mapping::{DrumHit, PitchCandidateEvidence},
    models,
    onnx::Runtime,
};
use anyhow::{Result, ensure};
use std::path::Path;

pub async fn instruments(
    audio: &Path,
    manifest: &crate::stem_review::StemManifest,
    wanted: &[String],
    progress: &mut impl FnMut(String),
) -> Result<crate::mapping::Transcription> {
    ensure!(
        !wanted.is_empty()
            && wanted
                .iter()
                .all(|name| ["guitar", "bass", "drums"].contains(&name.as_str())),
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
        ..Default::default()
    };
    if wanted.iter().any(|n| n == "drums") {
        let path = stem("drums")?;
        let (hits, recovered) = drums(&path, progress).await?;
        out.drums = hits;
        out.drum_recovered_hits = recovered;
        out.drum_articulations =
            crate::drum_articulation::analyze(&models::decode(&path, 22050, 1)?, &out.drums)?;
        out.drum_articulation_source =
            Some(crate::drum_articulation::Source::from_manifest(manifest)?);
    }
    if wanted.iter().any(|n| n == "guitar") {
        let path = stem("guitar")?;
        let inference = models::postprocessing::guitar_from_activations(
            pitch_activations(&path, progress).await?,
            &models::decode(&path, 11025, 1)?,
            Default::default(),
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
        let inference = models::postprocessing::recover_short_bass(
            &pitch_activations(&path, progress).await?,
            &models::decode(&path, 11025, 1)?,
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
    out.warnings.push("Automatic model detections and arrangements require listening review. Model scores do not establish musical correctness.".into());
    out.validate()?;
    Ok(out)
}

pub async fn drums(
    audio: &Path,
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
    Ok(models::supported_drum_peaks(
        &activations,
        &models::decode(audio, 22050, 1)?,
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
    const WINDOW: usize = 160000;
    const HOP: usize = 80000;
    let samples = models::decode(audio, 16000, 1)?;
    ensure!(
        !samples.is_empty(),
        "Piano inference requires nonempty audio"
    );
    let model = Runtime::load("piano", progress, true).await?;
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
        Default::default(),
    )
}
