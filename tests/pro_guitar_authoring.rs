use chart_engine::{
    editor::{Action, Actor, ActorKind, Request, Store},
    export_options::{Format, Options},
    note_groups::{self, Selection},
    pro_guitar,
};
use std::{collections::BTreeSet, fs, path::Path};

fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "Pro Guitar regression".into(),
    }
}
fn apply(store: &Store, action: Action) {
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: actor(),
            action,
        })
        .unwrap();
}
fn fixture(root: &Path) -> Store {
    let song = root.join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"), "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[Events]\n{\n}\n[ExpertSingle]\n{\n0 = N 0 0\n}\n").unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Pro Guitar regression\nsong_length = 10000\n",
    )
    .unwrap();
    Store::create(&song, None, &root.join("project")).unwrap()
}
fn edited_fixture(root: &Path) -> Store {
    let store = fixture(root);
    for (suffix, limit) in [("ProGuitar17", 17u8), ("ProGuitar22", 22)] {
        for (d, tier) in ["Easy", "Medium", "Hard", "Expert"].into_iter().enumerate() {
            let track = format!("{tier}{suffix}");
            apply(
                &store,
                Action::CreateTrack {
                    track: track.clone(),
                },
            );
            for string in 0..6u8 {
                apply(
                    &store,
                    Action::AddNote {
                        track: track.clone(),
                        tick: d as u64 * 1800 + string as u64 * 240,
                        lane: string,
                        sustain: 120,
                        cymbal: false,
                    },
                );
                let id = store.load().unwrap().selected_note.unwrap();
                apply(
                    &store,
                    Action::EditProGuitarNote {
                        note_id: id,
                        fret: if string == 5 { limit } else { string + 1 },
                        channel: string,
                        forced_hopo: string % 2 == 1,
                    },
                );
            }
        }
    }
    store
}
fn export(store: &Store, output: &Path) {
    fs::create_dir(output).unwrap();
    store
        .write_export_with_options(
            &store.load().unwrap(),
            output,
            &actor(),
            &Options {
                include_audio: false,
                ..Default::default()
            },
        )
        .unwrap();
}

#[test]
fn eight_parts_export_strings_frets_channels_and_hopo_then_reimport_without_private_metadata() {
    let root = tempfile::tempdir().unwrap();
    let store = edited_fixture(root.path());
    let output = root.path().join("export");
    export(&store, &output);
    assert!(output.join("notes.mid").is_file());
    assert!(!output.join("notes.chart").exists());
    let ini = fs::read_to_string(output.join("song.ini")).unwrap();
    assert!(ini.contains("diff_guitar_real = 0"));
    assert!(ini.contains("diff_guitar_real_22 = 0"));
    let midi = fs::read(output.join("notes.mid")).unwrap();
    let smf = midly::Smf::parse(&midi).unwrap();
    for (name, limit) in [
        (b"PART REAL_GUITAR".as_slice(), 17),
        (b"PART REAL_GUITAR_22".as_slice(), 22),
    ] {
        let track = smf.tracks.iter().find(|t| t.iter().any(|e| matches!(e.kind,midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(n)) if n==name))).unwrap();
        let mut starts = BTreeSet::new();
        let mut tick = 0u64;
        for e in track {
            tick += u64::from(e.delta.as_int());
            if let midly::TrackEventKind::Midi {
                channel,
                message: midly::MidiMessage::NoteOn { key, vel },
            } = e.kind
                && vel.as_int() > 0
            {
                starts.insert((tick, key.as_int(), vel.as_int(), channel.as_int()));
            }
        }
        for (d, base) in [24u8, 48, 72, 96].into_iter().enumerate() {
            for string in 0..6u8 {
                let tick = d as u64 * 1800 + string as u64 * 240;
                assert!(
                    starts.contains(&(
                        tick,
                        base + string,
                        100 + if string == 5 { limit } else { string + 1 },
                        string
                    )),
                    "wrong string/fret/channel at {tick}"
                );
                assert_eq!(starts.contains(&(tick, base + 6, 100, 0)), string % 2 == 1);
            }
        }
        assert_eq!(starts.len(), 36);
    }
    // Fresh ordinary MIDI and INI only: no working chart, import descriptor or backup.
    let fresh = root.path().join("ordinary");
    fs::create_dir(&fresh).unwrap();
    fs::write(fresh.join("notes.mid"), &midi).unwrap();
    fs::copy(output.join("song.ini"), fresh.join("song.ini")).unwrap();
    let reopened = Store::create(&fresh, None, &root.path().join("reopened")).unwrap();
    let doc = reopened.load().unwrap();
    for (suffix, limit) in [("ProGuitar17", 17), ("ProGuitar22", 22)] {
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            let track = format!("{tier}{suffix}");
            let notes = &doc.content.tracks[&track];
            assert_eq!(notes.iter().filter(|n| n.note.lane < 6).count(), 6);
            let last = notes.iter().find(|n| n.note.lane == 5).unwrap();
            assert_eq!(pro_guitar::fret(last), limit);
            assert_eq!(pro_guitar::channel(last), 5);
            assert!(pro_guitar::forced(notes, last.note.tick));
            assert_eq!(
                chart_engine::gameplay_view::classify(
                    &track,
                    &notes.iter().map(|n| n.note.clone()).collect::<Vec<_>>(),
                    &doc.timing(&reopened).unwrap(),
                    &chart_engine::gameplay_view::Policy::unknown()
                )
                .unwrap()
                .len(),
                6
            );
        }
    }
}

