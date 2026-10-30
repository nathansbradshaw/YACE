//! Refine raw bass detections against the same source audio.
//! Start each replay from raw notes so earlier corrections are not applied twice.
use crate::{
    mapping::{PitchNote, Transcription},
    models,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decoder {
    pub version: u32,
    pub implementation_sha256: String,
    pub model_sha256: String,
    pub bass_audio_sha256: String,
    pub candidates_sha256: String,
    pub short_candidates_sha256: String,
}

pub fn decoder_implementation_hash() -> String {
    let mut hash = Sha256::new();
    hash.update(include_bytes!("models.rs"));
    hash.update(include_bytes!("review.rs"));
    hash.update(include_bytes!("refine.rs"));
    hash.update(include_bytes!("../Cargo.lock"));
    format!("{:x}", hash.finalize())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    pub version: u32,
    pub implementation_sha256: String,
    pub bass_audio_sha256: String,
    pub raw_candidates_sha256: String,
    pub refined_notes_sha256: String,
    pub original_notes_sha256: String,
    pub refinement_evidence_sha256: String,
    pub original_notes: Vec<PitchNote>,
    pub warnings: Vec<String>,
}

pub fn implementation_hash() -> String {
    let mut hash = Sha256::new();
    for source in [
        include_bytes!("bass_analysis.rs").as_slice(),
        include_bytes!("refine.rs"),
        include_bytes!("refine/harmonics.rs"),
        include_bytes!("models.rs"),
        include_bytes!("review.rs"),
        include_bytes!("../Cargo.lock"),
    ] {
        hash.update(source);
    }
    format!("{:x}", hash.finalize())
}

fn hash(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn evidence_hash(events: &Transcription) -> Result<String> {
    hash(&(
        &events.bass_corrections,
        &events.bass_timing_corrections,
        &events.bass_shadow_support,
        &events.bass_attack_reviews,
        &events.bass_harmonic_conflicts,
        &events.bass_short_candidates,
    ))
}

pub fn validate(events: &Transcription) -> Result<()> {
    let valid_hash =
        |value: &str| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit());
    if let Some(decoder) = &events.bass_decoder {
        ensure!(
            decoder.version == 1
                && [
                    &decoder.implementation_sha256,
                    &decoder.model_sha256,
                    &decoder.bass_audio_sha256,
                    &decoder.candidates_sha256,
                    &decoder.short_candidates_sha256
                ]
                .iter()
                .all(|value| valid_hash(value)),
            "Invalid bass decoder identity"
        );
        ensure!(
            decoder.candidates_sha256 == hash(&events.bass_candidates)?
                && decoder.short_candidates_sha256 == hash(&events.bass_short_candidates)?,
            "Bass decoder evidence changed; raw candidates and their support cannot reuse an old decoder receipt"
        );
    }
    let Some(analysis) = &events.bass_analysis else {
        return Ok(());
    };
    ensure!(
        analysis.version == 1
            && valid_hash(&analysis.implementation_sha256)
            && valid_hash(&analysis.bass_audio_sha256)
            && valid_hash(&analysis.raw_candidates_sha256)
            && valid_hash(&analysis.refined_notes_sha256),
        "Invalid bass analysis identity"
    );
    ensure!(
        analysis.original_notes_sha256 == hash(&analysis.original_notes)?
            && analysis.refinement_evidence_sha256 == evidence_hash(events)?,
        "Bass original-input or refinement-evidence witness changed"
    );
    ensure!(
        hash(&events.bass_candidates)? == analysis.raw_candidates_sha256
            && hash(&events.bass)? == analysis.refined_notes_sha256,
        "Bass analysis no longer matches its notes or raw candidates; retain the prior analysis and explicitly regenerate from the intended inputs"
    );
    let raw = Transcription {
        bass: analysis.original_notes.clone(),
        ..Default::default()
    };
    raw.validate()?;
    if !events.bass_candidates.is_empty() {
        let candidates: Vec<_> = events
            .bass_candidates
            .iter()
            .map(|c| c.candidate.clone())
            .collect();
        ensure!(
            hash(&candidates)? == hash(&analysis.original_notes)?,
            "Bass raw input witness changed"
        );
    }
    Ok(())
}

pub fn decoder_is_current(events: &Transcription, audio: &Path) -> Result<bool> {
    validate(events)?;
    let Some(decoder) = &events.bass_decoder else {
        return Ok(false);
    };
    let expected_model = models::assets()
        .into_iter()
        .find(|asset| asset.id == "pitch");
    Ok(
        decoder.implementation_sha256 == decoder_implementation_hash()
            && expected_model.is_some_and(|asset| asset.sha256 == decoder.model_sha256)
            && decoder.bass_audio_sha256 == models::hash_file(audio)?,
    )
}

/// Called only after actual decoder inference, never by refinement replay.
pub(crate) fn record_decoder(events: &mut Transcription, audio: &Path, model: &Path) -> Result<()> {
    events.bass_decoder = Some(Decoder {
        version: 1,
        implementation_sha256: decoder_implementation_hash(),
        model_sha256: models::hash_file(model)?,
        bass_audio_sha256: models::hash_file(audio)?,
        candidates_sha256: hash(&events.bass_candidates)?,
        short_candidates_sha256: hash(&events.bass_short_candidates)?,
    });
    Ok(())
}

/// Decode again when the candidate extractor changes. Refinement replay alone
/// cannot recover attacks discarded by a previous decoder.
pub fn infer(events: &mut Transcription, audio: &Path, model: &Path) -> Result<()> {
    events.validate()?;
    let source_sha = models::hash_file(audio)?;
    let model_sha = models::hash_file(model)?;
    ensure!(
        models::assets()
            .iter()
            .any(|asset| asset.id == "pitch" && asset.sha256 == model_sha),
        "Bass decoder requires the pinned pitch model"
    );
    let inference = models::bass_pitch_with_evidence(audio, model)?;
    ensure!(
        models::hash_file(audio)? == source_sha && models::hash_file(model)? == model_sha,
        "Bass decoder inputs changed; previous events retained"
    );
    let mut updated = events.clone();
    updated.bass_analysis = None;
    updated.bass_decoder = None;
    if let Some(previous) = &events.bass_analysis {
        updated
            .warnings
            .retain(|warning| !previous.warnings.contains(warning));
    }
    updated.bass_candidates = inference.candidates;
    updated.bass_short_candidates = inference.short_candidates;
    updated.bass = updated
        .bass_candidates
        .iter()
        .map(|candidate| candidate.candidate.clone())
        .collect();
    updated.bass_corrections.clear();
    updated.bass_timing_corrections.clear();
    updated.bass_shadow_support.clear();
    updated.bass_attack_reviews.clear();
    updated.bass_harmonic_conflicts.clear();
    record_decoder(&mut updated, audio, model)?;
    analyze(&mut updated, audio)?;
    ensure!(
        models::hash_file(audio)? == source_sha && models::hash_file(model)? == model_sha,
        "Bass decoder inputs changed during refinement; previous events retained"
    );
    updated.validate()?;
    *events = updated;
    Ok(())
}

pub fn is_current(events: &Transcription, audio: &Path) -> Result<bool> {
    validate(events)?;
    let Some(analysis) = &events.bass_analysis else {
        return Ok(false);
    };
    Ok(analysis.implementation_sha256 == implementation_hash()
        && analysis.bass_audio_sha256 == models::hash_file(audio)?)
}

/// Recompute from saved original notes, preserving other instruments. Mutation
/// is atomic with respect to source/decoder errors and source-file replacement.
pub fn analyze(events: &mut Transcription, audio: &Path) -> Result<()> {
    events.validate()?;
    let source_hash = models::hash_file(audio)?;
    let original_notes = if !events.bass_candidates.is_empty() {
        events
            .bass_candidates
            .iter()
            .map(|c| c.candidate.clone())
            .collect()
    } else if let Some(analysis) = &events.bass_analysis {
        analysis.original_notes.clone()
    } else {
        events.bass.clone()
    };
    let mut fresh = Transcription {
        bass: original_notes.clone(),
        bass_candidates: events.bass_candidates.clone(),
        bass_short_candidates: events.bass_short_candidates.clone(),
        ..Default::default()
    };
    crate::refine::bass(&mut fresh, audio)?;
    ensure!(
        models::hash_file(audio)? == source_hash,
        "Bass audio changed during analysis; previous events were retained"
    );
    fresh.validate()?;
    let analysis = Analysis {
        version: 1,
        implementation_sha256: implementation_hash(),
        bass_audio_sha256: source_hash,
        raw_candidates_sha256: hash(&fresh.bass_candidates)?,
        refined_notes_sha256: hash(&fresh.bass)?,
        original_notes_sha256: hash(&original_notes)?,
        refinement_evidence_sha256: evidence_hash(&fresh)?,
        original_notes,
        warnings: fresh.warnings.clone(),
    };
    if let Some(previous) = &events.bass_analysis {
        events
            .warnings
            .retain(|warning| !previous.warnings.contains(warning));
    } else if !events.bass_candidates.is_empty() {
        // Only the unversioned legacy refinement summary is superseded; model,
        // isolation and other-instrument warnings remain intact.
        events.warnings.retain(|warning| {
            !warning
                .split_once(" bass upper-harmonic candidates lowered ")
                .is_some_and(|(count, _)| count.parse::<usize>().is_ok())
        });
    }
    events.warnings.extend(fresh.warnings);
    events.bass = fresh.bass;
    events.bass_corrections = fresh.bass_corrections;
    events.bass_timing_corrections = fresh.bass_timing_corrections;
    events.bass_shadow_support = fresh.bass_shadow_support;
    events.bass_attack_reviews = fresh.bass_attack_reviews;
    events.bass_harmonic_conflicts = fresh.bass_harmonic_conflicts;
    events.bass_analysis = Some(analysis);
    Ok(())
}
