//! Guitar-only source-supported short-note proposals.
use super::{
    PitchActivations,
    guitar_harmonics::{RATE, fit, frequency, wrap},
};
use crate::mapping::PitchCandidateEvidence;
use std::{collections::BTreeMap, f64::consts::TAU};
type Recovery = (
    BTreeMap<String, usize>,
    Vec<PitchCandidateEvidence>,
    Vec<crate::mapping::GuitarPolyphonicNote>,
);
pub(super) fn recover(
    input: &PitchActivations,
    audio: &[f32],
    baseline: &[PitchCandidateEvidence],
    hypotheses: &[crate::mapping::ShortPitchCandidate],
) -> Recovery {
    let mut counts = BTreeMap::new();
    let mut proposed: Vec<PitchCandidateEvidence> = Vec::new();
    let mut receipts = Vec::new();
    let peaks = crate::review::attacks(&input.audio);
    for short in hypotheses.iter().filter(|s| {
        !s.retained && s.evidence.onset_score >= 0.5 && s.evidence.candidate.confidence >= 0.3
    }) {
        let e = &short.evidence;
        let n = &e.candidate;
        let start = n.start + 0.003;
        let end = baseline
            .iter()
            .map(|e| &e.candidate)
            .chain(hypotheses.iter().map(|s| &s.evidence.candidate))
            .filter(|other| other.start > n.start + 0.025)
            .map(|other| other.start - 0.003)
            .fold(n.end - 0.003, f64::min);
        let centers = [start + 0.016, end - 0.016];
        let mut reason = "supported_joint_harmonics";
        let mut receipt = None;
        if !matches!(
            short.reason.as_str(),
            "network_support_not_strong" | "unstable_source_pitch" | "source_pitch_disagrees"
        ) {
            reason = "legacy_prior_gate";
        } else if baseline
            .iter()
            .chain(&proposed)
            .any(|e| (e.candidate.start - n.start).abs() < 0.055)
        {
            reason = "near_existing_attack";
        } else if !(40..=88).contains(&n.pitch) || n.end - n.start < 0.055 {
            reason = "outside_candidate_policy";
        } else if crate::review::nearest(n.start, &peaks)
            .is_none_or(|peak| (peak.time - n.start).abs() > 0.035)
        {
            reason = "no_source_attack";
        } else if end - start < 0.045 || (end - start) * frequency(n.pitch) < 3.5 {
            reason = "insufficient_owned_window";
        } else {
            let active: Vec<_> = baseline
                .iter()
                .chain(hypotheses.iter().map(|s| &s.evidence))
                .filter(|c| {
                    c.candidate.confidence >= 0.3
                        && c.candidate.start <= start + 0.025
                        && c.candidate.end >= end - 0.005
                })
                .map(|c| c.candidate.pitch)
                .collect();
            let mut frequencies = Vec::new();
            for pitch in active.iter().copied().chain(std::iter::once(n.pitch)) {
                for harmonic in 1..=3 {
                    let f = frequency(pitch) * harmonic as f64;
                    if !frequencies.iter().any(|old: &f64| (*old - f).abs() < 0.2) {
                        frequencies.push(f);
                    }
                }
            }
            for ratio in [0.5, 1. / 3., 0.25] {
                let f = frequency(n.pitch) * ratio;
                if !frequencies.iter().any(|old: &f64| (*old - f).abs() < 0.2) {
                    frequencies.push(f);
                }
            }
            if frequencies.len() > 18 {
                reason = "crowded_model_voicing";
            } else if active
                .iter()
                .any(|p| [12, 19, 24, 28, 31, 36].contains(&(n.pitch as i16 - *p as i16)))
            {
                reason = "lower_model_harmonic";
            } else if let Some(fits) = centers
                .iter()
                .map(|center| fit(audio, *center, &frequencies))
                .collect::<Option<Vec<_>>>()
            {
                let f = frequencies
                    .iter()
                    .position(|f| (*f - frequency(n.pitch)).abs() < 0.2)
                    .unwrap();
                let power = fits
                    .iter()
                    .map(|fit| fit.power[f])
                    .fold(f64::INFINITY, f64::min);
                let delta = wrap(
                    fits[1].phase[f]
                        - fits[0].phase[f]
                        - TAU * frequency(n.pitch) * (centers[1] - centers[0]),
                ) / (TAU * (centers[1] - centers[0]));
                let reduced: Vec<_> = frequencies
                    .iter()
                    .copied()
                    .filter(|hz| (*hz - frequency(n.pitch)).abs() >= 0.2)
                    .collect();
                let improvement = centers
                    .iter()
                    .zip(&fits)
                    .map(|(center, full)| {
                        fit(audio, *center, &reduced)
                            .map(|reduced| full.explained - reduced.explained)
                            .unwrap_or(0.)
                    })
                    .fold(f64::INFINITY, f64::min);
                let subpower = fits
                    .iter()
                    .map(|fit| {
                        frequencies
                            .iter()
                            .enumerate()
                            .filter(|(_, hz)| {
                                [0.5, 1. / 3., 0.25]
                                    .iter()
                                    .any(|ratio| (**hz - frequency(n.pitch) * ratio).abs() < 0.2)
                            })
                            .map(|(i, _)| fit.power[i])
                            .fold(0., f64::max)
                    })
                    .fold(0., f64::max);
                let before = fit(audio, n.start - 0.04, &frequencies);
                let prior_lo = ((n.start - 0.056).max(0.) * RATE).round() as usize;
                let prior_hi = ((n.start - 0.024).max(0.) * RATE).round() as usize;
                let quiet_prior =
                    if n.start < 0.056 || prior_hi > audio.len() || prior_hi <= prior_lo {
                        false
                    } else {
                        let samples = &audio[prior_lo..prior_hi];
                        let mean =
                            samples.iter().map(|s| *s as f64).sum::<f64>() / samples.len() as f64;
                        samples
                            .iter()
                            .map(|s| (*s as f64 - mean).powi(2))
                            .sum::<f64>()
                            / (samples.len() as f64)
                            < 1e-9
                    };
                if power < 0.000064 {
                    reason = "weak_owned_fundamental";
                } else if improvement < 0.10 {
                    reason = "target_not_independently_needed";
                } else if subpower * 4. > power {
                    reason = "acoustic_lower_fundamental";
                } else if delta.abs() > frequency(n.pitch) * 0.021 {
                    reason = "source_frequency_disagrees";
                } else if centers.iter().any(|center| {
                    let best = (-8i32..=8)
                        .filter_map(|step| {
                            let mut alternative = reduced.clone();
                            alternative.push(frequency(n.pitch) * 2f64.powf(step as f64 / 48.));
                            fit(audio, *center, &alternative).map(|fit| (step, fit.explained))
                        })
                        .max_by(|a, b| a.1.total_cmp(&b.1));
                    best.is_none_or(|(step, _)| step.abs() > 1)
                }) {
                    // A short window's phase advance can wrap. Search a
                    // bounded +/-2 semitone neighborhood independently;
                    // do not mistake a wrapped neighboring tone for f0.
                    reason = "neighbor_frequency_fits_better";
                } else if let Some(before) = before {
                    if before.power[f] >= power * 0.25 {
                        reason = "source_already_sounding";
                    } else if before.power[f] >= 1e-7 {
                        let jump = wrap(
                            fits[0].phase[f]
                                - before.phase[f]
                                - TAU
                                    * (frequency(n.pitch) + delta)
                                    * (centers[0] - (n.start - 0.04)),
                        )
                        .abs();
                        if jump < 1. {
                            reason = "continuous_phase_rise";
                        }
                    }
                } else if !quiet_prior {
                    reason = "unavailable_source_continuity_fit";
                }
                if reason == "supported_joint_harmonics" && fits.iter().any(|f| f.explained < 0.8) {
                    reason = "otherwise_supported_but_partial_source_fit";
                }
                if matches!(
                    reason,
                    "supported_joint_harmonics" | "otherwise_supported_but_partial_source_fit"
                ) {
                    receipt = Some(crate::mapping::GuitarPolyphonicNote {
                        evidence: e.clone(),
                        original_rejection: short.reason.clone(),
                        source_attack_seconds: crate::review::nearest(n.start, &peaks)
                            .unwrap()
                            .time,
                        window_start: start,
                        window_end: end,
                        minimum_fundamental_power: power,
                        minimum_target_variance_gain: improvement,
                        minimum_source_explained: fits
                            .iter()
                            .map(|f| f.explained)
                            .fold(f64::INFINITY, f64::min),
                        maximum_subharmonic_power: subpower,
                        frequency_offset_hz: delta,
                        preexisting_power_fraction: fit(audio, n.start - 0.04, &frequencies)
                            .map(|before| before.power[f] / power),
                    });
                }
            } else {
                reason = "singular_fit";
            }
        }
        if matches!(
            reason,
            "supported_joint_harmonics" | "otherwise_supported_but_partial_source_fit"
        ) {
            proposed.push(e.clone());
            receipts.push(receipt.expect("Accepted acoustic evidence"));
        }
        *counts.entry(reason.into()).or_default() += 1;
    }
    (counts, proposed, receipts)
}
