use super::*;
use crate::{
    editor::{Action, Actor, ActorKind, Request, Store},
    note_groups::{self, Selection},
    sync_track::{self, Meter},
};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
    num::{u4, u7, u15, u24, u28},
};
use std::collections::BTreeSet;

type Event = (u64, TrackEventKind<'static>);
fn on(channel: u8, key: u8, velocity: u8) -> TrackEventKind<'static> {
    TrackEventKind::Midi {
        channel: u4::new(channel),
        message: MidiMessage::NoteOn {
            key: u7::new(key),
            vel: u7::new(velocity),
        },
    }
}
fn off(channel: u8, key: u8, velocity: u8) -> TrackEventKind<'static> {
    TrackEventKind::Midi {
        channel: u4::new(channel),
        message: MidiMessage::NoteOff {
            key: u7::new(key),
            vel: u7::new(velocity),
        },
    }
}
fn span(events: &mut Vec<Event>, start: u64, end: u64, key: u8) {
    events.extend([(start, on(0, key, 100)), (end, off(0, key, 0))]);
}
fn track(name: &'static [u8], mut events: Vec<Event>) -> Vec<TrackEvent<'static>> {
    events.insert(0, (0, TrackEventKind::Meta(MetaMessage::TrackName(name))));
    let end = events.iter().map(|e| e.0).max().unwrap_or(0).max(8000);
    events.push((end, TrackEventKind::Meta(MetaMessage::EndOfTrack)));
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
    let mut bytes = Vec::new();
    Smf {
        header: Header::new(Format::Parallel, Timing::Metrical(u15::new(480))),
        tracks,
    }
    .write_std(&mut bytes)
    .unwrap();
    bytes
}
fn fixture() -> Vec<u8> {
    let tempo = track(
        b"TEMPO",
        vec![
            (
                0,
                TrackEventKind::Meta(MetaMessage::Tempo(u24::new(500001))),
            ),
            (
                960,
                TrackEventKind::Meta(MetaMessage::Tempo(u24::new(600007))),
            ),
            (
                0,
                TrackEventKind::Meta(MetaMessage::TimeSignature(7, 3, 36, 8)),
            ),
            (
                960,
                TrackEventKind::Meta(MetaMessage::TimeSignature(9, 4, 48, 16)),
            ),
        ],
    );
    let mut guitar = vec![
        (
            0,
            TrackEventKind::Meta(MetaMessage::Text(b"[ENHANCED_OPENS]")),
        ),
        (240, on(3, 96, 83)),
        (360, on(3, 96, 0)),
        (720, on(2, 97, 75)),
        (840, off(2, 97, 9)),
        (0, on(4, 116, 82)),
        (1920, off(4, 116, 7)),
        (480, on(5, 103, 80)),
        (1600, off(5, 103, 6)),
        (300, on(7, 20, 60)),
        (400, off(7, 20, 4)),
        (320, on(1, 120, 74)), // unknown here, not a drum-fill event
        (
            240,
            TrackEventKind::Meta(MetaMessage::Text(b"[unknown author event]")),
        ),
        (300, TrackEventKind::SysEx(b"private\x01\xf7")),
        (310, TrackEventKind::Escape(b"\xf0opaque\xf7")),
    ];
    for d in 0..4u8 {
        span(&mut guitar, 1200, 1320, 59 + 12 * d);
        span(&mut guitar, 1500, 1620, 62 + 12 * d);
        span(&mut guitar, 1500, 1501, 65 + 12 * d);
    }
    let mut drums = vec![(
        0,
        TrackEventKind::Meta(MetaMessage::Text(b"[ENABLE_CHART_DYNAMICS]")),
    )];
    span(&mut drums, 240, 300, 95);
    drums.extend([
        (480, on(6, 98, 127)),
        (540, off(6, 98, 8)),
        (720, on(1, 99, 1)),
        (780, on(1, 99, 0)),
    ]);
    span(&mut drums, 700, 800, 111);
    for key in [103, 116, 120, 121, 122, 123, 124, 126, 127] {
        span(&mut drums, 960, 1440, key);
    }
    let mut tracks = vec![
        tempo,
        track(
            b"EVENTS",
            vec![
                (
                    0,
                    TrackEventKind::Meta(MetaMessage::Text(b"[section Intro]")),
                ),
                (
                    240,
                    TrackEventKind::Meta(MetaMessage::Text(b"[lighting (verse)]")),
                ),
            ],
        ),
        track(b"PART GUITAR", guitar),
        track(b"PART DRUMS", drums),
    ];
    for name in [b"PART BASS".as_slice(), b"PART KEYS"] {
        let mut events = Vec::new();
        for d in 0..4u8 {
            span(&mut events, 240, 360, 60 + 12 * d);
        }
        tracks.push(track(name, events));
    }
    for name in [
        b"PART REAL_KEYS_E".as_slice(),
        b"PART REAL_KEYS_M",
        b"PART REAL_KEYS_H",
        b"PART REAL_KEYS_X",
    ] {
        let mut events = Vec::new();
        for key in [0, 52, 115, 116] {
            span(&mut events, 240, 960, key);
        }
        tracks.push(track(name, events));
    }
    let mut vocal = vec![
        (240, TrackEventKind::Meta(MetaMessage::Lyric(b"Hello"))),
        (240, on(3, 60, 86)),
        (480, off(3, 60, 8)),
        (720, TrackEventKind::Meta(MetaMessage::Lyric(b"World#"))),
        (720, on(5, 67, 74)),
        (900, on(5, 67, 0)),
        (0, on(7, 105, 90)),
        (1920, off(7, 105, 7)),
        (0, TrackEventKind::Meta(MetaMessage::Text(b"[mood_play]"))),
    ];
    span(&mut vocal, 480, 500, 96);
    span(&mut vocal, 0, 1920, 116);
    tracks.push(track(b"PART VOCALS", vocal));
    for name in [
        b"HARM1".as_slice(),
        b"VENUE",
        b"PART REAL_GUITAR",
        b"PART GUITAR GHL",
        b"PART RHYTHM",
    ] {
        let mut events = vec![(0, TrackEventKind::Meta(MetaMessage::Text(b"Opaque data")))];
        span(&mut events, 240, 960, 60);
        tracks.push(track(name, events));
    }
    encode(tracks)
}
fn store(root: &Path, bytes: &[u8], delay: i32) -> Store {
    let song = root.join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.mid"), bytes).unwrap();
    fs::write(
        song.join("song.ini"),
        format!("[song]\nname = MIDI fixture\ndelay = {delay}\ncustom = keep\n"),
    )
    .unwrap();
    let samples = vec![0.; 44100 * 12];
    models::write_audio(&song.join("song.wav"), &samples, &samples).unwrap();
    Store::create(&song, None, &root.join("project")).unwrap()
}

#[test]
fn legacy_opaque_manual_parts_cannot_be_replaced_by_adding_a_blank_part() {
    let root = tempfile::tempdir().unwrap();
    let bytes = fixture();
    let mut midi = Smf::parse(&bytes).unwrap();
    for name in [b"PART REAL_BASS".as_slice(), b"PART REAL_BASS_22"] {
        midi.tracks.push(track(
            name,
            vec![(240, on(6, 101, 117)), (960, off(6, 101, 28))],
        ));
    }
    let mut original = Vec::new();
    midi.write_std(&mut original).unwrap();
    let store = store(root.path(), &original, 0);
    let song = store.root.join("source");
    let (mut imported, directory) = source(&song).unwrap().unwrap();
    let text = fs::read_to_string(directory.join("notes.chart")).unwrap();
    let chart = Chart::parse(text).unwrap();
    // Reproduce the retained, checksum-bound representation written before
    // manual parts became editable. The original MIDI chunks remain untouched.
    let legacy_part = |name: &str| {
        ["DoubleRhythm", "GHLGuitar", "ProBass17", "ProBass22"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
    };
    let legacy: String = chart
        .sections
        .iter()
        .filter(|s| !legacy_part(&s.name))
        .map(|s| {
            format!(
                "[{}]\n{{\n{}\n}}\n",
                s.name,
                &chart.raw[s.body_start..s.body_end]
            )
        })
        .collect();
    let mut attributes: Vec<AttributeRecord> =
        serde_json::from_slice(&fs::read(directory.join("attributes.json")).unwrap()).unwrap();
    attributes.retain(|a| !legacy_part(&a.track));
    imported.attributes.retain(|a| !legacy_part(&a.track));
    fs::write(
        directory.join("attributes.json"),
        serde_json::to_vec(&attributes).unwrap(),
    )
    .unwrap();
    fs::write(directory.join("notes.chart"), &legacy).unwrap();
    fs::write(song.join("notes.chart"), &legacy).unwrap();
    imported.chart_sha256 = hash(legacy.as_bytes());
    imported.attributes_chart_sha256 = imported.chart_sha256.clone();
    imported.baseline_attributes_sha256 =
        models::hash_file(&directory.join("attributes.json")).unwrap();
    fs::write(
        song_layout::resolve(&song, DESCRIPTOR),
        serde_json::to_vec(&imported).unwrap(),
    )
    .unwrap();
    store
        .initialize(&root.path().join("song"), None, 12.)
        .unwrap();
    assert!(extra_parts(&song).unwrap().contains("MIDI: PART RHYTHM"));
    let before = fs::read(store.root.join("project.json")).unwrap();
    for (tier, suffix) in ["Easy", "Medium", "Hard", "Expert"]
        .into_iter()
        .flat_map(|tier| {
            ["DoubleRhythm", "GHLGuitar", "ProBass17", "ProBass22"].map(|suffix| (tier, suffix))
        })
    {
        let error = apply(
            &store,
            Action::CreateTrack {
                track: format!("{tier}{suffix}"),
            },
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("older MIDI import"));
        assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    }
    assert_eq!(export(&store, &root.path().join("preserved")), original);
}

#[test]
fn deleting_final_lead_notes_keeps_unedited_percussion_power_and_author_events() {
    let root = tempfile::tempdir().unwrap();
    let bytes = fixture();
    let store = store(root.path(), &bytes, 0);
    let count = store.load().unwrap().content.vocals.unwrap().notes.len();
    assert!(count > 0);
    for _ in 0..count {
        apply(&store, Action::DeleteVocalNote { index: 0 }).unwrap();
    }
    assert!(store.load().unwrap().content.vocals.is_none());
    let output = export(&store, &root.path().join("deleted-lead-notes"));
    let midi = Smf::parse(&output).unwrap();
    let vocals = midi
        .tracks
        .iter()
        .find(|events| {
            events.iter().any(|event| {
                matches!(
                    event.kind,
                    TrackEventKind::Meta(MetaMessage::TrackName(b"PART VOCALS"))
                )
            })
        })
        .expect("Deleting pitch notes must not delete the unrelated vocal controls track");
    for key in [96, 105, 116] {
        assert!(
            vocals
                .iter()
                .any(|event| matches!(event.kind, TrackEventKind::Midi {
            message: MidiMessage::NoteOn {key:actual,vel}, ..
        } if actual.as_int()==key && vel.as_int()>0)),
            "Original control {key} was discarded"
        );
    }
    assert!(vocals.iter().any(|event| matches!(
        event.kind,
        TrackEventKind::Meta(MetaMessage::Text(b"[mood_play]"))
    )));
    assert!(
        !vocals
            .iter()
            .any(|event| matches!(event.kind, TrackEventKind::Midi {
        message: MidiMessage::NoteOn {key,vel}, ..
    } if (36..=84).contains(&key.as_int()) && vel.as_int()>0))
    );
    let reopened = Store::create(
        &root.path().join("deleted-lead-notes"),
        None,
        &root.path().join("reopened"),
    )
    .unwrap();
    assert!(reopened.load().unwrap().content.vocals.is_none());
    assert_eq!(
        export(&reopened, &root.path().join("second-export")),
        output
    );
}

#[test]
fn explicitly_omitted_vocal_controls_do_not_return_with_new_lead_notes() {
    use crate::export_options::{self, Options};
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &fixture(), 0);
    let doc = store.load().unwrap();
    let original_vocals = doc.content.vocals.clone().unwrap();
    let output = root.path().join("without-vocals");
    let selected = export_options::available(&store, &doc)
        .unwrap()
        .into_iter()
        .filter(|part| part != "Vocals")
        .collect();
    store
        .export_with_options(
            &doc,
            &output,
            &Actor {
                kind: ActorKind::Human,
                name: "Explicit omission fixture".into(),
            },
            &Options {
                tracks: Some(selected),
                ..Default::default()
            },
        )
        .unwrap();
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    let doc = reopened.load().unwrap();
    assert!(doc.content.vocals.is_none());
    assert!(
        !export_options::available(&reopened, &doc)
            .unwrap()
            .contains("Vocals")
    );
    apply(
        &reopened,
        Action::SetVocals {
            document: original_vocals,
        },
    )
    .unwrap();
    let new_output = root.path().join("new-lead");
    let bytes = export(&reopened, &new_output);
    let midi = Smf::parse(&bytes).unwrap();
    let track = midi
        .tracks
        .iter()
        .find(|t| decode::label(t).as_deref() == Some("PART VOCALS"))
        .unwrap();
    assert!(track.iter().any(|e| matches!(e.kind, TrackEventKind::Midi {
        message: MidiMessage::NoteOn { key,vel }, ..
    } if key.as_int() == 60 && vel.as_int()>0)));
    for key in [96, 116] {
        assert!(
            !track.iter().any(|e| matches!(e.kind, TrackEventKind::Midi {
            message: MidiMessage::NoteOn { key:k,vel }, ..
        } if k.as_int() == key && vel.as_int()>0))
        );
    }
    assert!(!track.iter().any(|e| matches!(
        e.kind,
        TrackEventKind::Meta(MetaMessage::Text(b"[mood_play]"))
    )));
    let again = Store::create(&new_output, None, &root.path().join("again")).unwrap();
    assert_eq!(export(&again, &root.path().join("again-export")), bytes);
}

#[test]
fn overlapping_edits_cannot_displace_a_retained_percussion_phrase() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &fixture(), 0);
    let mut vocals = store.load().unwrap().content.vocals.unwrap();
    // The original 0..1920 phrase also scores the unedited 480..500 hit.
    vocals.phrases[0].start = 600;
    vocals.notes.remove(0);
    apply(&store, Action::SetVocals { document: vocals }).unwrap();
    let output = root.path().join("conflict");
    let error = apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("retained percussion scoring phrase"));
    assert!(!output.exists());
}
fn apply(store: &Store, action: Action) -> Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Game MIDI fixture".into(),
        },
        action,
    })?;
    Ok(())
}
fn export(store: &Store, path: &Path) -> Vec<u8> {
    apply(
        store,
        Action::Export {
            output: path.into(),
        },
    )
    .unwrap();
    fs::read(path.join("notes.mid")).unwrap()
}
fn events(bytes: &[u8], name: &str) -> Vec<(u64, TrackEventKind<'static>)> {
    // Keep borrowed payloads separate in tests that need them. This helper is
    // only for numeric MIDI/meta messages, whose midly to_static is lossless.
    let smf = Smf::parse(bytes).unwrap();
    let mut result = Vec::new();
    for track in &smf.tracks {
        if decode::label(track).as_deref() != Some(name) {
            continue;
        }
        let mut tick = 0;
        for e in track {
            tick += u64::from(e.delta.as_int());
            if matches!(
                e.kind,
                TrackEventKind::Midi { .. }
                    | TrackEventKind::Meta(
                        MetaMessage::Tempo(_)
                            | MetaMessage::TimeSignature(..)
                            | MetaMessage::EndOfTrack
                    )
            ) {
                result.push((tick, e.kind.to_static()));
            }
        }
    }
    result
}
fn chunk<'a>(bytes: &'a [u8], name: &str) -> &'a [u8] {
    let smf = Smf::parse(bytes).unwrap();
    let index = smf
        .tracks
        .iter()
        .position(|t| decode::label(t).as_deref() == Some(name))
        .unwrap();
    merge::chunks(bytes, smf.tracks.len()).unwrap().1[index]
}

