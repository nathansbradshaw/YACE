use chart_engine::{
    chart::{Note, Tracks},
    midi,
    timing::TempoMap,
    vocals::{self, Syllable, VocalChart, VocalPhrase, VocalTone},
};
use midly::{MetaMessage, Smf, TrackEventKind};

fn fixture() -> VocalChart {
    serde_json::from_str(include_str!("fixtures/vocals-model-free.json")).unwrap()
}

#[test]
fn vocal_json_is_editable_and_rejects_unknown_fields() {
    let fixture = fixture();
    let text = serde_json::to_string_pretty(&fixture).unwrap();
    assert_eq!(serde_json::from_str::<VocalChart>(&text).unwrap(), fixture);
    let mut bad = serde_json::to_value(&fixture).unwrap();
    bad["invented"] = "unsafe".into();
    assert!(serde_json::from_value::<VocalChart>(bad).is_err());
}

#[test]
fn pitched_talkie_slide_and_join_round_trip_preserves_original_tracks() {
    let authored = Tracks::from([(
        "ExpertSingle".into(),
        vec![Note {
            tick: 120,
            lane: 2,
            sustain: 360,
        }],
    )]);
    let original = midi::new_midi(&TempoMap::constant(120.).unwrap(), &authored).unwrap();
    let expected = fixture();
    let output = vocals::append_midi(&original, &expected).unwrap();
    assert_eq!(&output[14..original.len()], &original[14..]);
    let parsed = Smf::parse(&output).unwrap();
    assert_eq!(parsed.tracks.len(), 3);
    let mut actual = vocals::import_midi(&output).unwrap();
    actual.source = expected.source.clone();
    assert_eq!(actual, expected);
    let lyrics: Vec<_> = parsed.tracks[2]
        .iter()
        .filter_map(|event| match event.kind {
            TrackEventKind::Meta(MetaMessage::Lyric(text)) => {
                Some(std::str::from_utf8(text).unwrap())
            }
            _ => None,
        })
        .collect();
    assert_eq!(lyrics, ["La-", "+", "la", "Hey#", "Yo^"]);
    assert!(vocals::append_midi(&output, &expected).is_err());
}

#[test]
fn missing_lyric_preserves_pitch_and_midi_identity_without_a_word_or_slide() {
    let mut expected = fixture();
    expected.notes[0].lyric = Syllable::Missing;
    expected.notes[1].lyric = Syllable::Missing;
    expected.validate().unwrap();
    let original = midi::new_midi(&TempoMap::constant(120.).unwrap(), &Tracks::new()).unwrap();
    let output = vocals::append_midi(&original, &expected).unwrap();
    assert_eq!(&output[14..original.len()], &original[14..]);
    assert_eq!(vocals::import_midi(&output).unwrap().notes, expected.notes);
    let parsed = Smf::parse(&output).unwrap();
    let track = parsed.tracks.last().unwrap();
    assert_eq!(
        track
            .iter()
            .filter(|event| matches!(
                event.kind,
                TrackEventKind::Meta(MetaMessage::SequencerSpecific(b"YACE\0missing-lyric\0v1"))
            ))
            .count(),
        2
    );
    assert!(
        !track
            .iter()
            .any(|event| matches!(event.kind, TrackEventKind::Meta(MetaMessage::Lyric(b"+"))))
    );
    for corruption in 0..4 {
        let mut bad = Smf::parse(&output).unwrap();
        let track = bad.tracks.last_mut().unwrap();
        let index = track
            .iter()
            .position(|e| {
                matches!(
                    e.kind,
                    TrackEventKind::Meta(MetaMessage::SequencerSpecific(_))
                )
            })
            .unwrap();
        match corruption {
            0 => {
                track[index].kind =
                    TrackEventKind::Meta(MetaMessage::SequencerSpecific(b"YACE\0missing-lyric\0v2"))
            }
            1 => track[index + 1].kind = TrackEventKind::Meta(MetaMessage::Lyric(b"invented")),
            2 => track.insert(index, track[index]),
            _ => {
                track[index].kind =
                    TrackEventKind::Meta(MetaMessage::SequencerSpecific(b"YACE\0continue-word\0v1"))
            }
        }
        let mut bytes = Vec::new();
        bad.write_std(&mut bytes).unwrap();
        assert!(vocals::import_midi(&bytes).is_err());
    }
    expected.notes[1].lyric = Syllable::ContinueWord;
    assert!(
        expected.validate().is_err(),
        "A missing lyric cannot own a word continuation"
    );
    expected.notes[1].lyric = Syllable::Missing;
    expected.notes[0].tone = VocalTone::Talkie { lenient: false };
    assert!(expected.validate().is_err());
}

