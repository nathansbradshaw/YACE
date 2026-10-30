//! Source-bound coverage observations become pending review prompts, not edits.
//! No filesystem or signal analysis occurs here: the analyzer attests its input
//! hashes; file callers must verify that their current inputs still match them.
use crate::{
    review_regions::{Issue, Policy, Reason, ReviewMap},
    stem_coverage::{CoverageReport, PitchSupport},
    stem_review::StemId,
};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn time(value: f64, duration: f64) -> bool {
    value.is_finite() && (0.0..=duration + 1e-9).contains(&value)
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6
}
fn valid_pitch(pitch: &PitchSupport) -> bool {
    pitch.midi.is_finite()
        && (0.0..=127.0).contains(&pitch.midi)
        && pitch.periodicity.is_finite()
        && (0.0..=1.0 + 1e-9).contains(&pitch.periodicity)
        && pitch.fundamental_fraction.is_finite()
        && (0.0..=1.0 + 1e-9).contains(&pitch.fundamental_fraction)
        && pitch.competitor_fundamental_ratio.is_finite()
        && pitch.competitor_fundamental_ratio >= 0.0
}
fn stable_pitch(pitch: &PitchSupport) -> bool {
    pitch.periodicity >= 0.94
        && pitch.fundamental_fraction >= 0.12
        && pitch.competitor_fundamental_ratio <= 0.67
        && (pitch.midi - pitch.midi.round()).abs() <= 0.30 + 1e-9
}