#[test]
fn unchanged_game_midi_opens_all_supported_parts_and_roundtrips_exact_bytes() {
    let original = fixture();
    for delay in [250, -125] {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path(), &original, delay);
        let doc = store.load().unwrap();
        for suffix in ["Single", "DoubleBass", "Drums", "Keyboard", "ProKeys"] {
            for tier in ["Easy", "Medium", "Hard", "Expert"] {
                if suffix != "Drums" || tier == "Expert" {
                    assert!(doc.content.tracks.contains_key(&format!("{tier}{suffix}")));
                }
            }
        }
        assert_eq!(doc.content.vocals.as_ref().unwrap().notes.len(), 2);
        assert_eq!(doc.timing(&store).unwrap().offset, delay as f64 / 1000.);
        let output = root.path().join("export");
        assert_eq!(export(&store, &output), original);
        let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
        assert_eq!(export(&reopened, &root.path().join("again")), original);
        assert_eq!(
            fs::read(root.path().join("song/notes.mid")).unwrap(),
            original
        );
        assert!(!fs::exists(root.path().join("song/.yace")));
    }
}

#[test]
fn moving_copying_and_undoing_gems_preserves_attributes_unknown_events_and_phrases() {
    let original = fixture();
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, 0);
    let doc = store.load().unwrap();
    let note = doc.content.tracks["ExpertSingle"]
        .iter()
        .find(|n| n.note.tick == 240 && n.note.lane == 0)
        .unwrap();
    let selection = Selection {
        track: "ExpertSingle".into(),
        ids: BTreeSet::from([note.id.clone()]),
        vocals: BTreeSet::new(),
    };
    let clipboard = note_groups::copy(&doc, &doc.timing(&store).unwrap(), &selection).unwrap();
    apply(
        &store,
        Action::UpdateNote {
            note_id: note.id.clone(),
            tick: 300,
            lane: 2,
            sustain: 120,
            cymbal: false,
        },
    )
    .unwrap();
    apply(
        &store,
        Action::EditNoteGroup {
            edit: note_groups::Edit::Paste {
                track: "ExpertSingle".into(),
                tick: 1920,
                clipboard,
            },
        },
    )
    .unwrap();
    let output = root.path().join("export");
    let edited = export(&store, &output);
    let numeric = events(&edited, "PART GUITAR");
    assert!(numeric.contains(&(300, on(3, 98, 83))));
    assert!(numeric.contains(&(420, on(3, 98, 0))));
    assert!(numeric.contains(&(1920, on(3, 96, 83))));
    assert!(numeric.contains(&(2040, on(3, 96, 0))));
    assert!(numeric.contains(&(0, on(4, 116, 82))));
    assert!(numeric.contains(&(1920, off(4, 116, 7))));
    assert!(numeric.contains(&(1600, off(5, 103, 6))));
    assert!(numeric.contains(&(300, on(7, 20, 60))));
    assert!(numeric.contains(&(8000, TrackEventKind::Meta(MetaMessage::EndOfTrack))));
    let parsed = Smf::parse(&edited).unwrap();
    let guitar = parsed
        .tracks
        .iter()
        .find(|t| decode::label(t).as_deref() == Some("PART GUITAR"))
        .unwrap();
    assert!(
        guitar
            .iter()
            .any(|e| matches!(e.kind, TrackEventKind::SysEx(b"private\x01\xf7")))
    );
    assert!(
        guitar
            .iter()
            .any(|e| matches!(e.kind, TrackEventKind::Escape(b"\xf0opaque\xf7")))
    );
    assert!(guitar.iter().any(|e| matches!(
        e.kind,
        TrackEventKind::Meta(MetaMessage::Text(b"[unknown author event]"))
    )));
    for name in [
        "TEMPO",
        "EVENTS",
        "PART DRUMS",
        "PART VOCALS",
        "HARM1",
        "VENUE",
        "PART REAL_GUITAR",
        "PART GUITAR GHL",
        "PART RHYTHM",
    ] {
        assert_eq!(chunk(&edited, name), chunk(&original, name), "{name}");
    }
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(export(&reopened, &root.path().join("again")), edited);
    apply(&store, Action::Undo).unwrap();
    apply(&store, Action::Undo).unwrap();
    assert_eq!(export(&store, &root.path().join("undo")), original);
    apply(&store, Action::Redo).unwrap();
    assert!(
        events(&export(&store, &root.path().join("redo")), "PART GUITAR")
            .contains(&(300, on(3, 98, 83)))
    );
}

