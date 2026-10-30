use chart_engine::{
    editor::{Action, Actor, ActorKind, Content, Request, Store},
    models,
    stem_review::{
        AlignmentEvidence, FrameCursor, LoadedStems, MixMode, SeparationIdentity, StemChannel,
        StemId, StemManifest, StemMix,
    },
};
use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

fn mix(stems: &[(StemId, StemChannel)]) -> StemMix {
    StemMix {
        mode: MixMode::Stems,
        channels: stems.iter().copied().collect(),
    }
}
fn impulses() -> LoadedStems {
    let drums = vec![0.3, 0.1, 0.0, 0.0, 0.0, 0.0, -0.2, 0.1];
    let bass = vec![0.0, 0.0, 0.2, -0.1, 0.0, 0.0, 0.1, -0.1];
    let original = drums.iter().zip(&bass).map(|(a, b)| a + b).collect();
    LoadedStems::from_pcm(
        44100,
        Arc::new(original),
        BTreeMap::from([
            (StemId::Drums, Arc::new(drums)),
            (StemId::Bass, Arc::new(bass)),
        ]),
    )
    .unwrap()
}

#[test]
fn original_and_reconstruction_are_exclusive_and_solo_retains_impulse_timing() {
    let loaded = impulses();
    assert_eq!(
        loaded.render(&StemMix::default()).unwrap(),
        *loaded.original
    );
    assert_eq!(loaded.render(&mix(&[])).unwrap(), *loaded.original);
    let solo = mix(&[(
        StemId::Drums,
        StemChannel {
            solo: true,
            ..Default::default()
        },
    )]);
    assert_eq!(loaded.render(&solo).unwrap(), *loaded.stems[&StemId::Drums]);
    let muted_solo = mix(&[(
        StemId::Drums,
        StemChannel {
            solo: true,
            mute: true,
            ..Default::default()
        },
    )]);
    assert_eq!(loaded.render(&muted_solo).unwrap(), vec![0.0; 8]);
    let original_with_solo_settings = StemMix {
        mode: MixMode::Original,
        ..solo
    };
    assert_eq!(
        loaded.render(&original_with_solo_settings).unwrap(),
        *loaded.original
    );
}

