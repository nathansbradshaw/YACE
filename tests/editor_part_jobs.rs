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
        detection: Default::default(),
        source: Source::Audio {
            path: store.audio_path(&store.load().unwrap()).unwrap(),
        },
        existing_stems: None,
        bpm: 120.,
        title: "Job fixture".into(),
        artist: "Test".into(),
        instruments: vec![instrument.into()],
        model_dir: store.root.join("missing-models"),
        guitar_source: Default::default(),
        keyboard_sound: Default::default(),
        piano_quality: Default::default(),
        vocal_language: Default::default(),
        vocal_analysis_mode: Default::default(),
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

fn completed_project_job(root: &Path, store: &Store, instruments: &[&str]) -> std::path::PathBuf {
    let before = store.load().unwrap();
    let mut request = request(store, "vocals");
    request.instruments = instruments.iter().map(|s| s.to_string()).collect();
    request.tempo_map = Some(before.timing(store).unwrap());
    let audio = store.audio_path(&before).unwrap();
    let mut stems = std::collections::BTreeMap::new();
    for id in chart_engine::stem_review::StemId::ALL {
        let path = root.join(format!("handoff-{}.wav", id.as_str()));
        fs::copy(&audio, &path).unwrap();
        stems.insert(id, path);
    }
    request.existing_stems = (instruments.len() == 1).then(|| {
        chart_engine::stem_review::StemManifest::build(
            &audio,
            stems,
            chart_engine::stem_review::AlignmentEvidence {
                method: "synthetic handoff regression".into(),
                details: "Identical silent test channels; no inference claim".into(),
            },
        )
        .unwrap()
    });
    let job = parts::create_project_job(request, &root.join("folder-job"), store, &before).unwrap();
    let generated = Store::create(
        &store.root.join("source"),
        None,
        &generation::project_path(&job),
    )
    .unwrap();
    generated
        .apply(Edit {
            expected_revision: 0,
            actor: human(),
            action: Action::SetVocals {
                document: chart_engine::vocals::VocalChart {
                    version: 1,
                    resolution: before.timing(store).unwrap().resolution,
                    source: "Synthetic generation handoff regression; not an inference test".into(),
                    phrases: vec![chart_engine::vocals::VocalPhrase {
                        start: 0,
                        end: 1920,
                        player: 1,
                    }],
                    notes: vec![chart_engine::vocals::VocalNote {
                        start: 480,
                        end: 960,
                        tone: chart_engine::vocals::VocalTone::Pitched { midi: 60 },
                        lyric: chart_engine::vocals::Syllable::Missing,
                        midi: None,
                    }],
                },
            },
        })
        .unwrap();
    if instruments.contains(&"guitar") {
        generated
            .apply(Edit {
                expected_revision: generated.load().unwrap().revision,
                actor: human(),
                action: Action::AddNote {
                    track: "ExpertSingle".into(),
                    tick: 1440,
                    lane: 2,
                    sustain: 0,
                    cymbal: false,
                },
            })
            .unwrap();
    }
    let mut status = generation::status(&job).unwrap();
    status.state = generation::State::Complete;
    generation::write_json(&job.join("job.json"), &status).unwrap();
    job
}

#[test]
fn whole_vocal_generation_updates_the_existing_project_with_undo_and_unchanged_source() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let before = store.load().unwrap();
    let chart = fs::read(store.root.join("source/notes.chart")).unwrap();
    let audio = fs::read(store.audio_path(&before).unwrap()).unwrap();
    let job = completed_project_job(root.path(), &store, &["vocals"]);
    store
        .apply(Edit {
            expected_revision: before.revision,
            actor: human(),
            action: Action::ApplyGeneratedPart { job },
        })
        .unwrap();
    let after = Store::new(&store.root).load().unwrap();
    assert_eq!(after.revision, before.revision + 1);
    assert_eq!(after.content.vocals.as_ref().unwrap().notes.len(), 1);
    assert_eq!(after.undo.len(), before.undo.len() + 1);
    assert_eq!(
        serde_json::to_value(&after.content.tracks).unwrap(),
        serde_json::to_value(&before.content.tracks).unwrap()
    );
    assert_eq!(
        fs::read(store.root.join("source/notes.chart")).unwrap(),
        chart
    );
    assert_eq!(fs::read(store.audio_path(&after).unwrap()).unwrap(), audio);
    let archive = fs::read_dir(store.root.join(".yace/generation"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(archive.join("request.json").is_file());
    store
        .apply(Edit {
            expected_revision: after.revision,
            actor: human(),
            action: Action::Undo,
        })
        .unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(before.content).unwrap()
    );
}