#[test]
fn vocal_edits_preserve_talkie_pitch_channels_percussion_overdrive_and_phrase_attributes() {
    let original = fixture();
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, 0);
    let doc = store.load().unwrap();
    let mut vocals = doc.content.vocals.clone().unwrap();
    vocals.notes[0].start = 300;
    vocals.notes[0].end = 540;
    vocals.notes[1].start = 750;
    vocals.notes[1].end = 930;
    apply(&store, Action::SetVocals { document: vocals }).unwrap();
    let bytes = export(&store, &root.path().join("export"));
    let notes = events(&bytes, "PART VOCALS");
    for event in [
        (300, on(3, 60, 86)),
        (540, off(3, 60, 8)),
        (750, on(5, 67, 74)),
        (930, on(5, 67, 0)),
        (0, on(7, 105, 90)),
        (1920, off(7, 105, 7)),
        (480, on(0, 96, 100)),
        (0, on(0, 116, 100)),
    ] {
        assert!(notes.contains(&event), "{event:?}");
    }
    let view = merge::vocal_view(&bytes).unwrap().unwrap();
    assert_eq!(
        crate::vocals::import_midi(&view).unwrap().notes[1].start,
        750
    );
    assert_eq!(chunk(&bytes, "HARM1"), chunk(&original, "HARM1"));
}

