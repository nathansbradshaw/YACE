use super::*;
use crate::editor::{Action, Actor, ActorKind, Request, Store};
use std::{fs, path::Path};
const CHART: &str = "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[ExpertDrums]\n{\n240 = N 1 0\n240 = E \"[mix 3 drums0d]\"\n480 = E \"[mix 3 drums0]\"\n}\n[HardDrums]\n{\n240 = N 2 0\n120 = E \"mix_2_drums0d\"\n600 = E \"mix_2_drums0\"\n}\n[ExpertSingle]\n{\n0 = N 0 0\n}\n";
fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "Disco regression".into(),
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
        let (bytes, _) =
            crate::game_export::song(&crate::chart::Chart::parse(CHART.into()).unwrap(), None)
                .unwrap();
        fs::write(song.join("notes.mid"), bytes).unwrap();
    } else {
        fs::write(song.join("notes.chart"), CHART).unwrap();
    }
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Disco regression\nsong_length = 12000\npro_drums = true\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    (root, store)
}
fn current(store: &Store, track: &str) -> Vec<Event> {
    let doc = store.load().unwrap();
    view(&doc, &store.source_drum_mix(&doc).unwrap(), track).unwrap()
}
fn edit(store: &Store, previous: Option<Range>, start: u64, end: u64) -> anyhow::Result<()> {
    apply(
        store,
        Action::EditDiscoRange {
            track: "ExpertDrums".into(),
            edit: Edit::Put {
                previous,
                start,
                end,
                configuration: 0,
            },
        },
    )
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
                format: crate::export_options::Format::Midi,
                tracks,
                include_audio: false,
                ..Default::default()
            },
        )
        .unwrap();
    fs::read(path.join("notes.mid")).unwrap()
}
#[test]
fn drum_mix_project_history_stale_bounds_export_reopen_clear_and_other_tiers() {
    for midi in [false, true] {
        let (root, store) = project(midi);
        let inherited = current(&store, "ExpertDrums");
        let hard = current(&store, "HardDrums");
        let previous = ranges(&inherited, 3).unwrap()[0].clone();
        edit(&store, Some(previous.clone()), 720, 960).unwrap();
        let moved = current(&store, "ExpertDrums");
        let saved = fs::read(store.root.join("project.json")).unwrap();
        assert!(edit(&store, Some(previous), 1200, 1440).is_err());
        assert!(edit(&store, None, 24000, 25000).is_err());
        assert_eq!(fs::read(store.root.join("project.json")).unwrap(), saved);
        apply(&store, Action::Undo).unwrap();
        assert_eq!(current(&store, "ExpertDrums"), inherited);
        apply(&store, Action::Redo).unwrap();
        assert_eq!(current(&Store::new(&store.root), "ExpertDrums"), moved);
        assert_eq!(current(&store, "HardDrums"), hard);
        let output = root.path().join("export");
        let bytes = export(&store, &output, None);
        let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
        assert_eq!(current(&reopened, "ExpertDrums"), moved);
        assert_eq!(export(&reopened, &root.path().join("again"), None), bytes);
        apply(
            &reopened,
            Action::EditDiscoRange {
                track: "ExpertDrums".into(),
                edit: Edit::Remove {
                    range: ranges(&moved, 3).unwrap()[0].clone(),
                },
            },
        )
        .unwrap();
        let clear = root.path().join("clear");
        export(&reopened, &clear, None);
        let cleared = Store::create(&clear, None, &root.path().join("cleared")).unwrap();
        assert!(
            ranges(&current(&cleared, "ExpertDrums"), 3)
                .unwrap()
                .is_empty()
        );
        assert_eq!(current(&cleared, "HardDrums"), hard);
    }
}
#[test]
fn drum_mix_project_scoped_export_does_not_revive_omitted_tier() {
    let (root, store) = project(true);
    edit(&store, None, 1920, 2400).unwrap();
    let output = root.path().join("partial");
    let bytes = export(
        &store,
        &output,
        Some(["HardDrums".into(), "ExpertSingle".into()].into()),
    );
    assert!(
        import_midi(&bytes)
            .unwrap()
            .events
            .iter()
            .all(|e| e.mix.difficulty == 2)
    );
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    apply(
        &reopened,
        Action::CreateTrack {
            track: "ExpertDrums".into(),
        },
    )
    .unwrap();
    edit(&reopened, None, 2880, 3360).unwrap();
    let later = export(&reopened, &root.path().join("later"), None);
    let events = import_midi(&later).unwrap().events;
    let expert = ranges(&events, 3).unwrap();
    assert_eq!(expert.len(), 1);
    assert_eq!((expert[0].start(), expert[0].end()), (2880, 3360));
    assert_eq!(ranges(&events, 2).unwrap().len(), 1);
}

