//! Musical event comparison and bounded passage navigation, shared by both clients.
use crate::{
    editor::{Content, TimeRange},
    timing::TempoMap,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difference {
    Unchanged,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub start: f64,
    pub end: f64,
    pub lane: u8,
    pub cymbal: bool,
    pub detail: String,
    pub difference: Difference,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comparison {
    pub events: Vec<Event>,
}

fn events(content: &Content, track: &str, timing: &TempoMap) -> Vec<(String, Event)> {
    if track == "Vocals" {
        return content
            .vocals
            .iter()
            .flat_map(|v| &v.notes)
            .map(|note| {
                let lane = match note.tone {
                    crate::vocals::VocalTone::Pitched { midi } => midi,
                    _ => 0,
                };
                (
                    serde_json::to_string(note).expect("serializable vocal note"),
                    Event {
                        start: timing.seconds(note.start),
                        end: timing.seconds(note.end),
                        lane,
                        cymbal: false,
                        detail: match &note.lyric {
                            crate::vocals::Syllable::Text { text, .. } => text.clone(),
                            crate::vocals::Syllable::Missing => "Lyric unknown".into(),
                            crate::vocals::Syllable::Continue => "Slide".into(),
                            crate::vocals::Syllable::ContinueWord => "Continued syllable".into(),
                        },
                        difference: Difference::Unchanged,
                    },
                )
            })
            .collect();
    }
    let notes = content
        .tracks
        .get(track)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut by_tick = BTreeMap::<u64, Vec<&crate::chart::Note>>::new();
    for item in notes
        .iter()
        .filter(|item| !crate::editor::playable(track, item.note.lane))
    {
        by_tick.entry(item.note.tick).or_default().push(&item.note);
    }
    notes
        .iter()
        .filter(|n| {
            crate::editor::playable(track, n.note.lane)
                || (track.ends_with("Drums") && n.note.lane == 32)
        })
        .map(|item| {
            let note = &item.note;
            // Modifier changes must be visible on their playable note, rather than
            // disappearing as hidden control lanes. IDs are deliberately excluded.
            let modifiers: Vec<_> = by_tick
                .get(&note.tick)
                .into_iter()
                .flatten()
                .filter(|n| {
                    if track.ends_with("Drums") {
                        n.lane == note.lane + 64
                    } else {
                        crate::note_types::supported(track) && matches!(n.lane, 5 | 6)
                    }
                })
                .copied()
                .collect();
            let cymbal = track.ends_with("Drums") && !modifiers.is_empty();
            let detail = if cymbal {
                "Cymbal"
            } else if track.ends_with("Drums") && note.lane > 0 && note.lane < 5 {
                "Pad"
            } else if modifiers.iter().any(|n| n.lane == 6) {
                "Tap"
            } else if modifiers.iter().any(|n| n.lane == 5) {
                "Forced"
            } else {
                "Note"
            };
            (
                serde_json::to_string(&(&item.note, &item.midi, &modifiers))
                    .expect("serializable note"),
                Event {
                    start: timing.seconds(note.tick),
                    end: timing.seconds(note.tick.saturating_add(note.sustain)),
                    lane: note.lane,
                    cymbal,
                    detail: detail.into(),
                    difference: Difference::Unchanged,
                },
            )
        })
        .collect()
}

impl Comparison {
    pub fn new(before: &Content, after: &Content, track: &str, timing: &TempoMap) -> Self {
        let before = events(before, track, timing);
        let after = events(after, track, timing);
        let mut remaining = BTreeMap::<String, usize>::new();
        for (key, _) in &after {
            *remaining.entry(key.clone()).or_default() += 1;
        }
        let mut result = Vec::new();
        for (key, mut event) in before {
            let count = remaining.entry(key).or_default();
            if *count > 0 {
                *count -= 1;
            } else {
                event.difference = Difference::Removed;
            }
            result.push(event);
        }
        for (key, mut event) in after {
            let count = remaining.entry(key).or_default();
            if *count > 0 {
                *count -= 1;
                event.difference = Difference::Added;
                result.push(event);
            }
        }
        result.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.lane.cmp(&b.lane)));
        Self { events: result }
    }

    /// Group nearby edits into passages; navigation skips unchanged stretches.
    pub fn passages(&self, range: TimeRange) -> Vec<f64> {
        let mut starts = Vec::new();
        let mut last = f64::NEG_INFINITY;
        for event in self.events.iter().filter(|e| {
            e.difference != Difference::Unchanged && e.start >= range.start && e.start < range.end
        }) {
            if event.start - last > 2.
                || starts.last().is_some_and(|start| event.start - start >= 6.)
            {
                starts.push(event.start);
            }
            last = event.start;
        }
        starts
    }

    pub fn lanes(&self, track: &str) -> Vec<u8> {
        if track == "Vocals" || crate::keys::is_pro(track) {
            let lanes: BTreeSet<_> = self.events.iter().map(|e| e.lane).collect();
            return lanes.into_iter().rev().collect();
        }
        let mut rows = crate::timeline::lane_rows(track);
        if track.ends_with("Drums") && self.events.iter().any(|e| e.lane == 32) {
            rows.push(32);
        }
        rows
    }
}

pub fn window(range: TimeRange, start: f64, span: f64) -> TimeRange {
    let span = span.min(range.end - range.start).max(0.001);
    let start = start.clamp(range.start, (range.end - span).max(range.start));
    TimeRange {
        start,
        end: (start + span).min(range.end),
    }
}

