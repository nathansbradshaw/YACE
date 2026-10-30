//! Find possible attacks in a mono guitar stem without a model.
//! Use the results for listening; they do not identify pitches or prove strums.
use anyhow::{Result, ensure};
use rustfft::{FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};

/// Version of this independent signal-analysis policy, not a confidence scale.
pub const ALGORITHM: &str = "guitar-multiband-attacks-v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Attack {
    /// Position in the original mono sample clock, without tempo snapping.
    pub seconds: f64,
    /// Gain-compensated positive spectral novelty; not a probability.
    pub strength: f64,
    /// Number of frequency bands contributing independent positive novelty.
    pub bands: usize,
}

struct Frame {
    center: usize,
    novelty: f64,
    bands: usize,
    high_band: bool,
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn band(hz: f64) -> usize {
    if hz < 180. {
        0
    } else if hz < 500. {
        1
    } else if hz < 1400. {
        2
    } else if hz < 4000. {
        3
    } else {
        4
    }
}

/// Analyze mono PCM at its supplied sample rate. Silence/insufficient real
/// context yields no candidates; invalid rates or nonfinite PCM are errors.
///
/// Only full source windows are analyzed. A centered spectral peak is localized
/// using a real-sample energy edge before refractory selection. No resampling,
/// synthetic padding, pitch model, beat grid or chart/reference input is used.
pub fn detect(audio: &[f32], sample_rate: u32) -> Result<Vec<Attack>> {
    ensure!(
        (8000..=192000).contains(&sample_rate),
        "Guitar attack analysis requires a sample rate between 8000 and 192000 Hz"
    );
    ensure!(
        audio.iter().all(|sample| sample.is_finite()),
        "Guitar attack analysis requires finite mono PCM"
    );
    let rate = sample_rate as f64;
    let desired = (rate * 0.046).round() as usize;
    let upper = desired.next_power_of_two();
    let lower = upper / 2;
    let size = if desired - lower < upper - desired {
        lower
    } else {
        upper
    };
    let half = size / 2;
    let hop = (rate * 0.005).round().max(1.) as usize;
    if audio.len() < size + 4 * hop {
        return Ok(Vec::new());
    }
    let window: Vec<_> = (0..size)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (size - 1) as f64).cos())
        .collect();
    let weight = window.iter().sum::<f64>();
    let bins: Vec<_> = (1..=half)
        .filter_map(|index| {
            let hz = index as f64 * rate / size as f64;
            (hz >= 60. && hz <= (rate * 0.475).min(10000.)).then_some((index, band(hz)))
        })
        .collect();
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(size);
    let mut buffer = vec![Complex::new(0., 0.); size];
    let mut previous = vec![0.; bins.len()];
    let mut frames = Vec::new();
    for center in (half..=audio.len() - half).step_by(hop) {
        for (i, value) in buffer.iter_mut().enumerate() {
            *value = Complex::new(audio[center - half + i] as f64 * window[i], 0.);
        }
        fft.process(&mut buffer);
        let current: Vec<_> = bins
            .iter()
            .map(|(index, _)| buffer[*index].norm() / weight)
            .collect();
        let prior_peak = previous.iter().copied().fold(0., f64::max);
        // A robust scalar gain estimate removes changes common to the existing
        // spectral components, rather than calling every volume rise a strum.
        let mut ratios: Vec<_> = current
            .iter()
            .zip(&previous)
            .filter(|(_, before)| **before > (prior_peak * 0.03).max(1e-8))
            .map(|(now, before)| now / before)
            .collect();
        let gain = if ratios.len() >= 3 {
            median(&mut ratios)
        } else {
            1.
        };
        let mut positive = [0.; 5];
        let mut totals = [0.; 5];
        for (((_, band), now), before) in bins.iter().zip(&current).zip(&previous) {
            positive[*band] += (now - gain * before).max(0.);
            totals[*band] += now;
        }
        let total = totals.iter().sum::<f64>();
        let contributing: Vec<_> = (0..5)
            .filter(|b| {
                positive[*b] >= total * 0.015
                    && totals[*b] > 1e-8
                    && positive[*b] >= totals[*b] * 0.30
            })
            .collect();
        let novelty = if total > 1e-7 && !frames.is_empty() {
            positive.iter().sum::<f64>() / total
        } else {
            0.
        };
        frames.push(Frame {
            center,
            novelty,
            bands: contributing.len(),
            high_band: contributing.iter().any(|b| *b >= 3),
        });
        previous = current;
    }

    // Millisecond blocks keep energy localization linear in time without an
    // extra f64 allocation for every PCM sample. Remainders are not padded.
    let edge_step = (rate * 0.001).round().max(1.) as usize;
    let edge_window = (rate * 0.012 / edge_step as f64).round().max(1.) as usize;
    let mut energy = Vec::with_capacity(audio.len() / edge_step + 1);
    energy.push(0.);
    for block in audio.chunks_exact(edge_step) {
        let power = block
            .iter()
            .map(|sample| (*sample as f64).powi(2))
            .sum::<f64>();
        energy.push(energy.last().copied().unwrap_or(0.) + power);
    }
    let local_radius = ((0.3 * rate) / hop as f64).ceil() as usize;
    let mut candidates = Vec::new();
    for i in 2..frames.len().saturating_sub(2) {
        let frame = &frames[i];
        if frame.bands < 3
            || !frame.high_band
            || frame.novelty < 0.12
            || frame.novelty <= frames[i - 1].novelty
            || frame.novelty < frames[i + 1].novelty
            || frame.novelty < frames[i - 2].novelty
            || frame.novelty < frames[i + 2].novelty
        {
            continue;
        }
        let lo = i.saturating_sub(local_radius);
        let hi = (i + local_radius + 1).min(frames.len());
        let mut local: Vec<_> = frames[lo..hi].iter().map(|f| f.novelty).collect();
        let floor = median(&mut local);
        let mut deviations: Vec<_> = local.iter().map(|v| (v - floor).abs()).collect();
        if frame.novelty <= floor + 4. * median(&mut deviations) {
            continue;
        }
        let start = frame
            .center
            .saturating_sub(half)
            .div_ceil(edge_step)
            .max(edge_window);
        let end = ((frame.center + half) / edge_step).min(energy.len() - 1 - edge_window);
        let edge = (start..=end)
            .map(|position| {
                let before = (energy[position] - energy[position - edge_window])
                    / (edge_window * edge_step) as f64;
                let after = (energy[position + edge_window] - energy[position])
                    / (edge_window * edge_step) as f64;
                (position, before, after)
            })
            .filter(|(_, before, after)| *after > 1e-8 && *after > *before * 1.6)
            .max_by(|a, b| (a.2 - a.1).total_cmp(&(b.2 - b.1)));
        if let Some((position, _, _)) = edge {
            candidates.push(Attack {
                seconds: (position * edge_step) as f64 / rate,
                strength: frame.novelty,
                bands: frame.bands,
            });
        }
    }
    // Greedy strongest-first refractory selection avoids an early weak spectral
    // shoulder consuming the true peak. The final list is chronological.
    candidates.sort_by(|a, b| {
        b.strength
            .total_cmp(&a.strength)
            .then(a.seconds.total_cmp(&b.seconds))
    });
    let refractory = (rate * 0.055).ceil() as usize;
    let mut selected = std::collections::BTreeMap::<usize, Attack>::new();
    for candidate in candidates {
        let position = (candidate.seconds * rate).round() as usize;
        let clear_before = selected
            .range(..=position)
            .next_back()
            .is_none_or(|(earlier, _)| position - earlier >= refractory);
        let clear_after = selected
            .range(position..)
            .next()
            .is_none_or(|(later, _)| later - position >= refractory);
        if clear_before && clear_after {
            selected.insert(position, candidate);
        }
    }
    Ok(selected.into_values().collect())
}