fn vocal_dedicated_percussion_source(uncovered: bool) -> Vec<u8> {
    encode(vec![
        track(
            b"TEMPO",
            vec![(
                0,
                TrackEventKind::Meta(MetaMessage::Tempo(u24::new(500000))),
            )],
        ),
        track(
            b"PART VOCALS",
            vec![
                (0, on(7, 105, 90)),
                (960, off(7, 105, 7)),
                (0, on(0, 116, 100)),
                (960, off(0, 116, 11)),
                (0, TrackEventKind::Meta(MetaMessage::Text(b"[mood_play]"))),
                (240, TrackEventKind::Meta(MetaMessage::Lyric(b"Hello"))),
                (240, on(3, 60, 86)),
                (360, off(3, 60, 9)),
                (720, TrackEventKind::Meta(MetaMessage::Lyric(b"World#"))),
                (720, on(5, 67, 74)),
                (840, on(5, 67, 0)),
                (2000, on(10, 106, 84)),
                (2960, off(10, 106, 11)),
                (2160, on(2, 96, 93)),
                (2160, on(2, 96, 0)),
                (if uncovered { 3000 } else { 2400 }, on(5, 97, 72)),
                (if uncovered { 3048 } else { 2448 }, off(5, 97, 17)),
            ],
        ),
        track(
            b"PART GUITAR",
            vec![(960, on(0, 96, 100)), (1080, off(0, 96, 17))],
        ),
        track(
            b"VENUE",
            vec![(
                123,
                TrackEventKind::Meta(MetaMessage::Text(b"Keep independent sentinel")),
            )],
        ),
    ])
}

