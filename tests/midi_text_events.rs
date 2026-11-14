use chart_engine::{
    midi_text_events as text,
    text_events::{Document, Edit, Scope},
    timing::TempoMap,
};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind as Kind,
    num::{u4, u7, u15, u28},
};
fn owner(name: &str) -> Scope {
    Scope::Physical { owner: name.into() }
}
fn track(name: &'static [u8], mut events: Vec<(u64, Kind<'static>)>) -> Vec<TrackEvent<'static>> {
    events.push((0, Kind::Meta(MetaMessage::TrackName(name))));
    events.push((2000, Kind::Meta(MetaMessage::EndOfTrack)));
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
    let midi = Smf {
        header: Header {
            format: if tracks.len() == 1 {
                Format::SingleTrack
            } else {
                Format::Parallel
            },
            timing: Timing::Metrical(u15::new(480)),
        },
        tracks,
    };
    let mut bytes = Vec::new();
    midi.write_std(&mut bytes).unwrap();
    bytes
}
fn note(on: bool) -> Kind<'static> {
    Kind::Midi {
        channel: u4::new(4),
        message: if on {
            MidiMessage::NoteOn {
                key: u7::new(96),
                vel: u7::new(89),
            }
        } else {
            MidiMessage::NoteOff {
                key: u7::new(96),
                vel: u7::new(19),
            }
        },
    }
}
fn fixture() -> Vec<u8> {
    encode(vec![
        track(
            b"TEMPO",
            vec![(0, Kind::Meta(MetaMessage::Tempo(500000.into())))],
        ),
        track(
            b"EVENTS",
            vec![
                (0, Kind::Meta(MetaMessage::Text(b"[section Intro]"))),
                (120, Kind::Meta(MetaMessage::Text(b"duplicate cue"))),
                (120, Kind::Meta(MetaMessage::Text(b"duplicate cue"))),
                (240, Kind::Meta(MetaMessage::Marker(b"opaque marker"))),
                (300, Kind::Meta(MetaMessage::Text(b"\xff opaque text"))),
            ],
        ),
        track(
            b"PART GUITAR",
            vec![
                (180, Kind::Meta(MetaMessage::Text(b"[play]"))),
                (200, Kind::Meta(MetaMessage::Lyric(b"keep lyric meta type"))),
                (243, note(true)),
                (320, note(false)),
                (400, Kind::Meta(MetaMessage::Text(b"[solo]"))),
                (800, Kind::Meta(MetaMessage::Text(b"[soloend]"))),
                (900, Kind::SysEx(b"\x01\x02\x03")),
            ],
        ),
        track(
            b"PART BASS",
            vec![
                (0, Kind::Meta(MetaMessage::Text(b"[idle]"))),
                (243, note(true)),
                (320, note(false)),
            ],
        ),
        track(
            b"HARM2",
            vec![
                (240, Kind::Meta(MetaMessage::Lyric(b"word"))),
                (243, note(true)),
                (320, note(false)),
            ],
        ),
        track(
            b"VENUE",
            vec![(77, Kind::Meta(MetaMessage::Text(b"unknown venue control")))],
        ),
    ])
}
#[derive(Debug, PartialEq, Eq)]
enum Owned {
    Text(Vec<u8>),
    Other(String),
}
fn events(bytes: &[u8], name: &[u8]) -> Vec<(u64, Owned)> {
    let midi = Smf::parse(bytes).unwrap();
    let track = midi
        .tracks
        .iter()
        .find(|t| {
            t.iter()
                .any(|e| matches!(e.kind,Kind::Meta(MetaMessage::TrackName(n)) if n==name))
        })
        .unwrap();
    let mut tick = 0;
    track
        .iter()
        .map(|e| {
            tick += u64::from(e.delta.as_int());
            (
                tick,
                match e.kind {
                    Kind::Meta(MetaMessage::Text(bytes)) => Owned::Text(bytes.to_vec()),
                    kind => Owned::Other(format!("{kind:?}")),
                },
            )
        })
        .collect()
}
fn chunks(bytes: &[u8]) -> Vec<&[u8]> {
    let mut at = 8 + u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let mut out = Vec::new();
    while at < bytes.len() {
        let end = at + 8 + u32::from_be_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        out.push(&bytes[at..end]);
        at = end;
    }
    out
}
fn timing() -> TempoMap {
    TempoMap::new(
        480,
        0.0,
        vec![chart_engine::timing::Tempo {
            tick: 0,
            bpm: 120.0,
        }],
    )
    .unwrap()
}
fn put(
    previous: Option<chart_engine::text_events::Event>,
    scope: Scope,
    tick: u64,
    value: &str,
) -> Edit {
    Edit::Put {
        previous,
        scope,
        tick,
        text: value.into(),
    }
}
fn edit(source: &text::Source, current: &Document, edit: Edit, revision: u64) -> Document {
    text::edited(
        source,
        current,
        &edit,
        revision + 1,
        revision,
        &timing(),
        10.0,
    )
    .unwrap()
    .0
}