#[test]
fn fret_and_articulation_changes_invalidate_review_even_when_geometry_is_unchanged() {
    use chart_engine::review_regions::{Priority, Region, ReviewStatus};
    let root = tempfile::tempdir().unwrap();
    let store = edited_fixture(root.path());
    let mut doc = store.load().unwrap();
    let track = "ExpertProGuitar22";
    doc.review.regions.push(Region {
        id: "pro-fret-review".into(),
        start: 0.,
        end: 10.,
        tracks: BTreeSet::from([track.into()]),
        priority: Priority::Attention,
        status: ReviewStatus::Pending,
        issues: vec![],
    });
    let timing = doc.timing(&store).unwrap();
    let before = doc
        .region_fingerprint("pro-fret-review", track, &timing)
        .unwrap();
    let note = doc.content.tracks[track]
        .iter()
        .find(|n| n.note.lane == 5)
        .unwrap()
        .clone();
    pro_guitar::edit(&mut doc, &note.id, 21, 5, true).unwrap();
    assert_eq!(doc.find_note(&note.id).unwrap().1.note, note.note);
    let fretted = doc
        .region_fingerprint("pro-fret-review", track, &timing)
        .unwrap();
    assert_ne!(before, fretted);
    pro_guitar::edit(&mut doc, &note.id, 21, 3, true).unwrap();
    assert_ne!(
        fretted,
        doc.region_fingerprint("pro-fret-review", track, &timing)
            .unwrap()
    );
}