fn validate(report: &CoverageReport, duration: f64) -> Result<()> {
    ensure!(
        report.version == 1 && duration.is_finite() && duration > 0.0,
        "Invalid coverage version or duration"
    );
    report.policy.validate()?;
    report
        .manifest
        .validate_binding(&report.source_audio_sha256, duration)?;
    ensure!(
        (duration - report.manifest.frames as f64 / report.manifest.sample_rate as f64).abs()
            <= 1.0 / report.manifest.sample_rate as f64,
        "Coverage duration must match the manifest sample clock"
    );
    ensure!(
        valid_hash(&report.source_audio_sha256)
            && valid_hash(&report.chart_sha256)
            && report
                .transcription_sha256
                .as_ref()
                .is_none_or(|h| valid_hash(h)),
        "Invalid coverage input hashes"
    );
    let mut stems = BTreeSet::new();
    for track in &report.tracks {
        let expected = match track.stem {
            StemId::Bass => "ExpertDoubleBass",
            StemId::Drums => "ExpertDrums",
            StemId::Guitar => "ExpertSingle",
            _ => anyhow::bail!("Unsupported coverage instrument"),
        };
        ensure!(
            track.expert_track == expected
                && stems.insert(track.stem)
                && report.manifest.ids().contains(&track.stem),
            "Coverage track is duplicate or does not match its bound stem"
        );
        ensure!(
            track.stem == StemId::Bass || track.alignment.is_empty(),
            "Coverage onset/pitch observations currently belong to bass only"
        );
        let mut tiers = BTreeSet::new();
        for tier in &track.tiers {
            ensure!(
                ["Easy", "Medium", "Hard"].iter().any(|prefix| tier.track
                    == format!("{prefix}{}", expected.trim_start_matches("Expert")))
                    && tiers.insert(&tier.track),
                "Invalid Expert-relative tier summary"
            );
        }
        for value in [
            track.timing.median_audio_minus_chart_seconds,
            track.timing.median_raw_minus_chart_seconds,
        ] {
            ensure!(
                value.is_none_or(|v| v.is_finite() && v.abs() <= duration + 0.2),
                "Invalid coverage timing summary"
            );
        }
        ensure!(
            track
                .timing
                .p90_absolute_audio_minus_chart_seconds
                .is_none_or(|v| time(v, duration)),
            "Invalid absolute timing summary"
        );
        for candidate in &track.candidates {
            ensure!(
                time(candidate.seconds, duration)
                    && time(candidate.gap_start, duration)
                    && time(candidate.gap_end, duration)
                    && candidate.gap_start <= candidate.seconds
                    && candidate.seconds <= candidate.gap_end
                    && candidate.gap_end > candidate.gap_start
                    && candidate.flux.is_finite()
                    && candidate.flux >= 0.0
                    && candidate.rms.is_finite()
                    && candidate.rms >= 0.0
                    && candidate.envelope_ratio.is_finite()
                    && candidate.envelope_ratio >= 0.0
                    && candidate
                        .nearest_expert_attack_seconds
                        .is_none_or(|v| v.is_finite() && v >= 0.0)
                    && candidate.pitch.as_ref().is_none_or(valid_pitch),
                "Invalid coverage attack candidate"
            );
            ensure!(
                !candidate.transcription_event_present || report.transcription_sha256.is_some(),
                "Coverage event observations require a transcription hash"
            );
            ensure!(
                track.stem == StemId::Bass
                    || (candidate.pitch.is_none() && candidate.proposed_bass_event.is_none()),
                "Only bass coverage can propose an unapproved raw pitch event"
            );
            if let Some(event) = &candidate.proposed_bass_event {
                ensure!(
                    candidate
                        .pitch
                        .as_ref()
                        .is_some_and(|p| close(p.midi.round(), event.pitch as f64))
                        && !candidate.transcription_event_present
                        && event.confidence == 0.0
                        && close(event.start, candidate.seconds)
                        && time(event.start, duration)
                        && time(event.end, duration)
                        && event.end > event.start
                        && event.end <= candidate.gap_end + 1e-9
                        && event.end - event.start <= 0.30 + 1e-9,
                    "Coverage pitch proposals must remain zero-confidence bounded listening windows"
                );
            }
        }
        for observation in &track.alignment {
            if let Some(window) = &observation.pitch_window {
                ensure!(
                    time(window.start, duration)
                        && time(window.end, duration)
                        && window.end > window.start
                        && time(window.candidate_end, duration)
                        && window
                            .competing_candidate_start
                            .is_none_or(|value| time(value, duration))
                        && observation.raw_transcription_seconds.is_some()
                        && window.owned
                            == (window.start >= observation.raw_transcription_seconds.unwrap()
                                && window.end <= window.candidate_end
                                && window
                                    .competing_candidate_start
                                    .is_none_or(|time| time >= window.end))
                        && (window.owned || observation.proposed_attack_tick.is_none()),
                    "Invalid pitch-window ownership evidence"
                );
            }
            // A source chart may have a negative offset. Such pre-audio attacks
            // are not invalid timestamps; ReviewMap clips a supported span to
            // the recording instead of inventing an offset correction.
            ensure!(
                observation.seconds.is_finite()
                    && observation.seconds <= duration + 0.1
                    && observation
                        .nearest_flux_seconds
                        .is_none_or(|v| time(v, duration))
                    && observation
                        .audio_minus_chart_seconds
                        .is_none_or(f64::is_finite)
                    && observation
                        .raw_transcription_seconds
                        .is_none_or(|v| time(v, duration))
                    && observation.raw_transcription_pitch.is_none_or(|p| p <= 127)
                    && observation
                        .raw_minus_chart_seconds
                        .is_none_or(f64::is_finite)
                    && observation.periodic_pitch.as_ref().is_none_or(valid_pitch)
                    && observation
                        .periodic_minus_raw_semitones
                        .is_none_or(f64::is_finite),
                "Invalid coverage alignment observation"
            );
            match (
                observation.nearest_flux_seconds,
                observation.audio_minus_chart_seconds,
            ) {
                (Some(peak), Some(residual)) => ensure!(
                    close(peak - observation.seconds, residual),
                    "Inconsistent stem timing residual"
                ),
                (None, None) => ensure!(
                    !observation.supported_attack && observation.proposed_attack_tick.is_none(),
                    "Supported attack requires an observed flux peak"
                ),
                _ => anyhow::bail!("Incomplete stem timing observation"),
            }
            match (
                observation.raw_transcription_seconds,
                observation.raw_transcription_pitch,
                observation.raw_minus_chart_seconds,
            ) {
                (Some(raw), Some(_), Some(residual)) => ensure!(
                    report.transcription_sha256.is_some()
                        && close(raw - observation.seconds, residual),
                    "Inconsistent or unbound raw transcription observation"
                ),
                (None, None, None) => (),
                _ => anyhow::bail!("Incomplete raw transcription observation"),
            }
            if let Some(error) = observation.periodic_minus_raw_semitones {
                ensure!(
                    observation
                        .periodic_pitch
                        .as_ref()
                        .zip(observation.raw_transcription_pitch)
                        .is_some_and(|(p, raw)| close(p.midi - raw as f64, error)),
                    "Inconsistent periodic/raw pitch disagreement"
                );
            }
        }
    }
    Ok(())
}

