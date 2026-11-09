//! Experimental note-level corroboration. Never adds/removes a voiced frame.
//! This profile was selected on development recordings; promotion requires a
//! separate reference-bound gate. No word text, key or scale enters selection.
use crate::{
    vocal_generation::{PitchFrame, PitchSupport},
    vocal_pitch::Evidence as Contour,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
pub const ALGORITHM: &str = "rosvot-swift-correction-v1;retain-v10-voicing;swift>=0.5;note-agreement<0.75st;change-cost3.2;RMVPE-residual-floor0.5;no-gap-fill";
pub const NOTE_ALGORITHM: &str =
    "rosvot-rust-onnx-v1;40-of-80-slaney;24k;hop128;rwbd0.5;notebd0.85;3000-core375-context";
pub const ENCODER_SHA: &str = "09e88a7cd4497d130e87e1abca59974bfaf80d705f89c88ae8ba71d256635f6f";
pub const DECODER_SHA: &str = "6b3b678a34a78ab8c418160f43b30c427cca415595003451472ac0403259e911";
pub const RWBD_SHA: &str = "20aa5fc99e405655800531f72a0cc5ca789dbb82e462d6eae4bbc4d74cde7962";
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Evidence {
    pub algorithm: String,
    pub implementation_sha256: String,
    pub checkpoint_revision: String,
    pub stem_sha256: String,
    pub encoder_sha256: String,
    pub decoder_sha256: String,
    pub rwbd_sha256: String,
    pub duration_seconds: f64,
    pub notes: Vec<Note>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Note {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub midi: u8,
}
pub fn validate(e: &Evidence, stem: &str, duration: f64) -> Result<()> {
    ensure!(
        e.algorithm == NOTE_ALGORITHM
            && e.implementation_sha256 == crate::vocal_note_recipe::implementation_hash()
            && e.checkpoint_revision == "3c8332bf43adae35f6e4d64971862f2f6139b310"
            && e.stem_sha256.eq_ignore_ascii_case(stem)
            && e.encoder_sha256 == ENCODER_SHA
            && e.decoder_sha256 == DECODER_SHA
            && e.rwbd_sha256 == RWBD_SHA
            && e.duration_seconds.is_finite()
            && (e.duration_seconds - duration).abs() <= 1. / 16000.,
        "Note-event evidence belongs to another source, model, implementation or clock"
    );
    ensure!(
        e.notes.len() <= 100_000
            && e.notes.iter().all(|n| n.start_seconds.is_finite()
                && n.end_seconds.is_finite()
                && n.start_seconds >= 0.
                && n.start_seconds < n.end_seconds
                && n.end_seconds <= duration + 128. / 24000.
                && (30..=85).contains(&n.midi)),
        "Invalid note-event bounds"
    );
    // Context chunks can emit overlapping candidates out of chronological
    // order. Consumers inspect every event and retain model output priority;
    // requiring sorted starts rejected valid evidence without improving pitch.
    Ok(())
}
/// Work on a copy, and commit only after cancellation and all validations pass.
pub fn refine(
    frames: &mut [PitchFrame],
    primary: &Contour,
    swift: &Contour,
    notes: &Evidence,
    cancelled: &dyn Fn() -> bool,
) -> Result<usize> {
    ensure!(!cancelled(), "Vocal generation cancelled");
    validate_inputs(primary, swift, notes)?;
    refine_validated(frames, primary, swift, notes, cancelled)
}
pub fn validate_inputs(primary: &Contour, swift: &Contour, notes: &Evidence) -> Result<()> {
    crate::vocal_rmvpe::validate(
        primary,
        &primary.audio_sha256,
        &primary.vocal_stem_sha256,
        primary.duration_seconds,
    )?;
    crate::vocal_generation::validate_vocal_frame_evidence(
        swift,
        &primary.audio_sha256,
        &primary.vocal_stem_sha256,
        primary.duration_seconds,
    )?;
    ensure!(
        swift.pcm_sha256 == primary.pcm_sha256,
        "Comparison contours use different PCM"
    );
    validate(notes, &primary.vocal_stem_sha256, primary.duration_seconds)?;
    Ok(())
}
fn refine_validated(
    frames: &mut [PitchFrame],
    primary: &Contour,
    swift: &Contour,
    notes: &Evidence,
    cancelled: &dyn Fn() -> bool,
) -> Result<usize> {
    ensure!(frames.len() <= 400_000, "Frame budget exceeded");
    let mut draft = frames.to_vec();
    let mut weights = vec![1.; frames.len()];
    for (i, f) in draft.iter_mut().enumerate() {
        if i % 1000 == 0 {
            ensure!(!cancelled(), "Vocal generation cancelled")
        };
        if f.midi.is_none() {
            continue;
        }
        let at = ((f.time_seconds / 0.01).round() as usize).min(primary.frames.len() - 1);
        let p = &primary.frames[at];
        weights[i] = (p.confidence as f64).max(0.5);
        let at = ((f.time_seconds / 0.016).round() as usize).min(swift.frames.len() - 1);
        let s = &swift.frames[at];
        let estimate = s
            .frequency_hz
            .filter(|_| s.confidence >= 0.5 && (s.time_seconds - f.time_seconds).abs() <= 0.008001)
            .map(|h| 69. + 12. * (h / 440.).log2());
        let note = notes
            .notes
            .iter()
            .find(|n| n.start_seconds <= f.time_seconds && f.time_seconds < n.end_seconds);
        if let Some((p, n)) = estimate
            .zip(note)
            .filter(|(p, n)| (36..=84).contains(&n.midi) && (*p - n.midi as f64).abs() < 0.75)
        {
            let _ = p;
            f.midi = Some(n.midi as f64);
        }
    }
    crate::vocal_pitch_tracking::weighted_note_path(&mut draft, 3.2, &weights, cancelled)?;
    let mut changed = 0;
    for (before, after) in frames.iter().zip(&mut draft) {
        ensure!(
            before.midi.is_some() == after.midi.is_some(),
            "Correction changed voicing"
        );
        if before.midi != after.midi {
            after.support = PitchSupport::AcousticAlternative;
            after.clarity = after.clarity.min(0.5);
            changed += 1
        }
    }
    ensure!(!cancelled(), "Vocal generation cancelled");
    frames.clone_from_slice(&draft);
    Ok(changed)
}
