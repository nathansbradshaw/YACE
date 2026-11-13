//! Keep corpus, audio, annotations and reports outside the public repository.
use anyhow::Result;
use clap::Parser;
use std::{fs, io::Write, path::PathBuf};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    corpus: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    diagnostic_partial: bool,
    /// Add separately labeled, duration-weighted annotated-note diagnostics.
    /// Preserves legacy F0 results; this supplementary report cannot promote.
    #[arg(long)]
    note_intervals: bool,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let report = if args.diagnostic_partial {
        chart_engine::vocal_benchmark::evaluate_partial(&args.corpus)?
    } else {
        chart_engine::vocal_benchmark::evaluate(&args.corpus)?
    };
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(args.output)?;
    if args.note_intervals {
        let notes = chart_engine::vocal_benchmark::diagnose_note_intervals(&args.corpus)?;
        file.write_all(&serde_json::to_vec_pretty(&serde_json::json!({
            "version":1,"profile":"separate_note_interval_diagnostic_v1",
            "status":"supplementary_diagnostic_not_a_promotion_gate","promotion_eligible":false,
            "legacy_f0_evaluation":&report,"note_interval_comparisons":notes,
            "interpretation":"Continuous performed F0 and annotated musical notes are distinct references. Interval metrics include missing and wrong note time and false notes in annotated gaps. Original F0 results/gates are retained, not redefined. A revised promotion protocol must be frozen and evaluated separately."
        }))?)?;
        println!("Separate note-interval diagnostic; promotion eligible: false");
        return Ok(());
    }
    file.write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}; promotion eligible: {}",
        report.status, report.promotion_eligible
    );
    Ok(())
}
