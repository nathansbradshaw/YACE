use chart_engine::{
    chart::{Chart, Note},
    editor::{Action, Actor, ActorKind, Content, EditorNote, Project, Request, Store, Transport},
    review_regions::{Policy, ReviewMap},
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

const SOURCE: &str = "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[Events]\n{\n 0 = E \"lyric Preserved\"\n}\n[ExpertDrums]\n{\n 480 = N 2 0\n 480 = N 66 0\n 480 = N 35 0\n 480 = N 41 0\n 960 = N 1 0\n 960 = N 34 0\n 1200 = N 12 17\n 0 = S 2 480\n}\n";

fn fixture(root: &Path) -> Store {
    fs::create_dir(root.join("source")).unwrap();
    fs::create_dir(root.join("checkpoints")).unwrap();
    fs::write(root.join("source/notes.chart"), SOURCE).unwrap();
    let audio = b"opaque audio bytes; note editing requires no decoder";
    fs::write(root.join("source/song.ogg"), audio).unwrap();
    let source_hash = format!("{:x}", Sha256::digest(SOURCE.as_bytes()));
    let audio_hash = format!("{:x}", Sha256::digest(audio));
    let chart = Chart::parse(SOURCE.into()).unwrap();
    let notes = chart
        .notes("ExpertDrums")
        .unwrap()
        .into_iter()
        .map(|note| EditorNote {
            id: format!("n-{}-{}", note.lane, note.tick),
            note,
        })
        .collect();
    let project = Project {
        version: 1,
        revision: 0,
        title: "Articulation fixture".into(),
        original_folder: root.join("original"),
        source_chart_sha256: source_hash.clone(),
        audio_filename: "song.ogg".into(),
        audio_sha256: audio_hash.clone(),
        duration: 4.0,
        review: ReviewMap::build(
            4.0,
            Policy::default(),
            BTreeMap::from([
                ("chart".into(), source_hash),
                ("original_audio".into(), audio_hash),
            ]),
            None,
            Default::default(),
            vec![],
        )
        .unwrap(),
        content: Content {
            star_power: Default::default(),
            sync: None,
            sections: None,
            track_origins: Default::default(),
            difficulty_links: Default::default(),
            vocals: None,
            vocal_review: None,
            tracks: BTreeMap::from([("ExpertDrums".into(), notes)]),
            decisions: vec![],
            hidden_flags: Default::default(),
            stems: None,
            keyboard_stems: None,
            stem_mix: Default::default(),
        },
        selected_track: "ExpertDrums".into(),
        selected_region: None,
        selected_note: Some("n-2-480".into()),
        kit_audition: None,
        transport: Transport {
            playback_rate: 1.0,
            position: 0.0,
            playing: false,
            anchor_ms: 0,
            loop_range: None,
        },
        next_id: 1,
        undo: vec![],
        redo: vec![],
        audit: vec![],
    };
    fs::write(
        root.join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    Store::new(root)
}

fn request(store: &Store, action: Action) -> Request {
    Request {
        expected_revision: store.load().unwrap().revision,
        actor: Actor {
            kind: ActorKind::Ai,
            name: "Articulation regression".into(),
        },
        action,
    }
}

fn update(tick: u64, lane: u8, cymbal: bool) -> Action {
    Action::UpdateNote {
        note_id: "n-2-480".into(),
        tick,
        lane,
        sustain: 0,
        cymbal,
    }
}

#[test]
fn articulated_drums_move_delete_undo_and_reopen_without_orphaning_markers() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    store.apply(request(&store, update(720, 3, true))).unwrap();
    let project = store.load().unwrap();
    for (id, lane) in [("n-35-480", 36), ("n-41-480", 42)] {
        let (_, marker) = project.find_note(id).unwrap();
        assert_eq!(
            marker.note,
            Note {
                tick: 720,
                lane,
                sustain: 0
            }
        );
    }
    assert!(
        project.content.tracks["ExpertDrums"]
            .iter()
            .any(|event| event.note.tick == 720 && event.note.lane == 67)
    );
    assert!(
        !project.content.tracks["ExpertDrums"]
            .iter()
            .any(|event| event.note.tick == 480)
    );
    assert_eq!(project.find_note("n-34-960").unwrap().1.note.lane, 34);
    let inspection = store.inspect().unwrap();
    let note = inspection["octave_view"]["tracks"][0]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == "n-2-480")
        .unwrap();
    assert_eq!(note["lane_name"], "blueCymbal");
    assert_eq!(note["flags"]["accent"], true);
    assert_eq!(note["flags"]["ghost"], true);
    assert_eq!(note["editable"], true);
    let text = project.chart_text(&store).unwrap();
    assert!(text.contains("0 = S 2 480\n"));
    assert!(text.contains("0 = E \"lyric Preserved\"\n"));
    assert!(text.contains("1200 = N 12 17\n"));
    store.apply(request(&store, Action::Undo)).unwrap();
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), SOURCE);
    store.apply(request(&store, Action::Redo)).unwrap();
    let reopened = Store::new(root.path());
    assert_eq!(
        reopened
            .load()
            .unwrap()
            .find_note("n-35-480")
            .unwrap()
            .1
            .note
            .lane,
        36
    );
    reopened
        .apply(request(
            &reopened,
            Action::DeleteNote {
                note_id: "n-2-480".into(),
            },
        ))
        .unwrap();
    let project = reopened.load().unwrap();
    assert!(project.find_note("n-35-480").is_none());
    assert!(project.find_note("n-41-480").is_none());
    assert!(
        !project.content.tracks["ExpertDrums"]
            .iter()
            .any(|event| [36, 42, 67].contains(&event.note.lane))
    );
    assert!(project.find_note("n-34-960").is_some());
    assert_eq!(
        fs::read_to_string(root.path().join("source/notes.chart")).unwrap(),
        SOURCE
    );
}

#[test]
fn changing_an_articulated_pad_to_kick_removes_incompatible_controls_reversibly() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    store.apply(request(&store, update(720, 0, false))).unwrap();
    let project = store.load().unwrap();
    assert_eq!(project.find_note("n-2-480").unwrap().1.note.lane, 0);
    for marker in ["n-35-480", "n-41-480", "n-66-480"] {
        assert!(project.find_note(marker).is_none());
    }
    assert!(
        !project.content.tracks["ExpertDrums"]
            .iter()
            .any(|event| event.note.tick == 720 && [33, 39, 64].contains(&event.note.lane))
    );
    store.apply(request(&store, Action::Undo)).unwrap();
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), SOURCE);
}

#[test]
fn articulation_collision_rejects_the_whole_move_without_losing_marker_ids() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let mut project = store.load().unwrap();
    project
        .content
        .tracks
        .get_mut("ExpertDrums")
        .unwrap()
        .push(EditorNote {
            id: "destination-accent".into(),
            note: Note {
                tick: 720,
                lane: 36,
                sustain: 0,
            },
        });
    fs::write(
        root.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let before = fs::read(root.path().join("project.json")).unwrap();
    let error = store
        .apply(request(&store, update(720, 3, true)))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("articulation marker already exists")
    );
    assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
    assert!(store.load().unwrap().find_note("n-35-480").is_some());
    assert!(
        store
            .load()
            .unwrap()
            .find_note("destination-accent")
            .is_some()
    );
}
