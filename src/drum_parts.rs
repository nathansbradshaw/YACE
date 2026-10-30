//! Model-free spectral activity on an existing mono drum stem.
//!
//! Show activity in overlapping frequency bands.
//! A kick and a bass note can share a band, so these are listening aids rather
//! than isolated instruments or automatic note labels.
use anyhow::{Result, ensure};
use rustfft::{FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};

pub const ALGORITHM: &str = "drum-spectral-activity-v1";
pub const BANDS: [(&str, f64, f64); 4] = [
    ("low_30_120_hz", 30., 120.),
    ("body_120_500_hz", 120., 500.),
    ("attack_2000_6000_hz", 2000., 6000.),
    ("metal_6000_16000_hz", 6000., 16000.),
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PartEvidence {
    /// Spectral-band identity, never a claim that this is an isolated drum.
    pub part: String,
    /// Approximate rises in band energy, on the input sample clock. Centered
    /// windows can smear an onset by half a window (about23ms at44.1kHz).
    pub attacks: Vec<f64>,
    /// Frame-pooled band RMS, in input amplitude units; no per-song gain.
    pub rms: f64,
}

fn peaks(envelope: &[f64], hop: usize, rate: u32, duration: f64) -> Vec<f64> {
    let scores: Vec<_> = envelope
        .iter()
        .enumerate()
        .map(|(i, value)| (value - i.checked_sub(1).map_or(0., |j| envelope[j])).max(0.))
        .collect();
    let maximum = scores.iter().copied().fold(0., f64::max);
    let signal = envelope.iter().copied().fold(0., f64::max);
    let threshold = (maximum * 0.12).max(signal * 0.02).max(1e-6);
    let mut selected: Vec<usize> = Vec::new();
    for (i, score) in scores.iter().copied().enumerate() {
        if score < threshold
            || scores[i.saturating_sub(2)..i].iter().any(|v| *v >= score)
            || scores[i + 1..(i + 3).min(scores.len())]
                .iter()
                .any(|v| *v > score)
        {
            continue;
        }
        if let Some(previous) = selected.last_mut()
            && (i - *previous) as f64 * (hop as f64) / (rate as f64) < 0.06
        {
            if score > scores[*previous] {
                *previous = i;
            }
            continue;
        }
        selected.push(i);
    }
    selected
        .into_iter()
        .map(|i| (i * hop) as f64 / rate as f64)
        .filter(|time| *time < duration)
        .collect()
}

/// Analyse finite mono PCM without models, downloads, resampling or chart edits.
/// Supported rates8–192kHz; maximum one hour. Bands above Nyquist remain present
/// with zero energy/attacks. Attacks are band-energy candidates, not beat ticks.
pub fn analyse(audio: &[f32], sample_rate: u32) -> Result<Vec<PartEvidence>> {
    ensure!(
        (8000..=192000).contains(&sample_rate),
        "Unsupported drum analysis sample rate"
    );
    ensure!(!audio.is_empty(), "Empty drum analysis audio");
    ensure!(
        audio.len() as u64 <= sample_rate as u64 * 3600,
        "Drum analysis exceeds one hour"
    );
    ensure!(
        audio.iter().all(|v| v.is_finite()),
        "Nonfinite drum analysis audio"
    );
    let fft_size = 1usize << ((sample_rate as f64 * 0.046).log2().round() as u32);
    let hop = (sample_rate as f64 * 0.005).round() as usize;
    let window: Vec<_> = (0..fft_size)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / fft_size as f64).cos())
        .collect();
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(fft_size);
    let mut buffer = vec![Complex::new(0., 0.); fft_size];
    let mut envelopes = vec![Vec::with_capacity(audio.len().div_ceil(hop)); BANDS.len()];
    for center in (0..audio.len()).step_by(hop) {
        let mut weight = 0.;
        for (i, value) in buffer.iter_mut().enumerate() {
            let source = center as i64 + i as i64 - (fft_size / 2) as i64;
            *value = if source >= 0 && (source as usize) < audio.len() {
                weight += window[i].powi(2);
                Complex::new(audio[source as usize] as f64 * window[i], 0.)
            } else {
                Complex::new(0., 0.)
            };
        }
        fft.process(&mut buffer);
        let mut energies = [0.; BANDS.len()];
        for (bin, value) in buffer[..=fft_size / 2].iter().enumerate() {
            let hz = bin as f64 * sample_rate as f64 / fft_size as f64;
            let power = value.norm_sqr()
                * if bin == 0 || bin == fft_size / 2 {
                    1.
                } else {
                    2.
                };
            for (index, (_, low, high)) in BANDS.iter().enumerate() {
                if hz >= *low && hz < *high {
                    energies[index] += power;
                }
            }
        }
        for (envelope, energy) in envelopes.iter_mut().zip(energies) {
            envelope.push((energy / (fft_size as f64 * weight)).sqrt());
        }
    }
    let duration = audio.len() as f64 / sample_rate as f64;
    Ok(BANDS
        .iter()
        .zip(envelopes)
        .map(|((part, _, _), envelope)| PartEvidence {
            part: (*part).into(),
            attacks: peaks(&envelope, hop, sample_rate, duration),
            rms: (envelope.iter().map(|v| v * v).sum::<f64>() / envelope.len() as f64).sqrt(),
        })
        .collect())
}