#[test]
fn invalid_fret_is_atomic_and_clipboard_history_retains_sixth_string() {
    let root = tempfile::tempdir().unwrap();
    let store = edited_fixture(root.path());
    let doc = store.load().unwrap();
    let source = doc.content.tracks["ExpertProGuitar22"]
        .iter()
        .find(|n| n.note.lane == 5)
        .unwrap();
    let selection = Selection {
        track: "ExpertProGuitar22".into(),
        ids: BTreeSet::from([source.id.clone()]),
        ..Default::default()
    };
    let clipboard = note_groups::Clipboard::decode(
        &note_groups::copy(&doc, &doc.timing(&store).unwrap(), &selection)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    let target = doc.content.tracks["ExpertProGuitar17"]
        .iter()
        .find(|n| n.note.lane == 5)
        .unwrap();
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        store
            .apply(Request {
                expected_revision: doc.revision,
                actor: actor(),
                action: Action::EditProGuitarNote {
                    note_id: target.id.clone(),
                    fret: 18,
                    channel: 0,
                    forced_hopo: false
                }
            })
            .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    assert!(
        store
            .apply(Request {
                expected_revision: doc.revision,
                actor: actor(),
                action: Action::EditNoteGroup {
                    edit: note_groups::Edit::Paste {
                        track: "ExpertSingle".into(),
                        tick: 8000,
                        clipboard: clipboard.clone()
                    }
                }
            })
            .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    apply(
        &store,
        Action::EditNoteGroup {
            edit: note_groups::Edit::Paste {
                track: "ExpertProGuitar22".into(),
                tick: 8000,
                clipboard,
            },
        },
    );
    let pasted = store.load().unwrap();
    let notes = &pasted.content.tracks["ExpertProGuitar22"];
    let note = notes
        .iter()
        .find(|n| n.note.tick == 8000 && n.note.lane == 5)
        .unwrap();
    assert_eq!(pro_guitar::fret(note), 22);
    assert!(pro_guitar::forced(notes, 8000));
    apply(&store, Action::Undo);
    assert!(
        !store.load().unwrap().content.tracks["ExpertProGuitar22"]
            .iter()
            .any(|n| n.note.tick == 8000)
    );
    apply(&store, Action::Redo);
    assert!(
        store.load().unwrap().content.tracks["ExpertProGuitar22"]
            .iter()
            .any(|n| n.note.tick == 8000 && n.note.lane == 5)
    );
    let options = Options {
        format: Format::Chart,
        ..Default::default()
    };
    assert!(options.selected(&store, &store.load().unwrap()).is_err());
}

fn named_chunk<'a>(bytes: &'a [u8], name: &[u8]) -> &'a [u8] {
    let midi = midly::Smf::parse(bytes).unwrap();
    let mut index = 0;
    let mut position = 8 + u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    while position < bytes.len() {
        assert_eq!(&bytes[position..position + 4], b"MTrk");
        let end = position
            + 8
            + u32::from_be_bytes(bytes[position + 4..position + 8].try_into().unwrap()) as usize;
        let chunk = &bytes[position..end];
        if midi.tracks[index].iter().any(|event| {
            matches!(event.kind,
            midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(label)) if label == name)
        }) {
            return chunk;
        }
        position = end;
        index += 1;
    }
    panic!("missing track");
}

#[test]
fn edited_import_keeps_unrelated_raw_track_and_unmanaged_controls_and_note_off_attributes() {
    for bass in [false, true] {
        edited_import_preserves_source(bass);
    }
}

