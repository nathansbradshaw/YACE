use chart_engine::{
    editor::Store,
    generation::{self, Request, Source, State},
    mapping::{DrumHit, Transcription, Voice},
    models,
};
use std::{fs, path::Path};

fn request(audio: &Path) -> Request {
    Request {
        guitar_source: Default::default(),
        vocal_language: Default::default(),
        source: Source::Audio {
            path: audio.to_owned(),
        },
        bpm: 120.0,
        title: "Job fixture".into(),
        artist: "Local test".into(),
        instruments: vec!["drums".into()],
        keyboard_sound: Default::default(),
        piano_quality: Default::default(),
        model_dir: audio.parent().unwrap().join("missing-models"),
        fetch_models: false,
        full_mix_drums: true,
        standard_drums: false,
        lyrics: false,
        song_info: None,
    }
}
fn audio(root: &Path) -> std::path::PathBuf {
    let path = root.join("original.wav");
    let samples = vec![0.0; 44100 * 3];
    models::write_audio(&path, &samples, &samples).unwrap();
    path
}

#[test]
fn piano_quality_request_defaults_and_headless_native_capabilities_are_explicit() {
    use chart_engine::keyboard_detection::{KeyboardSound, PianoQuality};
    let temp = tempfile::tempdir().unwrap();
    let input = audio(temp.path());
    let mut settings = request(&input);
    settings.full_mix_drums = false;
    settings.instruments = vec!["piano".into(), "pro_piano".into()];
    settings.piano_quality = PianoQuality::HighQuality;
    settings.validate().unwrap();
    let restored: Request =
        serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(restored.piano_quality, PianoQuality::HighQuality);
    restored.validate().unwrap();
    let mut saved = serde_json::to_value(&settings).unwrap();
    saved.as_object_mut().unwrap().remove("piano_quality");
    let legacy: Request = serde_json::from_value(saved).unwrap();
    assert_eq!(legacy.piano_quality, PianoQuality::Standard);
    if !cfg!(feature = "separation") {
        assert!(
            legacy.validate().is_err(),
            "standard route still requires its standard separator"
        );
        settings.instruments.push("bass".into());
        assert!(
            settings.validate().is_err(),
            "mixed jobs retain the standard separator requirement"
        );
        settings.instruments = vec!["piano".into()];
        settings.keyboard_sound = KeyboardSound::Synth;
        assert!(
            settings.validate().is_err(),
            "Synth does not silently switch separation backends"
        );
    }
}
fn saved_events(root: &Path) {
    // Supplied events exercise packaging/recovery without pretending a fixture
    // is evidence of real audio transcription. Native inference is tested separately.
    generation::write_json(
        &root.join("events.json"),
        &Transcription {
            drums: vec![
                DrumHit {
                    time: 0.5,
                    voice: Voice::Snare,
                    confidence: 0.8,
                },
                DrumHit {
                    time: 1.0,
                    voice: Voice::Kick,
                    confidence: 0.9,
                },
            ],
            ..Default::default()
        },
    )
    .unwrap();
}

#[cfg(feature = "separation")]
#[test]
fn guitar_sound_source_is_persisted_and_cannot_retarget_saved_candidates() {
    use chart_engine::mapping::GuitarSource;
    let temp = tempfile::tempdir().unwrap();
    let input = audio(temp.path());
    let mut settings = request(&input);
    settings.full_mix_drums = false;
    settings.instruments = vec!["guitar".into()];
    settings.guitar_source = GuitarSource::Other;
    let root = temp.path().join("job");
    generation::create(settings.clone(), &root).unwrap();
    let restored: Request =
        serde_json::from_slice(&fs::read(root.join("request.json")).unwrap()).unwrap();
    assert_eq!(restored.guitar_source, GuitarSource::Other);
    let mut legacy = serde_json::to_value(&settings).unwrap();
    legacy.as_object_mut().unwrap().remove("guitar_source");
    assert_eq!(
        serde_json::from_value::<Request>(legacy)
            .unwrap()
            .guitar_source,
        GuitarSource::Guitar
    );
    saved_events(&root);
    let previous = fs::read(root.join("events.json")).unwrap();
    let error = generation::run(&root).unwrap_err();
    assert!(format!("{error:#}").contains("different sound source"));
    assert_eq!(fs::read(root.join("events.json")).unwrap(), previous);
    assert!(!root.join("song").exists());
}