#[test]
fn mixer_limits_the_common_stereo_peak_and_preserves_channel_relationships() {
    let loaded = LoadedStems::from_pcm(
        44100,
        Arc::new(vec![0.4, 0.1]),
        BTreeMap::from([
            (StemId::Drums, Arc::new(vec![0.8, 0.2])),
            (StemId::Bass, Arc::new(vec![0.8, 0.2])),
        ]),
    )
    .unwrap();
    assert_eq!(loaded.mix_frame(0, &mix(&[])).unwrap(), [1.0, 0.25]);
    let gain = mix(&[
        (
            StemId::Drums,
            StemChannel {
                gain: 0.5,
                ..Default::default()
            },
        ),
        (
            StemId::Bass,
            StemChannel {
                mute: true,
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(loaded.mix_frame(0, &gain).unwrap(), [0.4, 0.1]);
    let prepared = gain.prepare(loaded.stems.keys().copied()).unwrap();
    assert_eq!(loaded.frame_with_mix(0, &prepared), [0.4, 0.1]);
    assert_eq!(loaded.frame_with_mix(1, &prepared), [0.0, 0.0]);
}

#[test]
fn shared_frame_cursor_seeks_and_loops_every_stem_on_identical_boundaries() {
    let loaded = impulses();
    let prepared = mix(&[]).prepare(loaded.stems.keys().copied()).unwrap();
    let mut cursor = FrameCursor::new(loaded.frame_count());
    cursor.set_loop(Some(1..4)).unwrap();
    let first: Vec<_> = (0..7)
        .map(|_| loaded.frame_with_mix(cursor.next_frame().unwrap(), &prepared))
        .collect();
    assert_eq!(
        first,
        vec![
            [0.2, -0.1],
            [0.0, 0.0],
            [-0.1, 0.0],
            [0.2, -0.1],
            [0.0, 0.0],
            [-0.1, 0.0],
            [0.2, -0.1]
        ]
    );
    assert!(cursor.set_loop(Some(2..2)).is_err());
    cursor.set_loop(None).unwrap();
    cursor.seek(3).unwrap();
    assert_eq!(cursor.next_frame(), Some(3));
    assert_eq!(cursor.next_frame(), None);
    assert!(cursor.seek(5).is_err());
}

#[test]
fn invalid_gains_missing_stems_and_nonfinite_or_misaligned_pcm_are_rejected() {
    for gain in [f32::NAN, f32::INFINITY, -0.1, 2.1] {
        assert!(
            mix(&[(
                StemId::Bass,
                StemChannel {
                    gain,
                    ..Default::default()
                }
            )])
            .prepare([StemId::Bass])
            .is_err()
        );
    }
    assert!(mix(&[]).prepare([]).is_err());
    assert!(
        mix(&[(StemId::Guitar, StemChannel::default())])
            .prepare([StemId::Drums])
            .is_err()
    );
    assert!(LoadedStems::from_pcm(44100, Arc::new(vec![f32::NAN, 0.0]), BTreeMap::new()).is_err());
    assert!(
        LoadedStems::from_pcm(
            44100,
            Arc::new(vec![0.0, 0.0]),
            BTreeMap::from([(StemId::Drums, Arc::new(vec![0.0; 4]))])
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<StemMix>(r#"{"mode":"stems","channels":{},"invented":true}"#)
            .is_err()
    );
}

fn write_fixture(root: &Path) -> (std::path::PathBuf, std::path::PathBuf, StemManifest) {
    let song = root.join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"),"[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[ExpertDrums]\n{\n 440 = N 1 0\n}\n").unwrap();
    let mut drums = vec![0.0; 44100];
    drums[4400] = 0.4;
    let mut bass = vec![0.0; 44100];
    bass[22000] = 0.2;
    let original: Vec<_> = drums.iter().zip(&bass).map(|(a, b)| a + b).collect();
    let source = song.join("song.wav");
    let drum_path = root.join("drums.wav");
    let bass_path = root.join("bass.wav");
    models::write_audio(&source, &original, &original).unwrap();
    models::write_audio(&drum_path, &drums, &drums).unwrap();
    models::write_audio(&bass_path, &bass, &bass).unwrap();
    let manifest = StemManifest::build(&source,BTreeMap::from([(StemId::Drums,drum_path),(StemId::Bass,bass_path)]),AlignmentEvidence{method:"synthetic_impulses".into(),details:"Known exact sample placement in this test fixture; this is not evidence about separated music".into()}).unwrap();
    (song, source, manifest)
}

#[test]
fn manifests_bind_real_hashes_exact_frames_and_relative_paths_without_claiming_alignment() {
    let root = tempfile::tempdir().unwrap();
    let (_song, source, mut manifest) = write_fixture(root.path());
    assert_eq!(manifest.frames, 44100);
    let mut relative = manifest.clone();
    for asset in &mut relative.stems {
        asset.path = asset.path.file_name().unwrap().into();
    }
    let path = root.path().join("manifest.json");
    fs::write(&path, serde_json::to_vec(&relative).unwrap()).unwrap();
    assert_eq!(StemManifest::read(&path).unwrap(), manifest);
    let relative_output = root.path().join("relative.json");
    manifest.write_relative(&relative_output).unwrap();
    assert_eq!(StemManifest::read(&relative_output).unwrap(), manifest);
    assert!(manifest.write_relative(&relative_output).is_err());
    let loaded = manifest.load(&source).unwrap();
    assert_eq!(loaded.render(&mix(&[])).unwrap(), *loaded.original);
    manifest.frames -= 1;
    assert!(manifest.load(&source).is_err());
    manifest.frames += 1;
    let mut wrong = manifest.clone();
    wrong.source_audio_sha256 = "a".repeat(64);
    assert!(wrong.load(&source).is_err());
    wrong = manifest.clone();
    wrong.alignment.details.clear();
    assert!(wrong.load(&source).is_err());
    wrong = manifest.clone();
    wrong.stems.push(wrong.stems[0].clone());
    assert!(wrong.load(&source).is_err());
    fs::write(&manifest.stems[0].path, b"changed recording").unwrap();
    assert!(manifest.load(&source).is_err());
}

#[test]
fn separation_cache_identity_changes_with_source_model_backend_and_parameters() {
    let source = "a".repeat(64);
    let mut identity = SeparationIdentity {
        model_id: "test-six-stem".into(),
        model_sha256: "b".repeat(64),
        backend: "native-rust-test-v1".into(),
        parameters: BTreeMap::from([("overlap".into(), "0.25".into())]),
        cache_key: String::new(),
    };
    identity.cache_key = identity.calculated_cache_key(&source).unwrap();
    identity.validate(&source).unwrap();
    assert!(identity.validate(&"c".repeat(64)).is_err());
    let mut changed = identity.clone();
    changed.model_sha256 = "d".repeat(64);
    assert!(changed.validate(&source).is_err());
    changed = identity.clone();
    changed.backend.push_str("-changed");
    assert!(changed.validate(&source).is_err());
    changed = identity;
    changed.parameters.insert("overlap".into(), "0.5".into());
    assert!(changed.validate(&source).is_err());
}

fn apply(store: &Store, action: Action) -> anyhow::Result<chart_engine::editor::Applied> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Ai,
            name: "Stem review regression".into(),
        },
        action,
    })
}
#[test]
fn primary_attachment_never_retargets_keyboard_analysis_and_both_export() {
    let root = tempfile::tempdir().unwrap();
    let (song, _, primary) = write_fixture(root.path());
    let chart = song.join("notes.chart");
    let mut text = fs::read_to_string(&chart).unwrap();
    text.push_str("\n[ExpertKeyboard]\n{\n 480 = N 2 120\n}\n");
    fs::write(chart, text).unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    let save = |name: &str, manifest: &StemManifest| {
        let path = root.path().join(format!("{name}.json"));
        fs::write(&path, serde_json::to_vec(manifest).unwrap()).unwrap();
        path
    };
    apply(
        &store,
        Action::AttachStems {
            manifest: save("primary", &primary),
        },
    )
    .unwrap();
    let mut keyboard = primary.clone();
    keyboard.alignment.method = "separate keyboard source".into();
    apply(
        &store,
        Action::AttachKeyboardStems {
            manifest: save("keyboard", &keyboard),
        },
    )
    .unwrap();
    apply(
        &store,
        Action::SelectTrack {
            track: "ExpertKeyboard".into(),
        },
    )
    .unwrap();
    let mut replacement = primary.clone();
    replacement.alignment.method = "replacement primary vocals".into();
    apply(
        &store,
        Action::AttachStems {
            manifest: save("replacement", &replacement),
        },
    )
    .unwrap();
    let project = store.load().unwrap();
    assert_eq!(project.content.stems.as_ref(), Some(&replacement));
    assert_eq!(project.content.keyboard_stems.as_ref(), Some(&keyboard));
    let export = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: export.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        StemManifest::read(&export.join("chart-engine-keyboard-isolation.json")).unwrap(),
        keyboard
    );
    assert_eq!(
        StemManifest::read(&export.join("chart-engine-isolation.json")).unwrap(),
        replacement
    );
    apply(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().content.stems.as_ref(), Some(&primary));
    assert_eq!(
        store.load().unwrap().content.keyboard_stems.as_ref(),
        Some(&keyboard)
    );
}
#[test]
fn attachment_and_mixer_are_durable_undoable_and_never_approve_or_modify_notes() {
    let root = tempfile::tempdir().unwrap();
    let (song, source, manifest) = write_fixture(root.path());
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    let chart = store.load().unwrap().chart_hash(&store).unwrap();
    let original_hash = models::hash_file(&source).unwrap();
    let path = root.path().join("manifest.json");
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    apply(
        &store,
        Action::AttachStems {
            manifest: path.clone(),
        },
    )
    .unwrap();
    let solo = mix(&[(
        StemId::Drums,
        StemChannel {
            solo: true,
            ..Default::default()
        },
    )]);
    apply(&store, Action::SetStemMix { mix: solo.clone() }).unwrap();
    let project = store.load().unwrap();
    assert_eq!(project.content.stem_mix, solo);
    assert_eq!(project.content.stems, Some(manifest));
    assert!(project.content.decisions.is_empty());
    assert_eq!(project.chart_hash(&store).unwrap(), chart);
    assert_eq!(
        store.inspect().unwrap()["project"]["content"]["stem_mix"]["channels"]["drums"]["solo"],
        true
    );
    apply(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().content.stem_mix, StemMix::default());
    apply(&store, Action::Undo).unwrap();
    assert!(store.load().unwrap().content.stems.is_none());
    apply(&store, Action::Redo).unwrap();
    assert!(store.load().unwrap().content.stems.is_some());
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        apply(
            &store,
            Action::SetStemMix {
                mix: mix(&[(StemId::Piano, StemChannel::default())])
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    fs::write(&source, b"changed original outside project").unwrap();
    // Project source is a snapshot; changing the external original cannot rebind it.
    assert!(
        store
            .load()
            .unwrap()
            .content
            .stems
            .unwrap()
            .load(&source)
            .is_err()
    );
    assert_ne!(models::hash_file(&source).unwrap(), original_hash);
    let mut legacy = serde_json::to_value(&store.load().unwrap().content).unwrap();
    legacy.as_object_mut().unwrap().remove("stems");
    legacy.as_object_mut().unwrap().remove("stem_mix");
    let legacy: Content = serde_json::from_value(legacy).unwrap();
    assert!(legacy.stems.is_none());
    assert_eq!(legacy.stem_mix, StemMix::default());
}

#[test]
fn generated_attachments_open_and_export_current_stems_without_blocking_original_fallback() {
    let root = tempfile::tempdir().unwrap();
    let (song, source, manifest) = write_fixture(root.path());
    let sidecar = song.join("chart-engine-isolation.json");
    fs::write(&sidecar, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let store = Store::create(&song, None, &root.path().join("auto-project")).unwrap();
    assert_eq!(store.load().unwrap().content.stems, Some(manifest.clone()));
    assert_eq!(store.load().unwrap().content.stem_mix, StemMix::default());
    let mut current = manifest.clone();
    current.stems.retain(|asset| asset.id == StemId::Drums);
    let current_path = root.path().join("current-stems.json");
    fs::write(&current_path, serde_json::to_vec(&current).unwrap()).unwrap();
    apply(
        &store,
        Action::AttachStems {
            manifest: current_path,
        },
    )
    .unwrap();
    apply(
        &store,
        Action::SetStemMix {
            mix: mix(&[(
                StemId::Drums,
                StemChannel {
                    solo: true,
                    ..Default::default()
                },
            )]),
        },
    )
    .unwrap();
    let original_hash = models::hash_file(&source).unwrap();
    let exported = root.path().join("exported");
    apply(
        &store,
        Action::Export {
            output: exported.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        StemManifest::read(&exported.join("chart-engine-isolation.json")).unwrap(),
        current
    );
    let archived: StemManifest = serde_json::from_slice(
        &fs::read(exported.join("editor-source/chart-engine-isolation.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(archived, manifest);
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(exported.join("editor-review.json")).unwrap()).unwrap();
    assert_eq!(report["stem_mix_requested"]["mode"], "stems");
    assert_eq!(
        models::hash_file(&exported.join("song.wav")).unwrap(),
        original_hash
    );
    assert_eq!(models::hash_file(&source).unwrap(), original_hash);
    let reopened = Store::create(&exported, None, &root.path().join("roundtrip")).unwrap();
    assert_eq!(
        reopened.load().unwrap().content.stems,
        Some(current.clone())
    );
    assert_eq!(
        reopened.load().unwrap().content.stem_mix,
        StemMix::default()
    );
    fs::remove_file(&current.stems[0].path).unwrap();
    let missing = Store::create(&exported, None, &root.path().join("missing-assets")).unwrap();
    assert!(missing.load().unwrap().content.stems.is_some());
    assert!(
        current
            .load(&missing.audio_path(&missing.load().unwrap()).unwrap())
            .is_err()
    );
    let mut stale = manifest;
    stale.source_audio_sha256 = "a".repeat(64);
    fs::write(&sidecar, serde_json::to_vec(&stale).unwrap()).unwrap();
    let mismatched = Store::create(&song, None, &root.path().join("mismatched"))
        .unwrap()
        .load()
        .unwrap();
    assert!(mismatched.content.stems.is_none());
    assert!(
        mismatched
            .review
            .coverage_limits
            .iter()
            .any(|notice| notice.starts_with("Stem attachment skipped:"))
    );
    fs::write(&sidecar, b"invalid known isolation JSON").unwrap();
    assert!(
        Store::create(&song, None, &root.path().join("malformed"))
            .unwrap()
            .load()
            .unwrap()
            .content
            .stems
            .is_none()
    );
    fs::remove_file(&sidecar).unwrap();
    fs::write(
        song.join("chart-engine-transcription.json"),
        br#"{"unrelated_legacy_metadata":true}"#,
    )
    .unwrap();
    let legacy = Store::create(&song, None, &root.path().join("legacy-sidecar"))
        .unwrap()
        .load()
        .unwrap();
    assert!(legacy.content.stems.is_none());
    assert!(
        !legacy
            .review
            .coverage_limits
            .iter()
            .any(|notice| notice.starts_with("Stem attachment skipped:"))
    );
    fs::write(
        song.join("chart-engine-transcription.json"),
        serde_json::to_vec(&serde_json::json!({"other_events":[],"isolation":current})).unwrap(),
    )
    .unwrap();
    assert!(
        Store::create(&song, None, &root.path().join("embedded-sidecar"))
            .unwrap()
            .load()
            .unwrap()
            .content
            .stems
            .is_some()
    );
}
