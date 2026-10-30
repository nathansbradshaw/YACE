use chart_engine::{editor::Project, models};
use std::{fs, process::Command};

fn run(args: &[&str]) -> serde_json::Value {
    let result = Command::new(env!("CARGO_BIN_EXE_chart-engine"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
#[test]
fn automation_seeks_edits_reviews_and_exports_through_the_same_durable_commands() {
    let root = tempfile::tempdir().unwrap();
    let song = root.path().join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"),"[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[ExpertDrums]\n{\n 440 = N 1 0\n}\n").unwrap();
    models::write_audio(
        &song.join("song.wav"),
        &vec![0.0; 44100 * 2],
        &vec![0.0; 44100 * 2],
    )
    .unwrap();
    let project = root.path().join("project");
    let project_arg = project.to_str().unwrap();
    let created = run(&[
        "editor",
        "create",
        song.to_str().unwrap(),
        "--output",
        project_arg,
    ]);
    let id = created["project"]["content"]["tracks"]["ExpertDrums"][0]["id"]
        .as_str()
        .unwrap();
    let region = created["project"]["review"]["regions"][0]["id"]
        .as_str()
        .unwrap();
    let request = root.path().join("command.json");
    for (revision,action) in [
        serde_json::json!({"type":"select_region","region_id":region}),
        serde_json::json!({"type":"update_note","note_id":id,"tick":480,"lane":2,"sustain":0,"cymbal":true}),
        serde_json::json!({"type":"decide","region_id":region,"track":"ExpertDrums","status":"resolved","comment":"Aligned attack and assigned hi-hat in this synthetic fixture"}),
    ].into_iter().enumerate(){
        fs::write(&request,serde_json::to_vec(&serde_json::json!({"expected_revision":revision,"actor":{"kind":"ai","name":"CLI integration"},"action":action})).unwrap()).unwrap();
        let result=run(&["editor","apply",project_arg,"--request",request.to_str().unwrap()]);assert_eq!(result["revision"],revision+1);
    }
    let state = run(&["editor", "show", project_arg]);
    assert_eq!(
        state["current_decisions"][0]["decision"]["status"],
        "resolved"
    );
    assert!(
        state["live_transport"].is_null(),
        "closed GUI must not claim audio output is active"
    );
    let output = root.path().join("export");
    fs::write(&request,serde_json::to_vec(&serde_json::json!({"expected_revision":3,"actor":{"kind":"ai","name":"CLI integration"},"action":{"type":"export","output":output}})).unwrap()).unwrap();
    run(&[
        "editor",
        "apply",
        project_arg,
        "--request",
        request.to_str().unwrap(),
    ]);
    assert!(output.join("notes.chart").exists());
    let restored: Project =
        serde_json::from_slice(&fs::read(project.join("project.json")).unwrap()).unwrap();
    assert_eq!(restored.content.decisions.len(), 1);
}
