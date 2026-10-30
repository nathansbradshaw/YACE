//! Compare chart attack times with an authored chart, without assuming equal
//! fret colors or treating the author's arrangement as a complete audio label.
use anyhow::{Result, ensure};
use chart_engine::chart::Chart;
use std::{collections::BTreeSet, fs};

fn attacks(path: &str, track: &str) -> Result<Vec<f64>> {
    let chart = Chart::parse(fs::read_to_string(path)?)?;
    Ok(chart
        .notes(track)?
        .iter()
        .filter(|n| n.lane <= 4 || (n.lane == 7 && !track.ends_with("Drums")))
        .map(|n| n.tick)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|tick| chart.timing.seconds(tick))
        .collect())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 4,
        "compare_attacks REFERENCE.chart GENERATED.chart TRACK"
    );
    let reference = attacks(&args[1], &args[3])?;
    let generated = attacks(&args[2], &args[3])?;
    ensure!(
        !reference.is_empty() && !generated.is_empty(),
        "Both tracks need notes"
    );
    let (mut r, mut g, mut matched) = (0, 0, 0);
    let (mut unmatched_reference, mut unmatched_generated) = (Vec::new(), Vec::new());
    while r < reference.len() && g < generated.len() {
        if (reference[r] - generated[g]).abs() <= 0.050 {
            matched += 1;
            r += 1;
            g += 1;
        } else if reference[r] < generated[g] {
            unmatched_reference.push(reference[r]);
            r += 1;
        } else {
            unmatched_generated.push(generated[g]);
            g += 1;
        }
    }
    unmatched_reference.extend_from_slice(&reference[r..]);
    unmatched_generated.extend_from_slice(&generated[g..]);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "reference":args[1], "generated":args[2], "track":args[3],
            "tolerance_ms":50, "reference_attacks":reference.len(),
            "generated_attacks":generated.len(), "one_to_one_matched_attacks":matched,
            "unmatched_reference":unmatched_reference, "unmatched_generated":unmatched_generated,
            "limits":"Attack-time agreement only; fret colors, chords, note selection and sustains are not scored. Authored arrangements may select different guitar lines. No automatic offset fitting or timing corrections are applied."
        }))?
    );
    Ok(())
}
