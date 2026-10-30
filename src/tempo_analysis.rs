//! Estimate tempo from changes in audio energy, without a model.
//! The result still needs review for tempo changes and the first beat.
use anyhow::Result;
use rustfft::{FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};
use std::path::Path;

const RATE: u32 = 11025;
const WINDOW: usize = 1024;
const HOP: usize = 256;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Estimate {
    pub bpm: Option<f64>,
    pub alternatives: Vec<f64>,
    /// Normalized periodicity, not a probability of musical correctness.
    pub periodicity: f64,
    pub note: String,
}

pub fn analyze(path: &Path) -> Result<Estimate> {
    Ok(estimate(&crate::models::decode(path, RATE, 1)?))
}

pub fn estimate(samples: &[f32]) -> Estimate {
    let missing = || Estimate {
        note:
            "No stable pulse was measured. A temporary 120 BPM map is suggested; edit it if needed."
                .into(),
        ..Default::default()
    };
    if samples.len() < RATE as usize * 8 {
        return missing();
    }
    let fft = FftPlanner::<f32>::new().plan_fft_forward(WINDOW);
    let mut bins = vec![Complex::new(0.0, 0.0); WINDOW];
    let mut previous = vec![0.0_f32; WINDOW / 2];
    let mut onset = Vec::new();
    for frame in samples.chunks(HOP).enumerate().filter_map(|(i, _)| {
        let start = i * HOP;
        (start + WINDOW <= samples.len()).then(|| &samples[start..start + WINDOW])
    }) {
        for (i, bin) in bins.iter_mut().enumerate() {
            *bin = Complex::new(
                frame[i]
                    * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (WINDOW - 1) as f32).cos()),
                0.0,
            );
        }
        fft.process(&mut bins);
        let mut flux = 0.0;
        for i in 4..WINDOW / 2 - 1 {
            let power = (1.0 + bins[i].norm() * 10.0).ln();
            flux += (power - previous[i]).max(0.0);
            previous[i] = power;
        }
        onset.push(flux as f64);
    }
    if onset.iter().copied().fold(0.0, f64::max) < 1e-5 {
        return missing();
    }
    // Remove the moving baseline so sustained loud passages do not themselves
    // become evidence of a beat. Keep positive transient energy.
    let raw = onset.clone();
    for (i, value) in onset.iter_mut().enumerate() {
        let begin = i.saturating_sub(8);
        let end = (i + 9).min(raw.len());
        *value = (*value - raw[begin..end].iter().sum::<f64>() / (end - begin) as f64).max(0.0);
    }
    let energy = onset.iter().map(|v| v * v).sum::<f64>();
    if energy < 1e-6 {
        return missing();
    }
    let frames_per_second = RATE as f64 / HOP as f64;
    let correlation = |lag: f64| {
        let whole = lag.floor() as usize;
        let fraction = lag.fract();
        if whole + 1 >= onset.len() {
            return 0.0;
        }
        let mut dot = 0.0;
        let mut a = 0.0;
        let mut b = 0.0;
        for i in whole + 1..onset.len() {
            let shifted = onset[i - whole] * (1.0 - fraction) + onset[i - whole - 1] * fraction;
            dot += onset[i] * shifted;
            a += onset[i] * onset[i];
            b += shifted * shifted;
        }
        dot / (a * b).sqrt().max(1e-12)
    };
    let mut candidates = Vec::new();
    for quarter in 240..=800 {
        let bpm = quarter as f64 / 4.0;
        let lag = frames_per_second * 60.0 / bpm;
        let periodicity = correlation(lag);
        let prior = 1.0 - 0.08 * ((bpm / 120.0).ln() / 2.0_f64.ln()).abs();
        let score =
            (periodicity + 0.35 * correlation(lag * 2.0) + 0.15 * correlation(lag * 3.0)) * prior;
        candidates.push((bpm, score, periodicity));
    }
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    let best = candidates[0];
    if best.2 < 0.12 {
        return missing();
    }
    let alternatives = candidates
        .iter()
        .filter(|item| (item.0 - best.0).abs() > 5.0 && item.1 > best.1 * 0.8)
        .fold(Vec::<f64>::new(), |mut list, item| {
            if list.len() < 3 && list.iter().all(|v| (*v - item.0).abs() > 5.0) {
                list.push(item.0);
            }
            list
        });
    Estimate {bpm:Some(best.0),alternatives,periodicity:best.2,note:"Audio-derived tempo proposal. Half/double tempo and tempo changes may need correction; the beat phase is not established.".into()}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measures_click_tempos_and_does_not_invent_a_pulse_in_silence() {
        for bpm in [90.0, 120.0, 150.0] {
            let mut samples = vec![0.0; RATE as usize * 24];
            let mut beat = 1.0;
            while beat < 23.0 {
                let start = (beat * RATE as f64) as usize;
                for i in 0..600 {
                    samples[start + i] = ((i as f32 * 0.7).sin() + (i as f32 * 1.73).sin())
                        * (-(i as f32) / 90.0).exp();
                }
                beat += 60.0 / bpm;
            }
            let result = estimate(&samples);
            assert!((result.bpm.unwrap() - bpm).abs() < 2.0, "{bpm}: {result:?}");
        }
        assert!(estimate(&vec![0.0; RATE as usize * 12]).bpm.is_none());
    }
}
