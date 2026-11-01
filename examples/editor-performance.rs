//! Long-lived Store timings, complementing the subprocess performance probe.
use anyhow::{Context, Result, ensure};
use chart_engine::editor::{Action, Actor, ActorKind, Store};
use serde_json::{Value, json};
use std::{fs, hint::black_box, path::PathBuf, time::Instant};

fn measure(mut operation: impl FnMut() -> Result<()>) -> Result<Value> {
    let mut times = Vec::new();
    for sample in 0..22 {
        let start = Instant::now();
        operation()?;
        if sample >= 2 {
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    times.sort_by(f64::total_cmp);
    Ok(json!({"samples":20,"p50_ms":times[10],"p95_ms":times[18],"max_ms":times[19]}))
}
fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("Usage: cargo run --release --example editor-performance -- PROJECT")?,
    );
    let original = fs::read(root.join("project.json"))?;
    let chart = fs::read(root.join("source/notes.chart"))?;
    let copy = tempfile::tempdir()?;
    fs::create_dir(copy.path().join("source"))?;
    fs::create_dir(copy.path().join("checkpoints"))?;
    fs::write(copy.path().join("project.json"), &original)?;
    fs::write(copy.path().join("source/notes.chart"), &chart)?;
    let store = Store::new(copy.path());
    let project = store.load()?;
    let load = measure(|| {
        black_box(store.load()?);
        Ok(())
    })?;
    let timing = measure(|| {
        black_box(project.timing(&store)?);
        Ok(())
    })?;
    let chart_hash = measure(|| {
        black_box(project.chart_hash(&store)?);
        Ok(())
    })?;
    let mut revision = project.revision;
    let seek = measure(|| {
        revision = store
            .apply(chart_engine::editor::Request {
                expected_revision: revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "performance-probe".into(),
                },
                action: Action::Seek { seconds: 0.0 },
            })?
            .revision;
        Ok(())
    })?;
    ensure!(
        fs::read(root.join("project.json"))? == original
            && fs::read(root.join("source/notes.chart"))? == chart,
        "Source changed during measurement"
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version":1,"project_bytes":original.len(),"note_count":project.content.tracks.values().map(Vec::len).sum::<usize>(),
            "load":load,"timing":timing,"chart_hash":chart_hash,"seek":seek,
            "scope":"In-process release measurements on one long-lived Store, using a disposable metadata-only copy; 2 warmups and 20 samples. Excludes UI reload/paint, audio, source media, and model inference. Host contention uncontrolled."
        }))?
    );
    Ok(())
}
