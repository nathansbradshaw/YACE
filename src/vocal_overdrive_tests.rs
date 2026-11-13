use super::*;
use midly::{
    Header, Timing,
    num::{u15, u28},
};

fn on(channel: u8, key: u8, velocity: u8) -> Event<'static> {
    Event::Midi {
        channel: u4::new(channel),
        message: Note::NoteOn {
            key: u7::new(key),
            vel: u7::new(velocity),
        },
    }
}
fn off(channel: u8, key: u8, velocity: u8) -> Event<'static> {
    Event::Midi {
        channel: u4::new(channel),
        message: Note::NoteOff {
            key: u7::new(key),
            vel: u7::new(velocity),
        },
    }
}
fn track(name: &'static [u8], mut events: Vec<(u64, Event<'static>)>) -> Vec<TrackEvent<'static>> {
    events.insert(0, (0, Event::Meta(Meta::TrackName(name))));
    events.push((8000, Event::Meta(Meta::EndOfTrack)));
    events.sort_by_key(|e| e.0);
    let mut previous = 0;
    events
        .into_iter()
        .map(|(tick, kind)| {
            let delta = tick - previous;
            previous = tick;
            TrackEvent {
                delta: u28::new(delta as u32),
                kind,
            }
        })
        .collect()
}
fn encode(tracks: Vec<Vec<TrackEvent<'static>>>) -> Vec<u8> {
    let mut output = Vec::new();
    Smf {
        header: Header::new(Format::Parallel, Timing::Metrical(u15::new(480))),
        tracks,
    }
    .write_std(&mut output)
    .unwrap();
    output
}
pub(super) fn original() -> Vec<u8> {
    encode(vec![
        track(
            b"PART VOCALS",
            vec![
                (0, on(2, 60, 89)),
                (240, off(2, 60, 17)),
                (0, Event::Meta(Meta::Lyric(b"Alpha"))),
                (0, on(3, 105, 81)),
                (480, off(3, 105, 21)),
                (0, on(4, 116, 87)),
                (240, off(4, 116, 9)),
                (720, on(5, 116, 76)),
                (960, on(5, 116, 0)),
                (2208, on(2, 96, 93)),
                (2208, off(2, 96, 0)),
                (
                    100,
                    Event::Midi {
                        channel: u4::new(7),
                        message: Note::Controller {
                            controller: u7::new(10),
                            value: u7::new(48),
                        },
                    },
                ),
            ],
        ),
        track(
            b"PART HARM1",
            vec![
                (0, on(3, 64, 91)),
                (240, off(3, 64, 18)),
                (0, Event::Meta(Meta::Lyric(b"Beta"))),
                (0, on(3, 105, 81)),
                (480, off(3, 105, 21)),
                (960, on(6, 116, 71)),
                (1440, off(6, 116, 19)),
            ],
        ),
        track(b"HARM2", vec![(0, on(1, 116, 99)), (480, off(1, 116, 23))]),
        track(
            b"PART GUITAR",
            vec![
                (0, on(1, 116, 77)),
                (1920, off(1, 116, 22)),
                (0, on(0, 96, 100)),
                (120, off(0, 96, 0)),
            ],
        ),
    ])
}

#[test]
fn vocal_overdrive_reads_lead_and_shared_harm1_with_exact_attributes() {
    let bytes = original();
    let state = import_midi(&bytes).unwrap();
    assert_eq!(state.source_sha256, Some(hash(&bytes)));
    assert_eq!(state.tracks.len(), 2);
    let lead = &state.tracks[&Part::Lead];
    assert_eq!(lead.phrases.len(), 2);
    assert_eq!(lead.phrases[0].midi.as_ref().unwrap().channel, 4);
    assert_eq!(lead.phrases[0].midi.as_ref().unwrap().velocity, 87);
    assert_eq!(lead.phrases[0].midi.as_ref().unwrap().off_velocity, 9);
    assert!(lead.phrases[1].midi.as_ref().unwrap().off_as_note_on);
    assert_eq!(state.tracks[&Part::Harm1].owner, "PART HARM1");
    assert_eq!(
        rewrite_midi(&bytes, &hash(&bytes), &state, &state).unwrap(),
        bytes
    );
}

#[test]
fn vocal_overdrive_move_resize_keeps_other_markers_voices_controls_and_raw_owners() {
    let bytes = original();
    let before = import_midi(&bytes).unwrap();
    let previous = before.tracks[&Part::Lead].phrases[1].clone();
    let after = edited(
        &before,
        Part::Lead,
        &Edit::Put {
            previous: Some(previous.clone()),
            range: Range {
                start: 840,
                end: 1200,
            },
        },
    )
    .unwrap();
    let output = rewrite_midi(&bytes, &hash(&bytes), &before, &after).unwrap();
    let imported = import_midi(&output).unwrap();
    assert!(same(&imported, &after));
    assert_eq!(imported.tracks[&Part::Lead].phrases[1].midi, previous.midi);
    let (_, old_chunks) = game_midi::midi_chunks(&bytes, 4).unwrap();
    let (_, new_chunks) = game_midi::midi_chunks(&output, 4).unwrap();
    assert_eq!(old_chunks[1..], new_chunks[1..]);
    let non_power = |bytes: &[u8]| {
        let midi = Smf::parse(bytes).unwrap();
        game_midi::midi_events(&midi.tracks[0])
            .into_iter()
            .filter(|(tick, event)| edge(*tick, *event).is_none())
            .map(|(tick, e)| (tick, format!("{e:?}")))
            .collect::<Vec<_>>()
    };
    assert_eq!(non_power(&bytes), non_power(&output));
    assert!(rewrite_midi(&output, &hash(&bytes), &before, &after).is_err());
    assert!(rewrite_midi(&output, &hash(&output), &before, &after).is_err());
}

