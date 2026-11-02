//! Bounded listening comparisons of the existing Rust separator checkpoints.
//! Four-source output has no independent guitar/keys; never a game-song export.
use anyhow::{Result, ensure};
use burn::backend::Wgpu;
use chart_engine::models;
use clap::{Parser, ValueEnum};
use demucs_core::{Demucs, ModelOptions};
use std::path::PathBuf;

#[derive(Clone, Copy, ValueEnum)]
enum Variant {
    SixStem,
    FineTuned,
}

#[derive(Parser)]
#[command(about = "Create an unreviewed, bounded separation listening comparison")]
struct Args {
    #[arg(long)]
    audio: PathBuf,
    #[arg(long)]
    model: PathBuf,
    #[arg(long, value_enum)]
    variant: Variant,
    #[arg(long, default_value_t = 0.)]
    start: f64,
    #[arg(long, default_value_t = 7.8)]
    seconds: f64,
    /// New directory. This tool never edits charts or replaces existing stems.
    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.start.is_finite()
            && args.start >= 0.
            && args.seconds.is_finite()
            && args.seconds > 0.
            && args.seconds <= 30.,
        "Choose a finite start and a preview length of at most 30 seconds"
    );
    chart_engine::pipeline::destination(&args.output, Some(&args.audio))?;
    let (options, expected) = match args.variant {
        Variant::SixStem => (
            ModelOptions::SixStem,
            "f56fe666f0bbf3a645764856ed90f8c5dd8cf4430b1d9649b94e29ec45aa9057",
        ),
        Variant::FineTuned => (
            ModelOptions::FineTuned(demucs_core::model::metadata::HTDEMUCS_FT.stems.to_vec()),
            "255c2650d26537ce4887c9c4cf08c6d4896fad2fecc0b78dc5b875b117bcc575",
        ),
    };
    ensure!(
        models::hash_file(&args.model)? == expected,
        "Checkpoint checksum mismatch"
    );
    let source_hash = models::hash_file(&args.audio)?;
    let pcm = models::decode(&args.audio, 44100, 2)?;
    let start = (args.start * 44100.).round() as usize;
    let frames = (args.seconds * 44100.).round() as usize;
    ensure!(
        frames > 0 && start < pcm.len() / 2,
        "Preview is outside the source"
    );
    let end = start.saturating_add(frames).min(pcm.len() / 2);
    let left: Vec<_> = pcm[start * 2..end * 2].iter().step_by(2).copied().collect();
    let right: Vec<_> = pcm[start * 2..end * 2]
        .iter()
        .skip(1)
        .step_by(2)
        .copied()
        .collect();
    drop(pcm);
    let bytes = std::fs::read(&args.model)?;
    let runner = Demucs::<Wgpu>::from_bytes(options, &bytes, Default::default())
        .map_err(|error| anyhow::anyhow!("Cannot load model: {error}"))?;
    drop(bytes);
    let stems = pollster::block_on(runner.separate(&left, &right, 44100))
        .map_err(|error| anyhow::anyhow!("Separation failed: {error}"))?;
    ensure!(
        stems.iter().all(|stem| stem.left.len() == left.len()
            && stem.right.len() == right.len()
            && stem.left.iter().chain(&stem.right).all(|s| s.is_finite())),
        "Model returned invalid preview audio"
    );
    let parent = args.output.parent().unwrap_or(std::path::Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".separation-preview-")
        .tempdir_in(parent)?;
    models::write_audio(&stage.path().join("original.wav"), &left, &right)?;
    for stem in &stems {
        models::write_audio(
            &stage.path().join(format!("{}.wav", stem.id.as_str())),
            &stem.left,
            &stem.right,
        )?;
    }
    ensure!(
        models::hash_file(&args.audio)? == source_hash
            && models::hash_file(&args.model)? == expected,
        "Preview inputs changed"
    );
    let receipt = serde_json::json!({
        "status":"listening_comparison_unreviewed", "source_sha256":source_hash,
        "model_sha256":expected, "sample_rate":44100, "start_frame":start,
        "frames":end-start, "normalization":"none",
        "stems":stems.iter().map(|stem| stem.id.as_str()).collect::<Vec<_>>(),
        "limitations":"An excerpt, not a full-song cache or game package. Four-source other includes guitar and keys. Source identity and finite samples do not establish musical quality."
    });
    std::fs::write(
        stage.path().join("comparison.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    chart_engine::pipeline::publish(stage.path(), &args.output)?;
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    Ok(())
}