#[test]
fn whole_generation_rejects_intervening_edits_without_overwriting_the_project() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let job = completed_project_job(root.path(), &store, &["vocals"]);
    let before = store.load().unwrap();
    let mut authored = Store::new(&generation::project_path(&job))
        .load()
        .unwrap()
        .content
        .vocals
        .unwrap();
    authored.source = "Authored while generation was running".into();
    authored.notes[0].tone = chart_engine::vocals::VocalTone::Pitched { midi: 62 };
    store
        .apply(Edit {
            expected_revision: before.revision,
            actor: human(),
            action: Action::SetVocals { document: authored },
        })
        .unwrap();
    let edited = store.load().unwrap();
    assert!(
        store
            .apply(Edit {
                expected_revision: edited.revision,
                actor: human(),
                action: Action::ApplyGeneratedPart { job: job.clone() }
            })
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(store.load().unwrap()).unwrap(),
        serde_json::to_value(edited).unwrap()
    );
    assert!(
        generation::project_path(&job)
            .join("project.json")
            .is_file()
    );
}

#[test]
#[cfg(feature = "separation")]
fn whole_multi_part_generation_is_one_undoable_edit_in_the_original_project() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let before = store.load().unwrap();
    let job = completed_project_job(root.path(), &store, &["guitar", "vocals"]);
    store
        .apply(Edit {
            expected_revision: before.revision,
            actor: human(),
            action: Action::ApplyGeneratedPart { job },
        })
        .unwrap();
    let after = store.load().unwrap();
    assert_eq!(after.revision, before.revision + 1);
    assert_eq!(after.undo.len(), before.undo.len() + 1);
    assert_eq!(
        after.content.tracks["ExpertSingle"].len(),
        before.content.tracks["ExpertSingle"].len() + 1
    );
    assert_eq!(after.content.vocals.as_ref().unwrap().notes.len(), 1);
    store
        .apply(Edit {
            expected_revision: after.revision,
            actor: human(),
            action: Action::Undo,
        })
        .unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(before.content).unwrap()
    );
}

#[test]
fn review_totals_keep_double_kick_open_and_pro_keys_notes_and_ignore_modifiers() {
    use chart_engine::{editor::EditorNote, reanalysis};
    let root = tempfile::tempdir().unwrap();
    let base = fixture(root.path()).load().unwrap();
    for (track, playable, modifiers) in [
        ("ExpertDrums", vec![0, 4, 32], vec![7, 34, 40, 66, 67, 68]),
        ("HardSingle", vec![0, 7], vec![5, 6]),
        ("ExpertDoubleBass", vec![4, 7], vec![5, 6]),
        ("ExpertKeyboard", vec![0, 4], vec![5, 7]),
        ("ExpertProKeys", vec![48, 72], vec![0, 47, 73, 116]),
    ] {
        let count = playable.len();
        let note = |id: &str, tick, lane| EditorNote {
            id: id.into(),
            midi: None,
            note: chart_engine::chart::Note {
                tick,
                lane,
                sustain: 0,
            },
        };
        let mut before = base.clone();
        before.content.tracks.insert(
            track.into(),
            playable
                .iter()
                .enumerate()
                .map(|(index, &lane)| note("old", index as u64 * 480, lane))
                .chain(modifiers.iter().map(|&lane| note("old marker", 0, lane)))
                .collect(),
        );
        let mut after = before.clone();
        // Regenerated IDs and a completely different set of modifier events
        // must not report extra musical notes or make the totals disagree.
        after.content.tracks.insert(
            track.into(),
            playable
                .iter()
                .enumerate()
                .map(|(index, &lane)| note("new", index as u64 * 480, lane))
                .chain(modifiers.iter().map(|&lane| note("new marker", 960, lane)))
                .collect(),
        );
        let result = reanalysis::changes(&before, &after, &[track.into()]).remove(0);
        assert_eq!(
            (result.before, result.after, result.added, result.removed),
            (count, count, 0, 0),
            "{track}"
        );
        let notes = after.content.tracks.get_mut(track).unwrap();
        notes.remove(0);
        // A duplicate playable event still counts, even though its ID differs.
        notes.push(note("duplicate", 480, playable[1]));
        let result = reanalysis::changes(&before, &after, &[track.into()]).remove(0);
        assert_eq!(
            (result.before, result.after, result.added, result.removed),
            (count, count, 1, 1),
            "{track}"
        );
    }
}

