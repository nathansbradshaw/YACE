//! Real local generation on disposable outputs; source media/models remain read-only.
use anyhow::{Context, Result, ensure};
use chart_engine::{generation, models, pipeline, stem_review::StemId, tempo_analysis};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

fn hash(value: &impl serde::Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn phase<T>(name: &str, run: impl FnOnce() -> Result<T>) -> Result<(T, f64)> {
    eprintln!("Starting {name}");
    let start = Instant::now();
    let result = run()?;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("Completed {name}: {ms:.2} ms");
    Ok((result, ms))
}
fn repeated(mut run: impl FnMut() -> Result<()>) -> Result<Value> {
    let mut samples = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        run()?;
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    Ok(json!({"samples_ms":samples,"median_ms":sorted[2],"max_ms":sorted[4]}))
}
fn project_in(root: &Path, depth: usize) -> Result<Option<PathBuf>> {
    if root.join("project.json").is_file() {
        return Ok(Some(root.to_owned()));
    }
    if depth == 0 {
        return Ok(None);
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && let Some(project) = project_in(&entry.path(), depth - 1)?
        {
            return Ok(Some(project));
        }
    }
    Ok(None)
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 2 && args[0] == "--worker" {
        return generation::run(Path::new(&args[1]));
    }
    ensure!(
        args.len() == 3,
        "Usage: cargo run --release --example generation-performance -- SHORT_AUDIO MODEL_DIR FULL_SONG_AUDIO; requires local models, FFmpeg and GPU; no downloads"
    );
    let audio = Path::new(&args[0]).canonicalize()?;
    let model_dir = Path::new(&args[1]).canonicalize()?;
    let parse_audio = Path::new(&args[2]).canonicalize()?;
    let audio_hash = models::hash_file(&audio)?;
    let parse_hash = models::hash_file(&parse_audio)?;
    let temp = tempfile::tempdir()?;
    let (metadata, metadata_ms) = phase("metadata", || pipeline::metadata(&audio))?;
    let (proposal, tempo_ms) = phase("full-song tempo analysis", || {
        tempo_analysis::analyze(&parse_audio)
    })?;
    let tempo_repeat = repeated(|| {
        tempo_analysis::analyze(&parse_audio)?;
        Ok(())
    })?;
    let cache = temp.path().join("isolation");
    let (manifest, cold_split_ms) = phase("cold six-part splitting", || {
        pipeline::cached_separation(&audio, &model_dir, &cache)
    })?;
    let warm_split = repeated(|| {
        pipeline::cached_separation(&audio, &model_dir, &cache)?;
        Ok(())
    })?;
    let validation = repeated(|| {
        manifest.validate_source(&audio, &audio_hash, metadata.duration)?;
        Ok(())
    })?;
    let stem = |id| {
        manifest
            .stems
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.path.clone())
            .context("Missing stem")
    };
    let ((drums, _), drums_ms) = phase("drum inference", || {
        models::drums_with_recovery(
            &stem(StemId::Drums)?,
            &models::verified(&model_dir, "drums")?,
        )
    })?;
    let (guitar, guitar_ms) = phase("guitar inference", || {
        models::guitar_pitch_with_evidence(
            &stem(StemId::Guitar)?,
            &models::verified(&model_dir, "pitch")?,
        )
    })?;
    let mut bass = chart_engine::mapping::Transcription::default();
    let (_, bass_ms) = phase("bass inference", || {
        chart_engine::bass_analysis::infer(
            &mut bass,
            &stem(StemId::Bass)?,
            &models::verified(&model_dir, "pitch")?,
        )
    })?;
    let mut jobs = Vec::new();
    for name in ["cold", "warm"] {
        let request = serde_json::from_value(json!({
            "source":{"type":"audio","path":audio},"bpm":120.0,"title":"Performance fixture","artist":"Local performance test",
            "instruments":["guitar","bass","drums"],"model_dir":model_dir,"fetch_models":false,"lyrics":false
        }))?;
        let destination = temp.path().join(format!("generation-{name}"));
        let (_, total_ms) = phase(&format!("complete generation {name}"), || {
            let root = generation::create(request, &destination)?;
            ensure!(
                std::process::Command::new(std::env::current_exe()?)
                    .arg("--worker")
                    .arg(&root)
                    .status()?
                    .success(),
                "Generation worker failed"
            );
            Ok(root)
        })?;
        let project = project_in(&destination, 6)?.context("Generated project missing")?;
        let store = chart_engine::editor::Store::new(&project);
        let doc = store.load()?;
        jobs.push(json!({"cache":name,"total_ms":total_ms,"chart_sha256":doc.chart_hash(&store)?,"note_count":doc.content.tracks.values().map(Vec::len).sum::<usize>()}));
    }
    ensure!(
        models::hash_file(&audio)? == audio_hash && models::hash_file(&parse_audio)? == parse_hash,
        "Source recording changed during measurement"
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version":1,"source_sha256":audio_hash,"source_seconds":metadata.duration,"parse_audio_sha256":parse_hash,
            "metadata_ms":metadata_ms,"tempo_first_ms":tempo_ms,"tempo_repeat":tempo_repeat,"tempo_proposal_sha256":hash(&proposal)?,
            "cold_split_ms":cold_split_ms,"warm_split":warm_split,"stem_validation":validation,
            "inference_ms":{"drums":drums_ms,"guitar":guitar_ms,"bass":bass_ms},
            "notes_sha256":hash(&json!({"drums":drums,"guitar":guitar.candidates,"bass":bass.bass}))?,
        "generation":jobs,"scope":"Real default six-stem GPU splitting, CPU transcription and complete local generation. Each complete generation runs in a fresh worker process, matching desktop. Cold means no application separation cache, not cold OS/GPU shader caches. Other repeats are warm same-process observations. Full-song tempo analysis is separate from the short generation fixture. No network requests or source/model writes; outputs removed on exit. Hardware contention uncontrolled."
        }))?
    );
    Ok(())
}