#[cfg(feature = "separation")]
fn stale_bass_job() -> (tempfile::TempDir, std::path::PathBuf, Transcription) {
    use chart_engine::{
        bass_analysis,
        mapping::{PitchCandidateEvidence, PitchCandidateSource, PitchNote},
        stem_review::{AlignmentEvidence, StemId, StemManifest},
    };
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original.wav");
    let samples: Vec<f32> = (0..44100 * 3)
        .map(|index| {
            let time = index as f64 / 44100.;
            if (0.2..2.5).contains(&time) {
                (0.3 * (std::f64::consts::TAU * 110. * time).sin()) as f32
            } else {
                0.
            }
        })
        .collect();
    models::write_audio(&original, &samples, &samples).unwrap();
    let mut req = request(&original);
    req.instruments = vec!["bass".into(), "drums".into()];
    req.full_mix_drums = false;
    let root = generation::create(req, &temp.path().join("job")).unwrap();
    assert!(
        generation::run(&root).is_err(),
        "Import is retained before missing-model failure"
    );
    let bass = temp.path().join("bass.wav");
    fs::copy(&original, &bass).unwrap();
    let drums = temp.path().join("drums.wav");
    let silence = vec![0.; 44100 * 3];
    models::write_audio(&drums, &silence, &silence).unwrap();
    let manifest=StemManifest::build(&original,[(StemId::Bass,bass.clone()),(StemId::Drums,drums)].into_iter().collect(),AlignmentEvidence{
        method:"constructed_test_fixture".into(),details:"Synthetic bass tone and silent drum stem share the source sample clock; no model inference or musical accuracy is claimed.".into(),
    }).unwrap();
    let note = PitchNote {
        start: 0.5,
        end: 1.3,
        pitch: 57,
        confidence: 0.9,
    };
    let mut events = Transcription {
        isolation: Some(manifest),
        bass: vec![note.clone()],
        bass_candidates: vec![PitchCandidateEvidence {
            candidate: note,
            source: PitchCandidateSource::NetworkOnset,
            onset_score: 0.9,
            inferred_onset_score: 0.9,
        }],
        drums: vec![DrumHit {
            time: 1.5,
            voice: Voice::Kick,
            confidence: 0.8,
        }],
        ..Default::default()
    };
    bass_analysis::analyze(&mut events, &bass).unwrap();
    assert_eq!(events.bass[0].pitch, 45);
    events.bass_decoder = Some(bass_analysis::Decoder {
        version: 1,
        implementation_sha256: bass_analysis::decoder_implementation_hash(),
        model_sha256: models::assets()
            .into_iter()
            .find(|asset| asset.id == "pitch")
            .unwrap()
            .sha256,
        bass_audio_sha256: models::hash_file(&bass).unwrap(),
        candidates_sha256: format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&events.bass_candidates).unwrap())
        ),
        short_candidates_sha256: format!("{:x}", Sha256::digest(b"[]")),
    });
    events.bass_analysis.as_mut().unwrap().implementation_sha256 = "0".repeat(64);
    generation::write_json(&root.join("events.json"), &events).unwrap();
    (temp, root, events)
}

#[cfg(feature = "separation")]
#[test]
fn stale_refinement_resumes_from_raw_without_model_and_archives_exact_previous_events() {
    use chart_engine::{bass_analysis, chart::Chart, stem_review::StemId};
    use sha2::{Digest, Sha256};
    let (_directory, root, previous) = stale_bass_job();
    let bytes = fs::read(root.join("events.json")).unwrap();
    generation::run(&root).unwrap();
    assert_eq!(generation::status(&root).unwrap().state, State::Complete);
    let archive = root
        .join("analysis-history")
        .join(format!("bass-{:x}.json", Sha256::digest(&bytes)));
    assert_eq!(fs::read(archive).unwrap(), bytes);
    let refreshed: Transcription =
        serde_json::from_slice(&fs::read(root.join("events.json")).unwrap()).unwrap();
    let stem = &refreshed
        .isolation
        .as_ref()
        .unwrap()
        .stems
        .iter()
        .find(|stem| stem.id == StemId::Bass)
        .unwrap()
        .path;
    assert!(bass_analysis::is_current(&refreshed, stem).unwrap());
    assert!(bass_analysis::decoder_is_current(&refreshed, stem).unwrap());
    assert_eq!(
        refreshed.bass_analysis.as_ref().unwrap().original_notes[0].pitch,
        57
    );
    assert_eq!(refreshed.bass[0].pitch, 45);
    assert_eq!(
        serde_json::to_value(&refreshed.drums).unwrap(),
        serde_json::to_value(&previous.drums).unwrap()
    );
    let chart = Chart::parse(fs::read_to_string(root.join("song/notes.chart")).unwrap()).unwrap();
    for track in [
        "EasyDoubleBass",
        "MediumDoubleBass",
        "HardDoubleBass",
        "ExpertDoubleBass",
        "ExpertDrums",
    ] {
        assert!(!chart.notes(track).unwrap().is_empty(), "Missing {track}");
    }
    let request: Request =
        serde_json::from_slice(&fs::read(root.join("request.json")).unwrap()).unwrap();
    assert!(!request.model_dir.exists());
}

