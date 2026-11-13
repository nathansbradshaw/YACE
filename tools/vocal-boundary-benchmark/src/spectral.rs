//! Audio-only spectral shape and per-recording timbre evidence; no note labels.
use anyhow::{Result, ensure};
use rustfft::{FftPlanner, num_complex::Complex64};
use std::f64::consts::{PI, TAU};
pub const WIDTH: usize = 20;
const FFT: usize = 1024;
const RATE: f64 = 16000.;
fn distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
}
fn centers(rows: &[[f64; WIDTH]], seeds: &[usize]) -> Vec<Vec<f64>> {
    if seeds.is_empty() {
        return vec![];
    }
    let mut means = vec![rows[seeds[seeds.len() / 2]][4..16].to_vec()];
    for _ in 1..3 {
        let i = *seeds
            .iter()
            .max_by(|&&a, &&b| {
                let d = |i: usize| {
                    means
                        .iter()
                        .map(|c| distance(&rows[i][4..16], c))
                        .fold(f64::INFINITY, f64::min)
                };
                d(a).total_cmp(&d(b)).then(a.cmp(&b))
            })
            .unwrap();
        if means.iter().any(|m| distance(m, &rows[i][4..16]) < 1e-8) {
            break;
        }
        means.push(rows[i][4..16].to_vec());
    }
    for _ in 0..12 {
        let mut sums = vec![vec![0.; 12]; means.len()];
        let mut count = vec![0usize; means.len()];
        for &i in seeds {
            let j = (0..means.len())
                .min_by(|&a, &b| {
                    distance(&rows[i][4..16], &means[a])
                        .total_cmp(&distance(&rows[i][4..16], &means[b]))
                        .then(a.cmp(&b))
                })
                .unwrap();
            count[j] += 1;
            for k in 0..12 {
                sums[j][k] += rows[i][k + 4];
            }
        }
        for j in 0..means.len() {
            if count[j] > 0 {
                for k in 0..12 {
                    means[j][k] = sums[j][k] / count[j] as f64;
                }
            }
        }
    }
    means
}