#[test]
fn continue_word_is_independent_pitch_with_exact_private_roundtrip_marker() {
    let mut expected = fixture();
    // This second measured contour has a gap. It still belongs to "La",
    // but must not assert the connecting slide represented by legacy '+'.
    expected.notes[1].lyric = Syllable::ContinueWord;
    expected.notes[1].start += 20;
    expected.validate().unwrap();
    let original = midi::new_midi(&TempoMap::constant(120.).unwrap(), &Tracks::new()).unwrap();
    let output = vocals::append_midi(&original, &expected).unwrap();
    let actual = vocals::import_midi(&output).unwrap();
    assert_eq!(actual.notes, expected.notes);
    assert_eq!(actual.phrases, expected.phrases);
    let parsed = Smf::parse(&output).unwrap();
    let track = parsed.tracks.last().unwrap();
    assert_eq!(
        track
            .iter()
            .filter(|event| matches!(
                event.kind,
                TrackEventKind::Meta(MetaMessage::SequencerSpecific(b"YACE\0continue-word\0v1"))
            ))
            .count(),
        1
    );
    let lyrics: Vec<_> = track
        .iter()
        .filter_map(|event| match event.kind {
            TrackEventKind::Meta(MetaMessage::Lyric(text)) => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(lyrics, [b"La-".as_slice(), b"", b"la", b"Hey#", b"Yo^"]);
    assert!(!lyrics.contains(&b"+".as_slice()));
    // YARG ignores SequencerSpecific meta and processes empty lyric with no
    // PitchSlide flag: its independent note has only the measured duration.
    // An external empty/missing lyric does not establish known-word ownership.
    for corrupt in 0..6 {
        let mut parsed = Smf::parse(&output).unwrap();
        let track = parsed.tracks.last_mut().unwrap();
        let marker = track
            .iter()
            .position(|event| {
                matches!(
                    event.kind,
                    TrackEventKind::Meta(MetaMessage::SequencerSpecific(_))
                )
            })
            .unwrap();
        let lyric = track
            .iter()
            .position(|event| matches!(event.kind, TrackEventKind::Meta(MetaMessage::Lyric(b""))))
            .unwrap();
        match corrupt {
            0 | 1 => {
                let index = if corrupt == 0 { marker } else { lyric };
                let removed = track.remove(index);
                track[index].delta = (track[index].delta.as_int() + removed.delta.as_int()).into();
            }
            2 => track[lyric].kind = TrackEventKind::Meta(MetaMessage::Lyric(b"invented")),
            3 => {
                let mut duplicate = track[marker];
                duplicate.delta = 0.into();
                track.insert(marker, duplicate);
            }
            4 => {
                track[marker].kind =
                    TrackEventKind::Meta(MetaMessage::SequencerSpecific(b"unknown"))
            }
            _ => {
                let mut orphan = track[marker];
                orphan.delta = 0.into();
                track.insert(1, orphan);
            }
        }
        let mut malformed = Vec::new();
        parsed.write_std(&mut malformed).unwrap();
        assert!(
            vocals::import_midi(&malformed).is_err(),
            "corruption {corrupt}"
        );
    }
}

#[test]
fn continue_word_needs_known_pitched_owner_in_same_phrase() {
    let valid = fixture();
    for index in [0, 3, 4] {
        let mut bad = valid.clone();
        bad.notes[index].lyric = Syllable::ContinueWord;
        assert!(bad.validate().is_err(), "orphan or talkie {index}");
    }
    let mut valid = valid;
    valid.notes[1].lyric = Syllable::ContinueWord;
    valid.validate().unwrap();
    // Old intentional slides stay exactly the same serialized kind.
    assert_eq!(
        serde_json::to_value(Syllable::Continue).unwrap()["kind"],
        "continue"
    );
    assert_eq!(
        serde_json::to_value(Syllable::ContinueWord).unwrap()["kind"],
        "continue_word"
    );
}

#[test]
fn harmony_tracks_and_trailing_midi_data_cannot_be_overwritten() {
    let original = midi::new_midi(&TempoMap::constant(120.).unwrap(), &Tracks::new()).unwrap();
    let output = vocals::append_midi(&original, &fixture()).unwrap();
    let mut parsed = Smf::parse(&output).unwrap();
    let name = parsed.tracks[1]
        .iter_mut()
        .find(|event| matches!(event.kind, TrackEventKind::Meta(MetaMessage::TrackName(_))))
        .unwrap();
    name.kind = TrackEventKind::Meta(MetaMessage::TrackName(b"HARM1"));
    let mut harmony = Vec::new();
    parsed.write_std(&mut harmony).unwrap();
    assert!(vocals::append_midi(&harmony, &fixture()).is_err());
    // A later TrackName must not conceal an existing vocal label from the guard.
    parsed.tracks[1][0].kind = TrackEventKind::Meta(MetaMessage::TrackName(b"PART GUITAR"));
    parsed.tracks[1].insert(
        1,
        midly::TrackEvent {
            delta: midly::num::u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::TrackName(b"PART VOCALS")),
        },
    );
    let mut ambiguous = Vec::new();
    parsed.write_std(&mut ambiguous).unwrap();
    assert!(vocals::append_midi(&ambiguous, &fixture()).is_err());
    let mut trailing = original.clone();
    trailing.extend_from_slice(b"junk");
    assert!(vocals::append_midi(&trailing, &fixture()).is_err());
    let mut trailing = output.clone();
    trailing.extend_from_slice(b"junk");
    assert!(vocals::import_midi(&trailing).is_err());
}

#[test]
fn malformed_phrases_and_polyphonic_lead_are_rejected() {
    let valid = fixture();
    let mut bad = valid.clone();
    bad.phrases.clear();
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.phrases[0].end = 1000;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.phrases[0].end = 3000;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.phrases.push(VocalPhrase {
        start: 4000,
        end: 5000,
        player: 1,
    });
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.notes[0].end = 1000;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.notes[0].end = bad.notes[0].start;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.phrases[0].player = 0;
    assert!(bad.validate().is_err());
    let mut bad = valid;
    bad.notes.reverse();
    assert!(bad.validate().is_err());
}

#[test]
fn control_symbols_invalid_pitch_and_orphan_syllables_are_rejected() {
    let valid = fixture();
    for text in [
        "",
        "two words",
        "word#",
        "[section]",
        "a\nb",
        "word-",
        "<b>word</b>",
        "a+b",
    ] {
        let mut bad = valid.clone();
        bad.notes[0].lyric = Syllable::Text {
            text: text.into(),
            join_next: false,
        };
        assert!(bad.validate().is_err(), "accepted {text:?}");
    }
    for midi in [0, 35, 85, 127] {
        let mut bad = valid.clone();
        bad.notes[0].tone = VocalTone::Pitched { midi };
        assert!(bad.validate().is_err());
    }
    let mut bad = valid.clone();
    bad.notes[0].lyric = Syllable::Continue;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.notes[3].lyric = Syllable::Continue;
    assert!(bad.validate().is_err());
    let mut bad = valid;
    bad.notes[4].lyric = Syllable::Text {
        text: "last".into(),
        join_next: true,
    };
    assert!(bad.validate().is_err());
}

#[test]
fn unicode_syllables_and_boundary_pitches_work() {
    let mut valid = fixture();
    valid.notes[0].tone = VocalTone::Pitched { midi: 36 };
    valid.notes[1].tone = VocalTone::Pitched { midi: 84 };
    valid.notes[2].lyric = Syllable::Text {
        text: "corazón".into(),
        join_next: false,
    };
    valid.validate().unwrap();
    let source = midi::new_midi(&TempoMap::constant(120.).unwrap(), &Tracks::new()).unwrap();
    let imported = vocals::import_midi(&vocals::append_midi(&source, &valid).unwrap()).unwrap();
    assert_eq!(imported.notes, valid.notes);
}

#[test]
fn mismatched_tick_resolution_and_empty_lyric_only_tracks_are_not_vocals() {
    let source = midi::new_midi(&TempoMap::constant(120.).unwrap(), &Tracks::new()).unwrap();
    assert!(vocals::import_midi(&source).is_err());
    let mut bad = fixture();
    bad.resolution = 192;
    assert!(vocals::append_midi(&source, &bad).is_err());
    bad = fixture();
    bad.notes.clear();
    assert!(bad.validate().is_err());
}

#[test]
fn importer_refuses_unsupported_markers_and_unaligned_lyrics() {
    let source = midi::new_midi(&TempoMap::constant(120.).unwrap(), &Tracks::new()).unwrap();
    let output = vocals::append_midi(&source, &fixture()).unwrap();
    let mut parsed = Smf::parse(&output).unwrap();
    let lyric = parsed.tracks[1]
        .iter_mut()
        .find(|event| matches!(event.kind, TrackEventKind::Meta(MetaMessage::Lyric(_))))
        .unwrap();
    lyric.kind = TrackEventKind::Meta(MetaMessage::Lyric(b"unsupported%"));
    let mut bytes = Vec::new();
    parsed.write_std(&mut bytes).unwrap();
    assert!(vocals::import_midi(&bytes).is_err());
    let mut parsed = Smf::parse(&output).unwrap();
    let index = parsed.tracks[1]
        .iter()
        .position(|event| matches!(event.kind, TrackEventKind::Meta(MetaMessage::Lyric(_))))
        .unwrap();
    // Shift the following note/phrase start, leaving the lyric at its old tick.
    parsed.tracks[1][index + 1].delta = midly::num::u28::new(1);
    let mut bytes = Vec::new();
    parsed.write_std(&mut bytes).unwrap();
    assert!(vocals::import_midi(&bytes).is_err());
}
