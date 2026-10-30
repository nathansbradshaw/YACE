use chart_engine::{
    editor::{Action, Actor, ActorKind, Request as Edit, Store},
    generation::{self, Request, Source},
    mapping::{DrumHit, Transcription, Voice},
    models, parts,
};
use std::{fs, path::Path};

fn fixture(root: &Path) -> Store {
    let song = root.join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"), "[Song]\n{\n Resolution = 960\n Offset = 0.25\n}\n[SyncTrack]\n{\n 0 = B 120000\n 960 = B 90000\n}\n[Events]\n{\n 0 = E \"section intro\"\n}\n[ExpertSingle]\n{\n 480 = N 0 0\n}\n[ExpertDrums]\n{\n 480 = N 1 0\n}\n").unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Job fixture\nartist = Test\ncustom = keep\n",
    )
    .unwrap();
    let samples = vec![0.; 44100 * 3];
    models::write_audio(&song.join("song.wav"), &samples, &samples).unwrap();
    Store::create(&song, None, &root.join("project")).unwrap()
}
fn request(store: &Store, instrument: &str) -> Request {
    Request {
        source: Source::Audio {
            path: store.audio_path(&store.load().unwrap()).unwrap(),
        },
        bpm: 120.,
        title: "Job fixture".into(),
        artist: "Test".into(),
        instruments: vec![instrument.into()],
        model_dir: store.root.join("missing-models"),
        guitar_source: Default::default(),
        keyboard_sound: Default::default(),
        piano_quality: Default::default(),
        vocal_language: Default::default(),
        fetch_models: false,
        full_mix_drums: instrument == "drums",
        standard_drums: false,
        lyrics: false,
        song_info: None,
    }
}
fn human() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "Part job test".into(),
    }
}

#[test]
fn a_saved_detection_job_keeps_the_open_charts_tempo_and_other_parts() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let project = store.load().unwrap();
    let target = parts::Target::capture(&store, &project, "ExpertDrums").unwrap();
    let job = parts::create_job(
        request(&store, "drums"),
        &root.path().join("drum-job"),
        &target,
    )
    .unwrap();
    // Supplied detections exercise the real job/arrangement/import path, not model accuracy.
    generation::write_json(
        &job.join("events.json"),
        &Transcription {
            drums: vec![
                DrumHit {
                    time: 0.5,
                    voice: Voice::Snare,
                    confidence: 0.9,
                },
                DrumHit {
                    time: 1.5,
                    voice: Voice::Kick,
                    confidence: 0.9,
                },
            ],
            ..Default::default()
        },
    )
    .unwrap();
    generation::run(&job).unwrap();
    let generated = Store::new(&job.join("project")).load().unwrap();
    assert_eq!(
        generated.timing(&Store::new(&job.join("project"))).unwrap(),
        target.timing
    );
    let tick = target.timing.tick(1.5).unwrap();
    assert!(
        generated.content.tracks["ExpertDrums"]
            .iter()
            .any(|note| note.note.tick == tick && note.note.lane == 0)
    );
    store
        .apply(Edit {
            expected_revision: project.revision,
            actor: human(),
            action: Action::ApplyGeneratedPart { job },
        })
        .unwrap();
    let imported = store.load().unwrap();
    assert_eq!(
        serde_json::to_value(&imported.content.tracks["ExpertSingle"]).unwrap(),
        serde_json::to_value(&project.content.tracks["ExpertSingle"]).unwrap()
    );
    assert_eq!(imported.timing(&store).unwrap(), target.timing);
    store
        .apply(Edit {
            expected_revision: imported.revision,
            actor: human(),
            action: Action::Undo,
        })
        .unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(project.content).unwrap()
    );
}

#[test]
fn vocal_only_jobs_need_no_instrumental_detections_or_invented_notes() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let project = store.load().unwrap();
    let request = request(&store, "vocals");
    if cfg!(feature = "separation") {
        request.validate().unwrap();
    }
    let job = root.path().join("vocal-container");
    fs::create_dir(&job).unwrap();
    let audio = store.audio_path(&project).unwrap();
    let timing = project.timing(&store).unwrap();
    parts::vocal_song(&job, &audio, &request, &timing).unwrap();
    let container = Store::create(&job.join("song"), None, &job.join("project")).unwrap();
    let draft = container.load().unwrap();
    assert_eq!(draft.audio_sha256, project.audio_sha256);
    assert_eq!(draft.timing(&container).unwrap(), timing);
    assert!(draft.content.tracks.values().all(Vec::is_empty));
    assert!(draft.content.vocals.is_none());
    assert!(!job.join("song/chart-engine-lyrics.json").exists());
}

#[test]
fn a_part_job_moves_with_its_project_and_resumes_from_the_bound_recording() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let target = parts::Target::capture(&store, &store.load().unwrap(), "ExpertDrums").unwrap();
    fs::create_dir(store.root.join("generation-jobs")).unwrap();
    let job = parts::create_job(
        request(&store, "drums"),
        &store.root.join("generation-jobs/draft"),
        &target,
    )
    .unwrap();
    generation::write_json(
        &job.join("events.json"),
        &Transcription {
            drums: vec![DrumHit {
                time: 1.,
                voice: Voice::Snare,
                confidence: 0.9,
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let moved = root.path().join("moved-project");
    fs::rename(&store.root, &moved).unwrap();
    let store = Store::new(&moved);
    let job = moved.join("generation-jobs/draft");
    parts::target(&job)
        .unwrap()
        .unwrap()
        .validate(&store, &store.load().unwrap())
        .unwrap();
    generation::run(&job).unwrap();
    store
        .apply(Edit {
            expected_revision: store.load().unwrap().revision,
            actor: human(),
            action: Action::ApplyGeneratedPart { job },
        })
        .unwrap();
    assert_eq!(store.load().unwrap().selected_track, "ExpertDrums");
}