#[test]
fn vocal_dedicated_percussion_import_keeps_singers_editable_and_preserves_both_on_export() {
    let original = vocal_dedicated_percussion_source(false);
    let expected_pc = crate::vocal_percussion::import_midi(&original)
        .unwrap()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, 0);
    let doc = store.load().unwrap();
    let mut vocals = doc
        .content
        .vocals
        .clone()
        .expect("Dedicated PC must not hide singing");
    assert_eq!(vocals.notes.len(), 2);
    assert_eq!(vocals.phrases.len(), 1);
    assert_eq!(vocals.phrases[0].end, 960);
    assert!(doc.content.vocal_percussion.is_none());
    assert_eq!(
        store.vocal_percussion(&doc).unwrap(),
        Some(expected_pc.clone())
    );
    vocals.notes[0].start = 300;
    vocals.notes[0].end = 420;
    apply(
        &store,
        Action::SetVocals {
            document: vocals.clone(),
        },
    )
    .unwrap();
    let bytes = export(&store, &root.path().join("edited"));
    assert_eq!(
        crate::vocal_percussion::import_midi(&bytes).unwrap(),
        Some(expected_pc.clone())
    );
    let notes = events(&bytes, "PART VOCALS");
    for event in [
        (300, on(3, 60, 86)),
        (420, off(3, 60, 9)),
        (2000, on(10, 106, 84)),
        (2960, off(10, 106, 11)),
        (2160, on(2, 96, 93)),
        (2160, on(2, 96, 0)),
        (2400, on(5, 97, 72)),
        (2448, off(5, 97, 17)),
    ] {
        assert!(notes.contains(&event), "{event:?}");
    }
    for name in ["TEMPO", "PART GUITAR", "VENUE"] {
        assert_eq!(chunk(&bytes, name), chunk(&original, name));
    }
    let reopened = Store::create(
        &root.path().join("edited"),
        None,
        &root.path().join("reopened"),
    )
    .unwrap();
    assert_eq!(reopened.load().unwrap().content.vocals, Some(vocals));
    assert_eq!(
        reopened
            .vocal_percussion(&reopened.load().unwrap())
            .unwrap(),
        Some(expected_pc.clone())
    );
    apply(&store, Action::DeleteVocalNote { index: 1 }).unwrap();
    apply(&store, Action::DeleteVocalNote { index: 0 }).unwrap();
    let cleared = export(&store, &root.path().join("cleared-singers"));
    assert_eq!(
        crate::vocal_percussion::import_midi(&cleared).unwrap(),
        Some(expected_pc)
    );
    assert!(
        !events(&cleared, "PART VOCALS")
            .iter()
            .any(|(_, kind)| matches!(kind,
        TrackEventKind::Midi {message: MidiMessage::NoteOn {key, vel}, ..}
        if (36..=84).contains(&key.as_int()) && vel.as_int()>0))
    );
    let (_, archived) = source(&store.root.join("source")).unwrap().unwrap();
    assert_eq!(fs::read(archived.join("notes.mid")).unwrap(), original);
    assert_eq!(
        fs::read(root.path().join("song/notes.mid")).unwrap(),
        original
    );
}

#[test]
fn vocal_dedicated_percussion_import_does_not_guess_scoring_for_uncovered_source_hits() {
    let original = vocal_dedicated_percussion_source(true);
    assert!(crate::vocal_percussion::import_midi(&original).is_err());
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, 0);
    let doc = store.load().unwrap();
    assert!(doc.content.vocals.is_none());
    assert!(
        doc.review
            .coverage_limits
            .iter()
            .any(|warning| warning.contains("Empty scoring phrase"))
    );
    assert_eq!(export(&store, &root.path().join("retained")), original);
}

#[test]
fn meter_edit_retains_exact_microsecond_tempos_and_source_meter_counters() {
    let original = fixture();
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, -125);
    let before = store.load().unwrap().timing(&store).unwrap();
    apply(
        &store,
        Action::EditSync {
            edit: sync_track::Edit::PutMeter {
                previous: Some(Meter {
                    tick: 960,
                    numerator: 9,
                    denominator: 16,
                }),
                meter: Meter {
                    tick: 960,
                    numerator: 11,
                    denominator: 32,
                },
            },
        },
    )
    .unwrap();
    let output = root.path().join("export");
    let bytes = export(&store, &output);
    let smf = Smf::parse(&bytes).unwrap();
    let numeric: Vec<_> = smf
        .tracks
        .iter()
        .flat_map(|t| t.iter())
        .map(|e| e.kind)
        .collect();
    for us in [500001, 600007] {
        assert!(numeric.contains(&TrackEventKind::Meta(MetaMessage::Tempo(u24::new(us)))));
    }
    assert!(
        numeric.contains(&TrackEventKind::Meta(MetaMessage::TimeSignature(
            11, 5, 48, 16
        )))
    );
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(reopened.load().unwrap().timing(&reopened).unwrap(), before);
    assert_eq!(chunk(&bytes, "VENUE"), chunk(&original, "VENUE"));
}

#[test]
fn offset_only_edit_keeps_every_midi_byte_and_reopens_with_one_ini_delay() {
    let original = fixture();
    for offset in [0.25, -0.125] {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path(), &original, 75);
        let before = store.load().unwrap();
        apply(
            &store,
            Action::EditSync {
                edit: sync_track::Edit::SetOffset {
                    previous: 0.075,
                    seconds: offset,
                },
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&store.load().unwrap().content.tracks).unwrap(),
            serde_json::to_value(&before.content.tracks).unwrap()
        );
        let output = root.path().join("export");
        assert_eq!(export(&store, &output), original);
        assert_eq!(
            crate::song_metadata::ini_fields(&fs::read_to_string(output.join("song.ini")).unwrap())
                ["delay"],
            ((offset * 1000.) as i32).to_string()
        );
        let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
        assert_eq!(
            reopened.load().unwrap().timing(&reopened).unwrap(),
            store.load().unwrap().timing(&store).unwrap()
        );
        apply(&store, Action::Undo).unwrap();
        assert_eq!(store.load().unwrap().timing(&store).unwrap().offset, 0.075);
        apply(&store, Action::Redo).unwrap();
        assert_eq!(store.load().unwrap().timing(&store).unwrap().offset, offset);
    }
}

