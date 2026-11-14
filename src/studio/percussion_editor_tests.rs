use super::*;
use crate::{song_files::game_midi::NoteAttributes, timing::Tempo};
#[test]
fn hit_form_preserves_midi_attributes_and_updates_physical_zero_length_intent() {
    let original = Hit {
        start: 240,
        end: 260,
        displayed: false,
        midi: Some(NoteAttributes {
            channel: 6,
            velocity: 89,
            off_velocity: 14,
            off_as_note_on: false,
            zero_length: false,
            talkie_pitch: None,
        }),
    };
    let phrase = Phrase {
        range: VocalPhrase {
            midi: None,
            start: 0,
            end: 960,
            player: 2,
        },
        midi: None,
    };
    let mut draft = Draft::new(Some(&original), Some(&phrase), 0, 480);
    assert_eq!(draft.hit(Some(&original)).unwrap(), original);
    draft.end = "240".into();
    draft.displayed = true;
    let edited = draft.hit(Some(&original)).unwrap();
    assert_eq!(edited.start, edited.end);
    assert!(edited.displayed);
    let attributes = edited.midi.unwrap();
    assert!(attributes.zero_length);
    assert_eq!(
        (
            attributes.channel,
            attributes.velocity,
            attributes.off_velocity
        ),
        (6, 89, 14)
    );
    assert_eq!(draft.phrase(Some(&phrase)).unwrap(), phrase);
    draft.start = "-1".into();
    assert!(draft.hit(Some(&original)).is_err());
    draft.start = "300".into();
    assert!(draft.hit(Some(&original)).is_err());
}
#[test]
fn long_percussion_preview_keeps_original_indices_and_allocates_only_the_clock_window() {
    let hits: Vec<_> = (0..100_000)
        .map(|i| Hit {
            start: i * 480,
            end: i * 480,
            displayed: i % 2 == 0,
            midi: None,
        })
        .collect();
    let timing = TempoMap::new(480, 0., vec![Tempo { tick: 0, bpm: 120. }]).unwrap();
    let dots = visible(&hits, &timing, 20_000., 4.);
    assert_eq!(dots.len(), 11);
    assert_eq!(dots[0].index, 39_998);
    assert_eq!(dots[0].x, 92.);
    assert_eq!(dots.last().unwrap().x, 982.);
    assert!(dots.iter().all(|dot| dot.x == dot.end));
    assert_eq!(dots.iter().filter(|dot| dot.active).count(), 1);
}
#[test]
fn an_inserted_earlier_hit_does_not_retarget_the_selected_form_or_its_edit() {
    use crate::vocal_percussion::{PercussionChart, Track};
    let hit = |start| Hit {
        start,
        end: start + 20,
        displayed: true,
        midi: None,
    };
    let chart = PercussionChart {
        version: 1,
        resolution: 480,
        source: "Selection fixture".into(),
        tracks: [(
            Part::Lead,
            Track {
                source: "Manual lead".into(),
                phrases: vec![Phrase {
                    range: VocalPhrase {
                        midi: None,
                        start: 0,
                        end: 960,
                        player: 1,
                    },
                    midi: None,
                }],
                hits: vec![hit(240), hit(480)],
            },
        )]
        .into(),
    };
    let selected = (1, hit(480));
    assert_eq!(
        witness_index(&chart.tracks[&Part::Lead].hits, Some(&selected)),
        Some(1)
    );
    let next = Edit::PutHit {
        part: Part::Lead,
        index: None,
        expected: None,
        hit: hit(120),
        phrase: None,
    }
    .apply(Some(&chart), 480)
    .unwrap()
    .unwrap();
    assert_eq!(
        witness_index(&next.tracks[&Part::Lead].hits, Some(&selected)),
        None
    );
    assert_eq!(next.tracks[&Part::Lead].hits[2], selected.1);
    assert!(
        Edit::RemoveHit {
            part: Part::Lead,
            index: selected.0,
            expected: selected.1.clone()
        }
        .apply(Some(&next), 480)
        .is_err()
    );
    assert_eq!(selected.1.start, 480);
}
