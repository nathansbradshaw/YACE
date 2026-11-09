use super::*;
use crate::editor::{Action, Actor, ActorKind, Content, Request, Store};
use std::{fs, path::Path};

fn project(midi: bool, mixed: bool) -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let song = root.path().join("song");
    fs::create_dir(&song).unwrap();
    let text = "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[ExpertSingle]\n{\n240 = N 0 120\n}\n";
    if midi {
        let chart = crate::chart::Chart::parse(text.into()).unwrap();
        let base = crate::game_export::song(&chart, None).unwrap().0;
        let mut bytes = append_midi(&base, &super::tests::fixture()).unwrap();
        if mixed {
            use midly::{
                MetaMessage as M, MidiMessage as N, TrackEventKind as K,
                num::{u4, u7, u28},
            };
            let mut source = midly::Smf::parse(&bytes).unwrap();
            let track = source
                .tracks
                .iter_mut()
                .find(|track| {
                    track
                        .iter()
                        .any(|event| matches!(event.kind, K::Meta(M::TrackName(b"PART VOCALS"))))
                })
                .unwrap();
            let mut tick = 0;
            let mut events: Vec<_> = track
                .iter()
                .map(|event| {
                    tick += u64::from(event.delta.as_int());
                    (tick, event.kind)
                })
                .collect();
            events.extend([
                (
                    600,
                    K::Midi {
                        channel: u4::new(3),
                        message: N::NoteOn {
                            key: u7::new(60),
                            vel: u7::new(86),
                        },
                    },
                ),
                (
                    720,
                    K::Midi {
                        channel: u4::new(3),
                        message: N::NoteOff {
                            key: u7::new(60),
                            vel: u7::new(9),
                        },
                    },
                ),
            ]);
            events.sort_by_key(|event| (event.0, matches!(event.1, K::Meta(M::EndOfTrack))));
            let mut previous = 0;
            *track = events
                .into_iter()
                .map(|(tick, kind)| {
                    let delta = u28::new((tick - previous) as u32);
                    previous = tick;
                    midly::TrackEvent { delta, kind }
                })
                .collect();
            let mut changed = Vec::new();
            source.write_std(&mut changed).unwrap();
            bytes = changed;
        }
        fs::write(song.join("notes.mid"), bytes).unwrap();
    } else {
        fs::write(song.join("notes.chart"), text).unwrap();
    }
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Percussion project fixture\nsong_length = 4000\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    (root, store)
}
fn apply(store: &Store, action: Action) -> anyhow::Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Percussion project regression".into(),
        },
        action,
    })?;
    Ok(())
}
fn authored(store: &Store) -> Option<PercussionChart> {
    store.vocal_percussion(&store.load().unwrap()).unwrap()
}
#[test]
fn neutral_content_keeps_the_legacy_serialization_and_inherits_original_hits_without_writes() {
    let (_root, store) = project(true, false);
    let path = store.root.join("project.json");
    let before = fs::read(&path).unwrap();
    let project = store.load().unwrap();
    let bytes = serde_json::to_vec(&project.content).unwrap();
    assert!(
        !serde_json::to_value(&project.content)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("vocal_percussion")
    );
    let legacy: Content = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&legacy).unwrap(), bytes);
    assert!(project.content.vocal_percussion.is_none());
    let chart = authored(&store).unwrap();
    assert_eq!(chart.tracks.len(), 2);
    assert_eq!(fs::read(path).unwrap(), before);
}
#[test]
fn explicit_clear_survives_restart_and_history_without_resurrecting_original_percussion() {
    let (_root, store) = project(true, false);
    let original = authored(&store).unwrap();
    apply(&store, Action::ClearVocalPercussion).unwrap();
    let reopened = Store::new(&store.root);
    assert_eq!(
        reopened.load().unwrap().content.vocal_percussion,
        Some(State::Cleared)
    );
    assert!(authored(&reopened).is_none());
    apply(&reopened, Action::Undo).unwrap();
    assert_eq!(authored(&reopened), Some(original));
    apply(&reopened, Action::Redo).unwrap();
    assert!(authored(&reopened).is_none());
    assert!(
        crate::song_files::game_midi::percussion_view(&store.root.join("source"))
            .unwrap()
            .is_some()
    );
}
#[test]
fn instrument_only_export_does_not_revive_omitted_original_percussion_on_reopen() {
    let (root, store) = project(true, false);
    let output = root.path().join("instrument-only");
    let options = crate::export_options::Options {
        tracks: Some(["ExpertSingle".to_owned()].into()),
        include_audio: false,
        ..Default::default()
    };
    store
        .export_with_options(
            &store.load().unwrap(),
            &output,
            &Actor {
                kind: ActorKind::Human,
                name: "Percussion omission regression".into(),
            },
            &options,
        )
        .unwrap();
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert!(reopened.load().unwrap().content.vocal_percussion.is_none());
    assert!(authored(&reopened).is_none());
    assert!(authored(&store).is_some());
}
#[test]
fn saved_percussion_edits_reopen_with_undo_redo_and_unchanged_source() {
    let (_root, store) = project(false, false);
    let source = fs::read(store.root.join("source/notes.chart")).unwrap();
    let first = super::tests::fixture();
    apply(
        &store,
        Action::SetVocalPercussion {
            document: first.clone(),
        },
    )
    .unwrap();
    let mut second = first.clone();
    second.tracks.get_mut(&Part::Lead).unwrap().hits[0].displayed = false;
    apply(
        &store,
        Action::SetVocalPercussion {
            document: second.clone(),
        },
    )
    .unwrap();
    let reopened = Store::new(&store.root);
    assert_eq!(authored(&reopened), Some(second.clone()));
    apply(&reopened, Action::Undo).unwrap();
    assert_eq!(authored(&reopened), Some(first));
    apply(&reopened, Action::Redo).unwrap();
    assert_eq!(authored(&reopened), Some(second));
    assert_eq!(
        fs::read(store.root.join("source/notes.chart")).unwrap(),
        source
    );
}
#[test]
fn conflicting_singing_and_unsupported_original_percussion_refuse_without_saving() {
    let (_root, store) = project(false, false);
    apply(
        &store,
        Action::SetVocalPercussion {
            document: super::tests::fixture(),
        },
    )
    .unwrap();
    let before = fs::read(store.root.join("project.json")).unwrap();
    let lead = crate::vocals::VocalChart {
        version: 1,
        resolution: 480,
        source: "Conflicting lead".into(),
        phrases: vec![VocalPhrase {
            start: 0,
            end: 960,
            player: 1,
        }],
        notes: vec![crate::vocals::VocalNote {
            start: 240,
            end: 480,
            tone: crate::vocals::VocalTone::Pitched { midi: 60 },
            lyric: crate::vocals::Syllable::Missing,
            midi: None,
        }],
    };
    assert!(apply(&store, Action::SetVocals { document: lead }).is_err());
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    let (_other, unsupported) = project(true, true);
    let before = fs::read(unsupported.root.join("project.json")).unwrap();
    let mut later = super::tests::fixture();
    for track in later.tracks.values_mut() {
        for phrase in &mut track.phrases {
            phrase.range.start += 1920;
            phrase.range.end += 1920;
        }
        for hit in &mut track.hits {
            hit.start += 1920;
            hit.end += 1920;
        }
    }
    assert!(apply(&unsupported, Action::SetVocalPercussion { document: later }).is_err());
    assert_eq!(
        fs::read(unsupported.root.join("project.json")).unwrap(),
        before
    );
}
#[test]
fn edited_percussion_refuses_playable_export_until_the_codec_is_connected_to_that_workflow() {
    let (root, store) = project(false, false);
    apply(
        &store,
        Action::SetVocalPercussion {
            document: super::tests::fixture(),
        },
    )
    .unwrap();
    let before = fs::read(store.root.join("project.json")).unwrap();
    let output = root.path().join("must-not-publish");
    assert!(
        apply(
            &store,
            Action::Export {
                output: output.clone()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("percussion edits")
    );
    assert!(!output.exists());
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
}
#[test]
fn portable_percussion_project_retains_content_addressed_history() {
    fn collect(root: &Path, path: &Path, files: &mut crate::project_portability::Files) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let (root, store) = project(false, false);
    let first = super::tests::fixture();
    apply(
        &store,
        Action::SetVocalPercussion {
            document: first.clone(),
        },
    )
    .unwrap();
    let mut second = first.clone();
    second.tracks.get_mut(&Part::Harm1).unwrap().hits[0].start += 1;
    apply(
        &store,
        Action::SetVocalPercussion {
            document: second.clone(),
        },
    )
    .unwrap();
    let mut files = crate::project_portability::Files::new();
    collect(&store.root, &store.root, &mut files);
    let destination = tempfile::tempdir().unwrap();
    crate::project_portability::relocate(&mut files, destination.path()).unwrap();
    for (name, bytes) in files {
        let path = destination.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    drop(root);
    let moved = Store::new(destination.path());
    assert_eq!(authored(&moved), Some(second.clone()));
    apply(&moved, Action::Undo).unwrap();
    assert_eq!(authored(&moved), Some(first));
    apply(&moved, Action::Redo).unwrap();
    assert_eq!(authored(&moved), Some(second));
}