#[test]
fn vocal_overdrive_explicit_clear_and_dedicated_owner_edits_remain_independent() {
    let bytes = original();
    let before = import_midi(&bytes).unwrap();
    let mut after = before.clone();
    for track in after.tracks.values_mut() {
        track.phrases.clear();
    }
    let output = rewrite_midi(&bytes, &hash(&bytes), &before, &after).unwrap();
    let cleared = import_midi(&output).unwrap();
    assert!(cleared.tracks.values().all(|t| t.phrases.is_empty()));
    let (_, old) = game_midi::midi_chunks(&bytes, 4).unwrap();
    let (_, new) = game_midi::midi_chunks(&output, 4).unwrap();
    assert_eq!(old[2..], new[2..]);
    let after = edited(
        &before,
        Part::Harm1,
        &Edit::Put {
            previous: None,
            range: Range {
                start: 1920,
                end: 2400,
            },
        },
    )
    .unwrap();
    let output = rewrite_midi(&bytes, &hash(&bytes), &before, &after).unwrap();
    let (_, new) = game_midi::midi_chunks(&output, 4).unwrap();
    assert_eq!(old[0], new[0]);
    assert_eq!(old[2..], new[2..]);
    assert_eq!(
        import_midi(&output).unwrap().tracks[&Part::Harm1]
            .phrases
            .len(),
        2
    );
}

#[test]
fn vocal_overdrive_new_owner_default_attributes_and_adjacent_boundaries_roundtrip() {
    let bytes = encode(vec![track(
        b"PART GUITAR",
        vec![(0, on(0, 96, 100)), (240, off(0, 96, 0))],
    )]);
    let before = import_midi(&bytes).unwrap();
    let mut after = before.clone();
    for (start, end) in [(0, 480), (480, 960)] {
        after = edited(
            &after,
            Part::Harm1,
            &Edit::Put {
                previous: None,
                range: Range { start, end },
            },
        )
        .unwrap();
    }
    let output = rewrite_midi(&bytes, &hash(&bytes), &before, &after).unwrap();
    let (_, old) = game_midi::midi_chunks(&bytes, 1).unwrap();
    let (_, new) = game_midi::midi_chunks(&output, 2).unwrap();
    assert_eq!(old[0], new[0]);
    let state = import_midi(&output).unwrap();
    let phrases = &state.tracks[&Part::Harm1].phrases;
    assert_eq!(phrases.len(), 2);
    assert_eq!(phrases[0].range.end, phrases[1].range.start);
    assert!(same(&state, &after));
    assert_eq!(phrases[0].midi.as_ref().unwrap().velocity, 100);
}

#[test]
fn vocal_overdrive_rejects_stale_overlap_changed_binding_and_incomplete_source() {
    let bytes = original();
    let before = import_midi(&bytes).unwrap();
    let saved = serde_json::to_vec(&before).unwrap();
    let old = before.tracks[&Part::Lead].phrases[0].clone();
    assert!(
        edited(
            &before,
            Part::Lead,
            &Edit::Put {
                previous: Some(old),
                range: Range { start: 0, end: 800 }
            }
        )
        .is_err()
    );
    let mut stale = before.tracks[&Part::Lead].phrases[0].clone();
    stale.range.end = 241;
    assert!(edited(&before, Part::Lead, &Edit::Remove { phrase: stale }).is_err());
    assert_eq!(saved, serde_json::to_vec(&before).unwrap());
    let mut foreign = before.clone();
    foreign.source_sha256 = Some("1".repeat(64));
    assert!(rewrite_midi(&bytes, &hash(&bytes), &before, &foreign).is_err());
    let unclosed = encode(vec![track(b"PART VOCALS", vec![(0, on(0, 116, 100))])]);
    assert!(import_midi(&unclosed).is_err());
    let empty = encode(vec![track(
        b"PART VOCALS",
        vec![(0, on(0, 116, 100)), (0, off(0, 116, 0))],
    )]);
    assert!(import_midi(&empty).is_err());
    let overlapping = encode(vec![track(
        b"PART VOCALS",
        vec![
            (0, on(0, 116, 100)),
            (120, on(1, 116, 70)),
            (240, off(0, 116, 0)),
            (480, off(1, 116, 0)),
        ],
    )]);
    assert!(import_midi(&overlapping).is_err());
    let aliases = encode(vec![track(b"HARM1", vec![]), track(b"PART HARM1", vec![])]);
    assert!(import_midi(&aliases).is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(import_midi(&trailing).is_err());
}
