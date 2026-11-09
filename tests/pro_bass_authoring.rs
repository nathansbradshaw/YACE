use chart_engine::{
    editor::{Action, Actor, ActorKind, Request, Store},
    export_options::{Format, Options, Target},
    note_groups::{self, Selection},
    pro_guitar,
    star_power::{Edit, Phrase},
};
use midly::{MetaMessage as M, MidiMessage as N, TrackEventKind as K};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "Pro Bass regression".into(),
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
    let source = root.join("song");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("notes.chart"),"[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[Events]\n{\n0 = E \"section preserved\"\n}\n[ExpertSingle]\n{\n240 = N 0 120\n}\n").unwrap();
    fs::write(
        source.join("song.ini"),
        "[song]\nname = Pro Bass regression\nsong_length = 10000\ndiff_guitar = 3\n",
    )
    .unwrap();
    Store::create(&source, None, &root.join("project")).unwrap()
}
fn edit(store: &Store, id: &str, fret: u8, channel: u8, forced_hopo: bool) {
    // Existing command consumers retain their old spelling; Bass has its own accepted alias.
    let action=serde_json::from_value(serde_json::json!({"type":"edit_pro_bass_note","note_id":id,"fret":fret,"channel":channel,"forced_hopo":forced_hopo})).unwrap();
    apply(store, action);
}
fn authored(root: &Path) -> Store {
    let store = fixture(root);
    for (suffix, limit) in [("ProBass17", 17), ("ProBass22", 22)] {
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
                        tick: d as u64 * 1800 + u64::from(string) * 240,
                        lane: string,
                        sustain: 120,
                        cymbal: false,
                    },
                );
                let id = store.load().unwrap().selected_note.unwrap();
                edit(
                    &store,
                    &id,
                    if string == 5 { limit } else { string },
                    if string == 5 { 6 } else { string },
                    string % 2 == 1,
                );
            }
            assert!(
                chart_engine::parts::Target::capture(&store, &store.load().unwrap(), &track)
                    .is_err()
            );
        }
        apply(
            &store,
            Action::EditStarPower {
                edit: Edit::Put {
                    track: format!("Expert{suffix}"),
                    previous: None,
                    phrase: Phrase {
                        start: 480,
                        end: 1440,
                    },
                },
            },
        );
    }
    store
}
fn export(store: &Store, directory: &Path) {
    fs::create_dir(directory).unwrap();
    store
        .write_export_with_options(
            &store.load().unwrap(),
            directory,
            &actor(),
            &Options {
                include_audio: false,
                ..Default::default()
            },
        )
        .unwrap();
}
fn starts(track: &[midly::TrackEvent<'_>]) -> BTreeMap<(u64, u8), (u8, u8)> {
    let mut tick = 0;
    let mut out = BTreeMap::new();
    for event in track {
        tick += u64::from(event.delta.as_int());
        if let K::Midi {
            channel,
            message: N::NoteOn { key, vel },
        } = event.kind
            && vel.as_int() > 0
        {
            assert!(
                out.insert((tick, key.as_int()), (vel.as_int(), channel.as_int()))
                    .is_none()
            );
        }
    }
    out
}
fn track<'a>(smf: &'a midly::Smf<'a>, name: &[u8]) -> &'a [midly::TrackEvent<'a>] {
    smf.tracks
        .iter()
        .find(|t| {
            t.iter()
                .any(|e| matches!(e.kind,K::Meta(M::TrackName(n))if n==name))
        })
        .unwrap()
}

#[test]
fn eight_manual_parts_export_bass_strings_frets_and_phrases_then_reopen_plain_midi() {
    let root = tempfile::tempdir().unwrap();
    let store = authored(root.path());
    let output = root.path().join("export");
    export(&store, &output);
    assert!(!output.join("notes.chart").exists());
    let bytes = fs::read(output.join("notes.mid")).unwrap();
    let smf = midly::Smf::parse(&bytes).unwrap();
    for (name, limit) in [
        (b"PART REAL_BASS".as_slice(), 17),
        (b"PART REAL_BASS_22".as_slice(), 22),
    ] {
        let notes = starts(track(&smf, name));
        for (d, base) in [24u8, 48, 72, 96].into_iter().enumerate() {
            for string in 0..6u8 {
                let tick = d as u64 * 1800 + u64::from(string) * 240;
                assert_eq!(
                    notes.get(&(tick, base + string)),
                    Some(&(
                        100 + if string == 5 { limit } else { string },
                        if string == 5 { 6 } else { string }
                    ))
                );
                assert_eq!(notes.contains_key(&(tick, base + 6)), string % 2 == 1);
            }
        }
        assert_eq!(notes.get(&(480, 116)), Some(&(100, 0)));
        assert_eq!(notes.len(), 37);
    }
    let guitar = starts(track(&smf, b"PART GUITAR"));
    assert_eq!(guitar.get(&(240, 96)), Some(&(100, 0)));
    let ini = fs::read_to_string(output.join("song.ini")).unwrap();
    for rank in [
        "diff_bass_real = 0",
        "diff_bass_real_22 = 0",
        "diff_guitar = 3",
    ] {
        assert!(ini.contains(rank));
    }
    let fresh = root.path().join("ordinary");
    fs::create_dir(&fresh).unwrap();
    fs::write(fresh.join("notes.mid"), &bytes).unwrap();
    fs::write(fresh.join("song.ini"), ini).unwrap();
    let reopened = Store::create(&fresh, None, &root.path().join("reopened")).unwrap();
    let doc = reopened.load().unwrap();
    let source_power = reopened.source_star_power(&doc).unwrap();
    for (suffix, limit) in [("ProBass17", 17), ("ProBass22", 22)] {
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            let name = format!("{tier}{suffix}");
            let notes = &doc.content.tracks[&name];
            assert_eq!(notes.iter().filter(|n| n.note.lane < 6).count(), 6);
            let last = notes.iter().find(|n| n.note.lane == 5).unwrap();
            assert_eq!(pro_guitar::fret(last), limit);
            assert_eq!(pro_guitar::channel(last), 6);
            assert!(pro_guitar::forced(notes, last.note.tick));
            assert_eq!(
                source_power[&name].clone().unwrap(),
                vec![Phrase {
                    start: 480,
                    end: 1440
                }]
            );
        }
    }
    let filtered = root.path().join("filtered");
    fs::create_dir(&filtered).unwrap();
    reopened
        .write_export_with_options(
            &doc,
            &filtered,
            &actor(),
            &Options {
                tracks: Some(BTreeSet::from(["ExpertProBass22".into()])),
                include_audio: false,
                ..Default::default()
            },
        )
        .unwrap();
    let filtered_ini = fs::read_to_string(filtered.join("song.ini")).unwrap();
    assert!(filtered_ini.contains("diff_bass_real = -1"));
    assert!(filtered_ini.contains("diff_guitar = -1"));
}

