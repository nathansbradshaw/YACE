use chart_engine::{
    clean::{AlignedWord, WordTimings},
    editor::{Actor, ActorKind, Store},
    models,
    stem_review::{AlignmentEvidence, StemId, StemManifest},
    timing::TempoMap,
    vocal_generation::Status,
    vocal_workflow,
};
use std::{collections::BTreeMap, fs, sync::Arc};

#[test]
fn source_bound_model_free_workflow_is_atomic_and_can_export_game_vocals_without_models() {
    let temp = tempfile::tempdir().unwrap();
    let audio = temp.path().join("original.wav");
    let stem = temp.path().join("voice.wav");
    let samples: Vec<_> = (0..44100 * 2)
        .map(|i| {
            let t = i as f32 / 44100.;
            if (0.3..1.6).contains(&t) {
                0.2 * (std::f32::consts::TAU * 220. * t).sin()
            } else {
                0.
            }
        })
        .collect();
    models::write_audio(&audio, &samples, &samples).unwrap();
    fs::copy(&audio, &stem).unwrap();
    let hash = models::hash_file(&audio).unwrap();
    let manifest = StemManifest::build(
        &audio,
        BTreeMap::from([(StemId::Vocals, stem.clone())]),
        AlignmentEvidence {
            method: "synthetic isolated voice".into(),
            details: "Same known 220 Hz signal and sample clock, no separator or AI".into(),
        },
    )
    .unwrap();
    let path = temp.path().join("manifest.json");
    manifest.write_relative(&path).unwrap();
    let words = WordTimings {
        version: 1,
        audio_sha256: hash,
        duration_seconds: 2.,
        source: "Known synthetic word bounds".into(),
        words: vec![AlignedWord {
            text: Some("stay".into()),
            start: Some(0.3),
            end: Some(1.6),
        }],
    };
    let map = TempoMap::constant(120.).unwrap();
    let missing_models = temp.path().join("no-models");
    let output = temp.path().join("vocal-output");
    let run = || {
        vocal_workflow::generate(
            vocal_workflow::Request {
                audio: &audio,
                manifest: &path,
                timing: &map,
                words: Some(&words),
                lyrics: None,
                recognize: false,
                language: Default::default(),
                neural_pitch: false,
                model_dir: &missing_models,
                download_model: false,
            },
            &output,
            Arc::default(),
        )
    };
    let generated = run().unwrap();
    assert_eq!(generated.status, Status::ReviewRequired);
    assert!(generated.chart.is_some());
    assert!(output.join("vocals.mid").exists());
    assert!(!missing_models.exists());
    let before = fs::read(output.join("generation.json")).unwrap();
    assert!(run().is_err());
    assert_eq!(before, fs::read(output.join("generation.json")).unwrap());
    // Cancelling before analysis cannot create a partially published result.
    let control = Arc::new(chart_engine::clean_recognition::Control::default());
    control.cancel();
    let cancelled = temp.path().join("cancelled");
    assert!(
        vocal_workflow::generate(
            vocal_workflow::Request {
                audio: &audio,
                manifest: &path,
                timing: &map,
                words: Some(&words),
                lyrics: None,
                recognize: false,
                language: Default::default(),
                neural_pitch: false,
                model_dir: &missing_models,
                download_model: false
            },
            &cancelled,
            control
        )
        .is_err()
    );
    assert!(!cancelled.exists());
    // An existing editor project can retain pitch-only proposals without
    // manufacturing words, changing instruments, or needing model weights.
    let song = temp.path().join("song");
    fs::create_dir(&song).unwrap();
    fs::copy(&audio, song.join("song.wav")).unwrap();
    fs::write(song.join("notes.chart"),"[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[ExpertDrums]\n{\n 480 = N 1 0\n}\n").unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Synthetic\nartist = Test\n",
    )
    .unwrap();
    fs::write(
        song.join("chart-engine-isolation.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let store = Store::create(&song, None, &temp.path().join("project")).unwrap();
    let revision = store.load().unwrap().revision;
    let result = vocal_workflow::generate_project(
        vocal_workflow::ProjectRequest {
            project: &store.root,
            model_dir: &missing_models,
            recognize: false,
            language: Default::default(),
            neural_pitch: false,
            download_model: false,
            lookup_lyrics: false,
            actor: Actor {
                kind: ActorKind::Ai,
                name: "Model-free workflow test".into(),
            },
        },
        Arc::default(),
    )
    .unwrap();
    assert_eq!(result.status, Status::NeedsLyrics);
    assert!(result.applied);
    assert!(result.output.join("generation.json").exists());
    assert_eq!(store.load().unwrap().revision, revision + 1);
    assert!(
        store
            .load()
            .unwrap()
            .content
            .vocals
            .unwrap()
            .notes
            .iter()
            .all(|note| note.lyric == chart_engine::vocals::Syllable::Missing)
    );
    assert!(result.output.join("vocals.mid").exists());
    assert!(!missing_models.exists());
}
