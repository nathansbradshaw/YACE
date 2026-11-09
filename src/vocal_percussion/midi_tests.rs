use super::*;
use midly::{
    Header, Timing,
    num::{u4, u7, u15, u24, u28},
};
type Event = (u32, K<'static>);
fn note(channel: u8, key: u8, velocity: u8, on: bool) -> K<'static> {
    let key = u7::new(key);
    let vel = u7::new(velocity);
    K::Midi {
        channel: u4::new(channel),
        message: if on {
            N::NoteOn { key, vel }
        } else {
            N::NoteOff { key, vel }
        },
    }
}
fn percussion(label: &'static str) -> Vec<Event> {
    vec![
        (0, K::Meta(M::TrackName(label.as_bytes()))),
        (0, note(7, 105, 90, true)),
        (960, note(7, 105, 7, false)),
        (240, note(3, 96, 86, true)),
        (260, note(3, 96, 9, false)),
        (480, note(6, 97, 87, true)),
        (480, note(6, 97, 0, true)),
        (1920, K::Meta(M::EndOfTrack)),
    ]
}
fn singing(label: &'static str, start: u32, end: u32) -> Vec<Event> {
    vec![
        (0, K::Meta(M::TrackName(label.as_bytes()))),
        (start, note(4, 60, 88, true)),
        (end, note(4, 60, 5, false)),
        (1920, K::Meta(M::EndOfTrack)),
    ]
}
fn encode(mut tracks: Vec<Vec<Event>>) -> Vec<u8> {
    tracks.insert(
        0,
        vec![
            (0, K::Meta(M::TrackName(b"TEMPO"))),
            (0, K::Meta(M::Tempo(u24::new(500000)))),
            (1920, K::Meta(M::EndOfTrack)),
        ],
    );
    let tracks = tracks
        .into_iter()
        .map(|mut track| {
            track.sort_by_key(|event| event.0);
            let mut last = 0;
            track
                .into_iter()
                .map(|(tick, kind)| {
                    let delta = u28::new(tick - last);
                    last = tick;
                    TrackEvent { delta, kind }
                })
                .collect()
        })
        .collect();
    let mut output = Vec::new();
    Smf {
        header: Header::new(Format::Parallel, Timing::Metrical(u15::new(480))),
        tracks,
    }
    .write_std(&mut output)
    .unwrap();
    output
}
#[test]
fn dedicated_source_hits_keep_hidden_zero_length_channels_velocities_and_phrase_attributes() {
    let chart = import_midi(&encode(vec![percussion("PART VOCALS")]))
        .unwrap()
        .unwrap();
    let track = &chart.tracks[&Part::Lead];
    assert_eq!(chart.resolution, 480);
    assert_eq!(track.phrases.len(), 1);
    let phrase = &track.phrases[0];
    assert_eq!(
        (phrase.range.start, phrase.range.end, phrase.range.player),
        (0, 960, 1)
    );
    assert_eq!(phrase.midi.as_ref().unwrap().channel, 7);
    assert_eq!(phrase.midi.as_ref().unwrap().off_velocity, 7);
    assert_eq!(track.hits.len(), 2);
    let hit = &track.hits[0];
    assert_eq!((hit.start, hit.end, hit.key()), (240, 260, 96));
    let attrs = hit.midi.as_ref().unwrap();
    assert_eq!(
        (attrs.channel, attrs.velocity, attrs.off_velocity),
        (3, 86, 9)
    );
    let hidden = &track.hits[1];
    assert_eq!((hidden.start, hidden.end, hidden.key()), (480, 480, 97));
    let attrs = hidden.midi.as_ref().unwrap();
    assert_eq!(
        (attrs.channel, attrs.velocity, attrs.off_velocity),
        (6, 87, 0)
    );
    assert!(attrs.zero_length && attrs.off_as_note_on);
}
#[test]
fn duplicate_aliases_and_ambiguous_vocal_names_refuse_instead_of_combining_parts() {
    let duplicate = encode(vec![percussion("HARM1"), percussion("PART HARM1")]);
    assert!(
        import_midi(&duplicate)
            .unwrap_err()
            .to_string()
            .contains("Duplicate")
    );
    let mut ambiguous = percussion("PART VOCALS");
    ambiguous.push((0, K::Meta(M::TrackName(b"HARM1"))));
    assert!(
        import_midi(&encode(vec![ambiguous]))
            .unwrap_err()
            .to_string()
            .contains("Ambiguous")
    );
}
#[test]
fn singing_and_lyrics_cannot_be_reclassified_as_a_percussion_phrase() {
    let mut mixed = percussion("PART VOCALS");
    mixed.extend([(720, note(2, 60, 87, true)), (840, note(2, 60, 9, false))]);
    assert!(
        import_midi(&encode(vec![mixed]))
            .unwrap_err()
            .to_string()
            .contains("singing")
    );
    let mut lyrics_only = percussion("PART VOCALS");
    lyrics_only.push((720, K::Meta(M::Lyric(b"Keep this word"))));
    assert!(
        import_midi(&encode(vec![lyrics_only]))
            .unwrap_err()
            .to_string()
            .contains("Lyrics")
    );
}
#[test]
fn harm1_percussion_respects_other_singers_even_when_harm1_has_no_pitched_notes() {
    let mut first = percussion("PART HARM1");
    first.extend([
        (960, note(7, 105, 90, true)),
        (1920, note(7, 105, 7, false)),
    ]);
    let valid = import_midi(&encode(vec![first.clone(), singing("HARM3", 1200, 1440)]))
        .unwrap()
        .unwrap();
    assert_eq!(valid.tracks[&Part::Harm1].phrases.len(), 1);
    let mixed = encode(vec![first.clone(), singing("PART HARM2", 600, 720)]);
    assert!(
        import_midi(&mixed)
            .unwrap_err()
            .to_string()
            .contains("singing")
    );
    let point_at_start = encode(vec![first.clone(), singing("HARM3", 0, 0)]);
    assert!(import_midi(&point_at_start).is_err());
    let point_at_end = encode(vec![first, singing("HARM3", 960, 960)]);
    import_midi(&point_at_end).unwrap().unwrap();
}
#[test]
fn uncovered_hits_and_unterminated_or_overlapping_source_keys_refuse_without_guessing() {
    let mut uncovered = percussion("PART VOCALS");
    uncovered.iter_mut().find(|event| event.0 == 260).unwrap().0 = 1000;
    assert!(import_midi(&encode(vec![uncovered])).is_err());
    let mut unmatched = percussion("PART VOCALS");
    unmatched.retain(|event| event.0 != 260);
    assert!(
        import_midi(&encode(vec![unmatched]))
            .unwrap_err()
            .to_string()
            .contains("Unterminated")
    );
    let mut overlapping = percussion("PART VOCALS");
    overlapping.push((200, note(3, 96, 88, true)));
    assert!(
        import_midi(&encode(vec![overlapping]))
            .unwrap_err()
            .to_string()
            .contains("Overlapping")
    );
}
#[test]
fn a_singing_only_source_creates_no_percussion_document() {
    let mut lead = singing("PART VOCALS", 240, 480);
    lead.extend([(0, note(0, 105, 100, true)), (960, note(0, 105, 0, false))]);
    assert!(import_midi(&encode(vec![lead])).unwrap().is_none());
}