#[test]
fn invalid_frets_and_foreign_clipboards_are_atomic_and_valid_bass_history_survives() {
    let root = tempfile::tempdir().unwrap();
    let store = authored(root.path());
    for track in ["ExpertProGuitar22", "ExpertDoubleBass"] {
        apply(
            &store,
            Action::CreateTrack {
                track: track.into(),
            },
        );
    }
    let doc = store.load().unwrap();
    let bass = &doc.content.tracks["ExpertProBass22"];
    let source = bass.iter().find(|n| n.note.lane == 5).unwrap();
    let clipboard = note_groups::copy(
        &doc,
        &doc.timing(&store).unwrap(),
        &Selection {
            track: "ExpertProBass22".into(),
            ids: BTreeSet::from([source.id.clone()]),
            ..Default::default()
        },
    )
    .unwrap();
    let clipboard = note_groups::Clipboard::decode(&clipboard.encode().unwrap()).unwrap();
    let before = fs::read(store.root.join("project.json")).unwrap();
    for destination in ["ExpertProBass17", "ExpertProGuitar22", "ExpertDoubleBass"] {
        assert!(
            store
                .apply(Request {
                    expected_revision: doc.revision,
                    actor: actor(),
                    action: Action::EditNoteGroup {
                        edit: note_groups::Edit::Paste {
                            track: destination.into(),
                            tick: 8000,
                            clipboard: clipboard.clone()
                        }
                    }
                })
                .is_err()
        );
        assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    }
    let target = doc.content.tracks["ExpertProBass17"]
        .iter()
        .find(|n| n.note.lane == 5)
        .unwrap();
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
    apply(
        &store,
        Action::EditNoteGroup {
            edit: note_groups::Edit::Paste {
                track: "ExpertProBass22".into(),
                tick: 8000,
                clipboard,
            },
        },
    );
    let restored = store.load().unwrap();
    let note = restored.content.tracks["ExpertProBass22"]
        .iter()
        .find(|n| n.note.tick == 8000 && n.note.lane == 5)
        .unwrap();
    assert_eq!(pro_guitar::fret(note), 22);
    assert_eq!(pro_guitar::channel(note), 6);
    assert!(pro_guitar::forced(
        &restored.content.tracks["ExpertProBass22"],
        8000
    ));
    apply(&store, Action::Undo);
    assert!(
        !store.load().unwrap().content.tracks["ExpertProBass22"]
            .iter()
            .any(|n| n.note.tick == 8000)
    );
    apply(&store, Action::Redo);
    assert!(
        Store::new(&store.root).load().unwrap().content.tracks["ExpertProBass22"]
            .iter()
            .any(|n| n.note.tick == 8000 && n.note.lane == 5)
    );
    for options in [
        Options {
            format: Format::Chart,
            ..Default::default()
        },
        Options {
            target: Target::CloneHero,
            ..Default::default()
        },
    ] {
        assert!(options.selected(&store, &store.load().unwrap()).is_err());
    }
}