#[test]
fn editing_one_power_range_keeps_other_ranges_exact_source_note_attributes() {
    let original = encode(vec![
        track(
            b"PART GUITAR",
            vec![
                (0, on(4, 116, 89)),
                (480, off(4, 116, 22)),
                (960, on(5, 116, 77)),
                (1440, on(5, 116, 0)),
                (240, on(2, 96, 81)),
                (360, off(2, 96, 17)),
            ],
        ),
        track(
            b"VENUE",
            vec![(
                123,
                TrackEventKind::Meta(MetaMessage::Text(b"Unrelated venue")),
            )],
        ),
    ]);
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, 0);
    apply(
        &store,
        Action::EditStarPower {
            edit: crate::star_power::Edit::Put {
                track: "ExpertSingle".into(),
                previous: None,
                phrase: crate::star_power::Phrase {
                    start: 1920,
                    end: 2400,
                },
            },
        },
    )
    .unwrap();
    let added = export(&store, &root.path().join("added"));
    let old = events(&original, "PART GUITAR");
    let actual = events(&added, "PART GUITAR");
    for edge in &old {
        assert!(
            actual.contains(edge),
            "Untouched source event changed: {edge:?}"
        );
    }
    assert!(actual.contains(&(1920, on(0, 116, 100))));
    assert!(actual.contains(&(2400, off(0, 116, 0))));
    assert_eq!(
        merge::chunks(&original, 2).unwrap().1[1],
        merge::chunks(&added, 2).unwrap().1[1]
    );
    apply(
        &store,
        Action::EditStarPower {
            edit: crate::star_power::Edit::Remove {
                track: "ExpertSingle".into(),
                phrase: crate::star_power::Phrase { start: 0, end: 480 },
            },
        },
    )
    .unwrap();
    let removed = export(&store, &root.path().join("removed"));
    let actual = events(&removed, "PART GUITAR");
    assert!(actual.contains(&(960, on(5, 116, 77))));
    assert!(actual.contains(&(1440, on(5, 116, 0))));
    assert!(!actual.iter().any(|(_,kind)|matches!(kind,TrackEventKind::Midi {channel,message:MidiMessage::NoteOn{key,..}|MidiMessage::NoteOff{key,..}} if channel.as_int()==4&&key.as_int()==116)));
    apply(&store, Action::Undo).unwrap();
    apply(&store, Action::Undo).unwrap();
    assert_eq!(export(&store, &root.path().join("undo")), original);
    apply(&store, Action::Redo).unwrap();
    apply(&store, Action::Redo).unwrap();
    assert_eq!(export(&store, &root.path().join("redo")), removed);
    let reopened = Store::create(
        &root.path().join("removed"),
        None,
        &root.path().join("reopened"),
    )
    .unwrap();
    assert_eq!(export(&reopened, &root.path().join("repeated")), removed);
}

#[test]
fn unchanged_power_solo_and_drum_marker_edges_survive_each_component_rewrite() {
    let mut guitar = vec![(240, on(2, 96, 81)), (360, off(2, 96, 17))];
    for (key, channel) in [(116, 4), (103, 3)] {
        guitar.extend([
            (0, on(channel, key, 89)),
            (480, off(channel, key, 22)),
            (960, on(channel + 1, key, 77)),
            (1440, on(channel + 1, key, 0)),
        ]);
    }
    let mut drums = vec![(240, on(2, 96, 81)), (360, off(2, 96, 17))];
    for key in [120, 121, 122, 123, 124, 126, 127] {
        drums.extend([
            (0, on(6, key, 63)),
            (480, off(6, key, 21)),
            (960, on(7, key, 64)),
            (1440, on(7, key, 0)),
        ]);
    }
    let original = encode(vec![
        track(b"PART GUITAR", guitar),
        track(b"PART DRUMS", drums),
        track(
            b"PART REAL_KEYS_X",
            vec![
                (240, on(2, 60, 71)),
                (360, off(2, 60, 17)),
                (0, on(12, 115, 91)),
                (480, off(12, 115, 51)),
            ],
        ),
    ]);
    let decoded = decode::import(&original, "[song]\n").unwrap();
    for (section, owner, rows) in [
        ("ExpertSingle", "PART GUITAR", " 3000 = S 2 240\n"),
        (
            "ExpertSingle",
            "PART GUITAR",
            " 3000 = E solo\n 3240 = E soloend\n",
        ),
        ("ExpertDrums", "PART DRUMS", " 3000 = S 64 240\n"),
        ("ExpertDrums", "PART DRUMS", " 3000 = S 65 240\n"),
        ("ExpertDrums", "PART DRUMS", " 3000 = S 66 240\n"),
        (
            "ExpertProKeys",
            "PART REAL_KEYS_X",
            " 3000 = E solo\n 3240 = E soloend\n",
        ),
    ] {
        let mut raw = decoded.chart.raw.clone();
        let body = decoded
            .chart
            .sections
            .iter()
            .find(|s| s.name == section)
            .unwrap();
        raw.insert_str(body.body_end, rows);
        let changed = Chart::parse(raw).unwrap();
        let generated = crate::game_export::song(&changed, decoded.vocals.as_ref())
            .unwrap()
            .0;
        let output = merge::rewrite(
            &original,
            &generated,
            merge::State {
                chart: &decoded.chart,
                vocals: decoded.vocals.as_ref(),
                attributes: &decoded.attributes,
            },
            merge::State {
                chart: &changed,
                vocals: decoded.vocals.as_ref(),
                attributes: &decoded.attributes,
            },
        )
        .unwrap();
        let old = events(&original, owner);
        let actual = events(&output, owner);
        for edge in old {
            assert!(
                actual.contains(&edge),
                "{section}: source edge changed: {edge:?}"
            );
        }
        for name in ["PART GUITAR", "PART DRUMS", "PART REAL_KEYS_X"] {
            if name != owner {
                assert_eq!(
                    events(&original, name),
                    events(&output, name),
                    "{section}: unrelated {name}"
                );
            }
        }
    }
}

