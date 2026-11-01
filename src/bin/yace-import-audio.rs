//! Model-free local import, also usable from scripts and the private URL importer.
use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    about = "Read a local recording, optionally look up song details and create a YACE project"
)]
struct Args {
    audio: PathBuf,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    artist: Option<String>,
    /// Share song metadata with catalogues to find release details, cover and lyrics.
    #[arg(long)]
    lookup: bool,
    /// New standard song folder. Omit to print analysis without creating a project.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Override measured timing with a constant BPM.
    #[arg(long)]
    bpm: Option<f64>,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let mut recording = chart_engine::song_import::local(&args.audio)?;
    if let Some(title) = args.title {
        recording.metadata.name = title;
    }
    if let Some(artist) = args.artist {
        recording.metadata.artist = artist;
    }
    if args.lookup {
        chart_engine::song_import::lookup(
            &mut recording.metadata,
            &mut recording.info,
            &chart_engine::song_import::artwork_cache(&args.audio)?,
            true,
        )?;
    }
    let project = args
        .output
        .as_ref()
        .map(|output| {
            chart_engine::song_import::create_project(&args.audio, &recording, args.bpm, output)
        })
        .transpose()?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"recording":recording,"project":project})
        )?
    );
    Ok(())
}
