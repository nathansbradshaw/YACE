use super::*;
use crate::editor::{Action, Actor, ActorKind, Request, Store};
use std::{fs, path::Path};
fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "BRE regression".into(),
    }
}
fn action(store: &Store, action: Action) -> anyhow::Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: actor(),
        action,
    })?;
    Ok(())
}
fn edit(store: &Store, edit: Edit) -> anyhow::Result<()> {
    action(store, Action::EditBre { edit })
}
fn project(midi: bool) -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let song = root.path().join("song");
    fs::create_dir(&song).unwrap();
    if midi {
        fs::write(song.join("notes.mid"), super::tests::fixture()).unwrap();
    } else {
        fs::write(song.join("notes.chart"),"[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[ExpertSingle]\n{\n0 = N 0 120\n}\n[ExpertDrums]\n{\n1920 = S 64 480\n1920 = N 1 0\n}\n").unwrap();
    }
    fs::write(
        song.join("song.ini"),
        "[song]\nname = BRE regression\nsong_length = 12000\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    (root, store)
}
fn current(store: &Store) -> State {
    let doc = store.load().unwrap();
    super::current(&doc, &store.source_bre(&doc).unwrap()).unwrap()
}
fn export(
    store: &Store,
    path: &Path,
    tracks: Option<std::collections::BTreeSet<String>>,
) -> Vec<u8> {
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
#[test]
fn bre_project_midi_history_stale_refusal_export_reopen_and_explicit_clear() {
    let (root, store) = project(true);
    assert_eq!(
        export(&store, &root.path().join("untouched"), None),
        super::tests::fixture()
    );
    let inherited = current(&store);
    let previous = inherited.owners["PART GUITAR"][0].clone();
    edit(
        &store,
        Edit::Put {
            owner: "PART GUITAR".into(),
            previous: Some(previous.clone()),
            range: Range {
                start: 2880,
                end: 3360,
            },
        },
    )
    .unwrap();
    let moved = current(&store);
    let saved = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        edit(
            &store,
            Edit::Remove {
                owner: "PART GUITAR".into(),
                phrase: previous
            }
        )
        .is_err()
    );
    assert!(
        edit(
            &store,
            Edit::SetCoda {
                previous: None,
                tick: Some(0)
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), saved);
    action(&store, Action::Undo).unwrap();
    assert_eq!(current(&store), inherited);
    action(&store, Action::Redo).unwrap();
    assert_eq!(current(&store), moved);
    let output = root.path().join("export");
    let bytes = export(&store, &output, None);
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(current(&reopened).owners, moved.owners);
    assert_eq!(export(&reopened, &root.path().join("again"), None), bytes);
    edit(
        &reopened,
        Edit::Remove {
            owner: "PART GUITAR".into(),
            phrase: current(&reopened).owners["PART GUITAR"][0].clone(),
        },
    )
    .unwrap();
    let output = root.path().join("clear");
    export(&reopened, &output, None);
    let cleared = Store::create(&output, None, &root.path().join("cleared")).unwrap();
    assert!(current(&cleared).owners.is_empty());
}
#[test]
fn bre_chart_origin_forces_representable_midi_and_allows_drums_chart_context() {
    let (root, store) = project(false);
    edit(
        &store,
        Edit::SetCoda {
            previous: None,
            tick: Some(1920),
        },
    )
    .unwrap();
    edit(
        &store,
        Edit::Put {
            owner: "PART GUITAR".into(),
            previous: None,
            range: Range {
                start: 1920,
                end: 2400,
            },
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    let opts = crate::export_options::Options {
        format: crate::export_options::Format::Chart,
        include_audio: false,
        ..Default::default()
    };
    assert!(opts.selected(&store, &doc).is_err());
    assert!(
        crate::export_options::Options::default()
            .midi(&store, &doc)
            .unwrap()
    );
    let output = root.path().join("export");
    let bytes = export(&store, &output, None);
    let parsed = crate::song_files::game_midi::import_bre(&bytes).unwrap();
    assert_eq!(parsed.coda.unwrap().tick, 1920);
    assert_eq!(parsed.owners["PART GUITAR"].len(), 1);
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(export(&reopened, &root.path().join("again"), None), bytes);
    let chart = root.path().join("drums-chart");
    store
        .export_with_options(
            &doc,
            &chart,
            &actor(),
            &crate::export_options::Options {
                tracks: Some(["ExpertDrums".into()].into()),
                ..opts
            },
        )
        .unwrap();
    let text = fs::read_to_string(chart.join("notes.chart")).unwrap();
    assert!(text.contains("[coda]"));
    assert!(text.contains("1920 = S 64 480"));
}
#[test]
fn bre_omitted_owner_can_be_reauthored_without_reviving_source_lanes() {
    let (root, store) = project(true);
    let output = root.path().join("omit");
    export(&store, &output, Some(["ExpertDrums".into()].into()));
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert!(current(&reopened).owners.is_empty());
    action(
        &reopened,
        Action::CreateTrack {
            track: "ExpertSingle".into(),
        },
    )
    .unwrap();
    action(
        &reopened,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 3840,
            lane: 0,
            sustain: 120,
            cymbal: false,
        },
    )
    .unwrap();
    edit(
        &reopened,
        Edit::Put {
            owner: "PART GUITAR".into(),
            previous: None,
            range: Range {
                start: 2880,
                end: 3360,
            },
        },
    )
    .unwrap();
    let bytes = export(&reopened, &root.path().join("reauthored"), None);
    let state = crate::song_files::game_midi::import_bre(&bytes).unwrap();
    assert_eq!(state.owners["PART GUITAR"].len(), 1);
    assert_eq!(
        state.owners["PART GUITAR"][0].range,
        Range {
            start: 2880,
            end: 3360
        }
    );
}

#[test]
fn bre_drum_commands_use_existing_activation_authority_and_keep_finishing_notes() {
    for midi in [false, true] {
        let (root, chart_store) = project(false);
        let store = if midi {
            let song = root.path().join("drum-midi-song");
            fs::create_dir(&song).unwrap();
            let chart = crate::chart::Chart::parse(
                fs::read_to_string(root.path().join("song/notes.chart")).unwrap(),
            )
            .unwrap();
            fs::write(
                song.join("notes.mid"),
                crate::game_export::song(&chart, None).unwrap().0,
            )
            .unwrap();
            fs::copy(root.path().join("song/song.ini"), song.join("song.ini")).unwrap();
            Store::create(&song, None, &root.path().join("drum-midi-project")).unwrap()
        } else {
            chart_store
        };
        edit(
            &store,
            Edit::SetCoda {
                previous: None,
                tick: Some(1920),
            },
        )
        .unwrap();
        let previous = crate::drum_ranges::view(
            &store.load().unwrap(),
            &store.source_drum_ranges(&store.load().unwrap()).unwrap(),
            "ExpertDrums",
        )
        .unwrap()[0]
            .clone();
        let saved = fs::read(store.root.join("project.json")).unwrap();
        assert!(
            edit(
                &store,
                Edit::DrumPut {
                    track: "ExpertDrums".into(),
                    previous: Some(previous.clone()),
                    range: Range { start: 0, end: 480 }
                }
            )
            .is_err()
        );
        assert_eq!(saved, fs::read(store.root.join("project.json")).unwrap());
        edit(
            &store,
            Edit::DrumPut {
                track: "ExpertDrums".into(),
                previous: Some(previous),
                range: Range {
                    start: 2880,
                    end: 3360,
                },
            },
        )
        .unwrap();
        let output = root.path().join("drum-export");
        store
            .export_with_options(
                &store.load().unwrap(),
                &output,
                &actor(),
                &crate::export_options::Options {
                    include_audio: false,
                    ..Default::default()
                },
            )
            .unwrap();
        let reopened = Store::create(&output, None, &root.path().join("drum-reopened")).unwrap();
        assert_eq!(current(&reopened).coda.unwrap().tick, 1920);
        let doc = reopened.load().unwrap();
        let ranges = crate::drum_ranges::view(
            &doc,
            &reopened.source_drum_ranges(&doc).unwrap(),
            "ExpertDrums",
        )
        .unwrap();
        assert_eq!((ranges[0].start, ranges[0].end), (2880, 3360));
        assert!(
            doc.content.tracks["ExpertDrums"]
                .iter()
                .any(|n| n.note.tick == 1920 && n.note.lane == 1)
        );
    }
}

#[test]
fn bre_source_context_binds_proposals_when_normalized_charts_match() {
    let (root, store) = project(true);
    let bytes = super::tests::fixture();
    let original = crate::song_files::game_midi::import_bre(&bytes).unwrap();
    let changed = edited(
        &original,
        &Edit::SetCoda {
            previous: original.coda.clone(),
            tick: Some(480),
        },
    )
    .unwrap();
    let midi = crate::song_files::game_midi::rewrite_bre(
        &bytes,
        &crate::drum_mix::hash(&bytes),
        &original,
        &changed,
    )
    .unwrap();
    let song = root.path().join("other-coda-song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.mid"), midi).unwrap();
    fs::copy(root.path().join("song/song.ini"), song.join("song.ini")).unwrap();
    let other = Store::create(&song, None, &root.path().join("other-coda-project")).unwrap();
    let a = store.load().unwrap();
    let b = other.load().unwrap();
    assert_eq!(a.source_chart_sha256, b.source_chart_sha256);
    let propose = |base: &crate::editor::Project, edited: &crate::editor::Project| {
        crate::collaboration::Proposal::between(
            base,
            edited,
            "bre-context-proposal".into(),
            "Note changes".into(),
            "Preserve source context".into(),
            actor(),
        )
    };
    let mut a_notes = a.clone();
    a_notes.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .tick += 1;
    let mut b_notes = b.clone();
    b_notes.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .tick += 1;
    assert!(propose(&a, &b_notes).is_err());
    assert_ne!(
        propose(&a, &a_notes).unwrap().star_power_sha256,
        propose(&b, &b_notes).unwrap().star_power_sha256
    );
}