/// Clip sustains at both edges; zero-length attacks retain a readable width.
pub fn geometry(event: &Event, view: TimeRange, width: f64) -> Option<(f64, f64)> {
    if event.start >= view.end || event.end.max(event.start + 0.001) <= view.start {
        return None;
    }
    let scale = width / (view.end - view.start);
    let x = (event.start.max(view.start) - view.start) * scale;
    let end = (event.end.min(view.end) - view.start) * scale;
    Some((x, (end - x).max(8.).min(width - x)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{chart::Note, editor::EditorNote};
    fn content(values: &[(u64, u8, u64)]) -> Content {
        let mut c = Content::default();
        c.tracks.insert(
            "ExpertDrums".into(),
            values
                .iter()
                .enumerate()
                .map(|(i, &(tick, lane, sustain))| EditorNote {
                    id: format!("{i}"),
                    note: Note {
                        tick,
                        lane,
                        sustain,
                    },
                    midi: None,
                })
                .collect(),
        );
        c
    }
    #[test]
    fn diff_ignores_ids_but_detects_sustain_and_cymbal_changes() {
        let a = content(&[(0, 1, 0), (480, 2, 0), (480, 66, 0), (960, 3, 0)]);
        let mut b = content(&[(0, 1, 0), (480, 2, 0), (960, 3, 480)]);
        for n in b.tracks.values_mut().flatten() {
            n.id.push_str("-regenerated");
        }
        let diff = Comparison::new(&a, &b, "ExpertDrums", &TempoMap::constant(120.).unwrap());
        assert_eq!(
            diff.events
                .iter()
                .filter(|e| e.difference == Difference::Unchanged)
                .count(),
            1
        );
        assert_eq!(
            diff.events
                .iter()
                .filter(|e| e.difference == Difference::Added)
                .count(),
            2
        );
        assert_eq!(
            diff.events
                .iter()
                .filter(|e| e.difference == Difference::Removed)
                .count(),
            2
        );
        assert!(
            diff.events
                .iter()
                .any(|e| e.cymbal && e.difference == Difference::Removed)
        );
    }
    #[test]
    fn duplicates_and_passages_are_counted_without_losing_events() {
        let a = content(&[(0, 0, 0), (0, 0, 0), (9600, 1, 0)]);
        let b = content(&[(0, 0, 0), (4800, 1, 0)]);
        let diff = Comparison::new(&a, &b, "ExpertDrums", &TempoMap::constant(120.).unwrap());
        assert_eq!(diff.events.len(), 4);
        assert_eq!(
            diff.passages(TimeRange {
                start: 0.,
                end: 12.
            }),
            vec![0., 5., 10.]
        );
    }
    #[test]
    fn window_and_sustains_stay_inside_selected_range() {
        let range = TimeRange {
            start: 20.,
            end: 24.,
        };
        assert_eq!(window(range, 100., 8.), range);
        let event = Event {
            start: 18.,
            end: 22.,
            lane: 0,
            cymbal: false,
            detail: String::new(),
            difference: Difference::Removed,
        };
        assert_eq!(geometry(&event, range, 800.), Some((0., 400.)));
        assert_eq!(
            geometry(
                &event,
                TimeRange {
                    start: 22.,
                    end: 24.
                },
                800.
            ),
            None
        );
    }

    #[test]
    fn dense_changes_still_have_multiple_navigation_targets() {
        let after = content(&(0..40).map(|i| (i * 480, 1, 0)).collect::<Vec<_>>());
        let diff = Comparison::new(
            &Content::default(),
            &after,
            "ExpertDrums",
            &TempoMap::constant(120.).unwrap(),
        );
        assert_eq!(
            diff.passages(TimeRange {
                start: 0.,
                end: 20.
            }),
            vec![0., 6., 12., 18.]
        );
    }

    #[test]
    fn pitched_tracks_keep_octaves_and_lyric_only_edits() {
        use crate::vocals::{Syllable, VocalChart, VocalNote, VocalTone};
        let vocal = |pitch| VocalNote {
            start: 480,
            end: 960,
            tone: VocalTone::Pitched { midi: pitch },
            lyric: Syllable::Missing,
            midi: None,
        };
        let mut before = Content::default();
        before.vocals = Some(VocalChart {
            version: 1,
            resolution: 480,
            source: "Fixture".into(),
            phrases: vec![],
            notes: vec![vocal(48), vocal(60)],
        });
        let mut after = before.clone();
        after.vocals.as_mut().unwrap().notes[0].lyric = Syllable::Text {
            text: "Word".into(),
            join_next: false,
        };
        let diff = Comparison::new(
            &before,
            &after,
            "Vocals",
            &TempoMap::constant(120.).unwrap(),
        );
        assert_eq!(diff.lanes("Vocals"), vec![60, 48]);
        assert_eq!(
            diff.events
                .iter()
                .filter(|e| e.difference != Difference::Unchanged)
                .count(),
            2
        );
        assert!(
            diff.events
                .iter()
                .any(|e| e.detail == "Word" && e.difference == Difference::Added)
        );
        let mut keys = content(&[(0, 48, 480), (480, 60, 0), (960, 72, 0)]);
        let notes = keys.tracks.remove("ExpertDrums").unwrap();
        keys.tracks.insert("ExpertProKeys".into(), notes);
        assert_eq!(
            Comparison::new(
                &Content::default(),
                &keys,
                "ExpertProKeys",
                &TempoMap::constant(120.).unwrap()
            )
            .lanes("ExpertProKeys"),
            vec![72, 60, 48]
        );
    }
}