impl CoverageReport {
    pub fn review_map(&self, duration: f64) -> Result<ReviewMap> {
        review_map_with_inputs(self, duration, BTreeMap::new())
    }
}

/// Extra hashes identify actual serialized files (for example an audit CLI's
/// report/manifest bytes). They cannot override any source identity supplied by
/// the analyzer. Both semantic and byte-level identities then bind region IDs.
pub fn review_map_with_inputs(
    report: &CoverageReport,
    duration: f64,
    extra: BTreeMap<String, String>,
) -> Result<ReviewMap> {
    validate(report, duration)?;
    let mut inputs = BTreeMap::from([
        ("original_audio".into(), report.source_audio_sha256.clone()),
        ("chart".into(), report.chart_sha256.clone()),
        (
            "stem_manifest_semantic".into(),
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&report.manifest)?)
            ),
        ),
        (
            "stem_coverage_semantic".into(),
            format!("{:x}", Sha256::digest(serde_json::to_vec(report)?)),
        ),
    ]);
    if let Some(hash) = &report.transcription_sha256 {
        inputs.insert("transcription".into(), hash.clone());
    }
    if let Some(identity) = &report.manifest.generation {
        inputs.insert("separation_model".into(), identity.model_sha256.clone());
    }
    for asset in &report.manifest.stems {
        inputs.insert(format!("stem:{}", asset.id.as_str()), asset.sha256.clone());
    }
    for (name, hash) in extra {
        ensure!(
            !name.trim().is_empty() && valid_hash(&hash) && !inputs.contains_key(&name),
            "Extra coverage hashes cannot replace source identities"
        );
        ensure!(
            !matches!(name.as_str(), "transcription" | "separation_model")
                && !name.starts_with("stem:"),
            "Extra coverage hashes cannot invent reserved source identities"
        );
        inputs.insert(name, hash);
    }
    let mut issues = vec![];
    let mut checks = BTreeSet::new();
    for track in &report.tracks {
        checks.insert(Reason::UnchartedStemAttack);
        if track.stem == StemId::Bass {
            checks.extend([Reason::StemAttackTiming, Reason::PitchDisagreement]);
        }
        for c in &track.candidates {
            // A reported possibility remains a possibility: demand its explicit
            // review flag and basic measured support, never insert the proposal.
            let supported = c.flux > 0.0
                && c.rms >= report.policy.minimum_rms
                && c.envelope_ratio >= 1.10
                && c.gap_end - c.gap_start + 1e-9 >= report.policy.long_gap_seconds
                && c.nearest_expert_attack_seconds
                    .is_none_or(|distance| distance > report.policy.attack_tolerance_seconds)
                && (track.stem != StemId::Bass || c.pitch.as_ref().is_some_and(stable_pitch));
            if !c.review_required || !supported {
                continue;
            }
            let mut issue = Issue::at(
                c.seconds,
                &track.expert_track,
                Reason::UnchartedStemAttack,
                &[
                    ("source_tick", c.tick as f64),
                    ("stem_flux_strength", c.flux as f64),
                    ("stem_rms", c.rms),
                    ("envelope_ratio", c.envelope_ratio),
                    ("gap_start_seconds", c.gap_start),
                    ("gap_end_seconds", c.gap_end),
                    (
                        "raw_event_present",
                        if c.transcription_event_present {
                            1.0
                        } else {
                            0.0
                        },
                    ),
                ],
            );
            issue.end = (c.seconds + 0.30).min(c.gap_end).min(duration);
            if let Some(distance) = c.nearest_expert_attack_seconds {
                issue
                    .evidence
                    .insert("nearest_expert_attack_distance_seconds".into(), distance);
            }
            if let Some(pitch) = &c.pitch {
                issue
                    .evidence
                    .insert("periodic_midi_pitch".into(), pitch.midi);
                issue
                    .evidence
                    .insert("periodicity".into(), pitch.periodicity);
            }
            issues.push(issue);
        }
        for o in &track.alignment {
            if o.review_required
                && o.supported_attack
                && o.audio_minus_chart_seconds
                    .is_some_and(|r| r.abs() > 0.050 + 1e-9)
            {
                let peak = o.nearest_flux_seconds.expect("validated supported peak");
                let residual = o.audio_minus_chart_seconds.unwrap();
                let mut issue = Issue::at(
                    o.seconds.min(peak),
                    &track.expert_track,
                    Reason::StemAttackTiming,
                    &[
                        ("chart_tick", o.tick as f64),
                        ("chart_attack_seconds", o.seconds),
                        ("stem_flux_seconds", peak),
                        ("audio_minus_chart_seconds", residual),
                        ("absolute_residual_seconds", residual.abs()),
                        ("threshold_seconds", 0.050),
                    ],
                );
                issue.end = o.seconds.max(peak).min(duration);
                if let Some(tick) = o.proposed_attack_tick {
                    issue
                        .evidence
                        .insert("proposed_tick_unapproved".into(), tick as f64);
                }
                issues.push(issue);
            }
            if o.review_required
                && o.periodic_minus_raw_semitones
                    .is_some_and(|error| error.abs() > 1.0 + 1e-9)
                && o.periodic_pitch.as_ref().is_some_and(stable_pitch)
            {
                let pitch = o.periodic_pitch.as_ref().unwrap();
                let raw = o
                    .raw_transcription_pitch
                    .expect("validated pitch comparison");
                let error = o.periodic_minus_raw_semitones.unwrap();
                let mut issue = Issue::at(
                    o.seconds,
                    &track.expert_track,
                    Reason::PitchDisagreement,
                    &[
                        ("chart_tick", o.tick as f64),
                        ("raw_midi_pitch", raw as f64),
                        ("periodic_midi_pitch", pitch.midi),
                        ("periodic_minus_raw_semitones", error),
                        ("absolute_error_semitones", error.abs()),
                        ("threshold_semitones", 1.0),
                        ("periodicity", pitch.periodicity),
                        ("fundamental_fraction", pitch.fundamental_fraction),
                        (
                            "competitor_fundamental_ratio",
                            pitch.competitor_fundamental_ratio,
                        ),
                    ],
                );
                issue.end = (o.seconds + 0.30).min(duration);
                if let Some(window) = &o.pitch_window {
                    issue.evidence.insert(
                        "pitch_window_owned".into(),
                        if window.owned { 1. } else { 0. },
                    );
                    issue
                        .evidence
                        .insert("pitch_window_start_seconds".into(), window.start);
                    issue
                        .evidence
                        .insert("pitch_window_end_seconds".into(), window.end);
                    issue
                        .evidence
                        .insert("candidate_end_seconds".into(), window.candidate_end);
                    if let Some(other) = window.competing_candidate_start {
                        issue
                            .evidence
                            .insert("competing_candidate_start_seconds".into(), other);
                    }
                }
                issues.push(issue);
            }
        }
    }
    let mut map = ReviewMap::build(duration, Policy::default(), inputs, None, checks, issues)?;
    map.coverage_limits = report.limits.clone();
    map.coverage_limits.extend([
        "Coverage markers are pending listening prompts bound to the supplied source/chart/events/stem hashes. No chart notes, easier tiers, decisions or approvals were changed. Uncharted attacks may be bleed, sustain coverage or deliberate arrangement omissions.".into(),
        "Stem timing uses supported spectral-flux observations, not physical onset truth or measured playback latency. A >50ms residual does not authorize a shift; pitch disagreements compare periodic audio with a raw MIDI event, never an absolute pitch encoded by a five-fret lane.".into(),
        "Semantic report/manifest hashes identify their represented data. Extra file hashes, when supplied, identify original serialized bytes separately. Conversion does not reread audio or re-establish model, sample/phase alignment or musical accuracy.".into(),
    ]);
    Ok(map)
}