#[cfg(feature = "separation")]
#[test]
fn failed_stale_analysis_backup_preserves_current_events_and_corrupt_history_for_recovery() {
    use sha2::{Digest, Sha256};
    let (_directory, root, _) = stale_bass_job();
    let bytes = fs::read(root.join("events.json")).unwrap();
    fs::create_dir(root.join("analysis-history")).unwrap();
    let archive = root
        .join("analysis-history")
        .join(format!("bass-{:x}.json", Sha256::digest(&bytes)));
    fs::write(&archive, b"incomplete archive from an earlier failure").unwrap();
    let error = generation::run(&root).unwrap_err();
    assert!(format!("{error:#}").contains("history changed"));
    assert_eq!(fs::read(root.join("events.json")).unwrap(), bytes);
    assert_eq!(
        fs::read(archive).unwrap(),
        b"incomplete archive from an earlier failure"
    );
    assert!(!root.join("song").exists());
    assert_eq!(generation::status(&root).unwrap().state, State::Failed);
}

#[cfg(feature = "separation")]
#[test]
fn stale_decoder_resume_requires_real_inference_and_retains_events_when_model_is_missing() {
    let (_directory, root, mut events) = stale_bass_job();
    events.bass_decoder.as_mut().unwrap().implementation_sha256 = "0".repeat(64);
    generation::write_json(&root.join("events.json"), &events).unwrap();
    let previous = fs::read(root.join("events.json")).unwrap();
    let error = generation::run(&root).unwrap_err();
    assert!(format!("{error:#}").contains("Model missing"));
    assert_eq!(fs::read(root.join("events.json")).unwrap(), previous);
    assert!(!root.join("analysis-history").exists());
    assert!(!root.join("song").exists());
    assert_eq!(generation::status(&root).unwrap().state, State::Failed);
}

#[test]
fn failed_jobs_keep_an_import_then_resume_into_an_editable_project() {
    let temp = tempfile::tempdir().unwrap();
    let original = audio(temp.path());
    let original_hash = models::hash_file(&original).unwrap();
    let root = generation::create(request(&original), &temp.path().join("job")).unwrap();
    assert!(generation::run(&root).is_err()); // Missing models, after successful import.
    assert_eq!(generation::status(&root).unwrap().state, State::Failed);
    assert!(
        generation::status(&root)
            .unwrap()
            .error
            .unwrap()
            .contains("Model missing")
    );
    assert_eq!(
        models::hash_file(&root.join("input/source.wav")).unwrap(),
        original_hash
    );
    assert_eq!(models::hash_file(&original).unwrap(), original_hash);
    fs::remove_file(&original).unwrap();
    saved_events(&root);
    generation::run(&root).unwrap();
    assert_eq!(generation::status(&root).unwrap().state, State::Complete);
    assert!(root.join("song/notes.chart").is_file());
    let store = Store::new(&root.join("project"));
    let project = store.load().unwrap();
    assert!(!project.content.tracks["ExpertDrums"].is_empty());
    let before = fs::read(root.join("project/project.json")).unwrap();
    generation::run(&root).unwrap();
    assert_eq!(fs::read(root.join("project/project.json")).unwrap(), before);
}

