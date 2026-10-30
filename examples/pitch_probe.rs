//! Inspect detector disagreement on a real stem without editing its chart.
use anyhow::Result;
use pitch_detection::detector::{PitchDetector, mcleod::McLeodDetector, yin::YINDetector};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len() >= 3, "pitch_probe STEM START_SECONDS");
    let audio = chart_engine::models::decode(std::path::Path::new(&args[1]), 11025, 1)?;
    let start: f64 = args[2].parse()?;
    anyhow::ensure!(
        start.is_finite() && start >= 0.0,
        "Start must be finite and nonnegative"
    );
    let mut m = McLeodDetector::<f64>::new(2048, 1024);
    let mut y = YINDetector::<f64>::new(2048, 1024);
    for offset in [0.1, 0.15, 0.2, 0.25, 0.3] {
        let center = ((start + offset) * 11025.0) as usize;
        anyhow::ensure!(
            center >= 1024 && center + 1024 <= audio.len(),
            "Frame outside stem"
        );
        let frame: Vec<_> = audio[center - 1024..center + 1024]
            .iter()
            .map(|v| *v as f64)
            .collect();
        let a = m
            .get_pitch(&frame, 11025, 0.01, 0.92)
            .map(|p| (69.0 + 12.0 * (p.frequency / 440.0).log2(), p.clarity));
        let b = y
            .get_pitch(&frame, 11025, 0.01, 0.9)
            .map(|p| (69.0 + 12.0 * (p.frequency / 440.0).log2(), p.clarity));
        println!("time={:.3} McLeod={a:?} YIN={b:?}", start + offset);
    }
    Ok(())
}
