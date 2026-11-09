//! Explicit local activity evidence; never changes a project or selects pitches.
use anyhow::{Context, Result, ensure};
use chart_engine::{models, vocal_activity, vocal_runtime};
use clap::Parser;
use std::{fs, io::Write, path::PathBuf};

#[derive(Parser)]
#[command(
    about = "Source-bound speech activity research; no lyrics, chart edits or model downloads"
)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    model_file: PathBuf,
    #[arg(long)]
    runtime: Option<PathBuf>,
    /// New evidence JSON file; existing results are never overwritten.
    #[arg(long)]
    output: PathBuf,
}
fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(!a.output.exists(), "Use a new activity evidence output");
    ensure!(
        models::hash_file(&a.model_file)? == vocal_activity::MODEL_SHA256,
        "Activity model checksum differs; another model was not substituted"
    );
    let executable = std::env::current_exe()?;
    let runtime = vocal_runtime::resolve(
        executable.parent().context("Probe directory")?,
        a.runtime
            .map(Into::into)
            .or_else(|| std::env::var_os("ORT_DYLIB_PATH")),
    )?
    .context("Set --runtime or ORT_DYLIB_PATH for standalone activity research")?;
    ort::init_from(runtime.path.to_str().context("Runtime path is not UTF-8")?).commit()?;
    let mut evidence = vocal_activity::analyze(&a.input, &a.model_file, &|| false)?;
    evidence.runtime = Some(vocal_activity::RuntimeIdentity {
        version: if runtime.source_sha256 == vocal_runtime::SOURCE_SHA256 {
            vocal_runtime::VERSION.into()
        } else {
            runtime.version
        },
        library_sha256: runtime.sha256,
        publisher_source_sha256: runtime.source_sha256,
    });
    vocal_activity::validate(
        &evidence,
        &evidence.source_sha256,
        evidence.duration_seconds,
    )?;
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(a.output)?
        .write_all(&serde_json::to_vec_pretty(&evidence)?)?;
    Ok(())
}
