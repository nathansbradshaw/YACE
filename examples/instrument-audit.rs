//! Fixed-scope native instrumentation of the ordinary generator for paired studies.
use anyhow::{Result, ensure};
use chart_engine::{
    generation::write_json,
    mapping::Transcription,
    models,
    pipeline::{self, Build, IsolationOptions},
    timing::TempoMap,
};
use clap::Parser;
use std::{fs, path::PathBuf};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    audio: PathBuf,
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    model_dir: PathBuf,
    #[arg(long)]
    bpm: f64,
    #[arg(long)]
    title: String,
    #[arg(long)]
    artist: String,
    #[arg(long, value_enum, default_value = "guitar")]
    guitar_source: chart_engine::mapping::GuitarSource,
    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    pipeline::destination(&args.output, None)?;
    let manifest = pipeline::read_isolation(&args.audio, &args.manifest)?;
    let source_hash = models::hash_file(&args.audio)?;
    let manifest_hash = models::hash_file(&args.manifest)?;
    let events = pipeline::transcribe_with_isolation(
        &args.audio,
        &args.model_dir,
        IsolationOptions {
            guitar_source: args.guitar_source,
            manifest: Some(&args.manifest),
            ..Default::default()
        },
        &["guitar".into(), "bass".into(), "drums".into()],
    )?;
    events.validate()?;
    let mut metadata = pipeline::metadata(&args.audio)?;
    metadata.name = args.title;
    metadata.artist = args.artist;
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let stage = tempfile::tempdir_in(parent)?;
    write_json(&stage.path().join("revised-events.json"), &events)?;
    pipeline::build(Build {
        audio: &args.audio,
        output: &stage.path().join("revised"),
        existing: None,
        timing: Some(TempoMap::constant(args.bpm)?),
        transcription: &events,
        meta: metadata,
        pro: true,
        lookup_lyrics: false,
    })?;
    ensure!(
        models::hash_file(&args.audio)? == source_hash
            && models::hash_file(&args.manifest)? == manifest_hash,
        "Inference source or manifest changed"
    );
    write_json(
        &stage.path().join("comparison.json"),
        &serde_json::json!({"version":1,"audio_sha256":source_hash,
            "stem_manifest_sha256":manifest_hash,"stems":manifest.stems,
            "scope":"Native guitar, bass, drums; paired evaluator scores Expert and Pro only",
            "targets_enter_inference":false}),
    )?;
    pipeline::publish(stage.path(), &args.output)?;
    let saved: Transcription =
        serde_json::from_slice(&fs::read(args.output.join("revised-events.json"))?)?;
    saved.validate()?;
    Ok(())
}