#[test]
fn section_reanalysis_previews_without_writing_and_undo_redo_restore_the_complete_edit() {
    use chart_engine::{editor::TimeRange, reanalysis};
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let mut project = store.load().unwrap();
    for tick in [0, 2400] {
        store
            .apply(Edit {
                expected_revision: project.revision,
                actor: human(),
                action: Action::AddNote {
                    track: "ExpertDrums".into(),
                    tick,
                    lane: 2,
                    sustain: 0,
                    cymbal: false,
                },
            })
            .unwrap();
        project = store.load().unwrap();
    }
    let timing = project.timing(&store).unwrap();
    let center = timing.seconds(480);
    let range = TimeRange {
        start: center - 0.04,
        end: center + 0.04,
    };
    let target = parts::Target::capture(&store, &project, "ExpertDrums")
        .unwrap()
        .with_range(&project, Some(range))
        .unwrap();
    let job = parts::create_job(
        request(&store, "drums"),
        &root.path().join("section"),
        &target,
    )
    .unwrap();
    generation::write_json(
        &job.join("events.json"),
        &Transcription {
            drums: vec![
                DrumHit {
                    time: center + 0.01,
                    voice: Voice::Kick,
                    confidence: 0.9,
                },
                DrumHit {
                    time: 2.5,
                    voice: Voice::Snare,
                    confidence: 0.9,
                },
            ],
            ..Default::default()
        },
    )
    .unwrap();
    generation::run(&job).unwrap();
    let prepared = parts::Prepared::load(&store, &project, &job).unwrap();
    let preview = prepared.preview(&project).unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(&project.content).unwrap()
    );
    for old in &project.content.tracks["ExpertDrums"] {
        if !reanalysis::contains(range, timing.seconds(old.note.tick)) {
            assert!(preview.content.tracks["ExpertDrums"].contains(old));
        }
    }
    let summary = reanalysis::changes(&project, &preview, &["ExpertDrums".into()]);
    assert!(summary[0].added > 0 && summary[0].removed > 0);
    store
        .apply(Edit {
            expected_revision: project.revision,
            actor: human(),
            action: Action::ApplyGeneratedPart { job },
        })
        .unwrap();
    let applied = store.load().unwrap();
    assert_eq!(
        serde_json::to_value(&applied.content).unwrap(),
        serde_json::to_value(&preview.content).unwrap()
    );
    store
        .apply(Edit {
            expected_revision: applied.revision,
            actor: human(),
            action: Action::Undo,
        })
        .unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(&project.content).unwrap()
    );
    store
        .apply(Edit {
            expected_revision: store.load().unwrap().revision,
            actor: human(),
            action: Action::Redo,
        })
        .unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(&preview.content).unwrap()
    );
}

