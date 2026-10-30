//! Source-independent octave/register arrangement for the 25-key game keyboard.
//! Original detections are never rewritten. Every adaptation has source evidence.
use chart_engine::{chart::Note, mapping::PitchNote, timing::TempoMap};
const WINDOWS: [(u8, u8, u8); 6] = [
    (0, 48, 64),
    (2, 49, 66),
    (4, 51, 68),
    (5, 53, 70),
    (7, 54, 71),
    (9, 56, 72),
];
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const RULE_ID: &str = "pro-keys-register-v3";

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
        let mut best: Option<(f64, u8, Vec<(usize, u8)>)> = None;
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
                let previous = previous_sources
                    .iter()
                    .filter(|(pitch, (at, _))| {
                        candidate.pitch.abs_diff(**pitch) <= 7
                            && timing.seconds(tick) - timing.seconds(*at) <= 1.5
                    })
                    .min_by_key(|(pitch, (at, _))| {
                        (candidate.pitch.abs_diff(**pitch), std::cmp::Reverse(*at))
                    });
                let nearby_future: Vec<_> = ordered
                    .iter()
                    .map(|i| &source[*i])
                    .filter(|n| {
                        n.start > candidate.start + 0.035
                            && n.start <= candidate.start + 1.5
                            && n.pitch.abs_diff(candidate.pitch) <= 7
                    })
                    .take(16)
                    .collect();
                let (score, target) = options(candidate.pitch, low, high)
                    .map(|target| {
                        let mut score =
                            (i16::from(target) - i16::from(candidate.pitch)).abs() as f64;
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
                                let error = options(next.pitch, low, high)
                                    .map(|future| {
                                        ((i16::from(future) - i16::from(target)) - interval).abs()
                                    })
                                    .min()
                                    .unwrap();
                                error
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
                    // Two different source octaves can collapse onto one game
                    // key. Preserve their common attack rather than inventing
                    // an unplayable double hit from subframe onset jitter.
                    // Real repeated attacks of the SAME source key are kept.
                    if !source_pitches[old].contains(&source[index].pitch)
                        && timing.seconds(tick) - timing.seconds(notes[old].tick) <= 0.035
                    {
                        notes[old].sustain =
                            notes[old].sustain.max(tick + duration - notes[old].tick);
                        decisions[index].output_tick = Some(notes[old].tick);
                        decisions[index].disposition = "merged_octave_collision".into();
                        source_pitches[old].insert(source[index].pitch);
                        previous_sources.insert(source[index].pitch, (tick, target));
                        continue;
                    }
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
            previous_sources.insert(source[index].pitch, (tick, target));
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
        chart_engine::key_ranges::plan(&notes)?.uncovered.is_empty(),
        "Pro Piano arrangement has uncovered sounding pitches"
    );
    Ok(Arrangement {
        rule_id: RULE_ID.into(),
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
