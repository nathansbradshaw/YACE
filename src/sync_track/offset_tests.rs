use super::*;
use crate::{
    editor::{Action, Actor, ActorKind, Request, Store},
    storage as fs,
};
use sha2::{Digest, Sha256};
use std::path::Path;

const CHART: &str = "\u{feff}[Song]\r\n{\r\n Resolution = 480\r\n Offset = 0.5\r\n Custom = retain\r\n}\r\n[SyncTrack]\r\n{\r\n // fine timing and an unpaired anchor\r\n 0 = B 120000\r\n 0 = TS 5 3\r\n 960 = A 1000000\r\n 1440 = B 150000\r\n}\r\n[Events]\r\n{\r\n 0 = E \"section Intro\"\r\n}\r\n[ExpertSingle]\r\n{\r\n 960 = N 0 240\r\n 960 = N 6 0\r\n 1920 = N 1 240\r\n 960 = S 2 960\r\n 960 = E solo\r\n 2160 = E soloend\r\n}\r\n[ExpertDoubleBass]\r\n{\r\n 960 = N 2 120\r\n}\r\n[Opaque]\r\n{\r\n 123 = X keep\r\n}\r\n";

fn fixture(root: &Path, audio: bool) -> Store {
    let song = root.join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"), CHART).unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\r\nname = Offset fixture\r\ndelay = 250\r\ncustom = keep\r\n",
    )
    .unwrap();
    if audio {
        crate::models::write_audio(
            &song.join("song.wav"),
            &vec![0.; 44100 * 4],
            &vec![0.; 44100 * 4],
        )
        .unwrap();
    }
    Store::create(&song, None, &root.join("project")).unwrap()
}
fn action(store: &Store, action: Action) -> Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Offset fixture".into(),
        },
        action,
    })?;
    Ok(())
}
fn set(store: &Store, previous: f64, seconds: f64) -> Result<()> {
    action(
        store,
        Action::EditSync {
            edit: Edit::SetOffset { previous, seconds },
        },
    )
}

#[test]
fn offset_edit_preserves_ticks_anchors_audio_and_undo_redo() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), true);
    let before = store.load().unwrap();
    let source = fs::read(store.root.join("source/notes.chart")).unwrap();
    let audio = crate::models::hash_file(&store.audio_path(&before).unwrap()).unwrap();
    action(&store, Action::Playback { playing: true }).unwrap();
    set(&store, 0.25, -0.125).unwrap();
    let after = store.load().unwrap();
    assert!(!after.transport.playing);
    assert_eq!(after.timing(&store).unwrap().seconds(960), 0.875);
    assert_eq!(
        after.timing(&store).unwrap().tempos,
        before.timing(&store).unwrap().tempos
    );
    assert_eq!(
        serde_json::to_value(&after.content.tracks).unwrap(),
        serde_json::to_value(&before.content.tracks).unwrap()
    );
    let source_chart = Chart::parse(String::from_utf8(source.clone()).unwrap()).unwrap();
    let text = after.chart_text(&store).unwrap();
    let changed = Chart::parse(text.clone()).unwrap();
    for original in source_chart.sections.iter().filter(|s| s.name != "Song") {
        let section = changed
            .sections
            .iter()
            .find(|s| s.name == original.name)
            .unwrap();
        assert_eq!(
            &source_chart.raw[original.body_start..original.body_end],
            &changed.raw[section.body_start..section.body_end]
        );
    }
    assert!(text.starts_with('\u{feff}'));
    assert!(text.contains(" Custom = retain\r\n"));
    assert!(text.contains(" Offset = -0.125\r\n"));
    action(&store, Action::Undo).unwrap();
    assert_eq!(
        store.load().unwrap().chart_text(&store).unwrap(),
        source_chart.raw
    );
    action(&store, Action::Redo).unwrap();
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), text);
    assert_eq!(
        fs::read(store.root.join("source/notes.chart")).unwrap(),
        source
    );
    assert_eq!(
        crate::models::hash_file(&store.audio_path(&after).unwrap()).unwrap(),
        audio
    );
    assert_eq!(
        fs::read_to_string(root.path().join("song/notes.chart")).unwrap(),
        CHART
    );
}

#[test]
fn edited_legacy_delay_exports_and_reopens_exactly_once_in_either_direction() {
    for offset in [0.75, -0.125] {
        let root = tempfile::tempdir().unwrap();
        let store = fixture(root.path(), true);
        set(&store, 0.25, offset).unwrap();
        let output = root.path().join("export");
        action(
            &store,
            Action::Export {
                output: output.clone(),
            },
        )
        .unwrap();
        let ini = fs::read_to_string(output.join("song.ini")).unwrap();
        assert_eq!(crate::song_metadata::ini_fields(&ini)["delay"], "0");
        let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
        assert_eq!(
            reopened.load().unwrap().timing(&reopened).unwrap(),
            store.load().unwrap().timing(&store).unwrap()
        );
        let original = store.load().unwrap();
        let restored = reopened.load().unwrap();
        assert_eq!(
            restored.content.tracks["ExpertSingle"]
                .iter()
                .map(|n| &n.note)
                .collect::<Vec<_>>(),
            original.content.tracks["ExpertSingle"]
                .iter()
                .map(|n| &n.note)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn invalid_stale_and_recording_overflow_offsets_leave_revision_and_history_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), true);
    let bytes = fs::read(store.root.join("project.json")).unwrap();
    for (previous, seconds) in [
        (0., 0.125),
        (f64::NAN, 0.125),
        (0.25, f64::NAN),
        (0.25, f64::INFINITY),
        (0.25, 0.0005),
        (0.25, 2147483.648),
        (0.25, -2147483.648),
        (0.25, 3.),
    ] {
        assert!(
            set(&store, previous, seconds).is_err(),
            "{previous} -> {seconds}"
        );
        assert_eq!(fs::read(store.root.join("project.json")).unwrap(), bytes);
        assert_eq!(
            fs::read_dir(store.root.join("checkpoints"))
                .unwrap()
                .count(),
            0
        );
    }
    set(&store, 0.25, 0.).unwrap();
    assert!(set(&store, 0.25, 0.125).is_err());
}