#[test]
fn ordinary_midi_text_distinguishes_global_physical_and_opaque_data() {
    let bytes = fixture();
    let source = text::imported(&bytes).unwrap();
    assert_eq!(source.document.events.len(), 4);
    assert_eq!(source.owners, vec!["HARM2", "PART BASS", "PART GUITAR"]);
    let duplicates: Vec<_> = source
        .document
        .events
        .iter()
        .filter(|e| e.text == "duplicate cue")
        .collect();
    assert_eq!(duplicates.len(), 2);
    assert_ne!(duplicates[0].id, duplicates[1].id);
    assert_eq!(duplicates[0].scope, Scope::Global);
    assert!(
        source
            .document
            .events
            .iter()
            .any(|e| e.text == "[play]" && e.scope == owner("PART GUITAR"))
    );
    assert_eq!(
        text::rewrite(&bytes, &source, &source.document).unwrap(),
        bytes
    );
}

#[test]
fn edit_move_add_remove_preserve_raw_other_tracks_and_every_unmanaged_event() {
    let bytes = fixture();
    let source = text::imported(&bytes).unwrap();
    let first = source.document.events[0].clone();
    let changed = edit(
        &source,
        &source.document,
        put(Some(first), Scope::Global, 240, "lighting (chorus)"),
        0,
    );
    let local = changed
        .events
        .iter()
        .find(|e| e.text == "[play]")
        .unwrap()
        .clone();
    let moved = edit(
        &source,
        &changed,
        put(Some(local), Scope::Global, 720, "moved to global"),
        1,
    );
    let added = edit(
        &source,
        &moved,
        put(None, owner("PART BASS"), 960, "bass cue"),
        2,
    );
    let output = text::rewrite(&bytes, &source, &added).unwrap();
    for index in [0, 4, 5] {
        assert_eq!(chunks(&bytes)[index], chunks(&output)[index]);
    }
    for name in [b"EVENTS".as_slice(), b"PART GUITAR", b"PART BASS"] {
        let unmanaged = |bytes: &[u8]| {
            events(bytes, name)
                .into_iter()
                .filter(|(_, kind)| !matches!(kind, Owned::Text(_)))
                .collect::<Vec<_>>()
        };
        assert_eq!(unmanaged(&bytes), unmanaged(&output));
    }
    let global = events(&output, b"EVENTS");
    assert!(global.iter().any(|(tick, kind)| *tick == 0
        && matches!(kind, Owned::Text(bytes) if bytes==b"[section Intro]")));
    assert!(global.iter().any(|(tick, kind)| *tick == 300
        && matches!(kind, Owned::Text(bytes) if bytes==b"\xff opaque text")));
    let parsed = text::imported(&output).unwrap();
    assert_eq!(
        parsed
            .document
            .events
            .iter()
            .filter(|e| e.text == "duplicate cue")
            .count(),
        1
    );
    assert!(
        parsed
            .document
            .events
            .iter()
            .any(|e| e.scope == Scope::Global && e.tick == 720 && e.text == "moved to global")
    );
    assert!(
        events(&output, b"PART GUITAR")
            .iter()
            .any(|(tick, kind)| *tick == 400
                && matches!(kind, Owned::Text(bytes) if bytes==b"[solo]"))
    );
    let target = added
        .events
        .iter()
        .find(|e| e.text == "bass cue")
        .unwrap()
        .clone();
    let removed = edit(&source, &added, Edit::Remove { event: target }, 3);
    assert_eq!(
        text::imported(&text::rewrite(&bytes, &source, &removed).unwrap())
            .unwrap()
            .document
            .events
            .len(),
        4
    );
}

