use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
mod run;
#[derive(Parser)]
#[command(
    about = "Source-bound Expert/Pro chart evaluation; no target timestamps enter generation"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Verify archive asset receipts and prepare direct oracle stems/text only.
    Prepare { case: PathBuf, output: PathBuf },
    /// Compare all five Expert/Pro instruments, retaining unsupported/missing failures.
    Compare {
        case: PathBuf,
        generated_notes: PathBuf,
        output: PathBuf,
    },
    /// Staged inference: oracle, full-mix, or model-free-vocals condition.
    Run {
        engine: PathBuf,
        case: PathBuf,
        oracle: PathBuf,
        output: PathBuf,
        model_dir: PathBuf,
        condition: String,
    },
    /// Prepare and generate a frozen batch; failures remain in its receipt.
    Batch {
        engine: PathBuf,
        batch: PathBuf,
        output: PathBuf,
        model_dir: PathBuf,
        condition: String,
    },
    /// Aggregate all successes and failures without dropping missing instruments.
    Evaluate {
        batch: PathBuf,
        runs: PathBuf,
        output: PathBuf,
    },
    /// Six-case oracle ablation of committed guitar/drum recoveries; no vocals/full-mix.
    RecoveryBatch {
        audit_binary: PathBuf,
        prepared_batch: PathBuf,
        new_output: PathBuf,
        model_dir: PathBuf,
    },
    /// Paired old/new revised-policy instrumental outputs on the same six oracle inputs.
    PairedInstrumentBatch {
        baseline_audit: PathBuf,
        candidate_audit: PathBuf,
        prepared_batch: PathBuf,
        new_output: PathBuf,
        model_dir: PathBuf,
    },
}
fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Prepare { case, output } => chart_benchmark::prepare(&case, &output)?,
        Command::Compare {
            case,
            generated_notes,
            output,
        } => chart_benchmark::compare(&case, &generated_notes, &output)?,
        Command::Run {
            engine,
            case,
            oracle,
            output,
            model_dir,
            condition,
        } => run::run(run::Request {
            engine: &engine,
            case: &case,
            oracle: &oracle,
            output: &output,
            models: &model_dir,
            condition: &condition,
        })?,
        Command::Batch {
            engine,
            batch,
            output,
            model_dir,
            condition,
        } => run::batch(&engine, &batch, &output, &model_dir, &condition)?,
        Command::Evaluate {
            batch,
            runs,
            output,
        } => chart_benchmark::evaluate::evaluate(&batch, &runs, &output)?,
        Command::RecoveryBatch {
            audit_binary,
            prepared_batch,
            new_output,
            model_dir,
        } => chart_benchmark::recoveries::batch(
            &audit_binary,
            &prepared_batch,
            &new_output,
            &model_dir,
        )?,
        Command::PairedInstrumentBatch {
            baseline_audit,
            candidate_audit,
            prepared_batch,
            new_output,
            model_dir,
        } => chart_benchmark::paired::batch(
            &baseline_audit,
            &candidate_audit,
            &prepared_batch,
            &new_output,
            &model_dir,
        )?,
    }
    Ok(())
}
