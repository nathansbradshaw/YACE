//! Source-bound cached SwiftF0 evidence; authored references are never loaded.
use anyhow::{Context, Result, ensure};
use chart_engine::{clean::StemAlignment, models, vocal_generation::Generation, vocal_pitch};
use clap::Parser;
use std::{fs, path::PathBuf};
#[derive(Parser)]
struct Args {
    audio: PathBuf,
    stem: PathBuf,
    report: PathBuf,
    model: PathBuf,
    output: PathBuf,
}
fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(!a.output.exists(), "Use a new output file");
    let r: Generation = serde_json::from_slice(&fs::read(a.report)?)?;
    let alignment = StemAlignment {
        audio_sha256: r.audio_sha256.clone(),
        stem_sha256: r.vocal_stem_sha256.clone(),
        evidence: "Source-bound independent contour evaluation; no reference input".into(),
    };
    ensure!(
        models::hash_file(&a.audio)? == r.audio_sha256
            && models::hash_file(&a.stem)? == r.vocal_stem_sha256,
        "Source differs"
    );
    let evidence = vocal_pitch::analyze(&a.audio, &a.stem, &alignment, &a.model, &|| false)
        .context("SwiftF0")?;
    fs::write(a.output, serde_json::to_vec(&evidence)?)?;
    Ok(())
}
