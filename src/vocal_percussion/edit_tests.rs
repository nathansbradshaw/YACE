use super::*;
use crate::vocal_percussion::{append_midi, import_midi};
use crate::vocals::VocalPhrase;

fn attributes(zero_length: bool) -> crate::song_files::game_midi::NoteAttributes {
    crate::song_files::game_midi::NoteAttributes {
        channel: 4,
        velocity: 88,
        off_velocity: 12,
        off_as_note_on: false,
        zero_length,
        talkie_pitch: None,
    }
}

fn phrase(start: u64, end: u64) -> Phrase {
    Phrase {
        range: VocalPhrase {
            start,
            end,
            player: 1,
        },
        midi: None,
    }
}
fn hit(start: u64) -> Hit {
    Hit {
        start,
        end: start + 60,
        displayed: true,
        midi: None,
    }
}
fn put(part: Part, start: u64, phrase: Option<Phrase>) -> Edit {
    Edit::PutHit {
        part,
        index: None,
        expected: None,
        hit: hit(start),
        phrase,
    }
}
fn document() -> PercussionChart {
    put(Part::Lead, 120, Some(phrase(0, 960)))
        .apply(None, 480)
        .unwrap()
        .unwrap()
}
#[test]
fn first_hit_requires_explicit_owner_phrase_and_clock() {
    assert!(put(Part::Lead, 120, None).apply(None, 480).is_err());
    assert!(
        put(Part::Lead, 120, Some(phrase(240, 960)))
            .apply(None, 480)
            .is_err()
    );
    let chart = document();
    assert_eq!(chart.tracks.len(), 1);
    assert!(put(Part::Lead, 240, None).apply(Some(&chart), 960).is_err());
    assert_eq!(
        import_midi(
            &append_midi(
                &crate::game_export::song(
                    &crate::chart::Chart::parse(
                        "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n".into()
                    )
                    .unwrap(),
                    None
                )
                .unwrap()
                .0,
                &chart
            )
            .unwrap()
        )
        .unwrap()
        .unwrap()
        .tracks[&Part::Lead]
            .hits[0]
            .key(),
        96
    );
}
#[test]
fn hit_edits_preserve_other_owner_and_refuse_stale_selection_and_duplicates() {
    let chart = put(Part::Harm1, 240, Some(phrase(0, 960)))
        .apply(Some(&document()), 480)
        .unwrap()
        .unwrap();
    let expected = chart.tracks[&Part::Lead].hits[0].clone();
    let mut changed = expected.clone();
    changed.start += 60;
    changed.end += 60;
    changed.displayed = false;
    let edit = Edit::PutHit {
        part: Part::Lead,
        index: Some(0),
        expected: Some(expected.clone()),
        hit: changed.clone(),
        phrase: None,
    };
    let next = edit.apply(Some(&chart), 480).unwrap().unwrap();
    assert_eq!(next.tracks[&Part::Lead].hits, [changed]);
    assert_eq!(next.tracks[&Part::Harm1], chart.tracks[&Part::Harm1]);
    assert!(edit.apply(Some(&next), 480).is_err());
    assert!(
        put(Part::Lead, expected.start, None)
            .apply(Some(&chart), 480)
            .is_err()
    );
    assert_eq!(chart.tracks[&Part::Lead].hits, [expected]);
}
#[test]
fn deleting_last_hit_prunes_only_its_dedicated_phrase_and_owner() {
    let first = document();
    let chart = put(Part::Lead, 1200, Some(phrase(960, 1920)))
        .apply(Some(&first), 480)
        .unwrap()
        .unwrap();
    let edit = Edit::RemoveHit {
        part: Part::Lead,
        index: 0,
        expected: chart.tracks[&Part::Lead].hits[0].clone(),
    };
    let next = edit.apply(Some(&chart), 480).unwrap().unwrap();
    assert_eq!(next.tracks[&Part::Lead].phrases, [phrase(960, 1920)]);
    assert!(edit.apply(Some(&next), 480).is_err());
    assert!(
        Edit::RemoveHit {
            part: Part::Lead,
            index: 0,
            expected: next.tracks[&Part::Lead].hits[0].clone()
        }
        .apply(Some(&next), 480)
        .unwrap()
        .is_none()
    );
}
#[test]
fn phrase_edits_do_not_move_hits_or_accept_uncovered_spans() {
    let chart = document();
    let original = chart.tracks[&Part::Lead].phrases[0].clone();
    let edit = Edit::PutPhrase {
        part: Part::Lead,
        index: 0,
        expected: original.clone(),
        phrase: phrase(0, 480),
    };
    let next = edit.apply(Some(&chart), 480).unwrap().unwrap();
    assert_eq!(
        next.tracks[&Part::Lead].hits,
        chart.tracks[&Part::Lead].hits
    );
    assert!(edit.apply(Some(&next), 480).is_err());
    assert!(
        Edit::PutPhrase {
            part: Part::Lead,
            index: 0,
            expected: original,
            phrase: phrase(180, 480)
        }
        .apply(Some(&chart), 480)
        .is_err()
    );
}
#[test]
fn split_and_merge_keep_hits_and_require_nonempty_compatible_phrases() {
    let chart = put(Part::Lead, 600, None)
        .apply(Some(&document()), 480)
        .unwrap()
        .unwrap();
    let expected = chart.tracks[&Part::Lead].phrases[0].clone();
    let split = Edit::SplitPhrase {
        part: Part::Lead,
        index: 0,
        expected: expected.clone(),
        tick: 480,
    };
    let next = split.apply(Some(&chart), 480).unwrap().unwrap();
    assert_eq!(
        next.tracks[&Part::Lead].hits,
        chart.tracks[&Part::Lead].hits
    );
    assert_eq!(
        next.tracks[&Part::Lead].phrases,
        [phrase(0, 480), phrase(480, 960)]
    );
    assert!(split.apply(Some(&next), 480).is_err());
    assert_eq!(
        Edit::MergePhrase {
            part: Part::Lead,
            index: 0,
            expected: phrase(0, 480),
            next_expected: phrase(480, 960)
        }
        .apply(Some(&next), 480)
        .unwrap()
        .unwrap(),
        chart
    );
    for tick in [0, 150, 900, 960] {
        assert!(
            Edit::SplitPhrase {
                part: Part::Lead,
                index: 0,
                expected: expected.clone(),
                tick
            }
            .apply(Some(&chart), 480)
            .is_err()
        );
    }
}
#[test]
fn merge_refuses_different_players_and_marker_attributes_without_losing_them() {
    let chart = put(Part::Lead, 1200, Some(phrase(960, 1920)))
        .apply(Some(&document()), 480)
        .unwrap()
        .unwrap();
    for differing_attributes in [false, true] {
        let mut different = chart.clone();
        let track = different.tracks.get_mut(&Part::Lead).unwrap();
        if differing_attributes {
            track.phrases[1].midi = Some(attributes(false));
        } else {
            track.phrases[1].range.player = 2;
        }
        let edit = Edit::MergePhrase {
            part: Part::Lead,
            index: 0,
            expected: track.phrases[0].clone(),
            next_expected: track.phrases[1].clone(),
        };
        assert!(edit.apply(Some(&different), 480).is_err());
        assert_ne!(different, chart);
    }
}
#[test]
fn zero_length_hit_edits_retain_physical_duration_and_hidden_encoding() {
    let chart = document();
    let expected = chart.tracks[&Part::Lead].hits[0].clone();
    let mut changed = expected.clone();
    changed.end = changed.start;
    changed.displayed = false;
    changed.midi = Some(attributes(true));
    let next = Edit::PutHit {
        part: Part::Lead,
        index: Some(0),
        expected: Some(expected),
        hit: changed.clone(),
        phrase: None,
    }
    .apply(Some(&chart), 480)
    .unwrap()
    .unwrap();
    assert_eq!(next.tracks[&Part::Lead].hits[0], changed);
    assert_eq!(changed.key(), 97);
}
#[test]
fn moving_the_last_hit_to_a_new_phrase_is_atomic_and_does_not_hide_invalid_requested_ranges() {
    let chart = document();
    let original = chart.tracks[&Part::Lead].hits[0].clone();
    let edit = Edit::PutHit {
        part: Part::Lead,
        index: Some(0),
        expected: Some(original.clone()),
        hit: hit(1200),
        phrase: Some(phrase(960, 1920)),
    };
    let next = edit.apply(Some(&chart), 480).unwrap().unwrap();
    assert_eq!(next.tracks[&Part::Lead].phrases, [phrase(960, 1920)]);
    assert_eq!(
        chart.tracks[&Part::Lead].hits.as_slice(),
        std::slice::from_ref(&original)
    );
    assert!(edit.apply(Some(&next), 480).is_err());
    assert!(
        Edit::PutHit {
            part: Part::Lead,
            index: Some(0),
            expected: Some(original),
            hit: hit(1200),
            phrase: Some(phrase(1920, 2880))
        }
        .apply(Some(&chart), 480)
        .is_err()
    );
}
