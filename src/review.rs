//! Signal checks and synchronized listening material. These are
//! diagnostics, never a substitute for listening or reference annotations.
use crate::{
    chart::{Chart, Note},
    mapping::Transcription,
    models, pipeline,
    review_regions::{Issue, Policy, Reason, ReviewMap},
};
use anyhow::{Context, Result, ensure};
use pitch_detection::detector::{PitchDetector, yin::YINDetector};
use rustfft::{FftPlanner, num_complex::Complex};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};

#[derive(Clone, Debug, Serialize)]
pub struct Attack {
    pub time: f64,
    pub strength: f32,
}

/// Positive spectral flux, independent of either transcription network.
/// 1024-sample centered Hann windows, 220-sample hops at 22.05 kHz.
pub fn attacks(audio: &[f32]) -> Vec<Attack> {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(1024);
    let mut previous = vec![0.0f32; 513];
    let mut flux = Vec::new();
    let mut buffer = vec![Complex::new(0.0, 0.0); 1024];
    for center in (0..audio.len()).step_by(220) {
        for (i, value) in buffer.iter_mut().enumerate() {
            let index = center as isize + i as isize - 512;
            let sample = if index >= 0 {
                audio.get(index as usize).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            *value = Complex::new(
                sample * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / 1023.0).cos()),
                0.0,
            );
        }
        fft.process(&mut buffer);
        let mut value = 0.0;
        for bin in 2..465 {
            let magnitude = (1.0 + buffer[bin].norm() * 10.0).ln();
            value += (magnitude - previous[bin]).max(0.0);
            previous[bin] = magnitude;
        }
        flux.push(value);
    }
    let mut peaks = Vec::new();
    let global = flux.iter().copied().fold(0.0, f32::max);
    for i in 2..flux.len().saturating_sub(2) {
        let lo = i.saturating_sub(20);
        let hi = (i + 21).min(flux.len());
        let mean = flux[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        if flux[i] > mean * 1.35
            && flux[i] > global * 0.025
            && flux[i] > flux[i - 1]
            && flux[i] >= flux[i + 1]
            && flux[i] >= flux[i - 2]
            && flux[i] >= flux[i + 2]
        {
            peaks.push(Attack {
                time: i as f64 * 220.0 / 22050.0,
                strength: flux[i],
            });
        }
    }
    peaks
}

pub fn nearest(time: f64, peaks: &[Attack]) -> Option<&Attack> {
    let index = peaks.partition_point(|p| p.time < time);
    [index.checked_sub(1), Some(index)]
        .into_iter()
        .flatten()
        .filter_map(|i| peaks.get(i))
        .min_by(|a, b| (a.time - time).abs().total_cmp(&(b.time - time).abs()))
}

#[derive(Debug, Serialize)]
pub struct BassCheck {
    pub time: f64,
    pub predicted: u8,
    pub periodic_midi: f64,
    pub semitone_error: f64,
    pub clarity: f64,
}

pub fn bass_checks(events: &Transcription, audio: &[f32]) -> Vec<BassCheck> {
    let mut detector = YINDetector::<f64>::new(2048, 1024);
    let mut results = Vec::new();
    for note in &events.bass {
        if note.end - note.start < 0.20 {
            continue;
        }
        // Sample a stable part of the note. Short notes/weak periodicity are
        // left unjudged; a monophonic detector is not a polyphonic oracle.
        let center = ((note.start + (note.end - note.start).min(0.4) * 0.5) * 11025.0) as usize;
        if center < 1024 || center + 1024 > audio.len() {
            continue;
        }
        let signal: Vec<f64> = audio[center - 1024..center + 1024]
            .iter()
            .map(|v| *v as f64)
            .collect();
        let Some(pitch) = detector.get_pitch(&signal, 11025, 0.01, 0.85) else {
            continue;
        };
        if !(35.0..500.0).contains(&pitch.frequency) {
            continue;
        }
        // pitch-detection 0.3.0 normalizes its YIN clarity against a negative
        // zero-lag value. Recompute 1-CMND at the chosen period instead of
        // treating the crate's potentially negative clarity as confidence.
        let clarity = crate::refine::periodicity(&signal, pitch.frequency, 11025.0);
        if clarity < 0.85 || !clarity.is_finite() {
            continue;
        }
        let midi = 69.0 + 12.0 * (pitch.frequency / 440.0).log2();
        results.push(BassCheck {
            time: note.start,
            predicted: note.pitch,
            periodic_midi: midi,
            semitone_error: midi - note.pitch as f64,
            clarity,
        });
    }
    results
}

#[derive(Debug, Serialize)]
struct TrackCheck {
    notes: usize,
    off_grid_over_35ms: usize,
    off_grid_times: Vec<f64>,
    same_lane_under_50ms: Vec<f64>,
    more_than_two_hand_hits_within_20ms: Vec<f64>,
}

fn track_check(notes: &[Note], chart: &Chart, drums: bool) -> TrackCheck {
    track_check_pitched(notes, chart, drums, false)
}
fn track_check_pitched(notes: &[Note], chart: &Chart, drums: bool, pro_keys: bool) -> TrackCheck {
    let mut notes: Vec<_> = notes
        .iter()
        .filter(|n| {
            if pro_keys {
                (48..=72).contains(&n.lane)
            } else {
                n.lane <= 4 || (!drums && n.lane == 7)
            }
        })
        .collect();
    notes.sort_by_key(|n| (n.tick, n.lane));
    let mut previous = BTreeMap::new();
    let mut fast = Vec::new();
    let mut crowded = Vec::new();
    let mut off_grid_times = Vec::new();
    for (i, note) in notes.iter().enumerate() {
        let time = chart.timing.seconds(note.tick);
        let resolution = chart.timing.resolution as f64;
        let residual = [4.0, 6.0]
            .into_iter()
            .map(|subdivision| {
                let snapped = (note.tick as f64 / (resolution / subdivision)).round()
                    * (resolution / subdivision);
                (chart.timing.seconds(snapped as u64) - time).abs()
            })
            .fold(f64::INFINITY, f64::min);
        if residual > 0.035 {
            off_grid_times.push(time);
        }
        if let Some(previous) = previous.insert(note.lane, time)
            && time - previous < 0.05
        {
            fast.push(time);
        }
        if drums && note.lane > 0 {
            let hands = notes[i..]
                .iter()
                .take_while(|n| chart.timing.seconds(n.tick) - time <= 0.020)
                .filter(|n| n.lane > 0)
                .count();
            if hands > 2 && crowded.last().is_none_or(|last| time - last > 0.02) {
                crowded.push(time);
            }
        }
    }
    TrackCheck {
        notes: notes.len(),
        off_grid_over_35ms: off_grid_times.len(),
        off_grid_times,
        same_lane_under_50ms: fast,
        more_than_two_hand_hits_within_20ms: crowded,
    }
}

fn all_track_checks(chart: &Chart) -> Result<BTreeMap<String, TrackCheck>> {
    let mut checks = BTreeMap::new();
    for tier in ["Easy", "Medium", "Hard", "Expert"] {
        for part in ["Single", "DoubleBass", "Drums", "Keyboard", "ProKeys"] {
            let name = format!("{tier}{part}");
            checks.insert(
                name.clone(),
                if part == "ProKeys" {
                    track_check_pitched(&chart.notes(&name)?, chart, false, true)
                } else {
                    track_check(&chart.notes(&name)?, chart, part == "Drums")
                },
            );
        }
    }
    Ok(checks)
}

fn pattern_issues(checks: &BTreeMap<String, TrackCheck>) -> Vec<Issue> {
    let mut issues = Vec::new();
    for (track, check) in checks {
        for (times, reason) in [
            (&check.off_grid_times, Reason::CoarseGridDeviation),
            (&check.same_lane_under_50ms, Reason::FastRepeat),
            (
                &check.more_than_two_hand_hits_within_20ms,
                Reason::HandCrowding,
            ),
        ] {
            for time in times {
                issues.push(Issue::at(*time, track, reason, &[]));
            }
        }
    }
    issues
}

fn generation_checks(has_origin: bool) -> BTreeSet<Reason> {
    let mut checks = BTreeSet::from([
        Reason::CoarseGridDeviation,
        Reason::FastRepeat,
        Reason::HandCrowding,
    ]);
    if has_origin {
        checks.insert(Reason::LowCandidateScore);
    }
    checks
}

