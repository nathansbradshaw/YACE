use chart_engine::{
    editor::{Action, Actor, ActorKind, Request, Store},
    models,
    stem_review::{AlignmentEvidence, MixMode, StemAsset, StemId, StemManifest},
};
use std::{fs, path::Path};

mod fixture {
    include!("support/assistant_project.rs");
}

fn project(root: &Path, sound: &str) -> Store {
    fs::create_dir_all(root).unwrap();
    let store = fixture::fixture(root);
    let mut doc = store.load().unwrap();
    let mut keys = doc.content.tracks["ExpertSingle"].clone();
    for note in &mut keys {
        note.id = format!("keys-{}", note.id);
    }
    doc.content.tracks.insert("ExpertKeyboard".into(), keys);
    doc.content.track_origins.insert(
        "ExpertKeyboard".into(),
        chart_engine::parts::Origin::Authored,
    );
    doc.content.stems = Some(StemManifest {
        version: 1,
        source_audio_sha256: doc.audio_sha256.clone(),
        sample_rate: 44100,
        channels: 2,
        frames: 176400,
        alignment: AlignmentEvidence {
            method: "fixture".into(),
            details: "routing fixture".into(),
        },
        stems: vec![StemAsset {
            id: StemId::Piano,
            path: root.join("source/song.ogg"),
            sha256: doc.audio_sha256.clone(),
        }],
        generation: None,
        conventional: None,
    });
    doc.content.stem_mix.mode = MixMode::Stems;
    let path = root.join("source/chart-engine-transcription.json");
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "keyboard_detection": {"sound":sound, "source_audio_sha256":doc.audio_sha256}
        }))
        .unwrap(),
    )
    .unwrap();
    doc.review
        .input_sha256
        .insert("transcription".into(), models::hash_file(&path).unwrap());
    fs::write(root.join("project.json"), serde_json::to_vec(&doc).unwrap()).unwrap();
    store
}

#[test]
fn selecting_synth_uses_original_without_changing_notes_or_other_part_mixes() {
    let root = tempfile::tempdir().unwrap();
    let store = project(root.path(), "synth");
    let before = store.load().unwrap();
    assert!(!store.synth_reference(&before));
    store
        .apply(Request {
            expected_revision: before.revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "test".into(),
            },
            action: Action::SelectTrack {
                track: "ExpertKeyboard".into(),
            },
        })
        .unwrap();
    let after = store.load().unwrap();
    assert!(store.synth_reference(&after));
    assert_eq!(after.content.stem_mix.mode, MixMode::Original);
    assert_eq!(before.content.tracks, after.content.tracks);
    let mut guitar = after.clone();
    guitar.selected_track = "ExpertSingle".into();
    assert!(!store.synth_reference(&guitar));
}

#[test]
fn piano_unknown_or_changed_evidence_does_not_claim_synth_reference() {
    for sound in ["piano", "synth"] {
        let root = tempfile::tempdir().unwrap();
        let store = project(root.path(), sound);
        let mut doc = store.load().unwrap();
        doc.selected_track = "ExpertKeyboard".into();
        assert_eq!(store.synth_reference(&doc), sound == "synth");
        doc.review.input_sha256.remove("transcription");
        assert!(!store.synth_reference(&doc));
        let store = project(&root.path().join("other"), "synth");
        let mut doc = store.load().unwrap();
        doc.selected_track = "ExpertKeyboard".into();
        doc.audio_sha256 = "0".repeat(64);
        assert!(!store.synth_reference(&doc));
        fs::write(
            store.root.join("source/chart-engine-transcription.json"),
            b"{}",
        )
        .unwrap();
        assert!(!store.synth_reference(&doc));
    }
}

#[test]
fn a_generated_part_uses_its_own_witness_and_rejects_changed_drafts() {
    let root = tempfile::tempdir().unwrap();
    let store = project(&root.path().join("song"), "piano");
    let job = root.path().join("job");
    let generated = project(&chart_engine::generation::project_path(&job), "synth");
    let draft = generated.load().unwrap();
    let mut doc = store.load().unwrap();
    doc.selected_track = "ExpertKeyboard".into();
    doc.content.track_origins.insert(
        doc.selected_track.clone(),
        chart_engine::parts::Origin::Generated {
            job,
            chart_sha256: draft.chart_hash(&generated).unwrap(),
        },
    );
    assert!(store.synth_reference(&doc));
    fs::write(
        generated
            .root
            .join("source/chart-engine-transcription.json"),
        b"{}",
    )
    .unwrap();
    assert!(!store.synth_reference(&doc));
}

#[test]
fn supplied_keyboard_audio_keeps_its_own_reference() {
    use chart_engine::stem_review::conventional;
    let root = tempfile::tempdir().unwrap();
    let store = project(root.path(), "synth");
    let mut doc = store.load().unwrap();
    doc.selected_track = "ExpertKeyboard".into();
    let mut manifest = StemManifest {
        version: 1,
        source_audio_sha256: doc.audio_sha256.clone(),
        sample_rate: 44100,
        channels: 2,
        frames: 176400,
        alignment: AlignmentEvidence {
            method: "fixture".into(),
            details: "routing fixture".into(),
        },
        stems: vec![],
        generation: None,
        conventional: None,
    };
    assert!(store.synth_reference(&doc));
    doc.content.keyboard_stems = Some(manifest.clone());
    assert!(!store.synth_reference(&doc));
    doc.content.keyboard_stems = None;
    manifest.conventional = Some(conventional::Clock {
        version: 1,
        headroom: conventional::Gain::try_from(1.0).unwrap(),
    });
    doc.content.stems = Some(manifest);
    assert!(!store.synth_reference(&doc));
}
