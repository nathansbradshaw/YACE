use chart_engine::{
    models,
    stem_review::{AlignmentEvidence, StemId, StemManifest},
};
use std::{collections::BTreeMap, fs, process::Command};

fn cli(args: &[&str]) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_chart-engine"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn ai_attachment_solo_and_undo_use_the_same_editor_request_protocol() {
    let root = tempfile::tempdir().unwrap();
    let song = root.path().join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"),"[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[ExpertDrums]\n{\n 440 = N 1 0\n}\n").unwrap();
    let mut original = vec![0.0; 44100];
    original[1000] = 0.3;
    let source = song.join("song.wav");
    let drums = root.path().join("drums.wav");
    models::write_audio(&source, &original, &original).unwrap();
    models::write_audio(&drums, &original, &original).unwrap();
    let source_hash = models::hash_file(&source).unwrap();
    let manifest = StemManifest::build(
        &source,
        BTreeMap::from([(StemId::Drums, drums)]),
        AlignmentEvidence {
            method: "synthetic_impulses".into(),
            details: "An explicitly aligned test fixture".into(),
        },
    )
    .unwrap();
    let manifest_path = root.path().join("stems.json");
    manifest.write_relative(&manifest_path).unwrap();
    let project = root.path().join("project");
    let project_arg = project.to_str().unwrap();
    let initial = cli(&[
        "editor",
        "create",
        song.to_str().unwrap(),
        "--output",
        project_arg,
    ]);
    let chart_hash = initial["chart_sha256"].clone();
    let request = root.path().join("request.json");
    for (revision,action) in [
        serde_json::json!({"type":"attach_stems","manifest":manifest_path}),
        serde_json::json!({"type":"set_stem_mix","mix":{"mode":"stems","channels":{"drums":{"gain":0.5,"mute":false,"solo":true}}}}),
        serde_json::json!({"type":"undo"}),
        serde_json::json!({"type":"redo"}),
    ].into_iter().enumerate() {
        fs::write(&request,serde_json::to_vec(&serde_json::json!({"expected_revision":revision,"actor":{"kind":"ai","name":"Stem CLI regression"},"action":action})).unwrap()).unwrap();
        let result=cli(&["editor","apply",project_arg,"--request",request.to_str().unwrap()]);
        assert_eq!(result["revision"],revision+1);
    }
    let shown = cli(&["editor", "show", project_arg]);
    assert_eq!(shown["project"]["content"]["stem_mix"]["mode"], "stems");
    assert_eq!(
        shown["project"]["content"]["stem_mix"]["channels"]["drums"]["gain"],
        0.5
    );
    assert_eq!(
        shown["project"]["content"]["stems"]["source_audio_sha256"],
        source_hash
    );
    assert_eq!(
        shown["project"]["content"]["decisions"],
        serde_json::json!([])
    );
    assert_eq!(shown["chart_sha256"], chart_hash);
    assert_eq!(models::hash_file(&source).unwrap(), source_hash);
}