#[test]
fn drum_mix_proposal_binding_and_preview_boundaries() {
    for midi in [false, true] {
        let (_root, store) = project(midi);
        let before = store.load().unwrap();
        edit(&store, None, 1920, 2400).unwrap();
        let after = store.load().unwrap();
        let proposal = |a, b| {
            crate::collaboration::Proposal::between(
                a,
                b,
                "disco-proposal".into(),
                "Drum notes".into(),
                "Retain mix context".into(),
                actor(),
            )
        };
        assert!(proposal(&before, &after).is_err());
        let mut before_notes = before.clone();
        before_notes.content.tracks.get_mut("ExpertDrums").unwrap()[0]
            .note
            .tick += 1;
        let mut after_notes = after.clone();
        after_notes.content.tracks.get_mut("ExpertDrums").unwrap()[0]
            .note
            .tick += 1;
        let initial = proposal(&before, &before_notes).unwrap();
        let edited = proposal(&after, &after_notes).unwrap();
        assert_ne!(initial.star_power_sha256, edited.star_power_sha256);
    }
    let chart = crate::chart::Chart::parse(CHART.into()).unwrap();
    let events = import_chart(&chart, "ExpertDrums").unwrap();
    assert_eq!(preview_lane(&events, 3, 239, 1, false), (1, false));
    assert_eq!(preview_lane(&events, 3, 240, 1, false), (2, true));
    assert_eq!(preview_lane(&events, 3, 479, 2, true), (1, false));
    assert_eq!(preview_lane(&events, 3, 479, 2, false), (1, false));
    assert_eq!(preview_lane(&events, 3, 480, 1, false), (1, false));
    assert_eq!(preview_lane(&events, 2, 240, 1, false), (1, false));
    assert_eq!(preview_lane(&events, 3, 240, 0, false), (0, false));
}

#[test]
fn drum_mix_marker_only_tier_is_available_preserved_and_explicitly_omittable() {
    let (root, base) = project(true);
    let source = base.root.join("source");
    let bytes = fs::read(root.path().join("song/notes.mid")).unwrap();
    let before = import_midi(&bytes).unwrap();
    let mut after = before.clone();
    after.events.push(Event::new(
        1440,
        Mix {
            difficulty: 1,
            configuration: 0,
            setting: Setting::DiscoNoFlip,
        },
    ));
    after.events.sort_by_key(|e| (e.tick, e.mix.difficulty));
    let midi = rewrite_midi(&bytes, &hash(&bytes), &before, &after).unwrap();
    let song = root.path().join("marker-song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.mid"), midi).unwrap();
    fs::copy(source.join("song.ini"), song.join("song.ini")).unwrap();
    let store = Store::create(&song, None, &root.path().join("marker-project")).unwrap();
    let doc = store.load().unwrap();
    assert!(!doc.content.tracks.contains_key("MediumDrums"));
    assert!(
        crate::export_options::available(&store, &doc)
            .unwrap()
            .contains("MediumDrums")
    );
    edit(&store, None, 1920, 2400).unwrap();
    let kept = export(&store, &root.path().join("marker-kept"), None);
    assert!(
        import_midi(&kept)
            .unwrap()
            .events
            .iter()
            .any(|e| e.mix.difficulty == 1 && e.mix.setting == Setting::DiscoNoFlip)
    );
    let omitted = root.path().join("marker-omitted");
    export(&store, &omitted, Some(["ExpertDrums".into()].into()));
    let reopened = Store::create(&omitted, None, &root.path().join("marker-reopened")).unwrap();
    assert!(
        !crate::export_options::available(&reopened, &reopened.load().unwrap())
            .unwrap()
            .contains("MediumDrums")
    );
    assert!(
        import_midi(&export(&reopened, &root.path().join("marker-later"), None))
            .unwrap()
            .events
            .iter()
            .all(|e| e.mix.difficulty == 3)
    );
}
