use super::*;
use crate::editor::{Action, Actor, ActorKind, Request, Store};
use std::{fs, path::Path};

fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "Vocal overdrive regression".into(),
    }
}
fn apply(store: &Store, action: Action) -> anyhow::Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: actor(),
        action,
    })?;
    Ok(())
}
fn project(midi: bool) -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let song = root.path().join("song");
    fs::create_dir(&song).unwrap();
    if midi {
        fs::write(song.join("notes.mid"), super::tests::original()).unwrap();
    } else {
        fs::write(song.join("notes.chart"), "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[ExpertSingle]\n{\n240 = N 0 120\n}\n").unwrap();
    }
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Power fixture\nsong_length = 12000\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    (root, store)
}
fn current(store: &Store) -> State {
    let project = store.load().unwrap();
    project
        .content
        .vocal_overdrive
        .clone()
        .unwrap_or_else(|| store.source_vocal_overdrive(&project).unwrap())
}
fn edit(
    store: &Store,
    part: Part,
    previous: Option<Phrase>,
    start: u64,
    end: u64,
) -> anyhow::Result<()> {
    apply(
        store,
        Action::EditVocalOverdrive {
            part,
            edit: Edit::Put {
                previous,
                range: Range { start, end },
            },
        },
    )
}
fn export(store: &Store, path: &Path, tracks: Option<BTreeSet<String>>) -> Vec<u8> {
    store
        .export_with_options(
            &store.load().unwrap(),
            path,
            &actor(),
            &crate::export_options::Options {
                tracks,
                include_audio: false,
                ..Default::default()
            },
        )
        .unwrap();
    fs::read(path.join("notes.mid")).unwrap()
}
fn markers(bytes: &[u8], part: Part) -> Vec<Range> {
    import_midi(bytes)
        .unwrap()
        .tracks
        .get(&part)
        .map_or_else(Vec::new, |t| {
            t.phrases.iter().map(|p| p.range.clone()).collect()
        })
}
fn singing() -> crate::vocals::VocalChart {
    crate::vocals::VocalChart {
        version: 1,
        resolution: 480,
        source: "Power fixture singer".into(),
        phrases: vec![crate::vocals::VocalPhrase {
            midi: None,
            start: 1920,
            end: 2880,
            player: 1,
        }],
        notes: vec![crate::vocals::VocalNote {
            start: 2040,
            end: 2400,
            tone: crate::vocals::VocalTone::Pitched { midi: 67 },
            lyric: crate::vocals::Syllable::Missing,
            midi: None,
        }],
    }
}

