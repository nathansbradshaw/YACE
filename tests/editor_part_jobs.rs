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
        tempo_map: None,
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
    let mut settings = request(&store, "drums");
    // Even a saved audio-analysis proposal cannot replace an open chart's clock.
    settings.tempo_map = Some(
        chart_engine::timing::TempoMap::new(
            480,
            0.0,
            vec![
                chart_engine::timing::Tempo {
                    tick: 0,
                    bpm: settings.bpm,
                },
                chart_engine::timing::Tempo {
                    tick: 480,
                    bpm: 90.0,
                },
            ],
        )
        .unwrap(),
    );
    let job = parts::create_job(settings, &root.path().join("drum-job"), &target).unwrap();
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

#[test]
fn applying_a_generated_part_retains_its_stems_for_game_export_and_undo() {
    use chart_engine::stem_review::{AlignmentEvidence, StemId, StemManifest};
    use std::collections::BTreeMap;
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let before = store.load().unwrap();
    assert!(before.content.stems.is_none());
    let target = parts::Target::capture(&store, &before, "ExpertDrums").unwrap();
    let job = parts::create_job(
        request(&store, "drums"),
        &root.path().join("part-job"),
        &target,
    )
    .unwrap();
    generation::write_json(
        &job.join("events.json"),
        &Transcription {
            drums: vec![DrumHit {
                time: 1.5,
                voice: Voice::Kick,
                confidence: 0.9,
            }],
            ..Default::default()
        },
    )
    .unwrap();
    generation::run(&job).unwrap();
    let audio = store.audio_path(&before).unwrap();
    let mut files = BTreeMap::new();
    for id in StemId::ALL {
        let path = root.path().join(format!("{}-separated.wav", id.as_str()));
        fs::copy(&audio, &path).unwrap();
        files.insert(id, path);
    }
    let manifest = StemManifest::build(&audio, files, AlignmentEvidence {
        method: "synthetic_silent_channels".into(),
        details: "Silent channels of the exact fixture length, testing persistence and routing rather than model separation.".into(),
    }).unwrap();
    let path = root.path().join("generated-stems.json");
    generation::write_json(&path, &manifest).unwrap();
    let generated = Store::new(&job.join("project"));
    generated
        .apply(Edit {
            expected_revision: generated.load().unwrap().revision,
            actor: human(),
            action: Action::AttachStems { manifest: path },
        })
        .unwrap();
    store
        .apply(Edit {
            expected_revision: before.revision,
            actor: human(),
            action: Action::ApplyGeneratedPart { job: job.clone() },
        })
        .unwrap();
    let imported = store.load().unwrap();
    let retained = imported.content.stems.as_ref().unwrap();
    assert_eq!(retained.ids().len(), 6);
    for asset in &retained.stems {
        assert!(asset.path.starts_with(store.root.join("source/stems")));
    }
    store
        .apply(Edit {
            expected_revision: imported.revision,
            actor: human(),
            action: Action::Undo,
        })
        .unwrap();
    assert!(store.load().unwrap().content.stems.is_none());
    store
        .apply(Edit {
            expected_revision: store.load().unwrap().revision,
            actor: human(),
            action: Action::Redo,
        })
        .unwrap();
    fs::remove_dir_all(&job).unwrap();
    for asset in manifest.stems {
        fs::remove_file(asset.path).unwrap();
    }
    let output = root.path().join("game-export");
    store
        .apply(Edit {
            expected_revision: store.load().unwrap().revision,
            actor: human(),
            action: Action::Export {
                output: output.clone(),
            },
        })
        .unwrap();
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("yace-audio.json")).unwrap()).unwrap();
    assert_eq!(receipt["mode"], "instrument_stems");
    assert!(output.join("drums.wav").is_file());
    assert!(output.join("keys.wav").is_file());
}
