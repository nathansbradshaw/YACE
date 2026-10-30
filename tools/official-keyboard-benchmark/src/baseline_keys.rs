//! Piano-source arrangements. Pro Keys stores actual MIDI pitches, never frets.
//! The ProKeys sections are YACE working data; game exports require notes.mid.
use anyhow::Result;
use chart_engine::{chart::Note, mapping::PitchNote, timing::TempoMap};
use std::collections::{BTreeMap, BTreeSet};

pub const MIN_PITCH: u8 = 48;
pub const MAX_PITCH: u8 = 72;

pub fn is_pro(track: &str) -> bool {
    track.ends_with("ProKeys")
}
pub fn is_keys(track: &str) -> bool {
    is_pro(track) || track.ends_with("Keyboard")
}
pub fn midi_track(track: &str) -> Option<&'static str> {
    match track {
        "EasyProKeys" => Some("PART REAL_KEYS_E"),
        "MediumProKeys" => Some("PART REAL_KEYS_M"),
        "HardProKeys" => Some("PART REAL_KEYS_H"),
        "ExpertProKeys" => Some("PART REAL_KEYS_X"),
        _ => None,
    }
}
pub fn pitch_name(pitch: u8) -> &'static str {
    const LABELS: [&str; 25] = [
        "C3", "C♯3", "D3", "D♯3", "E3", "F3", "F♯3", "G3", "G♯3", "A3", "A♯3", "B3", "C4", "C♯4",
        "D4", "D♯4", "E4", "F4", "F♯4", "G4", "G♯4", "A4", "A♯4", "B4", "C5",
    ];
    pitch
        .checked_sub(MIN_PITCH)
        .and_then(|i| LABELS.get(i as usize))
        .copied()
        .unwrap_or("Outside Pro Keys range")
}

/// Five-lane arrangement, or a shared Expert-derived Pro Keys arrangement.
pub fn arrange(
    notes: &[PitchNote],
    timing: &TempoMap,
    tier: usize,
    pro: bool,
) -> Result<Vec<Note>> {
    if pro {
        return Ok(chart_engine::pro_keys::reduce(
            &chart_engine::pro_keys::arrange(notes, timing)?,
            timing,
            tier,
        ));
    }
    let mut attacks: BTreeMap<u64, Vec<&PitchNote>> = BTreeMap::new();
    for note in notes.iter().filter(|n| n.confidence >= 0.20) {
        if let Some(tick) = timing.tick(note.start) {
            attacks.entry(tick).or_default().push(note);
        }
    }
    let mut pitches: Vec<_> = notes
        .iter()
        .filter(|n| n.confidence >= 0.20)
        .map(|n| n.pitch)
        .collect();
    pitches.sort_unstable();
    let low = pitches.first().copied().unwrap_or(0);
    let high = pitches.last().copied().unwrap_or(low);
    let min_gap = [0.30, 0.20, 0.10, 0.0][tier.min(3)];
    let chord_limit = [1, 2, 3, 5][tier.min(3)];
    let mut last = None;
    let mut result: Vec<Note> = Vec::new();
    let mut represented: Vec<BTreeSet<u8>> = Vec::new();
    let mut recent = BTreeMap::<u8, usize>::new();
    for (tick, mut group) in attacks {
        if last.is_some_and(|previous| timing.seconds(tick) - timing.seconds(previous) < min_gap) {
            continue;
        }
        group.sort_by(|a, b| {
            b.confidence
                .total_cmp(&a.confidence)
                .then(b.pitch.cmp(&a.pitch))
        });
        let mut lanes = BTreeMap::<u8, (u64, BTreeSet<u8>)>::new();
        for note in group {
            let lane = if low == high {
                2
            } else {
                ((note.pitch - low) as u16 * 4 / (high - low) as u16) as u8
            };
            let sustain = timing
                .tick(note.end)
                .unwrap_or(tick + 1)
                .saturating_sub(tick)
                .max(1);
            if let Some((duration, pitches)) = lanes.get_mut(&lane) {
                *duration = (*duration).max(sustain);
                pitches.insert(note.pitch);
                continue;
            }
            if lanes.len() == chord_limit {
                continue;
            }
            lanes.insert(lane, (sustain, BTreeSet::from([note.pitch])));
        }
        for (lane, (sustain, pitches)) in lanes {
            if let Some(&old) = recent.get(&lane) {
                if result[old].tick + result[old].sustain > tick
                    && timing.seconds(tick) - timing.seconds(result[old].tick) <= 0.035
                    && represented[old].is_disjoint(&pitches)
                {
                    result[old].sustain =
                        result[old].sustain.max(tick + sustain - result[old].tick);
                    represented[old].extend(pitches);
                    continue;
                }
            }
            recent.insert(lane, result.len());
            result.push(Note {
                tick,
                lane,
                sustain,
            });
            represented.push(pitches);
        }
        last = Some(tick);
    }
    result.sort_by_key(|n| (n.lane, n.tick));
    for i in 0..result.len().saturating_sub(1) {
        if result[i].lane == result[i + 1].lane {
            result[i].sustain = result[i].sustain.min(result[i + 1].tick - result[i].tick);
        }
    }
    result.sort();
    Ok(result)
}