#[test]
fn shared_star_power_edits_apply_to_all_difficulties_in_one_undo_step() {
    let original = fixture();
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, 0);
    apply(
        &store,
        Action::EditStarPower {
            edit: crate::star_power::Edit::Remove {
                track: "EasySingle".into(),
                phrase: crate::star_power::Phrase {
                    start: 0,
                    end: 1920,
                },
            },
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    for tier in ["Easy", "Medium", "Hard", "Expert"] {
        assert!(doc.content.star_power[&format!("{tier}Single")].is_empty());
    }
    let bytes = export(&store, &root.path().join("export"));
    assert!(!events(&bytes, "PART GUITAR").iter().any(|(_, k)| matches!(k, TrackEventKind::Midi { message: MidiMessage::NoteOn { key, .. }, .. } if key.as_int() == 116)));
    apply(&store, Action::Undo).unwrap();
    assert_eq!(export(&store, &root.path().join("undo")), original);
}

#[test]
fn malformed_ambiguous_or_unsupported_profiles_are_explicit_and_atomic() {
    let mut overlapping = vec![
        (240, on(0, 96, 100)),
        (300, on(0, 96, 100)),
        (360, off(0, 96, 0)),
    ];
    let bad = encode(vec![track(b"PART GUITAR", overlapping.clone())]);
    assert!(
        decode::import(&bad, "[song]\n")
            .err()
            .unwrap()
            .to_string()
            .contains("Overlapping")
    );
    overlapping = vec![(0, on(0, 101, 100)), (120, off(0, 101, 0))];
    assert!(decode::import(&encode(vec![track(b"PART DRUMS", overlapping)]), "[song]\n").is_err());
    assert!(decode::import(&fixture(), "[song]\nmultiplier_note = 103\n").is_err());
    let extended_meta = encode(vec![track(
        b"PART GUITAR",
        vec![(
            0,
            TrackEventKind::Meta(MetaMessage::Unknown(0x51, b"\x07\xa1\x20\xff")),
        )],
    )]);
    assert!(decode::import(&extended_meta, "[song]\n").is_err());
    let mut malformed_tail = encode(vec![track(b"PART GUITAR", vec![])]);
    let length = u32::from_be_bytes(malformed_tail[18..22].try_into().unwrap());
    malformed_tail[18..22].copy_from_slice(&(length + 2).to_be_bytes());
    malformed_tail.extend([0, 0xf1]);
    assert!(decode::import(&malformed_tail, "[song]\n").is_err());
    let original = fixture();
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, 0);
    fs::write(
        store
            .root
            .join("source/.yace/originals/game-midi/notes.mid"),
        b"tampered",
    )
    .unwrap();
    assert!(
        apply(
            &store,
            Action::Export {
                output: root.path().join("bad")
            }
        )
        .is_err()
    );
    assert!(!fs::exists(root.path().join("bad")));
    assert_eq!(
        fs::read(root.path().join("song/notes.mid")).unwrap(),
        original
    );
}

#[test]
fn unrepresentable_lead_vocals_remain_opaque_and_cannot_be_silently_replaced() {
    let original = encode(vec![
        track(
            b"PART VOCALS",
            vec![(240, on(3, 60, 86)), (480, off(3, 60, 8))],
        ),
        track(b"HARM1", vec![(240, on(1, 64, 90)), (480, off(1, 64, 0))]),
    ]);
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, 0);
    let doc = store.load().unwrap();
    assert!(doc.content.vocals.is_none());
    assert!(
        doc.review
            .coverage_limits
            .iter()
            .any(|w| w.contains("not editable"))
    );
    assert_eq!(export(&store, &root.path().join("unchanged")), original);
    let replacement = decode::import(&fixture(), "[song]\n")
        .unwrap()
        .vocals
        .unwrap();
    apply(
        &store,
        Action::SetVocals {
            document: replacement,
        },
    )
    .unwrap();
    let error = apply(
        &store,
        Action::Export {
            output: root.path().join("bad"),
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("replacing them would lose"));
    assert!(!fs::exists(root.path().join("bad")));
}

#[test]
fn dense_import_and_zero_length_notes_keep_exact_tick_and_note_attributes() {
    let mut notes = vec![(0, on(2, 96, 70)), (0, on(2, 96, 0))];
    for i in 1..20000u64 {
        span(&mut notes, i * 10, i * 10 + 1, 96 + (i % 5) as u8);
    }
    let bytes = encode(vec![track(b"PART GUITAR", notes)]);
    let decoded = decode::import(&bytes, "[song]\n").unwrap();
    assert_eq!(
        decoded
            .chart
            .notes("ExpertSingle")
            .unwrap()
            .iter()
            .filter(|n| n.lane <= 4)
            .count(),
        20000
    );
    assert!(
        decoded
            .attributes
            .iter()
            .any(|a| a.note.tick == 0 && a.attributes.zero_length && a.attributes.off_as_note_on)
    );
}

#[test]
fn phase_shift_ranges_and_forcing_keep_their_inclusive_and_exclusive_boundaries() {
    use crate::gameplay_view::{self, Technique};
    let mut notes = vec![
        (200, TrackEventKind::SysEx(b"PS\0\0\x03\x04\x01\xf7")),
        (300, TrackEventKind::SysEx(b"PS\0\0\x03\x04\x00\xf7")),
        (400, TrackEventKind::SysEx(b"PS\0\0\x03\x01\x01\xf7")),
        (500, TrackEventKind::SysEx(b"PS\0\0\x03\x01\x00\xf7")),
    ];
    for (start, key) in [
        (0, 96),
        (160, 97),
        (200, 98),
        (300, 99),
        (301, 100),
        (400, 96),
        (500, 96),
        (600, 96),
    ] {
        span(&mut notes, start, start + 1, key);
    }
    span(&mut notes, 600, 601, 101); // explicit HOPO even on a repeated fret
    let bytes = encode(vec![track(b"PART GUITAR", notes)]);
    let decoded = decode::import(&bytes, "[song]\n").unwrap();
    let notes = decoded.chart.notes("ExpertSingle").unwrap();
    let policy = gameplay_view::yarg_chart_policy(480, &BTreeMap::new()).unwrap();
    let gems =
        gameplay_view::classify_for_midi("ExpertSingle", &notes, &decoded.chart.timing, &policy)
            .unwrap();
    let technique = |tick| gems.iter().find(|g| g.tick == tick).unwrap().technique;
    assert_eq!(technique(200), Some(Technique::Tap));
    assert_eq!(technique(300), Some(Technique::Tap));
    assert_ne!(technique(301), Some(Technique::Tap));
    assert_eq!(technique(600), Some(Technique::Hopo));
    assert!(notes.iter().any(|n| n.tick == 400 && n.lane == 7));
    assert!(notes.iter().any(|n| n.tick == 500 && n.lane == 0));
}