#[test]
fn vocal_overdrive_project_history_export_and_reopen_preserve_exact_markers() {
    let (root, store) = project(true);
    assert!(store.load().unwrap().content.harmonies.is_some());
    let inherited = current(&store);
    let previous = inherited.tracks[&Part::Lead].phrases[1].clone();
    edit(&store, Part::Lead, Some(previous.clone()), 840, 1200).unwrap();
    let moved = current(&store);
    let saved = fs::read(store.root.join("project.json")).unwrap();
    assert!(edit(&store, Part::Lead, Some(previous), 1440, 1680).is_err());
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), saved);
    apply(&store, Action::Undo).unwrap();
    assert_eq!(current(&store), inherited);
    apply(&store, Action::Redo).unwrap();
    assert_eq!(current(&Store::new(&store.root)), moved);
    edit(&store, Part::Harm1, None, 1920, 2400).unwrap();
    let output = root.path().join("export");
    let bytes = export(&store, &output, None);
    assert_eq!(
        markers(&bytes, Part::Lead)[1],
        Range {
            start: 840,
            end: 1200
        }
    );
    let parsed = import_midi(&bytes).unwrap();
    assert_eq!(
        parsed.tracks[&Part::Lead].phrases[1].midi,
        moved.tracks[&Part::Lead].phrases[1].midi
    );
    assert_eq!(markers(&bytes, Part::Harm1).len(), 2);
    let original = super::tests::original();
    let (_, old) = game_midi::midi_chunks(&original, 4).unwrap();
    let smf = Smf::parse(&bytes).unwrap();
    let (_, new) = game_midi::midi_chunks(&bytes, smf.tracks.len()).unwrap();
    assert_eq!(
        new.iter().filter(|chunk| old[2..].contains(chunk)).count(),
        2
    );
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(
        markers(&bytes, Part::Lead),
        current(&reopened).tracks[&Part::Lead]
            .phrases
            .iter()
            .map(|p| p.range.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(export(&reopened, &root.path().join("again"), None), bytes);
    for part in [Part::Lead, Part::Harm1] {
        for phrase in current(&reopened).tracks[&part].phrases.clone() {
            apply(
                &reopened,
                Action::EditVocalOverdrive {
                    part,
                    edit: Edit::Remove { phrase },
                },
            )
            .unwrap();
        }
    }
    let clear_dir = root.path().join("clear");
    let clear = export(&reopened, &clear_dir, None);
    assert!(markers(&clear, Part::Lead).is_empty());
    assert!(markers(&clear, Part::Harm1).is_empty());
    let cleared = Store::create(&clear_dir, None, &root.path().join("cleared")).unwrap();
    assert!(
        current(&cleared)
            .tracks
            .values()
            .all(|t| t.phrases.is_empty())
    );
    assert_eq!(
        export(&cleared, &root.path().join("clear-again"), None),
        clear
    );
}

#[test]
fn vocal_overdrive_omitted_singers_can_be_reauthored_without_reviving_source_power() {
    let (root, store) = project(true);
    edit(&store, Part::Lead, None, 1920, 2400).unwrap();
    let output = root.path().join("omit");
    let bytes = export(&store, &output, Some(["ExpertSingle".to_owned()].into()));
    assert!(markers(&bytes, Part::Lead).is_empty());
    assert!(markers(&bytes, Part::Harm1).is_empty());
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert!(reopened.load().unwrap().content.vocals.is_none());
    assert!(reopened.load().unwrap().content.harmonies.is_none());
    apply(
        &reopened,
        Action::SetVocals {
            document: singing(),
        },
    )
    .unwrap();
    edit(&reopened, Part::Lead, None, 1920, 2880).unwrap();
    let later = export(&reopened, &root.path().join("new-singer"), None);
    assert_eq!(
        markers(&later, Part::Lead),
        vec![Range {
            start: 1920,
            end: 2880
        }]
    );
    assert!(markers(&later, Part::Harm1).is_empty());
    let new_project = Store::create(
        &root.path().join("new-singer"),
        None,
        &root.path().join("new-project"),
    )
    .unwrap();
    let voices = new_project.load().unwrap().content.vocals.unwrap();
    assert_eq!(voices.notes.len(), 1);
    assert_eq!(voices.notes[0].start, 2040);
}

#[test]
fn vocal_overdrive_raw_marker_only_owner_obeys_scope_and_new_singer_authoring() {
    let (root, _) = project(false);
    let original = super::tests::original();
    let smf = Smf::parse(&original).unwrap();
    let events = game_midi::midi_events(&smf.tracks[0]).into_iter().enumerate().filter(|(_,(_,e))| {
        matches!(e, Event::Midi { message: Note::NoteOn {key,..} | Note::NoteOff {key,..},..} if key.as_int()==116)
        || matches!(e, Event::Meta(Meta::TrackName(_) | Meta::EndOfTrack) | Event::Midi {message:Note::Controller {..},..})
    }).map(|(i,(t,e))|(t,1,i,e)).collect();
    let raw = game_midi::encode_midi_owner(events, smf.header).unwrap();
    let (_, chunks) = game_midi::midi_chunks(&original, smf.tracks.len()).unwrap();
    let mut bytes = original[..14].to_vec();
    bytes.extend(raw);
    for chunk in &chunks[1..] {
        bytes.extend(*chunk);
    }
    let song = root.path().join("raw-song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.mid"), bytes).unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Marker-only power\nsong_length = 12000\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("raw-project")).unwrap();
    assert!(store.load().unwrap().content.vocals.is_none());
    edit(&store, Part::Lead, None, 1920, 2400).unwrap();
    let available = crate::export_options::available(&store, &store.load().unwrap()).unwrap();
    assert!(available.contains("Vocals"));
    assert!(!available.contains("MIDI: PART VOCALS"));
    let authored = export(&store, &root.path().join("raw-authored"), None);
    assert_eq!(markers(&authored, Part::Lead).len(), 3);
    let omitted = root.path().join("raw-omit");
    export(&store, &omitted, Some(["ExpertSingle".to_owned()].into()));
    let reopened = Store::create(&omitted, None, &root.path().join("raw-reopened")).unwrap();
    apply(
        &reopened,
        Action::SetVocals {
            document: singing(),
        },
    )
    .unwrap();
    edit(&reopened, Part::Lead, None, 1920, 2880).unwrap();
    let later = export(&reopened, &root.path().join("raw-new-singer"), None);
    assert_eq!(
        markers(&later, Part::Lead),
        vec![Range {
            start: 1920,
            end: 2880
        }]
    );
}

