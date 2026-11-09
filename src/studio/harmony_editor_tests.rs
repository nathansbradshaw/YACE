use super::*;
#[test]
fn first_singer_form_uses_a_full_beat_outside_dedicated_percussion_without_moving_hits() {
    let blocked: Vec<_> = [(0, 960), (1200, 1440)]
        .into_iter()
        .map(|(start, end)| crate::vocal_percussion::Phrase {
            range: VocalPhrase {
                start,
                end,
                player: 1,
            },
            midi: None,
        })
        .collect();
    assert_eq!(first_free_tick(0, 480, &blocked), 1440);
    assert_eq!(first_free_tick(960, 240, &blocked), 960);
    assert_eq!(first_free_tick(1500, 480, &blocked), 1500);
    assert_eq!(blocked[0].range.start, 0);
}
#[test]
fn new_harm3_draft_uses_the_overlap_of_shared_score_and_lyric_ranges() {
    let ranges = RetainedHarmonyPhrases {
        scoring: vec![
            VocalPhrase {
                start: 0,
                end: 960,
                player: 1,
            },
            VocalPhrase {
                start: 1920,
                end: 2880,
                player: 1,
            },
        ],
        lyrics: vec![VocalPhrase {
            start: 1800,
            end: 2880,
            player: 1,
        }],
    };
    let (note, score, lyric) = initial(Part::Harm3, 480, &[], &ranges);
    let note = note.parse().unwrap();
    assert_eq!((note.start, note.end), (1920, 2160));
    assert_eq!(score.parse().unwrap(), ranges.scoring[1]);
    assert_eq!(lyric.parse().unwrap(), ranges.lyrics[0]);
}
#[test]
fn adding_after_an_existing_note_stays_inside_its_current_shared_range() {
    let ranges = RetainedHarmonyPhrases {
        scoring: vec![VocalPhrase {
            start: 0,
            end: 960,
            player: 1,
        }],
        lyrics: vec![],
    };
    let notes = [VocalNote {
        start: 240,
        end: 720,
        tone: VocalTone::Pitched { midi: 60 },
        lyric: Syllable::Missing,
        midi: None,
    }];
    let (note, score, _) = initial(Part::Harm1, 480, &notes, &ranges);
    let note = note.parse().unwrap();
    assert_eq!((note.start, note.end), (720, 960));
    assert_eq!(score.parse().unwrap(), ranges.scoring[0]);
}