#[test]
fn chart_only_offset_grows_timeline_and_undo_restores_both() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), false);
    let before = store.load().unwrap().duration;
    set(&store, 0.25, 120.).unwrap();
    assert!(store.load().unwrap().duration > 120.);
    action(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().duration, before);
    assert_eq!(store.load().unwrap().timing(&store).unwrap().offset, 0.25);
    action(&store, Action::Redo).unwrap();
    assert!(store.load().unwrap().duration > 120.);
}

#[test]
fn legacy_sync_json_retains_source_offset_and_submillisecond_precision() {
    let chart = Chart::parse(CHART.replace("Offset = 0.5", "Offset = 0.0005")).unwrap();
    let sync = imported(&chart).unwrap();
    let json = serde_json::to_value(&sync).unwrap();
    assert!(json.get("offset").is_none());
    let legacy: SyncTrack = serde_json::from_value(json).unwrap();
    assert_eq!(legacy.validate(&chart.timing).unwrap().offset, 0.0005);
    assert_eq!(render(chart.raw.clone(), &Some(legacy)).unwrap(), chart.raw);
}

#[test]
fn offset_keeps_linked_difficulty_notes_and_ancestry() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), true);
    action(
        &store,
        Action::CreateTrack {
            track: "HardSingle".into(),
        },
    )
    .unwrap();
    action(
        &store,
        Action::EnableDifficultyLink {
            track: "HardSingle".into(),
            enabled: true,
        },
    )
    .unwrap();
    let before = store.load().unwrap();
    set(&store, 0.25, -0.125).unwrap();
    let after = store.load().unwrap();
    assert_eq!(
        before.content.difficulty_links,
        after.content.difficulty_links
    );
    assert_eq!(
        serde_json::to_value(before.content.tracks).unwrap(),
        serde_json::to_value(after.content.tracks).unwrap()
    );
}

#[test]
fn offset_archives_vocal_checks_and_undo_restores_their_original_clock() {
    use crate::vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone};
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), true);
    let document = VocalChart {
        version: 1,
        resolution: 480,
        source: "Offset fixture".into(),
        phrases: vec![VocalPhrase {
            start: 960,
            end: 1920,
            player: 1,
        }],
        notes: vec![VocalNote {
            midi: None,
            start: 960,
            end: 1440,
            tone: VocalTone::Pitched { midi: 60 },
            lyric: Syllable::Text {
                text: "keep".into(),
                join_next: false,
            },
        }],
    };
    action(
        &store,
        Action::SetVocals {
            document: document.clone(),
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    let review = VocalReviewMap {
        version: 1,
        audio_sha256: doc.audio_sha256.clone(),
        vocal_stem_sha256: "b".repeat(64),
        duration_seconds: doc.duration,
        timing: Some(doc.timing(&store).unwrap()),
        report_sha256: "c".repeat(64),
        generated_chart_sha256: format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&document).unwrap())
        ),
        regions: vec![],
    };
    action(
        &store,
        Action::SetVocalReview {
            review: review.clone(),
        },
    )
    .unwrap();
    set(&store, 0.25, -0.125).unwrap();
    let after = store.load().unwrap();
    assert_eq!(after.content.vocals.as_ref(), Some(&document));
    assert!(after.content.vocal_review.is_none());
    assert_eq!(
        serde_json::to_value(after.content.sync.unwrap().archived_vocal_review.unwrap()).unwrap(),
        serde_json::to_value(&review).unwrap()
    );
    action(&store, Action::Undo).unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content.vocal_review.unwrap()).unwrap(),
        serde_json::to_value(review).unwrap()
    );
}

#[test]
fn offset_refuses_to_hide_original_phrases_and_other_timed_events() {
    for extra in [
        "[ExpertSingle]\r\n{\r\n960 = N 0 120\r\n960 = S 2 3840\r\n}\r\n",
        "[ExpertSingle]\r\n{\r\n960 = N 0 120\r\n}\r\n[Events]\r\n{\r\n4800 = E \"section tail\"\r\n}\r\n",
    ] {
        let root = tempfile::tempdir().unwrap();
        let song = root.path().join("song");
        fs::create_dir(&song).unwrap();
        fs::write(song.join("notes.chart"),format!("[Song]\r\n{{\r\nResolution = 480\r\n}}\r\n[SyncTrack]\r\n{{\r\n0 = B 120000\r\n}}\r\n{extra}")).unwrap();
        fs::write(song.join("song.ini"), "[song]\nname = Tail fixture\n").unwrap();
        crate::models::write_audio(
            &song.join("song.wav"),
            &vec![0.; 44100 * 6],
            &vec![0.; 44100 * 6],
        )
        .unwrap();
        let store = Store::create(&song, None, &root.path().join("project")).unwrap();
        let revision = store.load().unwrap().revision;
        assert!(set(&store, 0., 1.5).is_err());
        assert_eq!(store.load().unwrap().revision, revision);
        assert_eq!(store.load().unwrap().timing(&store).unwrap().offset, 0.);
    }
}
