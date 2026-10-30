//! Human and AI callers receive the same source-bound coverage and pending flags.
use anyhow::{Result, ensure};
use chart_engine::{
    chart::Chart, mapping::Transcription, models, pipeline, stem_coverage, stem_coverage_review,
    stem_review::StemManifest,
};
use clap::Parser;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    about = "Source-bound stem coverage diagnostics and pending review flags; musical review remains required"
)]
struct Args {
    #[arg(long)]
    audio: PathBuf,
    #[arg(long)]
    chart: PathBuf,
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    events: Option<PathBuf>,
    /// Matching existing markers whose genuine generation scope/issues survive.
    #[arg(long)]
    base_markers: Option<PathBuf>,
    /// New directory containing coverage.json and review-markers.json.
    #[arg(short, long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    pipeline::destination(&args.output, None)?;
    let manifest_file_hash = models::hash_file(&args.manifest)?;
    let base = args
        .base_markers
        .as_ref()
        .map(|path| -> Result<_> {
            let hash = models::hash_file(path)?;
            let map: chart_engine::review_regions::ReviewMap =
                serde_json::from_slice(&fs::read(path)?)?;
            Ok((map, hash))
        })
        .transpose()?;
    let manifest = StemManifest::read(&args.manifest)?;
    let duration = manifest.frames as f64 / manifest.sample_rate as f64;
    manifest.validate_binding(&models::hash_file(&args.audio)?, duration)?;
    let chart = Chart::parse(fs::read_to_string(&args.chart)?)?;
    let events: Option<Transcription> = args
        .events
        .as_ref()
        .map(|path| -> Result<_> { Ok(serde_json::from_slice(&fs::read(path)?)?) })
        .transpose()?;
    stem_coverage_review::validate_chart_events(&chart, events.as_ref(), duration)?;
    if let Some((base, hash)) = &base {
        // Fail a stale/different baseline before expensive signal processing.
        let identity = chart_engine::review_regions::ReviewMap::build(
            duration,
            Default::default(),
            BTreeMap::from([
                ("chart".into(), models::hash_file(&args.chart)?),
                ("original_audio".into(), models::hash_file(&args.audio)?),
            ]),
            None,
            Default::default(),
            vec![],
        )?;
        stem_coverage_review::merge_into_base(&identity, base, hash)?;
    }
    let report = stem_coverage::audit_files(
        &args.audio,
        &args.chart,
        &args.manifest,
        args.events.as_deref(),
        &Default::default(),
    )?;
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".stem-audit-")
        .tempdir_in(parent)?;
    let report_path = stage.path().join("coverage.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    let derived = stem_coverage_review::review_map_with_inputs(
        &report,
        duration,
        BTreeMap::from([
            ("stem_manifest_file".into(), manifest_file_hash.clone()),
            (
                "stem_coverage_report_file".into(),
                models::hash_file(&report_path)?,
            ),
        ]),
    )?;
    let markers = if let Some((base, hash)) = &base {
        stem_coverage_review::merge_into_base(&derived, base, hash)?
    } else {
        derived
    };
    fs::write(
        stage.path().join("review-markers.json"),
        serde_json::to_vec_pretty(&markers)?,
    )?;
    ensure!(
        models::hash_file(&args.audio)? == report.source_audio_sha256
            && models::hash_file(&args.chart)? == report.chart_sha256
            && models::hash_file(&args.manifest)? == manifest_file_hash,
        "Audit inputs changed before publication"
    );
    for asset in &report.manifest.stems {
        ensure!(
            models::hash_file(&asset.path)?.eq_ignore_ascii_case(&asset.sha256),
            "Audit stem changed before publication"
        );
    }
    if let Some(path) = &args.events {
        ensure!(
            Some(models::hash_file(path)?) == report.transcription_sha256,
            "Audit event file changed before publication"
        );
    }
    if let (Some(path), Some((_, expected))) = (&args.base_markers, &base) {
        ensure!(
            models::hash_file(path)?.eq_ignore_ascii_case(expected),
            "Base review markers changed before publication"
        );
    }
    pipeline::publish(stage.path(), &args.output)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "status":"source_bound_diagnostics", "output":args.output,
            "coverage":args.output.join("coverage.json"),"review_markers":args.output.join("review-markers.json"),
            "source_audio_sha256":report.source_audio_sha256,"chart_sha256":report.chart_sha256,
            "regions":markers.regions.len(),"musical_acceptance":"unverified",
            "interpretation":"Pending listening prompts only. No chart notes were changed; musical review remains required."
        }))?
    );
    Ok(())
}
