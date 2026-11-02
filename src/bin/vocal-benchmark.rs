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
    file.write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}; promotion eligible: {}",
        report.status, report.promotion_eligible
    );
    Ok(())
}
