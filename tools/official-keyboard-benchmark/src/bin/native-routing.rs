//! Exercise the real running editor through its revision-checked command API.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn read(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}
struct Check {
    executable: PathBuf,
    project: PathBuf,
    output: PathBuf,
    step: usize,
    samples: Vec<Value>,
}
impl Check {
    fn action(&mut self, action: Value) -> Result<()> {
        let project = read(&self.project.join("project.json"))?;
        let request = json!({"expected_revision":project["revision"],"actor":{"kind":"ai","name":"Native keyboard routing QA"},"action":action});
        let path = self.output.join(format!("request-{:02}.json", self.step));
        self.step += 1;
        fs::write(&path, serde_json::to_vec_pretty(&request)?)?;
        let result = Command::new(&self.executable)
            .args(["editor", "apply"])
            .arg(&self.project)
            .arg("--request")
            .arg(path)
            .output()?;
        ensure!(
            result.status.success(),
            "Editor command failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(())
    }
    fn wait(&mut self, label: &str, condition: impl Fn(&Value) -> bool) -> Result<Value> {
        let start = Instant::now();
        loop {
            if let Ok(live) = read(&self.project.join("live-state.json")) {
                let fresh = live["at_ms"]
                    .as_u64()
                    .is_some_and(|t| now().saturating_sub(t as u128) < 1500);
                if fresh && condition(&live) {
                    self.samples.push(json!({"phase":label,"live":live}));
                    println!("{label}");
                    return Ok(live);
                }
            }
            ensure!(
                start.elapsed() < Duration::from_secs(30),
                "Native state timed out: {label}"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }
}
fn ready(live: &Value) -> bool {
    live["audio_output_ready"] == true && live["audition"]["preparing"] == false
}
fn playing(live: &Value) -> bool {
    ready(live) && live["playing"] == true && live["audition"]["actual"]["player_paused"] == false
}
fn identity(live: &Value) -> &Value {
    &live["audition"]["actual"]["assets_identity_sha256"]
}
fn run(check: &mut Check, initial: &Value) -> Result<()> {
    let tracks = initial["content"]["tracks"].as_object().context("tracks")?;
    ensure!(
        tracks.contains_key("ExpertSingle") && tracks.contains_key("ExpertKeyboard"),
        "Use an isolated mixed guitar/keyboard QA project"
    );
    ensure!(
        !initial["content"]["keyboard_stems"].is_null(),
        "Separate keyboard source required"
    );
    check.wait("initial-ready", ready)?;
    check.action(json!({"type":"select_track","track":"ExpertSingle"}))?;
    check.action(json!({"type":"set_loop","range":{"start":5.0,"end":25.0}}))?;
    check.action(json!({"type":"seek","seconds":5.0}))?;
    check.action(json!({"type":"playback","playing":true}))?;
    let primary = check.wait("primary-playing", playing)?;
    let primary_id = identity(&primary).clone();
    check.action(json!({"type":"select_track","track":"ExpertKeyboard"}))?;
    check.wait("keyboard-preparing", |s| s["audition"]["preparing"] == true)?;
    check.action(json!({"type":"select_track","track":"ExpertSingle"}))?;
    let restored = check.wait("cancelled-load-resumes-primary", |s| {
        playing(s) && identity(s) == &primary_id
    })?;
    let position = restored["playhead_seconds"].as_f64().context("playhead")?;
    check.wait("primary-cursor-advances-after-cancel", |s| {
        playing(s)
            && identity(s) == &primary_id
            && s["playhead_seconds"]
                .as_f64()
                .is_some_and(|p| p > position + 0.3)
    })?;
    check.action(json!({"type":"select_track","track":"ExpertKeyboard"}))?;
    let keyboard = check.wait("keyboard-source-ready", |s| {
        playing(s) && identity(s) != &primary_id
    })?;
    let keyboard_id = identity(&keyboard).clone();
    check.action(json!({"type":"set_stem_mix","mix":{"mode":"stems","channels":{"drums":{"gain":0.0,"mute":false,"solo":false},"bass":{"gain":0.0,"mute":false,"solo":false},"vocals":{"gain":0.0,"mute":false,"solo":false},"guitar":{"gain":0.0,"mute":false,"solo":false},"piano":{"gain":1.0,"mute":false,"solo":false},"other":{"gain":0.0,"mute":false,"solo":false}}}}))?;
    check.wait("actual-keyboard-solo", |s| {
        playing(s)
            && identity(s) == &keyboard_id
            && s["audition"]["actual"]["mode"] == "stems"
            && s["audition"]["actual"]["active_gains"]["piano"] == 1.0
    })?;
    if tracks.contains_key("ExpertProKeys") {
        check.action(json!({"type":"select_track","track":"ExpertProKeys"}))?;
        check.wait("pro-piano-shares-keyboard-source", |s| {
            playing(s) && identity(s) == &keyboard_id
        })?;
    }
    check.action(json!({"type":"select_track","track":"ExpertSingle"}))?;
    check.wait("primary-source-restored", |s| {
        playing(s) && identity(s) == &primary_id
    })?;
    Ok(())
}
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    ensure!(
        args.len() == 3,
        "native-routing EXECUTABLE RUNNING_QA_PROJECT NEW_OUTPUT_DIRECTORY"
    );
    let mut check = Check {
        executable: args[0].clone().into(),
        project: args[1].clone().into(),
        output: args[2].clone().into(),
        step: 0,
        samples: vec![],
    };
    fs::create_dir(&check.output).context("Use a new output directory")?;
    let initial = read(&check.project.join("project.json"))?;
    fs::write(
        check.output.join("initial-project.json"),
        serde_json::to_vec_pretty(&initial)?,
    )?;
    let result = run(&mut check, &initial);
    // Restore reversible transport/navigation preferences even on a failed check.
    for action in [
        json!({"type":"playback","playing":false}),
        json!({"type":"select_track","track":initial["selected_track"]}),
        json!({"type":"set_stem_mix","mix":initial["content"]["stem_mix"]}),
        json!({"type":"set_loop","range":initial["transport"]["loop_range"]}),
        json!({"type":"seek","seconds":initial["transport"]["position"]}),
        json!({"type":"playback","playing":initial["transport"]["playing"]}),
    ] {
        check.action(action)?;
    }
    let final_project = read(&check.project.join("project.json"))?;
    ensure!(
        initial["content"] == final_project["content"],
        "QA must not alter notes, decisions or source attachments"
    );
    let report = json!({"passed":result.is_ok(),"error":result.as_ref().err().map(|e|format!("{e:#}")),"samples":check.samples,"scope":"Native source routing, actual player pause state and advancing cursor. No acoustic listening acceptance or measured device latency."});
    fs::write(
        check.output.join("receipt.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    result
}
