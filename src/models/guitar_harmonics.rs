//! Source-window harmonic regression for guitar candidate verification.
use std::f64::consts::{PI, TAU};
pub(super) const RATE: f64 = 11025.;
pub(super) fn wrap(phase: f64) -> f64 {
    (phase + PI).rem_euclid(TAU) - PI
}
pub(super) fn frequency(pitch: u8) -> f64 {
    440. * 2f64.powf((pitch as f64 - 69.) / 12.)
}

pub(super) struct Fit {
    pub(super) power: Vec<f64>,
    pub(super) phase: Vec<f64>,
    pub(super) explained: f64,
}
/// Joint nuisance-frequency fit avoids treating another chord tone's spectral
/// leakage as a new target fundamental. Real samples only, no padding.
pub(super) fn fit(audio: &[f32], center: f64, frequencies: &[f64]) -> Option<Fit> {
    let half = (0.016 * RATE).round() as usize;
    let middle = (center * RATE).round() as usize;
    if middle < half || middle + half >= audio.len() {
        return None;
    }
    let signal = &audio[middle - half..middle + half];
    let columns = 2 + frequencies.len() * 2;
    let mut normal = vec![vec![0.; columns + 1]; columns];
    let mut rows = Vec::with_capacity(signal.len());
    let mut mean = 0.;
    let mut weights = 0.;
    for (index, &sample) in signal.iter().enumerate() {
        let x = index as f64 / (signal.len() - 1) as f64;
        let w = 0.5 - 0.5 * (TAU * x).cos();
        // Fit phase at the requested source-clock center. Rounding the sample
        // slice must not silently move that phase origin by half a sample.
        let t = (middle - half + index) as f64 / RATE - center;
        let mut row = vec![1., 2. * x - 1.];
        for f in frequencies {
            let p = TAU * f * t;
            row.extend([p.cos(), p.sin()]);
        }
        for (normal_row, left) in normal.iter_mut().zip(&row) {
            for (value, right) in normal_row[..columns].iter_mut().zip(&row) {
                *value += left * right * w;
            }
            normal_row[columns] += left * sample as f64 * w;
        }
        mean += sample as f64 * w;
        weights += w;
        rows.push((row, w, sample as f64));
    }
    let scale = (0..columns).map(|i| normal[i][i]).fold(0., f64::max);
    for column in 0..columns {
        let pivot = (column..columns).max_by(|a, b| {
            normal[*a][column]
                .abs()
                .total_cmp(&normal[*b][column].abs())
        })?;
        if normal[pivot][column].abs() < scale * 1e-9 {
            return None;
        }
        normal.swap(pivot, column);
        let divisor = normal[column][column];
        for value in &mut normal[column][column..=columns] {
            *value /= divisor;
        }
        let pivot = normal[column].clone();
        for (i, row) in normal.iter_mut().enumerate() {
            if i == column {
                continue;
            }
            let factor = row[column];
            for (value, pivot) in row[column..=columns]
                .iter_mut()
                .zip(&pivot[column..=columns])
            {
                *value -= factor * pivot;
            }
        }
    }
    let beta: Vec<_> = normal.iter().map(|r| r[columns]).collect();
    if beta.iter().any(|b| !b.is_finite()) {
        return None;
    }
    mean /= weights;
    let (mut variance, mut error) = (0., 0.);
    for (row, w, sample) in rows {
        let predicted = row.iter().zip(&beta).map(|(a, b)| a * b).sum::<f64>();
        variance += (sample - mean).powi(2) * w;
        error += (sample - predicted).powi(2) * w;
    }
    if !variance.is_finite() || !error.is_finite() || variance < 1e-9 || error > variance {
        return None;
    }
    Some(Fit {
        power: beta[2..]
            .chunks_exact(2)
            .map(|p| p[0] * p[0] + p[1] * p[1])
            .collect(),
        phase: beta[2..]
            .chunks_exact(2)
            .map(|p| (-p[1]).atan2(p[0]))
            .collect(),
        explained: 1. - error / variance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase_origin_stays_at_requested_source_clock_center() {
        for frequency in [82.406889, 329.627557, 1318.510228] {
            for center in [0.401234, 0.508765, 0.611111] {
                let audio: Vec<_> = (0..11025)
                    .map(|i| (0.15 * (TAU * frequency * i as f64 / RATE + 0.7).cos()) as f32)
                    .collect();
                let fit = fit(&audio, center, &[frequency]).unwrap();
                assert!(wrap(fit.phase[0] - (TAU * frequency * center + 0.7)).abs() < 1e-6);
            }
        }
    }
}