#[test]
fn affected_difficulty_and_source_changes_invalidate_a_part_target() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let project = store.load().unwrap();
    let target = parts::Target::capture(&store, &project, "ExpertSingle").unwrap();
    let mut changed = project.clone();
    chart_engine::reanalysis::replace_source(
        &mut changed.content,
        "ExpertSingle",
        chart_engine::editor::TimeRange {
            start: 0.,
            end: changed.duration,
        },
        chart_engine::mapping::GuitarSource::Other,
    );
    assert!(target.validate(&store, &changed).is_err());
    let mut legacy = target.clone();
    legacy.version = 1;
    legacy.affected_fingerprints.clear();
    legacy.validate(&store, &project).unwrap();
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
    assert!(
        !chart_engine::song_layout::path(&job.join("song"), "chart-engine-lyrics.json").exists()
    );
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
fn managed_song_part_jobs_keep_relative_targets_when_the_song_folder_moves() {
    use chart_engine::stem_review::{AlignmentEvidence, StemId, StemManifest};
    let root = tempfile::tempdir().unwrap();
    let initial = fixture(root.path());
    let song = root.path().join("Visible Song");
    fs::create_dir_all(song.join(".yace")).unwrap();
    let project = song.join(".yace/project");
    fs::rename(initial.root, &project).unwrap();
    let store = Store::new(&project.canonicalize().unwrap());
    let target = parts::Target::capture(&store, &store.load().unwrap(), "ExpertDrums").unwrap();
    let jobs = chart_engine::song_layout::project_data(&store.root).join("jobs/parts");
    fs::create_dir_all(&jobs).unwrap();
    let job = parts::create_job(request(&store, "drums"), &jobs.join("draft"), &target).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(job.join("editor-target.json")).unwrap()).unwrap();
    assert_eq!(saved["project"], "../../../project");
    assert!(generation::run(&job).is_err()); // Preserve the imported recording before model failure.
    let cache =
        chart_engine::song_layout::project_data(&store.root).join("cache/separation/synthetic");
    fs::create_dir_all(&cache).unwrap();
    let source = job.join("input/source.wav");
    let mut files = std::collections::BTreeMap::new();
    for id in StemId::ALL {
        let path = cache.join(format!("{}.wav", id.as_str()));
        fs::copy(&source, &path).unwrap();
        files.insert(id, path);
    }
    let mut manifest = StemManifest::build(
        &source,
        files,
        AlignmentEvidence {
            method: "synthetic_part_channels".into(),
            details:
                "Known silent stems share the fixture clock and exercise unfinished-job recovery."
                    .into(),
        },
    )
    .unwrap();
    for asset in &mut manifest.stems {
        asset.path = std::path::PathBuf::from("../../../cache/separation/synthetic")
            .join(format!("{}.wav", asset.id.as_str()));
    }
    generation::write_json(&job.join("isolation.json"), &manifest).unwrap();
    generation::write_json(
        &job.join("events.json"),
        &Transcription {
            drums: vec![DrumHit {
                time: 1.0,
                voice: Voice::Snare,
                confidence: 0.9,
            }],
            isolation: Some(manifest),
            ..Default::default()
        },
    )
    .unwrap();
    let moved = root.path().join("Moved Song");
    fs::rename(song, &moved).unwrap();
    fs::remove_dir_all(root.path().join("song")).unwrap();
    let store = Store::new(&moved.join(".yace/project"));
    let job = moved.join(".yace/jobs/parts/draft");
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
    let project = store.load().unwrap();
    project
        .content
        .stems
        .as_ref()
        .unwrap()
        .load(&store.audio_path(&project).unwrap())
        .unwrap();
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
        assert!(
            asset
                .path
                .starts_with(store.root.join("source/.yace/audio/stems"))
        );
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
    let receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(chart_engine::song_layout::path(&output, "yace-audio.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["mode"], "instrument_stems");
    assert!(output.join("drums.wav").is_file());
    assert!(output.join("keys.wav").is_file());
}