#[test]
fn refusal_and_locking_preserve_existing_data_and_request_identity() {
    let temp = tempfile::tempdir().unwrap();
    let original = audio(temp.path());
    let root = generation::create(request(&original), &temp.path().join("job")).unwrap();
    assert!(generation::create(request(&original), &root).is_err());
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("worker.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(generation::run(&root).is_err());
    assert!(generation::mark_interrupted(&root, true).is_err());
    drop(lock);
    generation::mark_interrupted(&root, true).unwrap();
    assert_eq!(generation::status(&root).unwrap().state, State::Cancelled);
    let mut modified = request(&original);
    modified.bpm = 135.0;
    generation::write_json(&root.join("request.json"), &modified).unwrap();
    assert!(
        generation::run(&root)
            .unwrap_err()
            .to_string()
            .contains("settings changed")
    );
    assert!(!root.join("song").exists());
}

#[test]
fn invalid_settings_never_create_an_output_folder() {
    let temp = tempfile::tempdir().unwrap();
    let original = audio(temp.path());
    let mut req = request(&original);
    for value in [0.0, f64::NAN, 1001.0] {
        req.bpm = value;
        assert!(generation::create(req.clone(), &temp.path().join("job")).is_err());
    }
    req.bpm = 120.0;
    req.instruments = vec!["guitar".into()];
    assert!(generation::create(req, &temp.path().join("job")).is_err());
    assert!(!temp.path().join("job").exists());
}

#[test]
fn non_game_audio_keeps_the_original_and_binds_the_editor_to_compatible_wav() {
    let temp = tempfile::tempdir().unwrap();
    let wave = audio(temp.path());
    let flac = temp.path().join("original.flac");
    assert!(
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(&wave)
            .arg(&flac)
            .status()
            .unwrap()
            .success()
    );
    let original_hash = models::hash_file(&flac).unwrap();
    let root = generation::create(request(&flac), &temp.path().join("job")).unwrap();
    saved_events(&root);
    generation::run(&root).unwrap();
    assert_eq!(
        models::hash_file(&root.join("input/source.flac")).unwrap(),
        original_hash
    );
    let input: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("input/source.json")).unwrap()).unwrap();
    assert_eq!(input["original"][1], original_hash);
    let project = Store::new(&root.join("project")).load().unwrap();
    assert_eq!(project.audio_filename, "song.wav");
    assert_eq!(
        project.audio_sha256,
        models::hash_file(&root.join("input/source.wav")).unwrap()
    );
}

#[test]
fn cli_create_show_and_run_share_the_job_contract_with_paths_containing_spaces() {
    let temp = tempfile::tempdir().unwrap();
    let original = audio(temp.path());
    let req = temp.path().join("generation request.json");
    generation::write_json(&req, &request(&original)).unwrap();
    let root = temp.path().join("saved generation job");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_chart-engine"))
        .args(["generation", "create", "--request"])
        .arg(&req)
        .arg("--output")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    saved_events(&root);
    for action in ["show", "run", "run"] {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_chart-engine"))
            .args(["generation", action])
            .arg(&root)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let state: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(
            state["state"],
            if action == "show" {
                "queued"
            } else {
                "complete"
            }
        );
    }
}

#[test]
fn local_audio_exports_the_users_metadata_artwork_and_lyric_edits() {
    let temp = tempfile::tempdir().unwrap();
    let wave = audio(temp.path());
    let art = temp.path().join("chosen.jpg");
    fs::write(&art, b"artwork copy fixture").unwrap();
    let mut req = request(&wave);
    req.title = "Corrected title".into();
    req.artist = "Corrected artist".into();
    req.lyrics = true;
    req.song_info = Some(chart_engine::song_import::SongInfo {
        album: "Chosen album".into(),
        year: "2024".into(),
        genre: "Chosen genre".into(),
        artwork: Some(art),
        lyrics: Some(
            serde_json::json!({"syncedLyrics":"[00:00.50] Edited lyric","plainLyrics":"Edited lyric"}),
        ),
        ..Default::default()
    });
    let root = generation::create(req, &temp.path().join("job")).unwrap();
    saved_events(&root);
    generation::run(&root).unwrap();
    let ini = fs::read_to_string(root.join("song/song.ini")).unwrap();
    for value in [
        "Corrected title",
        "Corrected artist",
        "Chosen album",
        "2024",
        "Chosen genre",
    ] {
        assert!(ini.contains(value), "{ini}");
    }
    assert!(
        fs::read_to_string(root.join("song/notes.chart"))
            .unwrap()
            .contains("Edited lyric")
    );
    assert_eq!(
        fs::read(root.join("project/source/album.jpg")).unwrap(),
        b"artwork copy fixture"
    );
    assert!(
        root.join("project/source/import-information.json")
            .is_file()
    );
}
