//! Rust implementation of the WeSpeaker inference frontend's declared settings.
//! Numerical parity with torchaudio/Kaldi has not been established.
use anyhow::{Result, ensure};
use rustfft::{FftPlanner, num_complex::Complex64};
use std::f64::consts::TAU;
pub fn fbank(pcm: &[f32]) -> Result<Vec<f32>> {
    ensure!(
        (400..=16000 * 120).contains(&pcm.len())
            && pcm.iter().all(|v| v.is_finite() && v.abs() <= 8.),
        "Invalid speaker audio window"
    );
    let mel = |h: f64| 1127. * (1. + h / 700.).ln();
    let lo = mel(20.);
    let hi = mel(8000.);
    let mut weights = vec![vec![]; 80];
    for (j, w) in weights.iter_mut().enumerate() {
        let a = lo + (hi - lo) * j as f64 / 81.;
        let b = lo + (hi - lo) * (j + 1) as f64 / 81.;
        let c = lo + (hi - lo) * (j + 2) as f64 / 81.;
        for k in 0..256 {
            let m = mel(k as f64 * 16000. / 512.);
            let v = if m < b {
                (m - a) / (b - a)
            } else {
                (c - m) / (c - b)
            };
            if v > 0. {
                w.push((k, v));
            }
        }
    }
    let fft = FftPlanner::new().plan_fft_forward(512);
    let frames = (pcm.len() - 400) / 160 + 1;
    let mut out = Vec::with_capacity(frames * 80);
    for frame in 0..frames {
        let mut raw: Vec<_> = pcm[frame * 160..frame * 160 + 400]
            .iter()
            .map(|v| *v as f64 * 32768.)
            .collect();
        let mean = raw.iter().sum::<f64>() / 400.;
        for v in &mut raw {
            *v -= mean;
        }
        let mut values = vec![Complex64::new(0., 0.); 512];
        for k in 0..400 {
            let v = if k == 0 {
                raw[k] * (1. - 0.97)
            } else {
                raw[k] - 0.97 * raw[k - 1]
            };
            values[k].re = v * (0.54 - 0.46 * (TAU * k as f64 / 399.).cos());
        }
        fft.process(&mut values);
        let power: Vec<_> = values[..257].iter().map(|v| v.norm_sqr()).collect();
        for w in &weights {
            let energy = w.iter().map(|&(k, v)| power[k] * v).sum::<f64>();
            out.push(energy.max(f32::EPSILON as f64).ln() as f32);
        }
    }
    for j in 0..80 {
        let mean = (0..frames).map(|i| out[i * 80 + j] as f64).sum::<f64>() / frames as f64;
        for i in 0..frames {
            out[i * 80 + j] -= mean as f32;
        }
    }
    ensure!(
        out.iter().all(|v| v.is_finite()),
        "Nonfinite speaker features"
    );
    Ok(out)
}
pub fn normalize(v: &mut [f32]) -> Result<f64> {
    let norm = v.iter().map(|v| (*v as f64).powi(2)).sum::<f64>().sqrt();
    ensure!(norm.is_finite() && norm > 1e-9, "Invalid voice embedding");
    for v in v {
        *v = (*v as f64 / norm) as f32;
    }
    Ok(norm)
}
pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(a, b)| *a as f64 * *b as f64)
        .sum::<f64>()
        .clamp(-1., 1.)
}
pub fn prototypes(embeddings: &[Vec<f32>], seeds: &[usize]) -> Result<Vec<Vec<f32>>> {
    if seeds.is_empty() {
        return Ok(vec![]);
    }
    ensure!(
        !embeddings.is_empty()
            && !embeddings[0].is_empty()
            && embeddings[0].len() <= 512
            && embeddings
                .iter()
                .all(|e| e.len() == embeddings[0].len() && e.iter().all(|v| v.is_finite()))
            && seeds.iter().all(|&i| i < embeddings.len()),
        "Invalid voice profile seed/shape"
    );
    let mut means = vec![embeddings[seeds[0]].clone()];
    for _ in 1..3 {
        let index = *seeds
            .iter()
            .min_by(|&&a, &&b| {
                let score = |i: usize| {
                    means
                        .iter()
                        .map(|m| cosine(m, &embeddings[i]))
                        .fold(-1., f64::max)
                };
                score(a).total_cmp(&score(b)).then(a.cmp(&b))
            })
            .unwrap();
        if means.iter().any(|m| cosine(m, &embeddings[index]) > 0.9999) {
            break;
        }
        means.push(embeddings[index].clone());
    }
    for _ in 0..12 {
        let mut sums = vec![vec![0.; means[0].len()]; means.len()];
        let mut counts = vec![0; means.len()];
        for &i in seeds {
            let j = (0..means.len())
                .max_by(|&a, &b| {
                    cosine(&means[a], &embeddings[i])
                        .total_cmp(&cosine(&means[b], &embeddings[i]))
                        .then(b.cmp(&a))
                })
                .unwrap();
            counts[j] += 1;
            for (k, v) in embeddings[i].iter().enumerate() {
                sums[j][k] += *v;
            }
        }
        for j in 0..means.len() {
            if counts[j] > 0 && normalize(&mut sums[j]).is_ok() {
                means[j] = sums[j].clone();
            }
        }
    }
    Ok(means)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frontend_keeps_native_frame_count_and_normalizes_channels() {
        let p: Vec<_> = (0..16000)
            .map(|i| {
                let t = i as f64 / 16000.;
                (0.2 * (TAU * (200. * t + 100. * t * t)).sin()) as f32
            })
            .collect();
        let a = fbank(&p).unwrap();
        let b = fbank(&p.iter().map(|v| v * 0.5).collect::<Vec<_>>()).unwrap();
        assert_eq!(a.len(), 98 * 80);
        for j in 0..80 {
            assert!((0..98).map(|i| a[i * 80 + j] as f64).sum::<f64>().abs() / 98. < 1e-5);
        }
        assert!(
            a.iter()
                .zip(b)
                .map(|(a, b)| (*a as f64 - b as f64).abs())
                .sum::<f64>()
                / (a.len() as f64)
                < 0.001
        );
        assert!(fbank(&[0.; 399]).is_err());
        assert!(fbank(&[f32::NAN; 400]).is_err());
    }
    #[test]
    fn silence_is_finite_and_profiles_are_deterministic() {
        assert!(fbank(&[0.; 16000]).unwrap().iter().all(|v| v.is_finite()));
        let e = vec![vec![1., 0., 0.], vec![0., 1., 0.], vec![0., 0., 1.]];
        assert_eq!(
            prototypes(&e, &[0, 1, 2]).unwrap(),
            prototypes(&e, &[0, 1, 2]).unwrap()
        );
        assert!(prototypes(&e, &[]).unwrap().is_empty());
        assert!(prototypes(&e, &[3]).is_err());
        assert!(normalize(&mut [0.; 3]).is_err());
    }
}