/// Combine pending diagnostics while retaining the base's genuine generation
/// scope. The caller supplies the hash of the exact serialized base file; it
/// participates in region IDs before construction. Conflicting evidence fails.
pub fn merge_into_base(
    derived: &ReviewMap,
    base: &ReviewMap,
    base_file_sha256: &str,
) -> Result<ReviewMap> {
    ensure!(
        derived.version == 1 && base.version == 1,
        "Unsupported base or derived review version"
    );
    ensure!(
        derived.duration_seconds.is_finite()
            && base.duration_seconds.is_finite()
            && (derived.duration_seconds - base.duration_seconds).abs() <= 1. / 44100.,
        "Base review duration differs from the coverage source clock"
    );
    ensure!(
        valid_hash(base_file_sha256),
        "Base review file must have a SHA-256 identity"
    );
    for name in ["chart", "original_audio"] {
        let expected = derived
            .input_sha256
            .get(name)
            .filter(|value| valid_hash(value))
            .ok_or_else(|| anyhow::anyhow!("Coverage lacks valid {name} identity"))?;
        let actual = base
            .input_sha256
            .get(name)
            .filter(|value| valid_hash(value))
            .ok_or_else(|| anyhow::anyhow!("Base review lacks valid {name} identity"))?;
        ensure!(
            expected.eq_ignore_ascii_case(actual),
            "Base review belongs to different {name}"
        );
    }
    let mut inputs = BTreeMap::new();
    for (name, hash) in base.input_sha256.iter().chain(&derived.input_sha256) {
        ensure!(
            !name.trim().is_empty() && valid_hash(hash),
            "Invalid review evidence hash"
        );
        let normalized = hash.to_ascii_lowercase();
        if let Some(previous) = inputs.insert(name.clone(), normalized.clone()) {
            ensure!(
                previous == normalized,
                "Conflicting base/coverage evidence hash: {name}"
            );
        }
    }
    let base_key = "base_review_markers_file".to_owned();
    if let Some(previous) = inputs.insert(base_key, base_file_sha256.to_ascii_lowercase()) {
        ensure!(
            previous.eq_ignore_ascii_case(base_file_sha256),
            "Conflicting base review file identity"
        );
    }
    let checks = base
        .checks_performed
        .union(&derived.checks_performed)
        .copied()
        .collect();
    let issues = base
        .regions
        .iter()
        .chain(&derived.regions)
        .flat_map(|region| region.issues.iter().cloned())
        .collect();
    let mut merged = ReviewMap::build(
        base.duration_seconds,
        base.policy.clone(),
        inputs,
        base.generated_tracks.clone(),
        checks,
        issues,
    )?;
    let mut limits = BTreeSet::new();
    merged.coverage_limits = base
        .coverage_limits
        .iter()
        .chain(&derived.coverage_limits)
        .filter(|limit| limits.insert((*limit).clone()))
        .cloned()
        .collect();
    merged.coverage_limits.push("Combined review preserves the base file's actual generated-track provenance and existing issues, with new source-bound stem diagnostics. All rebuilt regions remain pending; no prior human/AI decision or musical approval is inherited.".into());
    Ok(merged)
}

