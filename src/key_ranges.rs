//! Pro Keys display ranges checked against YARG's ProKeysPlayer visible windows.
//! Pitches stay intact. Wide voicings are reported for review rather than folded.
use crate::{
    chart::Note,
    keys::{MAX_PITCH, MIN_PITCH},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{cmp::Reverse, collections::BTreeMap};

/// MIDI marker pitch, lowest visible playable pitch, highest visible pitch.
pub(crate) const WINDOWS: [(u8, u8, u8); 6] = [
    (0, 48, 64),
    (2, 49, 66),
    (4, 51, 68),
    (5, 53, 70),
    (7, 54, 71),
    (9, 56, 72),
];

#[derive(Clone, Debug, Serialize)]
pub struct Uncovered {
    pub tick: u64,
    pub pitches: Vec<u8>,
    pub visible_low: u8,
    pub visible_high: u8,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Plan {
    pub markers: Vec<Note>,
    pub uncovered: Vec<Uncovered>,
}

/// Prefer a range that covers every sounding pitch, including sustained notes.
/// Otherwise maximize covered voices, then new attacks, retaining the previous
/// view on ties. Every omitted sounding pitch is returned as review evidence.
pub fn plan(notes: &[Note]) -> Result<Plan> {
    let mut attacks: BTreeMap<u64, BTreeMap<u8, u64>> = BTreeMap::new();
    for note in notes {
        ensure!(
            (MIN_PITCH..=MAX_PITCH).contains(&note.lane),
            "Invalid Pro Keys pitch {}",
            note.lane
        );
        let end = note
            .tick
            .checked_add(note.sustain.max(1))
            .context("Pro Keys tick overflow")?;
        ensure!(
            attacks
                .entry(note.tick)
                .or_default()
                .insert(note.lane, end)
                .is_none(),
            "Duplicate Pro Keys pitch at tick {}",
            note.tick
        );
    }
    let mut result = Plan::default();
    let mut active = BTreeMap::new();
    let mut previous = 0u8;
    for (tick, incoming) in attacks {
        active.retain(|_, end| *end > tick);
        active.extend(incoming.iter().map(|(pitch, end)| (*pitch, *end)));
        let (marker, low, high) = WINDOWS
            .into_iter()
            .max_by_key(|&(marker, low, high)| {
                let covers = |pitch: &&u8| (low..=high).contains(*pitch);
                (
                    active.keys().filter(covers).count(),
                    incoming.keys().filter(covers).count(),
                    Reverse(marker.abs_diff(previous)),
                    Reverse(marker),
                )
            })
            .unwrap();
        if result.markers.is_empty() || marker != previous {
            result.markers.push(Note {
                tick: if result.markers.is_empty() { 0 } else { tick },
                lane: marker,
                sustain: 1,
            });
        }
        let pitches: Vec<_> = active
            .keys()
            .filter(|pitch| !(low..=high).contains(*pitch))
            .copied()
            .collect();
        if !pitches.is_empty() {
            result.uncovered.push(Uncovered {
                tick,
                pitches,
                visible_low: low,
                visible_high: high,
            });
        }
        previous = marker;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(tick: u64, pitch: u8, sustain: u64) -> Note {
        Note {
            tick,
            lane: pitch,
            sustain,
        }
    }

    #[test]
    fn high_register_starts_visible_and_changes_only_when_needed() {
        let high = plan(&[n(480, 72, 100), n(600, 69, 100)]).unwrap();
        assert_eq!(high.markers, [n(0, 9, 1)]);
        assert!(high.uncovered.is_empty());
        let shifted = plan(&[n(0, 48, 100), n(100, 72, 100)]).unwrap();
        assert_eq!(shifted.markers, [n(0, 0, 1), n(100, 9, 1)]);
        assert!(shifted.uncovered.is_empty());
    }

    #[test]
    fn sustains_influence_choice_and_wide_voicings_retain_explicit_misses() {
        let held = plan(&[n(0, 49, 200), n(100, 66, 100)]).unwrap();
        assert_eq!(held.markers, [n(0, 0, 1), n(100, 2, 1)]);
        assert!(held.uncovered.is_empty());
        let impossible = plan(&[n(0, 48, 200), n(100, 72, 100)]).unwrap();
        assert_eq!(impossible.uncovered.len(), 1);
        assert_eq!(impossible.uncovered[0].tick, 100);
        assert_eq!(impossible.uncovered[0].pitches, [48]);
        assert_eq!(impossible.uncovered[0].visible_high, 72);
        let chord = plan(&[n(0, 48, 100), n(0, 60, 100), n(0, 72, 100)]).unwrap();
        assert_eq!(chord.uncovered[0].pitches, [72]);
    }

    #[test]
    fn invalid_data_is_rejected_without_modifying_notes() {
        assert!(plan(&[n(0, 47, 100)]).is_err());
        assert!(plan(&[n(u64::MAX, 60, 1)]).is_err());
        assert!(plan(&[n(0, 60, 1), n(0, 60, 2)]).is_err());
        assert!(plan(&[]).unwrap().markers.is_empty());
    }
}
