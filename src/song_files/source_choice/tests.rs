use super::*;
use crate::{
    chart::{Note, Tracks},
    editor::{Action, Actor, ActorKind, Request, Store},
    timing::TempoMap,
};
use std::collections::BTreeMap;

const CHART: &str = "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[ExpertSingle]\n{\n240 = N 0 120\n}\n";
fn song(root: &Path) -> PathBuf {
    let path = root.join("song");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("notes.chart"), CHART).unwrap();
    let timing = TempoMap::new(480, 0., vec![]).unwrap();
    let tracks: Tracks = BTreeMap::from([(
        "ExpertSingle".into(),
        vec![Note {
            tick: 480,
            lane: 3,
            sustain: 120,
        }],
    )]);
    fs::write(
        path.join("notes.mid"),
        crate::midi::new_midi(&timing, &tracks).unwrap(),
    )
    .unwrap();
    fs::write(
        path.join("song.ini"),
        "[song]\nname = Source choice\ncustom = retain\n",
    )
    .unwrap();
    let samples = vec![0.; 44100 * 3];
    models::write_audio(&path.join("song.wav"), &samples, &samples).unwrap();
    path
}
fn apply(store: &Store, action: Action) -> Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Source choice fixture".into(),
        },
        action,
    })?;
    Ok(())
}

#[test]
fn both_sources_require_a_choice_and_either_choice_preserves_both_originals_through_reopen() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    let midi = fs::read(input.join("notes.mid")).unwrap();
    assert!(Store::create(&input, None, &root.path().join("ambiguous")).is_err());
    assert!(!fs::exists(root.path().join("ambiguous")));
    for (source, tick, lane) in [(Source::Chart, 240, 0), (Source::Midi, 480, 3)] {
        let project = root.path().join(source.filename().replace('.', "-"));
        let store = Store::create_with_source(&input, None, &project, Some(source)).unwrap();
        let doc = store.load().unwrap();
        assert!(
            doc.content.tracks["ExpertSingle"]
                .iter()
                .any(|n| n.note.tick == tick && n.note.lane == lane)
        );
        let copied = store.root.join("source");
        assert_eq!(recorded_source(&copied).unwrap(), Some(source));
        let originals = copied.join(".yace/originals/chart-source");
        assert_eq!(
            fs::read_to_string(originals.join("notes.chart")).unwrap(),
            CHART
        );
        assert_eq!(fs::read(originals.join("notes.mid")).unwrap(), midi);
        let export = root.path().join(format!("export-{source:?}"));
        apply(
            &store,
            Action::Export {
                output: export.clone(),
            },
        )
        .unwrap();
        assert!(fs::is_file(export.join(source.filename())));
        assert!(!ambiguous(&export));
        let reopened = Store::create(
            &export,
            None,
            &root.path().join(format!("reopened-{source:?}")),
        )
        .unwrap();
        assert_eq!(
            recorded_source(&reopened.root.join("source")).unwrap(),
            Some(source)
        );
        assert!(
            reopened.load().unwrap().content.tracks["ExpertSingle"]
                .iter()
                .any(|n| n.note.tick == tick && n.note.lane == lane)
        );
    }
    assert_eq!(
        fs::read_to_string(input.join("notes.chart")).unwrap(),
        CHART
    );
    assert_eq!(fs::read(input.join("notes.mid")).unwrap(), midi);
    assert!(!fs::exists(input.join(".yace")));
}

#[test]
fn selecting_chart_can_retain_an_opaque_invalid_alternate_without_parsing_it() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    fs::write(input.join("notes.mid"), b"opaque alternate, not valid MIDI").unwrap();
    let store = Store::create_with_source(
        &input,
        None,
        &root.path().join("chart"),
        Some(Source::Chart),
    )
    .unwrap();
    assert_eq!(
        fs::read(
            store
                .root
                .join("source/.yace/originals/chart-source/notes.mid")
        )
        .unwrap(),
        b"opaque alternate, not valid MIDI"
    );
    let failed = root.path().join("midi");
    assert!(Store::create_with_source(&input, None, &failed, Some(Source::Midi)).is_err());
    assert!(!fs::exists(failed));
    fs::remove_file(input.join("notes.mid")).unwrap();
    assert!(
        Store::create_with_source(
            &input,
            None,
            &root.path().join("missing"),
            Some(Source::Midi)
        )
        .is_err()
    );
}