#[test]
fn drum_and_pro_keys_edits_keep_dynamics_and_opaque_ranges() {
    let original = fixture();
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, 0);
    let doc = store.load().unwrap();
    let cymbal = doc.content.tracks["ExpertDrums"]
        .iter()
        .find(|n| n.note.tick == 480 && n.note.lane == 2)
        .unwrap();
    let selection = Selection {
        track: "ExpertDrums".into(),
        ids: BTreeSet::from([cymbal.id.clone()]),
        vocals: BTreeSet::new(),
    };
    let snapshot = note_groups::Snapshot::capture(&doc, &selection).unwrap();
    apply(
        &store,
        Action::EditNoteGroup {
            edit: note_groups::Edit::Move {
                selection: snapshot,
                ticks: 60,
                rows: 0,
            },
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    let piano = doc.content.tracks["ExpertProKeys"]
        .iter()
        .find(|n| n.note.lane == 52)
        .unwrap();
    apply(
        &store,
        Action::UpdateNote {
            note_id: piano.id.clone(),
            tick: 300,
            lane: 53,
            sustain: 600,
            cymbal: false,
        },
    )
    .unwrap();
    let bytes = export(&store, &root.path().join("export"));
    let notes = events(&bytes, "PART DRUMS");
    assert!(notes.contains(&(540, on(6, 98, 127))));
    assert!(notes.contains(&(600, off(6, 98, 8))));
    assert!(notes.contains(&(720, on(1, 99, 1))));
    let decoded = decode::import(&bytes, "[song]\n").unwrap();
    let drums = decoded.chart.notes("ExpertDrums").unwrap();
    assert!(drums.iter().any(|n| n.tick == 540 && n.lane == 66));
    assert!(drums.iter().any(|n| n.tick == 540 && n.lane == 35));
    assert!(!drums.iter().any(|n| n.tick == 720 && n.lane == 67));
    assert!(drums.iter().any(|n| n.tick == 720 && n.lane == 42));
    assert!(drums.iter().any(|n| n.tick == 240 && n.lane == 32));
    let piano = events(&bytes, "PART REAL_KEYS_X");
    assert!(piano.contains(&(240, on(0, 0, 100))));
    assert!(piano.contains(&(960, off(0, 0, 0))));
    assert!(piano.contains(&(300, on(0, 53, 100))));
}

#[test]
fn zero_length_gems_survive_edit_export_without_becoming_a_tick_long() {
    let original = encode(vec![track(
        b"PART GUITAR",
        vec![(240, on(2, 96, 80)), (240, on(2, 96, 0))],
    )]);
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, 0);
    let doc = store.load().unwrap();
    let note = doc.content.tracks["ExpertSingle"]
        .iter()
        .find(|n| n.note.lane == 0)
        .unwrap();
    apply(
        &store,
        Action::UpdateNote {
            note_id: note.id.clone(),
            tick: 300,
            lane: 1,
            sustain: 0,
            cymbal: false,
        },
    )
    .unwrap();
    let bytes = export(&store, &root.path().join("export"));
    let numeric = events(&bytes, "PART GUITAR");
    assert!(numeric.contains(&(300, on(2, 97, 80))));
    assert!(numeric.contains(&(300, on(2, 97, 0))));
    let decoded = decode::import(&bytes, "[song]\n").unwrap();
    assert_eq!(
        decoded
            .chart
            .notes("ExpertSingle")
            .unwrap()
            .iter()
            .find(|n| n.lane == 1)
            .unwrap()
            .sustain,
        0
    );
}

#[test]
fn overlapping_tom_markers_toggle_instead_of_becoming_a_union() {
    let mut notes = vec![
        (100, on(0, 110, 100)),
        (400, off(0, 110, 0)),
        (200, on(1, 110, 100)),
        (500, off(1, 110, 0)),
    ];
    for tick in [240, 360, 480] {
        span(&mut notes, tick, tick + 1, 98);
    }
    let decoded = decode::import(&encode(vec![track(b"PART DRUMS", notes)]), "[song]\n").unwrap();
    let notes = decoded.chart.notes("ExpertDrums").unwrap();
    for tick in [240, 360] {
        assert!(notes.iter().any(|n| n.tick == tick && n.lane == 66));
    }
    assert!(!notes.iter().any(|n| n.tick == 480 && n.lane == 66));
}

#[test]
fn empty_supported_parts_expose_their_existing_phrases_before_new_notes_are_authored() {
    let original = encode(vec![track(
        b"PART DRUMS",
        vec![(0, on(4, 116, 83)), (960, off(4, 116, 7))],
    )]);
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), &original, 0);
    let doc = store.load().unwrap();
    assert!(doc.content.tracks["ExpertDrums"].is_empty());
    let source = Chart::parse(doc.chart_text(&store).unwrap()).unwrap();
    assert_eq!(
        crate::star_power::current(&doc, &source, "ExpertDrums").unwrap(),
        [crate::star_power::Phrase { start: 0, end: 960 }]
    );
    apply(
        &store,
        Action::AddNote {
            track: "ExpertDrums".into(),
            tick: 240,
            lane: 0,
            sustain: 0,
            cymbal: false,
        },
    )
    .unwrap();
    let bytes = export(&store, &root.path().join("with-note"));
    assert!(events(&bytes, "PART DRUMS").contains(&(0, on(4, 116, 83))));
    apply(
        &store,
        Action::EditStarPower {
            edit: crate::star_power::Edit::Remove {
                track: "ExpertDrums".into(),
                phrase: crate::star_power::Phrase { start: 0, end: 960 },
            },
        },
    )
    .unwrap();
    assert!(
        !events(&export(&store, &root.path().join("removed")), "PART DRUMS")
            .contains(&(0, on(4, 116, 83)))
    );
}
