use super::*;
use crate::song_files::game_midi;
use midly::{
    Format, Header, MetaMessage as M, MidiMessage as N, Smf, Timing, TrackEvent,
    TrackEventKind as K,
    num::{u4, u7, u15, u28},
};

fn note(on: bool, key: u8, velocity: u8) -> K<'static> {
    K::Midi {
        channel: u4::new(5),
        message: if on {
            N::NoteOn {
                key: u7::new(key),
                vel: u7::new(velocity),
            }
        } else {
            N::NoteOff {
                key: u7::new(key),
                vel: u7::new(velocity),
            }
        },
    }
}
fn track(name: &'static [u8], mut events: Vec<(u64, K<'static>)>) -> Vec<TrackEvent<'static>> {
    events.insert(0, (0, K::Meta(M::TrackName(name))));
    events.push((8000, K::Meta(M::EndOfTrack)));
    events.sort_by_key(|e| (e.0, matches!(e.1, K::Meta(M::EndOfTrack))));
    let mut previous = 0;
    events
        .into_iter()
        .map(|(tick, kind)| {
            let delta = u28::new((tick - previous) as u32);
            previous = tick;
            TrackEvent { delta, kind }
        })
        .collect()
}
fn encode(format: Format, tracks: Vec<Vec<TrackEvent<'static>>>) -> Vec<u8> {
    let mut bytes = vec![];
    Smf {
        header: Header::new(format, Timing::Metrical(u15::new(480))),
        tracks,
    }
    .write_std(&mut bytes)
    .unwrap();
    bytes
}
fn fixture() -> Vec<u8> {
    encode(
        Format::Parallel,
        vec![
            track(
                b"PART DRUMS",
                vec![
                    (120, K::Meta(M::Text(b"mix_2_drums3d"))),
                    (240, K::Meta(M::Text(b"[mix 3 drums3d]"))),
                    (480, K::Meta(M::Text(b"[mix 3 drums3]"))),
                    (600, K::Meta(M::Text(b"mix_2_drums3"))),
                    (1920, K::Meta(M::Text(b"mix_1_drums3dnoflip"))),
                    (240, note(true, 97, 127)),
                    (360, note(false, 97, 23)),
                    (0, note(true, 116, 93)),
                    (960, note(false, 116, 21)),
                    (123, K::Meta(M::Text(b"[author custom]"))),
                    (124, K::Meta(M::Text(b"\xff opaque"))),
                ],
            ),
            track(
                b"PART GUITAR",
                vec![(0, note(true, 96, 79)), (120, note(false, 96, 19))],
            ),
            track(
                b"VENUE",
                vec![(42, K::Meta(M::Text(b"Original venue sentinel")))],
            ),
        ],
    )
}
fn changed(before: &State) -> State {
    let mut after = before.clone();
    let previous = ranges(&before.events, 3).unwrap()[0].clone();
    after.events = edited(
        &before.events,
        3,
        &Edit::Put {
            previous: Some(previous),
            start: 720,
            end: 960,
            configuration: 3,
        },
    )
    .unwrap();
    after
}
#[test]
fn drum_mix_midi_noop_is_exact_and_edits_keep_raw_owners_notes_and_other_mix_flags() {
    let bytes = fixture();
    let before = import_midi(&bytes).unwrap();
    assert_eq!(
        rewrite_midi(&bytes, &hash(&bytes), &before, &before).unwrap(),
        bytes
    );
    let after = changed(&before);
    let output = rewrite_midi(&bytes, &hash(&bytes), &before, &after).unwrap();
    let actual = import_midi(&output).unwrap();
    assert_eq!(actual.events, after.events);
    let (_, old) = game_midi::midi_chunks(&bytes, 3).unwrap();
    let (_, new) = game_midi::midi_chunks(&output, 3).unwrap();
    assert_eq!(old[1..], new[1..]);
    let retained = |bytes: &[u8]| {
        let midi = Smf::parse(bytes).unwrap();
        game_midi::midi_events(&midi.tracks[0]).into_iter().filter(|(_,k)|!matches!(k,K::Meta(M::Text(b)) if std::str::from_utf8(b).ok().and_then(|s|parse(s).ok()).flatten().is_some())).map(|(t,k)|(t,format!("{k:?}"))).collect::<Vec<_>>()
    };
    assert_eq!(retained(&bytes), retained(&output));
    assert_eq!(
        before
            .events
            .iter()
            .filter(|e| e.mix.difficulty != 3)
            .collect::<Vec<_>>(),
        actual
            .events
            .iter()
            .filter(|e| e.mix.difficulty != 3)
            .collect::<Vec<_>>()
    );
}
#[test]
fn drum_mix_midi_clear_retains_declared_configuration_and_other_difficulty() {
    let bytes = fixture();
    let before = import_midi(&bytes).unwrap();
    let mut after = before.clone();
    let range = ranges(&before.events, 3).unwrap()[0].clone();
    after.events = edited(&before.events, 3, &Edit::Remove { range }).unwrap();
    let output = rewrite_midi(&bytes, &hash(&bytes), &before, &after).unwrap();
    let actual = import_midi(&output).unwrap();
    assert!(ranges(&actual.events, 3).unwrap().is_empty());
    assert_eq!(ranges(&actual.events, 2).unwrap().len(), 1);
    assert!(actual.events.iter().all(|e| e.mix.configuration == 3));
}
#[test]
fn drum_mix_midi_first_marker_only_owner_keeps_single_track_source_raw() {
    let bytes = encode(
        Format::SingleTrack,
        vec![track(
            b"PART GUITAR",
            vec![(0, note(true, 96, 91)), (120, note(false, 96, 13))],
        )],
    );
    let before = import_midi(&bytes).unwrap();
    let mut after = before.clone();
    after.events = edited(
        &[],
        3,
        &Edit::Put {
            previous: None,
            start: 240,
            end: 480,
            configuration: 0,
        },
    )
    .unwrap();
    let output = rewrite_midi(&bytes, &hash(&bytes), &before, &after).unwrap();
    assert_eq!(Smf::parse(&output).unwrap().header.format, Format::Parallel);
    let (_, old) = game_midi::midi_chunks(&bytes, 1).unwrap();
    let (_, new) = game_midi::midi_chunks(&output, 2).unwrap();
    assert_eq!(old[0], new[0]);
    assert_eq!(import_midi(&output).unwrap().events, after.events);
}
#[test]
fn drum_mix_midi_changed_input_binding_or_mix_witness_refuses() {
    let bytes = fixture();
    let before = import_midi(&bytes).unwrap();
    let after = changed(&before);
    assert!(rewrite_midi(&bytes, &"0".repeat(64), &before, &after).is_err());
    let mut wrong = after.clone();
    wrong.source_sha256 = None;
    assert!(rewrite_midi(&bytes, &hash(&bytes), &before, &wrong).is_err());
    let mut stale = before.clone();
    stale.events[0].tick += 1;
    assert!(rewrite_midi(&bytes, &hash(&bytes), &stale, &after).is_err());
    let mut invalid = after;
    invalid.events[0].mix.configuration = 4;
    assert!(rewrite_midi(&bytes, &hash(&bytes), &before, &invalid).is_err());
}
#[test]
fn drum_mix_midi_unsupported_flags_aliases_duplicates_and_endings_refuse_editable_view() {
    let bad = encode(
        Format::Parallel,
        vec![track(
            b"PART DRUMS",
            vec![(0, K::Meta(M::Text(b"mix 3 drums0future")))],
        )],
    );
    assert!(import_midi(&bad).is_err());
    let alias = encode(Format::Parallel, vec![track(b"PART DRUM", vec![])]);
    assert!(import_midi(&alias).is_err());
    let duplicate = encode(
        Format::Parallel,
        vec![track(b"PART DRUMS", vec![]), track(b"PART DRUMS", vec![])],
    );
    assert!(import_midi(&duplicate).is_err());
    let mut broken = track(b"PART DRUMS", vec![]);
    broken.push(TrackEvent {
        delta: 0.into(),
        kind: K::Meta(M::EndOfTrack),
    });
    assert!(import_midi(&encode(Format::Parallel, vec![broken])).is_err());
}
