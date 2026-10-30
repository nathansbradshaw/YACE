//! Measure snapshot allocation on a real saved song rather than infer speed
//! from the implementation language. This isolates snapshot cost, not total FPS.
use anyhow::Result;
use chart_engine::editor::Store;
use clap::Parser;
use std::{hint::black_box, path::PathBuf, sync::Arc, time::Instant};
#[derive(Parser)]
struct Args {
    project: PathBuf,
    #[arg(long, default_value_t = 300)]
    iterations: usize,
}
fn measure(mut sample: impl FnMut(), iterations: usize) -> serde_json::Value {
    let mut ns: Vec<_> = (0..iterations)
        .map(|_| {
            let start = Instant::now();
            sample();
            start.elapsed().as_nanos() as u64
        })
        .collect();
    ns.sort_unstable();
    serde_json::json!({"p50_microseconds":ns[ns.len()/2] as f64/1000.,"p95_microseconds":ns[(ns.len()-1)*95/100] as f64/1000.,"max_microseconds":ns.last().unwrap().to_owned() as f64/1000.})
}
fn main() -> Result<()> {
    let args = Args::parse();
    anyhow::ensure!(
        (10..=10000).contains(&args.iterations),
        "Use 10–10000 samples"
    );
    let doc = Arc::new(Store::new(&args.project).load()?);
    let deep = measure(
        || {
            let value = doc.as_ref().clone();
            black_box(&value);
            drop(value);
        },
        args.iterations,
    );
    let shared = measure(
        || {
            let value = Arc::clone(&doc);
            black_box(&value);
            drop(value);
        },
        args.iterations,
    );
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"notes":doc.content.tracks.values().map(Vec::len).sum::<usize>(),"tracks":doc.content.tracks.len(),"iterations":args.iterations,"former_deep_snapshot":deep,"shared_snapshot":shared,"scope":"Snapshot creation and release only; not whole-editor FPS or an OCTAVE/Moonscraper comparison"})
        )?
    );
    Ok(())
}