#[test]
fn vocal_overdrive_chart_origin_authoring_exports_and_reopens_new_lead_owner() {
    let (root, store) = project(false);
    assert!(edit(&store, Part::Lead, None, 1920, 2880).is_err());
    apply(
        &store,
        Action::SetVocals {
            document: singing(),
        },
    )
    .unwrap();
    edit(&store, Part::Lead, None, 1920, 2880).unwrap();
    let path = root.path().join("export");
    let bytes = export(&store, &path, None);
    assert_eq!(
        markers(&bytes, Part::Lead),
        vec![Range {
            start: 1920,
            end: 2880
        }]
    );
    let reopened = Store::create(&path, None, &root.path().join("reopened")).unwrap();
    assert_eq!(export(&reopened, &root.path().join("again"), None), bytes);
}

#[test]
fn vocal_overdrive_first_power_in_an_inherited_percussion_owner_preserves_its_controls() {
    let (root, _) = project(false);
    let original = super::tests::original();
    let smf = Smf::parse(&original).unwrap();
    let mut events: Vec<_> = game_midi::midi_events(&smf.tracks[1])
        .into_iter()
        .enumerate()
        .filter(|(_, (_, e))| matches!(e, Event::Meta(Meta::TrackName(_) | Meta::EndOfTrack)))
        .map(|(i, (t, e))| (t, 1, i, e))
        .collect();
    for (tick, on, key, velocity) in [
        (3600, true, 105, 92),
        (4560, false, 105, 23),
        (3720, true, 96, 79),
        (3768, false, 96, 21),
    ] {
        events.push((
            tick,
            1,
            events.len(),
            Event::Midi {
                channel: u4::new(4),
                message: if on {
                    Note::NoteOn {
                        key: u7::new(key),
                        vel: u7::new(velocity),
                    }
                } else {
                    Note::NoteOff {
                        key: u7::new(key),
                        vel: u7::new(velocity),
                    }
                },
            },
        ));
    }
    let raw = game_midi::encode_midi_owner(events, smf.header).unwrap();
    let (_, chunks) = game_midi::midi_chunks(&original, 4).unwrap();
    let mut bytes = original[..14].to_vec();
    bytes.extend_from_slice(chunks[0]);
    bytes.extend(raw);
    bytes.extend_from_slice(chunks[2]);
    bytes.extend_from_slice(chunks[3]);
    let song = root.path().join("pc-song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.mid"), &bytes).unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = First harmony power\nsong_length = 12000\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("pc-project")).unwrap();
    let doc = store.load().unwrap();
    assert!(doc.content.harmonies.is_none());
    assert!(doc.content.vocal_percussion.is_none());
    assert!(current(&store).tracks[&Part::Harm1].phrases.is_empty());
    edit(&store, Part::Harm1, None, 3600, 4560).unwrap();
    let output = export(&store, &root.path().join("pc-export"), None);
    assert_eq!(
        markers(&output, Part::Harm1),
        vec![Range {
            start: 3600,
            end: 4560
        }]
    );
    let retained = |data: &[u8]| {
        let midi = Smf::parse(data).unwrap();
        let track = midi
            .tracks
            .iter()
            .find(|track| {
                track
                    .iter()
                    .any(|e| matches!(e.kind, Event::Meta(Meta::TrackName(b"PART HARM1"))))
            })
            .unwrap();
        game_midi::midi_events(track)
            .into_iter()
            .filter(|(t, e)| edge(*t, *e).is_none())
            .map(|(t, e)| (t, format!("{e:?}")))
            .collect::<Vec<_>>()
    };
    assert_eq!(retained(&output), retained(&bytes));
}