/// No model loading or stem requirement: every export can carry this initial
/// review map. The richer `run` review adds signal evidence when stems exist.
pub fn generation_map(
    chart: &Chart,
    events: &Transcription,
    duration: f64,
    inputs: BTreeMap<String, String>,
    generated_tracks: BTreeSet<String>,
) -> Result<ReviewMap> {
    events.validate()?;
    let policy = Policy::default();
    let mut issues = candidate_score_issues(chart, events, Some(&generated_tracks), &policy)?;
    if events.guitar_source != crate::mapping::GuitarSource::Guitar
        && generated_tracks.contains("ExpertSingle")
    {
        for note in chart
            .notes("ExpertSingle")?
            .iter()
            .filter(|note| crate::editor::playable("ExpertSingle", note.lane))
        {
            issues.push(Issue::at(
                chart.timing.seconds(note.tick),
                "ExpertSingle",
                Reason::MelodySourceReview,
                &[("source_lane", note.lane as f64)],
            ));
        }
    }
    let pro_arrangement = crate::pro_keys::arrange_transcription(events, &chart.timing)?;
    issues.extend(pattern_issues(&all_track_checks(chart)?));
    for track in generated_tracks.iter().filter(|t| crate::keys::is_keys(t)) {
        for note in chart
            .notes(track)?
            .iter()
            .filter(|n| crate::editor::playable(track, n.lane))
        {
            let mut issue = Issue::at(
                chart.timing.seconds(note.tick),
                track,
                Reason::PianoSourceReview,
                &[("source_lane", note.lane as f64)],
            );
            issue.end = chart.timing.seconds(note.tick + note.sustain).min(duration);
            issues.push(issue);
        }
        if crate::keys::is_pro(track) {
            let actual = chart.notes(track)?;
            for decision in &pro_arrangement.decisions {
                let (Some(tick), Some(pitch), Some(release)) = (
                    decision.output_tick,
                    decision.target_pitch,
                    decision.output_sustain,
                ) else {
                    continue;
                };
                let changed_release = decision
                    .source_sustain
                    .is_some_and(|source| release < source);
                if (decision.octave_shift != 0
                    || changed_release
                    || matches!(
                        decision.disposition.as_str(),
                        "merged_simultaneous" | "merged_octave_collision"
                    ))
                    && actual
                        .iter()
                        .any(|note| note.tick == tick && note.lane == pitch)
                {
                    let mut issue = Issue::at(
                        chart.timing.seconds(tick),
                        track,
                        Reason::ProKeysArrangement,
                        &[
                            ("source_midi_pitch", decision.source.pitch as f64),
                            ("arranged_midi_pitch", pitch as f64),
                            ("octave_shift", decision.octave_shift as f64),
                            ("source_attack_seconds", decision.source.start),
                            ("arranged_attack_seconds", chart.timing.seconds(tick)),
                            (
                                "source_sustain_ticks",
                                decision.source_sustain.unwrap_or(0) as f64,
                            ),
                            ("arranged_sustain_ticks", release as f64),
                        ],
                    );
                    issue.end = chart.timing.seconds(tick + release).min(duration);
                    issues.push(issue);
                }
            }
            for uncovered in crate::key_ranges::plan(&chart.notes(track)?)?.uncovered {
                issues.push(Issue::at(
                    chart.timing.seconds(uncovered.tick),
                    track,
                    Reason::ProKeysRangeCoverage,
                    &[
                        ("uncovered_pitch_count", uncovered.pitches.len() as f64),
                        ("visible_low_midi", uncovered.visible_low as f64),
                        ("visible_high_midi", uncovered.visible_high as f64),
                    ],
                ));
            }
        }
    }
    let mut checks = generation_checks(true);
    checks.extend(issues.iter().map(|issue| issue.reason));
    if generated_tracks.contains("ExpertDoubleBass") && !events.bass_attack_reviews.is_empty() {
        checks.insert(Reason::WeakAttackSupport);
    }
    let mut map = ReviewMap::build(
        duration,
        policy,
        inputs,
        Some(generated_tracks.clone()),
        checks,
        issues,
    )?;
    map.coverage_limits.push("Generation markers retain candidate and bass-refinement evidence but perform no fresh stem analysis. Run review with aligned stems for additional attack and pitch checks. Pattern checks cover the supplied chart representation; MIDI enhancement supplies only newly generated tracks.".into());
    if generated_tracks.iter().any(|t| crate::keys::is_keys(t)) {
        map.coverage_limits.push("Piano-source presence is unverified: separation can leak other instruments. Piano/Pro Piano notes remain pending source/chord/release/controller review; model confidence does not establish an audible piano part.".into());
    }
    Ok(map)
}

#[cfg(not(target_arch = "wasm32"))]
fn encode_mp3(path: &Path, left: &[f32], right: &[f32]) -> Result<()> {
    let wav = path.with_extension("wav");
    models::write_audio(&wav, left, right)?;
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&wav)
        .args(["-c:a", "libmp3lame", "-b:a", "160k", "-n"])
        .arg(path)
        .output()?;
    ensure!(output.status.success(), "Cannot encode review audio");
    fs::remove_file(wav)?;
    Ok(())
}