/// Bounds checks for the audit CLI before expensive decoding. Preserve unknown
/// source controls and legitimate negative chart offsets; reject overflows and
/// playable events extending beyond the recording tolerance used by generation.
pub fn validate_chart_events(
    chart: &crate::chart::Chart,
    events: Option<&crate::mapping::Transcription>,
    duration: f64,
) -> Result<()> {
    ensure!(
        duration.is_finite() && duration > 0.0,
        "Invalid audit duration"
    );
    for section in &chart.sections {
        if !["Easy", "Medium", "Hard", "Expert"].iter().any(|tier| {
            ["Single", "DoubleBass", "Drums"]
                .iter()
                .any(|part| section.name == format!("{tier}{part}"))
        }) {
            continue;
        }
        for note in chart.notes(&section.name)? {
            let end = note
                .tick
                .checked_add(note.sustain)
                .ok_or_else(|| anyhow::anyhow!("Audit source note tick overflow"))?;
            let playable = note.lane <= 4
                || if section.name.ends_with("Drums") {
                    note.lane == 32
                } else {
                    note.lane == 7
                };
            ensure!(
                !playable
                    || (chart.timing.seconds(end).is_finite()
                        && chart.timing.seconds(end) <= duration + 0.1),
                "Audit source note extends beyond recording"
            );
        }
    }
    if let Some(events) = events {
        events.validate()?;
        ensure!(
            events.drums.iter().all(|n| n.time <= duration + 0.1)
                && events
                    .bass
                    .iter()
                    .chain(&events.guitar)
                    .all(|n| n.end <= duration + 0.1),
            "Audit transcription event extends beyond recording"
        );
    }
    Ok(())
}
