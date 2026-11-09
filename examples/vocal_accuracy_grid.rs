//! Private development experiments: predictions finish before references are read.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    models,
    vocal_benchmark::{Corpus, Reference, score, score_exported_notes},
    vocal_generation::Generation,
    vocal_independent,
};
use clap::Parser;
use serde_json::json;
use std::{fs, path::PathBuf, process::Command};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    corpus: PathBuf,
    #[arg(long)]
    cache: PathBuf,
    #[arg(long)]
    predictor: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    case: String,
    #[arg(long)]
    consensus_extension: bool,
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "0.04,0.06,0.08,0.10,0.12,0.18"
    )]
    recovery: Vec<f64>,
    #[arg(long, value_delimiter = ',', default_value = "0.03,0.04,0.06,0.08")]
    note_minimum: Vec<f64>,
}
fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(!a.output.exists(), "Use a new output directory");
    ensure!(
        a.recovery.len() * a.note_minimum.len() <= 24
            && !a.recovery.is_empty()
            && !a.note_minimum.is_empty(),
        "Grid budget"
    );
    ensure!(
        a.recovery
            .iter()
            .all(|v| v.is_finite() && (0.04..=0.3).contains(v))
            && a.note_minimum
                .iter()
                .all(|v| v.is_finite() && (0.03..=0.5).contains(v)),
        "Invalid research thresholds"
    );
    let corpus: Corpus = serde_json::from_slice(&fs::read(&a.corpus)?)?;
    let r = corpus
        .recordings
        .iter()
        .find(|r| r.id == a.case)
        .context("Missing case")?;
    ensure!(
        r.split == "development",
        "Grid selection is restricted to development artists"
    );
    let baseline: Generation = serde_json::from_slice(&fs::read(
        r.candidate.as_ref().context("Candidate baseline")?,
    )?)?;
    ensure!(
        models::hash_file(&r.audio)? == r.audio_sha256 && baseline.audio_sha256 == r.audio_sha256,
        "Source differs"
    );
    let cache = a.cache.join(vocal_independent::cache_key(&baseline)?);
    ensure!(
        cache.join("source-swift.json").is_file()
            && cache.join("source.notes.json").is_file()
            && cache.join("fresh-full-source/evidence.json").is_file(),
        "Finish the source-bound independent cache first"
    );
    fs::create_dir(&a.output)?;
    let output = fs::canonicalize(&a.output)?;
    fs::write(
        output.join("recipe.json"),
        serde_json::to_vec_pretty(
            &json!({"case":r.id,"corpus_sha256":models::hash_file(&a.corpus)?,"predictor_sha256":models::hash_file(&a.predictor)?,"recovery_seconds":a.recovery,"note_minimum_seconds":a.note_minimum,"consensus_extension":a.consensus_extension,"predictions_precede_reference_read":true,"promotion_eligible":false}),
        )?,
    )?;
    let mut predictions = vec![];
    for recovery in &a.recovery {
        for minimum in &a.note_minimum {
            let target = output.join(format!(
                "recovery-{:03}-note-{:03}",
                (recovery * 1000.).round() as u32,
                (minimum * 1000.).round() as u32
            ));
            let mut command = Command::new(&a.predictor);
            if a.consensus_extension {
                command.arg("--research-consensus-extension");
            }
            let status = command
                .arg(cache.join("input.json"))
                .arg(&cache)
                .arg(&cache)
                .arg(&target)
                .args(["--notes-root"])
                .arg(&cache)
                .args(["--swift-root"])
                .arg(&cache)
                .args([
                    "--predict-only",
                    "--predict-recovery",
                    "--predict-energy",
                    "--held-out",
                    "--crepe-prefix",
                    "fresh-full-",
                    "--research-minimum-duration",
                ])
                .arg(recovery.to_string())
                .arg("--research-note-minimum")
                .arg(minimum.to_string())
                .status()?;
            ensure!(status.success(), "Research prediction failed");
            predictions.push((*recovery, *minimum, target.join("source.generation.json")));
        }
    }
    // The predictor never receives this reference path or its annotations.
    let reference: Reference = serde_json::from_slice(&fs::read(
        r.reference
            .as_ref()
            .context("Reference required for scoring")?,
    )?)?;
    let baseline_raw = score(&reference, &baseline)?;
    let baseline_notes = score_exported_notes(&reference, &baseline)?;
    let mut rows = vec![];
    for (recovery, minimum, path) in predictions {
        let prediction: Generation = serde_json::from_slice(&fs::read(&path)?)?;
        rows.push(json!({"recovery_seconds":recovery,"note_minimum_seconds":minimum,"generation":path,"raw":score(&reference,&prediction)?,"notes":score_exported_notes(&reference,&prediction)?}));
    }
    fs::write(
        output.join("scores.json"),
        serde_json::to_vec_pretty(
            &json!({"case":r.id,"baseline_raw":baseline_raw,"baseline_notes":baseline_notes,"rows":rows,"promotion_eligible":false}),
        )?,
    )?;
    Ok(())
}
