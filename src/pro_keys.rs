//! Source-independent octave/register arrangement for the 25-key game keyboard.
//! Original detections are never rewritten. Every adaptation has source evidence.
use crate::{chart::Note, key_ranges::WINDOWS, mapping::PitchNote, timing::TempoMap};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const RULE_ID: &str = "pro-keys-register-v4";

type ChordChoice = (f64, u8, Vec<(usize, u8)>);

#[derive(Clone, Debug, Serialize)]
pub struct Decision {
    pub source_index: usize,
    pub source: PitchNote,
    pub tick: Option<u64>,
    pub source_sustain: Option<u64>,
    pub target_pitch: Option<u8>,
    /// Actual emitted attack, which can precede an octave-collision merge.
    pub output_tick: Option<u64>,
    pub octave_shift: i16,
    pub output_sustain: Option<u64>,
    pub disposition: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Arrangement {
    pub rule_id: String,
    pub voicing_rule_id: String,
    pub filtered_short_candidates: usize,
    pub preferred_octave_shift: i16,
    pub notes: Vec<Note>,
    pub decisions: Vec<Decision>,
    pub octave_adapted_candidates: usize,
    pub merged_simultaneous_candidates: usize,
    pub merged_octave_collisions: usize,
    pub shortened_releases: usize,
    pub out_of_range_omitted: usize,
}

fn options(pitch: u8, low: u8, high: u8) -> impl Iterator<Item = u8> {
    (low..=high).filter(move |target| target % 12 == pitch % 12)
}

/// All new voices fit a window containing every still-held arranged voice.
/// Choose octave-equivalent pitches using movement, neighboring source intervals,
/// collisions and camera stability. Onsets stay in the original source clock.
pub fn arrange(source: &[PitchNote], timing: &TempoMap) -> Result<Arrangement> {
    arrange_with_register(source, timing, 0)
}

/// A compact detected lead is normalized from the common piano/lead C4–C6
/// register to controller C3–C5. Wide performances retain their natural
/// register preference; either path preserves raw acoustic pitches/evidence.
pub fn arrange_generated(source: &[PitchNote], timing: &TempoMap) -> Result<Arrangement> {
    arrange_with_register(source, timing, -12)
}

pub fn arrange_transcription(
    events: &crate::mapping::Transcription,
    timing: &TempoMap,
) -> Result<Arrangement> {
    if events.keyboard_detection.is_some() {
        arrange_generated(&events.pro_piano, timing)
    } else {
        arrange(&events.pro_piano, timing)
    }
}

fn arrange_with_register(
    source: &[PitchNote],
    timing: &TempoMap,
    preferred_shift: i16,
) -> Result<Arrangement> {
    ensure!(
        source.iter().all(|n| n.pitch <= 127
            && n.start.is_finite()
            && n.end.is_finite()
            && n.start >= 0.
            && n.end > n.start
            && n.confidence.is_finite()
            && (0.0..=1.0).contains(&n.confidence)),
        "Invalid Pro Piano candidate"
    );
    let eligible_indices = source
        .iter()
        .enumerate()
        .filter(|(_, n)| n.confidence >= 0.20 && timing.tick(n.start).is_some())
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let eligible = eligible_indices
        .iter()
        .map(|&i| source[i].clone())
        .collect::<Vec<_>>();
    let selection = crate::keyboard_voicing::select(&eligible);
    let voices = selection
        .voices
        .iter()
        .map(|v| v.note.clone())
        .collect::<Vec<_>>();
    let represented = selection
        .voices
        .iter()
        .map(|v| {
            v.source_indices
                .iter()
                .map(|&i| eligible[i].pitch)
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    let mut arranged = arrange_voices(&voices, &represented, timing, preferred_shift)?;
    let mut decisions = source
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let tick = timing.tick(n.start);
            Decision {
                source_index: i,
                source: n.clone(),
                tick,
                source_sustain: tick
                    .map(|t| timing.tick(n.end).unwrap_or(t).saturating_sub(t).max(1)),
                target_pitch: None,
                output_tick: None,
                octave_shift: 0,
                output_sustain: None,
                disposition: if n.confidence < 0.20 {
                    "below_confidence"
                } else if tick.is_none() {
                    "before_chart_start"
                } else {
                    "weak_short_isolated"
                }
                .into(),
            }
        })
        .collect::<Vec<_>>();
    let mut merged = 0;
    for (voice, d) in selection.voices.iter().zip(&arranged.decisions) {
        for (j, &local_index) in voice.source_indices.iter().enumerate() {
            let i = eligible_indices[local_index];
            let original = &mut decisions[i];
            original.target_pitch = d.target_pitch;
            original.output_tick = d.output_tick;
            original.output_sustain = d.output_sustain;
            original.octave_shift = d.target_pitch.map_or(0, |p| {
                (i16::from(p) - i16::from(original.source.pitch)) / 12
            });
            original.disposition = if j > 0 && d.target_pitch.is_some() {
                merged += 1;
                "merged_octave_collision".into()
            } else {
                d.disposition.clone()
            };
        }
    }
    arranged.decisions = decisions;
    arranged.voicing_rule_id = selection.rule_id;
    arranged.filtered_short_candidates = selection.weak_short_candidates.len();
    arranged.merged_octave_collisions += merged;
    arranged.octave_adapted_candidates = arranged
        .decisions
        .iter()
        .filter(|d| d.target_pitch.is_some() && d.octave_shift != 0)
        .count();
    arranged.shortened_releases=arranged.decisions.iter().filter(|d|matches!((d.tick,d.source_sustain,d.output_tick,d.output_sustain),(Some(t),Some(a),Some(o),Some(b)) if o+b<t+a)).count();
    Ok(arranged)
}

fn arrange_voices(
    source: &[PitchNote],
    represented: &[BTreeSet<u8>],
    timing: &TempoMap,
    preferred_shift: i16,
) -> Result<Arrangement> {
    let mut decisions = Vec::new();
    let mut attacks: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (index, note) in source.iter().enumerate() {
        ensure!(
            note.pitch <= 127
                && note.start.is_finite()
                && note.end.is_finite()
                && note.start >= 0.0
                && note.end > note.start
                && note.confidence.is_finite()
                && (0.0..=1.0).contains(&note.confidence),
            "Invalid Pro Piano candidate"
        );
        let tick = timing.tick(note.start);
        let duration = match tick {
            Some(tick) => Some(
                timing
                    .tick(note.end)
                    .context("Pro Piano end exceeds chart timing")?
                    .saturating_sub(tick)
                    .max(1),
            ),
            None => None,
        };
        if let (Some(tick), Some(duration)) = (tick, duration) {
            tick.checked_add(duration)
                .context("Pro Piano tick overflow")?;
        }
        let disposition = if note.confidence < 0.20 {
            "below_confidence"
        } else if tick.is_none() {
            "before_chart_start"
        } else {
            "pending"
        };
        decisions.push(Decision {
            source_index: index,
            source: note.clone(),
            tick,
            source_sustain: duration,
            target_pitch: None,
            output_tick: None,
            octave_shift: 0,
            output_sustain: None,
            disposition: disposition.into(),
        });
        if disposition == "pending" {
            attacks.entry(tick.unwrap()).or_default().push(index);
        }
    }
    let ordered: Vec<_> = attacks.values().flatten().copied().collect();
    let mut notes: Vec<Note> = Vec::new();
    let mut source_pitches: Vec<BTreeSet<u8>> = Vec::new();
    let mut active: BTreeMap<u8, usize> = BTreeMap::new();
    let mut previous_sources: BTreeMap<u8, (u64, u8)> = BTreeMap::new();
    let mut previous_marker = 0u8;
    for (tick, mut group) in attacks {
        active.retain(|_, index| notes[*index].tick + notes[*index].sustain > tick);
        group.sort_by(|a, b| {
            source[*b]
                .confidence
                .total_cmp(&source[*a].confidence)
                .then(source[*b].pitch.cmp(&source[*a].pitch))
                .then(a.cmp(b))
        });
        let mut best: Option<ChordChoice> = None;
        for (marker, low, high) in WINDOWS {
            if !active.keys().all(|pitch| (low..=high).contains(pitch)) {
                continue;
            }
            let mut used = BTreeMap::<u8, usize>::new();
            let mut placement = Vec::new();
            let mut cost = marker.abs_diff(previous_marker) as f64 * 0.6;
            // Reserve unchanged chord voices before fitting octave doublings.
            let mut priority = group.clone();
            priority.sort_by_key(|index| !(low..=high).contains(&source[*index].pitch));
            for &index in &priority {
                let candidate = &source[index];
                let register_shift = if preferred_shift == 0 {
                    0
                } else {
                    let low = source.partition_point(|n| n.start < candidate.start - 0.75);
                    let high = source.partition_point(|n| n.start <= candidate.start + 0.75);
                    let mut pitches = source[low..high]
                        .iter()
                        .filter(|n| n.confidence >= 0.65)
                        .map(|n| n.pitch)
                        .collect::<Vec<_>>();
                    pitches.sort_unstable();
                    if pitches.len() >= 4
                        && pitches[pitches.len() * 3 / 4] - pitches[pitches.len() / 4] > 12
                    {
                        0
                    } else {
                        preferred_shift
                    }
                };
                let previous = previous_sources
                    .iter()
                    .filter(|(pitch, (at, _))| {
                        candidate.pitch.abs_diff(**pitch) <= 7
                            && timing.seconds(tick) - timing.seconds(*at) <= 1.5
                    })
                    .min_by_key(|(pitch, (at, _))| {
                        (candidate.pitch.abs_diff(**pitch), std::cmp::Reverse(*at))
                    });
                let future_low =
                    ordered.partition_point(|&i| source[i].start <= candidate.start + 0.035);
                let future_high =
                    ordered.partition_point(|&i| source[i].start <= candidate.start + 1.5);
                let nearby_future: Vec<_> = ordered[future_low..future_high]
                    .iter()
                    .map(|i| &source[*i])
                    .filter(|n| n.pitch.abs_diff(candidate.pitch) <= 7)
                    .take(16)
                    .collect();
                let (score, target) = options(candidate.pitch, low, high)
                    .map(|target| {
                        let mut score = (i16::from(target)
                            - (i16::from(candidate.pitch) + register_shift))
                            .abs() as f64;
                        // The same detected key keeps its learned register
                        // through a phrase unless held voices make that
                        // placement impossible. Nearby different notes must
                        // not repeatedly flip this key between octaves.
                        if let Some(&(at, placed)) = previous_sources.get(&candidate.pitch)
                            && timing.seconds(tick) - timing.seconds(at) <= 2.0
                            && target != placed
                        {
                            score += if timing.seconds(tick) - timing.seconds(at) <= 0.035 {
                                1000.0
                            } else {
                                1.5
                            };
                        }
                        if used.contains_key(&target) {
                            score += 1000.0;
                        }
                        if let Some(&old) = active.get(&target) {
                            // Natural retriggers keep their source voice; avoid
                            // shortening an unrelated octave voice when possible.
                            let old_source = decisions.iter().find(|d| {
                                d.tick == Some(notes[old].tick) && d.target_pitch == Some(target)
                            });
                            if old_source.is_some_and(|d| d.source.pitch != candidate.pitch) {
                                score += 12.0;
                            }
                        }
                        if let Some((&pitch, &(_, placed))) = previous {
                            score += ((i16::from(target) - i16::from(placed))
                                - (i16::from(candidate.pitch) - i16::from(pitch)))
                            .abs() as f64
                                * 1.25;
                        }
                        let future_error = nearby_future
                            .iter()
                            .map(|next| {
                                let interval = i16::from(next.pitch) - i16::from(candidate.pitch);
                                options(next.pitch, low, high)
                                    .map(|future| {
                                        ((i16::from(future) - i16::from(target)) - interval).abs()
                                    })
                                    .min()
                                    .unwrap()
                            })
                            .max()
                            .unwrap_or(0);
                        score += future_error as f64 * 1.5;
                        (score, target)
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                    .unwrap();
                cost += score * f64::from(candidate.confidence.max(0.2));
                used.insert(target, index);
                placement.push((index, target));
            }
            if best.as_ref().is_none_or(|(old, _, _)| cost < *old) {
                best = Some((cost, marker, placement));
            }
        }
        let (_, marker, placement) =
            best.context("No keyboard window contains held Pro Piano voices")?;
        let mut at_tick = BTreeMap::<u8, usize>::new();
        for (index, target) in placement {
            let duration = decisions[index].source_sustain.unwrap();
            decisions[index].target_pitch = Some(target);
            decisions[index].output_tick = Some(tick);
            decisions[index].octave_shift =
                (i16::from(target) - i16::from(source[index].pitch)) / 12;
            if let Some(&slot) = at_tick.get(&target) {
                notes[slot].sustain = notes[slot].sustain.max(duration);
                source_pitches[slot].insert(source[index].pitch);
                decisions[index].disposition = "merged_simultaneous".into();
            } else {
                if let Some(&old) = active.get(&target) {
                    // Co-onset octave grouping already happened with the full
                    // represented pitch set. Every subsequent voice is a real
                    // new attack, including a repeat of a merged lower octave.
                    notes[old].sustain = notes[old].sustain.min(tick - notes[old].tick);
                }
                let slot = notes.len();
                notes.push(Note {
                    tick,
                    lane: target,
                    sustain: duration,
                });
                source_pitches.push(BTreeSet::from([source[index].pitch]));
                at_tick.insert(target, slot);
                active.insert(target, slot);
                decisions[index].disposition = "emitted".into();
            }
            for &pitch in &represented[index] {
                previous_sources.insert(pitch, (tick, target));
            }
        }
        previous_marker = marker;
    }
    notes.sort();
    let spans: BTreeMap<_, _> = notes
        .iter()
        .map(|n| ((n.tick, n.lane), n.sustain))
        .collect();
    for decision in &mut decisions {
        if let (Some(tick), Some(pitch)) = (decision.output_tick, decision.target_pitch) {
            decision.output_sustain = spans.get(&(tick, pitch)).copied();
        }
    }
    ensure!(
        crate::key_ranges::plan(&notes)?.uncovered.is_empty(),
        "Pro Piano arrangement has uncovered sounding pitches"
    );
    Ok(Arrangement {
        rule_id: RULE_ID.into(),
        voicing_rule_id: crate::keyboard_voicing::RULE_ID.into(),
        filtered_short_candidates: 0,
        preferred_octave_shift: preferred_shift / 12,
        octave_adapted_candidates: decisions.iter().filter(|d| d.octave_shift != 0).count(),
        merged_simultaneous_candidates: decisions
            .iter()
            .filter(|d| d.disposition == "merged_simultaneous")
            .count(),
        merged_octave_collisions: decisions
            .iter()
            .filter(|d| d.disposition == "merged_octave_collision")
            .count(),
        shortened_releases: decisions
            .iter()
            .filter(|d| matches!((d.source_sustain,d.output_sustain),(Some(a),Some(b)) if b<a))
            .count(),
        out_of_range_omitted: decisions
            .iter()
            .filter(|d| {
                d.source.confidence >= 0.20
                    && d.tick.is_some()
                    && !(48..=72).contains(&d.source.pitch)
                    && d.target_pitch.is_none()
            })
            .count(),
        notes,
        decisions,
    })
}

/// Lower tiers are subsets of the already fitted Expert arrangement. They
/// never independently change octaves or extend a held note across a shift.
pub fn reduce(arrangement: &Arrangement, timing: &TempoMap, tier: usize) -> Vec<Note> {
    if tier >= 3 {
        return arrangement.notes.clone();
    }
    let mut strengths = BTreeMap::<(u64, u8), f32>::new();
    for decision in &arrangement.decisions {
        if let (Some(tick), Some(pitch)) = (decision.output_tick, decision.target_pitch) {
            let strength = strengths.entry((tick, pitch)).or_default();
            *strength = strength.max(decision.source.confidence);
        }
    }
    let mut groups = BTreeMap::<u64, Vec<&Note>>::new();
    for note in &arrangement.notes {
        groups.entry(note.tick).or_default().push(note);
    }
    let mut previous = None;
    let mut result = Vec::new();
    for (tick, mut group) in groups {
        if previous.is_some_and(|old| {
            timing.seconds(tick) - timing.seconds(old) < [0.30, 0.20, 0.10][tier]
        }) {
            continue;
        }
        group.sort_by(|a, b| {
            strengths[&(b.tick, b.lane)]
                .total_cmp(&strengths[&(a.tick, a.lane)])
                .then(b.lane.cmp(&a.lane))
        });
        result.extend(group.into_iter().take(tier + 1).cloned());
        previous = Some(tick);
    }
    result.sort();
    result
}
