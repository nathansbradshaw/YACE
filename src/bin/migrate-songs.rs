//! Explicit, backed-up migration of an existing local charting library.
use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Plan or apply a backed-up migration to standard YACE song folders")]
struct Args {
    /// A charting library, song, or old generation wrapper. Never scans backups.
    #[arg(required_unless_present = "restore")]
    roots: Vec<PathBuf>,
    /// Apply the plan; without this flag nothing is changed.
    #[arg(long)]
    apply: bool,
    /// A new directory, outside all selected song folders, to retain originals.
    #[arg(long, required_if_eq("apply", "true"))]
    backup: Option<PathBuf>,
    /// Restore exact originals, retaining the migrated folders in this backup.
    #[arg(long, conflicts_with_all = ["apply", "backup", "roots"])]
    restore: Option<PathBuf>,
}
fn main() -> Result<()> {
    let args = Args::parse();
    if let Some(backup) = args.restore {
        chart_engine::song_migration::restore(&backup)?;
        println!(
            "Restored originals; migrated copies retained in {}",
            backup.display()
        );
        return Ok(());
    }
    let plan = chart_engine::song_migration::plan(&args.roots)?;
    if args.apply {
        let report = chart_engine::song_migration::apply(
            &plan,
            args.backup.as_deref().expect("clap requires backup"),
        )?;
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    }
    Ok(())
}
