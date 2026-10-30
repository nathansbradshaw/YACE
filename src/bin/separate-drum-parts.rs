//! Explicit local-only LarsNet drum-component experiment. Models must already
//! exist and match the pinned native conversion; no default fetch or chart edit.
use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    about = "Separate an existing drum stem with optional local LarsNet weights (CC-BY-NC4.0)"
)]
struct Arguments {
    #[arg(long)]
    audio: PathBuf,
    /// Folder containing pinned kick/snare/toms/hihat/cymbals.onnx files.
    #[arg(long)]
    models: PathBuf,
    /// New output folder; existing paths are never overwritten.
    #[arg(long)]
    output: PathBuf,
}
fn main() -> Result<()> {
    let args = Arguments::parse();
    let receipt = chart_engine::larsnet::separate_files_with_progress(
        &args.audio,
        &args.models,
        &args.output,
        |done, total| {
            eprintln!("Drum components {done}/{total}");
            Ok(())
        },
    )?;
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    Ok(())
}
