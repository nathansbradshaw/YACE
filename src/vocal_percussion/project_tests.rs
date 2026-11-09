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
fn singing(start: u64) -> crate::vocals::VocalChart {
    crate::vocals::VocalChart {
        version: 1,
        resolution: 480,
        source: "Independent singer preflight".into(),
        phrases: vec![VocalPhrase {
            start,
            end: start + 960,
            player: 1,
        }],
        notes: vec![crate::vocals::VocalNote {
            start: start + 120,
            end: start + 240,
            tone: crate::vocals::VocalTone::Pitched { midi: 60 },
            lyric: crate::vocals::Syllable::Missing,
            midi: None,
        }],
    }
}
#[test]
fn inherited_dedicated_percussion_refuses_conflicting_singer_drafts_before_saving() {
    let (_root, store) = project(true, false);
    let before = fs::read(store.root.join("project.json")).unwrap();
    let error = apply(
        &store,
        Action::SetVocals {
            document: singing(0),
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("dedicated phrase"));
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    assert!(store.load().unwrap().content.vocals.is_none());
    assert!(authored(&store).is_some());
}
#[test]
fn singer_preflight_honors_explicit_clear_and_current_authored_percussion_with_history() {
    let (_root, store) = project(true, false);
    apply(&store, Action::ClearVocalPercussion).unwrap();
    apply(
        &store,
        Action::SetVocals {
            document: singing(0),
        },
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().content.vocal_percussion,
        Some(State::Cleared)
    );
    apply(&store, Action::Undo).unwrap();
    apply(&store, Action::Undo).unwrap();
    assert!(authored(&store).is_some());
    assert!(store.load().unwrap().content.vocals.is_none());
    apply(&store, Action::Redo).unwrap();
    apply(&store, Action::Redo).unwrap();
    assert!(authored(&store).is_none());
    assert_eq!(store.load().unwrap().content.vocals, Some(singing(0)));
    let (_other, moved) = project(true, false);
    let mut document = authored(&moved).unwrap();
    for track in document.tracks.values_mut() {
        for phrase in &mut track.phrases {
            phrase.range.start += 1920;
            phrase.range.end += 1920;
        }
        for hit in &mut track.hits {
            hit.start += 1920;
            hit.end += 1920;
        }
    }
    apply(&moved, Action::SetVocalPercussion { document }).unwrap();
    apply(
        &moved,
        Action::SetVocals {
            document: singing(0),
        },
    )
    .unwrap();
    let before = fs::read(moved.root.join("project.json")).unwrap();
    assert!(
        apply(
            &moved,
            Action::SetVocals {
                document: singing(1920)
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(moved.root.join("project.json")).unwrap(), before);
}
#[test]
fn opaque_legacy_percussion_stays_retained_but_source_integrity_errors_are_not_ignored() {
    let (_root, store) = project(true, true);
    assert!(crate::song_files::game_midi::percussion_view(&store.root.join("source")).is_err());
    apply(
        &store,
        Action::SetVocals {
            document: singing(0),
        },
    )
    .unwrap();
    let (_other, corrupt) = project(true, false);
    // This fresh disposable fixture has one retained original (archive zero).
    let archived = corrupt
        .root
        .join("source/.yace/originals/game-midi/notes.mid");
    let mut bytes = fs::read(&archived).unwrap();
    bytes.push(0);
    fs::write(archived, bytes).unwrap();
    let before = fs::read(corrupt.root.join("project.json")).unwrap();
    let error = apply(
        &corrupt,
        Action::SetVocals {
            document: singing(0),
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("changed"));
    assert_eq!(fs::read(corrupt.root.join("project.json")).unwrap(), before);
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
fn selected_hit_commands_are_atomic_durable_and_restore_legacy_inheritance_with_undo() {
    let (_root, store) = project(true, false);
    let original = authored(&store).unwrap();
    let expected = original.tracks[&Part::Lead].hits[0].clone();
    let mut hit = expected.clone();
    hit.displayed = !hit.displayed;
    let edit = Edit::PutHit {
        part: Part::Lead,
        index: Some(0),
        expected: Some(expected),
        hit,
        phrase: None,
    };
    apply(&store, Action::EditVocalPercussion { edit: edit.clone() }).unwrap();
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(apply(&store, Action::EditVocalPercussion { edit }).is_err());
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    let reopened = Store::new(&store.root);
    let changed = authored(&reopened).unwrap();
    assert_ne!(changed, original);
    apply(&reopened, Action::Undo).unwrap();
    assert!(reopened.load().unwrap().content.vocal_percussion.is_none());
    assert_eq!(authored(&reopened), Some(original));
    apply(&reopened, Action::Redo).unwrap();
    assert_eq!(authored(&reopened), Some(changed));
}
#[test]
fn removing_final_source_hits_records_a_clear_instead_of_restoring_the_archive() {
    let (_root, store) = project(true, false);
    for part in [Part::Lead, Part::Harm1] {
        let current = authored(&store).unwrap();
        apply(
            &store,
            Action::EditVocalPercussion {
                edit: Edit::RemoveHit {
                    part,
                    index: 0,
                    expected: current.tracks[&part].hits[0].clone(),
                },
            },
        )
        .unwrap();
    }
    let reopened = Store::new(&store.root);
    assert_eq!(
        reopened.load().unwrap().content.vocal_percussion,
        Some(State::Cleared)
    );
    assert!(authored(&reopened).is_none());
    apply(&reopened, Action::Undo).unwrap();
    let last = authored(&reopened).unwrap();
    assert_eq!(last.tracks.len(), 1);
    assert!(last.tracks.contains_key(&Part::Harm1));
    apply(&reopened, Action::Redo).unwrap();
    assert!(authored(&reopened).is_none());
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
fn authored_percussion_exports_reopens_and_repeats_without_changing_the_editing_project() {
    let (root, store) = project(false, false);
    apply(
        &store,
        Action::SetVocalPercussion {
            document: super::tests::fixture(),
        },
    )
    .unwrap();
    let before = fs::read(store.root.join("project.json")).unwrap();
    let output = root.path().join("playable");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    let bytes = fs::read(output.join("notes.mid")).unwrap();
    let expected = import_midi(&bytes).unwrap().unwrap();
    assert_eq!(expected.tracks.len(), 2);
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(authored(&reopened), Some(expected));
    assert!(reopened.load().unwrap().content.vocal_percussion.is_some());
    let repeat = root.path().join("repeat");
    apply(
        &reopened,
        Action::Export {
            output: repeat.clone(),
        },
    )
    .unwrap();
    assert_eq!(fs::read(repeat.join("notes.mid")).unwrap(), bytes);
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
}
fn export_to(store: &Store, output: &Path, tracks: Option<&[&str]>) -> Vec<u8> {
    store
        .export_with_options(
            &store.load().unwrap(),
            output,
            &Actor {
                kind: ActorKind::Human,
                name: "Percussion export regression".into(),
            },
            &crate::export_options::Options {
                tracks: tracks.map(|tracks| tracks.iter().map(|name| (*name).to_owned()).collect()),
                include_audio: false,
                ..Default::default()
            },
        )
        .unwrap();
    fs::read(output.join("notes.mid")).unwrap()
}
fn first_harmony() -> crate::vocal_harmonies::HarmonyChart {
    let lead = singing(960);
    crate::vocal_harmonies::HarmonyChart {
        version: 1,
        resolution: 480,
        source: "Independent first harmony".into(),
        phrases: lead.phrases,
        lyric_phrases: vec![],
        voices: [(
            crate::vocal_harmonies::Part::Harm1,
            crate::vocal_harmonies::Voice {
                source: "Manual HARM1".into(),
                notes: lead.notes,
            },
        )]
        .into(),
    }
}
fn control_project() -> (tempfile::TempDir, Store) {
    use midly::{
        MetaMessage as M, MidiMessage as N, TrackEventKind as K,
        num::{u4, u7, u28},
    };
    let (root, _first) = project(true, false);
    let path = root.path().join("song/notes.mid");
    let bytes = fs::read(&path).unwrap();
    let mut midi = midly::Smf::parse(&bytes).unwrap();
    for track in &mut midi.tracks {
        if !track.iter().any(|event| matches!(event.kind, K::Meta(M::TrackName(name)) if std::str::from_utf8(name).ok().and_then(Part::from_midi).is_some())) { continue; }
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
                20,
                K::Midi {
                    channel: u4::new(4),
                    message: N::Controller {
                        controller: u7::new(11),
                        value: u7::new(75),
                    },
                },
            ),
            (22, K::Meta(M::Text(b"[mood_play]"))),
            (
                0,
                K::Midi {
                    channel: u4::new(5),
                    message: N::NoteOn {
                        key: u7::new(116),
                        vel: u7::new(81),
                    },
                },
            ),
            (
                960,
                K::Midi {
                    channel: u4::new(5),
                    message: N::NoteOff {
                        key: u7::new(116),
                        vel: u7::new(14),
                    },
                },
            ),
        ]);
        // Lead becomes metadata-only after PC stripping; HARM1 retains SP
        // notes. Exercise both omission branches with the same source song.
        if track.iter().any(|event| matches!(event.kind, K::Meta(M::TrackName(name)) if std::str::from_utf8(name).ok().and_then(Part::from_midi) == Some(Part::Lead))) {
            events.retain(|(_, kind)| !matches!(kind, K::Midi { message: N::NoteOn { key, .. } | N::NoteOff { key, .. }, .. } if key.as_int() == 116));
        }
        events.sort_by_key(|(tick, kind)| (*tick, matches!(kind, K::Meta(M::EndOfTrack))));
        let mut previous = 0;
        *track = events
            .into_iter()
            .map(|(tick, kind)| {
                let delta = u28::new((tick - previous) as u32);
                previous = tick;
                midly::TrackEvent { delta, kind }
            })
            .collect();
    }
    let mut output = Vec::new();
    midi.write_std(&mut output).unwrap();
    fs::write(path, output).unwrap();
    let store = Store::create(
        &root.path().join("song"),
        None,
        &root.path().join("controls"),
    )
    .unwrap();
    (root, store)
}
fn controls(bytes: &[u8], part: Part) -> Vec<String> {
    use midly::{MetaMessage as M, MidiMessage as N, TrackEventKind as K};
    let midi = midly::Smf::parse(bytes).unwrap();
    let Some(track)=midi.tracks.iter().find(|track| track.iter().any(|event| matches!(event.kind,K::Meta(M::TrackName(name)) if std::str::from_utf8(name).ok().and_then(Part::from_midi)==Some(part)))) else { return vec![]; };
    let mut tick = 0;
    track
        .iter()
        .filter_map(|event| {
            tick += u64::from(event.delta.as_int());
            let selected = match event.kind {
                K::Midi {
                    message: N::Controller { .. },
                    ..
                }
                | K::Meta(M::Text(_)) => true,
                K::Midi {
                    message: N::NoteOn { key, .. } | N::NoteOff { key, .. },
                    ..
                } => key.as_int() == 116,
                _ => false,
            };
            selected.then(|| format!("{tick}:{:?}", event.kind))
        })
        .collect()
}
#[test]
fn logical_omissions_never_revive_original_controls_after_new_singers_and_repeated_exports() {
    let (root, store) = control_project();
    let original = fs::read(
        store
            .root
            .join("source/.yace/originals/game-midi/notes.mid"),
    )
    .unwrap();
    let retained = controls(&original, Part::Harm1);
    assert!(!retained.is_empty());
    assert!(!controls(&original, Part::Lead).is_empty());
    apply(
        &store,
        Action::SetVocalPercussion {
            document: authored(&store).unwrap(),
        },
    )
    .unwrap();
    let output = root.path().join("harmonies");
    let bytes = export_to(&store, &output, Some(&["ExpertSingle", "Harmonies"]));
    assert!(controls(&bytes, Part::Lead).is_empty());
    assert_eq!(controls(&bytes, Part::Harm1), retained);
    let next = Store::create(&output, None, &root.path().join("next")).unwrap();
    apply(
        &next,
        Action::SetVocals {
            document: singing(0),
        },
    )
    .unwrap();
    let later = root.path().join("later");
    let bytes = export_to(&next, &later, None);
    assert!(controls(&bytes, Part::Lead).is_empty());
    assert_eq!(controls(&bytes, Part::Harm1), retained);
    let again = Store::create(&later, None, &root.path().join("again")).unwrap();
    let repeated = export_to(&again, &root.path().join("repeated"), None);
    assert!(controls(&repeated, Part::Lead).is_empty());
    assert_eq!(controls(&repeated, Part::Harm1), retained);
    let omitted = root.path().join("omit-harmony");
    let bytes = export_to(&again, &omitted, Some(&["ExpertSingle", "Vocals"]));
    assert!(controls(&bytes, Part::Harm1).is_empty());
    let fresh = Store::create(&omitted, None, &root.path().join("fresh")).unwrap();
    apply(
        &fresh,
        Action::SetHarmonies {
            document: first_harmony(),
        },
    )
    .unwrap();
    let new_harmony = root.path().join("new-harmony");
    let bytes = export_to(&fresh, &new_harmony, None);
    assert!(controls(&bytes, Part::Harm1).is_empty());
    assert!(import_midi(&bytes).unwrap().is_none());
    let last = Store::create(&new_harmony, None, &root.path().join("last")).unwrap();
    assert!(
        controls(
            &export_to(&last, &root.path().join("last-export"), None),
            Part::Harm1
        )
        .is_empty()
    );
    assert_eq!(
        fs::read(
            store
                .root
                .join("source/.yace/originals/game-midi/notes.mid")
        )
        .unwrap(),
        original
    );
}
#[test]
fn clearing_original_percussion_frees_its_phrase_for_singing_and_stays_cleared_after_export() {
    let (root, store) = project(true, false);
    let path = store
        .root
        .join("source/.yace/originals/game-midi/notes.mid");
    let original = fs::read(&path).unwrap();
    apply(&store, Action::ClearVocalPercussion).unwrap();
    apply(
        &store,
        Action::SetVocals {
            document: singing(0),
        },
    )
    .unwrap();
    let output = root.path().join("cleared");
    let bytes = export_to(&store, &output, None);
    assert!(import_midi(&bytes).unwrap().is_none());
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(
        reopened.load().unwrap().content.vocal_percussion,
        Some(State::Cleared)
    );
    assert!(authored(&reopened).is_none());
    assert_eq!(
        reopened.load().unwrap().content.vocals.unwrap().notes[0].start,
        120
    );
    assert!(
        import_midi(&export_to(&reopened, &root.path().join("repeat"), None))
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read(path).unwrap(), original);
}
#[test]
fn first_harmony_in_a_percussion_owner_survives_logical_selection_reopen_and_phrase_edits() {
    let (root, store) = project(true, false);
    let document = authored(&store).unwrap();
    apply(&store, Action::SetVocalPercussion { document }).unwrap();
    apply(
        &store,
        Action::SetHarmonies {
            document: first_harmony(),
        },
    )
    .unwrap();
    let available = crate::export_options::available(&store, &store.load().unwrap()).unwrap();
    assert!(available.contains("Harmonies") && available.contains("Vocals"));
    assert!(!available.iter().any(|name| name.starts_with("MIDI: ")));
    let output = root.path().join("harmonies");
    let bytes = export_to(&store, &output, Some(&["ExpertSingle", "Harmonies"]));
    let pc = import_midi(&bytes).unwrap().unwrap();
    assert_eq!(pc.tracks.len(), 1);
    assert!(pc.tracks.contains_key(&Part::Harm1));
    assert_eq!(
        crate::vocal_harmonies::import_midi(&bytes)
            .unwrap()
            .unwrap()
            .phrases,
        first_harmony().phrases
    );
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    let repeat = export_to(
        &reopened,
        &root.path().join("repeat"),
        Some(&["ExpertSingle", "Harmonies"]),
    );
    assert_eq!(repeat, bytes);
    apply(&reopened, Action::ClearVocalPercussion).unwrap();
    let mut singing = reopened.load().unwrap().content.harmonies.unwrap();
    singing.phrases[0].start = 0;
    apply(&reopened, Action::SetHarmonies { document: singing }).unwrap();
    let cleared = export_to(&reopened, &root.path().join("cleared"), None);
    assert!(import_midi(&cleared).unwrap().is_none());
    assert_eq!(
        crate::vocal_harmonies::import_midi(&cleared)
            .unwrap()
            .unwrap()
            .phrases[0]
            .start,
        0
    );
}
#[test]
fn first_singer_adopts_verified_percussion_ownership_in_the_same_undoable_command() {
    let (root, store) = project(true, false);
    let before = fs::read(store.root.join("project.json")).unwrap();
    let pc = authored(&store).unwrap();
    let mut conflicting = first_harmony();
    conflicting.phrases[0].start = 0;
    assert!(
        apply(
            &store,
            Action::SetHarmonies {
                document: conflicting
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    assert!(store.load().unwrap().content.vocal_percussion.is_none());
    apply(
        &store,
        Action::SetHarmonies {
            document: first_harmony(),
        },
    )
    .unwrap();
    assert_eq!(authored(&store), Some(pc));
    assert!(store.load().unwrap().content.vocal_percussion.is_some());
    apply(&store, Action::Undo).unwrap();
    assert!(store.load().unwrap().content.vocal_percussion.is_none());
    assert!(store.load().unwrap().content.harmonies.is_none());
    apply(&store, Action::Redo).unwrap();
    let bytes = export_to(&store, &root.path().join("first-singer"), None);
    assert!(
        crate::vocal_harmonies::import_midi(&bytes)
            .unwrap()
            .is_some()
    );
    assert!(import_midi(&bytes).unwrap().is_some());
}
#[test]
fn omitted_typed_percussion_does_not_return_when_new_singers_are_authored_later() {
    let (root, store) = project(true, false);
    apply(
        &store,
        Action::SetVocalPercussion {
            document: authored(&store).unwrap(),
        },
    )
    .unwrap();
    let output = root.path().join("instruments");
    assert!(
        import_midi(&export_to(&store, &output, Some(&["ExpertSingle"])))
            .unwrap()
            .is_none()
    );
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(
        reopened.load().unwrap().content.vocal_percussion,
        Some(State::Cleared)
    );
    apply(
        &reopened,
        Action::SetVocals {
            document: singing(0),
        },
    )
    .unwrap();
    apply(
        &reopened,
        Action::SetHarmonies {
            document: first_harmony(),
        },
    )
    .unwrap();
    let later = root.path().join("later");
    assert!(
        import_midi(&export_to(&reopened, &later, None))
            .unwrap()
            .is_none()
    );
    let again = Store::create(&later, None, &root.path().join("again")).unwrap();
    assert!(authored(&again).is_none());
}
#[test]
fn altered_working_percussion_state_refuses_reopen_without_replacing_the_source() {
    let (root, store) = project(false, false);
    apply(
        &store,
        Action::SetVocalPercussion {
            document: super::tests::fixture(),
        },
    )
    .unwrap();
    let output = root.path().join("export");
    let bytes = export_to(&store, &output, None);
    let sidecar = crate::song_layout::resolve(&output, "chart-engine-vocal-percussion.json");
    let mut data: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    data["state"]["document"]["tracks"]["LEAD"]["hits"][0]["displayed"] = false.into();
    fs::write(&sidecar, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(Store::create(&output, None, &root.path().join("must-not-open")).is_err());
    assert_eq!(fs::read(output.join("notes.mid")).unwrap(), bytes);
    assert!(!root.path().join("must-not-open").exists());
}
#[test]
fn percussion_only_selection_reopens_without_adding_playable_instrument_notes() {
    let (root, store) = project(false, false);
    apply(
        &store,
        Action::SetVocalPercussion {
            document: super::tests::fixture(),
        },
    )
    .unwrap();
    let output = root.path().join("percussion-only");
    let bytes = export_to(&store, &output, Some(&["Vocals"]));
    assert_eq!(import_midi(&bytes).unwrap().unwrap().tracks.len(), 1);
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert!(
        reopened
            .load()
            .unwrap()
            .content
            .tracks
            .values()
            .all(Vec::is_empty)
    );
    let midi = midly::Smf::parse(&bytes).unwrap();
    assert!(!midi.tracks.iter().any(|track| track.iter().any(|event| matches!(event.kind, midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(b"PART GUITAR")))) && track.iter().any(|event| matches!(event.kind,midly::TrackEventKind::Midi {message:midly::MidiMessage::NoteOn {vel,..},..} if vel.as_int()>0))));
    assert_eq!(
        export_to(&reopened, &root.path().join("repeat"), Some(&["Vocals"])),
        bytes
    );
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
    let output = root.path().join("bound-export");
    export_to(&store, &output, None);
    let store = Store::create(&output, None, &root.path().join("bound-project")).unwrap();
    let first = authored(&store).unwrap();
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
    assert_eq!(authored(&moved), Some(second.clone()));
    let export = tempfile::tempdir().unwrap();
    let bytes = export_to(&moved, &export.path().join("playable"), None);
    assert_eq!(import_midi(&bytes).unwrap(), Some(second));
}