#[test]
fn chart_choice_can_later_export_as_midi_without_reusing_the_alternate_midi() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    let alternate = fs::read(input.join("notes.mid")).unwrap();
    let store = Store::create_with_source(
        &input,
        None,
        &root.path().join("project"),
        Some(Source::Chart),
    )
    .unwrap();
    apply(
        &store,
        Action::CreateTrack {
            track: "ExpertProKeys".into(),
        },
    )
    .unwrap();
    apply(
        &store,
        Action::AddNote {
            track: "ExpertProKeys".into(),
            tick: 720,
            lane: 60,
            sustain: 120,
            cymbal: false,
        },
    )
    .unwrap();
    let output = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    assert!(!fs::exists(output.join("notes.chart")));
    assert_ne!(fs::read(output.join("notes.mid")).unwrap(), alternate);
    assert_eq!(
        fs::read(output.join(".yace/originals/chart-source/notes.mid")).unwrap(),
        alternate
    );
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    let doc = reopened.load().unwrap();
    assert!(
        doc.content.tracks["ExpertSingle"]
            .iter()
            .any(|n| n.note.tick == 240 && n.note.lane == 0)
    );
    assert!(
        !doc.content.tracks["ExpertSingle"]
            .iter()
            .any(|n| n.note.tick == 480)
    );
    assert!(
        doc.content.tracks["ExpertProKeys"]
            .iter()
            .any(|n| n.note.tick == 720 && n.note.lane == 60)
    );
    assert_eq!(
        recorded_source(&reopened.root.join("source")).unwrap(),
        Some(Source::Chart)
    );
}

#[test]
fn damaged_alternate_refuses_export_and_reopen_without_losing_the_original_song() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    let store = Store::create_with_source(
        &input,
        None,
        &root.path().join("project"),
        Some(Source::Chart),
    )
    .unwrap();
    let output = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    fs::write(
        store
            .root
            .join("source/.yace/originals/chart-source/notes.mid"),
        b"damaged",
    )
    .unwrap();
    assert!(
        apply(
            &store,
            Action::Export {
                output: root.path().join("refused")
            }
        )
        .is_err()
    );
    assert!(!fs::exists(root.path().join("refused")));
    fs::write(
        output.join(".yace/originals/chart-source/notes.chart"),
        "damaged",
    )
    .unwrap();
    assert!(Store::create(&output, None, &root.path().join("refused-reopen")).is_err());
    assert_eq!(
        fs::read_to_string(input.join("notes.chart")).unwrap(),
        CHART
    );
}

#[test]
fn library_reuses_saved_selection_and_history_instead_of_switching_sources() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    let library = crate::chart_library::discover(&input).unwrap();
    let item = &library.entries[0];
    let path = library.open_with_source(item, Some(Source::Midi)).unwrap();
    let store = Store::new(&path);
    apply(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 960,
            lane: 4,
            sustain: 0,
            cymbal: false,
        },
    )
    .unwrap();
    let revision = store.load().unwrap().revision;
    assert_eq!(library.open(item).unwrap(), path);
    assert_eq!(
        library.open_with_source(item, Some(Source::Midi)).unwrap(),
        path
    );
    assert!(library.open_with_source(item, Some(Source::Chart)).is_err());
    assert_eq!(store.load().unwrap().revision, revision);
    assert!(
        store.load().unwrap().content.tracks["ExpertSingle"]
            .iter()
            .any(|n| n.note.tick == 960)
    );
}

#[test]
#[cfg(feature = "editor")]
fn native_song_zip_keeps_its_staged_sources_alive_for_the_same_selection_flow() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    let names = ["notes.chart", "notes.mid", "song.ini", "song.wav"];
    let files: Vec<_> = names
        .iter()
        .map(|name| (*name, fs::read(input.join(name)).unwrap()))
        .collect();
    let zip = root.path().join("song.zip");
    fs::write(
        &zip,
        crate::archive_codec::encode(files.iter().map(|(name, bytes)| (*name, bytes.as_slice())))
            .unwrap(),
    )
    .unwrap();
    let staged = crate::project_files::unpack_song(&zip).unwrap();
    assert!(ambiguous(staged.path()));
    let store = Store::create_with_source(
        staged.path(),
        None,
        &root.path().join("project"),
        Some(Source::Midi),
    )
    .unwrap();
    drop(staged);
    assert_eq!(
        recorded_source(&store.root.join("source")).unwrap(),
        Some(Source::Midi)
    );
    assert!(
        store.load().unwrap().content.tracks["ExpertSingle"]
            .iter()
            .any(|n| n.note.tick == 480 && n.note.lane == 3)
    );
}
