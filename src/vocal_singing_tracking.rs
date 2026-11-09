//! Fixed, lyric-free JDC/RMVPE draft decoder. No artist, key or chart priors.
use crate::{
    vocal_generation::{self as vg, Generation, PitchSupport},
    vocal_posterior::{self, ModelFrames},
    vocal_rmvpe,
    vocal_singing::{self as jdc, Evidence},
};
use anyhow::{Context, Result, ensure};
pub const PROFILE: &str = "JDC-RMVPE-hysteresis-v2;JDC-band0.9-detection0.7-RMVPE0.03-agreement0.75;change3.2;bounded-same-pitch-gaps180ms-JDCband0.03-detection0.7-RMVPE0.03-agreement0.5-nonzero-local-energy;minimum-note60ms;no-lyrics";
pub fn implementation_hash() -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(include_bytes!("vocal_singing_tracking.rs"));
    h.update(jdc::implementation_hash());
    h.update(vocal_rmvpe::implementation_hash());
    h.update(vg::COMPARISON_ALGORITHM);
    format!("{:x}", h.finalize())
}
fn row(e: &Evidence, t: f64) -> Option<&jdc::Frame> {
    e.frames
        .get((t / 0.01).round() as usize)
        .filter(|r| (r.time_seconds - t).abs() <= 0.00501)
}
fn midi(f: &crate::vocal_pitch::Frame) -> Option<f64> {
    f.frequency_hz.map(|h| 69. + 12. * (h / 440.).log2())
}
fn nearest(nn: &[crate::vocal_pitch::Frame], t: f64) -> &crate::vocal_pitch::Frame {
    let n = nn.partition_point(|n| n.time_seconds < t);
    [n.saturating_sub(1), n.min(nn.len() - 1)]
        .into_iter()
        .map(|i| &nn[i])
        .min_by(|a, b| {
            (a.time_seconds - t)
                .abs()
                .total_cmp(&(b.time_seconds - t).abs())
        })
        .unwrap()
}
pub fn decode(
    base: &Generation,
    evidence: &Evidence,
    cancelled: &dyn Fn() -> bool,
) -> Result<Generation> {
    ensure!(!cancelled(), "Singing pitch analysis cancelled");
    jdc::validate(evidence, &base.vocal_stem_sha256, base.duration_seconds)?;
    let rmvpe = base.rmvpe_pitch.as_ref().context(
        "Singing pitch requires independent RMVPE evidence; another mode was not substituted",
    )?;
    vocal_rmvpe::validate(
        rmvpe,
        &base.audio_sha256,
        &base.vocal_stem_sha256,
        base.duration_seconds,
    )?;
    ensure!(
        base.policy.analysis_mode == vg::AnalysisMode::HighAccuracy
            && base.pitch_frames.len() <= 400_000
            && base.pitch_frames.iter().all(|f| f.time_seconds.is_finite()
                && f.time_seconds >= 0.
                && f.time_seconds <= base.duration_seconds
                && f.local_rms.is_none_or(|p| p.is_finite() && p >= 0.)),
        "Unsupported singing pitch frame clock or policy"
    );
    let mut out = base.clone();
    for f in &mut out.pitch_frames {
        ensure!(!cancelled(), "Singing pitch analysis cancelled");
        f.midi = row(evidence, f.time_seconds).and_then(|r| {
            let best = (0..jdc::BANDS)
                .max_by(|a, b| r.note_band_mass[*a].total_cmp(&r.note_band_mass[*b]))
                .unwrap();
            let p = (best + 36) as f64;
            let nn = rmvpe
                .frames
                .iter()
                .skip(((f.time_seconds / 0.01).round() as usize).saturating_sub(2))
                .take(5)
                .min_by(|a, b| {
                    (a.time_seconds - f.time_seconds)
                        .abs()
                        .total_cmp(&(b.time_seconds - f.time_seconds).abs())
                })
                .filter(|n| (n.time_seconds - f.time_seconds).abs() <= 0.020);
            let agreement = nn
                .filter(|n| midi(n).is_some_and(|v| (v - p).abs() <= 0.75))
                .map_or(0., |n| n.confidence as f64);
            (r.midi.is_some()
                && f.local_rms.unwrap_or(f.rms) > 0.
                && r.note_band_mass[best] >= 0.9
                && r.detection_singing_probability as f64 >= 0.7
                && agreement >= 0.03)
                .then_some(p)
        });
    }
    crate::vocal_pitch_tracking::note_path(&mut out.pitch_frames, 3.2, cancelled)?;
    out.policy.minimum_note_seconds = 0.06;
    out.policy.local_energy_gate = false;
    out.policy.recover_bounded_yin = false;
    for f in &mut out.pitch_frames {
        f.local_energy_active = None;
        if f.midi.is_some() {
            f.support = PitchSupport::AcousticAlternative;
            f.clarity = f.clarity.min(0.5);
        }
    }
    let mut i = 0;
    while i < out.pitch_frames.len() {
        ensure!(!cancelled(), "Singing pitch analysis cancelled");
        if out.pitch_frames[i].midi.is_some() {
            i += 1;
            continue;
        }
        let a = i;
        while i < out.pitch_frames.len() && out.pitch_frames[i].midi.is_none() {
            i += 1;
        }
        let b = i;
        let frames = &out.pitch_frames;
        if a == 0 || b == frames.len() {
            continue;
        }
        let p = frames[a - 1].midi.unwrap();
        if frames[b].midi != Some(p)
            || frames[b].time_seconds - frames[a].time_seconds > 0.18 + 1e-9
        {
            continue;
        }
        let supported = frames[a..b].iter().all(|f| {
            let n = nearest(&rmvpe.frames, f.time_seconds);
            row(evidence, f.time_seconds).is_some_and(|r| {
                r.note_band_mass[p as usize - 36] >= 0.03
                    && r.detection_singing_probability as f64 >= 0.7
            }) && f.local_rms.unwrap_or(f.rms) > 0.
                && (n.time_seconds - f.time_seconds).abs() <= 0.020
                && n.confidence as f64 >= 0.03
                && midi(n).is_some_and(|v| (v - p).abs() <= 0.5)
        });
        if supported {
            for f in &mut out.pitch_frames[a..b] {
                f.midi = Some(p);
                f.support = PitchSupport::AcousticAlternative;
                f.clarity = f.clarity.min(0.5);
            }
        }
    }
    out.words = None;
    out.lyrics = None;
    out.issues.clear();
    let assembly = vg::assemble_notes(
        &out.pitch_frames,
        out.duration_seconds,
        out.timing.as_ref().context("Source tempo map")?,
        None,
        &out.policy,
        &mut out.issues,
    )?;
    out.chart = assembly.chart;
    out.pitch_candidates = assembly.pitch_candidates;
    out.note_evidence = assembly.note_evidence;
    // Missing text is an expected authoring state for this explicit pitch-only
    // mode. Retain Missing syllables and every pitch/coverage review issue.
    out.issues.retain(|i| i.reason != vg::Reason::MissingLyrics);
    out.algorithm = PROFILE.into();
    out.status = if out.chart.is_some() {
        vg::Status::ReviewRequired
    } else {
        vg::Status::NoVoicedPitch
    };
    if let Some(chart) = &mut out.chart {
        chart.source="Source-bound JDC/RMVPE singing pitch draft; lyrics optional; review acoustic alternatives".into();
    }
    for n in &mut out.note_evidence {
        n.uncertain = true;
        n.reasons.push("JDC/RMVPE acoustic draft; model agreement does not establish correctness or singer identity.".into());
    }
    let jdc = out
        .pitch_frames
        .iter()
        .map(|f| row(evidence, f.time_seconds).and_then(|r| r.midi))
        .collect();
    let nn = out
        .pitch_frames
        .iter()
        .map(|f| {
            let n = nearest(&rmvpe.frames, f.time_seconds);
            if (n.time_seconds - f.time_seconds).abs() <= 0.020 && n.confidence >= 0.03 {
                midi(n).filter(|p| (35.5..84.5).contains(p))
            } else {
                None
            }
        })
        .collect();
    vocal_posterior::annotate(
        &mut out,
        &[
            ModelFrames {
                id: "JDC joint singing and pitch".into(),
                model_sha256: evidence.model_sha256.clone(),
                implementation_sha256: evidence.implementation_sha256.clone(),
                midi: jdc,
            },
            ModelFrames {
                id: "RMVPE".into(),
                model_sha256: vocal_rmvpe::SHA256.into(),
                implementation_sha256: vocal_rmvpe::implementation_hash(),
                midi: nn,
            },
        ],
    )?;
    out.warnings.push("Singing pitch uses no lyrics. JDC covers MIDI38..83; all notes remain uncertain drafts. Mono cancellation, leakage and harmonies may still cause omissions or wrong singer pitches.".into());
    ensure!(!cancelled(), "Singing pitch analysis cancelled");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Generation, Evidence) {
        let policy = vg::Policy {
            analysis_mode: vg::AnalysisMode::HighAccuracy,
            ..Default::default()
        };
        let timing = crate::timing::TempoMap::constant(120.).unwrap();
        let mut g = vg::generate_pcm(
            &vec![0.; 22050],
            &"a".repeat(64),
            &"b".repeat(64),
            1.,
            &timing,
            None,
            None,
            &policy,
        )
        .unwrap();
        for f in &mut g.pitch_frames {
            f.rms = 0.1;
            f.local_rms = Some(0.1);
        }
        let frames = (0..=100)
            .map(|i| crate::vocal_pitch::Frame {
                time_seconds: i as f64 * 0.01,
                frequency_hz: Some(440. * 2f64.powf((60. - 69.) / 12.)),
                confidence: 0.8,
                rms: 0.1,
            })
            .collect();
        let mut nn = vocal_rmvpe::finish_pcm(&vec![0.1; 16000], frames).unwrap();
        nn.audio_sha256 = g.audio_sha256.clone();
        nn.vocal_stem_sha256 = g.vocal_stem_sha256.clone();
        g.rmvpe_pitch = Some(nn);
        let mut posterior = vec![0.; jdc::CLASSES];
        posterior[1 + 22 * 16] = 1.;
        let e = Evidence {
            version: 1,
            source_sha256: g.vocal_stem_sha256.clone(),
            pcm_sha256: "c".repeat(64),
            checkpoint_sha256: jdc::CHECKPOINT_SHA256.into(),
            model_sha256: jdc::MODEL_SHA256.into(),
            implementation_sha256: jdc::implementation_hash(),
            runtime_sha256: "d".repeat(64),
            runtime_publisher_sha256: jdc::RUNTIME_PUBLISHER_SHA256.into(),
            sample_count: 8000,
            sample_rate: 8000,
            hop_samples: 80,
            duration_seconds: 1.,
            frames: (0..=100)
                .map(|i| jdc::frame(i, &posterior, 1.).unwrap())
                .collect(),
        };
        (g, e)
    }
    #[test]
    fn lyrics_are_unused_and_pitched_missing_syllables_survive_midi() {
        let (mut g, e) = fixture();
        let a = decode(&g, &e, &|| false).unwrap();
        g.lyrics = Some(vg::LyricSource {
            text: "incorrect words".into(),
            source: "test".into(),
            synced_lyrics: None,
        });
        let b = decode(&g, &e, &|| false).unwrap();
        assert_eq!(
            a.pitch_frames.iter().map(|f| f.midi).collect::<Vec<_>>(),
            b.pitch_frames.iter().map(|f| f.midi).collect::<Vec<_>>()
        );
        assert_eq!(a.chart, b.chart);
        assert!(b.words.is_none() && b.lyrics.is_none());
        assert!(
            b.note_evidence
                .iter()
                .all(|n| n.uncertain && n.models.len() == 2)
        );
        assert!(
            b.issues
                .iter()
                .all(|i| i.reason != vg::Reason::MissingLyrics)
        );
        let c = b.chart.unwrap();
        assert!(
            !c.notes.is_empty()
                && c.notes
                    .iter()
                    .all(|n| n.lyric == crate::vocals::Syllable::Missing)
        );
        let midi = crate::vocals::append_midi(
            &crate::midi::new_midi(
                &crate::timing::TempoMap::constant(120.).unwrap(),
                &Default::default(),
            )
            .unwrap(),
            &c,
        )
        .unwrap();
        let back = crate::vocals::import_midi(&midi).unwrap();
        assert_eq!(c.notes, back.notes);
        assert_eq!(c.phrases, back.phrases);
    }
    #[test]
    fn short_gaps_need_energy_and_independent_pitch_throughout() {
        let (mut g, mut e) = fixture();
        for r in &mut e.frames[20..30] {
            r.midi = None;
            r.note_band_mass[24] = 0.1;
        }
        let filled = decode(&g, &e, &|| false).unwrap();
        assert!(filled.pitch_frames.iter().all(|f| f.midi == Some(60.)));
        for f in &mut g.pitch_frames {
            if (0.24..0.26).contains(&f.time_seconds) {
                f.local_rms = Some(0.);
            }
        }
        let silent = decode(&g, &e, &|| false).unwrap();
        assert!(
            silent
                .pitch_frames
                .iter()
                .any(|f| (0.24..0.26).contains(&f.time_seconds) && f.midi.is_none())
        );
        let (mut g, _) = fixture();
        g.rmvpe_pitch.as_mut().unwrap().frames[25].frequency_hz = Some(440.);
        let disagreement = decode(&g, &e, &|| false).unwrap();
        assert!(
            disagreement
                .pitch_frames
                .iter()
                .any(|f| (0.24..0.26).contains(&f.time_seconds) && f.midi.is_none())
        );
    }
    #[test]
    fn cancellation_and_source_mismatch_cannot_publish_or_mutate_a_draft() {
        let (g, mut e) = fixture();
        let before = serde_json::to_vec(&g).unwrap();
        assert!(decode(&g, &e, &|| true).is_err());
        e.source_sha256 = "f".repeat(64);
        assert!(decode(&g, &e, &|| false).is_err());
        assert_eq!(before, serde_json::to_vec(&g).unwrap());
    }
}
