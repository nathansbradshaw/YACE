//! Native before/after ablation from identical model outputs and source audio.
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
#[path = "support/rhythm_audit.rs"]
mod recovery;

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
    let mut metadata = pipeline::metadata(&args.audio)?;
    metadata.name = args.title;
    metadata.artist = args.artist;
    let baseline = recovery::recovery_disabled(&events)?;
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let stage = tempfile::tempdir_in(parent)?;
    for (name, transcription) in [("baseline", &baseline), ("revised", &events)] {
        write_json(
            &stage.path().join(format!("{name}-events.json")),
            transcription,
        )?;
        pipeline::build(Build {
            audio: &args.audio,
            output: &stage.path().join(name),
            existing: None,
            timing: Some(TempoMap::constant(args.bpm)?),
            transcription,
            meta: metadata.clone(),
            pro: true,
            lookup_lyrics: false,
        })?;
    }
    // Both charts use a single actual source clock; this is an ablation, not
    // an independently annotated timing/precision or listening benchmark.
    let counts = |e: &Transcription| serde_json::json!({"guitar":e.guitar.len(),"bass":e.bass.len(),"drums":e.drums.len()});
    let summary = serde_json::json!({"version":2,"audio_sha256":models::hash_file(&args.audio)?,"stem_manifest_sha256":models::hash_file(&args.manifest)?,"stems":manifest.stems,"baseline":counts(&baseline),"revised":counts(&events),"recovered_guitar":events.guitar_short_candidates.iter().filter(|r|r.retained).count(),"recovered_percussion":events.drum_recovered_hits.len(),"ablated_passes":recovery::pass_counts(&events),"note":"Identical inference, all current guitar/percussion recovery and drum articulation disabled/enabled. This expanded ablation is not the historical frozen two-pass audit or a comparison against that generator. No reference notes or listening approval; Bass is unchanged."});
    write_json(&stage.path().join("comparison.json"), &summary)?;
    ensure!(
        !args.output.exists(),
        "Audit output appeared during inference"
    );
    pipeline::publish(stage.path(), &args.output)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    // Ensure published files remain parseable by the ordinary saved-event path.
    let _: Transcription =
        serde_json::from_slice(&fs::read(args.output.join("revised-events.json"))?)?;
    Ok(())
}