/// Native source times, optional raw F0 in Hz and independent singing evidence.
/// Every seed is chosen from this audio's model evidence before reading labels.
pub fn extract(
    pcm: &[f32],
    times: &[f64],
    pitches: &[Option<f64>],
    voice: &[f64],
    confidence: &[f64],
) -> Result<Vec<[f64; WIDTH]>> {
    ensure!(
        !pcm.is_empty() && pcm.len() <= 16000 * 600 && pcm.iter().all(|v| v.is_finite()),
        "Invalid/over-budget audio"
    );
    let n = times.len();
    ensure!(
        n > 0 && n <= 60001 && pitches.len() == n && voice.len() == n && confidence.len() == n,
        "Feature clock lengths"
    );
    ensure!(
        times.iter().enumerate().all(|(i, &t)| t.is_finite()
            && t >= 0.
            && (i == 0 || times[i - 1] < t)
            && t <= pcm.len() as f64 / RATE + 1. / RATE)
            && pitches.iter().flatten().all(|p| p.is_finite() && *p > 0.)
            && voice
                .iter()
                .chain(confidence)
                .all(|p| p.is_finite() && (0.0..=1.).contains(p)),
        "Invalid feature observations"
    );
    let fft = FftPlanner::new().plan_fft_forward(FFT);
    let mut weights = vec![vec![]; 32];
    let mel = |h: f64| 2595. * (1. + h / 700.).log10();
    let hz = |m: f64| 700. * (10f64.powf(m / 2595.) - 1.);
    let edges: Vec<_> = (0..34)
        .map(|i| hz(mel(80.) + (mel(6000.) - mel(80.)) * i as f64 / 33.))
        .collect();
    for (j, w) in weights.iter_mut().enumerate() {
        for k in 0..=FFT / 2 {
            let h = k as f64 * RATE / FFT as f64;
            let v = if h < edges[j + 1] {
                (h - edges[j]) / (edges[j + 1] - edges[j])
            } else {
                (edges[j + 2] - h) / (edges[j + 2] - edges[j + 1])
            };
            if v > 0. {
                w.push((k, v));
            }
        }
    }
    let mut rows = vec![];
    let mut previous = vec![0.; FFT / 2 + 1];
    for (i, &time) in times.iter().enumerate() {
        let at = (time * RATE).round() as isize;
        let mut energy = 0.;
        let mut values: Vec<_> = (0..FFT)
            .map(|k| {
                let at = at + k as isize - FFT as isize / 2;
                let v = if at >= 0 {
                    pcm.get(at as usize).copied().unwrap_or(0.) as f64
                } else {
                    0.
                };
                energy += v * v;
                Complex64::new(
                    v * (0.5 - 0.5 * (TAU * k as f64 / (FFT - 1) as f64).cos()),
                    0.,
                )
            })
            .collect();
        fft.process(&mut values);
        let power: Vec<_> = values[..=FFT / 2].iter().map(|v| v.norm_sqr()).collect();
        let total = power[5..].iter().sum::<f64>().max(1e-20);
        let mut row = [0.; WIDTH];
        row[0] = 10. * (energy / FFT as f64).max(1e-12).log10();
        row[1] = (power[5..].iter().map(|p| p.max(1e-20).ln()).sum::<f64>()
            / (power.len() - 5) as f64)
            .exp()
            / (total / (power.len() - 5) as f64);
        row[2] = power
            .iter()
            .zip(&previous)
            .map(|(a, b)| (a / total - b).max(0.))
            .sum::<f64>();
        for k in 0..power.len() {
            previous[k] = power[k] / total;
        }
        if let Some(h) = pitches[i] {
            row[3] = power
                .iter()
                .enumerate()
                .filter(|(k, _)| {
                    let f = *k as f64 * RATE / FFT as f64;
                    let harmonic = (f / h).round();
                    (1.0..=24.0).contains(&harmonic)
                        && (f - h * harmonic).abs()
                            <= ((RATE / FFT as f64) * 0.75).max(h * harmonic * 0.015)
                })
                .map(|(_, p)| p)
                .sum::<f64>()
                / power.iter().sum::<f64>().max(1e-20);
        }
        let bands: Vec<_> = weights
            .iter()
            .map(|w| {
                w.iter()
                    .map(|&(k, v)| power[k] * v)
                    .sum::<f64>()
                    .max(1e-20)
                    .ln()
            })
            .collect();
        let mean = bands.iter().sum::<f64>() / 32.;
        for k in 1..=12 {
            row[k + 3] = bands
                .iter()
                .enumerate()
                .map(|(j, b)| (b - mean) * (PI * k as f64 * (j as f64 + 0.5) / 32.).cos())
                .sum::<f64>()
                / 32.;
        }
        let norm = row[4..16].iter().map(|v| v * v).sum::<f64>().sqrt().max(1.);
        for v in &mut row[4..16] {
            *v /= norm;
        }
        rows.push(row);
    }
    let strong: Vec<_> = (0..n)
        .step_by(5)
        .filter(|&i| voice[i] >= 0.9 && confidence[i] >= 0.6 && rows[i][0] >= -45.)
        .collect();
    let background: Vec<_> = (0..n)
        .step_by(5)
        .filter(|&i| voice[i] <= 0.1 && confidence[i] >= 0.6 && rows[i][0] >= -45.)
        .collect();
    let positive = centers(&rows, &strong);
    let negative = centers(&rows, &background);
    for row in &mut rows {
        row[16] = positive
            .iter()
            .map(|m| distance(m, &row[4..16]))
            .fold(2., f64::min);
        row[17] = negative
            .iter()
            .map(|m| distance(m, &row[4..16]))
            .fold(2., f64::min);
        row[18] = row[17] - row[16];
        row[19] = f64::from(!positive.is_empty()) + 2. * f64::from(!negative.is_empty());
    }
    ensure!(
        rows.iter().flatten().all(|v| v.is_finite()),
        "Nonfinite spectral features"
    );
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_edges_silence_and_clock_validation() {
        let rows = extract(
            &vec![0.; 16000],
            &[0., 0.5, 1.],
            &[None; 3],
            &[0.; 3],
            &[0.; 3],
        )
        .unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().flatten().all(|v| v.is_finite()));
        assert!(rows.iter().all(|r| r[19] == 0.));
        assert!(extract(&[0.; 16], &[1.], &[None], &[0.], &[0.]).is_err());
    }
    #[test]
    fn normalized_shape_ignores_gain_and_changes_with_harmonic_envelope() {
        let wave = |gain: f64, high: f64| {
            (0..16000)
                .map(|i| {
                    let phase = TAU * 220. * i as f64 / RATE;
                    (gain * (phase.sin() + high * (4. * phase).sin())) as f32
                })
                .collect::<Vec<_>>()
        };
        let times = vec![0.25, 0.5, 0.75];
        let pitches = vec![Some(220.); 3];
        let a = extract(&wave(0.2, 0.2), &times, &pitches, &[1.; 3], &[1.; 3]).unwrap();
        let b = extract(&wave(0.02, 0.2), &times, &pitches, &[1.; 3], &[1.; 3]).unwrap();
        let c = extract(&wave(0.2, 2.), &times, &pitches, &[1.; 3], &[1.; 3]).unwrap();
        assert!(distance(&a[1][4..16], &b[1][4..16]) < 0.001);
        assert!(distance(&a[1][4..16], &c[1][4..16]) > 0.01);
        assert!((a[1][0] - b[1][0] - 20.).abs() < 0.001);
    }
}