#[test]
fn stale_source_scope_timing_and_dedicated_text_refuse_without_mutation() {
    let bytes = fixture();
    let source = text::imported(&bytes).unwrap();
    for bad in [
        put(
            None,
            Scope::Local {
                track: "ExpertSingle".into(),
            },
            480,
            "wrong scope",
        ),
        put(None, owner("VENUE"), 0, "unknown owner"),
        put(None, Scope::Global, 0, "[prc_Intro]"),
        put(None, owner("PART GUITAR"), 0, "[solo]"),
        put(None, Scope::Global, 100000, "outside"),
    ] {
        assert!(text::edited(&source, &source.document, &bad, 1, 0, &timing(), 10.0).is_err());
        assert_eq!(
            text::rewrite(&bytes, &source, &source.document).unwrap(),
            bytes
        );
    }
    let old = source.document.events[0].clone();
    let after = edit(
        &source,
        &source.document,
        put(Some(old.clone()), Scope::Global, 240, "changed"),
        0,
    );
    assert!(
        text::edited(
            &source,
            &after,
            &Edit::Remove { event: old },
            2,
            1,
            &timing(),
            10.0
        )
        .is_err()
    );
    let mut foreign = after.clone();
    foreign.source_sha256 = "0".repeat(64);
    assert!(text::rewrite(&bytes, &source, &foreign).is_err());
}

#[test]
fn omitted_local_owner_is_not_reintroduced_and_global_track_can_be_added() {
    let bytes = fixture();
    let source = text::imported(&bytes).unwrap();
    let after = edit(
        &source,
        &source.document,
        put(None, owner("PART GUITAR"), 960, "guitar cue"),
        0,
    );
    let mut midi = Smf::parse(&bytes).unwrap();
    midi.tracks.remove(2);
    let mut filtered = Vec::new();
    midi.write_std(&mut filtered).unwrap();
    assert_eq!(text::rewrite(&filtered, &source, &after).unwrap(), filtered);
    let single = encode(vec![track(
        b"PART GUITAR",
        vec![(243, note(true)), (320, note(false))],
    )]);
    let source = text::imported(&single).unwrap();
    let after = edit(
        &source,
        &source.document,
        put(None, Scope::Global, 480, "music_start"),
        0,
    );
    let output = text::rewrite(&single, &source, &after).unwrap();
    assert_eq!(Smf::parse(&output).unwrap().header.format, Format::Parallel);
    assert_eq!(chunks(&single)[0], chunks(&output)[0]);
    assert!(
        text::imported(&output)
            .unwrap()
            .document
            .events
            .iter()
            .any(|e| e.scope == Scope::Global && e.tick == 480 && e.text == "music_start")
    );
}

#[test]
fn reopened_working_text_baseline_keeps_identity_after_moves_and_duplicate_edits() {
    let bytes = fixture();
    let source = text::imported(&bytes).unwrap();
    let old = source.document.events[0].clone();
    let first = edit(
        &source,
        &source.document,
        put(Some(old.clone()), owner("PART BASS"), 960, "moved cue"),
        0,
    );
    let working = text::rewrite(&bytes, &source, &first).unwrap();
    let moved = first
        .events
        .iter()
        .find(|e| e.id == old.id)
        .unwrap()
        .clone();
    let second = edit(
        &source,
        &first,
        put(Some(moved), Scope::Global, 240, "returned cue"),
        1,
    );
    assert!(
        text::rewrite(&working, &source, &second).is_err(),
        "An unverified baseline must refuse"
    );
    let output = text::rewrite_from(&working, &source, &first, &second).unwrap();
    let actual = text::imported(&output).unwrap();
    assert!(
        actual
            .document
            .events
            .iter()
            .any(|e| e.scope == Scope::Global && e.tick == 240 && e.text == "returned cue")
    );
    assert_eq!(
        actual
            .document
            .events
            .iter()
            .filter(|e| e.text == "duplicate cue")
            .count(),
        1
    );
    for index in [0, 2, 4, 5] {
        assert_eq!(chunks(&working)[index], chunks(&output)[index]);
    }
    assert_eq!(
        text::rewrite_from(&output, &source, &second, &first).unwrap(),
        working
    );
}

#[test]
fn midi_utf8_quotes_backslashes_and_lyric_meta_types_remain_distinct() {
    let bytes = fixture();
    let source = text::imported(&bytes).unwrap();
    let value = "cue \"Coro\" \\ lumière";
    let after = edit(
        &source,
        &source.document,
        put(None, owner("HARM2"), 480, value),
        0,
    );
    let output = text::rewrite(&bytes, &source, &after).unwrap();
    assert!(
        text::imported(&output)
            .unwrap()
            .document
            .events
            .iter()
            .any(|e| e.scope == owner("HARM2") && e.tick == 480 && e.text == value)
    );
    assert!(events(&output,b"HARM2").iter().any(|(tick,kind)|*tick==240&&matches!(kind,Owned::Other(value) if value.contains("Lyric")&&value.contains("119"))));
    for index in [0, 1, 2, 3, 5] {
        assert_eq!(chunks(&bytes)[index], chunks(&output)[index]);
    }
}
