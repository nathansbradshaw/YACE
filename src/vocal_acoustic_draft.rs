//! Explicit research: separate RMVPE observations, voicing, and MIDI assembly.
//! This profile is not a production quality mode or a calibrated singing model.
use crate::{
    vocal_generation::{self as vg, Generation, PitchSupport},
    vocal_posterior, vocal_rmvpe,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub minimum_confidence: f32,
    pub minimum_rms: f64,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.minimum_confidence.is_finite()
                && (0.03..=1.).contains(&self.minimum_confidence)
                && self.minimum_rms.is_finite()
                && (0. ..=1.).contains(&self.minimum_rms),
            "Invalid acoustic draft thresholds"
        );
        Ok(())
    }
}

pub fn implementation_hash() -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(include_bytes!("vocal_acoustic_draft.rs"));
    hash.update(include_bytes!("vocal_generation.rs"));
    hash.update(vocal_rmvpe::implementation_hash());
    format!("{:x}", hash.finalize())
}

/// Source-bound threshold diagnostic. Never uses words to accept a pitch.
/// The model's original continuous evidence remains unchanged in the report;
/// selected frame pitches remain continuous, and only chart notes are rounded.
pub fn reconstruct(
    base: &Generation,
    settings: Settings,
    cancelled: &dyn Fn() -> bool,
) -> Result<Generation> {
    settings.validate()?;
    ensure!(!cancelled(), "Acoustic reconstruction cancelled");
    ensure!(
        base.version == 1 && base.policy.analysis_mode == vg::AnalysisMode::HighAccuracy,
        "Acoustic reconstruction requires an explicit High accuracy report"
    );
    let raw = base
        .rmvpe_pitch
        .as_ref()
        .context("Missing RMVPE observations; another detector was not substituted")?;
    vocal_rmvpe::validate(
        raw,
        &base.audio_sha256,
        &base.vocal_stem_sha256,
        base.duration_seconds,
    )?;
    ensure!(
        !base.pitch_frames.is_empty() && base.pitch_frames.len() <= 400_000,
        "Unsupported acoustic frame budget"
    );
    let timing = base.timing.as_ref().context("Missing source MIDI clock")?;
    let mut out = base.clone();
    for (i, frame) in out.pitch_frames.iter_mut().enumerate() {
        if i % 1024 == 0 {
            ensure!(!cancelled(), "Acoustic reconstruction cancelled");
        }
        let at = raw
            .frames
            .partition_point(|f| f.time_seconds < frame.time_seconds);
        let observed = [at.saturating_sub(1), at.min(raw.frames.len() - 1)]
            .into_iter()
            .map(|j| &raw.frames[j])
            .min_by(|a, b| {
                (a.time_seconds - frame.time_seconds)
                    .abs()
                    .total_cmp(&(b.time_seconds - frame.time_seconds).abs())
            })
            .unwrap();
        frame.midi = observed
            .frequency_hz
            .filter(|_| {
                (observed.time_seconds - frame.time_seconds).abs() <= 0.00501
                    && observed.confidence >= settings.minimum_confidence
                    && observed.rms >= settings.minimum_rms
            })
            .map(|h| 69. + 12. * (h / 440.).log2())
            .filter(|p| (35.5..84.5).contains(p));
        frame.support = PitchSupport::AcousticAlternative;
        frame.clarity = observed.confidence as f64;
        frame.local_energy_active = None;
    }
    out.issues.clear();
    let assembly = vg::assemble_note_cells(
        &out.pitch_frames,
        out.duration_seconds,
        timing,
        out.words.as_ref(),
        &out.policy,
        &mut out.issues,
    )?;
    out.chart = assembly.chart;
    out.pitch_candidates = assembly.pitch_candidates;
    out.note_evidence = assembly.note_evidence;
    out.algorithm = format!(
        "{};RMVPE-voicing-threshold={};RMS={};uncalibrated-research-voicing",
        vg::NOTE_CELLS_ALGORITHM,
        settings.minimum_confidence,
        settings.minimum_rms
    );
    out.status = if out.chart.is_some() {
        vg::Status::ReviewRequired
    } else {
        vg::Status::NoVoicedPitch
    };
    out.warnings.push("Experimental acoustic reconstruction has not passed the production quality gate. Confidence is not a singing probability; inspect false voicing, boundaries and note fragmentation separately.".into());
    vocal_posterior::annotate(
        &mut out,
        &[vocal_posterior::ModelFrames {
            id: vocal_rmvpe::MODEL_ID.into(),
            model_sha256: raw.model_sha256.clone(),
            implementation_sha256: raw.implementation_sha256.clone(),
            midi: base
                .pitch_frames
                .iter()
                .map(|f| {
                    let at = raw
                        .frames
                        .partition_point(|r| r.time_seconds < f.time_seconds);
                    [at.saturating_sub(1), at.min(raw.frames.len() - 1)]
                        .into_iter()
                        .map(|i| &raw.frames[i])
                        .min_by(|a, b| {
                            (a.time_seconds - f.time_seconds)
                                .abs()
                                .total_cmp(&(b.time_seconds - f.time_seconds).abs())
                        })
                        .filter(|r| (r.time_seconds - f.time_seconds).abs() <= 0.00501)
                        .and_then(|r| r.frequency_hz)
                        .map(|h| 69. + 12. * (h / 440.).log2())
                        .filter(|p| (35.5..84.5).contains(p))
                })
                .collect(),
        }],
    )?;
    ensure!(!cancelled(), "Acoustic reconstruction cancelled");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{timing::TempoMap, vocal_pitch::Frame, vocals};
    fn fixture() -> Generation {
        let mut raw = vocal_rmvpe::finish_pcm(
            &vec![0.; 3200],
            (0..=20)
                .map(|i| Frame {
                    time_seconds: i as f64 * 0.01,
                    frequency_hz: Some(440.),
                    confidence: 0.9,
                    rms: 0.1,
                })
                .collect(),
        )
        .unwrap();
        raw.audio_sha256 = "a".repeat(64);
        raw.vocal_stem_sha256 = "b".repeat(64);
        Generation {
            version: 1,
            audio_sha256: raw.audio_sha256.clone(),
            vocal_stem_sha256: raw.vocal_stem_sha256.clone(),
            duration_seconds: 0.2,
            algorithm: "fixture".into(),
            timing: Some(TempoMap::constant(120.).unwrap()),
            policy: vg::Policy {
                analysis_mode: vg::AnalysisMode::HighAccuracy,
                ..Default::default()
            },
            status: vg::Status::ReviewRequired,
            chart: None,
            pitch_candidates: vec![],
            pitch_frames: (0..20)
                .map(|i| vg::PitchFrame {
                    time_seconds: i as f64 * 0.01,
                    midi: None,
                    yin_midi: None,
                    mcleod_midi: None,
                    clarity: 0.,
                    rms: 0.1,
                    local_rms: None,
                    local_energy_active: None,
                    support: PitchSupport::Unspecified,
                })
                .collect(),
            words: None,
            lyrics: None,
            issues: vec![],
            warnings: vec![],
            neural_pitch: None,
            vocal_pitch: None,
            rmvpe_pitch: Some(raw),
            note_evidence: vec![],
        }
    }
    #[test]
    fn raw_contour_and_clock_survive_voicing_and_playable_midi() {
        let mut base = fixture();
        let raw = base.rmvpe_pitch.as_mut().unwrap();
        raw.frames[4].confidence = 0.2;
        raw.frames[7].frequency_hz = None;
        raw.frames[10].rms = 0.;
        raw.frames[12].frequency_hz = Some(445.);
        let before = serde_json::to_vec(&base).unwrap();
        let out = reconstruct(
            &base,
            Settings {
                minimum_confidence: 0.8,
                minimum_rms: 0.0015,
            },
            &|| false,
        )
        .unwrap();
        assert_eq!(before, serde_json::to_vec(&base).unwrap());
        assert_eq!(
            serde_json::to_value(&base.rmvpe_pitch).unwrap(),
            serde_json::to_value(&out.rmvpe_pitch).unwrap()
        );
        assert_eq!(
            out.pitch_frames.iter().filter(|f| f.midi.is_some()).count(),
            17
        );
        assert_eq!(
            out.pitch_frames[12].midi,
            Some(69. + 12. * (445f64 / 440.).log2())
        );
        let chart = out.chart.unwrap();
        let timing = out.timing.unwrap();
        for f in &out.pitch_frames {
            let tick = timing.tick(f.time_seconds).unwrap();
            assert_eq!(
                chart.notes.iter().any(|n| n.start <= tick && tick < n.end),
                f.midi.is_some()
            );
        }
        let blank = crate::midi::new_midi(&timing, &Default::default()).unwrap();
        let bytes = vocals::append_midi(&blank, &chart).unwrap();
        let imported = vocals::import_midi(&bytes).unwrap();
        assert!(
            imported
                .notes
                .iter()
                .map(|n| (n.start, n.end, &n.tone, &n.lyric))
                .eq(chart
                    .notes
                    .iter()
                    .map(|n| (n.start, n.end, &n.tone, &n.lyric)))
        );
        assert!(
            out.note_evidence
                .iter()
                .all(|n| n.uncertain && !n.models.is_empty())
        );
    }
    #[test]
    fn stale_sources_invalid_settings_and_cancellation_cannot_publish_a_draft() {
        let base = fixture();
        let settings = Settings {
            minimum_confidence: 0.8,
            minimum_rms: 0.0015,
        };
        assert!(reconstruct(&base, settings, &|| true).is_err());
        for change in 0..3 {
            let mut bad = base.clone();
            match change {
                0 => bad.audio_sha256 = "c".repeat(64),
                1 => bad.rmvpe_pitch.as_mut().unwrap().model_sha256 = "c".repeat(64),
                _ => bad.duration_seconds = 0.21,
            };
            assert!(reconstruct(&bad, settings, &|| false).is_err());
        }
        assert!(
            reconstruct(
                &base,
                Settings {
                    minimum_confidence: f32::NAN,
                    minimum_rms: 0.
                },
                &|| false
            )
            .is_err()
        );
        let calls = std::cell::Cell::new(0);
        assert!(
            reconstruct(&base, settings, &|| {
                let n = calls.get();
                calls.set(n + 1);
                n > 0
            })
            .is_err()
        );
    }
}