fn drum_render(notes: &[Note], chart: &Chart, start: f64, length: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; length];
    let cymbals: BTreeSet<_> = notes
        .iter()
        .filter(|n| (66..=68).contains(&n.lane))
        .map(|n| (n.tick, n.lane - 64))
        .collect();
    for note in notes.iter().filter(|n| n.lane <= 4) {
        let onset = chart.timing.seconds(note.tick) - start;
        if onset < 0.0 || onset >= length as f64 / 44100.0 {
            continue;
        }
        let offset = (onset * 44100.0).round() as usize;
        let cymbal = cymbals.contains(&(note.tick, note.lane));
        let duration = if cymbal { 0.11 } else { 0.07 };
        let frequency = match note.lane {
            0 => 70.0,
            1 => 190.0,
            2 => {
                if cymbal {
                    6500.0
                } else {
                    260.0
                }
            }
            3 => {
                if cymbal {
                    4300.0
                } else {
                    180.0
                }
            }
            _ => {
                if cymbal {
                    5200.0
                } else {
                    120.0
                }
            }
        };
        let mut seed = 0x1234567u32;
        for i in 0..(duration * 44100.0) as usize {
            let Some(sample) = out.get_mut(offset + i) else {
                break;
            };
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let time = i as f32 / 44100.0;
            let wave = if cymbal || note.lane == 1 {
                noise * 0.6 + (std::f32::consts::TAU * frequency * time).sin() * 0.4
            } else {
                (std::f32::consts::TAU * frequency * time).sin()
            };
            *sample += wave * (-time / duration as f32 * 6.0).exp() * 0.45;
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Scores belong to generated candidates, not to preserved authored notes.
/// Even on generated tracks a nearby attack is only an approximate association.
fn candidate_score_issues(
    chart: &Chart,
    events: &Transcription,
    generated: Option<&BTreeSet<String>>,
    policy: &Policy,
) -> Result<Vec<Issue>> {
    let mut issues = Vec::new();
    let Some(generated) = generated else {
        return Ok(issues);
    };
    if generated.contains("ExpertSingle") && !events.guitar_short_candidates.is_empty() {
        let sources = crate::mapping::guitar_attack_sources(&events.guitar, &chart.timing);
        let notes = chart.notes("ExpertSingle")?;
        for recovered in events.guitar_short_candidates.iter().filter(|c| c.retained) {
            let candidate = &recovered.evidence.candidate;
            let played = sources.iter().any(|(tick, group)| {
                group.iter().any(|(lane, source)| {
                    source.start == candidate.start
                        && source.end == candidate.end
                        && source.pitch == candidate.pitch
                        && notes.iter().any(|n| n.tick == *tick && n.lane == *lane)
                })
            });
            if played {
                let mut issue = Issue::at(
                    candidate.start,
                    "ExpertSingle",
                    Reason::RecoveredShortNote,
                    &[
                        ("network_onset_score", recovered.evidence.onset_score as f64),
                        (
                            "source_attack_seconds",
                            recovered.source_attack_seconds.unwrap_or(candidate.start),
                        ),
                    ],
                );
                issue.end = candidate.end;
                issues.push(issue);
            }
        }
    }
    if generated.contains("ExpertDrums") {
        let actual = chart.notes("ExpertDrums")?;
        for (index, tick, voice) in crate::mapping::played_drum_articulations(events, &chart.timing)
        {
            let Some(evidence) = events
                .drum_articulations
                .iter()
                .find(|e| e.hit_index == index)
            else {
                continue;
            };
            if !actual
                .iter()
                .any(|note| note.tick == tick && note.lane == voice.lane())
            {
                continue;
            }
            let mut measurements = vec![
                ("proposal_margin_uncalibrated", evidence.confidence),
                (
                    "proposal_applied",
                    if evidence.proposed == Some(voice) {
                        1.
                    } else {
                        0.
                    },
                ),
                ("played_lane", voice.lane() as f64),
                (
                    "proposed",
                    if evidence.proposed.is_some() { 1. } else { 0. },
                ),
            ];
            if let Some(value) = evidence.resonance_hz {
                measurements.push(("resonance_hz", value));
            }
            if let Some(value) = evidence.spectral_peak_fraction {
                measurements.push(("spectral_peak_fraction", value));
            }
            let mut issue = Issue::at(
                chart.timing.seconds(tick),
                "ExpertDrums",
                Reason::DrumArticulation,
                &measurements,
            );
            issue.end = evidence
                .cymbal_model
                .as_ref()
                .map(|m| m.window_end)
                .or(evidence.window_end)
                .unwrap_or(evidence.time)
                .max(issue.start);
            issues.push(issue);
        }
    }
    if generated.contains("ExpertSingle") && !events.guitar_polyphonic_notes.is_empty() {
        let sources = crate::mapping::guitar_attack_sources(&events.guitar, &chart.timing);
        let notes = chart.notes("ExpertSingle")?;
        for recovered in &events.guitar_polyphonic_notes {
            let candidate = &recovered.evidence.candidate;
            if sources.iter().any(|(tick, group)| {
                group.iter().any(|(lane, source)| {
                    source.start == candidate.start
                        && source.end == candidate.end
                        && source.pitch == candidate.pitch
                        && notes.iter().any(|n| n.tick == *tick && n.lane == *lane)
                })
            }) {
                let mut issue = Issue::at(
                    candidate.start,
                    "ExpertSingle",
                    Reason::RecoveredPolyphonicGuitar,
                    &[
                        ("network_onset_score", recovered.evidence.onset_score as f64),
                        ("mean_frame_score", candidate.confidence as f64),
                        ("source_attack_seconds", recovered.source_attack_seconds),
                        (
                            "target_variance_gain",
                            recovered.minimum_target_variance_gain,
                        ),
                        ("frequency_offset_hz", recovered.frequency_offset_hz),
                    ],
                );
                issue.end = candidate.end;
                issues.push(issue);
            }
        }
    }
    if generated.contains("ExpertDrums") && !events.drum_recovered_hits.is_empty() {
        let mut baseline = events.clone();
        baseline.drums.retain(|h| {
            !events
                .drum_recovered_hits
                .iter()
                .any(|r| r.time == h.time && r.voice == h.voice && r.confidence == h.confidence)
        });
        baseline.drum_recovered_hits.clear();
        // Removing recovered hits shifts raw indices. Keep surviving articulation
        // evidence attached to the same source hit for the before/after comparison.
        baseline.drum_articulations.retain_mut(|e| {
            let original = &events.drums[e.hit_index];
            if let Some(index) = baseline.drums.iter().position(|h| {
                h.time == original.time
                    && h.voice == original.voice
                    && h.confidence == original.confidence
            }) {
                e.hit_index = index;
                true
            } else {
                false
            }
        });
        let before = crate::mapping::tracks(&baseline, &chart.timing, false)?;
        let after = crate::mapping::tracks(events, &chart.timing, false)?;
        for note in chart.notes("ExpertDrums")?.iter().filter(|n| n.lane <= 4) {
            if after
                .get("ExpertDrums")
                .is_some_and(|notes| notes.contains(note))
                && before
                    .get("ExpertDrums")
                    .is_none_or(|notes| !notes.contains(note))
            {
                issues.push(Issue::at(
                    chart.timing.seconds(note.tick),
                    "ExpertDrums",
                    Reason::RecoveredPercussionAttack,
                    &[("maximum_source_attack_distance_ms", 25.)],
                ));
            }
        }
    }
    let pro_arrangement = crate::pro_keys::arrange_transcription(events, &chart.timing)?;
    for (track, candidates) in [
        ("ExpertSingle", &events.guitar),
        ("ExpertDoubleBass", &events.bass),
        ("ExpertKeyboard", &events.piano),
        ("ExpertProKeys", &events.pro_piano),
    ] {
        if !generated.contains(track) {
            continue;
        }
        let notes = chart.notes(track)?;
        for (candidate_index, candidate) in candidates.iter().enumerate() {
            let eligible = candidate.confidence >= 0.20
                && if track == "ExpertDoubleBass" {
                    (24..=67).contains(&candidate.pitch)
                } else if crate::keys::is_pro(track) {
                    true
                } else {
                    candidate.pitch >= 40
                };
            if eligible
                && (candidate.confidence as f64) < policy.low_candidate_score_below
                && notes.iter().any(|n| {
                    crate::editor::playable(track, n.lane)
                        && (!crate::keys::is_pro(track)
                            || pro_arrangement.decisions[candidate_index].target_pitch
                                == Some(n.lane))
                        && (chart.timing.seconds(n.tick) - candidate.start).abs() <= 0.026
                })
            {
                let mut issue = Issue::at(
                    candidate.start,
                    track,
                    Reason::LowCandidateScore,
                    &[
                        ("candidate_score", candidate.confidence as f64),
                        ("threshold_below", policy.low_candidate_score_below),
                        ("candidate_midi_pitch", candidate.pitch as f64),
                    ],
                );
                issue.end = candidate.end;
                issues.push(issue);
            }
        }
    }
    if generated.contains("ExpertDrums") {
        let notes = chart.notes("ExpertDrums")?;
        for candidate in &events.drums {
            if (candidate.confidence as f64) < policy.low_candidate_score_below
                && notes.iter().any(|n| {
                    n.lane == candidate.voice.lane()
                        && (chart.timing.seconds(n.tick) - candidate.time).abs() <= 0.026
                })
            {
                issues.push(Issue::at(
                    candidate.time,
                    "ExpertDrums",
                    Reason::LowCandidateScore,
                    &[
                        ("candidate_score", candidate.confidence as f64),
                        ("threshold_below", policy.low_candidate_score_below),
                        ("candidate_lane", candidate.voice.lane() as f64),
                    ],
                ));
            }
        }
    }
    if generated.contains("ExpertDoubleBass") {
        let notes = chart.notes("ExpertDoubleBass")?;
        let selected = crate::mapping::bass_attack_sources(&events.bass, &chart.timing);
        let played = |candidate: &crate::mapping::PitchNote| {
            selected.iter().any(|(tick, sources)| {
                sources.iter().any(|(lane, source)| {
                    source.start == candidate.start
                        && source.end == candidate.end
                        && source.pitch == candidate.pitch
                        && notes
                            .iter()
                            .any(|note| note.tick == *tick && note.lane == *lane)
                })
            })
        };
        for conflict in &events.bass_harmonic_conflicts {
            let candidate = &conflict.candidate;
            let competing = &conflict.competing_candidate;
            if !played(candidate)
                || !events.bass.iter().any(|source| {
                    source.start == competing.start
                        && source.end == competing.end
                        && source.pitch == competing.pitch
                })
            {
                continue;
            }
            let mut issue = Issue::at(
                candidate.start,
                "ExpertDoubleBass",
                Reason::PitchDisagreement,
                &[
                    ("unresolved_harmonic_candidate_conflict", 1.),
                    ("candidate_midi_pitch", candidate.pitch as f64),
                    ("competing_candidate_midi_pitch", competing.pitch as f64),
                    ("competing_candidate_start_seconds", competing.start),
                    ("candidate_end_seconds", candidate.end),
                    (
                        "candidate_onset_separation_ms",
                        (competing.start - candidate.start) * 1000.,
                    ),
                    ("candidate_score", candidate.confidence as f64),
                    ("competing_candidate_score", competing.confidence as f64),
                    ("pitch_change_approved", 0.),
                ],
            );
            issue.end = candidate.end;
            issues.push(issue);
        }
        for observation in events
            .bass_attack_reviews
            .iter()
            .filter(|observation| !observation.supported)
        {
            let candidate = &observation.candidate;
            if !played(candidate) {
                continue;
            }
            let mut evidence = vec![
                ("attack_tolerance_ms", 50.),
                ("candidate_score", candidate.confidence as f64),
            ];
            if let Some(residual) = observation.residual_seconds {
                evidence.push(("nearest_attack_ms", residual * 1000.));
            }
            let mut issue = Issue::at(
                candidate.start,
                "ExpertDoubleBass",
                Reason::WeakAttackSupport,
                &evidence,
            );
            issue.end = candidate.end;
            issues.push(issue);
        }
        let has_attack = |time: f64| {
            notes.iter().any(|note| {
                (note.lane <= 4 || note.lane == 7)
                    && (chart.timing.seconds(note.tick) - time).abs() <= 0.026
            })
        };
        for recovered in events
            .bass_short_candidates
            .iter()
            .filter(|candidate| candidate.retained)
        {
            let original = &recovered.evidence.candidate;
            let mut candidate = original.clone();
            if let Some(correction) = events.bass_corrections.iter().find(|correction| {
                correction.original.start == original.start
                    && correction.original.end == original.end
                    && correction.original.pitch == original.pitch
            }) {
                candidate.pitch = correction.revised_pitch;
            }
            if let Some(correction) = events.bass_timing_corrections.iter().find(|correction| {
                correction.original.start == original.start
                    && correction.original.end == original.end
                    && correction.original.pitch == candidate.pitch
            }) {
                candidate.start = correction.revised_start;
            }
            if !played(&candidate) {
                continue;
            }
            let mut evidence = vec![
                (
                    "candidate_duration_ms",
                    (original.end - original.start) * 1000.,
                ),
                ("network_onset_score", recovered.evidence.onset_score as f64),
            ];
            if let Some(value) = recovered.source_attack_seconds {
                evidence.push(("source_attack_seconds", value));
            }
            if let Some(value) = recovered.periodic_pitch {
                evidence.push(("periodic_midi_pitch", value));
            }
            if let Some(value) = recovered.periodicity {
                evidence.push(("periodicity", value));
            }
            if let Some(value) = recovered.fundamental_fraction {
                evidence.push(("fundamental_fraction", value));
            }
            let mut issue = Issue::at(
                candidate.start,
                "ExpertDoubleBass",
                Reason::RecoveredShortNote,
                &evidence,
            );
            issue.end = candidate.end;
            issues.push(issue);
        }
        for correction in &events.bass_timing_corrections {
            if !has_attack(correction.revised_start) {
                continue;
            }
            let mut issue = Issue::at(
                correction.revised_start,
                "ExpertDoubleBass",
                Reason::StemAttackTiming,
                &[
                    ("original_attack_seconds", correction.original.start),
                    ("revised_attack_seconds", correction.revised_start),
                    (
                        "sample_boundary_shift_ms",
                        (correction.revised_start - correction.original.start) * 1000.,
                    ),
                    ("before_rms", correction.before_rms),
                    ("after_rms", correction.after_rms),
                ],
            );
            issue.end = correction.original.end;
            issues.push(issue);
        }
        for correction in &events.bass_corrections {
            let time = events
                .bass_timing_corrections
                .iter()
                .find(|timing| {
                    timing.original.start == correction.original.start
                        && timing.original.pitch == correction.revised_pitch
                })
                .map_or(correction.original.start, |timing| timing.revised_start);
            if !has_attack(time) {
                continue;
            }
            let mut issue = Issue::at(
                time,
                "ExpertDoubleBass",
                Reason::PitchDisagreement,
                &[
                    ("original_midi_pitch", correction.original.pitch as f64),
                    ("revised_midi_pitch", correction.revised_pitch as f64),
                    ("periodic_midi_pitch", correction.periodic_pitch),
                    ("periodicity", correction.clarity),
                ],
            );
            issue.end = correction.original.end;
            issues.push(issue);
        }
    }
    Ok(issues)
}

fn region_html(map: &ReviewMap) -> String {
    let mut html = format!(
        "<h2>Sections needing review</h2><p>{} marked sections. All remain pending. Unmarked sections are unverified. Priority describes review urgency, not the probability of an error. Model scores are uncalibrated. <a href='review-regions.json'>Structured markers for tools and AI</a></p><nav aria-label='Review sections'>",
        map.regions.len()
    );
    for region in &map.regions {
        html.push_str(&format!(
            "<a href='#{}'>{:.1}–{:.1}s</a> ",
            region.id, region.start, region.end
        ));
    }
    html.push_str("</nav><details><summary>Check coverage and limitations</summary><ul>");
    for limit in &map.coverage_limits {
        html.push_str(&format!("<li>{}</li>", escape(limit)));
    }
    html.push_str("</ul></details>");
    for region in &map.regions {
        let mut reasons = BTreeMap::new();
        for issue in &region.issues {
            *reasons.entry(issue.reason).or_insert(0usize) += 1;
        }
        let summary = reasons
            .iter()
            .map(|(reason, count)| format!("{}: {count}", reason.label()))
            .collect::<Vec<_>>()
            .join(" · ");
        html.push_str(&format!("<section id='{}'><h3>{:.1}–{:.1} seconds · {:?} priority · Pending</h3><p>{}</p><p>{}</p><p>Original recording at this section:</p><audio controls preload='none' src='recording.mp3#t={:.3},{:.3}'></audio><details><summary>{} evidence flags — inspect exact events</summary><ul>", region.id, region.start, region.end, region.priority, escape(&region.tracks.iter().cloned().collect::<Vec<_>>().join(", ")), escape(&summary), region.start, region.end, region.issues.len()));
        for issue in &region.issues {
            let evidence = issue
                .evidence
                .iter()
                .map(|(key, value)| format!("{key}={value:.4}"))
                .collect::<Vec<_>>()
                .join(", ");
            html.push_str(&format!(
                "<li>{:.3}–{:.3}s · {}: {}. {}</li>",
                issue.start,
                issue.end,
                escape(&issue.track),
                escape(issue.reason.description()),
                escape(&evidence)
            ));
        }
        html.push_str("</ul></details></section>");
    }
    html
}

/// Reassess the notes being reviewed, not a report left by their original export.
/// Authored tracks stay outside the generated-only rule profile. Structural
/// failures remain visible without discarding the independent signal evidence.
#[cfg(not(target_arch = "wasm32"))]
fn with_current_generated_rules(
    mut map: ReviewMap,
    chart: &Chart,
) -> Result<(ReviewMap, Option<crate::generated_rules::RuleReport>)> {
    let Some(generated) = &map.generated_tracks else {
        map.coverage_limits.push("Generated-only sustain/reduction and structural checks were not assessed because generated-track provenance is unavailable.".into());
        return Ok((map, None));
    };
    let emitted: crate::chart::Tracks = generated
        .iter()
        .map(|track| Ok((track.clone(), chart.notes(track)?)))
        .collect::<Result<_>>()?;
    let rules = crate::generated_rules::assess(&emitted, &chart.timing, map.duration_seconds);
    if rules.has_errors() {
        let errors = rules
            .findings
            .iter()
            .filter(|finding| finding.severity == crate::generated_rules::Severity::Error)
            .count();
        map.coverage_limits.push(format!(
            "Current generated tracks have {errors} structural rule error(s); see audit.json generated_rule_report for exact findings. Generated-rule sustain/reduction prompts were not merged into this map. Existing stem and pattern diagnostics remain available; no structural or musical approval is implied."
        ));
        Ok((map, Some(rules)))
    } else {
        let mut merged = pipeline::with_generated_rule_reviews(map, &rules)?;
        merged.coverage_limits.push("Generated-rule findings in this review were recomputed from the current chart's generated-track subset. audit.json generated_rule_report is the current assessment; any original generation receipt may describe older notes.".into());
        Ok((merged, Some(rules)))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run(
    folder: &Path,
    drum_audio: &Path,
    bass_audio: Option<&Path>,
    output: &Path,
) -> Result<serde_json::Value> {
    pipeline::destination(output, None)?;
    let chart_path = folder.join("notes.chart");
    let chart = Chart::parse(fs::read_to_string(&chart_path)?)?;
    let event_path = folder.join("chart-engine-transcription.json");
    let mut events: Transcription = serde_json::from_slice(&fs::read(&event_path)?)?;
    events.validate()?;
    let original = pipeline::audio_file(folder)?;
    let original_signal = models::decode(&original, 44100, 1)?;
    let duration = original_signal.len() as f64 / 44100.0;
    let drums = chart.notes("ExpertDrums")?;
    let bass = chart.notes("ExpertDoubleBass")?;
    // A nearer discarded harmonic is not the pitch actually selected for a
    // played gem. Unmatched/authored colors and polyphonic attacks are unjudged.
    events.bass = crate::mapping::bass_monophonic_chart_sources(&events.bass, &chart.timing, &bass)
        .into_values()
        .collect();
    let drum_signal = models::decode(drum_audio, 22050, 1)?;
    ensure!(
        (drum_signal.len() as f64 / 22050.0 - duration).abs() <= 0.10,
        "Drum stem duration differs from the recording; review requires time-aligned full-song stems"
    );
    let peaks = attacks(&drum_signal);
    let support: Vec<_> = drums
        .iter()
        .filter(|n| n.lane <= 4)
        .map(|n| {
            let time = chart.timing.seconds(n.tick);
            (
                time,
                nearest(time, &peaks).map(|p| (p.time - time) * 1000.0),
            )
        })
        .collect();
    let weak: Vec<_> = support
        .iter()
        .filter(|(_, error)| error.is_none_or(|e| e.abs() > 50.0))
        .map(|(t, error)| serde_json::json!({"time":t,"nearest_attack_ms":error}))
        .collect();
    let bass_signal = bass_audio
        .map(|path| models::decode(path, 11025, 1))
        .transpose()?;
    if let Some(signal) = &bass_signal {
        ensure!(
            (signal.len() as f64 / 11025.0 - duration).abs() <= 0.10,
            "Bass stem duration differs from the recording; review requires time-aligned full-song stems"
        );
    }
    let checks = if let Some(signal) = &bass_signal {
        bass_checks(&events, signal)
    } else {
        Vec::new()
    };
    let disagreements: Vec<_> = checks
        .iter()
        .filter(|c| c.semitone_error.abs() > 1.0)
        .collect();
    let voices = events
        .drums
        .iter()
        .fold(BTreeMap::new(), |mut counts, hit| {
            *counts.entry(format!("{:?}", hit.voice)).or_insert(0usize) += 1;
            counts
        });
    let track_checks = all_track_checks(&chart)?;
    let mut inputs = BTreeMap::from([
        ("chart".into(), models::hash_file(&chart_path)?),
        ("transcription".into(), models::hash_file(&event_path)?),
        ("original_audio".into(), models::hash_file(&original)?),
        ("drum_audio".into(), models::hash_file(drum_audio)?),
    ]);
    if let Some(path) = bass_audio {
        inputs.insert("bass_audio".into(), models::hash_file(path)?);
    }
    let receipt_path = folder.join("chart-engine-report.json");
    let generated_tracks: Option<BTreeSet<String>> = if receipt_path.exists() {
        let receipt: serde_json::Value = serde_json::from_slice(&fs::read(&receipt_path)?)?;
        inputs.insert(
            "generation_receipt".into(),
            models::hash_file(&receipt_path)?,
        );
        Some(serde_json::from_value(
            receipt
                .get("added")
                .context("Generation receipt has no added tracks")?
                .clone(),
        )?)
    } else {
        None
    };
    let policy = Policy::default();
    let mut issues = candidate_score_issues(&chart, &events, generated_tracks.as_ref(), &policy)?;
    for (time, error) in &support {
        if error.is_none_or(|e| e.abs() > 50.0) {
            let evidence: Vec<_> = error
                .map(|e| ("nearest_attack_ms", e))
                .into_iter()
                .collect();
            issues.push(Issue::at(
                *time,
                "ExpertDrums",
                Reason::WeakAttackSupport,
                &evidence,
            ));
        }
    }
    for check in &disagreements {
        issues.push(Issue::at(
            check.time,
            "ExpertDoubleBass",
            Reason::PitchDisagreement,
            &[
                ("candidate_midi_pitch", check.predicted as f64),
                ("periodic_midi_pitch", check.periodic_midi),
                ("semitone_difference", check.semitone_error),
                ("periodicity", check.clarity),
            ],
        ));
    }
    issues.extend(pattern_issues(&track_checks));
    let mut performed = generation_checks(generated_tracks.is_some());
    performed.insert(Reason::WeakAttackSupport);
    if bass_audio.is_some() {
        performed.insert(Reason::PitchDisagreement);
    }
    performed.extend(issues.iter().map(|issue| issue.reason));
    let review_map = ReviewMap::build(
        duration,
        policy,
        inputs,
        generated_tracks,
        performed,
        issues,
    )?;
    let (review_map, generated_rule_report) = with_current_generated_rules(review_map, &chart)?;
    let mut residuals: Vec<_> = support.iter().filter_map(|(_, r)| *r).collect();
    residuals.sort_by(f64::total_cmp);
    let median = residuals.get(residuals.len() / 2).copied();
    let report = serde_json::json!({
        "status":"needs_listening_review",
        "chart_sha256":review_map.input_sha256["chart"],
        "original_audio_sha256":review_map.input_sha256["original_audio"],
        "drum_audio_sha256":review_map.input_sha256["drum_audio"],
        "bass_audio_sha256":review_map.input_sha256.get("bass_audio"),
        "review_regions":review_map,
        "generated_rule_report":generated_rule_report,
        "raw_drum_voice_counts":voices,"tracks":track_checks,
        "independent_flux_attacks":peaks.len(),
        "drum_note_attack_residual_median_ms":median,
        "drum_notes_over_50ms_from_independent_attack":weak,
        "bass_eligible_candidates_near_chart_attacks":events.bass.len(),
        "bass_periodic_frames":checks.len(),
        "bass_corrections":events.bass_corrections,
        "bass_timing_corrections":events.bass_timing_corrections,
        "bass_pitch_disagreements_over_one_semitone":disagreements,"bass_checks":checks,
        "limits":[
            "Listening approval requires human review of the source recording.",
            "Spectral flux supports attack timing only; it cannot establish kick/snare/tom/cymbal identity or count all true hits. Missing flux peaks are review flags, not proven false positives.",
            "YIN is also used in bass harmonic refinement. On refined output, this comparison measures detector consistency, not independent musical accuracy. Octave errors and stem leakage remain possible.",
            "Bass pitch checks cover monophonic attacks whose lane matches the effective mapper within 26 ms. Discarded harmonics, polyphonic attacks and unmatched/authored lane layouts are unjudged; five-fret colors alone cannot establish pitch identity. Notes shorter than 200 ms are unjudged.",
            "Pattern thresholds identify review candidates, not automatic musical errors. Neither coarse-grid deviation nor a fast roll alone establishes an error.",
            "Raw event counts and an auto-player completing a chart are not musical correctness."
        ]
    });
    let parent = output.parent().context("Output needs a parent directory")?;
    let stage = tempfile::tempdir_in(parent)?;
    fs::write(
        stage.path().join("audit.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(
        stage.path().join("review-regions.json"),
        serde_json::to_vec_pretty(&review_map)?,
    )?;
    encode_mp3(
        &stage.path().join("recording.mp3"),
        &original_signal,
        &original_signal,
    )?;
    let mut starts = vec![0.0, 40.0, 80.0, 120.0, 160.0, (duration - 12.0).max(0.0)];
    for candidate in weak.iter().take(3) {
        if let Some(time) = candidate["time"].as_f64() {
            starts.push((time - 2.0).max(0.0));
        }
    }
    for track in track_checks.values() {
        for time in track
            .same_lane_under_50ms
            .iter()
            .chain(&track.more_than_two_hand_hits_within_20ms)
        {
            starts.push((time - 2.0).max(0.0));
        }
    }
    for candidate in checks
        .iter()
        .filter(|c| c.semitone_error.abs() > 1.0)
        .take(3)
    {
        starts.push((candidate.time - 2.0).max(0.0));
    }
    starts.sort_by(f64::total_cmp);
    starts.dedup_by(|a, b| (*a - *b).abs() < 5.0);
    starts.retain(|s| *s < duration);
    let mut html = format!(
        "<!doctype html><meta charset=utf-8><title>Chart audio review</title><style>body{{font:16px system-ui;background:#131722;color:#eee;max-width:1100px;margin:40px auto;padding:20px}}section{{background:#212838;padding:20px;margin:20px 0;border-radius:12px}}audio{{width:100%}}svg{{width:100%;background:#101522}}a{{color:#8dceff}}p{{line-height:1.5}}</style><h1>{}</h1><p>Listening approval is pending. Left channel: original recording. Right channel: synthesized hits from the actual Expert drum chart. Use headphones or compare the rhythmic attacks through speakers. The drum sounds are schematic; their timbre does not prove a particular kit assignment.</p><p><a href=audit.json>Full independent signal and pattern report</a></p>",
        escape(&folder.file_name().unwrap_or_default().to_string_lossy())
    );
    html.push_str(&format!("<p>{} Expert drum gems; {} gems without an independent attack within 50 ms. These are review flags, not an accuracy score. Signed median attack residual: {:?} ms.</p><p>Listen especially around: <strong>{}</strong> seconds. The JSON report contains per-tier fast-repeat and hand-crowding flags.</p>", support.len(),weak.len(),median,starts.iter().map(|t|format!("{t:.1}")).collect::<Vec<_>>().join(", ")));
    html.push_str(&region_html(&review_map));
    html.push_str("<h2>Synchronized drum and bass comparisons</h2>");
    let bass_full = bass_audio
        .map(|path| models::decode(path, 44100, 1))
        .transpose()?;
    for (i, start) in starts.iter().enumerate() {
        let first = (*start * 44100.0) as usize;
        let end = (first + 12 * 44100).min(original_signal.len());
        let right = drum_render(&drums, &chart, *start, end - first);
        let filename = format!("drums-{i:02}.mp3");
        encode_mp3(
            &stage.path().join(&filename),
            &original_signal[first..end],
            &right,
        )?;
        let mut svg = String::from(
            "<svg viewBox='0 0 1000 190' role=img aria-label='Drum chart and independently measured attacks'>",
        );
        for peak in peaks
            .iter()
            .filter(|p| p.time >= *start && p.time < *start + 12.0)
        {
            let x = (peak.time - start) / 12.0 * 1000.0;
            svg.push_str(&format!("<path d='M{x:.1} 0v160' stroke='#444d60'/>"));
        }
        let colors = ["#ff9b3c", "#ff5d63", "#ffd655", "#56aaff", "#6fda86"];
        for note in drums.iter().filter(|n| n.lane <= 4) {
            let t = chart.timing.seconds(note.tick) - start;
            if (0.0..12.0).contains(&t) {
                let x = t / 12.0 * 1000.0;
                let y = 20 + note.lane as usize * 30;
                svg.push_str(&format!(
                    "<circle cx='{x:.1}' cy='{y}' r='4' fill='{}'/>",
                    colors[note.lane as usize]
                ));
            }
        }
        svg.push_str("<text x='10' y='182' fill='#ddd' font-size='13'>Rows: kick, snare, yellow, blue, green. Grey lines: independent audio attacks.</text></svg>");
        html.push_str(&format!("<section><h2>{start:.1}–{:.1} seconds</h2><audio controls preload=none src='{filename}'></audio>{svg}</section>",(*start+12.0).min(duration)));
        if let Some(signal) = &bass_full {
            let bass_end = end.min(signal.len());
            if first < bass_end {
                let right = bass_render(&events, *start, bass_end - first);
                let filename = format!("bass-{i:02}.mp3");
                encode_mp3(
                    &stage.path().join(&filename),
                    &signal[first..bass_end],
                    &right,
                )?;
                html.push_str(&format!("<section><h3>Bass at {start:.1} seconds</h3><p>Left: separated bass stem. Right: eligible predicted pitches near chart attacks. This previews transcription candidates; five-fret lanes encode relative movement and do not specify these absolute pitches.</p><audio controls preload=none src='{filename}'></audio></section>"));
            }
        }
    }
    if !bass.is_empty() {
        html.push_str("<section><h2>Bass check</h2><p>The JSON report compares predicted MIDI pitch with YIN estimates. YIN also participates in bass refinement, so agreement measures detector consistency rather than independent accuracy. Five-fret positions encode relative movement, not absolute musical pitch. Disagreements flag sections for review.</p></section>");
    }
    fs::write(stage.path().join("review.html"), html)?;
    pipeline::publish(stage.path(), output)?;
    Ok(report)
}

fn bass_render(events: &Transcription, start: f64, length: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; length];
    for note in &events.bass {
        let onset = note.start - start;
        if note.end <= start || onset >= length as f64 / 44100.0 {
            continue;
        }
        let first = (onset.max(0.0) * 44100.0).round() as usize;
        let end = (((note.end - start) * 44100.0).round() as usize).min(length);
        let frequency = 440.0 * 2.0f64.powf((note.pitch as f64 - 69.0) / 12.0);
        for (i, sample) in out.iter_mut().enumerate().take(end).skip(first) {
            let elapsed = i as f64 / 44100.0 - onset;
            let release = ((note.end - start - i as f64 / 44100.0) / 0.015).clamp(0.0, 1.0);
            let attack = (elapsed / 0.006).clamp(0.0, 1.0);
            *sample += (std::f64::consts::TAU * frequency * elapsed).sin() as f32
                * 0.18
                * (attack * release) as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::PitchNote;

    fn chart() -> Chart {
        Chart::parse(
            "[Song]\n{\n Resolution = 480\n Offset = -0.5\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n"
                .into(),
        )
        .unwrap()
    }

    fn rule_map(generated: Option<BTreeSet<String>>) -> ReviewMap {
        ReviewMap::build(
            4.0,
            Policy::default(),
            BTreeMap::new(),
            generated,
            BTreeSet::from([Reason::WeakAttackSupport]),
            vec![Issue::at(
                0.5,
                "ExpertDrums",
                Reason::WeakAttackSupport,
                &[],
            )],
        )
        .unwrap()
    }

    #[test]
    fn rich_rule_review_reassesses_current_notes_and_retains_signal_flags() {
        let current = Chart::parse(format!(
            "{}[ExpertSingle]\n{{\n 960 = N 0 960\n 1200 = N 0 0\n}}\n[HardSingle]\n{{\n 1440 = N 1 0\n}}\n",
            chart().raw
        ))
        .unwrap();
        let original = rule_map(Some(BTreeSet::from([
            "ExpertSingle".into(),
            "HardSingle".into(),
        ])));
        let (merged, rules) = with_current_generated_rules(original, &current).unwrap();
        let rules = rules.unwrap();
        assert!(!rules.has_errors());
        assert_eq!(rules.assessed_tracks, 2);
        assert_eq!(rules.assessed_note_events, 3);
        for reason in [
            Reason::WeakAttackSupport,
            Reason::SustainOverlap,
            Reason::ReductionDifference,
        ] {
            assert!(merged.checks_performed.contains(&reason));
            assert!(
                merged
                    .regions
                    .iter()
                    .any(|region| region.issues.iter().any(|issue| issue.reason == reason))
            );
        }
        assert_eq!(merged.musical_acceptance, "unverified");
    }

    #[test]
    fn rich_rule_review_keeps_structural_errors_visible_without_losing_signal_flags() {
        let current = Chart::parse(format!(
            "{}[ExpertDrums]\n{{\n 960 = N 2 20\n}}\n",
            chart().raw
        ))
        .unwrap();
        let original = rule_map(Some(BTreeSet::from(["ExpertDrums".into()])));
        let original_issues: Vec<_> = original
            .regions
            .iter()
            .flat_map(|region| region.issues.clone())
            .collect();
        let (merged, rules) = with_current_generated_rules(original, &current).unwrap();
        assert!(rules.unwrap().has_errors());
        let retained: Vec<_> = merged
            .regions
            .iter()
            .flat_map(|region| region.issues.clone())
            .collect();
        assert_eq!(retained, original_issues);
        assert!(!merged.checks_performed.contains(&Reason::SustainOverlap));
        assert!(
            merged
                .coverage_limits
                .iter()
                .any(|limit| limit.contains("structural rule error(s)"))
        );
        assert!(region_html(&merged).contains("structural rule error(s)"));
        assert_eq!(merged.musical_acceptance, "unverified");
    }

    #[test]
    fn rich_rule_review_does_not_assess_authored_tracks_as_generated() {
        let current = Chart::parse(format!(
            "{}[ExpertSingle]\n{{\n 960 = N 7 0\n 960 = N 0 0\n}}\n[ExpertDrums]\n{{\n 960 = N 1 0\n}}\n",
            chart().raw
        ))
        .unwrap();
        let original = rule_map(Some(BTreeSet::from(["ExpertDrums".into()])));
        let (_, rules) = with_current_generated_rules(original, &current).unwrap();
        let rules = rules.unwrap();
        assert!(!rules.has_errors());
        assert_eq!(rules.assessed_tracks, 1);
        assert_eq!(rules.assessed_note_events, 1);
    }

    #[test]
    fn rich_rule_review_reports_unknown_provenance_instead_of_claiming_checks_ran() {
        let (map, rules) = with_current_generated_rules(rule_map(None), &chart()).unwrap();
        assert!(rules.is_none());
        assert!(!map.checks_performed.contains(&Reason::SustainOverlap));
        assert!(
            map.coverage_limits
                .iter()
                .any(|limit| limit.contains("provenance is unavailable"))
        );
        assert_eq!(map.musical_acceptance, "unverified");
    }

    #[test]
    fn independent_flux_finds_known_attacks_and_rejects_silence() {
        assert!(attacks(&vec![0.0; 22050]).is_empty());
        let mut signal = vec![0.0; 22050 * 2];
        for time in [0.4, 0.9, 1.4] {
            let offset = (time * 22050.0) as usize;
            for i in 0..500 {
                signal[offset + i] = (std::f32::consts::TAU * 130.0 * i as f32 / 22050.0).cos()
                    * (-(i as f32) / 90.0).exp();
            }
        }
        let peaks = attacks(&signal);
        for time in [0.4, 0.9, 1.4] {
            assert!((nearest(time, &peaks).unwrap().time - time).abs() < 0.025);
        }
        assert!(nearest(1.0, &[]).is_none());
        assert_eq!(nearest(0.0, &peaks).unwrap().time, peaks[0].time);
        assert_eq!(
            nearest(9.0, &peaks).unwrap().time,
            peaks.last().unwrap().time
        );
    }

    #[test]
    fn independent_pitch_agrees_with_known_bass_and_flags_wrong_pitch() {
        let signal: Vec<_> = (0..11025)
            .map(|i| (std::f32::consts::TAU * 82.40689 * i as f32 / 11025.0).sin() * 0.3)
            .collect();
        let events = Transcription {
            bass: vec![
                PitchNote {
                    start: 0.2,
                    end: 0.5,
                    pitch: 40,
                    confidence: 0.9,
                },
                PitchNote {
                    start: 0.5,
                    end: 0.8,
                    pitch: 43,
                    confidence: 0.9,
                },
                PitchNote {
                    start: 0.8,
                    end: 0.85,
                    pitch: 40,
                    confidence: 0.9,
                },
            ],
            ..Default::default()
        };
        let checks = bass_checks(&events, &signal);
        assert_eq!(checks.len(), 2, "short notes must be unjudged");
        assert!(checks[0].semitone_error.abs() < 0.1);
        assert!((checks[1].semitone_error + 3.0).abs() < 0.1);
        assert!(checks.iter().all(|c| (0.85..=1.0).contains(&c.clarity)));
        assert!(bass_checks(&events, &vec![0.0; 11025]).is_empty());
    }

    #[test]
    fn pattern_checks_flag_nearby_three_hand_hits_without_counting_kick() {
        let chart = chart();
        let notes: Vec<_> = [(960, 0), (960, 1), (965, 2), (970, 3), (980, 3)]
            .into_iter()
            .map(|(tick, lane)| Note {
                tick,
                lane,
                sustain: 0,
            })
            .collect();
        let report = track_check(&notes, &chart, true);
        assert_eq!(report.more_than_two_hand_hits_within_20ms, vec![0.5]);
        assert_eq!(report.same_lane_under_50ms.len(), 1);
        let two_hands: Vec<_> = notes.iter().filter(|n| n.lane != 3).cloned().collect();
        assert!(
            track_check(&two_hands, &chart, true)
                .more_than_two_hand_hits_within_20ms
                .is_empty()
        );
    }

    #[test]
    fn unresolved_bass_harmonics_flag_only_the_played_generated_candidate() {
        let upper = PitchNote {
            start: 0.5,
            end: 0.8,
            pitch: 52,
            confidence: 0.4,
        };
        let lower = PitchNote {
            start: 0.54,
            end: 1.,
            pitch: 40,
            confidence: 0.9,
        };
        let discarded = PitchNote {
            pitch: 64,
            confidence: 0.21,
            ..upper.clone()
        };
        let events = Transcription {
            bass: vec![upper.clone(), discarded.clone(), lower.clone()],
            bass_harmonic_conflicts: [upper, discarded]
                .into_iter()
                .map(|candidate| crate::mapping::BassHarmonicConflict {
                    candidate,
                    competing_candidate: lower.clone(),
                })
                .collect(),
            ..Default::default()
        };
        events.validate().unwrap();
        let timing = chart().timing;
        let tracks = crate::mapping::tracks(&events, &timing, false).unwrap();
        let mapped = Chart::parse(crate::chart::new_chart("Test", "", &timing, &tracks)).unwrap();
        let generated = BTreeSet::from(["ExpertDoubleBass".into()]);
        let issues =
            candidate_score_issues(&mapped, &events, Some(&generated), &Policy::default()).unwrap();
        let conflicts: Vec<_> = issues
            .iter()
            .filter(|issue| issue.reason == Reason::PitchDisagreement)
            .collect();
        assert_eq!(
            conflicts.len(),
            1,
            "discarded upper harmonics do not own the played gem"
        );
        assert_eq!(
            conflicts[0].evidence["unresolved_harmonic_candidate_conflict"],
            1.
        );
        assert_eq!(conflicts[0].evidence["pitch_change_approved"], 0.);
        assert!(
            !conflicts[0].evidence.contains_key("periodic_midi_pitch"),
            "source conflict must not pretend to be an audio pitch estimate"
        );
        assert!(
            candidate_score_issues(&mapped, &events, Some(&BTreeSet::new()), &Policy::default())
                .unwrap()
                .is_empty()
        );
        let note = &mapped.notes("ExpertDoubleBass").unwrap()[0];
        let changed = Chart::parse(format!(
            "{}[ExpertDoubleBass]\n{{\n {} = N {} 0\n}}\n",
            chart().raw,
            note.tick,
            (note.lane + 1) % 5
        ))
        .unwrap();
        assert!(
            candidate_score_issues(&changed, &events, Some(&generated), &Policy::default())
                .unwrap()
                .iter()
                .all(|issue| issue.reason != Reason::PitchDisagreement)
        );
        let mut removed_competitor = events.clone();
        removed_competitor
            .bass
            .retain(|candidate| candidate.pitch != 40);
        assert!(
            candidate_score_issues(
                &mapped,
                &removed_competitor,
                Some(&generated),
                &Policy::default()
            )
            .unwrap()
            .iter()
            .all(|issue| issue.reason != Reason::PitchDisagreement),
            "a removed competitor cannot leave stale source-conflict evidence"
        );
        let legacy: Transcription = serde_json::from_str("{\"bass\":[]}").unwrap();
        assert!(legacy.bass_harmonic_conflicts.is_empty());
    }

    #[test]
    fn bass_attack_flags_follow_selected_generated_sources_only() {
        let note = PitchNote {
            start: 0.5,
            end: 0.8,
            pitch: 40,
            confidence: 0.9,
        };
        let rejected = PitchNote {
            pitch: 52,
            confidence: 0.21,
            ..note.clone()
        };
        let events = Transcription {
            bass: vec![note.clone(), rejected.clone()],
            bass_attack_reviews: [note, rejected]
                .into_iter()
                .map(|candidate| crate::mapping::BassAttackReview {
                    candidate,
                    nearest_attack_seconds: Some(0.35),
                    residual_seconds: Some(-0.15),
                    supported: false,
                })
                .collect(),
            ..Default::default()
        };
        let timing = chart().timing;
        let tracks = crate::mapping::tracks(&events, &timing, false).unwrap();
        let mapped = Chart::parse(crate::chart::new_chart("Test", "", &timing, &tracks)).unwrap();
        let generated = BTreeSet::from(["ExpertDoubleBass".into()]);
        let issues =
            candidate_score_issues(&mapped, &events, Some(&generated), &Policy::default()).unwrap();
        let weak: Vec<_> = issues
            .iter()
            .filter(|issue| issue.reason == Reason::WeakAttackSupport)
            .collect();
        assert_eq!(
            weak.len(),
            1,
            "discarded harmonics cannot create an attack flag"
        );
        assert_eq!(weak[0].start, 0.5);
        assert_eq!(weak[0].evidence["attack_tolerance_ms"], 50.);
        assert!(
            candidate_score_issues(&mapped, &events, Some(&BTreeSet::new()), &Policy::default())
                .unwrap()
                .is_empty()
        );
        let selected = mapped.notes("ExpertDoubleBass").unwrap();
        let changed = Chart::parse(format!(
            "{}[ExpertDoubleBass]\n{{\n {} = N {} 0\n}}\n",
            chart().raw,
            selected[0].tick,
            (selected[0].lane + 1) % 5
        ))
        .unwrap();
        assert!(
            candidate_score_issues(&changed, &events, Some(&generated), &Policy::default())
                .unwrap()
                .iter()
                .all(|issue| issue.reason != Reason::WeakAttackSupport),
            "a different played gem cannot inherit source evidence"
        );
        let map = generation_map(&mapped, &events, 2., BTreeMap::new(), generated).unwrap();
        assert!(map.checks_performed.contains(&Reason::WeakAttackSupport));
    }

    #[test]
    fn generated_bass_corrections_are_pending_and_never_assigned_to_authored_bass() {
        let chart = Chart::parse(format!(
            "{}[ExpertDoubleBass]\n{{\n 960 = N 2 0\n}}\n",
            chart().raw
        ))
        .unwrap();
        let note = PitchNote {
            start: 0.5,
            end: 0.8,
            pitch: 40,
            confidence: 0.8,
        };
        let events = Transcription {
            bass: vec![note.clone()],
            bass_corrections: vec![crate::mapping::BassCorrection {
                original: PitchNote {
                    start: 0.57,
                    pitch: 52,
                    ..note.clone()
                },
                revised_pitch: 40,
                periodic_pitch: 40.01,
                clarity: 0.99,
            }],
            bass_timing_corrections: vec![crate::mapping::BassTimingCorrection {
                original: PitchNote {
                    start: 0.57,
                    ..note
                },
                revised_start: 0.5,
                before_rms: 0.001,
                after_rms: 0.1,
                periodic_pitch: 40.01,
            }],
            ..Default::default()
        };
        let map = generation_map(
            &chart,
            &events,
            2.,
            BTreeMap::new(),
            BTreeSet::from(["ExpertDoubleBass".into()]),
        )
        .unwrap();
        let issues: Vec<_> = map
            .regions
            .iter()
            .flat_map(|region| &region.issues)
            .collect();
        assert!(
            issues
                .iter()
                .any(|issue| issue.reason == Reason::PitchDisagreement)
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.reason == Reason::StemAttackTiming)
        );
        assert!(map.checks_performed.contains(&Reason::StemAttackTiming));
        let authored =
            generation_map(&chart, &events, 2., BTreeMap::new(), BTreeSet::new()).unwrap();
        assert!(authored.regions.is_empty());
    }

    #[test]
    fn generation_markers_use_audio_seconds_and_skip_authored_candidate_scores() {
        let chart = Chart::parse(format!(
            "{}[ExpertSingle]\n{{\n 960 = N 3 0\n}}\n[ExpertDrums]\n{{\n 960 = N 0 0\n}}\n",
            chart().raw
        ))
        .unwrap();
        let events = Transcription {
            guitar: vec![PitchNote {
                start: 0.5,
                end: 0.8,
                pitch: 60,
                confidence: 0.3,
            }],
            drums: vec![crate::mapping::DrumHit {
                time: 0.5,
                voice: crate::mapping::Voice::Kick,
                confidence: 0.4,
            }],
            ..Default::default()
        };
        let generated = BTreeSet::from(["ExpertDrums".into()]);
        let map = generation_map(&chart, &events, 2.0, BTreeMap::new(), generated).unwrap();
        assert_eq!(map.regions.len(), 1);
        let issues = &map.regions[0].issues;
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].track, "ExpertDrums");
        assert_eq!(issues[0].start, 0.5);
        assert_eq!(issues[0].reason, Reason::LowCandidateScore);
        assert!(!map.checks_performed.contains(&Reason::PitchDisagreement));
        assert!(
            candidate_score_issues(&chart, &events, None, &Policy::default())
                .unwrap()
                .is_empty()
        );
        let notes = vec![
            Note {
                tick: 960,
                lane: 7,
                sustain: 0,
            },
            Note {
                tick: 970,
                lane: 7,
                sustain: 0,
            },
        ];
        assert_eq!(
            track_check(&notes, &chart, false)
                .same_lane_under_50ms
                .len(),
            1,
            "open notes must be checked too"
        );
    }

    #[test]
    fn review_render_uses_chart_offset_in_audio_seconds() {
        let notes = vec![Note {
            tick: 960,
            lane: 0,
            sustain: 0,
        }];
        let signal = drum_render(&notes, &chart(), 0.25, 22050);
        assert!(signal[..11025].iter().all(|v| *v == 0.0));
        assert!(signal[11025..11500].iter().any(|v| v.abs() > 0.1));
    }

    #[test]
    fn review_flags_unmatched_hits_on_silent_audio_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let song = root.path().join("song");
        fs::create_dir(&song).unwrap();
        fs::write(
            song.join("notes.chart"),
            format!("{}[ExpertDrums]\n{{\n 960 = N 1 0\n}}\n", chart().raw),
        )
        .unwrap();
        fs::write(song.join("chart-engine-transcription.json"), "{}").unwrap();
        models::write_audio(&song.join("song.wav"), &vec![0.0; 44100], &vec![0.0; 44100]).unwrap();
        let output = root.path().join("review");
        let report = run(&song, &song.join("song.wav"), None, &output).unwrap();
        assert_eq!(
            report["drum_notes_over_50ms_from_independent_attack"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(
            report["drum_notes_over_50ms_from_independent_attack"][0]["nearest_attack_ms"]
                .is_null()
        );
        assert!(output.join("drums-00.mp3").exists());
        assert!(output.join("recording.mp3").exists());
        let markers: ReviewMap =
            serde_json::from_slice(&fs::read(output.join("review-regions.json")).unwrap()).unwrap();
        assert_eq!(markers.regions[0].issues[0].start, 0.5);
        assert_eq!(
            markers.regions[0].issues[0].reason,
            Reason::WeakAttackSupport
        );
        assert_eq!(
            markers.input_sha256["chart"],
            models::hash_file(&song.join("notes.chart")).unwrap()
        );
        let html = fs::read_to_string(output.join("review.html")).unwrap();
        assert!(html.contains(&format!("id='{}'", markers.regions[0].id)));
        assert!(html.contains("recording.mp3#t=0.000,1.000"));
        let before = fs::read(output.join("audit.json")).unwrap();
        assert!(run(&song, &song.join("song.wav"), None, &output).is_err());
        assert_eq!(before, fs::read(output.join("audit.json")).unwrap());
        let truncated = root.path().join("truncated.wav");
        models::write_audio(&truncated, &vec![0.0; 22050], &vec![0.0; 22050]).unwrap();
        let rejected = root.path().join("rejected");
        assert!(
            run(&song, &truncated, None, &rejected)
                .unwrap_err()
                .to_string()
                .contains("duration")
        );
        assert!(!rejected.exists());
    }
}