fn edited_import_preserves_source(bass: bool) {
    use midly::{MetaMessage as M, MidiMessage as N, TrackEvent as E, TrackEventKind as K};
    let root = tempfile::tempdir().unwrap();
    let original = edited_fixture(root.path());
    let output = root.path().join("first");
    export(&original, &output);
    let bytes = fs::read(output.join("notes.mid")).unwrap();
    let mut smf = midly::Smf::parse(&bytes).unwrap();
    let midi_name: &[u8] = if bass {
        b"PART REAL_BASS_22"
    } else {
        b"PART REAL_GUITAR_22"
    };
    let track_name = if bass {
        "ExpertProBass22"
    } else {
        "ExpertProGuitar22"
    };
    if bass {
        for events in &mut smf.tracks {
            for event in events {
                if matches!(event.kind, K::Meta(M::TrackName(b"PART REAL_GUITAR_22"))) {
                    event.kind = K::Meta(M::TrackName(midi_name));
                }
            }
        }
    }
    let pro = smf
        .tracks
        .iter_mut()
        .find(|t| {
            t.iter()
                .any(|e| matches!(e.kind, K::Meta(M::TrackName(name)) if name == midi_name))
        })
        .unwrap();
    pro.insert(
        1,
        E {
            delta: 0.into(),
            kind: K::Meta(M::Text(b"[chrd0 preserved chord name]")),
        },
    );
    pro.insert(
        2,
        E {
            delta: 0.into(),
            kind: K::Midi {
                channel: 9.into(),
                message: N::Controller {
                    controller: 11.into(),
                    value: 71.into(),
                },
            },
        },
    );
    for event in pro.iter_mut() {
        if matches!(event.kind,K::Midi{channel,message:N::NoteOff{key,..}} if channel.as_int()==5 && key.as_int()==101)
        {
            event.kind = K::Midi {
                channel: 5.into(),
                message: N::NoteOff {
                    key: 101.into(),
                    vel: 73.into(),
                },
            };
        }
    }
    smf.tracks.push(vec![
        E {
            delta: 0.into(),
            kind: K::Meta(M::TrackName(b"PART REAL_BASS")),
        },
        E {
            delta: 123.into(),
            kind: K::Meta(M::Text(b"opaque bass control")),
        },
        E {
            delta: 0.into(),
            kind: K::Midi {
                channel: 7.into(),
                message: N::NoteOn {
                    key: 96.into(),
                    vel: 112.into(),
                },
            },
        },
        E {
            delta: 321.into(),
            kind: K::Midi {
                channel: 7.into(),
                message: N::NoteOff {
                    key: 96.into(),
                    vel: 43.into(),
                },
            },
        },
        E {
            delta: 0.into(),
            kind: K::Meta(M::EndOfTrack),
        },
    ]);
    let mut source_bytes = Vec::new();
    smf.write_std(&mut source_bytes).unwrap();
    let source = root.path().join("ordinary");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("notes.mid"), &source_bytes).unwrap();
    fs::copy(output.join("song.ini"), source.join("song.ini")).unwrap();
    let store = Store::create(&source, None, &root.path().join("imported")).unwrap();
    let doc = store.load().unwrap();
    let note = doc.content.tracks[track_name]
        .iter()
        .find(|n| n.note.lane == 5)
        .unwrap();
    assert_eq!(note.midi.as_ref().unwrap().off_velocity, 73);
    apply(
        &store,
        Action::EditProGuitarNote {
            note_id: note.id.clone(),
            fret: 19,
            channel: 3,
            forced_hopo: true,
        },
    );
    let edited = root.path().join("edited");
    export(&store, &edited);
    let final_bytes = fs::read(edited.join("notes.mid")).unwrap();
    assert_eq!(
        named_chunk(&source_bytes, b"PART REAL_BASS"),
        named_chunk(&final_bytes, b"PART REAL_BASS")
    );
    let final_midi = midly::Smf::parse(&final_bytes).unwrap();
    let pro = final_midi
        .tracks
        .iter()
        .find(|t| {
            t.iter()
                .any(|e| matches!(e.kind, K::Meta(M::TrackName(name)) if name == midi_name))
        })
        .unwrap();
    assert!(
        pro.iter()
            .any(|e| matches!(e.kind, K::Meta(M::Text(b"[chrd0 preserved chord name]"))))
    );
    assert!(pro.iter().any(|e|matches!(e.kind,K::Midi{channel,message:N::Controller{controller,value}} if channel.as_int()==9 && controller.as_int()==11 && value.as_int()==71)));
    assert!(pro.iter().any(|e|matches!(e.kind,K::Midi{channel,message:N::NoteOff{key,vel}} if channel.as_int()==3 && key.as_int()==101 && vel.as_int()==73)));
    assert_eq!(fs::read(source.join("notes.mid")).unwrap(), source_bytes);
    // Invalid fret velocity refuses the import rather than clamping an original performance.
    let mut invalid = midly::Smf::parse(&source_bytes).unwrap();
    let pro = invalid
        .tracks
        .iter_mut()
        .find(|t| {
            t.iter()
                .any(|e| matches!(e.kind, K::Meta(M::TrackName(name)) if name == midi_name))
        })
        .unwrap();
    let event = pro
        .iter_mut()
        .find(|e| matches!(e.kind,K::Midi{message:N::NoteOn{key,..},..} if key.as_int()==101))
        .unwrap();
    if let K::Midi {
        message: N::NoteOn { vel, .. },
        ..
    } = &mut event.kind
    {
        *vel = 99.into();
    }
    let mut invalid_bytes = Vec::new();
    invalid.write_std(&mut invalid_bytes).unwrap();
    fs::write(source.join("notes.mid"), &invalid_bytes).unwrap();
    assert!(Store::create(&source, None, &root.path().join("refused")).is_err());
    assert_eq!(fs::read(source.join("notes.mid")).unwrap(), invalid_bytes);
}
